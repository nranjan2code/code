//! FinOps (docs/design/27 Phase D): persisted cost ledger + pre-dispatch
//! budget admission. Every settled dispatch appends an estimated-USD row
//! keyed by durable attribution ids; caps are checked BEFORE each paid
//! call. Denial is a question (budget Ask), not a crash; unattended
//! surfaces auto-deny via the normal approver path.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use vak_agent::{SpendCheck, SpendGate};
use vak_llm::Usage;

/// Above this size, `append` compacts the ledger before writing (see
/// [`FinOpsLedger::compact_if_large`]) instead of letting it grow forever —
/// every `authorize()` used to re-read the whole file from the start of
/// time on every paid dispatch, so an unbounded file meant unbounded
/// per-dispatch latency as well as unbounded disk use.
const COST_LOG_COMPACT_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;
/// How much history compaction keeps: comfortably more than the 14-day
/// admin trend chart and the `total_usd_since` callers in this codebase
/// use (7/30-day rollups), so compaction never changes a real answer.
const COST_LOG_RETENTION_DAYS: i64 = 90;
/// Same idea for the budget-alert log, which is read in full by
/// `last_alert`/`recent_budget_alerts` and only ever needs recent history.
const ALERTS_COMPACT_THRESHOLD_BYTES: u64 = 1024 * 1024;
const ALERTS_RETENTION_ROWS: usize = 2000;

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
        self.compact_if_large()?;
        let line = serde_json::to_string(row)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")
    }

    /// Bound the ledger's on-disk size: cheap to skip on every call (one
    /// `metadata()` stat), and only reads+rewrites the whole file the rare
    /// time it actually crosses the threshold. Drops rows older than
    /// [`COST_LOG_RETENTION_DAYS`]; never touches today's numbers.
    fn compact_if_large(&self) -> std::io::Result<()> {
        self.compact_if_larger_than(
            COST_LOG_COMPACT_THRESHOLD_BYTES,
            chrono::Duration::days(COST_LOG_RETENTION_DAYS),
        )
    }

    /// Parameterized so tests can exercise compaction without writing
    /// megabytes of fixture rows first.
    fn compact_if_larger_than(
        &self,
        threshold_bytes: u64,
        retention: chrono::Duration,
    ) -> std::io::Result<()> {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return Ok(());
        };
        if meta.len() < threshold_bytes {
            return Ok(());
        }
        let cutoff = chrono::Utc::now() - retention;
        let kept = self
            .all_rows()
            .into_iter()
            .filter(|r| r.ts >= cutoff)
            .collect::<Vec<_>>();
        let tmp = self.path.with_extension("jsonl.compact.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            for row in &kept {
                let line = serde_json::to_string(row)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                writeln!(f, "{line}")?;
            }
        }
        std::fs::rename(&tmp, &self.path)
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

/// Process-wide, cross-session admission state for the day cap, shared by
/// every [`CoreSpendGate`] built from the same `Core`. Closes two gaps a
/// gate-local `Mutex<f64>` can't: (1) a per-turn gate used to start the
/// run cap over from zero every turn (`Core::spend_gate_for` now hands
/// back the SAME gate for the life of a session instead, so `run_spent_usd`
/// finally means "this run", not "this turn"); (2) concurrent dispatches
/// (parallel tool calls, or multiple sessions sharing one `Core`) used to
/// all read the same stale on-disk day total and could jointly blow past
/// the cap before any of their rows landed — `reserved_usd` below is
/// credited at admission time, before the paid call happens, so a second
/// concurrent `authorize()` sees the first one's reservation immediately.
pub(crate) struct DayBudget {
    day: chrono::NaiveDate,
    /// Settled USD for `day`, read from the ledger once per day (not once
    /// per dispatch — this used to be a full-file re-scan on every single
    /// `authorize()` call).
    baseline_usd: f64,
    /// Admitted-but-not-yet-appended estimates for `day`. Rolled into
    /// `baseline_usd` (approximately — via the settled estimate, not the
    /// exact reservation) as each dispatch settles; a rollover to a new
    /// day always re-derives `baseline_usd` from the ledger, so any drift
    /// self-corrects at most once a day.
    reserved_usd: f64,
}

impl DayBudget {
    /// A tracker with a deliberately-stale sentinel day, so the first
    /// `roll()` always re-derives `baseline_usd` from the ledger.
    pub(crate) fn new() -> Self {
        DayBudget {
            day: chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or_default(),
            baseline_usd: 0.0,
            reserved_usd: 0.0,
        }
    }

    fn roll(&mut self, ledger: &FinOpsLedger, now: chrono::DateTime<chrono::Utc>) {
        let today = now.date_naive();
        if self.day != today {
            self.day = today;
            self.baseline_usd = ledger.day_total_usd(now);
            self.reserved_usd = 0.0;
        }
    }
}

/// Core-side SpendGate: run/day caps + pricing-driven estimates.
pub struct CoreSpendGate {
    ledger: FinOpsLedger,
    max_run_usd: Mutex<Option<f64>>,
    max_day_usd: Mutex<Option<f64>>,
    overrides: Mutex<BTreeMap<String, vak_config::PriceEntry>>,
    run_spent_usd: Mutex<f64>,
    raised_once: AtomicBool,
    day_budget: Arc<Mutex<DayBudget>>,
}

impl CoreSpendGate {
    pub fn new(sessions_home: &std::path::Path, finops: &vak_config::FinopsResolved) -> Self {
        Self::with_shared_day_budget(
            sessions_home,
            finops,
            Arc::new(Mutex::new(DayBudget::new())),
        )
    }

    /// Same as [`Self::new`] but sharing the day-cap admission state with
    /// every other gate built from the same `Core` (see [`DayBudget`]).
    /// `Core::spend_gate_for` is the only caller that needs this; direct
    /// `new` (tests, the one-off reflection gate) is fine with its own
    /// isolated tracker since nothing else observes it.
    pub(crate) fn with_shared_day_budget(
        sessions_home: &std::path::Path,
        finops: &vak_config::FinopsResolved,
        day_budget: Arc<Mutex<DayBudget>>,
    ) -> Self {
        CoreSpendGate {
            ledger: FinOpsLedger::new(sessions_home),
            max_run_usd: Mutex::new(finops.max_run_usd),
            max_day_usd: Mutex::new(finops.max_day_usd),
            overrides: Mutex::new(finops.price_overrides.clone()),
            run_spent_usd: Mutex::new(0.0),
            raised_once: AtomicBool::new(false),
            day_budget,
        }
    }

    /// Pick up a live `PATCH /finops` cap change on a gate that is being
    /// reused across turns (see [`DayBudget`] doc). Deliberately leaves
    /// `run_spent_usd`/`raised_once` untouched — a cap edit mid-run must
    /// not reset what's already been spent or re-arm a denial the
    /// approver already raised.
    pub(crate) fn refresh_caps(&self, finops: &vak_config::FinopsResolved) {
        *self
            .max_run_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = finops.max_run_usd;
        *self
            .max_day_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = finops.max_day_usd;
        *self
            .overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = finops.price_overrides.clone();
    }

    /// The approver answered "raise the cap for this run once".
    pub fn raise_once(&self) {
        self.raised_once.store(true, Ordering::SeqCst);
    }

    fn estimate(&self, model: &str, usage: &Usage) -> Option<f64> {
        let overrides = self
            .overrides
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        vak_config::finops::estimate_cost_usd(model, usage, &overrides)
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
        let max_run_usd = *self
            .max_run_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run_spent = *self
            .run_spent_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cap) = max_run_usd
            && !self.raised_once.load(Ordering::SeqCst)
            && run_spent + est > cap
        {
            return Err(format!(
                "run budget ${cap:.2} would be exceeded by this dispatch (+${est:.2}, ${run_spent:.2} already spent)"
            ));
        }
        let max_day_usd = *self
            .max_day_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cap) = max_day_usd {
            let mut day = self
                .day_budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            day.roll(&self.ledger, chrono::Utc::now());
            let projected = day.baseline_usd + day.reserved_usd + est;
            if projected > cap {
                return Err(format!(
                    "day budget ${cap:.2} would be exceeded by this dispatch (+${est:.2}, ${:.2} spent today)",
                    day.baseline_usd + day.reserved_usd
                ));
            }
            // Reserve immediately so a concurrent authorize() racing this
            // one sees the commitment before either dispatch settles.
            day.reserved_usd += est;
        }
        Ok(())
    }

    fn record_settled(&self, provider: &str, model: &str, session_id: &str, usage: &Usage) {
        let usd = self.estimate(model, usage);
        // Fail-closed accounting: a run/day cap must still hold even if
        // the ledger write below fails (e.g. disk full) — an I/O error
        // must not silently re-open the budget it was there to enforce.
        if let Some(usd) = usd {
            let mut spent = self
                .run_spent_usd
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *spent += usd;
            let mut day = self
                .day_budget
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            day.roll(&self.ledger, chrono::Utc::now());
            // Release this dispatch's reservation and fold the settled
            // amount into the baseline; clamp guards a same-day estimate
            // mismatch (planned vs. actual tokens) from going negative.
            day.reserved_usd = (day.reserved_usd - usd).max(0.0);
            day.baseline_usd += usd;
        }
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
            // The ledger write itself is still best-effort — the receipt
            // entries in the session log carry usage independently — but
            // the in-memory run/day counters above are already updated,
            // so caps stay enforced even when this fails.
            eprintln!("warning: cost ledger append failed: {e}");
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
    compact_alerts_if_large(home)?;
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

/// Same bounded-growth treatment as [`FinOpsLedger::compact_if_large`]:
/// alerts are only ever read for "most recent N", so unbounded history
/// buys nothing but disk and scan time.
fn compact_alerts_if_large(home: &std::path::Path) -> std::io::Result<()> {
    let path = alerts_path(home);
    let Ok(meta) = std::fs::metadata(&path) else {
        return Ok(());
    };
    if meta.len() < ALERTS_COMPACT_THRESHOLD_BYTES {
        return Ok(());
    }
    let Ok(f) = std::fs::File::open(&path) else {
        return Ok(());
    };
    let mut lines: Vec<String> = BufReader::new(f).lines().map_while(Result::ok).collect();
    if lines.len() <= ALERTS_RETENTION_ROWS {
        return Ok(());
    }
    let drop = lines.len() - ALERTS_RETENTION_ROWS;
    lines.drain(0..drop);
    let tmp = path.with_extension("jsonl.compact.tmp");
    std::fs::write(&tmp, lines.join("\n") + "\n")?;
    std::fs::rename(&tmp, &path)
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

    /// Audit fix: the ledger used to grow forever with no rotation, and
    /// every `authorize()` re-read the whole file from disk on every paid
    /// dispatch. Compaction should trim old rows once the file crosses
    /// its size threshold, and must never drop anything within the
    /// retention window.
    #[test]
    fn compaction_drops_only_rows_older_than_retention() {
        let dir = tempdir().unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::days(400);
        ledger.append(&row_on(old.date_naive(), Some(1.0))).unwrap();
        ledger.append(&row_on(now.date_naive(), Some(2.0))).unwrap();

        // Force compaction on the next append regardless of actual file
        // size, with a short retention window so the seeded old row falls
        // outside it.
        ledger
            .compact_if_larger_than(0, chrono::Duration::days(1))
            .unwrap();

        let rows = ledger.all_rows();
        assert_eq!(rows.len(), 1, "the old row must be dropped, not the fresh one");
        assert_eq!(rows[0].usd, Some(2.0));
        // Aggregates must be unaffected by compaction for anything still
        // within the retention window.
        assert_eq!(ledger.day_total_usd(now), 2.0);
    }

    #[test]
    fn compaction_is_a_noop_below_the_size_threshold() {
        let dir = tempdir().unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::days(400);
        ledger.append(&row_on(old.date_naive(), Some(1.0))).unwrap();

        // A generous threshold the tiny fixture file can never cross:
        // compaction must leave old-but-still-present rows alone.
        ledger
            .compact_if_larger_than(u64::MAX, chrono::Duration::days(1))
            .unwrap();
        assert_eq!(ledger.all_rows().len(), 1);
    }

    /// Audit fix: concurrent dispatches used to each read the same stale
    /// on-disk day total before any of them settled, so a burst could
    /// jointly blow past `max_day_usd`. `DayBudget::reserved_usd` credits
    /// an admission immediately so a second concurrent `authorize()` sees
    /// the first one's reservation before either settles.
    #[tokio::test]
    async fn concurrent_authorize_calls_cannot_jointly_exceed_the_day_cap() {
        let dir = tempdir().unwrap();
        let mut finops = vak_config::FinopsResolved::default();
        // sonnet est ~= $4.50/dispatch; cap admits exactly one.
        finops.max_day_usd = Some(5.0);
        let day_budget = Arc::new(Mutex::new(DayBudget::new()));
        let gate_a =
            CoreSpendGate::with_shared_day_budget(dir.path(), &finops, day_budget.clone());
        let gate_b = CoreSpendGate::with_shared_day_budget(dir.path(), &finops, day_budget);

        // Both "concurrent" calls check against the ledger before either
        // has appended anything — a stale-read race would admit both.
        let first = gate_a.authorize(&check("claude-sonnet")).await;
        let second = gate_b.authorize(&check("claude-sonnet")).await;
        assert!(first.is_ok(), "{first:?}");
        assert!(
            second.is_err(),
            "the second concurrent dispatch must see the first one's reservation"
        );
    }

    /// `record_settled` releases a dispatch's reservation and folds the
    /// settled amount into the day baseline; a same-day gate built after
    /// the first must see the earlier settlement.
    #[tokio::test]
    async fn record_settled_updates_the_shared_day_budget_for_later_gates() {
        let dir = tempdir().unwrap();
        let mut finops = vak_config::FinopsResolved::default();
        // sonnet est ~= $4.50/dispatch; cap admits exactly one, so a
        // gate that doesn't see gate_a's settlement would wrongly admit
        // a second one at ~$9.00 total.
        finops.max_day_usd = Some(5.0);
        let day_budget = Arc::new(Mutex::new(DayBudget::new()));
        let gate_a =
            CoreSpendGate::with_shared_day_budget(dir.path(), &finops, day_budget.clone());
        gate_a.authorize(&check("claude-sonnet")).await.unwrap();
        gate_a.record_settled(
            "anthropic",
            "claude-sonnet",
            "s1",
            &usage(1_000_000, 100_000),
        );

        // A later gate sharing the same tracker (e.g. the next turn's
        // session, or a different session on the same Core) must see
        // gate_a's settled spend, not a fresh zero.
        let gate_b = CoreSpendGate::with_shared_day_budget(dir.path(), &finops, day_budget);
        let err = gate_b
            .authorize(&check("claude-sonnet"))
            .await
            .expect_err("day cap must already reflect gate_a's settled spend");
        assert!(err.contains("day budget $5.00"), "{err}");
    }
}
