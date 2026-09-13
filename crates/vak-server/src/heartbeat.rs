//! Proactive heartbeat (docs/design/29-personal-os.md P7): one bounded LLM
//! review turn per interval in a dedicated persistent session. Anti-nag by
//! architecture — a "nothing" reply costs tokens but produces zero inbox
//! noise and zero deliveries; findings park as `Kind::Heartbeat` inbox
//! entries and reach chat surfaces only when a line carries an URGENT
//! marker.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use tokio_util::sync::CancellationToken;
use vak_core::Core;
use vak_session::{SessionHeader, SessionLog};

use crate::AppState;

/// Fixed ledger id of the dedicated heartbeat session.
pub(crate) const HEARTBEAT_SESSION_ID: &str = "heartbeat";

/// Upper bound on one heartbeat turn so a stuck stream cannot wedge the
/// scheduler (mirrors REFLECTION_CALL_TIMEOUT).
const HEARTBEAT_TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// Grace period for the agent loop to observe cancellation and hand the
/// ledger back before the cycle gives up on it.
const CANCEL_GRACE: Duration = Duration::from_secs(10);

/// Cadence of the cheap due-check pass; the real gate is
/// `[heartbeat].interval_secs` (minimum 300).
pub(crate) const TICK_SECS: u64 = 5;

pub(crate) struct HeartbeatRuntime {
    /// Per-process fire timer; a restart re-fires immediately, matching
    /// interval tasks' self-healing behavior.
    last_fire: std::sync::Mutex<Option<DateTime<Utc>>>,
    inflight: AtomicBool,
}

impl HeartbeatRuntime {
    pub(crate) fn new() -> Self {
        HeartbeatRuntime {
            last_fire: std::sync::Mutex::new(None),
            inflight: AtomicBool::new(false),
        }
    }
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Pure due-matrix: never-run fires immediately; otherwise only once a
/// full interval has elapsed.
fn heartbeat_due(last_fire: Option<DateTime<Utc>>, now: DateTime<Utc>, interval_secs: u64) -> bool {
    match last_fire {
        None => true,
        Some(t) => (now - t).num_seconds() >= interval_secs as i64,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum HeartbeatReply {
    Nothing,
    Findings {
        /// Original trimmed finding lines (URGENT markers intact).
        lines: Vec<String>,
        /// Lines with any leading "URGENT:" marker removed.
        stripped: Vec<String>,
        urgent: bool,
    },
}

fn classify_reply(text: &str, max_findings: usize) -> HeartbeatReply {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("nothing") {
        return HeartbeatReply::Nothing;
    }
    let lines: Vec<String> = trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(max_findings.max(1))
        .map(String::from)
        .collect();
    let urgent = lines.iter().any(|l| l.starts_with("URGENT:"));
    let stripped = lines
        .iter()
        .map(|l| {
            l.strip_prefix("URGENT:")
                .map(str::trim_start)
                .unwrap_or(l)
                .to_string()
        })
        .collect();
    HeartbeatReply::Findings {
        lines,
        stripped,
        urgent,
    }
}

/// One cheap scheduler pass. Errors are values: failures log and leave the
/// next cycle to retry — this function never panics.
pub(crate) async fn heartbeat_tick(state: &AppState) {
    if !state.core.config().heartbeat.enabled {
        return;
    }
    if state.heartbeat.inflight.swap(true, Ordering::SeqCst) {
        return;
    }
    let result = heartbeat_cycle(state).await;
    state.heartbeat.inflight.store(false, Ordering::SeqCst);
    if let Err(e) = result {
        eprintln!("[heartbeat] cycle failed: {e}");
    }
}

async fn heartbeat_cycle(state: &AppState) -> Result<(), String> {
    let cfg = state.core.config().heartbeat.clone();
    let now = Utc::now();
    let due = {
        let last = lock(&state.heartbeat.last_fire);
        heartbeat_due(*last, now, cfg.interval_secs)
    };
    if !due {
        return Ok(());
    }
    if let Some(window) = cfg.quiet_hours {
        let local = chrono::Local::now();
        let minutes = local.hour() * 60 + local.minute();
        if window.contains(minutes) {
            return Ok(());
        }
    }
    // Same day-spend read the budget alerts use; denial skips this cycle
    // silently — an unattended prober must never be what bursts a cap.
    if let Some(cap) = state.core.config().finops.max_day_usd
        && vak_core::finops::FinOpsLedger::new(&state.core.sessions_home())
            .day_total_usd(Utc::now())
            >= cap
    {
        return Ok(());
    }
    // Claim the slot BEFORE dispatching so overlapping ticks cannot
    // double-fire while a turn outlives one tick period.
    *lock(&state.heartbeat.last_fire) = Some(now);
    run_heartbeat_turn(state, &cfg).await
}

async fn run_heartbeat_turn(
    state: &AppState,
    cfg: &vak_config::HeartbeatResolved,
) -> Result<(), String> {
    let provider = state
        .core
        .provider()
        .map_err(|e| format!("no provider credential: {e}"))?;
    // Dedicated child core over the SERVER cwd: the model pin stays scoped
    // here instead of mutating shared runtime overrides.
    let core = vak_core::Core::new_with_trust(state.core.cwd().clone(), true)
        // Unattended by construction: the turn's approver is `AutoDeny` and
        // no one is reading the reply as it streams. Both facts are stamped
        // before `take_persistent_session` composes and freezes the prompt,
        // so it never advertises a capability this turn cannot use.
        .map(|c| {
            c.with_surface(vak_core::Surface::Background)
                .with_approver_answerable(false)
        })
        .map_err(|e| format!("heartbeat core failed: {e}"))?;
    core.set_provider_instance(provider);
    core.set_sessions_home(state.core.sessions_home());
    if let Some(pin) = cfg
        .model
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let (pin_provider, pin_model) = crate::split_model_pin(pin, &core.effective_provider());
        core.set_route(pin_provider, pin_model);
    }

    let ledger = take_persistent_session(&core).await?;
    let prompt = format!(
        "Heartbeat review pass. You are an unattended watchdog: use your \
         tools (session_search, read, glob, grep) to review recent activity \
         in this workspace — recent sessions, tasks, failing checks, stale \
         work. Reply either exactly \"nothing\" (when nothing needs \
         attention) or up to {max} short actionable findings, one per line. \
         Prefix any finding that cannot wait until the next check-in with \
         \"URGENT:\".",
        max = cfg.max_findings
    );

    let approver: Arc<dyn vak_agent::Approver> = Arc::new(vak_agent::AutoDeny);
    let cancel = CancellationToken::new();
    let (events_tx, events_rx) = tokio::sync::mpsc::channel::<vak_agent::AgentEvent>(512);
    let mut fut = std::pin::pin!(core.run_turn_with(
        ledger,
        &prompt,
        cancel.clone(),
        Some(approver),
        None,
        None,
        events_tx,
    ));
    let outcome = match tokio::time::timeout(HEARTBEAT_TURN_TIMEOUT, fut.as_mut()).await {
        Ok(result) => Some(result),
        Err(_) => {
            eprintln!("[heartbeat] turn exceeded its bound; cancelling");
            cancel.cancel();
            // Abort preserves partial output: give the loop room to land
            // whatever it has before the cycle moves on.
            tokio::time::timeout(CANCEL_GRACE, fut.as_mut()).await.ok()
        }
    };
    drop(events_rx);

    match outcome {
        Some(Ok((outcome, _log))) => {
            let text = outcome_text(&outcome);
            record_reply(state, cfg, &text).await;
            Ok(())
        }
        Some(Err(e)) => Err(format!("heartbeat turn failed: {e}")),
        None => Err("heartbeat turn did not finish within its bound".to_string()),
    }
}

/// Open-or-create the fixed "heartbeat" ledger. The filesystem lock makes
/// concurrent takers (another process, a wedged prior cycle) fail closed
/// for this cycle instead of corrupting appends.
async fn take_persistent_session(core: &Core) -> Result<SessionLog, String> {
    match core.open_session(HEARTBEAT_SESSION_ID).await {
        Ok(log) => Ok(log),
        Err(_) => create_persistent_session(core).await,
    }
}

async fn create_persistent_session(core: &Core) -> Result<SessionLog, String> {
    let prepared = core.prepare_turn().await;
    let path = vak_session::SessionPath::new_session_file(
        &core.sessions_home(),
        core.cwd(),
        HEARTBEAT_SESSION_ID,
    );
    let header = SessionHeader {
        agent: None,
        session_id: HEARTBEAT_SESSION_ID.to_string(),
        created_at: chrono::Utc::now(),
        cwd: core.cwd().clone(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        contract: vak_session::FrozenContract {
            app_version: vak_core::APP_VERSION.to_string(),
            provider: core.effective_provider(),
            model: core.effective_model(),
            // Legacy single-model admission (empty ladder), same as
            // subagent children; dispatch falls back to the primary leg.
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: prepared.system_prompt,
            permission_mode: permission_mode_tag(core.effective_permission_mode()).to_string(),
            capabilities: core.capability_descriptors(),
            prompt_layers: Vec::new(),
        },
    };
    SessionLog::create(path, header).map_err(|e| format!("heartbeat session create: {e}"))
}

fn permission_mode_tag(mode: vak_config::PermissionMode) -> &'static str {
    match mode {
        vak_config::PermissionMode::ReadOnly => "read-only",
        vak_config::PermissionMode::WorkspaceWrite => "workspace-write",
        vak_config::PermissionMode::FullAccess => "full-access",
    }
}

fn outcome_text(o: &vak_agent::TurnOutcome) -> String {
    use vak_agent::TurnOutcome;
    match o {
        TurnOutcome::Completed { response } => response.text_content(),
        TurnOutcome::Aborted { partial } => partial
            .as_ref()
            .map(|m| m.text_content())
            .unwrap_or_default(),
        // Failures carry no review reply; classify treats them as
        // nothing-to-report rather than inventing findings from an error.
        TurnOutcome::Failed { .. } | TurnOutcome::MaxTurnsReached => String::new(),
    }
}

/// Anti-nag routing: "nothing"/"" logs a debug line and records nothing
/// anywhere; findings always park one inbox entry, and additionally ride
/// the delivery chokepoint only when some line is marked URGENT.
async fn record_reply(state: &AppState, cfg: &vak_config::HeartbeatResolved, text: &str) {
    let reply = classify_reply(text, cfg.max_findings);
    let HeartbeatReply::Findings {
        stripped, urgent, ..
    } = reply
    else {
        eprintln!("[heartbeat] nothing to report");
        return;
    };
    let n = stripped.len();
    let title = format!("heartbeat: {n} finding{}", if n == 1 { "" } else { "s" });
    let body = stripped.join("\n");
    let home = state.core.sessions_home();
    let _ = vak_core::inbox::record(
        &home,
        vak_core::inbox::Kind::Heartbeat,
        &title,
        &body,
        Some(HEARTBEAT_SESSION_ID),
        None,
    );
    if !urgent {
        return;
    }
    let mut targets = crate::configured_delivery_targets(state);
    if targets.is_empty() {
        targets.push(crate::FALLBACK_ALERT_TARGET.to_string());
    }
    let text = format!("{title}\n{body}");
    for target in targets {
        let _ = crate::gateway::deliver_and_record(
            &state.core,
            &target,
            &text,
            vak_core::inbox::Kind::Heartbeat,
            title.clone(),
            Some(HEARTBEAT_SESSION_ID),
            None,
        )
        .await;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{HeartbeatReply, classify_reply, heartbeat_due};
    use chrono::{TimeZone, Utc};

    #[test]
    fn due_matrix_never_run_interval_boundaries_and_future() {
        let now = Utc
            .with_ymd_and_hms(2026, 8, 25, 12, 0, 0)
            .single()
            .unwrap();
        assert!(heartbeat_due(None, now, 1800));
        let ago_100 = now - chrono::Duration::seconds(100);
        assert!(!heartbeat_due(Some(ago_100), now, 1800));
        // Exactly one interval elapsed counts as due (>= semantics).
        let ago_exact = now - chrono::Duration::seconds(1800);
        assert!(heartbeat_due(Some(ago_exact), now, 1800));
        let ago_more = now - chrono::Duration::seconds(1801);
        assert!(heartbeat_due(Some(ago_more), now, 1800));
        // A marker in the future (clock rewind) waits it out.
        let ahead = now + chrono::Duration::seconds(60);
        assert!(!heartbeat_due(Some(ahead), now, 1800));
    }

    #[test]
    fn nothing_variants_classify_as_nothing() {
        for s in ["", "   ", "nothing", " Nothing\n", "NOTHING"] {
            assert_eq!(
                classify_reply(s, 3),
                HeartbeatReply::Nothing,
                "'{s}' must be silent"
            );
        }
    }

    #[test]
    fn plain_findings_are_not_urgent_and_pass_through_verbatim() {
        let reply = classify_reply("fix the flaky cron\nclose stale PR #4", 3);
        match reply {
            HeartbeatReply::Findings {
                lines,
                stripped,
                urgent,
            } => {
                assert_eq!(lines, vec!["fix the flaky cron", "close stale PR #4"]);
                assert_eq!(stripped, lines);
                assert!(!urgent);
            }
            other => panic!("expected findings, got {other:?}"),
        }
    }

    #[test]
    fn urgent_marker_is_detected_and_stripped_from_body() {
        let reply = classify_reply("URGENT: disk almost full\nminor lint debt", 3);
        match reply {
            HeartbeatReply::Findings {
                lines,
                stripped,
                urgent,
            } => {
                assert_eq!(lines, vec!["URGENT: disk almost full", "minor lint debt"]);
                assert_eq!(stripped, vec!["disk almost full", "minor lint debt"]);
                assert!(urgent);
            }
            other => panic!("expected findings, got {other:?}"),
        }
    }

    #[test]
    fn findings_cap_at_max_findings_and_drop_blank_lines() {
        let reply = classify_reply("a\n\n  \nb\nc\nd\ne", 3);
        match reply {
            HeartbeatReply::Findings { lines, .. } => {
                assert_eq!(lines, vec!["a", "b", "c"]);
            }
            other => panic!("expected findings, got {other:?}"),
        }
    }
}
