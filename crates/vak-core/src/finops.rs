//! FinOps (docs/design/27 Phase D): persisted cost ledger + pre-dispatch
//! budget admission. Every settled dispatch appends an estimated-USD row
//! keyed by durable attribution ids; caps are checked BEFORE each paid
//! call. Denial is a question (budget Ask), not a crash; unattended
//! surfaces auto-deny via the normal approver path.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use vak_agent::{SpendCheck, SpendGate};
use vak_llm::Usage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRow {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    /// Estimated USD. `None` when the model is unpriced — absent is
    /// UNKNOWN, never zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
    /// Always "estimated" today; reserved for providers that return real
    /// settlement data.
    pub source: String,
    pub session_id: String,
}

pub struct FinOpsLedger {
    path: PathBuf,
}

impl FinOpsLedger {
    pub fn new(sessions_home: &std::path::Path) -> Self {
        FinOpsLedger {
            path: sessions_home.join("cost-log.jsonl"),
        }
    }

    pub fn append(&self, row: &CostRow) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(row)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")
    }

    /// USD total over rows at or after `since`. Unpriced rows contribute
    /// nothing but are never treated as evidence of zero spend elsewhere.
    pub fn total_usd_since(&self, since: chrono::DateTime<chrono::Utc>) -> f64 {
        let Ok(f) = std::fs::File::open(&self.path) else {
            return 0.0;
        };
        let mut total = 0.0;
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            if let Ok(row) = serde_json::from_str::<CostRow>(&line)
                && row.ts >= since
                && let Some(usd) = row.usd
            {
                total += usd;
            }
        }
        total
    }

    pub fn day_total_usd(&self, now: chrono::DateTime<chrono::Utc>) -> f64 {
        let day_start = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(chrono::Utc).single());
        match day_start {
            Some(start) => self.total_usd_since(start),
            None => 0.0,
        }
    }
}

/// Core-side SpendGate: run/day caps + pricing-driven estimates.
pub struct CoreSpendGate {
    ledger: FinOpsLedger,
    max_run_usd: Option<f64>,
    max_day_usd: Option<f64>,
    overrides: BTreeMap<String, vak_config::PriceEntry>,
    run_spent_usd: Mutex<f64>,
    raised_once: AtomicBool,
}

impl CoreSpendGate {
    pub fn new(sessions_home: &std::path::Path, finops: &vak_config::FinopsResolved) -> Self {
        CoreSpendGate {
            ledger: FinOpsLedger::new(sessions_home),
            max_run_usd: finops.max_run_usd,
            max_day_usd: finops.max_day_usd,
            overrides: finops.price_overrides.clone(),
            run_spent_usd: Mutex::new(0.0),
            raised_once: AtomicBool::new(false),
        }
    }

    /// The approver answered "raise the cap for this run once".
    pub fn raise_once(&self) {
        self.raised_once.store(true, Ordering::SeqCst);
    }

    fn estimate(&self, model: &str, usage: &Usage) -> Option<f64> {
        vak_config::finops::estimate_cost_usd(model, usage, &self.overrides)
    }
}

#[async_trait::async_trait]
impl SpendGate for CoreSpendGate {
    async fn authorize(&self, check: &SpendCheck<'_>) -> Result<(), String> {
        let planned = Usage {
            input_tokens: check.est_input_tokens,
            output_tokens: check.planned_output_tokens,
            ..Default::default()
        };
        // Unpriced model: UNKNOWN cost cannot be admitted or denied on
        // dollars — allow and record without a USD figure.
        let Some(est) = self.estimate(check.model, &planned) else {
            return Ok(());
        };
        let run_spent = *self
            .run_spent_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cap) = self.max_run_usd
            && !self.raised_once.load(Ordering::SeqCst)
            && run_spent + est > cap
        {
            return Err(format!(
                "run budget ${cap:.2} would be exceeded by this dispatch (+${est:.2}, ${run_spent:.2} already spent)"
            ));
        }
        if let Some(cap) = self.max_day_usd {
            let day = self.ledger.day_total_usd(chrono::Utc::now());
            if day + est > cap {
                return Err(format!(
                    "day budget ${cap:.2} would be exceeded by this dispatch (+${est:.2}, ${day:.2} spent today)"
                ));
            }
        }
        Ok(())
    }

    fn record_settled(&self, model: &str, session_id: &str, usage: &Usage) {
        let usd = self.estimate(model, usage);
        let row = CostRow {
            ts: chrono::Utc::now(),
            model: model.to_string(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            usd,
            source: "estimated".to_string(),
            session_id: session_id.to_string(),
        };
        if let Err(e) = self.ledger.append(&row) {
            // Ledger write failure must not kill a healthy run; the
            // receipt entries in the session log still carry usage.
            eprintln!("warning: cost ledger append failed: {e}");
            return;
        }
        if let Some(usd) = usd {
            let mut spent = self
                .run_spent_usd
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *spent += usd;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use tempfile::tempdir;

    fn gate_with(caps: &[(&str, f64)]) -> (CoreSpendGate, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let mut finops = vak_config::FinopsResolved::default();
        for (k, v) in caps {
            match *k {
                "run" => finops.max_run_usd = Some(*v),
                "day" => finops.max_day_usd = Some(*v),
                _ => unreachable!(),
            }
        }
        (CoreSpendGate::new(dir.path(), &finops), dir)
    }

    fn usage(in_tok: u64, out_tok: u64) -> Usage {
        Usage {
            input_tokens: in_tok,
            output_tokens: out_tok,
            ..Default::default()
        }
    }

    fn check(model: &'static str) -> SpendCheck<'static> {
        SpendCheck {
            model,
            session_id: "s1",
            est_input_tokens: 1_000_000,
            planned_output_tokens: 100_000,
        }
    }

    #[tokio::test]
    async fn unpriced_model_admits_and_records_unknown_usd() {
        let (gate, dir) = gate_with(&[("run", 0.01)]);
        gate.authorize(&check("mystery-model")).await.unwrap();
        gate.record_settled("mystery-model", "s1", &usage(1_000_000, 100_000));
        let ledger = FinOpsLedger::new(dir.path());
        assert_eq!(ledger.day_total_usd(chrono::Utc::now()), 0.0);
        let text = std::fs::read_to_string(dir.path().join("cost-log.jsonl")).unwrap();
        assert!(text.contains("\"usd\":null") || !text.contains("\"usd\""));
    }

    #[tokio::test]
    async fn run_cap_denies_then_raise_once_admits() {
        // sonnet: $3/MTok in → est = 3*1 + 15*0.1 = $4.50 per dispatch.
        let (gate, _dir) = gate_with(&[("run", 5.0)]);
        gate.authorize(&check("claude-sonnet")).await.unwrap(); // 4.5 <= 5
        gate.record_settled("claude-sonnet", "s1", &usage(1_000_000, 100_000));
        let err = gate.authorize(&check("claude-sonnet")).await.unwrap_err();
        assert!(err.contains("run budget $5.00"), "{err}");
        gate.raise_once();
        gate.authorize(&check("claude-sonnet")).await.unwrap();
    }

    #[tokio::test]
    async fn day_cap_counts_seeded_ledger_rows() {
        let (gate, dir) = gate_with(&[("day", 6.0)]);
        let ledger = FinOpsLedger::new(dir.path());
        ledger
            .append(&CostRow {
                ts: chrono::Utc::now(),
                model: "claude-sonnet".into(),
                input_tokens: 1_000_000,
                output_tokens: 100_000,
                cache_read_input_tokens: None,
                usd: Some(2.0),
                source: "estimated".into(),
                session_id: "seed".into(),
            })
            .unwrap();
        // est 4.50 + day 2.00 > 6.00 → denied with day wording.
        let err = gate.authorize(&check("claude-sonnet")).await.unwrap_err();
        assert!(err.contains("day budget $6.00"), "{err}");
    }

    #[test]
    fn day_window_ignores_yesterday() {
        let dir = tempdir().unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        let yesterday = chrono::Utc::now() - chrono::Duration::hours(30);
        ledger
            .append(&CostRow {
                ts: yesterday,
                model: "m".into(),
                input_tokens: 1,
                output_tokens: 1,
                cache_read_input_tokens: None,
                usd: Some(99.0),
                source: "estimated".into(),
                session_id: "old".into(),
            })
            .unwrap();
        assert_eq!(ledger.day_total_usd(chrono::Utc::now()), 0.0);
    }
}
