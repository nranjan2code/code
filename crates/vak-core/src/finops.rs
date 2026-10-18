//! FinOps (docs/design/15-reliability.md): persisted cost ledger + pre-dispatch
//! budget admission. Every settled dispatch appends an estimated-USD row
//! keyed by durable attribution ids; caps are checked BEFORE each paid
//! call. Denial is a question (budget Ask), not a crash; unattended
//! surfaces auto-deny via the normal approver path.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use vak_session::chain::RecordChain;
use vak_session::rollup::Rollup;

use serde::{Deserialize, Serialize};
use vak_agent::{SpendCheck, SpendGate, SpendReservationId};
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
    /// "estimated" for runtime pricing; "owner_verified" for an explicit
    /// owner-confirmed final charge or append-only unknown-cost correction.
    pub source: String,
    pub session_id: String,
    /// Exact admission that produced this row, allowing a crash after append
    /// but before reservation disposition to reconcile without double count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_id: Option<String>,
    /// Stable session whose lifetime cap owns this spend. Child dispatch rows
    /// keep their actual session and trace while charging their parent's gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingReservation {
    pub id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub estimate_usd: Option<f64>,
    pub budget_session_id: String,
    pub session_id: String,
    pub model: String,
    pub provider: String,
    pub trace: Option<vak_session::trace::TraceKey>,
}

vak_session::impl_traced!(CostRow, "cost_row");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityRow {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub kind: String,
    pub name: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// The tool call this row is about: the call itself, or the call a
    /// pre/post-tool hook ran for (docs/design/85-turn-graph.md, G0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(ActivityRow, "activity_row");

pub struct ActivityLedger {
    path: PathBuf,
}

impl ActivityLedger {
    pub fn new(sessions_home: &std::path::Path) -> Self {
        Self {
            path: vak_config::scope::AgentScope::new(sessions_home).activity_log(),
        }
    }

    pub fn append(&self, row: &ActivityRow) -> std::io::Result<()> {
        RecordChain::at(&self.path)
            .append_relaxed(row)
            .map_err(std::io::Error::other)
    }

    pub fn all_rows(&self) -> Vec<ActivityRow> {
        RecordChain::at(&self.path).read()
    }
}

pub struct FinOpsLedger {
    path: PathBuf,
    rollup_path: PathBuf,
    reservations_path: PathBuf,
}

impl FinOpsLedger {
    pub fn new(sessions_home: &std::path::Path) -> Self {
        let scope = vak_config::scope::AgentScope::new(sessions_home);
        FinOpsLedger {
            path: scope.cost_log(),
            rollup_path: scope.cost_rollup(),
            reservations_path: vak_config::scope::SharedScope::new(sessions_home)
                .cost_reservations(),
        }
    }

    /// Appends one row. The ledger is never rewritten: rows past their
    /// retention leave through the lifecycle reconciler (plan M7a), not a
    /// compaction pass here.
    pub fn append(&self, row: &CostRow) -> std::io::Result<()> {
        RecordChain::at(&self.path)
            .append(row)
            .map_err(std::io::Error::other)
    }

    /// USD total over rows at or after `since`. Unpriced rows contribute
    /// nothing but are never treated as evidence of zero spend elsewhere.
    pub fn total_usd_since(&self, since: chrono::DateTime<chrono::Utc>) -> f64 {
        let mut total = 0.0;
        RecordChain::at(&self.path).scan(|row: CostRow| {
            if row.ts >= since
                && let Some(usd) = row.usd
            {
                total += usd;
            }
            true
        });
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

    /// Live admission liabilities that still need an owner-verified outcome.
    /// Priced rows already in the strict cost rollup are omitted even if a
    /// prior cleanup write was interrupted.
    pub fn pending_reservations(&self) -> Result<Vec<PendingReservation>, String> {
        let state = self.read_reservations()?;
        let totals = self
            .strict_session_totals()
            .map_err(|_| "cost history could not be verified".to_string())?;
        let mut found = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for row in state.reservations {
            seen.insert(row.id.clone());
            if let Some(settled) = totals.reservations.get(&row.id) {
                if row.budget_session_id.is_empty() || settled.session_id != row.budget_session_id {
                    return Err("reservation settlement ownership could not be verified".into());
                }
                if settled.usd.is_some() {
                    continue;
                }
            }
            found.push(PendingReservation {
                id: row.id,
                created_at: row.created_at,
                estimate_usd: row.estimate_usd,
                budget_session_id: row.budget_session_id,
                session_id: row.session_id,
                model: if row.model.is_empty() {
                    "unknown".into()
                } else {
                    row.model
                },
                provider: row.provider,
                trace: row.trace,
            });
        }
        for (id, settled) in totals.reservations {
            if settled.usd.is_none() && !seen.contains(&id) {
                found.push(PendingReservation {
                    id,
                    created_at: settled.created_at,
                    estimate_usd: None,
                    budget_session_id: settled.session_id,
                    session_id: settled.original_session_id,
                    model: settled.model,
                    provider: settled.provider,
                    trace: settled.trace,
                });
            }
        }
        found.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(found)
    }

    /// Reconciles an unresolved admission with a dollar amount verified by
    /// the owner. The append is unique by reservation id under the cost-chain
    /// writer lock; cleanup follows only after that row is verified durable.
    pub fn reconcile_reservation(
        &self,
        reservation_id: &str,
        actual_usd: f64,
        actor: vak_session::ids::PrincipalId,
    ) -> Result<(), String> {
        if !valid_usd(actual_usd) {
            return Err("verified cost must be finite and non-negative".into());
        }
        let parsed = uuid::Uuid::parse_str(reservation_id)
            .map_err(|_| "reservation id is invalid".to_string())?;
        if parsed.get_version_num() != 7 || parsed.hyphenated().to_string() != reservation_id {
            return Err("reservation id is invalid".into());
        }

        let reservation = self
            .read_reservations()?
            .reservations
            .into_iter()
            .find(|row| row.id == reservation_id);
        loop {
            let (totals, at) = self
                .strict_cost_totals_at()
                .map_err(|_| "cost history could not be verified".to_string())?;
            if let Some(existing) = totals.reservations.get(reservation_id) {
                let expected_owner = reservation
                    .as_ref()
                    .map(|row| row.budget_session_id.as_str())
                    .filter(|owner| !owner.is_empty())
                    .unwrap_or(existing.session_id.as_str());
                if existing.session_id != expected_owner {
                    return Err("reservation already has a different verified outcome".into());
                }
                if existing.usd == Some(actual_usd)
                    && existing.source == "owner_verified"
                    && existing.actor == Some(actor)
                {
                    return match reservation.as_ref() {
                        Some(row) => self.remove_reconciled_reservation(row),
                        None => Ok(()),
                    };
                }
                if existing.usd.is_some() {
                    return Err("reservation already has a different verified outcome".into());
                }
            }
            let derived;
            let row = if let Some(row) = reservation.as_ref() {
                row
            } else if let Some(existing) = totals.reservations.get(reservation_id) {
                derived = DurableReservation {
                    id: reservation_id.to_string(),
                    created_at: existing.created_at,
                    day: existing.created_at.date_naive(),
                    estimate_usd: None,
                    budget_session_id: existing.session_id.clone(),
                    session_id: existing.original_session_id.clone(),
                    model: existing.model.clone(),
                    provider: existing.provider.clone(),
                    trace: existing.trace.clone(),
                    actor: existing.actor,
                };
                &derived
            } else {
                return Err("reservation was not found".into());
            };
            if row.budget_session_id.is_empty() {
                return Err("reservation has no verifiable budget owner".into());
            }
            let cost = CostRow {
                ts: chrono::Utc::now(),
                model: if row.model.is_empty() {
                    "unknown".into()
                } else {
                    row.model.clone()
                },
                provider: row.provider.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                usd: Some(actual_usd),
                source: "owner_verified".into(),
                session_id: if row.session_id.is_empty() {
                    row.budget_session_id.clone()
                } else {
                    row.session_id.clone()
                },
                reservation_id: Some(row.id.clone()),
                budget_session_id: Some(row.budget_session_id.clone()),
                trace: row.trace.clone(),
                actor: Some(actor),
            };
            if self
                .cost_chain()
                .append_all_after(at, &[cost])
                .map_err(|error| error.to_string())?
            {
                // Read the synchronized row back through the strict rollup;
                // cleanup cannot happen on an unverified append.
                continue;
            }
        }
    }

    fn read_reservations(&self) -> Result<DurableReservations, String> {
        let Some(raw) = vak_session::documents::read(&self.reservations_path)? else {
            return Ok(DurableReservations {
                schema: 1,
                reservations: Vec::new(),
            });
        };
        let state: DurableReservations = serde_json::from_str(&raw)
            .map_err(|_| "durable budget reservations are unreadable".to_string())?;
        if state.schema != 1 {
            return Err("durable budget reservations have an unsupported schema".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for row in &state.reservations {
            let parsed = uuid::Uuid::parse_str(&row.id)
                .map_err(|_| "durable budget reservation id is invalid".to_string())?;
            if parsed.get_version_num() != 7 || !ids.insert(row.id.as_str()) {
                return Err("durable budget reservation ids are invalid or duplicated".into());
            }
            if row.estimate_usd.is_some_and(|amount| !valid_usd(amount)) {
                return Err("durable budget reservation estimate is invalid".into());
            }
        }
        Ok(state)
    }

    fn remove_reconciled_reservation(&self, expected: &DurableReservation) -> Result<(), String> {
        vak_session::documents::update(&self.reservations_path, |current| {
            let Some(raw) = current else {
                return Ok(None);
            };
            let mut state: DurableReservations = serde_json::from_str(raw)
                .map_err(|_| "durable budget reservations are unreadable".to_string())?;
            if state.schema != 1 {
                return Err("durable budget reservations have an unsupported schema".into());
            }
            let Some(index) = state
                .reservations
                .iter()
                .position(|row| row.id == expected.id)
            else {
                return Ok(None);
            };
            let current = &state.reservations[index];
            if current.budget_session_id != expected.budget_session_id
                || current.session_id != expected.session_id
                || current.trace != expected.trace
            {
                return Err("reservation changed while its outcome was being recorded".into());
            }
            state.reservations.remove(index);
            let next = serde_json::to_string(&state)
                .map_err(|_| "durable budget reservations could not be encoded".to_string())?;
            Ok(Some((next, ())))
        })?;
        Ok(())
    }

    fn strict_cost_totals_at(
        &self,
    ) -> Result<(DailyCostTotals, vak_session::tail::Position), vak_session::types::SessionError>
    {
        Rollup::new(&self.path, &self.rollup_path).try_read_at(
            |totals: &mut DailyCostTotals, bytes| {
                let row: CostRow = serde_json::from_slice(bytes).map_err(|error| {
                    vak_session::types::SessionError::Corrupt {
                        line: 0,
                        message: error.to_string(),
                    }
                })?;
                if row.usd.is_some_and(|usd| !valid_usd(usd)) {
                    return Err(vak_session::types::SessionError::Corrupt {
                        line: 0,
                        message: "cost row USD must be finite and non-negative".into(),
                    });
                }
                if let Some(id) = &row.reservation_id {
                    let budget_session =
                        row.budget_session_id.as_deref().unwrap_or(&row.session_id);
                    let correction = if let Some(previous) = totals.reservations.get(id) {
                        if previous.usd.is_none()
                            && row.usd.is_some()
                            && row.source == "owner_verified"
                            && row.actor.is_some()
                            && previous.session_id == budget_session
                            && !previous.corrected
                        {
                            Some(previous.clone())
                        } else {
                            return Err(vak_session::types::SessionError::Corrupt {
                                line: 0,
                                message: "reservation id has multiple settled rows".into(),
                            });
                        }
                    } else {
                        None
                    };
                    if let Some(previous) = correction {
                        let old_day = totals.days.entry(previous.day).or_default();
                        old_day.unknown_calls = old_day.unknown_calls.saturating_sub(1);
                        let old_session = totals
                            .sessions
                            .entry(previous.session_id.clone())
                            .or_default();
                        old_session.unknown_calls = old_session.unknown_calls.saturating_sub(1);
                        totals.reservations.insert(
                            id.clone(),
                            SettledReservation {
                                session_id: budget_session.to_string(),
                                usd: row.usd,
                                source: row.source.clone(),
                                actor: row.actor,
                                day: row.ts.date_naive(),
                                corrected: true,
                                created_at: row.ts,
                                original_session_id: row.session_id.clone(),
                                model: row.model.clone(),
                                provider: row.provider.clone(),
                                trace: row.trace.clone(),
                            },
                        );
                    } else {
                        totals.reservations.insert(
                            id.clone(),
                            SettledReservation {
                                session_id: budget_session.to_string(),
                                usd: row.usd,
                                source: row.source.clone(),
                                actor: row.actor,
                                day: row.ts.date_naive(),
                                corrected: false,
                                created_at: row.ts,
                                original_session_id: row.session_id.clone(),
                                model: row.model.clone(),
                                provider: row.provider.clone(),
                                trace: row.trace.clone(),
                            },
                        );
                    }
                }
                let total = totals.days.entry(row.ts.date_naive()).or_default();
                if let Some(usd) = row.usd {
                    total.usd += usd;
                    if !total.usd.is_finite() {
                        return Err(vak_session::types::SessionError::Corrupt {
                            line: 0,
                            message: "daily cost total is not finite".into(),
                        });
                    }
                } else {
                    total.unknown_calls = total.unknown_calls.saturating_add(1);
                }
                let budget_session = row.budget_session_id.as_deref().unwrap_or(&row.session_id);
                let session = totals
                    .sessions
                    .entry(budget_session.to_string())
                    .or_default();
                match row.usd {
                    Some(usd) => {
                        session.usd += usd;
                        if !session.usd.is_finite() {
                            return Err(vak_session::types::SessionError::Corrupt {
                                line: 0,
                                message: "session cost total is not finite".into(),
                            });
                        }
                    }
                    None => session.unknown_calls = session.unknown_calls.saturating_add(1),
                }
                if row.budget_session_id.is_none()
                    && row.trace.as_ref().is_some_and(|trace| {
                        matches!(trace.cause, vak_session::trace::Cause::Delegation { .. })
                    })
                {
                    totals.unmapped_delegated = totals.unmapped_delegated.saturating_add(1);
                }
                Ok(())
            },
        )
    }

    fn strict_cost_totals(&self) -> Result<DailyCostTotals, vak_session::types::SessionError> {
        self.strict_cost_totals_at().map(|(totals, _)| totals)
    }

    fn strict_session_totals(&self) -> Result<DailyCostTotals, String> {
        self.strict_cost_totals().map_err(|error| error.to_string())
    }

    fn cost_chain(&self) -> RecordChain {
        RecordChain::at(&self.path)
    }

    /// Every row from the ledger, oldest first, for callers that need to
    /// bucket or roll them up themselves (admin console trend chart,
    /// provider/model breakdowns) rather than a single aggregate. Corrupt
    /// lines are skipped, same tolerance `total_usd_since` already has.
    pub fn all_rows(&self) -> Vec<CostRow> {
        RecordChain::at(&self.path).read()
    }

    /// Cost rows for summaries. A verified owner correction replaces its
    /// earlier unpriced row in this projection while preserving the original
    /// usage counts; `all_rows` remains the unmodified audit evidence.
    pub fn accounted_rows(&self) -> Result<Vec<CostRow>, String> {
        let rows = self
            .cost_chain()
            .try_read::<CostRow>()
            .map_err(|_| "cost history could not be verified".to_string())?;
        self.strict_cost_totals()
            .map_err(|_| "cost history could not be verified".to_string())?;
        let corrections: std::collections::BTreeMap<_, _> = rows
            .iter()
            .filter(|row| {
                row.reservation_id.is_some()
                    && row.source == "owner_verified"
                    && row.usd.is_some()
                    && row.actor.is_some()
            })
            .filter_map(|row| {
                row.reservation_id
                    .as_ref()
                    .map(|id| (id.clone(), row.clone()))
            })
            .collect();
        let mut output = Vec::with_capacity(rows.len());
        let mut emitted = std::collections::BTreeSet::new();
        for row in rows {
            let Some(id) = row.reservation_id.as_ref() else {
                output.push(row);
                continue;
            };
            if row.usd.is_none()
                && let Some(correction) = corrections.get(id)
            {
                if emitted.insert(id.clone()) {
                    let mut accounted = correction.clone();
                    accounted.input_tokens = row.input_tokens;
                    accounted.output_tokens = row.output_tokens;
                    accounted.cache_read_input_tokens = row.cache_read_input_tokens;
                    output.push(accounted);
                }
                continue;
            }
            if row.source == "owner_verified" && emitted.contains(id) {
                continue;
            }
            output.push(row);
        }
        Ok(output)
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

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DailyCostTotals {
    #[serde(default)]
    days: BTreeMap<chrono::NaiveDate, DailySpend>,
    #[serde(default)]
    sessions: BTreeMap<String, SessionSpend>,
    #[serde(default)]
    reservations: BTreeMap<String, SettledReservation>,
    #[serde(default)]
    unmapped_delegated: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DailySpend {
    #[serde(default)]
    usd: f64,
    #[serde(default)]
    unknown_calls: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SessionSpend {
    #[serde(default)]
    usd: f64,
    #[serde(default)]
    unknown_calls: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettledReservation {
    session_id: String,
    usd: Option<f64>,
    source: String,
    actor: Option<vak_session::ids::PrincipalId>,
    day: chrono::NaiveDate,
    corrected: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    original_session_id: String,
    model: String,
    provider: String,
    trace: Option<vak_session::trace::TraceKey>,
}

fn valid_usd(usd: f64) -> bool {
    usd.is_finite() && usd >= 0.0
}

/// What one Agent or one run spent, summed from cost rows. A row with no
/// price adds to `unknown_calls`, never to `usd` (absent is unknown).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CostRollup {
    pub key: String,
    pub usd: f64,
    pub calls: u64,
    pub unknown_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
}

fn rollup_by<'a>(
    rows: impl IntoIterator<Item = &'a CostRow>,
    key: impl Fn(&CostRow) -> Option<String>,
) -> Vec<CostRollup> {
    let mut by_key: BTreeMap<String, CostRollup> = BTreeMap::new();
    for row in rows {
        let Some(key) = key(row) else { continue };
        let entry = by_key.entry(key.clone()).or_insert_with(|| CostRollup {
            key,
            ..CostRollup::default()
        });
        entry.calls += 1;
        match row.usd {
            Some(usd) => entry.usd += usd,
            None => entry.unknown_calls += 1,
        }
        entry.input_tokens += row.input_tokens;
        entry.output_tokens += row.output_tokens;
        entry.cache_read_tokens += row.cache_read_input_tokens.unwrap_or(0);
    }
    let mut out: Vec<CostRollup> = by_key.into_values().collect();
    out.sort_by(|a, b| b.usd.total_cmp(&a.usd).then_with(|| a.key.cmp(&b.key)));
    out
}

/// Spend per Agent, keyed by the trace key's Agent id. Rows written without
/// a key are grouped under `unattributed` rather than dropped.
pub fn rollup_by_agent<'a>(rows: impl IntoIterator<Item = &'a CostRow>) -> Vec<CostRollup> {
    rollup_by(rows, |row| {
        Some(
            row.trace
                .as_ref()
                .map_or_else(|| "unattributed".to_string(), |t| t.agent.to_string()),
        )
    })
}

/// Spend per run, keyed by the trace key's run id. A row with no key belongs
/// to no run and is left out.
pub fn rollup_by_run<'a>(rows: impl IntoIterator<Item = &'a CostRow>) -> Vec<CostRollup> {
    rollup_by(rows, |row| row.trace.as_ref().map(|t| t.run.to_string()))
}

/// Process-wide lock shared by gates for the same data home. Durable day
/// reservations live in the shared Reservations Document; this lock only
/// serializes this process while it updates its run-local counters.
pub(crate) struct DayBudget;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableReservations {
    schema: u32,
    reservations: Vec<DurableReservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableReservation {
    id: String,
    created_at: chrono::DateTime<chrono::Utc>,
    day: chrono::NaiveDate,
    estimate_usd: Option<f64>,
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    budget_session_id: String,
    trace: Option<vak_session::trace::TraceKey>,
    actor: Option<vak_session::ids::PrincipalId>,
}

struct SpendReservation {
    estimated_usd: Option<f64>,
    budget_session_id: String,
    trace: Option<vak_session::trace::TraceKey>,
    actor: Option<vak_session::ids::PrincipalId>,
    terminal: bool,
}

struct CostSettlement<'a> {
    provider: &'a str,
    model: &'a str,
    session_id: &'a str,
    usage: &'a Usage,
    latency_ms: u64,
    usd: Option<f64>,
    trace: Option<vak_session::trace::TraceKey>,
    actor: Option<vak_session::ids::PrincipalId>,
    reservation_id: Option<String>,
    budget_session_id: Option<String>,
}

impl DayBudget {
    pub(crate) fn new() -> Self {
        DayBudget
    }
}

/// Day-cap admission is shared by every Core operating on the same data
/// home in this process. Weak entries avoid keeping abandoned homes alive.
pub(crate) fn shared_day_budget(data_home: &Path) -> Arc<Mutex<DayBudget>> {
    static BUDGETS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<DayBudget>>>>> = OnceLock::new();
    let root = std::fs::canonicalize(data_home).unwrap_or_else(|_| {
        if data_home.is_absolute() {
            data_home.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(data_home)
        }
    });
    let registry = BUDGETS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut budgets = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    budgets.retain(|_, budget| budget.strong_count() > 0);
    if let Some(budget) = budgets.get(&root).and_then(Weak::upgrade) {
        return budget;
    }
    let budget = Arc::new(Mutex::new(DayBudget::new()));
    budgets.insert(root, Arc::downgrade(&budget));
    budget
}

/// Core-side SpendGate: run/day caps + pricing-driven estimates.
pub struct CoreSpendGate {
    ledger: FinOpsLedger,
    settings: Mutex<vak_config::FinopsResolved>,
    run_spent_usd: Mutex<f64>,
    /// Estimated cost admitted but not yet settled. Without this reservation,
    /// concurrent dispatches can all pass against the same stale run total.
    run_reserved_usd: Mutex<f64>,
    /// Active or unsettled calls whose price was unknown at admission.
    run_unknown_reservations: Mutex<u64>,
    /// Exact estimates for dispatches admitted through the identity-bearing
    /// API. Unknown dispatched outcomes deliberately remain in these maps.
    reservations: Mutex<BTreeMap<SpendReservationId, SpendReservation>>,
    /// IDs created by the legacy identity-free `authorize` API. Usage-only
    /// settlement releases the smallest outstanding estimate, conservatively
    /// preserving larger concurrent reservations whose identity is unknown.
    legacy_reservations: Mutex<VecDeque<SpendReservationId>>,
    raised_once: AtomicBool,
    day_budget: Arc<Mutex<DayBudget>>,
    /// The trace of the turn now spending, stamped onto the rows it writes.
    trace: Mutex<Option<vak_session::trace::TraceKey>>,
    budget_session_id: Mutex<Option<String>>,
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
    /// every other gate built for the same data home (see [`DayBudget`]).
    /// Core-owned gates use the process-wide registry; direct
    /// `new` (tests, the one-off reflection gate) is fine with its own
    /// isolated tracker since nothing else observes it.
    pub(crate) fn with_shared_day_budget(
        sessions_home: &std::path::Path,
        finops: &vak_config::FinopsResolved,
        day_budget: Arc<Mutex<DayBudget>>,
    ) -> Self {
        #[cfg(test)]
        let _isolated_home = vak_config::paths::isolate_home_for_tests();
        CoreSpendGate {
            ledger: FinOpsLedger::new(sessions_home),
            trace: Mutex::new(None),
            budget_session_id: Mutex::new(None),
            settings: Mutex::new(finops.clone()),
            run_spent_usd: Mutex::new(0.0),
            run_reserved_usd: Mutex::new(0.0),
            run_unknown_reservations: Mutex::new(0),
            reservations: Mutex::new(BTreeMap::new()),
            legacy_reservations: Mutex::new(VecDeque::new()),
            raised_once: AtomicBool::new(false),
            day_budget,
        }
    }

    pub(crate) fn bind_budget_session(&self, session_id: &str) {
        *self
            .budget_session_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session_id.to_string());
    }

    /// Pick up a live `PATCH /finops` cap change on a gate that is being
    /// reused across turns (see [`DayBudget`] doc). Deliberately leaves
    /// `run_spent_usd`/`raised_once` untouched — a cap edit mid-run must
    /// not reset what's already been spent or re-arm a denial the
    /// approver already raised.
    pub(crate) fn refresh_caps(&self, finops: &vak_config::FinopsResolved) {
        *self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = finops.clone();
    }

    /// The approver answered "raise the cap for this run once".
    pub fn raise_once(&self) {
        self.raised_once.store(true, Ordering::SeqCst);
    }

    /// Lower the run cap to `cap` for this gate's session, never raise it.
    ///
    /// An envelope's lifetime spend limit reaches the turn through here:
    /// the configured `max_run_usd` and the engagement's `spend_ceiling_usd`
    /// meet, and the smaller governs.
    pub fn narrow_run_cap(&self, cap: f64) {
        let mut settings = self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        settings.max_run_usd = Some(
            settings
                .max_run_usd
                .map_or(cap, |existing| existing.min(cap)),
        );
    }

    /// Estimated cost of `usage` on `model` under the configured prices,
    /// or `None` when the model's price is unknown.
    pub fn estimate_usd(&self, model: &str, usage: &Usage) -> Option<f64> {
        self.estimate(model, usage)
    }

    fn estimate(&self, model: &str, usage: &Usage) -> Option<f64> {
        let settings = self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        vak_config::finops::estimate_cost_usd(model, usage, &settings.price_overrides)
    }

    fn admit(&self, check: &SpendCheck<'_>) -> Result<SpendReservationId, String> {
        self.admit_at(check, chrono::Utc::now())
    }

    fn admit_at(
        &self,
        check: &SpendCheck<'_>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<SpendReservationId, String> {
        let planned = Usage {
            input_tokens: check.est_input_tokens,
            output_tokens: check.planned_output_tokens,
            ..Default::default()
        };
        // Admission must use caps and prices from one policy snapshot. A
        // concurrent refresh must not combine an old cheap price with a new
        // high cap when both complete policies would deny the dispatch.
        let (max_run, max_day, quoted) = {
            let settings = self
                .settings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                settings.max_run_usd,
                settings.max_day_usd,
                vak_config::finops::estimate_cost_usd(
                    check.model,
                    &planned,
                    &settings.price_overrides,
                ),
            )
        };
        let estimate = match quoted {
            Some(value) if valid_usd(value) => Some(value),
            Some(_) => return Err("cannot enforce dollar budget: estimated cost is invalid".into()),
            None if max_run.is_some() || max_day.is_some() => {
                return Err(format!(
                    "cannot enforce dollar budget: model '{}' has unknown pricing; configure a price override before dispatch",
                    check.model
                ));
            }
            None => None,
        };
        let _day = self
            .day_budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run_spent = *self
            .run_spent_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut run_reserved = self
            .run_reserved_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let run_cap_enabled = max_run.is_some() && !self.raised_once.load(Ordering::SeqCst);
        // The verified ledger is the durable authority. Keep local failed-
        // write liabilities conservative too, including when both durable
        // settlement writes failed and the store becomes writable again.
        if let Some(cap) = max_run.filter(|_| run_cap_enabled) {
            let unknown = *self
                .run_unknown_reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if unknown > 0 || estimate.is_none() {
                return Err(format!(
                    "cannot enforce run budget ${cap:.2}: an active call has unknown pricing"
                ));
            }
            let estimate = estimate.unwrap_or_default();
            if run_spent + *run_reserved + estimate > cap {
                return Err(format!(
                    "run budget ${cap:.2} would be exceeded by this dispatch (+${estimate:.2}, ${run_spent:.2} spent and ${:.2} reserved)",
                    *run_reserved
                ));
            }
        }
        let id = SpendReservationId::new();
        let trace = check.trace.cloned().or_else(|| self.current_trace().0);
        let actor = trace.as_ref().and_then(|trace| trace.actor);
        let budget_session_id = self
            .budget_session_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| check.session_id.to_string());
        if let Err(error) = self.reserve_durable(
            DurableReservation {
                id: id.as_str(),
                created_at: now,
                day: now.date_naive(),
                estimate_usd: estimate,
                session_id: check.session_id.to_string(),
                model: check.model.to_string(),
                provider: check.provider.to_string(),
                budget_session_id: budget_session_id.clone(),
                trace: trace.clone(),
                actor,
            },
            max_day,
            max_run.filter(|_| run_cap_enabled),
        ) {
            let history_unverified = error == "cost history could not be verified";
            let budget_limit = error.starts_with("day budget ")
                || error.starts_with("cannot enforce day budget ")
                || error.starts_with("run budget ")
                || error.starts_with("cannot enforce run budget ");
            tracing::warn!(error_kind = %vak_telemetry::error_kind(&std::io::Error::other(error.clone())), "durable budget admission failed");
            if budget_limit {
                // These messages are assembled only from the configured cap
                // and measured amounts inside this module.
                return Err(error);
            }
            if (max_day.is_some() || max_run.filter(|_| run_cap_enabled).is_some())
                && history_unverified
            {
                return Err("cannot enforce day budget: cost history could not be verified; dispatch denied".into());
            }
            return Err("cannot enforce budget: durable reservation could not be verified or saved; dispatch denied".into());
        }
        self.reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id,
                SpendReservation {
                    estimated_usd: estimate,
                    budget_session_id: budget_session_id.clone(),
                    trace,
                    actor,
                    terminal: false,
                },
            );
        if let Some(estimate) = estimate {
            *run_reserved += estimate;
        } else {
            *self
                .run_unknown_reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
        }
        Ok(id)
    }

    /// Name the turn that is about to spend. A gate outlives its turns, so
    /// each one sets its own key before it dispatches.
    pub(crate) fn set_trace(&self, trace: Option<vak_session::trace::TraceKey>) {
        *self
            .trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = trace;
    }

    fn current_trace(
        &self,
    ) -> (
        Option<vak_session::trace::TraceKey>,
        Option<vak_session::ids::PrincipalId>,
    ) {
        let trace = self
            .trace
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let actor = trace.as_ref().and_then(|t| t.actor);
        (trace, actor)
    }

    fn reserve_durable(
        &self,
        record: DurableReservation,
        day_cap: Option<f64>,
        run_cap: Option<f64>,
    ) -> Result<(), String> {
        let estimate_usd = record.estimate_usd;
        if estimate_usd.is_some_and(|usd| !valid_usd(usd)) {
            return Err("cannot enforce budget: estimated cost is invalid".into());
        }
        let now = record.created_at;
        let budget_session_id = record.budget_session_id.as_str();
        let id_text = record.id.clone();
        vak_session::documents::update(&self.ledger.reservations_path, |current| {
            let mut state = match current {
                Some(raw) => serde_json::from_str::<DurableReservations>(raw)
                    .map_err(|_| "durable budget reservations are unreadable".to_string())?,
                None => DurableReservations {
                    schema: 1,
                    reservations: Vec::new(),
                },
            };
            if state.schema != 1 {
                return Err("durable budget reservations have an unsupported schema".into());
            }
            if state.reservations.iter().any(|entry| entry.id == id_text) {
                return Err("budget reservation id already exists".into());
            }
            // Every CAS retry reaches this closure again, so both the strict
            // cost history and the full unresolved liability set are fresh.
            let totals = if day_cap.is_some() || run_cap.is_some() {
                Some(
                    self.ledger
                        .strict_session_totals()
                        .map_err(|_| "cost history could not be verified".to_string())?,
                )
            } else {
                None
            };
            if let Some(totals) = &totals {
                // A priced settlement is the durable acknowledgement of this
                // exact admission. Finish its interrupted cleanup in the same
                // CAS that publishes the new admission. Never infer an
                // outcome, expire a liability, or remove an unknown charge.
                for pending in &state.reservations {
                    if let Some(settled) = totals.reservations.get(&pending.id)
                        && (pending.budget_session_id.is_empty()
                            || settled.session_id != pending.budget_session_id)
                    {
                        return Err("reservation settlement ownership could not be verified".into());
                    }
                }
                state.reservations.retain(|pending| {
                    !totals
                        .reservations
                        .get(&pending.id)
                        .is_some_and(|settled| settled.usd.is_some())
                });
            }
            let total = if let Some(_cap) = day_cap {
                let day = totals
                    .as_ref()
                    .and_then(|totals| totals.days.get(&now.date_naive()))
                    .cloned()
                    .unwrap_or_default();
                if day.unknown_calls > 0 {
                    return Err(
                        "cannot enforce day budget: current day contains unpriced cost rows".into(),
                    );
                }
                day.usd
            } else {
                0.0
            };
            let mut reserved = 0.0;
            for entry in &state.reservations {
                let Some(amount) = entry.estimate_usd else {
                    if day_cap.is_some() {
                        return Err(
                            "cannot enforce day budget: an unsettled call has unknown pricing"
                                .into(),
                        );
                    }
                    continue;
                };
                if !valid_usd(amount) {
                    return Err("durable budget reservations contain an invalid estimate".into());
                }
                if let Some(row) = totals
                    .as_ref()
                    .and_then(|totals| totals.reservations.get(&entry.id))
                {
                    if row.usd.is_none() {
                        if day_cap.is_some() {
                            return Err(
                                "cannot enforce day budget: a settled call has unknown pricing"
                                    .into(),
                            );
                        }
                    } else {
                        // A row appended before the reservation Document was
                        // settled is already in `total`; retain only any
                        // estimate excess so the liability is max(estimate,
                        // actual), never their sum.
                        reserved += (amount - row.usd.unwrap_or_default()).max(0.0);
                    }
                } else {
                    reserved += amount;
                }
            }
            if !reserved.is_finite() {
                return Err("durable budget reservation total is invalid".into());
            }
            if let Some(cap) = day_cap {
                let Some(estimate) = estimate_usd else {
                    return Err(format!(
                        "cannot enforce day budget ${cap:.2}: this call has unknown pricing"
                    ));
                };
                let projected = total + reserved + estimate;
                if !valid_usd(projected) || projected > cap {
                    return Err(format!(
                        "day budget ${cap:.2} would be exceeded by this dispatch (+${estimate:.2}, ${:.2} spent or reserved)",
                        total + reserved
                    ));
                }
            }
            if let Some(cap) = run_cap {
                let Some(totals) = totals.as_ref() else {
                    return Err("cost history could not be verified".into());
                };
                if totals.unmapped_delegated > 0 {
                    return Err("cannot enforce run budget: historical delegated spend cannot be attributed to this session".into());
                }
                let history = totals
                    .sessions
                    .get(budget_session_id)
                    .cloned()
                    .unwrap_or_default();
                if history.unknown_calls > 0 {
                    return Err(format!(
                        "cannot enforce run budget ${cap:.2}: this session has unknown settled spend"
                    ));
                }
                if state
                    .reservations
                    .iter()
                    .any(|entry| entry.budget_session_id.is_empty())
                {
                    return Err(format!(
                        "cannot enforce run budget ${cap:.2}: an unsettled legacy reservation has no session attribution"
                    ));
                }
                let settled = history.usd;
                let mut run_reserved = 0.0;
                for entry in state
                    .reservations
                    .iter()
                    .filter(|entry| entry.budget_session_id == budget_session_id)
                {
                    let Some(amount) = entry.estimate_usd else {
                        return Err(format!(
                            "cannot enforce run budget ${cap:.2}: an unsettled call has unknown pricing"
                        ));
                    };
                    if !valid_usd(amount) {
                        return Err("durable budget reservation total is invalid".into());
                    }
                    if let Some(row) = totals.reservations.get(&entry.id) {
                        if row.session_id != budget_session_id {
                            return Err(
                                "reservation settlement is attributed to another budget session"
                                    .into(),
                            );
                        }
                        if row.usd.is_none() {
                            return Err(format!(
                                "cannot enforce run budget ${cap:.2}: this session has unknown settled spend"
                            ));
                        }
                        // The rollup total already contains this actual row;
                        // retain any estimate excess to protect the larger
                        // of estimated and actual cost across a crash.
                        run_reserved += (amount - row.usd.unwrap_or_default()).max(0.0);
                        continue;
                    }
                    run_reserved += amount;
                }
                let Some(estimate) = estimate_usd else {
                    return Err(format!(
                        "cannot enforce run budget ${cap:.2}: this call has unknown pricing"
                    ));
                };
                let projected = settled + run_reserved + estimate;
                if !valid_usd(projected) || projected > cap {
                    return Err(format!(
                        "run budget ${cap:.2} would be exceeded by this dispatch (+${estimate:.2}, ${settled:.2} spent and ${run_reserved:.2} reserved)"
                    ));
                }
            }
            state.reservations.push(record.clone());
            let next = serde_json::to_string(&state)
                .map_err(|_| "durable budget reservations could not be encoded".to_string())?;
            Ok(Some((next, ())))
        })?;
        Ok(())
    }

    fn remove_durable(&self, id: SpendReservationId) -> Result<(), String> {
        let id_text = id.as_str();
        vak_session::documents::update(&self.ledger.reservations_path, |current| {
            let Some(current) = current else {
                return Ok(None);
            };
            let mut state: DurableReservations = serde_json::from_str(current)
                .map_err(|_| "durable budget reservations are unreadable".to_string())?;
            if state.schema != 1 {
                return Err("durable budget reservations have an unsupported schema".into());
            }
            let before = state.reservations.len();
            state.reservations.retain(|entry| entry.id != id_text);
            if state.reservations.len() == before {
                return Ok(None);
            }
            let next = serde_json::to_string(&state)
                .map_err(|_| "durable budget reservations could not be encoded".to_string())?;
            Ok(Some((next, ())))
        })?;
        Ok(())
    }

    fn update_durable_estimate(
        &self,
        id: SpendReservationId,
        estimate_usd: Option<f64>,
    ) -> Result<(), String> {
        let id_text = id.as_str();
        vak_session::documents::update(&self.ledger.reservations_path, |current| {
            let Some(current) = current else {
                return Err("durable budget reservation is missing".into());
            };
            let mut state: DurableReservations = serde_json::from_str(current)
                .map_err(|_| "durable budget reservations are unreadable".to_string())?;
            if state.schema != 1 {
                return Err("durable budget reservations have an unsupported schema".into());
            }
            let Some(record) = state
                .reservations
                .iter_mut()
                .find(|entry| entry.id == id_text)
            else {
                return Err("durable budget reservation is missing".into());
            };
            record.estimate_usd = estimate_usd;
            let next = serde_json::to_string(&state)
                .map_err(|_| "durable budget reservations could not be encoded".to_string())?;
            Ok(Some((next, ())))
        })?;
        Ok(())
    }

    fn settle_durable(&self, id: SpendReservationId, actual: Option<f64>) -> Result<(), String> {
        let id_text = id.as_str();
        vak_session::documents::update(&self.ledger.reservations_path, |current| {
            let Some(current) = current else {
                return Err("durable budget reservations are missing".into());
            };
            let mut state: DurableReservations = serde_json::from_str(current)
                .map_err(|_| "durable budget reservations are unreadable".to_string())?;
            if state.schema != 1 {
                return Err("durable budget reservations have an unsupported schema".into());
            }
            let Some(_reservation) = state
                .reservations
                .iter()
                .find(|entry| entry.id == id_text)
                .cloned()
            else {
                return Err("durable budget reservation is missing".into());
            };
            if actual.is_some_and(|usd| !valid_usd(usd)) {
                return Err("settled cost is invalid".into());
            }
            if actual.is_some() {
                state.reservations.retain(|entry| entry.id != id_text);
            } else if let Some(reservation) = state
                .reservations
                .iter_mut()
                .find(|entry| entry.id == id_text)
            {
                // Keep unknown outcomes owner-reconcilable after restart.
                reservation.estimate_usd = None;
            }
            let next = serde_json::to_string(&state)
                .map_err(|_| "durable budget reservations could not be encoded".to_string())?;
            Ok(Some((next, ())))
        })?;
        Ok(())
    }

    fn append_settled_row(&self, settlement: &CostSettlement<'_>) -> Result<(), std::io::Error> {
        let row = CostRow {
            ts: chrono::Utc::now(),
            model: settlement.model.to_string(),
            provider: settlement.provider.to_string(),
            input_tokens: settlement.usage.input_tokens,
            output_tokens: settlement.usage.output_tokens,
            cache_read_input_tokens: settlement.usage.cache_read_input_tokens,
            usd: settlement.usd,
            source: "estimated".to_string(),
            session_id: settlement.session_id.to_string(),
            trace: settlement.trace.clone(),
            actor: settlement.actor,
            reservation_id: settlement.reservation_id.clone(),
            budget_session_id: settlement.budget_session_id.clone(),
        };
        self.ledger.append(&row)?;
        let _ = ActivityLedger::new(
            self.ledger
                .path
                .parent()
                .unwrap_or(self.ledger.path.as_path()),
        )
        .append(&ActivityRow {
            ts: chrono::Utc::now(),
            kind: "provider".into(),
            name: format!("{}/{}", settlement.provider, settlement.model),
            success: true,
            duration_ms: (settlement.latency_ms > 0).then_some(settlement.latency_ms),
            session_id: Some(settlement.session_id.to_string()),
            plugin: None,
            tool_use_id: None,
            actor: settlement.actor,
            trace: settlement.trace.clone(),
        });
        Ok(())
    }
}

#[async_trait::async_trait]
impl SpendGate for CoreSpendGate {
    async fn authorize(&self, check: &SpendCheck<'_>) -> Result<(), String> {
        let id = self.admit(check)?;
        self.legacy_reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(id);
        Ok(())
    }

    async fn reserve_dispatch(
        &self,
        check: &SpendCheck<'_>,
    ) -> Result<Option<SpendReservationId>, String> {
        let id = self.admit(check)?;
        Ok(Some(id))
    }

    fn settle_dispatch(
        &self,
        id: SpendReservationId,
        provider: &str,
        model: &str,
        session_id: &str,
        usage: &Usage,
        latency_ms: u64,
    ) {
        let _day = self
            .day_budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut reservations = self
            .reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(reservation) = reservations.get_mut(&id) else {
            return;
        };
        if reservation.terminal {
            return;
        }
        let estimated = reservation.estimated_usd;
        let actual = self.estimate(model, usage).filter(|usd| valid_usd(*usd));
        let trace = reservation.trace.clone();
        let actor = reservation.actor;
        let budget_session_id = reservation.budget_session_id.clone();
        let appended = self.append_settled_row(&CostSettlement {
            provider,
            model,
            session_id,
            usage,
            latency_ms,
            usd: actual,
            trace,
            actor,
            reservation_id: Some(id.as_str().to_string()),
            budget_session_id: Some(budget_session_id),
        });
        let durable_settled = appended.is_ok() && self.settle_durable(id, actual).is_ok();
        let retain = !durable_settled;
        if retain {
            let retained = match (estimated, actual) {
                (Some(estimate), Some(actual)) => Some(estimate.max(actual)),
                (None, Some(actual)) => Some(actual),
                // Unknown settlement is durable unknown, even if an estimate
                // existed at admission. Keep the in-memory estimate below as
                // a conservative current-process check, but do not let a
                // restart mistake the stale estimate for a settled amount.
                (_, None) => None,
            };
            reservation.estimated_usd = retained;
            reservation.terminal = true;
            if actual.is_none() && estimated.is_some() {
                *self
                    .run_unknown_reservations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            }
            match (estimated, retained) {
                (Some(estimate), Some(retained)) => {
                    *self
                        .run_reserved_usd
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) += retained - estimate;
                }
                (None, Some(retained)) => {
                    *self
                        .run_unknown_reservations
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) -= 1;
                    *self
                        .run_reserved_usd
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) += retained;
                }
                _ => {}
            }
            if (appended.is_err() || !durable_settled)
                && let Err(error) = self.update_durable_estimate(id, retained)
            {
                tracing::warn!(error_kind = %vak_telemetry::error_kind(&std::io::Error::other(error)), "durable budget reservation update failed");
            }
            if let Err(error) = appended {
                tracing::warn!(error_kind = %vak_telemetry::error_kind(&error), "the cost ledger append failed");
            }
            return;
        }
        let actual = actual.unwrap_or_default();
        reservation.terminal = true;
        if let Some(estimated) = estimated {
            let mut reserved = self
                .run_reserved_usd
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *reserved = (*reserved - estimated).max(0.0);
        } else {
            *self
                .run_unknown_reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) -= 1;
        }
        *self
            .run_spent_usd
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) += actual;
        reservations.remove(&id);
    }

    fn release_dispatch(&self, id: SpendReservationId) {
        let _day = self
            .day_budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut reservations = self
            .reservations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(reservation) = reservations.get(&id) else {
            return;
        };
        if reservation.terminal {
            return;
        }
        if let Err(error) = self.remove_durable(id) {
            tracing::warn!(error_kind = %vak_telemetry::error_kind(&std::io::Error::other(error)), "durable budget reservation release failed");
            return;
        }
        let Some(reservation) = reservations.remove(&id) else {
            return;
        };
        if let Some(estimated) = reservation.estimated_usd {
            let mut reserved = self
                .run_reserved_usd
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *reserved = (*reserved - estimated).max(0.0);
        } else {
            *self
                .run_unknown_reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) -= 1;
        }
    }

    fn record_settled(&self, provider: &str, model: &str, session_id: &str, usage: &Usage) {
        self.record_settled_with_latency(provider, model, session_id, usage, 0);
    }

    fn record_settled_with_latency(
        &self,
        provider: &str,
        model: &str,
        session_id: &str,
        usage: &Usage,
        latency_ms: u64,
    ) {
        let id = {
            let mut legacy = self
                .legacy_reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let reservations = self
                .reservations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let selected = legacy
                .iter()
                .enumerate()
                .filter_map(|(index, id)| {
                    reservations
                        .get(id)
                        .filter(|reservation| !reservation.terminal)
                        .and_then(|reservation| {
                            reservation.estimated_usd.map(|amount| (index, *id, amount))
                        })
                })
                .min_by(|a, b| a.2.total_cmp(&b.2));
            selected.map(|(index, id, _)| {
                legacy.remove(index);
                id
            })
        };
        if let Some(id) = id {
            self.settle_dispatch(id, provider, model, session_id, usage, latency_ms);
            return;
        }
        let (trace, actor) = self.current_trace();
        let usd = self.estimate(model, usage).filter(|usd| valid_usd(*usd));
        let _day = self
            .day_budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let budget_session_id = self
            .budget_session_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| session_id.to_string());
        if let Err(error) = self.append_settled_row(&CostSettlement {
            provider,
            model,
            session_id,
            usage,
            latency_ms,
            usd,
            trace: trace.clone(),
            actor,
            reservation_id: None,
            budget_session_id: Some(budget_session_id.clone()),
        }) {
            tracing::warn!(error_kind = %vak_telemetry::error_kind(&error), "the cost ledger append failed");
            let id = SpendReservationId::new();
            let now = chrono::Utc::now();
            if let Err(error) = self.reserve_durable(
                DurableReservation {
                    id: id.as_str(),
                    created_at: now,
                    day: now.date_naive(),
                    estimate_usd: usd,
                    session_id: session_id.to_string(),
                    model: model.to_string(),
                    provider: provider.to_string(),
                    budget_session_id,
                    trace,
                    actor,
                },
                None,
                None,
            ) {
                tracing::warn!(error_kind = %vak_telemetry::error_kind(&std::io::Error::other(error)), "durable settled-cost liability could not be saved");
            }
            if let Some(usd) = usd {
                *self
                    .run_reserved_usd
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) += usd;
            } else {
                *self
                    .run_unknown_reservations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            }
        } else {
            if let Some(usd) = usd {
                *self
                    .run_spent_usd
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) += usd;
            } else {
                *self
                    .run_unknown_reservations
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) += 1;
            }
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
/// the `<home>/budget-alerts` chain; never enters session logs, so projections
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(BudgetAlertRow, "budget_alert_row");

fn alerts_path(home: &std::path::Path) -> PathBuf {
    home.join("budget-alerts")
}

/// Append an alert row for `level`, stamping it with the CURRENT day
/// spend from the cost ledger. Returns the row as written.
pub fn record_alert(
    home: &std::path::Path,
    level: AlertLevel,
    session_id: &str,
) -> std::io::Result<BudgetAlertRow> {
    let row = BudgetAlertRow {
        kind: "budget_alert".to_string(),
        ts: chrono::Utc::now(),
        level,
        day_total_usd: FinOpsLedger::new(home).day_total_usd(chrono::Utc::now()),
        session_id: session_id.to_string(),
        trace: None,
        actor: None,
    };
    RecordChain::at(alerts_path(home))
        .append(&row)
        .map_err(std::io::Error::other)?;
    Ok(row)
}

/// Most recent recorded alert at exactly `level`, for once-per-window
/// firing decisions. Rows that do not decode are skipped; none is None.
pub fn last_alert(home: &std::path::Path, level: AlertLevel) -> Option<BudgetAlertRow> {
    let mut found = None;
    RecordChain::at(alerts_path(home)).scan(|row: BudgetAlertRow| {
        if row.kind == "budget_alert" && row.level == level {
            found = Some(row);
        }
        true
    });
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

    #[tokio::test]
    async fn concurrent_price_and_cap_refresh_cannot_admit_a_mixed_policy() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempdir().unwrap();
        let policy = |price, cap| vak_config::FinopsResolved {
            max_run_usd: Some(cap),
            price_overrides: BTreeMap::from([(
                "atomic-fixture".into(),
                vak_config::PriceEntry {
                    input: price,
                    output: price,
                },
            )]),
            ..Default::default()
        };
        let lower = policy(10.0, 5.0);
        let higher = policy(100.0, 50.0);
        let gate = Arc::new(CoreSpendGate::new(dir.path(), &lower));
        let writer = gate.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let start = barrier.clone();
        let updates = std::thread::spawn(move || {
            start.wait();
            for n in 0..10_000 {
                writer.refresh_caps(if n % 2 == 0 { &higher } else { &lower });
            }
        });
        barrier.wait();
        for _ in 0..1_000 {
            assert!(
                gate.reserve_dispatch(&check("atomic-fixture"))
                    .await
                    .is_err(),
                "both complete policies refuse this request"
            );
        }
        updates.join().unwrap();
    }

    /// Manual growth probe for the strict cost rollup and durable admission
    /// path. Run with `cargo test -p vak-core finops_growth_ -- --ignored
    /// --nocapture`; this intentionally measures production code and is not a
    /// timing-sensitive CI test.
    #[tokio::test]
    #[ignore = "manual FinOps scaling benchmark"]
    async fn finops_growth_100_1000_10000() {
        vak_config::paths::isolate_home_for_tests();
        use std::hint::black_box;
        use std::time::Instant;

        for count in [100_usize, 1_000, 10_000] {
            let dir = tempdir().unwrap();
            let ledger = FinOpsLedger::new(dir.path());
            let now = chrono::Utc::now();
            let rows: Vec<_> = (0..count)
                .map(|index| CostRow {
                    ts: now,
                    model: "benchmark-model".into(),
                    provider: "benchmark-provider".into(),
                    input_tokens: 1_000,
                    output_tokens: 100,
                    cache_read_input_tokens: None,
                    usd: Some(0.001),
                    source: "estimated".into(),
                    session_id: format!("session-{index}"),
                    reservation_id: Some(uuid::Uuid::now_v7().hyphenated().to_string()),
                    budget_session_id: Some("benchmark-budget-owner".into()),
                    trace: None,
                    actor: None,
                })
                .collect();

            let seed_started = Instant::now();
            assert!(
                ledger
                    .cost_chain()
                    .append_all_after(vak_session::tail::Position::default(), &rows)
                    .unwrap()
            );
            let seed_elapsed = seed_started.elapsed();

            let cold_started = Instant::now();
            let totals = ledger.strict_session_totals().unwrap();
            let cold_elapsed = cold_started.elapsed();
            assert_eq!(totals.reservations.len(), count);

            let warm_started = Instant::now();
            for _ in 0..25 {
                black_box(ledger.strict_session_totals().unwrap());
            }
            let warm_elapsed = warm_started.elapsed() / 25;

            let mut price_overrides = BTreeMap::new();
            price_overrides.insert(
                "benchmark-model".to_string(),
                vak_config::PriceEntry {
                    input: 1.0,
                    output: 1.0,
                },
            );
            let finops = vak_config::FinopsResolved {
                max_day_usd: Some(1_000_000.0),
                price_overrides,
                ..Default::default()
            };
            let gate = CoreSpendGate::new(dir.path(), &finops);
            gate.bind_budget_session("benchmark-budget-owner");
            let admission_check = SpendCheck {
                model: "benchmark-model",
                provider: "benchmark-provider",
                session_id: "benchmark-session",
                est_input_tokens: 1_000,
                planned_output_tokens: 100,
                trace: None,
            };
            let gate_started = Instant::now();
            for _ in 0..5 {
                let reservation = gate
                    .reserve_dispatch(&admission_check)
                    .await
                    .unwrap()
                    .expect("core gate returns a reservation id");
                gate.release_dispatch(reservation);
            }
            let gate_elapsed = gate_started.elapsed() / 5;

            eprintln!(
                "FinOps rows={count}: seed_append_all={seed_elapsed:?}, strict_cold={cold_elapsed:?}, strict_warm_avg={warm_elapsed:?}, durable_admit_release_avg={gate_elapsed:?}"
            );
        }
    }

    fn durable_reservations(home: &Path) -> DurableReservations {
        let path = vak_config::scope::SharedScope::new(home).cost_reservations();
        let raw = vak_session::documents::read(&path).unwrap().unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    #[tokio::test]
    async fn day_budget_is_shared_by_cores_for_the_same_data_home_only() {
        let first_home = tempdir().unwrap();
        let other_home = tempdir().unwrap();
        let first = shared_day_budget(first_home.path());
        let second = shared_day_budget(first_home.path());
        let separate = shared_day_budget(other_home.path());

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &separate));

        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate_a = CoreSpendGate::with_shared_day_budget(first_home.path(), &finops, first);
        let gate_b = CoreSpendGate::with_shared_day_budget(first_home.path(), &finops, second);
        let check = check("claude-sonnet");
        assert!(gate_a.authorize(&check).await.is_ok());
        assert!(gate_b.authorize(&check).await.is_err());
    }

    #[tokio::test]
    async fn durable_day_reservation_is_shared_by_independent_gate_maps() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let first = CoreSpendGate::new(dir.path(), &finops);
        let second = CoreSpendGate::new(dir.path(), &finops);
        let check = check("claude-sonnet");

        assert!(first.reserve_dispatch(&check).await.unwrap().is_some());
        assert!(second.reserve_dispatch(&check).await.is_err());
    }

    #[tokio::test]
    async fn durable_day_reservation_survives_gate_drop_and_reopen() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let check = check("claude-sonnet");
        let first = CoreSpendGate::new(dir.path(), &finops);
        assert!(first.reserve_dispatch(&check).await.unwrap().is_some());
        drop(first);

        let reopened = CoreSpendGate::new(dir.path(), &finops);
        assert!(reopened.reserve_dispatch(&check).await.is_err());
    }

    #[tokio::test]
    async fn durable_session_spend_survives_gate_reopen_and_isolation() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let check = check("claude-sonnet");
        let first = CoreSpendGate::new(dir.path(), &finops);
        first.bind_budget_session("session-a");
        let id = first.reserve_dispatch(&check).await.unwrap().unwrap();
        first.settle_dispatch(
            id,
            "anthropic",
            "claude-sonnet",
            "s1",
            &usage(1_000_000, 100_000),
            1,
        );
        drop(first);

        let reopened = CoreSpendGate::new(dir.path(), &finops);
        reopened.bind_budget_session("session-a");
        assert!(
            reopened.reserve_dispatch(&check).await.is_err(),
            "settled session spend survives process/gate loss"
        );

        let other_session = CoreSpendGate::new(dir.path(), &finops);
        other_session.bind_budget_session("session-b");
        assert!(
            other_session
                .reserve_dispatch(&check)
                .await
                .unwrap()
                .is_some(),
            "one session's lifetime total does not charge another"
        );
    }

    #[tokio::test]
    async fn durable_session_cap_serializes_independent_gates() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let first = Arc::new(CoreSpendGate::new(dir.path(), &finops));
        let second = Arc::new(CoreSpendGate::new(dir.path(), &finops));
        first.bind_budget_session("same-session");
        second.bind_budget_session("same-session");
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let a = first.clone();
        let a_barrier = barrier.clone();
        let handle = tokio::runtime::Handle::current();
        let first_task = tokio::task::spawn_blocking(move || {
            a_barrier.wait();
            handle.block_on(a.reserve_dispatch(&check("claude-sonnet")))
        });
        let b = second.clone();
        let handle = tokio::runtime::Handle::current();
        let second_task = tokio::task::spawn_blocking(move || {
            barrier.wait();
            handle.block_on(b.reserve_dispatch(&check("claude-sonnet")))
        });
        let results = tokio::join!(first_task, second_task);
        let a = results.0.unwrap();
        let b = results.1.unwrap();
        assert_ne!(
            a.is_ok(),
            b.is_ok(),
            "only one concurrent CAS admission can fit the cap: {a:?}, {b:?}"
        );
    }

    #[tokio::test]
    async fn child_trace_is_preserved_and_admission_session_owns_settlement() {
        use vak_session::ids::{AgentId, SpaceId, TenantId};
        use vak_session::trace::{Cause, TraceKey};
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let child_trace = TraceKey::root(
            TenantId::new(),
            SpaceId::new(),
            AgentId::new(),
            Cause::Heartbeat,
        )
        .child();
        let child_check = SpendCheck {
            session_id: "admitted-session",
            trace: Some(&child_trace),
            ..check("claude-sonnet")
        };
        let id = gate.reserve_dispatch(&child_check).await.unwrap().unwrap();
        gate.settle_dispatch(
            id,
            "anthropic",
            "claude-sonnet",
            "reported-session",
            &usage(1_000_000, 100_000),
            1,
        );
        let rows = gate.ledger.all_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "reported-session");
        assert_eq!(rows[0].trace.as_ref(), Some(&child_trace));
        assert_eq!(
            rows[0].budget_session_id.as_deref(),
            Some("admitted-session")
        );
        assert!(rows[0].reservation_id.is_some());
        drop(gate);
        let reopened = CoreSpendGate::new(dir.path(), &finops);
        reopened.bind_budget_session("admitted-session");
        assert!(
            reopened
                .reserve_dispatch(&check("claude-sonnet"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn repeated_reservation_rows_do_not_silently_discount_paid_dispatches() {
        let dir = tempdir().unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        let mut row = row_on(chrono::Utc::now().date_naive(), Some(1.0));
        row.reservation_id = Some(SpendReservationId::new().as_str());
        row.budget_session_id = Some("session".into());
        ledger.append(&row).unwrap();
        ledger.append(&row).unwrap();
        let gate = CoreSpendGate::new(
            dir.path(),
            &vak_config::FinopsResolved {
                max_day_usd: Some(100.0),
                ..Default::default()
            },
        );
        assert!(
            gate.reserve_dispatch(&check("claude-sonnet"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn legacy_rollup_cache_is_rebuilt_before_budget_admission() {
        let dir = tempdir().unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        ledger
            .append(&row_on(chrono::Utc::now().date_naive(), Some(2.0)))
            .unwrap();
        // Simulate the prior verified cache shape: a bare date map inside Stored<S>.
        Rollup::new(&ledger.path, &ledger.rollup_path)
            .try_read(|state: &mut BTreeMap<chrono::NaiveDate, f64>, bytes| {
                if let Ok(row) = serde_json::from_slice::<CostRow>(bytes) {
                    *state.entry(row.ts.date_naive()).or_default() += row.usd.unwrap_or_default();
                }
                Ok(())
            })
            .unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let error = CoreSpendGate::new(dir.path(), &finops)
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("legacy date map cannot decode as the new strict totals");
        assert!(
            error.contains("day budget"),
            "the cache was rebuilt from the two-dollar cost row: {error}"
        );
    }

    #[tokio::test]
    async fn priced_settlement_completes_interrupted_cleanup_before_next_admission() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let first = CoreSpendGate::new(dir.path(), &finops);
        first.bind_budget_session("session-a");
        let pending_check = check("claude-sonnet");
        let id = first
            .reserve_dispatch(&pending_check)
            .await
            .unwrap()
            .unwrap();
        let estimate = first
            .estimate("claude-sonnet", &usage(1_000_000, 100_000))
            .unwrap();
        assert!(estimate > 4.0);
        // Simulate a crash after appending actual usage but before the durable
        // reservation is removed. Actual is below the reservation estimate.
        first
            .ledger
            .append(&CostRow {
                ts: chrono::Utc::now(),
                model: "claude-sonnet".into(),
                provider: "anthropic".into(),
                input_tokens: 1_000_000,
                output_tokens: 100_000,
                cache_read_input_tokens: None,
                usd: Some(3.0),
                source: "estimated".into(),
                session_id: "s1".into(),
                reservation_id: Some(id.as_str().to_string()),
                budget_session_id: Some("session-a".into()),
                trace: None,
                actor: None,
            })
            .unwrap();
        drop(first);

        let reopened_finops = vak_config::FinopsResolved {
            max_run_usd: Some(4.0),
            ..Default::default()
        };
        let reopened = CoreSpendGate::new(dir.path(), &reopened_finops);
        reopened.bind_budget_session("session-a");
        let small = SpendCheck {
            est_input_tokens: 100_000,
            planned_output_tokens: 0,
            ..check("claude-sonnet")
        };
        let next = reopened
            .reserve_dispatch(&small)
            .await
            .expect("the verified final cost replaces the interrupted estimate")
            .unwrap();
        let state = durable_reservations(dir.path());
        assert_eq!(state.reservations.len(), 1);
        assert_eq!(state.reservations[0].id, next.as_str());
        assert!(
            !state
                .reservations
                .iter()
                .any(|pending| pending.id == id.as_str())
        );
    }

    #[tokio::test]
    async fn definite_release_frees_durable_day_reservation() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let check = check("claude-sonnet");
        let id = gate.reserve_dispatch(&check).await.unwrap().unwrap();
        assert!(gate.reserve_dispatch(&check).await.is_err());

        gate.release_dispatch(id);
        assert!(gate.reserve_dispatch(&check).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn settlement_row_replaces_durable_reservation_without_double_counting() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let check = check("claude-sonnet");
        let id = gate.reserve_dispatch(&check).await.unwrap().unwrap();
        gate.settle_dispatch(id, "anthropic", "claude-sonnet", "s1", &usage(1, 1), 1);

        assert!(gate.reserve_dispatch(&check).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn malformed_durable_reservations_fail_closed() {
        let _isolated_home = vak_config::paths::isolate_home_for_tests();
        let dir = tempdir().unwrap();
        let path = vak_config::scope::SharedScope::new(dir.path()).cost_reservations();
        vak_session::documents::update(&path, |_| {
            Ok(Some((
                r#"{"schema":1,"reservations":[{"broken":true}]}"#.into(),
                (),
            )))
        })
        .unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);

        assert!(
            gate.reserve_dispatch(&check("claude-sonnet"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn legacy_reservation_without_budget_session_blocks_run_cap() {
        let dir = tempdir().unwrap();
        let path = vak_config::scope::SharedScope::new(dir.path()).cost_reservations();
        vak_session::documents::update(&path, |_| {
            Ok(Some((
                serde_json::json!({
                    "schema": 1,
                    "reservations": [{
                        "id": "legacy-reservation",
                        "created_at": chrono::Utc::now(),
                        "day": chrono::Utc::now().date_naive(),
                        "estimate_usd": 0.01,
                        "trace": null,
                        "actor": null
                    }]
                })
                .to_string(),
                (),
            )))
        })
        .unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let error = CoreSpendGate::new(dir.path(), &finops)
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("a pre-attribution unresolved reservation cannot be ignored");
        assert!(
            error.contains("legacy reservation has no session attribution"),
            "{error}"
        );
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
            trace: None,
        }
    }

    #[tokio::test]
    async fn unpriced_model_admits_and_records_unknown_usd() {
        let (gate, dir) = gate_with(&[]);
        gate.authorize(&check("mystery-model")).await.unwrap();
        gate.record_settled(
            "anthropic",
            "mystery-model",
            "s1",
            &usage(1_000_000, 100_000),
        );
        let ledger = FinOpsLedger::new(dir.path());
        assert_eq!(ledger.day_total_usd(chrono::Utc::now()), 0.0);
        let rows: Vec<serde_json::Value> = RecordChain::at(dir.path().join("cost-log")).read();
        assert!(rows.iter().all(|row| row["usd"].is_null()));
    }

    #[tokio::test]
    async fn unpriced_model_cannot_bypass_either_dollar_cap() {
        for cap in ["run", "day"] {
            let (gate, _dir) = gate_with(&[(cap, 0.01)]);
            let error = gate.authorize(&check("mystery-model")).await.unwrap_err();
            assert!(error.contains("unknown pricing"), "{error}");
        }
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
                reservation_id: None,
                budget_session_id: None,
                trace: None,
                actor: None,
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
                reservation_id: None,
                budget_session_id: None,
                trace: None,
                actor: None,
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
            reservation_id: None,
            budget_session_id: None,
            trace: None,
            actor: None,
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
    fn spend_rolls_up_per_agent_and_per_run() {
        use vak_session::ids::{AgentId, SpaceId, TenantId};
        use vak_session::trace::{Cause, TraceKey};
        let key = |agent: AgentId| {
            TraceKey::root(TenantId::new(), SpaceId::new(), agent, Cause::Heartbeat)
        };
        let (a, b) = (AgentId::new(), AgentId::new());
        let (run_a1, run_a2, run_b) = (key(a), key(a), key(b));
        let row = |trace: Option<&TraceKey>, usd: Option<f64>| CostRow {
            ts: chrono::Utc::now(),
            model: "m".into(),
            provider: "p".into(),
            input_tokens: 10,
            output_tokens: 5,
            cache_read_input_tokens: None,
            usd,
            source: "estimated".into(),
            session_id: "s".into(),
            reservation_id: None,
            budget_session_id: None,
            trace: trace.cloned(),
            actor: None,
        };
        let rows = vec![
            row(Some(&run_a1), Some(1.0)),
            row(Some(&run_a1.child()), Some(0.5)),
            row(Some(&run_a2), Some(2.0)),
            row(Some(&run_b), None),
            row(None, Some(4.0)),
        ];
        let agents = rollup_by_agent(&rows);
        let of = |k: &str| agents.iter().find(|r| r.key == k).unwrap();
        assert_eq!(agents.len(), 3);
        assert!((of(&a.to_string()).usd - 3.5).abs() < 1e-9);
        assert_eq!(of(&a.to_string()).calls, 3);
        assert_eq!(of(&b.to_string()).unknown_calls, 1);
        assert_eq!(of(&b.to_string()).usd, 0.0);
        assert!((of("unattributed").usd - 4.0).abs() < 1e-9);
        assert_eq!(agents[0].key, "unattributed", "largest spender first");

        let runs = rollup_by_run(&rows);
        assert_eq!(runs.len(), 3, "the keyless row belongs to no run");
        let run = |k: &TraceKey| runs.iter().find(|r| r.key == k.run.to_string()).unwrap();
        assert!(
            (run(&run_a1).usd - 1.5).abs() < 1e-9,
            "a child span is the same run"
        );
        assert_eq!(run(&run_a1).calls, 2);
        assert!((run(&run_a2).usd - 2.0).abs() < 1e-9);
        assert_eq!(run(&run_b).unknown_calls, 1);
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
    fn corrupt_or_foreign_rows_are_ignored_by_readback() {
        let dir = tempdir().unwrap();
        RecordChain::at(alerts_path(dir.path()))
            .append_all(&[
                serde_json::json!("not a row"),
                serde_json::json!({"kind": "receipt", "other": 1}),
                serde_json::json!({
                    "kind": "budget_alert", "ts": "2026-08-24T00:00:00Z", "level": "eighty",
                    "day_total_usd": 1.5, "session_id": "seed",
                }),
            ])
            .unwrap();
        let found = last_alert(dir.path(), AlertLevel::Eighty).unwrap();
        assert_eq!(found.session_id, "seed");
        assert!((found.day_total_usd - 1.5).abs() < 1e-9);
        assert!(last_alert(dir.path(), AlertLevel::Full).is_none());
    }

    /// Audit fix: concurrent dispatches used to each read the same stale
    /// on-disk day total before any of them settled, so a burst could
    /// jointly blow past `max_day_usd`. `DayBudget::reserved_usd` credits
    /// an admission immediately so a second concurrent `authorize()` sees
    /// the first one's reservation before either settles.
    #[tokio::test]
    async fn concurrent_authorize_calls_cannot_jointly_exceed_the_day_cap() {
        let dir = tempdir().unwrap();
        // sonnet est ~= $4.50/dispatch; cap admits exactly one.
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let day_budget = Arc::new(Mutex::new(DayBudget::new()));
        let gate_a = CoreSpendGate::with_shared_day_budget(dir.path(), &finops, day_budget.clone());
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

    /// A run cap must include admitted estimates, not only usage from calls
    /// that have already settled. Parallel tool/child dispatches share this
    /// gate and therefore serialize their admission against one run total.
    #[tokio::test]
    async fn concurrent_authorize_calls_cannot_jointly_exceed_the_run_cap() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);

        let first = gate.authorize(&check("claude-sonnet")).await;
        let second = gate.authorize(&check("claude-sonnet")).await;
        assert!(first.is_ok(), "{first:?}");
        assert!(
            second.is_err(),
            "the second dispatch must see the first dispatch's run reservation"
        );
    }

    #[tokio::test]
    async fn legacy_out_of_order_settlement_keeps_larger_estimate_reserved() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(1.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let make_check = |input| SpendCheck {
            model: "claude-sonnet",
            provider: "anthropic",
            session_id: "s1",
            est_input_tokens: input,
            planned_output_tokens: 0,
            trace: None,
        };
        gate.authorize(&make_check(2_000)).await.unwrap();
        gate.authorize(&make_check(1_000)).await.unwrap();

        // Identity-free settlement reports the smaller call first even though
        // authorize saw the larger estimate first. Release the smallest
        // pending estimate so the larger still-active call remains covered.
        gate.record_settled("anthropic", "claude-sonnet", "s1", &usage(1_000, 0));
        let small = gate.estimate("claude-sonnet", &usage(1_000, 0)).unwrap();
        let large = gate.estimate("claude-sonnet", &usage(2_000, 0)).unwrap();
        assert!((gate.run_reserved_usd.lock().unwrap().to_owned() - large).abs() < 1e-9);
        assert!((gate.run_spent_usd.lock().unwrap().to_owned() - small).abs() < 1e-9);
    }

    #[tokio::test]
    async fn exact_dispatch_reservation_can_be_released_once() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let check = check("claude-sonnet");
        let id = gate
            .reserve_dispatch(&check)
            .await
            .unwrap()
            .expect("CoreSpendGate returns exact reservation ids");
        assert!(gate.reserve_dispatch(&check).await.is_err());

        gate.release_dispatch(id);
        gate.release_dispatch(id); // terminal release is idempotent
        assert!(gate.reserve_dispatch(&check).await.is_ok());
    }

    #[tokio::test]
    async fn exact_dispatch_settlement_replaces_its_estimate_once() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let check = check("claude-sonnet");
        let id = gate
            .reserve_dispatch(&check)
            .await
            .unwrap()
            .expect("CoreSpendGate returns exact reservation ids");
        let actual = Usage::default();
        gate.settle_dispatch(id, "anthropic", "claude-sonnet", "s1", &actual, 7);
        gate.settle_dispatch(id, "anthropic", "claude-sonnet", "s1", &actual, 7);

        // A duplicate terminal event must not add a second cost row.
        assert_eq!(gate.ledger.all_rows().len(), 1);
        assert!(gate.reserve_dispatch(&check).await.is_ok());
    }

    #[tokio::test]
    async fn strict_day_admission_sees_external_same_day_append_after_cache() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let id = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .unwrap()
            .unwrap();
        gate.release_dispatch(id);
        FinOpsLedger::new(dir.path())
            .append(&row_on(chrono::Utc::now().date_naive(), Some(2.0)))
            .unwrap();

        let error = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("freshly appended same-day cost must be folded into strict state");
        assert!(error.contains("day budget"), "{error}");
    }

    #[tokio::test]
    async fn strict_day_admission_rejects_corrupt_suffix_after_old_tolerant_cache() {
        let dir = tempdir().unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        RecordChain::at(&ledger.path)
            .append(&serde_json::json!({"kind": "unreadable cost row"}))
            .unwrap();
        Rollup::new(&ledger.path, &ledger.rollup_path).read(|_: &mut DailyCostTotals, _| {});
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);

        let error = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("strict reader must reject a row the tolerant projection skipped");
        assert!(error.contains("history could not be verified"), "{error}");
    }

    #[tokio::test]
    async fn strict_day_admission_rejects_negative_usd() {
        let dir = tempdir().unwrap();
        FinOpsLedger::new(dir.path())
            .append(&row_on(chrono::Utc::now().date_naive(), Some(-1.0)))
            .unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let error = CoreSpendGate::new(dir.path(), &finops)
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("negative USD must not reduce the cap total");
        assert!(error.contains("history could not be verified"), "{error}");
    }

    #[tokio::test]
    async fn strict_day_admission_rejects_unpriced_current_day_rows() {
        let dir = tempdir().unwrap();
        FinOpsLedger::new(dir.path())
            .append(&row_on(chrono::Utc::now().date_naive(), None))
            .unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let error = CoreSpendGate::new(dir.path(), &finops)
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("unpriced spend cannot be treated as zero under a new day cap");
        assert!(error.contains("budget"), "{error}");
    }

    #[tokio::test]
    async fn unresolved_reservation_stays_charged_across_utc_day_rollover() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let id = gate
            .admit_at(&check("claude-sonnet"), chrono::Utc::now())
            .unwrap();
        let tomorrow = chrono::Utc::now() + chrono::Duration::days(1);

        let error = gate
            .admit_at(&check("claude-sonnet"), tomorrow)
            .expect_err("an unresolved prior-day dispatch remains conservatively reserved");
        assert!(error.contains("day budget"), "{error}");
        assert!(gate.reservations.lock().unwrap().contains_key(&id));
    }

    #[tokio::test]
    async fn settlement_does_not_double_count_actual_and_reservation() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        use vak_session::ids::{AgentId, SpaceId, TenantId};
        use vak_session::trace::{Cause, TraceKey};
        let trace = TraceKey::root(
            TenantId::new(),
            SpaceId::new(),
            AgentId::new(),
            Cause::Heartbeat,
        );
        gate.set_trace(Some(trace.clone()));
        let id = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .unwrap()
            .unwrap();
        gate.set_trace(None);
        gate.settle_dispatch(id, "anthropic", "claude-sonnet", "s1", &Usage::default(), 0);

        assert!(gate.reserve_dispatch(&check("claude-sonnet")).await.is_ok());
        assert_eq!(gate.ledger.all_rows().len(), 1);
        assert_eq!(gate.ledger.all_rows()[0].trace, Some(trace));
    }

    #[tokio::test]
    async fn reservations_keep_dispatch_trace_across_gate_trace_changes() {
        use vak_session::ids::{AgentId, PrincipalId, SpaceId, TenantId};
        use vak_session::trace::{Cause, TraceKey};

        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(10.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let trace = || {
            TraceKey::root(
                TenantId::new(),
                SpaceId::new(),
                AgentId::new(),
                Cause::Heartbeat,
            )
        };
        let first_trace = trace().acting(PrincipalId::new(), None);
        let second_trace = trace().acting(PrincipalId::new(), None);
        fn check_for_trace<'a>(trace: &'a vak_session::trace::TraceKey) -> SpendCheck<'a> {
            SpendCheck {
                model: "claude-sonnet",
                provider: "anthropic",
                session_id: "s1",
                est_input_tokens: 1_000_000,
                planned_output_tokens: 100_000,
                trace: Some(trace),
            }
        }

        let first = gate
            .reserve_dispatch(&check_for_trace(&first_trace))
            .await
            .unwrap()
            .unwrap();
        let second = gate
            .reserve_dispatch(&check_for_trace(&second_trace))
            .await
            .unwrap()
            .unwrap();
        assert!(
            gate.reserve_dispatch(&check_for_trace(&first_trace))
                .await
                .is_err()
        );

        // Simulate the mutable legacy turn trace changing before the calls
        // settle in the opposite order from admission.
        gate.set_trace(Some(trace()));
        gate.settle_dispatch(
            second,
            "anthropic",
            "claude-sonnet",
            "s1",
            &Usage::default(),
            0,
        );
        gate.settle_dispatch(
            first,
            "anthropic",
            "claude-sonnet",
            "s1",
            &Usage::default(),
            0,
        );

        let rows = gate.ledger.all_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].trace, Some(second_trace.clone()));
        assert_eq!(rows[1].trace, Some(first_trace.clone()));
        assert_eq!(rows[0].actor, second_trace.actor);
        assert_eq!(rows[1].actor, first_trace.actor);
        assert!(
            gate.reserve_dispatch(&check_for_trace(&first_trace))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn uncapped_settlement_is_counted_if_a_run_cap_is_enabled_later() {
        let dir = tempdir().unwrap();
        let gate = CoreSpendGate::new(dir.path(), &vak_config::FinopsResolved::default());
        let id = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .unwrap()
            .unwrap();
        gate.settle_dispatch(
            id,
            "anthropic",
            "claude-sonnet",
            "s1",
            &usage(1_000_000, 100_000),
            0,
        );
        gate.refresh_caps(&vak_config::FinopsResolved {
            max_run_usd: Some(4.0),
            ..Default::default()
        });

        let error = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("known spend must remain visible when a cap is added later");
        assert!(error.contains("run budget"), "{error}");
    }

    #[tokio::test]
    async fn unknown_active_reservation_blocks_a_later_cap_until_definite_release() {
        let dir = tempdir().unwrap();
        let gate = CoreSpendGate::new(dir.path(), &vak_config::FinopsResolved::default());
        let id = gate
            .reserve_dispatch(&check("mystery-model"))
            .await
            .unwrap()
            .unwrap();
        let mut prices = BTreeMap::new();
        prices.insert(
            "mystery-model".to_string(),
            vak_config::PriceEntry {
                input: 3.0,
                output: 15.0,
            },
        );
        gate.refresh_caps(&vak_config::FinopsResolved {
            max_run_usd: Some(20.0),
            price_overrides: prices,
            ..Default::default()
        });
        assert!(
            gate.reserve_dispatch(&check("mystery-model"))
                .await
                .expect_err("unknown pending cost must deny after a cap appears")
                .contains("unknown pricing")
        );

        gate.release_dispatch(id);
        assert!(gate.reserve_dispatch(&check("mystery-model")).await.is_ok());
    }

    #[tokio::test]
    async fn unknown_settlement_marks_run_liability_unknown_after_price_change() {
        let dir = tempdir().unwrap();
        let mut prices = BTreeMap::new();
        prices.insert(
            "mystery-model".to_string(),
            vak_config::PriceEntry {
                input: 3.0,
                output: 15.0,
            },
        );
        let gate = CoreSpendGate::new(
            dir.path(),
            &vak_config::FinopsResolved {
                price_overrides: prices.clone(),
                ..Default::default()
            },
        );
        let id = gate
            .reserve_dispatch(&check("mystery-model"))
            .await
            .unwrap()
            .unwrap();
        gate.refresh_caps(&vak_config::FinopsResolved {
            max_run_usd: Some(20.0),
            ..Default::default()
        });
        gate.settle_dispatch(
            id,
            "anthropic",
            "mystery-model",
            "s1",
            &usage(1_000_000, 100_000),
            0,
        );
        gate.refresh_caps(&vak_config::FinopsResolved {
            max_run_usd: Some(20.0),
            price_overrides: prices,
            ..Default::default()
        });

        let error = gate
            .reserve_dispatch(&check("mystery-model"))
            .await
            .expect_err("unknown settlement must remain visible under the enabled run cap");
        assert!(error.contains("unknown settled spend"), "{error}");
    }

    #[tokio::test]
    async fn durable_unpriced_settlement_stays_reserved_across_day_rollover() {
        let dir = tempdir().unwrap();
        let gate = CoreSpendGate::new(dir.path(), &vak_config::FinopsResolved::default());
        let id = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .unwrap()
            .unwrap();
        gate.settle_dispatch(
            id,
            "anthropic",
            "mystery-model",
            "s1",
            &usage(1_000_000, 100_000),
            0,
        );
        assert!(!gate.reservations.lock().unwrap().contains_key(&id));
        assert!(
            durable_reservations(dir.path())
                .reservations
                .iter()
                .any(|entry| { entry.id == id.as_str() && entry.estimate_usd.is_none() })
        );

        gate.refresh_caps(&vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        });
        let today_error = gate
            .admit_at(&check("claude-sonnet"), chrono::Utc::now())
            .expect_err("today's unpriced row remains unknown spend");
        assert!(today_error.contains("budget"));

        let tomorrow = chrono::Utc::now() + chrono::Duration::days(1);
        let rollover_error = gate
            .admit_at(&check("claude-sonnet"), tomorrow)
            .expect_err("unresolved unknown owner liability remains reserved across rollover");
        assert!(
            rollover_error.contains("unknown") || rollover_error.contains("budget"),
            "{rollover_error}"
        );
        assert!(
            durable_reservations(dir.path())
                .reservations
                .iter()
                .any(|entry| entry.id == id.as_str() && entry.estimate_usd.is_none())
        );
    }

    #[tokio::test]
    async fn failed_settlement_append_retains_day_and_run_reservation() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        let id = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .unwrap()
            .unwrap();
        let cost_path = gate.ledger.path.clone();
        let _ = std::fs::remove_dir_all(&cost_path);
        std::fs::write(&cost_path, "occupied").unwrap();
        gate.settle_dispatch(
            id,
            "anthropic",
            "claude-sonnet",
            "s1",
            &usage(2_000_000, 100_000),
            0,
        );

        assert!(gate.reservations.lock().unwrap()[&id].terminal);
        assert!(gate.reservations.lock().unwrap()[&id].estimated_usd >= Some(7.5));
        assert!(
            durable_reservations(dir.path())
                .reservations
                .iter()
                .any(|entry| entry.id == id.as_str() && entry.estimate_usd >= Some(7.5))
        );
        assert!(
            gate.reserve_dispatch(&check("claude-sonnet"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn failed_unknown_settlement_persists_unknown_liability_across_reopen() {
        let dir = tempdir().unwrap();
        let finops = vak_config::FinopsResolved {
            max_run_usd: Some(5.0),
            ..Default::default()
        };
        let gate = CoreSpendGate::new(dir.path(), &finops);
        gate.bind_budget_session("session-a");
        let id = gate
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .unwrap()
            .unwrap();
        let cost_path = gate.ledger.path.clone();
        let _ = std::fs::remove_dir_all(&cost_path);
        std::fs::write(&cost_path, "occupied").unwrap();
        gate.settle_dispatch(
            id,
            "anthropic",
            "mystery-model",
            "s1",
            &usage(1_000_000, 100_000),
            0,
        );
        assert_eq!(
            durable_reservations(dir.path())
                .reservations
                .iter()
                .find(|row| row.id == id.as_str())
                .and_then(|row| row.estimate_usd),
            None
        );
        drop(gate);

        std::fs::remove_file(&cost_path).unwrap();
        let reopened = CoreSpendGate::new(dir.path(), &finops);
        reopened.bind_budget_session("session-a");
        let error = reopened
            .reserve_dispatch(&check("claude-sonnet"))
            .await
            .expect_err("unknown settlement remains fail-closed after restart");
        assert!(error.contains("unknown pricing"), "{error}");
    }

    #[test]
    fn owner_reconcile_records_actual_above_estimate_and_is_idempotent() {
        use vak_session::ids::{AgentId, PrincipalId, SpaceId, TenantId};
        use vak_session::trace::{Cause, TraceKey};
        let dir = tempdir().unwrap();
        let gate = CoreSpendGate::new(dir.path(), &vak_config::FinopsResolved::default());
        let trace = TraceKey::root(
            TenantId::new(),
            SpaceId::new(),
            AgentId::new(),
            Cause::Heartbeat,
        );
        gate.set_trace(Some(trace.clone()));
        let id = futures::executor::block_on(gate.reserve_dispatch(&check("claude-sonnet")))
            .unwrap()
            .unwrap();
        let reservation = durable_reservations(dir.path()).reservations[0].clone();
        let estimate = reservation.estimate_usd.unwrap();
        let actor = PrincipalId::new();
        let ledger = FinOpsLedger::new(dir.path());

        ledger
            .reconcile_reservation(id.as_str().as_str(), estimate + 2.0, actor)
            .unwrap();
        ledger
            .reconcile_reservation(id.as_str().as_str(), estimate + 2.0, actor)
            .unwrap();
        assert!(
            ledger
                .reconcile_reservation(id.as_str().as_str(), estimate + 3.0, actor)
                .is_err()
        );
        assert!(ledger.pending_reservations().unwrap().is_empty());
        let rows = ledger.all_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].usd, Some(estimate + 2.0));
        assert_eq!(rows[0].source, "owner_verified");
        assert_eq!(
            rows[0].reservation_id.as_deref(),
            Some(id.as_str().as_str())
        );
        assert_eq!(rows[0].budget_session_id.as_deref(), Some("s1"));
        assert_eq!(rows[0].trace.as_ref(), Some(&trace));
        assert_eq!(rows[0].actor, Some(actor));
        assert_eq!(rows[0].model, "claude-sonnet");
        assert_eq!(rows[0].provider, "anthropic");
    }

    #[test]
    fn concurrent_owner_reconcile_conflicts_append_exactly_one_row() {
        use vak_session::ids::PrincipalId;
        let dir = tempdir().unwrap();
        let gate = CoreSpendGate::new(dir.path(), &vak_config::FinopsResolved::default());
        let id = futures::executor::block_on(gate.reserve_dispatch(&check("claude-sonnet")))
            .unwrap()
            .unwrap();
        let ledger = Arc::new(FinOpsLedger::new(dir.path()));
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let a_ledger = ledger.clone();
        let a_barrier = barrier.clone();
        let a_id = id.as_str();
        let a = std::thread::spawn(move || {
            a_barrier.wait();
            a_ledger.reconcile_reservation(&a_id, 6.0, PrincipalId::new())
        });
        let b_id = id.as_str();
        let b = std::thread::spawn(move || {
            barrier.wait();
            ledger.reconcile_reservation(&b_id, 7.0, PrincipalId::new())
        });
        let results = [a.join().unwrap(), b.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let rows = FinOpsLedger::new(dir.path()).all_rows();
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].usd, Some(6.0) | Some(7.0)));
    }

    #[test]
    fn owner_reconcile_refuses_unknown_or_unmatched_outcomes() {
        use vak_session::ids::PrincipalId;
        let dir = tempdir().unwrap();
        let gate = CoreSpendGate::new(dir.path(), &vak_config::FinopsResolved::default());
        let id = futures::executor::block_on(gate.reserve_dispatch(&check("mystery-model")))
            .unwrap()
            .unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        assert_eq!(ledger.pending_reservations().unwrap().len(), 1);
        let reservation = durable_reservations(dir.path()).reservations[0].clone();
        ledger
            .append(&CostRow {
                ts: chrono::Utc::now(),
                model: "mystery-model".into(),
                provider: "anthropic".into(),
                input_tokens: 1_000_000,
                output_tokens: 100_000,
                cache_read_input_tokens: None,
                usd: None,
                source: "estimated".into(),
                session_id: "s1".into(),
                reservation_id: Some(id.as_str()),
                budget_session_id: Some("s1".into()),
                trace: reservation.trace.clone(),
                actor: None,
            })
            .unwrap();
        let actor = PrincipalId::new();
        ledger
            .reconcile_reservation(&id.as_str(), 1.0, actor)
            .expect("an owner may replace an unknown settlement with a verified amount");
        ledger
            .reconcile_reservation(&id.as_str(), 1.0, actor)
            .expect("the verified correction is idempotent");
        assert!(
            ledger
                .reconcile_reservation(&id.as_str(), 2.0, actor)
                .is_err()
        );
        assert_eq!(durable_reservations(dir.path()).reservations.len(), 0);
        assert_eq!(ledger.pending_reservations().unwrap().len(), 0);
        assert_eq!(
            ledger.all_rows().len(),
            2,
            "the original unknown row stays append-only"
        );
        let accounted = ledger.accounted_rows().unwrap();
        assert_eq!(accounted.len(), 1);
        assert_eq!(accounted[0].usd, Some(1.0));
        assert_eq!(accounted[0].input_tokens, 1_000_000);
        assert_eq!(accounted[0].source, "owner_verified");
        let totals = ledger.strict_cost_totals().unwrap();
        assert_eq!(totals.sessions.get("s1").unwrap().unknown_calls, 0);
        assert_eq!(totals.sessions.get("s1").unwrap().usd, 1.0);
        assert_eq!(
            totals
                .days
                .values()
                .map(|day| day.unknown_calls)
                .sum::<u64>(),
            0
        );
        assert!(
            ledger
                .reconcile_reservation(&SpendReservationId::new().as_str(), 1.0, PrincipalId::new())
                .is_err()
        );
    }

    #[test]
    fn owner_reconcile_can_resolve_legacy_unknown_row_by_its_recorded_reservation_id() {
        use vak_session::ids::PrincipalId;
        let dir = tempdir().unwrap();
        let id = SpendReservationId::new();
        let ledger = FinOpsLedger::new(dir.path());
        ledger
            .append(&CostRow {
                ts: chrono::Utc::now(),
                model: "legacy-model".into(),
                provider: "legacy-provider".into(),
                input_tokens: 8,
                output_tokens: 3,
                cache_read_input_tokens: None,
                usd: None,
                source: "estimated".into(),
                session_id: "session-a".into(),
                reservation_id: Some(id.as_str()),
                budget_session_id: Some("session-a".into()),
                trace: None,
                actor: None,
            })
            .unwrap();

        let pending = ledger.pending_reservations().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id.as_str());
        assert_eq!(pending[0].model, "legacy-model");
        assert!(pending[0].estimate_usd.is_none());
        ledger
            .reconcile_reservation(&id.as_str(), 4.5, PrincipalId::new())
            .unwrap();
        assert!(ledger.pending_reservations().unwrap().is_empty());
        assert_eq!(ledger.all_rows().len(), 2);
        let totals = ledger.strict_cost_totals().unwrap();
        assert_eq!(totals.sessions.get("session-a").unwrap().usd, 4.5);
        assert_eq!(totals.sessions.get("session-a").unwrap().unknown_calls, 0);
    }

    #[test]
    fn owner_reconcile_fails_closed_on_corrupt_history() {
        use vak_session::ids::PrincipalId;
        let dir = tempdir().unwrap();
        let gate = CoreSpendGate::new(dir.path(), &vak_config::FinopsResolved::default());
        let id = futures::executor::block_on(gate.reserve_dispatch(&check("claude-sonnet")))
            .unwrap()
            .unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        RecordChain::at(&ledger.path)
            .append(&serde_json::json!({"invalid_cost_row": true}))
            .unwrap();
        assert!(
            ledger
                .reconcile_reservation(&id.as_str(), 5.0, PrincipalId::new())
                .is_err()
        );
        assert!(ledger.pending_reservations().is_err());
        assert_eq!(durable_reservations(dir.path()).reservations.len(), 1);
    }

    #[test]
    fn owner_reconcile_uses_unknown_model_for_legacy_reservation_metadata() {
        use vak_session::ids::PrincipalId;
        let dir = tempdir().unwrap();
        let id = SpendReservationId::new();
        let path = vak_config::scope::SharedScope::new(dir.path()).cost_reservations();
        vak_session::documents::update(&path, |_| {
            Ok(Some((
                serde_json::json!({
                    "schema": 1,
                    "reservations": [{
                        "id": id.as_str(), "created_at": chrono::Utc::now(),
                        "day": chrono::Utc::now().date_naive(), "estimate_usd": null,
                        "budget_session_id": "s1", "trace": null, "actor": null
                    }]
                })
                .to_string(),
                (),
            )))
        })
        .unwrap();
        let ledger = FinOpsLedger::new(dir.path());
        ledger
            .reconcile_reservation(&id.as_str(), 2.5, PrincipalId::new())
            .unwrap();
        let row = &ledger.all_rows()[0];
        assert_eq!(row.model, "unknown");
        assert_eq!(row.provider, "");
        assert_eq!(row.session_id, "s1");
    }

    /// `record_settled` releases a dispatch's reservation and folds the
    /// settled amount into the day baseline; a same-day gate built after
    /// the first must see the earlier settlement.
    #[tokio::test]
    async fn record_settled_updates_the_shared_day_budget_for_later_gates() {
        let dir = tempdir().unwrap();
        // sonnet est ~= $4.50/dispatch; cap admits exactly one, so a
        // gate that doesn't see gate_a's settlement would wrongly admit
        // a second one at ~$9.00 total.
        let finops = vak_config::FinopsResolved {
            max_day_usd: Some(5.0),
            ..Default::default()
        };
        let day_budget = Arc::new(Mutex::new(DayBudget::new()));
        let gate_a = CoreSpendGate::with_shared_day_budget(dir.path(), &finops, day_budget.clone());
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

    #[test]
    fn activity_ledger_round_trips_observed_rows() {
        let dir = tempdir().unwrap();
        let ledger = ActivityLedger::new(dir.path());
        ledger
            .append(&ActivityRow {
                ts: chrono::Utc::now(),
                kind: "mcp".into(),
                name: "server/tool".into(),
                success: false,
                duration_ms: Some(42),
                session_id: Some("s1".into()),
                plugin: Some("demo".into()),
                tool_use_id: None,
                trace: None,
                actor: None,
            })
            .unwrap();
        let rows = ledger.all_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].duration_ms, Some(42));
        assert_eq!(rows[0].plugin.as_deref(), Some("demo"));
    }
}
