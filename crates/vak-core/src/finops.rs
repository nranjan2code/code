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
    /// Serving provider of the frozen-ladder leg (Phase R per-provider
    /// FinOps rollups). Empty on legacy rows.
    #[serde(default)]
    pub provider: String,
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

    /// Every row from the ledger, oldest first, for callers that need to
    /// bucket or roll them up themselves (admin console trend chart,
    /// provider/model breakdowns) rather than a single aggregate. Corrupt
    /// lines are skipped, same tolerance `total_usd_since` already has.
    pub fn all_rows(&self) -> Vec<CostRow> {
        let Ok(f) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<CostRow>(&line).ok())
            .collect()
    }

    /// One USD total per UTC calendar day, oldest first, for the trailing
    /// `days` days including today — a fixed-length series so a chart never
    /// has to guess whether a missing day means "no spend" or "no data
    /// yet"; both render as `0.0`.
    pub fn daily_totals(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        days: u32,
    ) -> Vec<(chrono::NaiveDate, f64)> {
        let today = now.date_naive();
        let mut totals: BTreeMap<chrono::NaiveDate, f64> = BTreeMap::new();
        for row in self.all_rows() {
            if let Some(usd) = row.usd {
                *totals.entry(row.ts.date_naive()).or_insert(0.0) += usd;
            }
        }
        (0..days)
            .rev()
            .filter_map(|offset| today.checked_sub_signed(chrono::Duration::days(offset as i64)))
            .map(|day| (day, totals.get(&day).copied().unwrap_or(0.0)))
            .collect()
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

    fn record_settled(&self, provider: &str, model: &str, session_id: &str, usage: &Usage) {
        let usd = self.estimate(model, usage);
        let row = CostRow {
            ts: chrono::Utc::now(),
            model: model.to_string(),
            provider: provider.to_string(),
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

// ---- Budget alerts (docs/design/29-personal-os.md P2) ----------------------

/// Proactive spend-alert thresholds, delivered to surfaces once per
/// threshold window instead of being discovered at denial time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Eighty,
    Full,
}

impl AlertLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertLevel::Eighty => "eighty",
            AlertLevel::Full => "full",
        }
    }
}

/// Which threshold `day_total_usd` has crossed against `cap`. Pure:
/// `>= 100%` → Full, `>= 80%` → Eighty, otherwise None. A non-positive or
/// non-finite cap means "no cap", which can never alert; a non-finite
/// total likewise.
pub fn alert_level(day_total_usd: f64, cap: f64) -> Option<AlertLevel> {
    if !cap.is_finite() || cap <= 0.0 || !day_total_usd.is_finite() {
        return None;
    }
    if day_total_usd >= cap {
        Some(AlertLevel::Full)
    } else if day_total_usd / cap >= 0.8 {
        Some(AlertLevel::Eighty)
    } else {
        None
    }
}

/// One audit-only budget-alert ledger row. Lives beside the cost log in
/// `<home>/budget-alerts.jsonl`; never enters session logs, so projections
/// are untouched. The `"kind"` tag mirrors session receipt entries so
/// ledger consumers discriminate rows uniformly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetAlertRow {
    #[serde(rename = "kind")]
    kind: String,
    pub ts: chrono::DateTime<chrono::Utc>,
    pub level: AlertLevel,
    /// Day spend as observed when the alert fired.
    pub day_total_usd: f64,
    pub session_id: String,
}

fn alerts_path(home: &std::path::Path) -> PathBuf {
    home.join("budget-alerts.jsonl")
}

/// Append an alert row for `level`, stamping it with the CURRENT day
/// spend from the cost ledger. Returns the row as written.
pub fn record_alert(
    home: &std::path::Path,
    level: AlertLevel,
    session_id: &str,
) -> std::io::Result<BudgetAlertRow> {
    std::fs::create_dir_all(home)?;
    let row = BudgetAlertRow {
        kind: "budget_alert".to_string(),
        ts: chrono::Utc::now(),
        level,
        day_total_usd: FinOpsLedger::new(home).day_total_usd(chrono::Utc::now()),
        session_id: session_id.to_string(),
    };
    let line = serde_json::to_string(&row)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(alerts_path(home))?;
    writeln!(f, "{line}")?;
    Ok(row)
}

/// Most recent recorded alert at exactly `level`, for once-per-window
/// firing decisions. Corrupt lines are skipped; a missing file is None.
pub fn last_alert(home: &std::path::Path, level: AlertLevel) -> Option<BudgetAlertRow> {
    let f = std::fs::File::open(alerts_path(home)).ok()?;
    let mut found = None;
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        if let Ok(row) = serde_json::from_str::<BudgetAlertRow>(&line)
            && row.kind == "budget_alert"
            && row.level == level
        {
            found = Some(row);
        }
    }
    found
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
            provider: "anthropic",
            session_id: "s1",
            est_input_tokens: 1_000_000,
            planned_output_tokens: 100_000,
        }
    }

    #[tokio::test]
    async fn unpriced_model_admits_and_records_unknown_usd() {
        let (gate, dir) = gate_with(&[("run", 0.01)]);
        gate.authorize(&check("mystery-model")).await.unwrap();
        gate.record_settled(
            "anthropic",
            "mystery-model",
            "s1",
            &usage(1_000_000, 100_000),
        );
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
        gate.record_settled(
            "anthropic",
            "claude-sonnet",
            "s1",
            &usage(1_000_000, 100_000),
        );
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
                provider: String::new(),
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
                provider: String::new(),
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

    fn row_on(day: chrono::NaiveDate, usd: Option<f64>) -> CostRow {
        CostRow {
            ts: day.and_hms_opt(12, 0, 0).unwrap().and_utc(),
            model: "m".into(),
            provider: "p".into(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_input_tokens: None,
            usd,
            source: "estimated".into(),
            session_id: "s".into(),
        }
    }

    /// The chart-feeding series: always exactly `days` entries, oldest
    /// first ending at today, zero-filled for a day with no rows — a
    /// sparse map would leave a chart guessing which days are "no spend"
    /// versus simply absent.
    #[test]
    fn daily_totals_is_fixed_length_and_zero_fills_gaps() {
        let dir = tempdir().unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        let now = chrono::Utc::now();
        let today = now.date_naive();
        let two_days_ago = today - chrono::Duration::days(2);
        ledger.append(&row_on(today, Some(3.0))).unwrap();
        ledger.append(&row_on(today, Some(1.5))).unwrap();
        ledger.append(&row_on(two_days_ago, Some(2.0))).unwrap();
        // Unpriced rows must not silently count as zero spend where a
        // priced row exists, nor crash the bucketing.
        ledger.append(&row_on(today, None)).unwrap();

        let series = ledger.daily_totals(now, 3);
        assert_eq!(series.len(), 3);
        assert_eq!(series[2].0, today);
        assert!((series[2].1 - 4.5).abs() < 1e-9, "{:?}", series[2]);
        assert_eq!(series[1].0, today - chrono::Duration::days(1));
        assert_eq!(series[1].1, 0.0, "a day with no rows must zero-fill");
        assert_eq!(series[0].0, two_days_ago);
        assert!((series[0].1 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn alert_level_threshold_matrix() {
        assert_eq!(alert_level(0.0, 10.0), None);
        assert_eq!(alert_level(7.9, 10.0), None);
        // Exactly at the thresholds fires.
        assert_eq!(alert_level(8.0, 10.0), Some(AlertLevel::Eighty));
        assert_eq!(alert_level(9.99, 10.0), Some(AlertLevel::Eighty));
        assert_eq!(alert_level(10.0, 10.0), Some(AlertLevel::Full));
        assert_eq!(alert_level(150.0, 10.0), Some(AlertLevel::Full));
        // No cap / nonsense inputs never alert.
        assert_eq!(alert_level(100.0, 0.0), None);
        assert_eq!(alert_level(100.0, -5.0), None);
        assert_eq!(alert_level(f64::NAN, 10.0), None);
    }

    #[test]
    fn record_and_last_alert_roundtrip_per_level() {
        let dir = tempdir().unwrap();
        let home = dir.path();

        assert_eq!(last_alert(home, AlertLevel::Eighty), None);

        let first = record_alert(home, AlertLevel::Eighty, "s1").unwrap();
        assert_eq!(first.kind, "budget_alert");
        assert_eq!(first.level, AlertLevel::Eighty);

        let full = record_alert(home, AlertLevel::Full, "s1").unwrap();
        let later = record_alert(home, AlertLevel::Eighty, "s2").unwrap();

        let eighty = last_alert(home, AlertLevel::Eighty).unwrap();
        assert_eq!(eighty.session_id, "s2");
        assert_eq!(eighty.ts, later.ts);
        let full_back = last_alert(home, AlertLevel::Full).unwrap();
        assert_eq!(full_back.ts, full.ts);
        assert_eq!(full_back.day_total_usd, full.day_total_usd);
        // Rows carry the current ledger day total (zero here).
        assert_eq!(first.day_total_usd, 0.0);
    }

    #[test]
    fn corrupt_or_foreign_lines_are_ignored_by_readback() {
        let dir = tempdir().unwrap();
        std::fs::write(
            alerts_path(dir.path()),
            concat!(
                "{not json\n",
                "{\"kind\":\"receipt\",\"other\":1}\n",
                "{\"kind\":\"budget_alert\",\"ts\":\"2026-08-24T00:00:00Z\",\"level\":\"eighty\",\"day_total_usd\":1.5,\"session_id\":\"seed\"}\n",
            ),
        )
        .unwrap();
        let found = last_alert(dir.path(), AlertLevel::Eighty).unwrap();
        assert_eq!(found.session_id, "seed");
        assert!((found.day_total_usd - 1.5).abs() < 1e-9);
        assert!(last_alert(dir.path(), AlertLevel::Full).is_none());
    }
}
