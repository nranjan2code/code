//! vak-server: HTTP+SSE wrapper around vak-core. The TUI, web clients,
//! IDE extensions, and the desktop shell are all just consumers of these
//! endpoints — one headless agent core, many surfaces.
//!
//! Endpoints:
//! - `GET  /health`
//! - `POST /sessions`                     → {session_id}
//! - `GET  /sessions`                     → persisted session summaries (sidebar)
//! - `POST /sessions/:id/attach`          → resume a persisted session into memory
//! - `POST /sessions/:id/run` {prompt}    → 202 (events stream on SSE)
//! - `POST /sessions/:id/steering` {text} → 202
//! - `POST /sessions/:id/approvals/:rid` {approve} → resolve a pending gate
//! - `GET  /sessions/:id/events`          → SSE of AgentEvent JSON
//! - `GET  /sessions/:id/transcript`      → derived messages + usage
//! - `GET  /sessions/:id/transcript.md`   → markdown export (shared renderer)
//! - `GET  /sessions/:id/diff`            → git diff + status of the workspace
//! - `GET  /sessions/:id/checkpoints`     → workspace snapshots (time travel)
//! - `POST /sessions/:id/checkpoints/:seq/restore` → rewind the workspace
//! - `POST /sessions/:id/archive` {archived} → toggle sidebar visibility
//! - `GET  /skills`                       → discovered skills (name + description)
//! - `GET  /fs/file?path=`                → read a file confined to cwd
//! - `PUT  /fs/file` {path, content}      → write a file confined to cwd
//! - `POST /config/mode` {mode}           → switch permission mode at runtime
//! - `PUT  /config/key` {provider, key}   → store a provider credential (0600)
//! - `DELETE /config/key` {provider}      → revoke a stored credential
//! - `GET  /providers`                    → provider picker data (no secrets)
//! - `GET  /providers/:name/models`   → models the stored key can reach
//! - `GET  /providers/:name/status`   → provider-published account metadata
//! - `PATCH/DELETE /memory/:note_id`  → amend / forget one memory note
//! - `GET  /search?all=true`          → cross-project recall (23-memory)
//! - `GET  /doctor?session=`          → HealthReport JSON (29-personal-os P3)
//! - `POST /backup/export`            → directory backup of the home dir
//! - `POST /backup/import`            → restore with skip-or-rename conflicts
//! - `GET  /digest?days=N`            → usage digest over the trailing window
//! - `GET  /inbox?limit=&unread=true` → inbox entries + unread count (29-personal-os P6)
//! - `POST /inbox/:id/ack`            → idempotent read-state tombstone
//! - `GET  /inbox/unread_count`       → live unread total
//! - `POST /gateway/inbound`          → surface message routed to its bound session (22-gateway)
//! - `POST /voice/transcribe`         → bounded provider-routed batch transcription
//! - `GET  /gateway/status`           → gateway enabled flag + binding table
//! - `DELETE /gateway/bindings/:key`  → unbind a surface from its session
//! - `POST /agent-network/capabilities` → issue an explicitly scoped agent capability
//! - `POST /agent-network/messages`      → broker a bounded workspace message
//! - `GET  /agent-network/messages`      → receive queued workspace messages
//! - `GET/POST /presentations`           → inspect/register validated experience-pack records
//! - `POST /presentations/revisions`    → validate and store a disabled immutable revision preview
//! - `POST /presentations/:id/:revision/activate` → explicitly activate one scoped revision
//! - `POST /presentations/:id/deactivate` → remove one scoped activation
//! - `DELETE /presentations/plugins/:plugin_id` → revoke a plugin's presentation records

/// Pin `VAK_HOME` to one throwaway directory for this whole test binary.
///
/// `data_home()` backs the workspace-trust marker store, and a test that
/// approves a channel or records a trust decision writes into it. Without
/// this pin those writes land in the developer's real
/// `~/Library/Application Support/vak/trusted` and stay there, one orphan
/// marker per tempdir, quietly granting trust to paths that no longer
/// exist. `sessions_home` is already isolated per test for exactly this
/// reason; the data home was not.
///
/// Process-global by nature, so it is set once and leaked: `VAK_HOME` has
/// no scope smaller than the process, and unsetting it while parallel tests
/// are running would be worse than pinning it.
#[cfg(test)]
pub(crate) fn pin_test_data_home() {
    use std::sync::OnceLock;
    static HOME: OnceLock<std::path::PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        #[allow(clippy::expect_used)]
        let dir = tempfile::tempdir().expect("test data home");
        let path = dir.keep();
        vak_config::set_override("VAK_HOME", path.to_string_lossy().to_string());
        path
    });
}

mod admin;
mod admin_ui;
mod agent_chats;
pub mod agents;
mod bus;
mod channels;
mod client_ui;
mod core_pool;
mod delivery;
mod embedded_ui;
mod events;
mod feeds;
pub mod gateway;
mod heartbeat;
mod operations;
mod projection;
mod rate_limit;
mod service_control;
mod site;
pub mod surfaces;
mod web;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod test_support {
    /// A throwaway `AppState` rooted in a temp dir.
    ///
    /// `set_sessions_home` is not optional: without it `sessions_home()`
    /// falls back to the developer's real data home, and `AppState::new`
    /// loads (and can seed) the gateway allowlist store there.
    pub(crate) fn state() -> crate::AppState {
        let dir = tempfile::tempdir().unwrap();
        let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        // The TempDir guard is deliberately leaked: these states outlive
        // the call and a removed directory would fail reads mid-test.
        std::mem::forget(dir);
        crate::AppState::new(core)
    }
}

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, Approver, SteeringQueues};
use vak_core::Core;
pub use vak_core::tasks::{TaskDef, WtMeta};
use vak_llm::Provider;
use vak_plugin::{InstallOptions, InstallScope, MarketplaceTrust, PluginStore, SignatureEvidence};
use vak_session::SessionLog;

/// Ceiling on simultaneously-cached session handles. Generous on purpose:
/// eviction should be invisible to interactive use and only bound a
/// long-running gateway process.
const MAX_LIVE_SESSIONS: usize = 128;

pub(crate) struct SessionHandle {
    pub(crate) id: String,
    /// The `Core` this session runs under, resolved once at creation.
    ///
    /// A session's contract freezes when it is created (invariant 17), and
    /// its permission ceiling is part of that contract — so it belongs on
    /// the handle rather than being re-read from `state.core` at dispatch.
    /// The gateway already passes its pooled `Core` explicitly into turn
    /// execution; this is the same fact, held where every run path can see
    /// it, which is what lets a session be capped *below* the workspace
    /// mode (doc 46 security invariant 5).
    pub(crate) core: Core,
    /// Workspace this session's tools/diffs operate in (main cwd, or a
    /// best-of-N worktree).
    pub(crate) cwd: PathBuf,
    pub(crate) session: Arc<Mutex<Option<SessionLog>>>,
    /// Latest host-admitted intent, retained while the runner owns the log.
    pub(crate) intent: Arc<Mutex<Option<vak_session::types::IntentRecord>>>,
    pub(crate) steering: Arc<SteeringQueues>,
    /// Cancel for the CURRENT run only; replaced with a fresh token when a
    /// run ends so one `/cancel` doesn't poison every later run.
    pub(crate) cancel: Arc<std::sync::Mutex<CancellationToken>>,
    /// Live events for the MAIN transcript, with replay so a dropped
    /// connection can resume rather than lose the gap (events::EventBus).
    pub(crate) events_tx: events::EventBus,
    /// Pending approval gates scoped to THIS session — a client holding
    /// session A can never resolve session B's approvals.
    pub(crate) pending: Arc<Mutex<HashMap<String, ApprovalRequest>>>,
    /// Lifecycle facts produced while the runner owns the SessionLog. They
    /// are appended atomically when the runner returns the ledger.
    pub(crate) activity_buffer: Arc<Mutex<Vec<vak_session::ActivityRecord>>>,
    /// Reconnectable presentation state while the runner owns the ledger.
    pub(crate) presentation: Arc<Mutex<vak_delivery::OutputTimeline>>,
    /// Notified when an SSE consumer attaches, so runs don't start (and
    /// finish) before anyone is listening.
    pub(crate) subscribed: Arc<tokio::sync::Notify>,
    /// Side-chat stream + cancel: branched turns that read the session
    /// context but never land on the main chain.
    /// Live events for the `/btw` side branch. Same bus type as the
    /// main transcript so both resume identically.
    pub(crate) side_events_tx: events::EventBus,
    /// Last time a request resolved this handle, for idle eviction.
    pub(crate) last_touched: Mutex<std::time::Instant>,
    pub(crate) side_cancel: Arc<std::sync::Mutex<CancellationToken>>,
    /// Admission identities currently owned by this handle. This closes the
    /// retry race while the runner owns the ledger.
    pub(crate) admissions: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone)]
pub struct AppState {
    pub core: Core,
    /// Monotonic start point for this server state. Keeping it on the state
    /// avoids reporting the first Operations request as process start and
    /// keeps embedded/test routers independent from one another.
    started_at: Instant,
    /// Port used by the bound server. Plain routers use the configured
    /// operations default; `serve_with` overwrites this with its actual
    /// listener port so health probes and the console never drift from the
    /// process being inspected.
    ops_port: u16,
    sessions: Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>,
    /// Live best-of-N runs keyed by child session id.
    pub(crate) best_runs: Arc<Mutex<HashMap<String, BestRunMeta>>>,
    /// Scheduled tasks for this workspace (store shape owned by vak-core).
    tasks: Arc<Mutex<HashMap<String, TaskDef>>>,
    /// In-memory cron markers: task id → next scheduled local fire. Interval
    /// tasks keep using `last_run_at`; only `schedule:` tasks appear here.
    next_fire: Arc<Mutex<HashMap<String, chrono::DateTime<chrono::Local>>>>,
    /// Script tasks currently executing (no child session to inspect, so
    /// this stands in for the busy-check that prompt tasks get).
    script_inflight: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Managed dev servers (preview pane), keyed by session::name.
    procs: Arc<Mutex<HashMap<String, ManagedProc>>>,
    /// Gateway surface bindings + enable gate (docs/design/22-gateway.md).
    pub(crate) gateway: Arc<gateway::GatewayState>,
    /// Proactive heartbeat runtime (docs/design/29-personal-os.md P7).
    pub(crate) heartbeat: Arc<heartbeat::HeartbeatRuntime>,
    /// Global event hub for admin console SSE streaming.
    pub(crate) hub: events::EventHub,
    /// SQLite FTS5 session index (rebuildable from JSONL).
    pub(crate) store: Option<vak_store::Store>,
    /// Expected auth token (login endpoint compares against it).
    pub(crate) auth_token: Arc<String>,
    /// Workspace the browser client currently has open, when it has moved
    /// away from the one this process started in (docs/design/48-web-client.md
    /// §5). `None` means "the process's own workspace".
    ///
    /// Only *new* sessions are affected: a session freezes its `Core` at
    /// creation (invariant 17), so switching the active workspace never
    /// retargets work already under way — it decides where the next task
    /// will live, which is exactly what an operator switching projects
    /// means by it.
    pub(crate) active_core: Arc<Mutex<Option<Core>>>,
    /// Number of live voice websocket sessions. Admission is checked against
    /// the effective configuration at connection time and released on exit.
    pub(crate) voice_active: Arc<std::sync::atomic::AtomicUsize>,
    /// Rolling admission window for batch voice endpoints.
    pub(crate) voice_requests: Arc<Mutex<(Instant, usize)>>,
}

#[derive(Clone)]
pub struct BestRunMeta {
    pub repo: PathBuf,
    pub wt_path: PathBuf,
    pub branch: String,
}

impl AppState {
    pub fn new(core: Core) -> Self {
        let gateway = Arc::new(gateway::GatewayState::load(&core, false));
        let hub = events::init_global();
        // Canonical layout (doc 32): the FTS index is a rebuildable cache,
        // never user data — it lives under Library/Caches / XDG_CACHE_HOME.
        let store = vak_store::Store::open(&core.cache_home()).ok();
        if store.is_none() {
            eprintln!("[warn] store open failed, search will use fallback");
        }
        // Token selection lives here so every router flavor (plain,
        // gateway, secured) shares one identity for auth + login.
        let auth_token = Arc::new(
            std::env::var("VAK_GATEWAY_TOKEN")
                .ok()
                .filter(|t| !t.trim().is_empty())
                .or_else(|| vak_config::get_var("VAK_GATEWAY_TOKEN"))
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| format!("vk_{}", uuid::Uuid::now_v7())),
        );
        AppState {
            core,
            started_at: Instant::now(),
            ops_port: vak_ops::OpsConfig::detect().port,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            best_runs: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            next_fire: Arc::new(Mutex::new(HashMap::new())),
            script_inflight: Arc::new(Mutex::new(std::collections::HashSet::new())),
            procs: Arc::new(Mutex::new(HashMap::new())),
            gateway,
            heartbeat: Arc::new(heartbeat::HeartbeatRuntime::new()),
            hub,
            store,
            auth_token,
            active_core: Arc::new(Mutex::new(None)),
            voice_active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            voice_requests: Arc::new(Mutex::new((Instant::now(), 0))),
        }
    }

    /// The `Core` new work should run under: the browser's chosen
    /// workspace if it has picked one, else this process's own.
    pub(crate) fn active_core(&self) -> Core {
        self.active_core
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| self.core.clone())
    }

    /// Force-enable the gateway (`serve --gateway`) before the state is
    /// shared; the config gate alone governs every other entry point.
    pub fn enable_gateway(&mut self) {
        if let Some(gw) = Arc::get_mut(&mut self.gateway) {
            gw.set_enabled(true);
        }
    }

    fn get(&self, id: &str) -> Option<Arc<SessionHandle>> {
        let handle = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned();
        if let Some(handle) = &handle
            && let Ok(mut touched) = handle.last_touched.lock()
        {
            *touched = std::time::Instant::now();
        }
        handle
    }

    /// Drop the least-recently-touched idle sessions once the live set exceeds
    /// [`MAX_LIVE_SESSIONS`].
    ///
    /// The map was insert-only. Each handle pins the whole ledger in memory
    /// (`Vec<Entry>` of every message, tool result, and receipt) plus a
    /// presentation snapshot and two broadcast channels, so a long-lived
    /// gateway process grew without bound — and because `SessionLog::open`
    /// holds an exclusive file lock for the handle's lifetime, every session
    /// the daemon ever touched stayed locked against the CLI.
    ///
    /// Eviction is deliberately conservative: a session is only a candidate
    /// when nothing else holds a reference, no SSE client is subscribed, and
    /// the runner is not holding the ledger. `/sessions/{id}/attach` re-opens
    /// an evicted session from disk, so this is a cache bound, not a
    /// lifecycle.
    fn evict_idle_sessions(&self) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sessions.len() <= MAX_LIVE_SESSIONS {
            return;
        }
        let mut idle: Vec<(std::time::Instant, String)> = sessions
            .iter()
            .filter(|(_, handle)| {
                Arc::strong_count(handle) == 1
                    && handle.events_tx.receiver_count() == 0
                    && handle.side_events_tx.receiver_count() == 0
                    && handle.session.lock().is_ok_and(|guard| guard.is_some())
            })
            .filter_map(|(id, handle)| {
                let touched = *handle.last_touched.lock().ok()?;
                Some((touched, id.clone()))
            })
            .collect();
        idle.sort_by_key(|(touched, _)| *touched);
        let mut over = sessions.len().saturating_sub(MAX_LIVE_SESSIONS);
        for (_, id) in idle {
            if over == 0 {
                break;
            }
            sessions.remove(&id);
            over -= 1;
        }
    }

    /// Snapshot of every live session handle (admin surfaces aggregate
    /// across sessions; nothing here crosses a session's approval scope —
    /// answering still goes through the per-session endpoint).
    pub(crate) fn live_handles(&self) -> Vec<Arc<SessionHandle>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }
}

#[derive(Clone)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool: String,
    pub args_json: String,
    pub reason: String,
    pub requested_at: chrono::DateTime<chrono::Utc>,
    respond: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
}

impl ApprovalRequest {
    pub fn respond(&self, approve: bool) {
        if let Some(tx) = self
            .respond
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = tx.send(approve);
        }
    }
}

/// How long an HTTP-surfaced gate waits for a console or desktop client to
/// answer before failing closed.
///
/// There was no bound at all, which was survivable while every run behind
/// this approver had a human watching an SSE stream — and was not, once the
/// scheduler started firing runs through the same path. An unanswered gate
/// held the session handle open forever, so the routine never completed and
/// its slot never freed. Generous, because a person may genuinely be away
/// from the tab, but finite: a run that fails closed can be retried, and one
/// that hangs cannot.
const HTTP_APPROVAL_TIMEOUT: Duration = Duration::from_secs(900);

struct HttpApprover {
    events_tx: events::EventBus,
    pending: Arc<Mutex<HashMap<String, ApprovalRequest>>>,
    /// Owning session, so admin-console surfaces can attribute gates.
    session_id: String,
    activity_buffer: Arc<Mutex<Vec<vak_session::ActivityRecord>>>,
    /// False when this run has no client watching — a scheduled routine, a
    /// best-of-N leg. The gate is then a foregone denial, and saying so
    /// through `answerable()` is what lets `vak_core::reach` drop the
    /// capability from the turn instead of letting the model discover it by
    /// blocking on a question nobody will read.
    answerable: bool,
}

#[async_trait::async_trait]
impl Approver for HttpApprover {
    fn answerable(&self) -> bool {
        self.answerable
    }

    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool {
        if !self.answerable {
            return false;
        }
        let id = uuid::Uuid::now_v7().to_string();
        let (respond, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.clone(),
                ApprovalRequest {
                    id: id.clone(),
                    tool: tool.to_string(),
                    args_json: args_json.to_string(),
                    reason: reason.to_string(),
                    requested_at: chrono::Utc::now(),
                    respond: Arc::new(Mutex::new(Some(respond))),
                },
            );
        let _ = self.events_tx.send(AgentEvent::ApprovalRequested {
            id: id.clone(),
            tool: tool.to_string(),
            args_json: args_json.to_string(),
            reason: reason.to_string(),
        });
        self.activity_buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(vak_session::ActivityRecord {
                activity_id: format!("approval-{id}"),
                turn: None,
                kind: vak_session::ActivityKind::Approval,
                status: vak_session::ActivityStatus::Pending,
                label: format!("Approval required for {tool}"),
                detail: Some(reason.to_string()),
                data: [
                    ("request_id".into(), id.clone()),
                    ("tool".into(), tool.to_string()),
                    ("args_json".into(), args_json.to_string()),
                ]
                .into(),
            });
        if let Some(hub) = events::global() {
            hub.emit(events::SystemEvent::ApprovalRequested {
                id: id.clone(),
                session_id: self.session_id.clone(),
                tool: tool.to_string(),
                reason: reason.to_string(),
            });
        }
        // Bounded, and failing closed on expiry — the same contract
        // `GatewayApprover` already had. Dropping the entry before returning
        // means a reply that arrives after the deadline resolves nothing
        // rather than answering a gate the run has already moved past.
        let approved = match tokio::time::timeout(HTTP_APPROVAL_TIMEOUT, rx).await {
            Ok(answer) => answer.unwrap_or(false),
            Err(_) => {
                eprintln!(
                    "[approvals] gate {} for `{tool}` expired after {}s; denied",
                    &id[..8.min(id.len())],
                    HTTP_APPROVAL_TIMEOUT.as_secs()
                );
                false
            }
        };
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
        self.activity_buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(vak_session::ActivityRecord {
                activity_id: format!("approval-{id}"),
                turn: None,
                kind: vak_session::ActivityKind::Approval,
                status: if approved {
                    vak_session::ActivityStatus::Succeeded
                } else {
                    vak_session::ActivityStatus::Denied
                },
                label: format!(
                    "Approval {} for {tool}",
                    if approved { "granted" } else { "denied" }
                ),
                detail: Some(reason.to_string()),
                data: [
                    ("request_id".into(), id.clone()),
                    ("tool".into(), tool.to_string()),
                    ("args_json".into(), args_json.to_string()),
                ]
                .into(),
            });
        if let Some(hub) = events::global() {
            hub.emit(if approved {
                events::SystemEvent::ApprovalGranted {
                    id: id.clone(),
                    tool: tool.to_string(),
                }
            } else {
                events::SystemEvent::ApprovalDenied {
                    id,
                    tool: tool.to_string(),
                }
            });
        }
        approved
    }
}

pub fn router(core: Core) -> Router {
    router_with_state(AppState::new(core))
}

/// Unauthenticated router with the gateway force-enabled and no background
/// scheduler. For embedders that run their own supervision loop and need
/// clean teardown: dropping this router releases every session lock,
/// whereas `secured_router`'s scheduler pins handles until process exit.
pub fn gateway_router(core: Core) -> Router {
    let mut state = AppState::new(core);
    state.enable_gateway();
    router_with_state(state)
}

fn router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}/attach", post(attach_session))
        .route("/sessions/{id}/diff", get(session_diff))
        .route("/sessions/{id}/receipts", get(session_receipts))
        .route(
            "/sessions/{id}/work",
            get(session_work).post(session_work_command),
        )
        .route("/sessions/{id}/work/confirm", post(session_work_confirm))
        .route("/sessions/{id}/work/revise", post(session_work_revise))
        .route(
            "/sessions/{id}/work/items/{item_id}/retry",
            post(session_work_retry),
        )
        .route(
            "/sessions/{id}/work/items/{item_id}/cancel",
            post(session_work_cancel_item),
        )
        .route(
            "/sessions/{id}/work/items/{item_id}/reassign",
            post(session_work_reassign),
        )
        .route("/flows", get(flows_list))
        .route("/flows/{name}/runs", get(flow_runs_list))
        .route("/flows/{name}/runs/{run}/graph", get(flow_run_graph))
        .route("/sessions/{id}/checkpoints", get(list_checkpoints))
        .route(
            "/sessions/{id}/checkpoints/{seq}/restore",
            post(restore_checkpoint),
        )
        .route("/sessions/{id}/archive", post(set_archived))
        .route("/sessions/archived", delete(delete_all_archived))
        .route("/sessions/{id}", delete(delete_session))
        .route("/skills", get(list_skills))
        .route("/commands", get(list_commands))
        .route("/plugins", get(list_plugins))
        .route("/plugins/catalog", get(plugin_catalog))
        .route("/plugins/audit", get(plugin_audit))
        .route("/plugins/invocations", get(plugin_invocations))
        .route(
            "/presentations",
            get(list_presentations).post(register_presentations),
        )
        .route(
            "/presentations/primitives",
            get(list_presentation_primitives),
        )
        .route(
            "/presentations/specs/{id}/{revision}",
            get(get_presentation_spec),
        )
        .route(
            "/presentations/revisions",
            post(propose_presentation_revision),
        )
        .route(
            "/sessions/{id}/presentation/proposals",
            post(propose_session_presentation_revision),
        )
        .route("/presentations/export", get(export_presentations))
        .route("/presentations/import", post(import_presentations))
        .route(
            "/presentations/activate-all",
            post(activate_all_presentations),
        )
        .route(
            "/presentations/deactivate-all",
            post(deactivate_all_presentations),
        )
        .route(
            "/presentations/{id}/{revision}/activate",
            post(activate_presentation),
        )
        .route(
            "/presentations/{id}/deactivate",
            post(deactivate_presentation),
        )
        .route("/presentations/{id}/reset", post(reset_presentation))
        .route(
            "/presentations/plugins/{plugin_id}",
            delete(revoke_presentations_plugin),
        )
        .route(
            "/plugins/retired",
            get(list_retired_plugins).delete(remove_retired_plugins),
        )
        .route(
            "/plugins/sources",
            get(plugin_sources).post(plugin_register_source),
        )
        .route("/plugins/sources/{id}/enable", post(plugin_source_enable))
        .route("/plugins/sources/{id}/disable", post(plugin_source_disable))
        .route("/plugins/keys/{id}/revoke", post(plugin_key_revoke))
        .route("/plugins/keys/{id}/restore", post(plugin_key_restore))
        .route("/plugins/install", post(plugin_install))
        .route("/plugins/update", post(plugin_update))
        .route("/plugins/{name}/enable", post(plugin_enable))
        .route("/plugins/{name}/disable", post(plugin_disable))
        .route("/plugins/{name}/rollback", post(plugin_rollback))
        .route("/plugins/{name}", delete(plugin_remove))
        .route("/sessions/{id}/pr", get(session_pr))
        .route("/sessions/{id}/pr/merge", post(pr_merge))
        .route("/tasks", get(list_tasks).post(create_task))
        .route(
            "/tasks/{id}",
            axum::routing::patch(patch_task).delete(delete_task),
        )
        .route("/tasks/{id}/run-now", post(run_task_now))
        .route("/tasks/{id}/retry-delivery", post(retry_task_delivery))
        .route("/sessions/{id}/launch", get(get_launch))
        .route("/sessions/{id}/launch/start", post(start_launch))
        .route("/sessions/{id}/launch/stop", post(stop_launch))
        .route("/sessions/{id}/launch/logs", get(launch_logs))
        .route("/sessions/{id}/run", post(run_prompt))
        .route("/sessions/{id}/steering", post(send_steering))
        .route("/sessions/{id}/cancel", post(cancel_run))
        .route("/sessions/{id}/pause", post(pause_run))
        .route("/sessions/{id}/resume", post(resume_run))
        .route("/sessions/{id}/control-state", get(control_state))
        .route("/sessions/{id}/plan-change", post(plan_change))
        .route("/sessions/{id}/workers", get(list_workers))
        .route("/sessions/{id}/workers/{child}/steer", post(steer_worker))
        .route("/sessions/{id}/workers/{child}/stop", post(stop_worker))
        // Backward-compatible aliases for the old `subagents` route names.
        .route("/sessions/{id}/subagents", get(list_workers))
        .route("/sessions/{id}/subagents/{child}/steer", post(steer_worker))
        .route("/sessions/{id}/subagents/{child}/stop", post(stop_worker))
        .route("/sessions/{id}/approvals/{req_id}", post(answer_approval))
        .route("/sessions/{id}/outcome-review", post(record_outcome_review))
        .route("/sessions/{id}/events", get(events_sse))
        .route(
            "/sessions/{id}/sandbox/executions",
            get(session_sandbox_executions),
        )
        .route("/sessions/{id}/presentation", get(presentation_snapshot))
        .route("/sessions/{id}/results/{result_id}", get(session_result))
        .route(
            "/sessions/{id}/presentation/feedback",
            post(presentation_feedback),
        )
        .route(
            "/sessions/{id}/presentation/select",
            post(select_presentation_for_session),
        )
        .route(
            "/sessions/{id}/presentation/events",
            get(presentation_events_sse),
        )
        .route("/sessions/{id}/transcript", get(transcript))
        .route("/sessions/{id}/transcript.md", get(transcript_markdown))
        .route("/sessions/{id}/side", post(side_chat))
        .route("/sessions/{id}/side/events", get(side_events_sse))
        .route("/sessions/{id}/side/cancel", post(side_cancel_run))
        .route("/sessions/{id}/bestofn", post(start_bestofn))
        .route("/sessions/{id}/keep", post(keep_best_run))
        .route("/sessions/{id}/discard", post(discard_best_run))
        .route("/fs/file", get(read_file).put(write_file))
        .route("/fs/file/raw", get(read_file_raw))
        .route("/fs/preview/{*path}", get(preview_file))
        .route(
            "/sandbox/records",
            get(list_sandbox_records).post(append_sandbox_record),
        )
        .route("/sandbox/candidates", post(export_sandbox_candidate))
        .route("/sandbox/promote", post(promote_sandbox_candidate))
        .route("/fs/tree", get(fs_tree))
        .route("/config", get(get_config).patch(patch_config))
        .route("/config/intent/evidence", post(patch_evidence_policy))
        .route(
            "/config/global",
            get(get_global_config_layer).patch(patch_global_config),
        )
        .route("/config/workspace", get(get_workspace_config_layer))
        .route("/config/project", get(get_workspace_config_layer))
        .route("/config/mode", post(set_permission_mode))
        .route(
            "/agent-network/capabilities",
            post(agent_network_capability),
        )
        .route(
            "/agent-network/messages",
            post(agent_network_send).get(agent_network_receive),
        )
        .route("/config/mcp", get(get_mcp_servers).put(put_mcp_servers))
        .route(
            "/config/mcp/global",
            get(get_global_mcp_servers).put(put_global_mcp_servers),
        )
        .route("/config/integrations", get(get_integration_catalog))
        .route(
            "/config/integrations/{id}",
            get(get_scoped_integration)
                .put(put_scoped_integration)
                .delete(delete_scoped_integration),
        )
        .route("/config/hooks", get(get_hooks).put(put_hooks))
        .route(
            "/config/prompts",
            get(get_prompt_layer).put(put_prompt_block),
        )
        .route("/config/prompts/effective", get(get_prompt_effective))
        .route("/config/prompts/preview", post(preview_prompt))
        .route("/config/prompts/roles", get(list_prompt_roles))
        .route("/agents", get(agent_chats::list))
        .route("/agents/{agent}/open", post(agent_chats::open))
        .route("/config/agents", get(get_agents).put(put_agents))
        .route(
            "/config/hooks/global",
            get(get_global_hooks).put(put_global_hooks),
        )
        .route(
            "/config/key",
            put(put_provider_key).delete(delete_provider_key),
        )
        // Distributed event bus status (vak-bus, docs/design/53).
        // Credentials are never returned; only the connection state and
        // metrics are exposed.
        .route(
            "/config/bus",
            get(get_bus_config)
                .put(put_bus_config)
                .delete(delete_bus_config),
        )
        // The approval policy: whether an `Ask` raised on an unattended
        // chat surface reaches a human at all. Read-only everywhere until
        // now, which made `vak_core::reach`'s own printed remedy an action
        // no surface could perform.
        .route(
            "/gateway/approvals",
            get(get_gateway_approvals).put(put_gateway_approvals),
        )
        // The three permission rule lists, as the engine evaluates them.
        .route(
            "/config/permissions",
            get(get_permission_rules).put(put_permission_rules),
        )
        // Multi-bot-per-surface (docs/design/34, multi-bot): a `Bot` is an
        // independent identity, so it gets its own id-addressed routes
        // rather than reusing the one-slot-per-surface ones above.
        .route("/gateway/bots", get(list_bots).post(create_bot))
        .route(
            "/gateway/bots/{id}",
            axum::routing::patch(update_bot).delete(delete_bot),
        )
        .route(
            "/gateway/bots/{id}/token",
            put(put_bot_id_token).delete(delete_bot_id_token),
        )
        .route("/providers", get(list_providers))
        .route("/providers/{name}/models", get(discover_models))
        .route(
            "/providers/{name}/models/availability",
            get(model_availability),
        )
        .route("/providers/{name}/status", get(provider_status))
        .route("/search", get(search_sessions))
        .route("/ops/status", get(ops_status))
        .route("/ops/center", get(operations_center))
        .route("/ops/actions", get(operations_actions))
        .route("/ops/incidents", get(operations_incidents))
        .route("/ops/outbox", get(operations_outbox))
        .route(
            "/ops/outbox/{job_id}/replay",
            post(replay_operations_outbox),
        )
        .route("/ops/{service}/{action}", post(ops_action))
        .route("/ops/diagnostics", get(ops_diagnostics))
        // Activation, explicitly (docs/design/46 D6). Configuration writes
        // never register a service; this is the one action that does.
        .route("/ops/services/activate", post(activate_services))
        .route("/finops", get(finops_status).patch(patch_finops))
        .route("/voice/speak", post(voice_speak))
        .route("/voice/transcribe", post(voice_transcribe))
        .route("/voice/providers", get(list_voice_providers))
        .route("/memory", get(list_memory).post(append_memory))
        .route("/memory/cleanup", post(cleanup_memory))
        .route("/memory/consolidate", post(consolidate_memory_route))
        .route(
            "/memory/{note_id}",
            axum::routing::patch(amend_memory_note).delete(forget_memory_note),
        )
        .route(
            "/entities",
            get(list_entities_route).post(upsert_entity_route),
        )
        .route(
            "/entities/{id}",
            get(get_entity_route).delete(delete_entity_route),
        )
        .route("/agents/templates", get(list_agent_templates))
        .route("/agents/instantiate", post(instantiate_agent_template))
        .route("/agents/{id}/schedule", post(update_agent_schedule_route))
        .route("/agents/{id}/runs", get(list_agent_runs_route))
        .route("/canvas/preview", post(canvas_preview))
        .route("/intent/explain", get(intent_explain))
        .route("/intent/policy", get(intent_policy))
        .route("/commitments", get(list_commitments))
        .route("/commitments/{id}", get(get_commitment))
        .route("/commitments/{id}/close", post(close_commitment))
        .route("/doctor", get(doctor_report))
        .route("/onboarding", get(onboarding_state))
        // The composition layer setup needs, and nothing more: every other
        // choice reuses the config, provider, integration, and bot APIs
        // that already exist (doc 46, "API and command design").
        .route("/onboarding/seed", post(onboarding_seed))
        .route(
            "/onboarding/workspace-review",
            post(onboarding_workspace_review),
        )
        .route("/onboarding/trust", post(onboarding_trust))
        .route("/onboarding/first-task", post(onboarding_first_task))
        // ---- browser auth (docs/design/48-web-client.md §4.3) -----------
        //
        // ONE login for every browser surface — the workspace client and
        // the operations console share this exchange and this cookie.
        // These replace `/admin/login` and `/admin/logout`, which were
        // removed rather than kept alongside: two endpoints against one
        // cookie is two contracts that must agree forever (invariant 30).
        .route("/auth/login", post(web::login))
        .route("/auth/logout", post(web::logout))
        .route("/auth/session", get(web::session_status))
        // ---- the workspace client's own host surface --------------------
        .route("/host", get(web::host_info))
        .route("/host/events", get(web::host_events))
        .route("/workspaces", get(web::list_workspaces))
        .route("/workspaces/open", post(web::open_workspace))
        .route("/workspaces/forget", post(web::forget_workspace))
        .route("/fs/dirs", get(web::list_dirs))
        .route("/pty", get(web::pty_socket))
        .route("/voice/session", get(web::voice_socket))
        .route("/version", get(web::version))
        .route("/backup/export", post(backup_export))
        .route("/backup/import", post(backup_import))
        .route("/digest", get(digest_report))
        .route("/inbox", get(inbox_list))
        .route("/inbox/unread_count", get(inbox_unread_count))
        .route("/inbox/{id}/ack", post(inbox_ack))
        .route("/skills/proposals", get(list_proposals_route))
        .route("/skills/proposals/{id}/promote", post(promote_proposal))
        .route("/skills/proposals/{id}/reject", post(reject_proposal))
        .merge(gateway::routes())
        .merge(feeds::routes())
        .merge(admin::routes())
        // Compression is scoped to the three STATIC bundles and nowhere
        // else. These are the big, highly compressible responses — a site
        // page is ~78 KB of inlined CSS and markup, the vendored motion
        // build is 141 KB, and the SPA bundles are larger still — and this
        // product is explicitly built to be reached over a tunnel, where
        // that is the whole first-visit cost.
        //
        // It is NOT applied to the API. `/sessions/:id/events` is
        // server-sent events: a compressor sits between the writer and the
        // socket, and a live transcript that arrives in buffer-sized
        // batches instead of per frame is a worse product than an
        // uncompressed one. Scoping it here rather than at the root is the
        // difference between a smaller page and a laggy agent.
        .merge(
            axum::Router::new()
                .merge(admin_ui::routes())
                .merge(client_ui::routes())
                .merge(site::routes())
                .layer(tower_http::compression::CompressionLayer::new().gzip(true)),
        )
        .with_state(state)
}

/// Service-control plane over vak-ops: lets TUI/desktop/tray agree on the
/// same truth (docs/design/28-operations.md).
fn ops_payload(cfg: &vak_ops::OpsConfig) -> serde_json::Value {
    let st = |svc| vak_ops::status(svc, cfg);
    serde_json::json!({
        "gateway": { "state": st(vak_ops::Service::Gateway).to_string() },
        "bridges": { "state": st(vak_ops::Service::Bridges).to_string() },
        "gateway_healthy": vak_ops::health_ok(cfg),
    })
}

async fn ops_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut cfg = vak_ops::OpsConfig::detect();
    cfg.port = state.ops_port;
    let payload = tokio::task::spawn_blocking(move || ops_payload(&cfg))
        .await
        .unwrap_or_else(|_| {
            serde_json::json!({
                "gateway": { "state": "unknown" },
                "telegram": { "state": "unknown" },
                "gateway_healthy": false,
            })
        });
    Json(payload)
}

/// Read-only operational projection for desktop/TUI surfaces. This keeps
/// service, gateway, flow and health state in one refreshable payload without
/// exposing credentials or implementation paths.
async fn ops_diagnostics(State(state): State<AppState>) -> Json<serde_json::Value> {
    refresh_control_plane(&state);
    let mut cfg = vak_ops::OpsConfig::detect();
    cfg.port = state.ops_port;
    let root = state.core.sessions_home().join("flow-runs");
    let mut flows = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten().filter(|e| e.path().is_dir()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let runs = std::fs::read_dir(entry.path())
                .map(|items| {
                    items
                        .flatten()
                        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                        .count()
                })
                .unwrap_or(0);
            flows.push(serde_json::json!({ "name": name, "runs": runs }));
        }
    }
    flows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let gateway = state.gateway.snapshot();
    let services = tokio::task::spawn_blocking(move || ops_payload(&cfg))
        .await
        .unwrap_or_else(|_| {
            serde_json::json!({
                "gateway": { "state": "unknown" },
                "telegram": { "state": "unknown" },
                "gateway_healthy": false,
            })
        });
    Json(serde_json::json!({
        "health": health_projection(&state),
        "services": services,
        "gateway": {
            "enabled": state.gateway.enabled,
            "bindings": gateway.into_iter().map(|(target, binding)| serde_json::json!({
                "target": target,
                "session_id": binding.session_id,
                "provider": binding.provider,
                "model": binding.model,
                "workspace": binding.workspace,
                "route_revision": binding.route_revision,
            })).collect::<Vec<_>>(),
            "approvals": {
                "mode": state.gateway.approvals_mode(),
                "approver": state.gateway.approver_target(),
                "pending": state.gateway.pending_approval_count(),
            },
        },
        "flows": flows,
    }))
}

fn operation_services(cfg: &vak_ops::OpsConfig) -> serde_json::Value {
    let gateway = vak_ops::status(vak_ops::Service::Gateway, cfg);
    let bridges = vak_ops::status(vak_ops::Service::Bridges, cfg);
    serde_json::json!({
        "gateway": { "state": gateway.to_string() },
        "bridges": { "state": bridges.to_string() },
        "gateway_healthy": vak_ops::health_ok(cfg),
    })
}

fn operation_runs(state: &AppState) -> Vec<serde_json::Value> {
    state
        .live_handles()
        .into_iter()
        .filter_map(|handle| {
            let active = handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none();
            let pending = handle
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .map(|request| {
                    serde_json::json!({
                        "id": request.id,
                        "tool": request.tool,
                        "reason": request.reason,
                        "requested_at": request.requested_at,
                    })
                })
                .collect::<Vec<_>>();
            if !active && pending.is_empty() {
                return None;
            }
            let (agent_id, agent_name) = handle
                .core
                .agent_identity()
                .map(|id| (id.id.clone(), id.name.clone()))
                .unwrap_or_else(|| ("vak".to_string(), "Vak".to_string()));
            Some(serde_json::json!({
                "session_id": handle.id,
                "workspace": handle.cwd,
                "agent_id": agent_id,
                "agent_name": agent_name,
                "state": if !pending.is_empty() { "waiting_approval" } else { "running" },
                "pending_approvals": pending,
            }))
        })
        .collect()
}

fn operation_tasks(state: &AppState) -> Vec<serde_json::Value> {
    let tasks = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let next_fire = state
        .next_fire
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let inflight = state
        .script_inflight
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    tasks
        .values()
        .map(|task| {
            let mut value = serde_json::to_value(task).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "next_fire".into(),
                    next_fire
                        .get(&task.id)
                        .map(|fire| serde_json::Value::String(fire.to_rfc3339()))
                        .unwrap_or(serde_json::Value::Null),
                );
                object.insert(
                    "running".into(),
                    serde_json::Value::Bool(inflight.contains(&task.id)),
                );
            }
            value
        })
        .collect()
}

fn operation_outbox(state: &AppState) -> Result<(Vec<serde_json::Value>, usize, usize), String> {
    let records = delivery::outbox_records(&state.core)?;
    let pending = records
        .iter()
        .filter(|record| record.state == vak_delivery::outbox::OutboxState::Pending)
        .count();
    let dead = records
        .iter()
        .filter(|record| record.state == vak_delivery::outbox::OutboxState::DeadLetter)
        .count();
    let rows = records
        .into_iter()
        .take(200)
        .map(|record| {
            serde_json::json!({
                "job_id": record.job.job_id,
                "target": record.job.target,
                "kind": record.job.kind,
                "state": record.state,
                "attempts": record.attempts,
                "created_at_ms": record.created_at_ms,
                "updated_at_ms": record.updated_at_ms,
                "last_error": record.last_error,
            })
        })
        .collect();
    Ok((rows, pending, dead))
}

/// Unified, evidence-backed projection for the Operations Center. Every row
/// is derived from an existing ledger, manager probe, or in-process handle;
/// unavailable state stays explicit instead of being painted green.
async fn operations_center(State(state): State<AppState>) -> Json<serde_json::Value> {
    refresh_control_plane(&state);
    load_tasks(&state);
    let health = health_projection(&state);
    let mut service_cfg = vak_ops::OpsConfig::detect();
    service_cfg.port = state.ops_port;
    let ops_port = service_cfg.port;
    let services = tokio::task::spawn_blocking(move || operation_services(&service_cfg))
        .await
        .unwrap_or_else(|_| {
            serde_json::json!({
                "gateway": { "state": "unknown" },
                "telegram": { "state": "unknown" },
                "gateway_healthy": false,
            })
        });
    let default_route = state.core.effective_route();
    let gateway = state.gateway.snapshot();
    let pool = state
        .gateway
        .core_pool
        .snapshot_at(Instant::now())
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "workspace": entry.workspace,
                "is_default": entry.is_default,
                "state": "warm",
                "idle_secs": entry.idle_secs,
                "permission_override": entry.permission_override,
                "effective_permission_mode": entry.effective_permission_mode,
            })
        })
        .collect::<Vec<_>>();
    let allowlist_snapshot = state.gateway.allowlist_snapshot();
    let allowlist_map: HashMap<String, String> = allowlist_snapshot
        .iter()
        .map(|e| {
            (
                e.key.clone(),
                e.agent_id.clone().unwrap_or_else(|| "vak".to_string()),
            )
        })
        .collect();
    let mut bound_targets = std::collections::HashSet::new();
    let mut bindings = gateway
        .into_iter()
        .map(|(target, binding)| {
            bound_targets.insert(target.clone());
            let agent_id = allowlist_map
                .get(&target)
                .cloned()
                .unwrap_or_else(|| "vak".to_string());
            serde_json::json!({
                "target": target,
                "session_id": binding.session_id,
                "workspace": binding.workspace,
                "agent_id": agent_id,
                "provider": binding
                    .provider
                    .unwrap_or_else(|| default_route.provider.clone()),
                "model": binding.model.unwrap_or_else(|| default_route.model.clone()),
                "route_revision": binding
                    .route_revision
                    .unwrap_or_else(|| default_route.revision.clone()),
            })
        })
        .collect::<Vec<_>>();
    for entry in allowlist_snapshot {
        if entry.status != gateway::AllowlistStatus::Allowed || bound_targets.contains(&entry.key) {
            continue;
        }
        bindings.push(serde_json::json!({
            "target": entry.key,
            "session_id": null,
            "workspace": state.gateway.workspace_for_entry(&state.core, &entry.key),
            "agent_id": entry.agent_id.as_deref().unwrap_or("vak"),
            "provider": entry.route.as_ref().map(|route| route.provider.clone()).unwrap_or_else(|| state.core.effective_provider()),
            "model": entry.route.as_ref().map(|route| route.model.clone()).unwrap_or_else(|| state.core.effective_model()),
            "route_revision": entry.route.as_ref().map(|route| format!("channel:{}:{}", route.provider, route.model)).unwrap_or_else(|| state.core.effective_route().revision),
            "cold": true,
        }));
    }
    let (outbox, outbox_pending, outbox_dead, outbox_error) = match operation_outbox(&state) {
        Ok((rows, pending, dead)) => (rows, pending, dead, None),
        Err(error) => (Vec::new(), 0, 0, Some(error)),
    };
    let runs = operation_runs(&state);
    let approvals = state
        .live_handles()
        .into_iter()
        .map(|handle| {
            handle
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        })
        .sum::<usize>();
    let security = vak_core::security_events::list(&state.core.sessions_home(), 30)
        .into_iter()
        .map(|event| serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})))
        .collect::<Vec<_>>();
    let workspace = Some(state.core.cwd().to_string_lossy().into_owned());
    let mut candidates = Vec::new();
    if health["failures"].as_u64().unwrap_or(0) > 0 {
        let evidence = health["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|check| check["status"] == "fail")
            .filter_map(|check| check["label"].as_str().map(str::to_string))
            .collect();
        candidates.push(operations::IncidentCandidate {
            fingerprint: "doctor:health-checks".to_string(),
            severity: "critical".to_string(),
            source: "doctor".to_string(),
            title: "Health checks need attention".to_string(),
            detail: format!("{} check(s) failed", health["failures"]),
            workspace: workspace.clone(),
            evidence,
        });
    }
    if approvals > 0 {
        candidates.push(operations::IncidentCandidate {
            fingerprint: "permission:pending-approvals".to_string(),
            severity: "warning".to_string(),
            source: "permission-engine".to_string(),
            title: "Runs are waiting for approval".to_string(),
            detail: format!("{approvals} approval gate(s) are blocking work"),
            workspace: workspace.clone(),
            evidence: runs
                .iter()
                .filter_map(|run| run["session_id"].as_str().map(|id| format!("session:{id}")))
                .collect(),
        });
    }
    if outbox_pending > 0 || outbox_dead > 0 {
        candidates.push(operations::IncidentCandidate {
            fingerprint: "delivery:outbox".to_string(),
            severity: if outbox_dead > 0 {
                "critical"
            } else {
                "warning"
            }
            .to_string(),
            source: "delivery".to_string(),
            title: "Outbound delivery needs attention".to_string(),
            detail: format!("{outbox_pending} pending, {outbox_dead} dead-lettered"),
            workspace: workspace.clone(),
            evidence: outbox
                .iter()
                .filter(|row| row["state"] != "delivered")
                .filter_map(|row| row["job_id"].as_str().map(|id| format!("outbox:{id}")))
                .collect(),
        });
    }
    if let Some(error) = &outbox_error {
        candidates.push(operations::IncidentCandidate {
            fingerprint: "delivery:outbox-read".to_string(),
            severity: "critical".to_string(),
            source: "delivery".to_string(),
            title: "Delivery evidence is unavailable".to_string(),
            detail: error.clone(),
            workspace: workspace.clone(),
            evidence: vec!["outbox:read".to_string()],
        });
    }
    if services["gateway"]["state"] != "running" && state.gateway.enabled {
        candidates.push(operations::IncidentCandidate {
            fingerprint: "service:gateway".to_string(),
            severity: "warning".to_string(),
            source: "service-manager".to_string(),
            title: "Gateway service is not running".to_string(),
            detail: services["gateway"]["state"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            workspace: workspace.clone(),
            evidence: vec!["service:gateway".to_string()],
        });
    }
    let incidents = operations::reconcile(&state.core.sessions_home(), candidates)
        .into_iter()
        .map(|incident| serde_json::to_value(incident).unwrap_or_else(|_| serde_json::json!({})))
        .collect::<Vec<_>>();
    let mut all_agents = vec![serde_json::json!({
        "id": "vak",
        "name": "Vak",
        "personality": "Codex-grade safety, pi-grade transparency, Claude Code-grade extensibility, opencode-grade simplicity.",
        "lifecycle": "active",
    })];
    if let Ok(custom) = agents::effective(&state.core) {
        for a in custom {
            all_agents.push(serde_json::json!({
                "id": a.id,
                "name": a.name,
                "personality": a.personality,
                "lifecycle": a.lifecycle,
            }));
        }
    }
    Json(serde_json::json!({
        "generated_at": Utc::now(),
        "server": {
            "pid": std::process::id(),
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_secs": state.started_at.elapsed().as_secs(),
            "cwd": state.core.cwd(),
            "posture": health["posture"],
        },
        "health": health,
        "services": services,
        "agents": all_agents,
        "gateway": {
            "enabled": state.gateway.enabled,
            "approvals": { "pending": approvals, "mode": state.gateway.approvals_mode(), "approver": state.gateway.approver_target() },
            "bindings": bindings,
            "workspace_catalog": crate::admin::workspace_catalog(&state),
        },
        "pool": {
            "max": state.core.config().gateway.core_pool_max,
            "idle_secs": state.core.config().gateway.core_pool_idle_secs,
            "entries": pool,
        },
        "runs": runs,
        "tasks": operation_tasks(&state),
        "outbox": { "pending": outbox_pending, "dead_letter": outbox_dead, "records": outbox, "error": outbox_error },
        "bus": state.hub.bus_status(),
        "security": security,
        "incidents": incidents,
        "actions": operations::recent_actions(&state.core.sessions_home(), 50),
        "ops_port": ops_port,
    }))
}

async fn operations_actions(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "actions": operations::recent_actions(&state.core.sessions_home(), 200),
    }))
}

async fn operations_incidents(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "incidents": operations::list(&state.core.sessions_home()),
    }))
}

async fn operations_outbox(State(state): State<AppState>) -> axum::response::Response {
    match delivery::outbox_records(&state.core) {
        Ok(records) => Json(serde_json::json!({
            "records": records.into_iter().take(500).map(|record| serde_json::json!({
                "job_id": record.job.job_id,
                "target": record.job.target,
                "kind": record.job.kind,
                "state": record.state,
                "attempts": record.attempts,
                "created_at_ms": record.created_at_ms,
                "updated_at_ms": record.updated_at_ms,
                "last_error": record.last_error,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn replay_operations_outbox(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> axum::response::Response {
    let state_label = |state: vak_delivery::outbox::OutboxState| match state {
        vak_delivery::outbox::OutboxState::Pending => "pending",
        vak_delivery::outbox::OutboxState::Delivered => "delivered",
        vak_delivery::outbox::OutboxState::DeadLetter => "dead_letter",
    };
    let before = delivery::outbox_records(&state.core)
        .ok()
        .and_then(|records| {
            records
                .into_iter()
                .find(|record| record.job.job_id == job_id)
        })
        .map(|record| state_label(record.state).to_string())
        .unwrap_or_else(|| "not found".to_string());
    let requested_at = Utc::now();
    match delivery::replay_outbox_job(&state.core, &job_id).await {
        Ok(()) => {
            let after_record = delivery::outbox_records(&state.core)
                .ok()
                .and_then(|records| {
                    records
                        .into_iter()
                        .find(|record| record.job.job_id == job_id)
                });
            let after = after_record
                .as_ref()
                .map(|record| state_label(record.state).to_string())
                .unwrap_or_else(|| "not found".to_string());
            if after == "delivered"
                && let Some(record) = after_record.as_ref()
                && let vak_delivery::DeliveryContent::Answer(answer) = &record.job.content
                && let Some(task_id) = answer.metadata.get("vak_task_id")
            {
                update_tasks(&state, |tasks| {
                    if let Some(task) = tasks.get_mut(task_id) {
                        task.last_delivery_state = Some("delivered".into());
                    }
                });
            }
            let verification_status = if after == "delivered" {
                "verified"
            } else {
                "pending"
            };
            let mut receipt = operations::ActionReceipt {
                receipt_id: format!("OP-{}", uuid::Uuid::now_v7().simple()),
                service: format!("outbox:{job_id}"),
                action: "replay".to_string(),
                requested_at,
                completed_at: Utc::now(),
                succeeded: true,
                verification: operations::ActionVerification {
                    status: verification_status.to_string(),
                    before,
                    after,
                    detail: if verification_status == "verified" {
                        "The adapter delivered the replayed job during the verification probe."
                    } else {
                        "Replay was accepted; the durable outbox record remains pending until the adapter confirms delivery."
                    }
                    .to_string(),
                },
                persisted: false,
            };
            receipt.persisted =
                operations::record_action(&state.core.sessions_home(), &receipt).is_ok();
            Json(serde_json::json!({
                "ok": true,
                "job_id": job_id,
                "receipt_id": receipt.receipt_id,
                "receipt_persisted": receipt.persisted,
                "verification": receipt.verification,
            }))
            .into_response()
        }
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

/// Trailing window for the admin console's spend trend chart — long enough
/// to show a real shape, short enough that a fixed-length zero-filled
/// series is cheap to compute on every request.
const FINOPS_TREND_DAYS: u32 = 14;

/// Default voice used when nothing in the bot/chat inheritance chain (nor
/// the request's own `voice_override`) names one — keeps `/voice/speak`
/// usable out of the box without any admin configuration.

#[derive(serde::Deserialize)]
struct VoiceSpeakBody {
    text: String,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    chat_key: Option<String>,
    #[serde(default)]
    voice_override: Option<vak_config::VoiceConfig>,
    /// Optional append-only ledger to attribute this synthesis to. Bridges
    /// may populate it after gateway admission; callers without a session
    /// remain fully supported.
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct VoiceTranscribeBody {
    audio_base64: String,
    #[serde(default = "default_voice_mime")]
    mime: String,
    #[serde(default)]
    session_id: Option<String>,
}
fn default_voice_mime() -> String {
    "audio/ogg".into()
}

fn admit_voice_request(state: &AppState, limit: usize) -> bool {
    let mut window = state
        .voice_requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if window.0.elapsed() >= std::time::Duration::from_secs(60) {
        *window = (Instant::now(), 0);
    }
    if window.1 >= limit {
        false
    } else {
        window.1 += 1;
        true
    }
}

/// `POST /voice/transcribe`: bounded batch transcription endpoint used by
/// channel bridges. Credentials and provider routing remain server-owned.
async fn voice_transcribe(
    State(state): State<AppState>,
    Json(body): Json<VoiceTranscribeBody>,
) -> axum::response::Response {
    use base64::Engine as _;
    let voice_settings = state.core.effective_voice();
    let audio_limit = voice_settings.max_audio_bytes.min(16 * 1024 * 1024);
    let audio = match base64::engine::general_purpose::STANDARD.decode(body.audio_base64.trim()) {
        Ok(bytes) if !bytes.is_empty() && (bytes.len() as u64) <= audio_limit => bytes,
        Ok(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("audio must be 1 byte to {} bytes", audio_limit)})),
            )
                .into_response();
        }
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":"audio_base64 is invalid"})),
            )
                .into_response();
        }
    };
    if !voice_settings.enabled {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":"voice is disabled"})),
        )
            .into_response();
    }
    // Admit only a valid, enabled request. Malformed or disabled requests must
    // not consume the caller's rolling voice quota.
    if !admit_voice_request(&state, voice_settings.max_requests_per_minute) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error":"voice request rate limit exceeded"})),
        )
            .into_response();
    }
    let provider = voice_settings
        .provider
        .as_deref()
        .unwrap_or("google")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        provider.as_str(),
        "google" | "gemini" | "google-live" | "gemini-live" | "openai" | "openai-compatible"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("voice provider '{provider}' has no transcription adapter installed")})),
        ).into_response();
    }
    let key = if matches!(provider.as_str(), "openai" | "openai-compatible") {
        vak_config::get_var("OPENAI_API_KEY")
    } else {
        vak_config::get_var("GEMINI_API_KEY").or_else(|| vak_config::get_var("GOOGLE_API_KEY"))
    };
    let Some(api_key) = key else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"no credential configured for the selected voice provider"})),
        )
            .into_response();
    };
    let model = voice_settings
        .transcription_model
        .or(voice_settings.model)
        .unwrap_or_default();
    let cancel = tokio_util::sync::CancellationToken::new();
    let result = if matches!(provider.as_str(), "openai" | "openai-compatible") {
        let config = vak_llm::openai::OpenAiConfig {
            api_key,
            base_url: vak_llm::openai::OPENAI_DEFAULT_BASE_URL.into(),
        };
        vak_llm::openai::transcribe(&config, &audio, &body.mime, &model, &cancel).await
    } else {
        let config = vak_llm::google_live::GoogleLiveConfig::new(api_key, &model);
        vak_llm::google_live::transcribe(&config, &audio, &body.mime, &cancel).await
    };
    match result {
        Ok(text) => {
            if let Some(session_id) = body
                .session_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                && let Ok(mut session) = state.core.open_session(session_id).await
            {
                let _ = session.append_voice_transcript(
                    uuid::Uuid::now_v7().to_string(),
                    text.clone(),
                    true,
                );
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"text": text, "provider":provider, "model": model})),
            )
                .into_response()
        }
        Err(error) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": error.to_string()})),
        )
            .into_response(),
    }
}

/// `POST /voice/speak`: synthesize `text` through the Gemini Live API using
/// the resolved voice/persona (explicit `voice_override` > the named
/// chat's/bot's resolved voice > a sensible built-in default), and return
/// raw WAV bytes. Guarded by the same `require_bearer` middleware every
/// other route on this router already sits behind.
async fn voice_speak(
    State(state): State<AppState>,
    Json(body): Json<VoiceSpeakBody>,
) -> axum::response::Response {
    if body.text.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "text must not be empty" })),
        )
            .into_response();
    }
    let voice_settings = state.core.effective_voice();
    if body.text.chars().count() > voice_settings.max_text_chars {
        return (StatusCode::PAYLOAD_TOO_LARGE, Json(serde_json::json!({"error": format!("text exceeds voice.max_text_chars ({})", voice_settings.max_text_chars)}))).into_response();
    }
    if !admit_voice_request(&state, voice_settings.max_requests_per_minute) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error":"voice request rate limit exceeded"})),
        )
            .into_response();
    }

    // Resolution order: explicit override > resolved chat/bot voice > a
    // provider-defined default. The chat lookup mirrors `core_for_entry`'s own
    // bot/chat fold (`GatewayState::resolve_voice`), and a bare `bot_id`
    // with no `chat_key` falls back to that bot's own tier directly.
    let resolved = body.voice_override.clone().or_else(|| {
        if let Some(chat_key) = body.chat_key.as_deref() {
            state.gateway.resolve_voice(chat_key)
        } else if let Some(bot_id) = body.bot_id.as_deref() {
            state.gateway.bot_get(bot_id).and_then(|b| b.voice)
        } else {
            None
        }
    });
    let voice_name = resolved
        .as_ref()
        .and_then(|v| v.voice_name.clone())
        .filter(|v| !v.trim().is_empty());
    // The persona now comes from the bot/chat `identity` prompt block; the
    // legacy `VoiceConfig.persona` is the fallback inside `resolve_persona`
    // (docs/design/45-prompt-layers.md). An explicit `voice_override` from
    // the caller — what the admin console's Preview button sends — still
    // wins, since it is the operator auditioning a value directly.
    let persona = body
        .voice_override
        .as_ref()
        .and_then(|v| v.persona.clone())
        .filter(|p| !p.trim().is_empty())
        .or_else(|| {
            if let Some(chat_key) = body.chat_key.as_deref() {
                state.gateway.resolve_persona(chat_key)
            } else if let Some(bot_id) = body.bot_id.as_deref() {
                state.gateway.resolve_bot_persona(bot_id)
            } else {
                None
            }
        });

    if !voice_settings.enabled {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "voice is disabled" })),
        )
            .into_response();
    }
    let provider = voice_settings
        .provider
        .as_deref()
        .unwrap_or("google")
        .trim()
        .to_ascii_lowercase();
    if provider == "local" {
        use vak_voice::{LocalTtsSpeaker, SpeakFormat, SpeakSpec, Speaker};
        let format = match body.format.as_deref().unwrap_or("wav") {
            "pcm" | "pcm16" => SpeakFormat::Pcm16,
            "wav" => SpeakFormat::Wav,
            "opus" | "ogg_opus" => SpeakFormat::OggOpus,
            "mp3" => SpeakFormat::Mp3,
            value => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("unsupported local speech format '{value}'")}))).into_response(),
        };
        let speaker = LocalTtsSpeaker::from_env();
        let mut stream = match speaker
            .speak(
                SpeakSpec {
                    text: body.text.clone(),
                    model: voice_settings
                        .synthesis_model
                        .clone()
                        .or_else(|| voice_settings.model.clone()),
                    voice: voice_name.clone(),
                    format,
                },
                CancellationToken::new(),
            )
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": error.to_string() })),
                )
                    .into_response();
            }
        };
        let mut pcm = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => pcm.extend_from_slice(&chunk.data),
                Err(error) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({ "error": error.to_string() })),
                    )
                        .into_response();
                }
            }
        }
        // The configured executable receives the requested format contract and
        // returns that encoded representation. Preserve it byte-for-byte here;
        // wrapping every response as WAV would corrupt PCM, Opus, and MP3.
        let audio = pcm;
        if let Some(session_id) = body
            .session_id
            .as_deref()
            .filter(|id| !id.trim().is_empty())
            && let Ok(mut session) = state.core.open_session(session_id).await
        {
            let _ = session.append_voice_playback(uuid::Uuid::now_v7().to_string(), 0, false);
        }
        let mut receipt = vak_llm::WorkReceipt::new(
            vak_llm::WorkPurpose::VoiceSynthesis,
            "local",
            voice_settings
                .synthesis_model
                .as_deref()
                .or(voice_settings.model.as_deref())
                .unwrap_or("offline"),
        );
        receipt.record(
            vak_llm::AttemptReason::Initial,
            vak_llm::FailureDomain::Unknown,
            vak_llm::Settlement::Ok,
            0,
            None,
            None,
        );
        let receipt_json = serde_json::to_string(&receipt).unwrap_or_else(|_| "{}".into());
        let content_type = match format {
            SpeakFormat::Pcm16 => "audio/pcm",
            SpeakFormat::Wav => "audio/wav",
            SpeakFormat::OggOpus => "audio/ogg",
            SpeakFormat::Mp3 => "audio/mpeg",
        };
        return (
            [
                (axum::http::header::CONTENT_TYPE, content_type),
                (
                    axum::http::header::HeaderName::from_static("x-vak-work-receipt"),
                    receipt_json.as_str(),
                ),
            ],
            audio,
        )
            .into_response();
    }
    if matches!(provider.as_str(), "openai" | "openai-compatible") {
        let Some(api_key) =
            vak_config::get_var("OPENAI_API_KEY").filter(|key| !key.trim().is_empty())
        else {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error":"no OPENAI_API_KEY configured for voice synthesis"}),
                ),
            )
                .into_response();
        };
        let Some(model) = voice_settings
            .synthesis_model
            .clone()
            .or_else(|| voice_settings.model.clone())
            .filter(|m| !m.trim().is_empty())
        else {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":"voice synthesis requires an explicitly discovered model"}))).into_response();
        };
        let format = body.format.as_deref().unwrap_or("mp3");
        let allowed = ["mp3", "opus", "aac", "flac", "wav", "pcm"];
        if !allowed.contains(&format) {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error":format!("unsupported OpenAI speech format '{format}'")}))).into_response();
        }
        let config = vak_llm::openai::OpenAiConfig {
            api_key,
            base_url: vak_llm::openai::OPENAI_DEFAULT_BASE_URL.into(),
        };
        let result = vak_llm::openai::speak(
            &config,
            &body.text,
            &model,
            voice_name.as_deref(),
            format,
            &CancellationToken::new(),
        )
        .await;
        return match result {
            Ok(audio) => {
                let mime = match format {
                    "mp3" => "audio/mpeg",
                    "opus" => "audio/ogg",
                    "aac" => "audio/aac",
                    "flac" => "audio/flac",
                    "pcm" => "audio/pcm",
                    _ => "audio/wav",
                };
                if let Some(session_id) = body
                    .session_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())
                    && let Ok(mut session) = state.core.open_session(session_id).await
                {
                    let _ =
                        session.append_voice_playback(uuid::Uuid::now_v7().to_string(), 0, false);
                }
                ([(axum::http::header::CONTENT_TYPE, mime)], audio).into_response()
            }
            Err(error) => (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error":error.to_string()})),
            )
                .into_response(),
        };
    }
    if !matches!(
        provider.as_str(),
        "google" | "gemini" | "google-live" | "gemini-live"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("voice provider '{provider}' has no synthesis adapter installed") })),
        ).into_response();
    }
    let api_key = match vak_config::get_var("GEMINI_API_KEY")
        .or_else(|| vak_config::get_var("GOOGLE_API_KEY"))
        .filter(|k| !k.trim().is_empty())
        .map(|k| k.trim().to_string())
    {
        Some(k) => k,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "no GEMINI_API_KEY/GOOGLE_API_KEY configured for voice synthesis"
                })),
            )
                .into_response();
        }
    };
    let mut config = vak_llm::google_live::GoogleLiveConfig::new(api_key, "");
    if let Some(model) = voice_settings
        .synthesis_model
        .or(voice_settings.model)
        .filter(|m| !m.trim().is_empty())
    {
        config.model = model;
    }
    let cancel = CancellationToken::new();
    let mut receipt = vak_llm::WorkReceipt::new(
        vak_llm::WorkPurpose::VoiceSynthesis,
        "google",
        &config.model,
    );
    let started = std::time::Instant::now();
    let result = vak_llm::google_live::speak(
        &config,
        &body.text,
        persona.as_deref(),
        voice_name.as_deref(),
        &cancel,
    )
    .await;

    match result {
        Ok(wav) => {
            if let Some(session_id) = body
                .session_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                && let Ok(mut session) = state.core.open_session(session_id).await
            {
                let _ = session.append_voice_playback(uuid::Uuid::now_v7().to_string(), 0, false);
            }
            receipt.record(
                vak_llm::AttemptReason::Initial,
                vak_llm::FailureDomain::Unknown,
                vak_llm::Settlement::Ok,
                started.elapsed().as_millis() as u64,
                None,
                None,
            );
            let mut response = (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "audio/wav")],
                wav,
            )
                .into_response();
            if let Ok(encoded) = serde_json::to_string(&receipt)
                && let Ok(value) = axum::http::HeaderValue::try_from(encoded)
            {
                response.headers_mut().insert("x-vak-work-receipt", value);
            }
            response
        }
        Err(e) => {
            let (domain, settlement) = vak_llm::work::classify_error(&e);
            receipt.record(
                vak_llm::AttemptReason::Initial,
                domain,
                settlement,
                started.elapsed().as_millis() as u64,
                None,
                Some(e.to_string()),
            );
            if cancel.is_cancelled() {
                receipt.settle_cancelled();
            }
            let status = match &e {
                vak_llm::LlmError::Auth(_) => StatusCode::BAD_REQUEST,
                vak_llm::LlmError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
                vak_llm::LlmError::RateLimit { .. } => StatusCode::TOO_MANY_REQUESTS,
                vak_llm::LlmError::Overloaded(_) => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::BAD_GATEWAY,
            };
            (status, Json(serde_json::json!({ "error": e.to_string() }))).into_response()
        }
    }
}

/// Capability catalogue for voice settings and administration. Credentials
/// and discovered model names are intentionally absent until a provider
/// client performs authenticated discovery.
async fn list_voice_providers() -> Json<serde_json::Value> {
    let registry = vak_voice::default_registry();
    let providers: Vec<_> = registry
        .list()
        .map(|descriptor| {
            let configured = descriptor
                .env_var
                .as_deref()
                .is_some_and(|name| std::env::var(name).is_ok());
            // Keep credentials out of the response while making setup state
            // actionable in Settings and administration.
            serde_json::json!({
                "name": descriptor.name,
                "env_var": descriptor.env_var,
                "default_base_url": descriptor.default_base_url,
                "endpointing": descriptor.endpointing,
                "formats": descriptor.formats,
                "input_formats": descriptor.input_formats,
                "voices": descriptor.voices,
                "models": descriptor.models,
                "model_provenance": descriptor.model_provenance,
                "voice_provenance": descriptor.voice_provenance,
                "configured": configured,
                "readiness": (descriptor.name == "local").then(vak_voice::local_tts_readiness),
            })
        })
        .collect();
    Json(serde_json::json!({
        "providers": providers,
        "discovery": "models and voices are populated only from provider responses",
    }))
}

/// FinOps projection from the append-only cost ledger. Unknown-priced rows
/// are retained as `unknown_rows`; they are never reported as zero spend.
/// Caps are read through the live-effective accessors, not `Core::config()`
/// directly, so a PATCH from `patch_finops` (below) is reflected
/// immediately rather than only after a restart.
async fn finops_status(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut homes = vec![core.shared_data_home()];
    if core.sessions_home() != core.shared_data_home() {
        homes.push(core.sessions_home());
    }
    let mut rows: Vec<vak_core::finops::CostRow> = Vec::new();
    let mut activity: Vec<vak_core::finops::ActivityRow> = Vec::new();
    for home in &homes {
        rows.extend(vak_core::finops::FinOpsLedger::new(home).all_rows());
        activity.extend(vak_core::finops::ActivityLedger::new(home).all_rows());
    }
    let ledger = vak_core::finops::FinOpsLedger::new(&core.shared_data_home());
    let now = chrono::Utc::now();
    let day_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Utc).single());
    let day_rows: Vec<&vak_core::finops::CostRow> = rows
        .iter()
        .filter(|r| day_start.is_some_and(|start| r.ts >= start))
        .collect();
    let day_activity = activity
        .iter()
        .filter(|r| day_start.is_some_and(|start| r.ts >= start));
    let mut activity_by_name =
        std::collections::BTreeMap::<(String, String, Option<String>), (u64, u64, u64)>::new();
    for row in day_activity {
        let entry = activity_by_name
            .entry((row.kind.clone(), row.name.clone(), row.plugin.clone()))
            .or_default();
        entry.0 += 1;
        entry.1 += u64::from(row.success);
        if let Some(duration) = row.duration_ms {
            entry.2 += duration;
        }
    }
    let day_usd: f64 = day_rows.iter().filter_map(|r| r.usd).sum();
    let unknown_rows = day_rows.iter().filter(|r| r.usd.is_none()).count();
    let mut by_provider = std::collections::BTreeMap::<String, (f64, u64, u64, u64, u64)>::new();
    let mut by_model = std::collections::BTreeMap::<String, (f64, u64, u64, u64, u64)>::new();
    for row in &day_rows {
        let usd = row.usd.unwrap_or(0.0);
        let p = by_provider.entry(row.provider.clone()).or_default();
        p.0 += usd;
        p.1 += 1;
        p.2 += row.input_tokens;
        p.3 += row.output_tokens;
        p.4 += row.cache_read_input_tokens.unwrap_or(0);
        let m = by_model.entry(row.model.clone()).or_default();
        m.0 += usd;
        m.1 += 1;
        m.2 += row.input_tokens;
        m.3 += row.output_tokens;
        m.4 += row.cache_read_input_tokens.unwrap_or(0);
    }
    let rollup = |source: std::collections::BTreeMap<String, (f64, u64, u64, u64, u64)>| -> Vec<serde_json::Value> {
            source.into_iter().map(|(name, (usd, calls, input_tokens, output_tokens, cache_read_tokens))| serde_json::json!({ "name": name, "usd": usd, "calls": calls, "input_tokens": input_tokens, "output_tokens": output_tokens, "cache_read_tokens": cache_read_tokens })).collect()
        };
    let daily: Vec<serde_json::Value> = ledger
        .daily_totals(now, FINOPS_TREND_DAYS)
        .into_iter()
        .map(|(date, usd)| serde_json::json!({ "date": date.to_string(), "usd": usd }))
        .collect();
    let mut alerts = recent_budget_alerts(&core.shared_data_home(), 10);
    if alerts.is_empty() && core.sessions_home() != core.shared_data_home() {
        alerts = recent_budget_alerts(&core.sessions_home(), 10);
    }
    Json(serde_json::json!({
        "day_usd": day_usd,
        "run_cap_usd": core.effective_finops_max_run_usd(),
        "day_cap_usd": core.effective_finops_max_day_usd(),
        "unknown_rows": unknown_rows,
        "total_rows": rows.len(),
        "day_input_tokens": day_rows.iter().map(|r| r.input_tokens).sum::<u64>(),
        "day_output_tokens": day_rows.iter().map(|r| r.output_tokens).sum::<u64>(),
        "day_cache_read_tokens": day_rows.iter().map(|r| r.cache_read_input_tokens.unwrap_or(0)).sum::<u64>(),
        "activity": activity_by_name.into_iter().map(|((kind, name, plugin), (calls, successes, duration_ms))| serde_json::json!({"kind": kind, "name": name, "plugin": plugin, "calls": calls, "successes": successes, "duration_ms": duration_ms})).collect::<Vec<_>>(),
        "by_provider": rollup(by_provider),
        "by_model": rollup(by_model),
        "daily": daily,
        "recent_alerts": alerts,
    }))
    .into_response()
}

/// Most recent budget-alert rows, newest first, tolerant of corrupt or
/// foreign lines exactly like [`vak_core::finops::last_alert`] is.
fn recent_budget_alerts(home: &std::path::Path, limit: usize) -> Vec<serde_json::Value> {
    let Ok(body) = std::fs::read_to_string(home.join("budget-alerts.jsonl")) else {
        return Vec::new();
    };
    let mut rows: Vec<serde_json::Value> = body
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|row| row.get("kind").and_then(|k| k.as_str()) == Some("budget_alert"))
        .collect();
    rows.reverse();
    rows.truncate(limit);
    rows
}

#[derive(serde::Deserialize, Default)]
struct FinopsPatch {
    /// Absent = leave alone; explicit `null` = clear the cap; a number =
    /// set it. Same [`gateway::deserialize_present`] shape as
    /// `UpdateBotBody`'s fields, for the same reason: a plain
    /// `Option<Option<f64>>` can't tell "not sent" from "sent as null"
    /// apart otherwise.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    max_run_usd: Option<Option<f64>>,
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    max_day_usd: Option<Option<f64>>,
    #[serde(default)]
    agent: Option<String>,
}

/// `PATCH /finops` — set or clear the run/day budget caps, applied live
/// (no restart) and persisted to `.vak/config.toml`'s `[finops]` table.
async fn patch_finops(
    State(state): State<AppState>,
    Json(body): Json<FinopsPatch>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if body
        .max_run_usd
        .flatten()
        .is_some_and(|v| !v.is_finite() || v < 0.0)
        || body
            .max_day_usd
            .flatten()
            .is_some_and(|v| !v.is_finite() || v < 0.0)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "a budget cap must be a non-negative number" })),
        )
            .into_response();
    }
    if body.max_run_usd.is_none() && body.max_day_usd.is_none() {
        return StatusCode::OK.into_response();
    }
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    if vak_config::persist_project_finops_caps(core.cwd(), body.max_run_usd, body.max_day_usd)
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    core.apply_persisted_finops_caps(body.max_run_usd, body.max_day_usd);
    vak_core::security_events::record(
        &core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "finops_caps_patched",
        &format!(
            "run={:?} day={:?}",
            core.effective_finops_max_run_usd(),
            core.effective_finops_max_day_usd()
        ),
        None,
    );
    state.hub.emit_config_changed("finops_caps_patched", "");
    StatusCode::OK.into_response()
}

#[derive(serde::Deserialize)]
struct OpsActionQuery {
    #[serde(default)]
    port: Option<u16>,
}

/// `POST /ops/services/activate` — reconcile the service manager with
/// configuration.
///
/// Creating a bot, renaming it, or storing its token writes `bots.json`
/// and stops there; this is the deliberate act that turns those records
/// into running bridges. Same contract as `vak self services-sync`, and
/// the same one `vak setup` runs at its activation step: install and
/// configuration place things, activation starts them
/// (`docs/design/46-stabilization-install-and-onboarding.md` D6).
async fn activate_services(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    match service_control::reconcile(&state.core, state.ops_port).await {
        Ok(outcomes) => {
            state.hub.emit_config_changed("services_activated", "");
            let failed: Vec<&service_control::UnitOutcome> =
                outcomes.iter().filter(|o| o.error.is_some()).collect();
            Json(serde_json::json!({
                "ok": failed.is_empty(),
                "units": outcomes,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn ops_action(
    State(state): State<AppState>,
    Path((service, action)): Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<OpsActionQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let svc = match service.as_str() {
        "gateway" => Some(vak_ops::Service::Gateway),
        "bridges" => Some(vak_ops::Service::Bridges),
        _ => None,
    };
    let Some(svc) = svc else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("unknown service '{service}'") })),
        )
            .into_response();
    };
    let mut cfg = vak_ops::OpsConfig::detect();
    cfg.port = state.ops_port;
    if let Some(port) = q.port {
        cfg.port = port;
    }
    // Every service-manager call goes through service_control, which runs
    // it on a blocking worker: launchctl and systemctl are subprocesses,
    // and `launchctl bootstrap` can block indefinitely. Calling them from
    // this async handler stalled a tokio worker for as long as the manager
    // took to answer (AGENTS.md invariant 26).
    let before = service_control::status(svc, cfg.clone())
        .await
        .map(|s| s.to_string())
        .unwrap_or_else(|e| e);
    let requested_at = Utc::now();
    let (result, succeeded) = match action.as_str() {
        "start" => {
            let ok = service_control::act(svc, service_control::Action::Start, cfg.clone())
                .await
                .unwrap_or(false);
            (
                if ok {
                    serde_json::json!({ "ok": true, "action": "start" })
                } else {
                    serde_json::json!({
                        "ok": false,
                        "action": "start",
                        "error": format!("service manager failed to start {service}"),
                    })
                },
                ok,
            )
        }
        "stop" => {
            let ok = service_control::act(svc, service_control::Action::Stop, cfg.clone())
                .await
                .unwrap_or(false);
            (
                if ok {
                    serde_json::json!({ "ok": true, "action": "stop" })
                } else {
                    serde_json::json!({
                        "ok": false,
                        "action": "stop",
                        "error": format!("service manager failed to stop {service}"),
                    })
                },
                ok,
            )
        }
        "restart" => {
            let ok = service_control::act(svc, service_control::Action::Restart, cfg.clone())
                .await
                .unwrap_or(false);
            (
                if ok {
                    serde_json::json!({ "ok": true, "action": "restart" })
                } else {
                    serde_json::json!({
                        "ok": false,
                        "action": "restart",
                        "error": format!("service manager failed to restart {service}"),
                    })
                },
                ok,
            )
        }
        "install" => match vak_ops::install(svc, &cfg) {
            Ok(()) => (serde_json::json!({ "ok": true, "action": "install" }), true),
            Err(e) => (serde_json::json!({ "ok": false, "error": e }), false),
        },
        "uninstall" => match vak_ops::uninstall(svc, &cfg) {
            Ok(()) => (
                serde_json::json!({ "ok": true, "action": "uninstall" }),
                true,
            ),
            Err(e) => (serde_json::json!({ "ok": false, "error": e }), false),
        },
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown action '{other}'") })),
            )
                .into_response();
        }
    };
    let after = service_control::status(svc, cfg.clone())
        .await
        .map(|s| s.to_string())
        .unwrap_or_else(|e| e);
    let desired_reached = match action.as_str() {
        "start" | "restart" | "install" => after == "running",
        "stop" => matches!(after.as_str(), "stopped" | "not installed"),
        "uninstall" => after == "not installed",
        _ => false,
    };
    let verification_status = if !succeeded {
        "failed"
    } else if desired_reached {
        "verified"
    } else {
        "pending"
    };
    let verification_detail = if !succeeded {
        "The service manager rejected the requested operation; the post-action probe is authoritative."
    } else if desired_reached {
        "The post-action service-manager probe reached the requested state."
    } else {
        "The manager accepted the request but the desired state is not visible yet; keep the receipt and re-probe."
    };
    let mut receipt = operations::ActionReceipt {
        receipt_id: format!("OP-{}", uuid::Uuid::now_v7().simple()),
        service: service.clone(),
        action: action.clone(),
        requested_at,
        completed_at: Utc::now(),
        succeeded,
        verification: operations::ActionVerification {
            status: verification_status.to_string(),
            before,
            after,
            detail: verification_detail.to_string(),
        },
        persisted: false,
    };
    receipt.persisted = operations::record_action(&state.core.sessions_home(), &receipt).is_ok();
    let receipt_json = serde_json::to_value(&receipt).unwrap_or_else(|_| serde_json::json!({}));
    let mut result = result;
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "receipt_id".to_string(),
            serde_json::json!(receipt.receipt_id),
        );
        object.insert(
            "verification".to_string(),
            receipt_json["verification"].clone(),
        );
        object.insert(
            "receipt_persisted".to_string(),
            serde_json::json!(receipt.persisted),
        );
    }
    if succeeded {
        vak_core::security_events::record(
            &state.core.sessions_home(),
            vak_core::security_events::EventKind::ConfigChange,
            "service_action",
            &format!("service={service} action={action}"),
            None,
        );
        state
            .hub
            .emit_config_changed("service_action", &format!("{service}:{action}"));
        (StatusCode::OK, Json(result)).into_response()
    } else {
        (StatusCode::CONFLICT, Json(result)).into_response()
    }
}

fn note_payload(n: &vak_core::memory::NoteBlock, scope: &str) -> serde_json::Value {
    serde_json::json!({
        "id": n.id,
        "ts": n.ts.to_rfc3339(),
        "kind": n.kind,
        "tag": n.tag,
        "session_id": n.session_id,
        "text": n.text,
        "scope": scope,
    })
}

/// Which Agent an endpoint scoped to "the currently open Agent's own data"
/// (memory, learning proposals) should resolve against. Absent means the
/// built-in "vak" Agent — the same default `agent_chats::open` uses.
#[derive(serde::Deserialize, Default)]
struct AgentScopeQuery {
    #[serde(default)]
    agent: Option<String>,
}

/// Resolve the `Core` an Agent-scoped endpoint should read/write through.
///
/// A registered session already carries the exact `Core` it was opened
/// under (agent identity, isolated workspace, and now-agent-scoped
/// `sessions_home` all resolved once at `agent_chats::open` time) — reusing
/// it is cheaper and more precise than re-deriving identity from an id, so
/// `session_id` (when the caller already has one, e.g. `AppendMemoryBody`)
/// takes precedence over an explicit `agent` id.
///
/// This is the single place "which Agent's data does this endpoint mean"
/// gets decided, so a future endpoint scoped the same way calls this
/// instead of reading `state.core` directly and drifting out of sync with
/// `agent_chats::open` the way `list_sessions` once did (see commit
/// 7e6713c0 and its follow-up).
#[allow(clippy::result_large_err)]
fn resolve_scoped_core(
    state: &AppState,
    session_id: Option<&str>,
    agent: Option<&str>,
) -> Result<vak_core::Core, axum::response::Response> {
    if let Some(sid) = session_id
        && let Some(handle) = state.get(sid)
    {
        return Ok(handle.core.clone());
    }
    let id = agent.unwrap_or("vak");
    agent_chats::resolve_agent_core(state, id).map(|(_, core)| core)
}

async fn list_memory(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    // The resolved Agent's own (now agent-scoped) sessions_home is primary;
    // `state.core`'s plain, un-scoped home is kept as a fallback merge so
    // notes written before Agents carried their own sessions_home (or by an
    // older build) are not silently hidden.
    let mut homes = vec![core.sessions_home(), state.core.sessions_home()];
    let shared = state.core.shared_data_home();
    if !homes.contains(&shared) {
        homes.push(shared);
    }
    let mut blocks: Vec<serde_json::Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut seen_home = std::collections::HashSet::new();
    for home in homes {
        if !seen_home.insert(home.clone()) {
            continue;
        }
        for n in vak_core::memory::list_notes(&home, core.cwd()) {
            if seen.insert((n.kind.clone(), n.tag.clone(), n.text.clone())) {
                blocks.push(note_payload(&n, "workspace"));
            }
        }
        for n in vak_core::memory::list_profile_notes(&home) {
            if seen.insert((n.kind.clone(), n.tag.clone(), n.text.clone())) {
                blocks.push(note_payload(&n, "profile"));
            }
        }
    }
    Json(serde_json::json!({ "notes": blocks })).into_response()
}

async fn cleanup_memory(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut homes = vec![core.sessions_home(), state.core.sessions_home()];
    let shared = state.core.shared_data_home();
    if !homes.contains(&shared) {
        homes.push(shared);
    }
    let mut report = vak_core::memory::CleanupReport::default();
    let mut seen_home = std::collections::HashSet::new();
    for home in homes {
        if !seen_home.insert(home.clone()) {
            continue;
        }
        let home_report =
            vak_core::memory::cleanup_artifacts(&home, std::time::Duration::from_secs(86_400));
        report.removed_locks += home_report.removed_locks;
        report.removed_temps += home_report.removed_temps;
        report.removed_empty_dirs += home_report.removed_empty_dirs;
    }
    vak_core::security_events::record(
        &core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "memory_cleanup",
        &format!(
            "locks={} temps={} empty_dirs={}",
            report.removed_locks, report.removed_temps, report.removed_empty_dirs
        ),
        None,
    );
    Json(serde_json::json!({
        "removed_locks": report.removed_locks,
        "removed_temps": report.removed_temps,
        "removed_empty_dirs": report.removed_empty_dirs,
    }))
    .into_response()
}

async fn consolidate_memory_route(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    match core.consolidate_memory() {
        Ok(report) => (
            StatusCode::OK,
            Json(serde_json::to_value(&report).unwrap_or_default()),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ListEntitiesQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

async fn list_entities_route(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListEntitiesQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let is_global = query.scope.as_deref() == Some("global");
    let cwd_buf = state.core.cwd();
    let cwd = if is_global {
        None
    } else {
        Some(cwd_buf.as_path())
    };
    let entities = if let Some(ref q) = query.q {
        vak_core::entities::search_entities(&home, cwd, q)
    } else {
        vak_core::entities::list_entities(&home, cwd)
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({ "entities": entities })),
    )
        .into_response()
}

async fn get_entity_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<ListEntitiesQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let is_global = query.scope.as_deref() == Some("global");
    let cwd_buf = state.core.cwd();
    let cwd = if is_global {
        None
    } else {
        Some(cwd_buf.as_path())
    };
    if let Some(entity) = vak_core::entities::get_entity(&home, cwd, &id) {
        (
            StatusCode::OK,
            Json(serde_json::to_value(&entity).unwrap_or_default()),
        )
            .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "entity not found" })),
        )
            .into_response()
    }
}

#[derive(serde::Deserialize)]
struct UpsertEntityBody {
    #[serde(default)]
    id: Option<String>,
    name: String,
    entity_type: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    attributes: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    relations: Vec<vak_core::entities::EntityRelation>,
    #[serde(default)]
    scope: Option<String>,
}

async fn upsert_entity_route(
    State(state): State<AppState>,
    Json(body): Json<UpsertEntityBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let is_global = body.scope.as_deref() == Some("global");
    let cwd_buf = state.core.cwd();
    let cwd = if is_global {
        None
    } else {
        Some(cwd_buf.as_path())
    };
    let id = body.id.unwrap_or_else(|| {
        let slug = body
            .name
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        if slug.is_empty() {
            uuid::Uuid::now_v7().to_string()
        } else {
            slug
        }
    });

    let record = vak_core::entities::EntityRecord {
        id,
        name: body.name,
        entity_type: body.entity_type,
        summary: body.summary,
        attributes: body.attributes,
        relations: body.relations,
        updated_at: chrono::Utc::now(),
    };

    match vak_core::entities::upsert_entity(&home, cwd, record) {
        Ok(saved) => (
            StatusCode::OK,
            Json(serde_json::to_value(&saved).unwrap_or_default()),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn delete_entity_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<ListEntitiesQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let is_global = query.scope.as_deref() == Some("global");
    let cwd_buf = state.core.cwd();
    let cwd = if is_global {
        None
    } else {
        Some(cwd_buf.as_path())
    };
    match vak_core::entities::delete_entity(&home, cwd, &id) {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({ "deleted": true }))).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "deleted": false, "error": "entity not found" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// Resolve a note id to the markdown store it lives in. The workspace tier/// is per-cwd; the profile tier is global (`<home>/memory/user/USER.md`).
#[derive(serde::Deserialize)]
struct AppendMemoryBody {
    text: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    scope: Option<MemoryScope>,
    #[serde(default)]
    session_id: Option<String>,
    /// Which Agent this note belongs to; see `AgentScopeQuery`. Absent
    /// means "vak", unless `session_id` names a currently-registered
    /// session, whose own Agent takes precedence (see `resolve_scoped_core`).
    #[serde(default)]
    agent: Option<String>,
}

/// Append a note to either tier. Keeps gateway/desktop/CLI symmetric —
/// every surface writes through the same validated core API.
async fn append_memory(
    State(state): State<AppState>,
    Json(body): Json<AppendMemoryBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, body.session_id.as_deref(), body.agent.as_deref())
    {
        Ok(core) => core,
        Err(response) => return response,
    };
    let home = core.sessions_home();
    let scope = body.scope.unwrap_or(MemoryScope::Workspace);
    let kind = body.kind.unwrap_or_else(|| "fact".to_string());
    let tag = body.tag.unwrap_or_default();
    let session = body.session_id.unwrap_or_else(|| "http".to_string());
    let result = match scope {
        MemoryScope::Workspace => {
            vak_core::memory::append_note(&home, core.cwd(), &kind, &tag, &session, &body.text)
        }
        MemoryScope::Profile => {
            vak_core::memory::append_profile_note(&home, &kind, &tag, &body.text, &session)
        }
    };
    match result {
        Ok(note) => {
            let scope_str = match scope {
                MemoryScope::Workspace => "workspace",
                MemoryScope::Profile => "profile",
            };
            vak_core::security_events::record(
                &home,
                vak_core::security_events::EventKind::ConfigChange,
                "memory_append",
                &format!("scope={scope_str} note_id={}", note.id),
                None,
            );
            (StatusCode::CREATED, Json(note_payload(&note, scope_str))).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

fn memory_store_path(core: &vak_core::Core, scope: MemoryScope) -> PathBuf {
    let home = core.sessions_home();
    match scope {
        MemoryScope::Workspace => home
            .join("memory")
            .join(vak_core::memory::hash_cwd(core.cwd()))
            .join("MEMORY.md"),
        MemoryScope::Profile => vak_core::memory::profile_path(&home),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum MemoryScope {
    #[default]
    Workspace,
    Profile,
}

async fn forget_memory_note(
    State(state): State<AppState>,
    Path(note_id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<MemoryScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let path = memory_store_path(&core, q.scope.unwrap_or_default());
    let result = vak_core::memory::forget_note(&path, &note_id);
    // A note written before this Agent's data moved to its own sessions_home
    // subfolder still lives at the legacy, un-scoped path — fall back to it
    // the same way `promote_proposal`/`reject_proposal` already do, so an
    // old note surfaced by `list_memory`'s merged view can still be forgotten.
    let legacy_path = memory_store_path(&state.core, q.scope.unwrap_or_default());
    let result = match result {
        Err(_) if legacy_path != path => vak_core::memory::forget_note(&legacy_path, &note_id),
        other => other,
    };
    match result {
        Ok(bytes) => {
            vak_core::security_events::record(
                &core.sessions_home(),
                vak_core::security_events::EventKind::ConfigChange,
                "memory_forget",
                &format!("scope={:?} note_id={note_id}", q.scope.unwrap_or_default()),
                None,
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({ "forgotten": note_id, "bytes": bytes })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct MemoryAmendBody {
    text: String,
    #[serde(default)]
    scope: Option<MemoryScope>,
    /// See `AgentScopeQuery`.
    #[serde(default)]
    agent: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct MemoryScopeQuery {
    #[serde(default)]
    scope: Option<MemoryScope>,
    /// See `AgentScopeQuery`.
    #[serde(default)]
    agent: Option<String>,
}

async fn amend_memory_note(
    State(state): State<AppState>,
    Path(note_id): Path<String>,
    Json(body): Json<MemoryAmendBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if body.text.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "note must not be empty" })),
        )
            .into_response();
    }
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let path = memory_store_path(&core, body.scope.unwrap_or_default());
    let result = vak_core::memory::amend_note(&path, &note_id, &body.text);
    // See the identical fallback in `forget_memory_note`.
    let legacy_path = memory_store_path(&state.core, body.scope.unwrap_or_default());
    let result = match result {
        Err(_) if legacy_path != path => {
            vak_core::memory::amend_note(&legacy_path, &note_id, &body.text)
        }
        other => other,
    };
    match result {
        Ok(()) => {
            vak_core::security_events::record(
                &core.sessions_home(),
                vak_core::security_events::EventKind::ConfigChange,
                "memory_amend",
                &format!(
                    "scope={:?} note_id={note_id}",
                    body.scope.unwrap_or_default()
                ),
                None,
            );
            (
                StatusCode::OK,
                Json(serde_json::json!({ "amended": note_id })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

fn proposals_payload(core: &Core) -> Vec<serde_json::Value> {
    let mut proposals = vak_core::learning::list_proposals(&core.sessions_home(), core.cwd());
    if proposals.is_empty() && core.shared_data_home() != core.sessions_home() {
        proposals = vak_core::learning::list_proposals(&core.shared_data_home(), core.cwd());
    }
    proposals
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "description": p.description,
            })
        })
        .collect()
}

async fn list_proposals_route(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    Json(serde_json::json!({ "proposals": proposals_payload(&core) })).into_response()
}

async fn promote_proposal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let res = vak_core::learning::promote(&core.sessions_home(), core.cwd(), &id);
    let res = match res {
        Err(_) if state.core.shared_data_home() != core.sessions_home() => {
            vak_core::learning::promote(&state.core.shared_data_home(), core.cwd(), &id)
        }
        other => other,
    };
    match res {
        Ok(name) => (
            StatusCode::OK,
            Json(serde_json::json!({ "promoted": name })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn reject_proposal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let res = vak_core::learning::reject(&core.sessions_home(), core.cwd(), &id);
    let res = match res {
        Err(_) if state.core.shared_data_home() != core.sessions_home() => {
            vak_core::learning::reject(&state.core.shared_data_home(), core.cwd(), &id)
        }
        other => other,
    };
    match res {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "rejected": id }))).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    limit: Option<usize>,
    /// Session id whose (already-in-context) content should be skipped.
    #[serde(default)]
    exclude: Option<String>,
    /// Cross-project recall: search every project's ledgers under the
    /// sessions home (docs/design/29-personal-os.md P1), not just this cwd.
    #[serde(default)]
    all: bool,
}

async fn search_sessions(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<SearchQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let cwd = state.core.cwd().clone();
    let query = q.q.clone();
    let limit = q.limit.unwrap_or(vak_session::DEFAULT_LIMIT);
    let exclude = q.exclude.clone();
    let all = q.all;
    let mut extras = Vec::new();
    let mut workspace_notes = vak_core::memory::list_notes(&home, &cwd);
    if all {
        let root = home.join("memory");
        if let Ok(entries) = std::fs::read_dir(&root) {
            workspace_notes.clear();
            for entry in entries.flatten() {
                let path = entry.path().join("MEMORY.md");
                if let Ok(raw) = std::fs::read_to_string(path) {
                    workspace_notes.extend(vak_core::memory::parse_blocks(&raw));
                }
            }
        }
    }
    for note in workspace_notes {
        let id = if note.tag.is_empty() {
            note.id.clone()
        } else {
            note.tag.clone()
        };
        extras.push(vak_session::ExternalDoc {
            id,
            text: format!("[{}] {}", note.kind, note.text),
            ts: Some(note.ts),
            role: Some("memory".into()),
        });
    }
    for note in vak_core::memory::list_profile_notes(&home) {
        let id = format!(
            "profile/{}",
            if note.tag.is_empty() {
                note.id.clone()
            } else {
                note.tag.clone()
            }
        );
        extras.push(vak_session::ExternalDoc {
            id,
            text: format!("[{}] {}", note.kind, note.text),
            ts: Some(note.ts),
            role: Some("profile".into()),
        });
    }
    match tokio::task::spawn_blocking(move || {
        // Both hit shapes are Serialize; the workspace path keeps its flat
        // SessionHit wire shape, cross-project adds the project_hash wrapper.
        let searched = if all {
            vak_session::search_all_extended(&home, &query, limit, exclude.as_deref(), &extras)
                .map(|hits| serde_json::to_value(&hits).map_err(|e| e.to_string()))
        } else {
            vak_session::search_extended(&home, &cwd, &query, limit, exclude.as_deref(), &extras)
                .map(|hits| serde_json::to_value(&hits).map_err(|e| e.to_string()))
        };
        match searched {
            Ok(inner) => inner,
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    {
        Ok(Ok(hits)) => Json(serde_json::json!({ "all": all, "hits": hits })).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// The bearer token lives for the life of the process; embedders (desktop
/// shell, tests) need it to hand to their webview, so build the secured
/// stack here instead of inside `serve()`.
pub fn secured_router(core: Core) -> (Router, String) {
    secured_router_with(core, false)
}

/// Same stack with a CLI-level gateway override (`serve --gateway`).
///
/// Token selection: when `VAK_GATEWAY_TOKEN` is set in the
/// environment, it is used verbatim so service-managed bridges and other
/// long-lived clients can survive process restarts. Otherwise a fresh
/// per-process token is minted as before. The variable is never logged.
pub fn secured_router_with(core: Core, force_gateway: bool) -> (Router, String) {
    secured_router_with_port(core, force_gateway, vak_ops::OpsConfig::detect().port)
}

/// Same secured stack with the actual listener port carried into operational
/// probes. `serve_with` uses this so a non-default `--port` cannot make the
/// console probe a different process.
pub fn secured_router_with_port(core: Core, force_gateway: bool, port: u16) -> (Router, String) {
    // Tauri can use either its custom scheme or the loopback-style origin,
    // depending on the platform and WebView runtime, plus vite dev servers.
    let origins = [
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
        "http://localhost:1420",
        "http://127.0.0.1:1420",
        "http://localhost:5173",
        "http://127.0.0.1:5173",
    ]
    .into_iter()
    .filter_map(|o| o.parse::<axum::http::HeaderValue>().ok())
    .collect::<Vec<_>>();
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins)
        // Must cover every method the router exposes: PATCH (/config,
        // /sessions/:id/config) and DELETE are preflighted, so omitting them
        // makes the browser reject the request before it is ever sent.
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::PATCH,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);
    let mut state = AppState::new(core);
    state.ops_port = port;
    if force_gateway {
        state.enable_gateway();
    }
    let token = (*state.auth_token).clone();
    let rl_settings = state.core.config().gateway.rate_limit.clone();
    let rl_config = rate_limit::RateLimitConfig::from_settings(rl_settings);
    let limiter = rate_limit::RateLimiter::new(rl_config, state.core.sessions_home());
    let app = router_with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            limiter,
            rate_limit::rate_limit_layer,
        ))
        .layer(axum::middleware::from_fn_with_state(
            AuthPolicy {
                token: token.clone(),
                home: state.core.sessions_home(),
                trusted_hosts: state.core.config().server.trusted_hosts.clone(),
            },
            require_bearer,
        ))
        .layer(cors);
    #[cfg(unix)]
    {
        let broker = state.core.agent_network_broker();
        let policy_file = state
            .core
            .sessions_home()
            .join("agent-network/policies.json");
        if let Err(error) = broker.load_policies(&policy_file) {
            eprintln!("[agent-network] policy load failed: {error}");
        }
        let socket =
            vak_core::agent_network::AgentNetworkBroker::socket_path(&state.core.sessions_home());
        tokio::spawn(async move {
            if let Err(error) = broker.serve_unix(&socket).await {
                eprintln!("[agent-network] broker stopped: {error}");
            }
        });
    }
    // Local routines: fires due scheduled tasks while this server lives.
    start_scheduler(&state);
    delivery::start_replay(&state.core);
    // Capability discovery is NOT started here.
    //
    // It used to be, and the CLI did its own bounded wait, and the desktop
    // did neither — three surfaces answering "when may a prompt be frozen?"
    // three different ways, which is how an admitted, working MCP server
    // still produced a session that had never seen its catalog.
    // `Core::admitted_capabilities` owns that decision now, so every surface
    // gets the same packet whether it was reached from a terminal, this
    // server, or the desktop app.
    // Background index sync: keeps the admin console populated from the
    // very first boot. Idempotent; never blocks request handling.
    if let Some(store) = state.store.clone() {
        let home = state.core.sessions_home();
        tokio::spawn(async move {
            match store.rebuild(&home) {
                Ok(s) if s.files_scanned > 0 => eprintln!(
                    "[store] indexed {} files / {} entries",
                    s.files_scanned, s.entries_indexed
                ),
                Ok(_) => {}
                Err(e) => eprintln!("[store] startup rebuild failed: {e}"),
            }
        });
    }
    (app, token)
}

/// Initialize the distributed event bus from config and attach it to the
/// global `EventHub`. Called from `serve_with` (async) so NATS connection
/// retries don't block request handling.
pub async fn init_server_bus(core: &Core) {
    let config = core.config().server.bus.clone();
    if config.nats_url.is_none() {
        return;
    }
    let sessions_home = core.sessions_home();
    let workspace_id = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(sessions_home.to_string_lossy().as_bytes());
        let result = hasher.finalize();
        // First 6 bytes → 12 hex chars, a compact workspace-scoped subject prefix.
        let prefix = &result[..6];
        let hex: String = prefix.iter().map(|b| format!("{b:02x}")).collect();
        format!("ws_{}", hex)
    };
    let bus = crate::bus::ServerBus::from_resolved(&workspace_id, &config).await;
    if let Some(mut hub) = crate::events::global() {
        hub.set_server_bus(std::sync::Arc::new(bus));
    }
}

pub async fn serve(core: Core, addr: std::net::SocketAddr) -> std::io::Result<()> {
    serve_with(core, addr, false).await
}

/// `force_gateway` mirrors `serve --gateway`: enable routing regardless of
/// the (untrusted-stripped) project config.
/// Serve an already-bound listener with an already-built router.
///
/// The pieces `secured_router` returns, joined. `vak setup` binds its own
/// ephemeral loopback port so it can print the URL *before* serving, and
/// needs the token from the same call — which `serve_with` cannot give it,
/// because that mints and consumes the token internally. Exposed here so
/// axum stays a dependency of this crate rather than leaking into the CLI.
pub async fn serve_router(listener: tokio::net::TcpListener, app: Router) -> std::io::Result<()> {
    axum::serve(listener, app).await
}

pub async fn serve_with(
    core: Core,
    addr: std::net::SocketAddr,
    force_gateway: bool,
) -> std::io::Result<()> {
    // Presentation seeds are versioned, additive previews. Reconcile them at
    // process startup so a newly installed binary reaches existing workspaces
    // even when the admin presentation list is never opened. User revisions
    // and activations remain untouched by register().
    reconcile_builtin_presentations(&core)
        .map_err(|error| std::io::Error::other(format!("presentation seed failed: {error}")))?;
    // Local-only does not mean safe-by-default: any local process could
    // reach an unauthenticated agent and drive arbitrary tool execution
    // plus self-approval. Every serve() instance gets a per-process
    // bearer token; /health stays open.
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual_addr = listener.local_addr()?;
    let (app, token) = secured_router_with_port(core.clone(), force_gateway, actual_addr.port());
    // Initialize the distributed event bus (vak-bus, docs/design/53).
    // Falls back to InMemoryBus when NATS is absent or unreachable.
    init_server_bus(&core).await;
    eprintln!("Vak server listening on http://{actual_addr}");
    // Same source the real token-selection logic above (auth_token, in
    // AppState::new) already checks: `vak_config::get_var` also sees a
    // value that only reached the process through a loaded `.env` file
    // (never a real `std::env` var), so a plain `std::env::var` check
    // here — which is all this ever did — reported "generated" for
    // every service-managed deployment, since none of them export
    // VAK_GATEWAY_TOKEN into the actual process environment; they rely
    // on the user `.env` main() already loads unconditionally at
    // startup. The token itself was always correctly pinned; only this
    // log line was wrong, in exactly the deployment shape (a durable
    // service reading `.env`) where getting it right matters most for
    // debugging a stale-cookie/token mismatch after a restart.
    if vak_config::get_var("VAK_GATEWAY_TOKEN").is_some_and(|t| !t.trim().is_empty()) {
        eprintln!("auth token: (pinned via VAK_GATEWAY_TOKEN)");
    } else if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        eprintln!("auth token: {token}");
        eprintln!("clients must send 'Authorization: Bearer {token}' (or ?token=)");
    } else {
        eprintln!("auth token: generated for this process (suppressed in non-interactive output)");
    }
    if force_gateway {
        eprintln!("gateway: ENABLED (--gateway overrides config)");
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\n[shutting down: draining connections]");
        })
        .await
}

fn reconcile_builtin_presentations(core: &Core) -> Result<(), String> {
    let store = vak_store::presentation::PresentationStore::new(
        core.sessions_home().join("presentations.json"),
    );
    let mut library = store.load().map_err(|error| error.to_string())?;
    let before = library.definitions().count();
    let mut changed = false;
    for seed in vak_presentation::seeds::built_in_seed_pack() {
        if let Some(existing) = library.get(&seed.spec.id, seed.spec.revision)
            && existing.digest != seed.digest
        {
            changed = true;
        }
        library.register(seed).map_err(|error| error.to_string())?;
    }
    if changed || library.definitions().count() != before {
        store.save(&library).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Paths that must be reachable without a token: health probe, the SPA
/// shell (static assets carry no data), and the login endpoint itself.
fn auth_exempt_path(path: &str) -> bool {
    path == "/health"
        || path == "/admin"
        || path == "/admin/"
        || path == "/admin/favicon.svg"
        || path == "/admin/vak-icon.png"
        || path.starts_with("/admin/assets/")
        // The workspace client's shell and its hashed assets carry no data
        // and must load before a session exists — the login form is part of
        // the bundle. Every route it then calls is authenticated.
        || path == "/app"
        || path == "/app/"
        || path == "/app/vak-icon.png"
        || path == "/app/manifest.webmanifest"
        || path == "/app/sw.js"
        || path.starts_with("/app/assets/")
        // The login exchange itself, and the probe that decides whether to
        // show it. `/auth/session` answers `{authenticated:false}` rather
        // than 401 so an unauthenticated client can tell "no session" from
        // "server unreachable".
        || path == "/auth/login"
        || path == "/auth/session"
        // The public site and its build stamp. Every page answers an
        // unauthenticated stranger by design — a blank 401 at `/` told a
        // visitor nothing at all, not even that anything was listening —
        // and `site.rs` owns what those pages may say.
        || site::ROUTES.iter().any(|(uri, _)| {
            *uri == path || (*uri != "/" && path.len() == uri.len() + 1 && path.starts_with(uri) && path.ends_with('/'))
        })
        || path.starts_with("/site/")
        || path == "/version"
        || path == "/favicon.ico"
        || path == "/favicon.svg"
}

#[cfg(test)]
mod auth_exempt_path_tests {
    use super::{auth_exempt_path, host_is_loopback};

    /// A page that resolves its own domain to 127.0.0.1 becomes same-origin
    /// with this server, and `SameSite=Strict` does not help — after
    /// rebinding the request is not cross-site. The Host name it carries is
    /// still the attacker's, which is what this rejects.
    #[test]
    fn only_loopback_hostnames_are_answered() {
        for host in [
            "localhost",
            "localhost:8901",
            "127.0.0.1",
            "127.0.0.1:8901",
            "[::1]:8901",
            "::1",
        ] {
            assert!(host_is_loopback(Some(host)), "{host} is loopback");
        }
        for host in [
            "rebind.attacker.example",
            "rebind.attacker.example:8901",
            "vak.internal",
            "127.0.0.1.attacker.example",
            "evil.com:80",
        ] {
            assert!(!host_is_loopback(Some(host)), "{host} must be rejected");
        }
    }

    /// Rebinding is a browser attack and browsers always send Host, so an
    /// absent value is a non-browser client rather than something to defend
    /// against.
    #[test]
    fn an_absent_host_is_not_treated_as_an_attack() {
        assert!(host_is_loopback(None));
    }

    /// The login screen's own logo must be reachable before a cookie
    /// exists to authenticate the request that would fetch it — the same
    /// reasoning that already exempts favicon.svg. This regressed once
    /// already: the admin console shipped a login-screen `<img>` pointing
    /// at a root-level dist file, and only /admin/assets/* (the hashed
    /// JS/CSS bundle) was exempt, so the logo 401'd on every fresh login.
    #[test]
    fn login_screen_logo_is_exempt() {
        assert!(auth_exempt_path("/admin/vak-icon.png"));
    }

    #[test]
    fn admin_api_routes_still_require_auth() {
        assert!(!auth_exempt_path("/admin/api/config"));
        assert!(!auth_exempt_path("/admin/api/gateway/status"));
    }
}

/// Whether a `Host` header names this server legitimately.
///
/// Loopback names always pass. Anything else must be listed verbatim in
/// `[server] trusted_hosts`, which is a privileged setting an untrusted
/// project cannot write (docs/design/48-web-client.md §4.2).
///
/// This is the DNS-rebinding defence: binding 127.0.0.1 does not stop a
/// page the user visits from resolving its *own* domain to 127.0.0.1 and
/// becoming same-origin with this server, and `SameSite=Strict` does not
/// help once that has happened, because the request is then not cross-site.
/// Pinning the `Host` name is the check that does — and the reason
/// `trusted_hosts` takes exact names rather than patterns: a wildcard here
/// re-opens exactly the hole the list closes.
fn host_is_trusted(host: Option<&str>, trusted: &[String]) -> bool {
    if host_is_loopback(host) {
        return true;
    }
    let Some(host) = host else { return true };
    let name = strip_port(host).to_ascii_lowercase();
    trusted.iter().any(|allowed| allowed == &name)
}

/// Host name without its port, leaving a bare IPv6 literal intact.
fn strip_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map_or(rest, |(name, _)| name)
    } else if host.matches(':').count() > 1 {
        host
    } else {
        host.split_once(':').map_or(host, |(name, _)| name)
    }
}

/// Whether `Host` names this machine's own loopback interface.
fn host_is_loopback(host: Option<&str>) -> bool {
    let Some(host) = host else {
        // Absent entirely: not a browser. Rebinding is a browser attack and
        // every browser sends Host (or HTTP/2 `:authority`, which axum
        // surfaces as the URI authority), so an absent value carries no
        // attacker-chosen name to defend against. Rejecting here would only
        // break well-behaved non-browser clients that omit it.
        return true;
    };
    // Strip the port without mangling a bare IPv6 literal, which is all
    // colons: `"::1".rsplit_once(':')` yields `"::"`.
    matches!(
        strip_port(host),
        "localhost" | "127.0.0.1" | "::1" | "0:0:0:0:0:0:0:1"
    )
}

/// Whether a state-changing request's `Origin` is one of ours.
///
/// Cookies alone are not enough to authorize a mutation: a cookie is
/// attached by the browser to whoever asks, and `SameSite=Strict` covers
/// the common cases but not a same-site subdomain or a rebound name. So a
/// mutation carrying a cookie must ALSO carry an `Origin` we recognise.
///
/// A request with no `Origin` at all is not a browser form post — browsers
/// always send one on cross-origin mutations — so it is allowed through
/// here and still has to satisfy `require_bearer` with a real header
/// token. That is what keeps curl, the CLI, and the bridges working
/// without giving a page any new power.
fn origin_is_trusted(origin: Option<&str>, trusted: &[String]) -> bool {
    let Some(origin) = origin else { return true };
    // The Tauri webview's own origins: the desktop is a first-party client
    // and its scheme is not something an attacker can mint.
    if matches!(
        origin,
        "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
    ) {
        return true;
    }
    let authority = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    host_is_trusted(Some(authority), trusted)
}

/// What the auth layer needs to know about this deployment's exposure.
#[derive(Clone)]
pub(crate) struct AuthPolicy {
    pub(crate) token: String,
    pub(crate) home: std::path::PathBuf,
    /// Non-loopback `Host` names this server answers to (`[server]
    /// trusted_hosts`). Empty on a default install.
    pub(crate) trusted_hosts: Vec<String>,
}

pub(crate) async fn require_bearer(
    State(policy): State<AuthPolicy>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let AuthPolicy {
        token,
        home,
        trusted_hosts,
    } = policy;
    let host = req
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| req.uri().host());
    let loopback = host_is_loopback(host);
    if !host_is_trusted(host, &trusted_hosts) {
        return (
            StatusCode::MISDIRECTED_REQUEST,
            Json(serde_json::json!({
                "error": "this server does not answer to that hostname; \
                          add it to [server] trusted_hosts to allow it",
            })),
        )
            .into_response();
    }
    // Cross-origin mutations are refused before routing, whatever
    // credential they carry. See `origin_is_trusted` for why a *missing*
    // Origin is not treated as a failure.
    let mutating = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let origin = req
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if mutating && !origin_is_trusted(origin, &trusted_hosts) {
        vak_core::security_events::record(
            &home,
            vak_core::security_events::EventKind::AuthFailure,
            "cross_origin_rejected",
            &format!(
                "origin={} path={}",
                origin.unwrap_or("<none>"),
                req.uri().path()
            ),
            None,
        );
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "cross-origin request refused" })),
        )
            .into_response();
    }
    if auth_exempt_path(req.uri().path()) {
        return next.run(req).await;
    }
    use subtle::ConstantTimeEq;
    let header_token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(String::from);
    // Browser surfaces authenticate once via /auth/login which sets an
    // HttpOnly cookie; EventSource cannot send Authorization headers, so
    // the cookie is the only workable channel for SSE.
    let cookie_token = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|pair| {
                let pair = pair.trim();
                pair.strip_prefix("vak_session=")
                    .map(|v| v.trim().to_string())
            })
        });
    // `EventSource` cannot set request headers, and the desktop app never
    // performs the `/auth/login` cookie exchange -- that is the browser
    // surfaces' flow, not the desktop's. The query parameter is
    // therefore the ONLY channel the desktop's SSE streams can
    // authenticate on, and `openEventStream`/`openSideStream` have always
    // used it. It was never accepted here, so every desktop event stream
    // was rejected 401: the agent completed turns and durably logged them
    // while the UI received not one event -- no reply, "Working" forever,
    // usage stuck at 0 in / 0 out, and nothing in the console, because a
    // 401 on an EventSource surfaces only as a bare `onerror`.
    //
    // The startup banner has advertised `?token=` since before this
    // middleware existed; this makes the implementation match the
    // contract rather than narrowing the contract to the implementation.
    // A token in a query string is a real (if bounded) exposure -- it can
    // reach access logs and `Referer` headers -- but this server is
    // loopback-only with a token that is either ephemeral per boot or
    // pinned into a 0600 `.env`, and no other channel exists for the one
    // client that needs it.
    //
    // Loopback ONLY. A token in a query string can reach access logs,
    // `Referer` headers, and browser history; on a loopback server with an
    // in-process client and no proxy between them, none of those exist.
    // On any deployment reachable by a real hostname they all do, and the
    // web client does not need this channel anyway — it is same-origin, so
    // its cookie covers `EventSource` (docs/design/48-web-client.md §4.3).
    let query_token = loopback
        .then(|| {
            req.uri().query().and_then(|q| {
                q.split('&').find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    if key != "token" {
                        return None;
                    }
                    Some(
                        percent_encoding::percent_decode_str(value)
                            .decode_utf8_lossy()
                            .into_owned(),
                    )
                })
            })
        })
        .flatten();
    let provided = header_token.or(cookie_token).or(query_token);
    let ok = provided
        .as_deref()
        .map(|p| p.as_bytes().ct_eq(token.as_bytes()).into())
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        let ip = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let detail = format!(
            "path={} provided={}",
            req.uri().path(),
            provided
                .as_deref()
                .map(|p| format!(
                    "{}...{}",
                    &p[..4.min(p.len())],
                    &p[p.len().saturating_sub(4)..]
                ))
                .unwrap_or_else(|| "<none>".into())
        );
        vak_core::security_events::record(
            &home,
            vak_core::security_events::EventKind::AuthFailure,
            "auth_failure",
            &detail,
            ip,
        );
        if let Some(hub) = events::global() {
            hub.emit_security("AuthFailure", req.uri().path());
        }
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn health_projection(state: &AppState) -> serde_json::Value {
    let report = vak_core::health::collect(&state.core, None);
    let checks: Vec<serde_json::Value> = report
        .checks
        .into_iter()
        .map(|check| match check.detail {
            Ok(detail) => {
                serde_json::json!({ "label": check.label, "status": "pass", "detail": detail })
            }
            Err(detail) => {
                serde_json::json!({ "label": check.label, "status": "fail", "detail": detail })
            }
        })
        .collect();
    let route = state.core.effective_route();
    let posture = if report.failures == 0 {
        "healthy"
    } else {
        "degraded"
    };
    serde_json::json!({
        // `status = ok` is retained for existing health clients; posture is
        // the truthful operational signal and is what the Operations Center
        // renders. This keeps the compatibility contract without hiding
        // failed doctor checks.
        "status": "ok",
        "posture": posture,
        "provider": route.provider,
        "model": route.model,
        "provider_source": route.provider_source,
        "model_source": route.model_source,
        "route_revision": route.revision,
        "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
        "approval_mode": state.core.effective_approval_mode().as_str(),
        "sandbox": state.core.effective_sandbox_name(),
        "context_window": state.core.config().context_window,
        "voice": {
            "enabled": state.core.effective_voice().enabled,
            "provider": state.core.effective_voice().provider,
            "model": state.core.effective_voice().model,
            "max_session_secs": state.core.effective_voice().max_session_secs,
            "max_concurrent": state.core.effective_voice().max_concurrent,
            "max_audio_bytes": state.core.effective_voice().max_audio_bytes,
            "source": "effective",
            "active_sessions": state.voice_active.load(std::sync::atomic::Ordering::Relaxed),
            "capacity_remaining": state.core.effective_voice().max_concurrent.saturating_sub(
                state.voice_active.load(std::sync::atomic::Ordering::Relaxed),
            ),
            "quota": {
                "session_seconds": state.core.effective_voice().max_session_secs,
                "concurrent_sessions": state.core.effective_voice().max_concurrent,
                "inbound_audio_bytes": state.core.effective_voice().max_audio_bytes,
                "scope": "workspace",
                "source": "effective",
            },
            // Historical voice telemetry is intentionally unavailable until
            // it is derived from persisted evidence; never fabricate zeros.
            "historical": {
                "available": false,
                "reason": "No persisted voice latency, error, or cost aggregates are available"
            },
        },
        "cwd": state.core.cwd(),
        "warnings": state.core.config().warnings,
        "checks": checks,
        "facts": report.facts,
        "failures": report.failures,
    })
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    refresh_control_plane(&state);
    Json(health_projection(&state))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn register_handle(
    state: &AppState,
    id: String,
    session: SessionLog,
    cwd: PathBuf,
    core: Core,
) -> Arc<SessionHandle> {
    let core = core
        .with_agent_identity(session.header().and_then(|header| header.agent.clone()))
        .with_conversation_context(
            session
                .header()
                .and_then(|header| header.conversation.clone()),
        );
    let durable_home = core.sessions_home();
    let latest_intent = session.chain_to_root().iter().rev().find_map(|entry| {
        if let vak_session::EntryPayload::Intent(record) = &entry.payload {
            Some((**record).clone())
        } else {
            None
        }
    });
    let events_tx = events::EventBus::new();
    let side_events_tx = events::EventBus::new();
    let planner = delivery::merged_presentation_planner(&core);
    let adaptive_store = vak_store::presentation::PresentationStore::new(
        core.sessions_home().join("presentations.json"),
    );
    let mut presentation_snapshot = match adaptive_store.load() {
        Ok(library) => {
            crate::projection::snapshot_with_planner_and_library(&id, &session, &planner, &library)
        }
        Err(_) => crate::projection::snapshot_with_planner(&id, &session, &planner),
    };
    crate::projection::append_sandbox_artifacts(&mut presentation_snapshot, &durable_home, &id);
    let presentation = Arc::new(Mutex::new(presentation_snapshot));
    let mut presentation_rx = events_tx.subscribe();
    let presentation_state = presentation.clone();
    let presentation_activities = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(SessionHandle {
        id: id.clone(),
        core,
        cwd,
        session: Arc::new(Mutex::new(Some(session))),
        intent: Arc::new(Mutex::new(latest_intent)),
        steering: Arc::new(SteeringQueues::new()),
        cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
        events_tx,
        pending: Arc::new(Mutex::new(HashMap::new())),
        activity_buffer: presentation_activities.clone(),
        presentation,
        subscribed: Arc::new(tokio::sync::Notify::new()),
        side_events_tx,
        last_touched: Mutex::new(std::time::Instant::now()),
        side_cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
        admissions: Arc::new(Mutex::new(HashSet::new())),
    });
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let durable_session_id = id.clone();
        runtime.spawn(async move {
            loop {
                match presentation_rx.recv().await {
                    Ok(framed) => {
                        let event = framed.event.clone();
                        append_session_sandbox_event(&durable_home, &durable_session_id, &event);
                        crate::projection::project_frame(
                            &mut presentation_state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner),
                            framed,
                        );
                        let activity = match event {
                            AgentEvent::WorkerStarted { label } => {
                                Some(vak_session::ActivityRecord {
                                    activity_id: format!("worker-{label}"),
                                    turn: None,
                                    kind: vak_session::ActivityKind::Worker,
                                    status: vak_session::ActivityStatus::Running,
                                    label,
                                    detail: Some("Worker started".into()),
                                    data: std::collections::BTreeMap::new(),
                                })
                            }
                            AgentEvent::WorkerFinished {
                                label,
                                is_error,
                                elapsed_ms,
                            } => Some(vak_session::ActivityRecord {
                                activity_id: format!("worker-{label}"),
                                turn: None,
                                kind: vak_session::ActivityKind::Worker,
                                status: if is_error {
                                    vak_session::ActivityStatus::Failed
                                } else {
                                    vak_session::ActivityStatus::Succeeded
                                },
                                label,
                                detail: Some(format!("Completed in {elapsed_ms} ms")),
                                data: std::collections::BTreeMap::new(),
                            }),
                            _ => None,
                        };
                        if let Some(activity) = activity {
                            presentation_activities
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(activity);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }
    state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id, handle.clone());
    state.evict_idle_sessions();
    handle
}

async fn create_session(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    refresh_control_plane(&state);
    // The workspace the client currently has open, which on the web is
    // switchable at runtime (docs/design/48-web-client.md §5). Existing
    // sessions keep the `Core` they froze at creation (invariant 17); this
    // only decides where the NEXT task lives.
    let core = state.active_core();
    let session = match core.start_session().await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let id = session
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();
    register_handle(
        &state,
        id.clone(),
        session,
        core.cwd().clone(),
        core.clone(),
    );

    state.hub.emit_session_created(&id, "");
    index_session_later(state.store.clone(), state.core.sessions_home(), id.clone());

    Json(serde_json::json!({ "session_id": id })).into_response()
}

/// Re-index one session's JSONL in the background. Reading does not
/// conflict with the live handle's exclusive write lock.
pub(crate) fn index_session_later(
    store: Option<vak_store::Store>,
    home: std::path::PathBuf,
    session_id: String,
) {
    let Some(store) = store else {
        return;
    };
    tokio::spawn(async move {
        import_session_sync(&store, &home, &session_id);
    });
}

/// Locate `<home>/sessions/<hash>/<session>.jsonl` and import it into the
/// index synchronously. Idempotent; cheap when nothing changed.
pub(crate) fn import_session_sync(
    store: &vak_store::Store,
    home: &std::path::Path,
    session_id: &str,
) -> bool {
    let dir = home.join("sessions");
    if let Ok(read) = std::fs::read_dir(&dir) {
        for project in read.flatten() {
            let candidate = project.path().join(format!("{session_id}.jsonl"));
            if candidate.is_file()
                && let Ok(stats) = store.import_session(home, &candidate)
            {
                return stats.entries_indexed > 0 || stats.skipped > 0;
            }
        }
    }
    let shared = if home.join("agents").is_dir() {
        home.to_path_buf()
    } else if let Some(parent) = home.parent().and_then(|p| p.parent()) {
        parent.to_path_buf()
    } else {
        home.to_path_buf()
    };
    if let Ok(agents) = std::fs::read_dir(shared.join("agents")) {
        for agent in agents.flatten() {
            let agent_home = agent.path();
            let agent_sessions = agent_home.join("sessions");
            if let Ok(projects) = std::fs::read_dir(&agent_sessions) {
                for project in projects.flatten() {
                    let candidate = project.path().join(format!("{session_id}.jsonl"));
                    if candidate.is_file()
                        && let Ok(stats) = store.import_session(&agent_home, &candidate)
                    {
                        return stats.entries_indexed > 0 || stats.skipped > 0;
                    }
                }
            }
        }
    }
    false
}

#[derive(serde::Deserialize)]
struct AttachBody {
    session_id: String,
}

async fn attach_session(
    State(state): State<AppState>,
    Json(body): Json<AttachBody>,
) -> axum::response::Response {
    // Already attached? Return before touching the file.
    //
    // The live handle owns an exclusive lock on the session JSONL for its
    // whole lifetime, and the lock is per open-file-description: opening the
    // same path again from THIS process conflicts with our own handle just
    // as it would with a stranger's. Re-attaching is routine — the desktop
    // calls it on every task switch, and mid-run the handle's session is
    // temporarily owned by the agent — so this must be a no-op, not a
    // second open.
    if state.get(&body.session_id).is_some() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "session_id": body.session_id })),
        )
            .into_response();
    }
    let session = if let Ok(s) = state.active_core().open_session(&body.session_id).await {
        Ok(s)
    } else if let Ok(s) = state.core.open_session(&body.session_id).await {
        Ok(s)
    } else if let Ok(s) = state
        .active_core()
        .open_session_read_only(&body.session_id)
        .await
    {
        Ok(s)
    } else if let Ok(s) = state.core.open_session_read_only(&body.session_id).await {
        Ok(s)
    } else {
        find_session_on_disk(&state.core, &body.session_id).ok_or_else(|| {
            vak_core::CoreError::Session(vak_session::SessionError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("session not found: {}", body.session_id),
            )))
        })
    };
    match session {
        Ok(session) => {
            let id = session
                .header()
                .map(|h| h.session_id.clone())
                .unwrap_or_else(|| body.session_id.clone());
            let session_cwd = session
                .header()
                .map(|h| h.cwd.clone())
                .unwrap_or_else(|| state.core.cwd().clone());
            let active = state.active_core();
            let handle_core = if session_cwd == *active.cwd() {
                active
            } else if session_cwd == *state.core.cwd() {
                state.core.clone()
            } else if let Ok(c) =
                state
                    .gateway
                    .core_pool
                    .resolve_at(&session_cwd, None, std::time::Instant::now())
            {
                c
            } else {
                // `resolve_at` failing here is not a trust decision — an
                // unconditional `true` would let a workspace whose trust
                // prompt an operator declined have its hooks/MCP
                // servers/`.env` applied anyway. Recompute trust the same
                // way `resolve_at` does rather than assuming it.
                vak_core::Core::new_with_trust(
                    session_cwd.clone(),
                    vak_core::trust::is_trusted(&session_cwd),
                )
                .unwrap_or_else(|_| state.core.clone())
            };
            // The header id can differ from the requested one; if that handle
            // is already live, keep it rather than replacing it.
            if state.get(&id).is_none() {
                register_handle(&state, id.clone(), session, session_cwd, handle_core);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({ "session_id": id })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Sidebar projection over the persisted store: one summary per JSONL file.
async fn list_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Sessions are stored per workspace, so this follows the workspace the
    // client has open rather than the one the process started in.
    let active = state.active_core();
    let dir = vak_session::SessionPath::sessions_dir(&state.core.sessions_home(), active.cwd());
    let active_cwd = active.cwd().to_string_lossy().into_owned();
    let archive_map = read_archive(&state.core);
    let deleted_map = read_deleted(&state.core);
    let mut sessions = Vec::new();
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir)
        .map(|read| read.flatten().collect())
        .unwrap_or_default();
    // A workspace can be renamed or canonicalized between runs (notably
    // `/var` vs `/private/var` on macOS). Recover sessions by their durable
    // header cwd when the hashed directory no longer matches, while still
    // filtering strictly to the active workspace.
    if let Ok(projects) = std::fs::read_dir(state.core.sessions_home().join("sessions")) {
        for project in projects.flatten() {
            if let Ok(files) = std::fs::read_dir(project.path()) {
                for file in files.flatten() {
                    let duplicate = entries
                        .iter()
                        .any(|existing| existing.path() == file.path());
                    if !duplicate {
                        entries.push(file);
                    }
                }
            }
        }
    }
    let shared_home = state.core.shared_data_home();
    if let Ok(agents) = std::fs::read_dir(shared_home.join("agents")) {
        for agent in agents.flatten() {
            if let Ok(projects) = std::fs::read_dir(agent.path().join("sessions")) {
                for project in projects.flatten() {
                    if let Ok(files) = std::fs::read_dir(project.path()) {
                        for file in files.flatten() {
                            let duplicate = entries
                                .iter()
                                .any(|existing| existing.path() == file.path());
                            if !duplicate {
                                entries.push(file);
                            }
                        }
                    }
                }
            }
        }
    }
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(session_id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        if deleted_map.get(&session_id).copied().unwrap_or(false) {
            continue;
        }
        let updated_at = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
        let (created_at, title, entry_count, cwd, agent) = summarize_jsonl(&path);
        // A user-created Agent lives in its own isolated workspace (see
        // agent_chats::open / agent_workspace), independent of whichever
        // default workspace the browser client currently has open — that
        // switch only ever applied to the built-in "vak" identity, which
        // still shares the process's default workspace. So only the "vak"
        // (or header-less/legacy) sessions are filtered by `active_cwd`;
        // every other Agent's sessions are always its own to show.
        let is_default_agent = agent.as_ref().is_none_or(|a| a.id == "vak");
        if is_default_agent && cwd.as_deref() != Some(active_cwd.as_str()) {
            continue;
        }
        // Header-only sessions are abandoned drafts (for example, creating a
        // task and immediately switching away). Keep the ledger append-only,
        // but do not let empty drafts accumulate in the task switcher. This
        // applies equally to built-in and user-created Agents, but actively
        // registered sessions must remain discoverable.
        let is_active = state.get(&session_id).is_some();
        if entry_count <= 1 && !is_active {
            continue;
        }
        let running = state.get(&session_id).is_some_and(|handle| {
            handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        });
        let archived = archive_map.get(&session_id).copied().unwrap_or(false);
        sessions.push(serde_json::json!({
            "session_id": session_id,
            "cwd": cwd.unwrap_or_else(|| state.core.cwd().to_string_lossy().into_owned()),
            "created_at": created_at,
            "updated_at": updated_at,
            "entries": entry_count,
            "agent": agent,
            "title": title,
            "running": running,
            "archived": archived,
        }));
    }
    sessions.sort_by_key(|s| s["updated_at"].as_str().unwrap_or("").to_string());
    sessions.reverse();
    Json(serde_json::json!({ "sessions": sessions }))
}

/// Bounded scan: header line for created_at + first user message as title.
fn summarize_jsonl(
    path: &std::path::Path,
) -> (
    Option<String>,
    Option<String>,
    u64,
    Option<String>,
    Option<vak_session::types::AgentIdentity>,
) {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else {
        return (None, None, 0, None, None);
    };
    let mut reader = std::io::BufReader::new(file);
    let mut created_at = None;
    let mut title = None;
    let mut cwd = None;
    let mut agent = None;
    let mut entries = 0u64;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                entries += 1;
                if let Ok(entry) = serde_json::from_str::<vak_session::Entry>(line.trim()) {
                    match entry.payload {
                        vak_session::EntryPayload::Header(h) => {
                            created_at = Some(h.created_at.to_rfc3339());
                            cwd = Some(h.cwd.to_string_lossy().into_owned());
                            agent = h.agent;
                        }
                        vak_session::EntryPayload::Message(rec) => {
                            if title.is_none() && rec.message.role == vak_llm::Role::User {
                                let text = rec.message.text_content();
                                let text = text.trim();
                                if !text.is_empty() {
                                    let first_line = text.lines().next().unwrap_or(text).trim();
                                    let mut snippet: String = first_line.chars().take(72).collect();
                                    if first_line.chars().count() > 72 {
                                        snippet.push('…');
                                    }
                                    title = Some(snippet);
                                }
                            }
                        }
                        vak_session::EntryPayload::Compaction(_) => {}
                        vak_session::EntryPayload::Receipt(_) => {}
                        vak_session::EntryPayload::Goal(_) => {}
                        vak_session::EntryPayload::GoalUpdate(_) => {}
                        vak_session::EntryPayload::Activity(_) => {}
                        vak_session::EntryPayload::Work(_) => {}
                        vak_session::EntryPayload::Intent(_) => {}
                        vak_session::EntryPayload::TurnCapabilitiesBound(_) => {}
                        vak_session::EntryPayload::ChildRun { .. } => {}
                    }
                }
                if title.is_some() && entries > 400 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    (created_at, title, entries, cwd, agent)
}

#[derive(serde::Deserialize)]
struct RoutingEnvelope {
    /// The user message that caused this admission. This is metadata, not a
    /// capability or an instruction to the model.
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    target_work_id: Option<String>,
    #[serde(default)]
    target_result_id: Option<String>,
    /// `independent`, `follow_up`, `correction`, `status`, `cancel`, or
    /// `schedule`; unknown values are retained as provenance but never used
    /// to authorize work.
    #[serde(default)]
    relation: Option<String>,
    #[serde(default)]
    outcome_revision: Option<u64>,
    #[serde(default)]
    provenance: Option<String>,
}

#[derive(serde::Deserialize)]
struct RunBody {
    prompt: String,
    /// Stable client identity used to make network retries idempotent.
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    routing: Option<RoutingEnvelope>,
    /// Optional run-scoped work profile. `managed` creates and persists a
    /// work contract before the agent can execute tools.
    #[serde(default)]
    work_mode: Option<String>,
    /// Optional base64 images appended to the prompt as vision content
    /// (docs/design/22-gateway.md media passthrough).
    #[serde(default)]
    attachments: Vec<RunAttachment>,
    /// Goal mode (docs/design/42-managed-work-contracts.md): durable objective; completion
    /// is audited against `criteria`, never self-reported.
    #[serde(default)]
    goal: Option<String>,
    /// Acceptance criteria for goal mode (`verify:` prefixed criteria run
    /// as brokered shell commands; others are judged from evidence).
    #[serde(default)]
    criteria: Vec<String>,
}

#[derive(serde::Deserialize)]
struct RunAttachment {
    #[serde(default = "default_image_mime")]
    mime: String,
    data: String,
}

fn default_image_mime() -> String {
    "image/png".into()
}

pub(crate) fn mpsc_to_broadcast(tx: events::EventBus) -> mpsc::Sender<AgentEvent> {
    let (tx_in, mut rx) = mpsc::channel::<AgentEvent>(512);
    tokio::spawn(async move {
        // Forward into the BROADCAST channel (sync send). Forwarding into
        // tx_in would feed the channel back into itself.
        //
        // Headless consumers (gateway turns, cron routines) legitimately run
        // with zero broadcast subscribers; send errors must NEVER tear the
        // pump down — the agent treats a dropped mpsc receiver as a lost
        // consumer and cancels the run mid-flight.
        while let Some(ev) = rx.recv().await {
            let _ = tx.send(ev);
        }
    });
    tx_in
}

/// A run cannot start without a working provider credential.
///
/// Every one of these three call sites used to return a bare 503 with no
/// body and nothing logged, which made "the agent never replied" a
/// symptom with no server-side trail: a client saw an empty response, an
/// operator reading gateway.log saw nothing at all, and diagnosing it
/// meant reading this file. `Core::provider()` already carries a precise
/// `CoreError::MissingAuth { env, provider }` — this puts it where an
/// operator and a client can both actually see it.
///
/// The body is `{"error": <message>}`, matching every other handler in
/// this file. A `{"error": <code>, "detail": <message>}` shape was tried
/// first and reverted: the desktop frontend's error handling already
/// reads `.error` as the human-readable string every other endpoint puts
/// there, so a two-field body would have shown the user the machine code
/// ("provider_unavailable") instead of the message that says what to fix.
fn provider_unavailable(err: vak_core::CoreError) -> axum::response::Response {
    use axum::response::IntoResponse;
    let detail = err.to_string();
    eprintln!("[run] refused: {detail}");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({ "error": detail })),
    )
        .into_response()
}

/// `run_prompt`, `side_chat`, and `start_bestofn` all fall back to
/// `provider_unavailable` when `Core::provider()` fails; this pins the
/// response it produces so a regression — an empty body, or the
/// `{"error": <code>, "detail": <message>}` shape tried and reverted
/// above — fails a fast unit test instead of surfacing as "the agent
/// never replied" with nothing in gateway.log to explain why.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod provider_unavailable_tests {
    use super::provider_unavailable;
    use axum::response::IntoResponse as _;
    use http_body_util::BodyExt as _;

    #[tokio::test]
    async fn reports_status_and_a_body_naming_the_missing_credential() {
        let err = vak_core::CoreError::MissingAuth {
            env: "ANTHROPIC_API_KEY".into(),
            provider: "anthropic".into(),
        };
        let response = provider_unavailable(err).into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );

        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body readable")
            .to_bytes();
        assert!(
            !bytes.is_empty(),
            "body must not be empty — that was the original bug"
        );

        let body: serde_json::Value = serde_json::from_slice(&bytes).expect("body is JSON");
        // Single `error` field carrying the human-readable message, same
        // shape as every other handler in this file — not a machine code
        // in `error` with the message hidden in a `detail` the frontend
        // never reads.
        let fields: Vec<&String> = body.as_object().expect("object body").keys().collect();
        assert_eq!(
            fields,
            vec!["error"],
            "body must have exactly the `error` field"
        );
        let message = body["error"].as_str().expect("error is a string");
        assert!(
            message.contains("ANTHROPIC_API_KEY"),
            "message must name the env var to set, got: {message}"
        );
        assert!(
            message.contains("anthropic"),
            "message must name the provider, got: {message}"
        );
    }
}

async fn run_prompt(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RunBody>,
) -> axum::response::Response {
    refresh_control_plane(&state);
    use axum::response::IntoResponse;
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(routing) = body.routing.as_ref()
        && let Some(expected) = routing.outcome_revision
        && !routing_revision_is_current(&handle, expected)
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "target result is stale; refresh before continuing",
                "target_revision": expected,
            })),
        )
            .into_response();
    }
    let request_id = body.request_id.clone();
    if let Some(request_id) = request_id.as_deref()
        && handle
            .admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(request_id)
    {
        return StatusCode::ACCEPTED.into_response();
    }
    let Some(mut taken) = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        if request_id.is_some() {
            return StatusCode::ACCEPTED.into_response();
        }
        return StatusCode::CONFLICT.into_response(); // run already active
    };
    if taken.is_read_only() {
        match vak_session::SessionLog::open(taken.path().to_path_buf()) {
            Ok(writable) => {
                taken = writable;
            }
            Err(vak_session::SessionError::Locked(_)) => {
                *handle
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "This conversation is currently active in Vak Desktop. Close or finish the task in Desktop before continuing here."
                    })),
                )
                    .into_response();
            }
            Err(e) => {
                *handle
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
    }
    if let Some(request_id) = request_id.as_deref()
        && taken.has_request_admission(request_id)
    {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        return StatusCode::ACCEPTED.into_response();
    }
    if let Err(e) = handle.core.provider() {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        return provider_unavailable(e);
    }
    if let Some(request_id) = request_id.as_deref() {
        let mut data = std::collections::BTreeMap::new();
        data.insert("request_id".into(), request_id.to_owned());
        if let Some(routing) = body.routing.as_ref() {
            record_routing_data(&mut data, routing);
        }
        if taken
            .append_activity(vak_session::ActivityRecord {
                activity_id: format!("admission-{request_id}"),
                turn: None,
                kind: vak_session::ActivityKind::Run,
                status: vak_session::ActivityStatus::Running,
                label: "Request accepted".into(),
                detail: None,
                data,
            })
            .is_err()
        {
            *handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(
                    serde_json::json!({"error": "could not durably record request admission"}),
                ),
            )
                .into_response();
        }
        handle
            .admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request_id.to_owned());
    }

    // Give SSE consumers a moment to attach so terminal events are seen.
    let _ = tokio::time::timeout(Duration::from_secs(2), handle.subscribed.notified()).await;

    // Driven by a client that is holding the SSE stream open, so a gate
    // raised here reaches a person.
    let approver: Arc<dyn Approver> = Arc::new(HttpApprover {
        events_tx: handle.events_tx.clone(),
        pending: handle.pending.clone(),
        session_id: handle.id.clone(),
        activity_buffer: handle.activity_buffer.clone(),
        answerable: true,
    });
    let events = mpsc_to_broadcast(handle.events_tx.clone());
    let steering = handle.steering.clone();
    let cancel = handle
        .cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let core = handle.core.clone();

    let expanded_prompt = body.prompt.clone();
    let prompt_message = if body.attachments.is_empty() {
        None
    } else {
        let mut blocks = vec![vak_llm::ContentBlock::text(expanded_prompt.clone())];
        for a in &body.attachments {
            if a.data.trim().is_empty() {
                continue;
            }
            blocks.push(vak_llm::ContentBlock::image_base64(
                a.mime.clone(),
                a.data.trim().to_string(),
            ));
        }
        Some(vak_llm::Message {
            role: vak_llm::Role::User,
            content: blocks,
        })
    };
    let preview_message = prompt_message
        .clone()
        .unwrap_or_else(|| vak_llm::Message::user_text(expanded_prompt.clone()));
    let preview_intent = core.resolve_turn_intent(&taken, &preview_message);
    let mut preview_outcome = vak_intent::OutcomeSpec::from_reading(
        &expanded_prompt,
        &preview_intent.reading,
        preview_intent.provenance.resolver_version,
    );
    preview_outcome.evidence_max_age_secs = Some(core.effective_evidence_max_age_secs());
    preview_outcome.max_turns = preview_intent.engagement.limits.max_turns;
    *handle
        .intent
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(vak_session::types::IntentRecord {
            reading: preview_intent.reading,
            engagement: preview_intent.engagement,
            provenance: preview_intent.provenance,
            outcome: Some(preview_outcome),
            model_visible: None,
            commitment_id: None,
        });
    if body.goal.is_some() && !body.attachments.is_empty() {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        let _ = handle.events_tx.send(AgentEvent::RunFinished {
            summary: "failed: goal runs do not support attachments".into(),
            is_error: true,
        });
        return StatusCode::BAD_REQUEST.into_response();
    }
    if let Some(mode) = body.work_mode.as_deref()
        && !matches!(mode, "direct" | "managed" | "auto")
    {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        let _ = handle.events_tx.send(AgentEvent::RunFinished {
            summary: format!("failed: unknown work_mode '{mode}'"),
            is_error: true,
        });
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Goal mode (Phase H): captured before the spawn consumes `body`.
    let goal_pair = body.goal.clone().map(|g| (g, body.criteria.clone()));
    let managed = matches!(body.work_mode.as_deref(), Some("managed"));
    let automatic = matches!(body.work_mode.as_deref(), Some("auto"));
    if (managed || automatic) && !body.attachments.is_empty() {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        let _ = handle.events_tx.send(AgentEvent::RunFinished {
            summary: "failed: managed work currently requires text-only input".into(),
            is_error: true,
        });
        return StatusCode::BAD_REQUEST.into_response();
    }
    let run_id = id.clone();
    let hub = state.hub.clone();
    let admin_store = state.store.clone();
    let sessions_home = state.core.sessions_home();

    tokio::spawn(async move {
        let outcome = if let Some((objective, criteria)) = goal_pair {
            core.run_goal_turn_with(
                taken,
                &expanded_prompt,
                &objective,
                criteria,
                cancel.clone(),
                Some(approver.clone()),
                None,
                Some(steering.clone()),
                events,
            )
            .await
        } else if managed {
            core.run_managed_turn_with(
                taken,
                &expanded_prompt,
                cancel,
                Some(approver),
                None,
                Some(steering.clone()),
                events,
            )
            .await
        } else if automatic {
            core.run_auto_turn_with(
                taken,
                &expanded_prompt,
                cancel,
                Some(approver),
                None,
                Some(steering.clone()),
                events,
            )
            .await
        } else if let Some(msg) = prompt_message {
            core.run_turn_with_message(
                taken,
                msg,
                cancel,
                Some(approver),
                None,
                Some(steering.clone()),
                events,
            )
            .await
        } else {
            core.run_turn_with(
                taken,
                &expanded_prompt,
                cancel,
                Some(approver),
                None,
                Some(steering.clone()),
                events,
            )
            .await
        };
        // Reset the token so the next run on this session is not born
        // already-cancelled.
        *handle
            .cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = CancellationToken::new();
        match outcome {
            Ok((o, mut session_log)) => {
                let (summary, is_error) = match &o {
                    vak_agent::TurnOutcome::Completed { .. } => ("completed".to_string(), false),
                    vak_agent::TurnOutcome::Aborted { .. } => ("aborted".to_string(), false),
                    vak_agent::TurnOutcome::Failed { error } => (format!("failed: {error}"), true),
                    vak_agent::TurnOutcome::MaxTurnsReached => ("max_turns".to_string(), true),
                };
                let activity_status = match &o {
                    vak_agent::TurnOutcome::Completed { .. } => {
                        vak_session::ActivityStatus::Succeeded
                    }
                    vak_agent::TurnOutcome::Aborted { .. } => {
                        vak_session::ActivityStatus::Cancelled
                    }
                    vak_agent::TurnOutcome::Failed { .. }
                    | vak_agent::TurnOutcome::MaxTurnsReached => {
                        vak_session::ActivityStatus::Failed
                    }
                };
                let _ = session_log.append_activity(vak_session::ActivityRecord {
                    activity_id: format!("run-{run_id}-{}", chrono::Utc::now().timestamp_micros()),
                    turn: None,
                    kind: vak_session::ActivityKind::Run,
                    status: activity_status,
                    label: "Run finished".into(),
                    detail: Some(summary.clone()),
                    data: std::collections::BTreeMap::new(),
                });
                let buffered = std::mem::take(
                    &mut *handle
                        .activity_buffer
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                for activity in buffered {
                    let request_id = activity.data.get("request_id").cloned();
                    let _ = session_log.append_activity(activity);
                    if let Some(request_id) = request_id {
                        handle
                            .admissions
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&request_id);
                    }
                }
                *handle
                    .presentation
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    crate::projection::snapshot(&run_id, &session_log);
                hub.emit_agent_summary(&summary, Some(run_id.clone()));
                let _ = handle.events_tx.send(AgentEvent::RunFinished {
                    summary: summary.clone(),
                    is_error,
                });
                index_session_later(admin_store.clone(), sessions_home.clone(), run_id.clone());
                // Background reflection seam (docs/design/29 P1): after the
                // summary is recorded and while this task still owns the
                // ledger (a second in-process handle cannot take the file
                // lock). Bounded; the result is deliberately ignored — a
                // completed run never fails on reflection.
                if !is_error && core.config().memory.reflection {
                    let _ = tokio::time::timeout(
                        REFLECTION_CALL_TIMEOUT,
                        core.reflect_after_turn(&session_log, ""),
                    )
                    .await;
                }
                // Return the ledger so transcript stays available.
                *handle
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session_log);
            }
            Err(e) => {
                // Same leak class: restore from the durable ledger so the
                // handle does not stay wedged on "run in progress".
                if let Some(mut restored) = reopen_ledger(&core, &run_id) {
                    for activity in std::mem::take(
                        &mut *handle
                            .activity_buffer
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    ) {
                        let _ = restored.append_activity(activity);
                    }
                    *handle
                        .presentation
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        crate::projection::snapshot(&run_id, &restored);
                    *handle
                        .session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(restored);
                }
                let _ = handle.events_tx.send(AgentEvent::RunFinished {
                    summary: format!("error: {e}"),
                    is_error: true,
                });
            }
        }
        if let Some(request_id) = request_id {
            handle
                .admissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&request_id);
        }
        drop(steering);
    });

    StatusCode::ACCEPTED.into_response()
}

#[derive(serde::Deserialize)]
struct SteeringBody {
    text: String,
    /// Caller-owned id used to recover a retry without enqueuing duplicate
    /// steering input. Older callers may omit it; the server then generates
    /// one for the single attempt.
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    routing: Option<RoutingEnvelope>,
    /// Origin is metadata for the audit trail, never an authority grant.
    #[serde(default = "default_intervention_source")]
    source: String,
    /// Optional base64 images appended to the steered prompt, mirroring
    /// /run so queued input is never degraded to bare text.
    #[serde(default)]
    attachments: Vec<RunAttachment>,
}

fn default_intervention_source() -> String {
    "human".into()
}

async fn send_steering(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SteeringBody>,
) -> axum::response::Response {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(routing) = body.routing.as_ref()
        && let Some(expected) = routing.outcome_revision
        && !routing_revision_is_current(&handle, expected)
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "target result is stale; refresh before continuing",
                "target_revision": expected,
            })),
        )
            .into_response();
    }
    let request_id = body
        .request_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("intervention-{}", uuid::Uuid::now_v7()));
    let already_admitted = handle
        .admissions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&request_id)
        || handle
            .session
            .lock()
            .ok()
            .and_then(|guard| {
                guard
                    .as_ref()
                    .map(|log| log.has_request_admission(&request_id))
            })
            .unwrap_or(false);
    if already_admitted {
        return (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "request_id": request_id,
                "decision": "duplicate",
                "state": "already_admitted",
            })),
        )
            .into_response();
    }
    handle
        .admissions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(request_id.clone());
    let kind = vak_intent::classify_intervention(&body.text);
    let evaluation = vak_intent::evaluate_intervention(vak_intent::InterventionRequest {
        request_id: request_id.clone(),
        kind: kind.clone(),
        text: body.text.clone(),
        source: body.source.clone(),
        target_revision: None,
    });
    record_activity_or_buffer(
        &handle,
        vak_session::ActivityRecord {
            activity_id: evaluation.request.request_id.clone(),
            turn: None,
            kind: vak_session::ActivityKind::Diagnostic,
            status: if evaluation.decision == vak_intent::InterventionDecision::Queued {
                vak_session::ActivityStatus::Pending
            } else {
                vak_session::ActivityStatus::Succeeded
            },
            label: if evaluation.decision == vak_intent::InterventionDecision::Queued {
                "Intervention queued"
            } else {
                "Intervention accepted"
            }
            .into(),
            detail: Some(body.text.clone()),
            data: std::collections::BTreeMap::from([
                ("kind".into(), kind.as_str().into()),
                ("decision".into(), evaluation.decision.as_str().into()),
                ("source".into(), body.source.clone()),
                ("reason".into(), evaluation.reason.clone()),
                ("routing".into(), routing_summary(body.routing.as_ref())),
            ]),
        },
    );
    if evaluation.decision == vak_intent::InterventionDecision::RequiresHuman {
        handle
            .admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&request_id);
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "request_id": evaluation.request.request_id,
                "decision": evaluation.decision.as_str(),
                "reason": evaluation.reason,
            })),
        )
            .into_response();
    }
    match kind {
        vak_intent::InterventionKind::Status => {
            handle
                .admissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&request_id);
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "request_id": evaluation.request.request_id,
                    "decision": evaluation.decision.as_str(),
                    "state": "status_requested",
                })),
            )
                .into_response();
        }
        vak_intent::InterventionKind::Pause => {
            handle
                .admissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&request_id);
            handle.steering.pause();
            record_control_activity(&handle, "Run paused", "pause");
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "request_id": evaluation.request.request_id,
                    "decision": evaluation.decision.as_str(),
                    "state": "paused",
                    "reason": evaluation.reason,
                })),
            )
                .into_response();
        }
        vak_intent::InterventionKind::Resume => {
            handle
                .admissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&request_id);
            handle.steering.resume();
            record_control_activity(&handle, "Run resumed", "resume");
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "request_id": evaluation.request.request_id,
                    "decision": evaluation.decision.as_str(),
                    "state": "resumed",
                    "reason": evaluation.reason,
                })),
            )
                .into_response();
        }
        vak_intent::InterventionKind::Cancel => {
            handle
                .admissions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&request_id);
            handle
                .cancel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .cancel();
            record_control_activity(&handle, "Run cancelled", "cancel");
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "request_id": evaluation.request.request_id,
                    "decision": evaluation.decision.as_str(),
                    "state": "cancelled",
                    "reason": evaluation.reason,
                })),
            )
                .into_response();
        }
        _ => {}
    }
    let usable: Vec<&RunAttachment> = body
        .attachments
        .iter()
        .filter(|a| !a.data.trim().is_empty())
        .collect();
    if usable.is_empty() {
        handle.steering.push_steering(body.text);
    } else {
        let mut blocks = vec![vak_llm::ContentBlock::text(body.text.clone())];
        for a in usable {
            blocks.push(vak_llm::ContentBlock::image_base64(
                a.mime.clone(),
                a.data.trim().to_string(),
            ));
        }
        handle.steering.push_steering_message(vak_llm::Message {
            role: vak_llm::Role::User,
            content: blocks,
        });
    }
    // If the ledger was available, the activity is durable already and the
    // in-memory guard can be released. A busy runner keeps it until its
    // buffered activity is flushed at turn completion.
    if handle
        .session
        .lock()
        .ok()
        .is_some_and(|guard| guard.is_some())
    {
        handle
            .admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&request_id);
    }
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "request_id": evaluation.request.request_id,
            "decision": evaluation.decision.as_str(),
            "state": "steering_queued",
        })),
    )
        .into_response()
}

async fn cancel_run(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    handle
        .cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cancel();
    record_control_activity(&handle, "Run cancelled", "cancel");
    deny_pending_approvals(&handle);
    let _ = handle.events_tx.send(AgentEvent::RunFinished {
        summary: "cancelled by client".into(),
        is_error: false,
    });
    StatusCode::ACCEPTED
}

async fn pause_run(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    handle.steering.pause();
    record_control_activity(&handle, "Run paused", "pause");
    StatusCode::ACCEPTED
}

async fn resume_run(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    handle.steering.resume();
    record_control_activity(&handle, "Run resumed", "resume");
    StatusCode::ACCEPTED
}

async fn control_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let running = handle
        .session
        .lock()
        .map(|session| session.is_none())
        .unwrap_or(false);
    let revision = handle
        .session
        .lock()
        .ok()
        .and_then(|guard| {
            guard.as_ref().and_then(|session| {
                session.chain_to_root().iter().rev().find_map(|entry| {
                    if let vak_session::EntryPayload::Intent(record) = &entry.payload {
                        record.outcome.as_ref().map(|outcome| outcome.revision)
                    } else {
                        None
                    }
                })
            })
        })
        .or_else(|| {
            handle.intent.lock().ok().and_then(|guard| {
                guard
                    .as_ref()?
                    .outcome
                    .as_ref()
                    .map(|outcome| outcome.revision)
            })
        })
        .unwrap_or(0);
    Json(serde_json::json!({
        "running": running,
        "paused": handle.steering.is_paused(),
        "revision": revision,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct PlanChangeBody {
    text: String,
    #[serde(default = "default_intervention_source")]
    source: String,
    #[serde(default)]
    target_revision: Option<u64>,
}

fn apply_plan_change(
    mut outcome: vak_intent::OutcomeSpec,
    request: &vak_intent::InterventionRequest,
) -> (vak_intent::OutcomeSpec, (String, String)) {
    let before = outcome
        .requirements
        .iter()
        .map(|requirement| requirement.id.clone())
        .collect::<Vec<_>>();
    match request.kind {
        vak_intent::InterventionKind::AddRequirement => {
            outcome.requirements.push(vak_intent::OutcomeRequirement {
                id: format!("intervention-{}", request.request_id),
                kind: vak_intent::RequirementKind::Deliverable,
                description: request.text.clone(),
                origin: vak_intent::RequirementOrigin::Explicit,
                importance: vak_intent::RequirementImportance::Must,
                target: None,
            })
        }
        vak_intent::InterventionKind::RemoveRequirement => {
            if let Some(target) = request.text.split_whitespace().last() {
                outcome
                    .requirements
                    .retain(|requirement| requirement.id != target);
            }
        }
        vak_intent::InterventionKind::Replan | vak_intent::InterventionKind::Reprioritize => {
            outcome.assumptions.push(request.text.clone());
        }
        _ => {}
    }
    outcome.revision = outcome.revision.saturating_add(1);
    let after = outcome
        .requirements
        .iter()
        .map(|requirement| requirement.id.clone())
        .collect::<Vec<_>>();
    (outcome, (before.join(","), after.join(",")))
}

async fn plan_change(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PlanChangeBody>,
) -> axum::response::Response {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let kind = vak_intent::classify_intervention(&body.text);
    if !matches!(
        kind,
        vak_intent::InterventionKind::Replan
            | vak_intent::InterventionKind::Reprioritize
            | vak_intent::InterventionKind::AddRequirement
            | vak_intent::InterventionKind::RemoveRequirement
    ) {
        return (StatusCode::BAD_REQUEST, "request is not a plan change").into_response();
    }
    let revision = handle
        .session
        .lock()
        .ok()
        .and_then(|guard| {
            guard.as_ref().and_then(|session| {
                session.chain_to_root().iter().rev().find_map(|entry| {
                    if let vak_session::EntryPayload::Intent(record) = &entry.payload {
                        record.outcome.as_ref().map(|outcome| outcome.revision)
                    } else {
                        None
                    }
                })
            })
        })
        .or_else(|| {
            handle.intent.lock().ok().and_then(|guard| {
                guard
                    .as_ref()
                    .and_then(|record| record.outcome.as_ref().map(|outcome| outcome.revision))
            })
        })
        .unwrap_or(0);
    if body
        .target_revision
        .is_some_and(|target| target != revision)
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "decision": "rejected",
                "reason": "plan revision is stale",
                "revision": revision,
            })),
        )
            .into_response();
    }
    let evaluation = vak_intent::evaluate_intervention(vak_intent::InterventionRequest {
        request_id: format!("plan-change-{}", uuid::Uuid::now_v7()),
        kind,
        text: body.text.clone(),
        source: body.source.clone(),
        target_revision: Some(revision),
    });
    let mut admitted_revision = None;
    let mut requirement_diff = None;
    if evaluation.decision == vak_intent::InterventionDecision::Queued
        && let Ok(mut guard) = handle.session.lock()
        && let Some(session) = guard.as_mut()
        && let Some(record) = session.chain_to_root().iter().rev().find_map(|entry| {
            if let vak_session::EntryPayload::Intent(record) = &entry.payload {
                Some(record.as_ref())
            } else {
                None
            }
        })
        && let Some(outcome) = record.outcome.clone()
    {
        let (outcome, diff) = apply_plan_change(outcome, &evaluation.request);
        requirement_diff = Some(diff);
        let update = vak_session::types::IntentRecord {
            reading: record.reading.clone(),
            engagement: record.engagement.clone(),
            provenance: record.provenance.clone(),
            outcome: Some(outcome),
            model_visible: None,
            commitment_id: record.commitment_id.clone(),
        };
        if session.append_intent(update.clone()).is_ok() {
            *handle
                .intent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(update);
            admitted_revision = Some(revision + 1);
        }
    }
    if evaluation.decision == vak_intent::InterventionDecision::Queued
        && admitted_revision.is_none()
        && let Some(mut record) = handle.intent.lock().ok().and_then(|guard| guard.clone())
        && let Some(outcome) = record.outcome.take()
    {
        let (outcome, diff) = apply_plan_change(outcome, &evaluation.request);
        requirement_diff = Some(diff);
        let update = vak_session::types::IntentRecord {
            reading: record.reading,
            engagement: record.engagement,
            provenance: record.provenance,
            outcome: Some(outcome.clone()),
            model_visible: None,
            commitment_id: record.commitment_id,
        };
        *handle
            .intent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(update.clone());
        handle.steering.push_outcome_update(update);
        admitted_revision = Some(outcome.revision);
    }
    record_activity_or_buffer(
        &handle,
        vak_session::ActivityRecord {
            activity_id: evaluation.request.request_id.clone(),
            turn: None,
            kind: vak_session::ActivityKind::Diagnostic,
            status: if evaluation.decision == vak_intent::InterventionDecision::RequiresHuman {
                vak_session::ActivityStatus::Denied
            } else {
                vak_session::ActivityStatus::Pending
            },
            label: "Plan change evaluated".into(),
            detail: Some(evaluation.reason.clone()),
            data: std::collections::BTreeMap::from([
                ("decision".into(), evaluation.decision.as_str().into()),
                ("source".into(), body.source),
                (
                    "revision".into(),
                    admitted_revision.unwrap_or(revision).to_string(),
                ),
                (
                    "requirements_before".into(),
                    requirement_diff
                        .as_ref()
                        .map(|diff| diff.0.clone())
                        .unwrap_or_else(String::new),
                ),
                (
                    "requirements_after".into(),
                    requirement_diff
                        .as_ref()
                        .map(|diff| diff.1.clone())
                        .unwrap_or_else(String::new),
                ),
                ("request".into(), body.text),
            ]),
        },
    );
    let status = if evaluation.decision == vak_intent::InterventionDecision::RequiresHuman {
        StatusCode::CONFLICT
    } else {
        handle
            .steering
            .push_steering(evaluation.request.text.clone());
        StatusCode::ACCEPTED
    };
    (
        status,
        Json(serde_json::json!({
            "request_id": evaluation.request.request_id,
            "decision": evaluation.decision.as_str(),
            "revision": admitted_revision.unwrap_or(revision),
            "reason": evaluation.reason,
        })),
    )
        .into_response()
}

pub(crate) fn record_control_activity(handle: &SessionHandle, label: &str, control: &str) {
    record_activity_or_buffer(
        handle,
        vak_session::ActivityRecord {
            activity_id: format!("control-{}", uuid::Uuid::now_v7()),
            turn: None,
            kind: vak_session::ActivityKind::Diagnostic,
            status: vak_session::ActivityStatus::Succeeded,
            label: label.into(),
            detail: Some("operator control-plane request".into()),
            data: std::collections::BTreeMap::from([
                ("control".into(), control.into()),
                ("source".into(), "human".into()),
            ]),
        },
    );
}

fn record_activity_or_buffer(handle: &SessionHandle, activity: vak_session::ActivityRecord) {
    if let Ok(mut session) = handle.session.lock()
        && let Some(session) = session.as_mut()
    {
        let _ = session.append_activity(activity);
    } else if let Ok(mut activities) = handle.activity_buffer.lock() {
        activities.push(activity);
    }
}

fn record_routing_data(
    data: &mut std::collections::BTreeMap<String, String>,
    routing: &RoutingEnvelope,
) {
    if let Some(value) = routing.message_id.as_deref() {
        data.insert("message_id".into(), value.into());
    }
    if let Some(value) = routing.conversation_id.as_deref() {
        data.insert("conversation_id".into(), value.into());
    }
    if let Some(value) = routing.target_work_id.as_deref() {
        data.insert("target_work_id".into(), value.into());
    }
    if let Some(value) = routing.target_result_id.as_deref() {
        data.insert("target_result_id".into(), value.into());
    }
    if let Some(value) = routing.relation.as_deref() {
        data.insert("relation".into(), value.into());
    }
    if let Some(value) = routing.outcome_revision {
        data.insert("outcome_revision".into(), value.to_string());
    }
    if let Some(value) = routing.provenance.as_deref() {
        data.insert("routing_provenance".into(), value.into());
    }
}

fn routing_revision_is_current(handle: &SessionHandle, expected: u64) -> bool {
    handle.intent.lock().ok().is_some_and(|record| {
        record
            .as_ref()
            .and_then(|intent| intent.outcome.as_ref())
            .is_some_and(|outcome| outcome.revision == expected)
    })
}

fn routing_summary(routing: Option<&RoutingEnvelope>) -> String {
    let Some(routing) = routing else {
        return String::new();
    };
    let mut data = std::collections::BTreeMap::new();
    record_routing_data(&mut data, routing);
    serde_json::to_string(&data).unwrap_or_default()
}

fn deny_pending_approvals(handle: &SessionHandle) {
    let requests: Vec<ApprovalRequest> = handle
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .drain()
        .map(|(_, request)| request)
        .collect();
    for request in requests {
        request.respond(false);
    }
}

// ---- Worker control plane -------------------------------------------------
//
// Children already stream lifecycle/tool events into the parent session's
// SSE channel; these endpoints add the missing half: listing, steering, and
// stopping from a remote surface. Scope-checked against the parent so one
// session can never touch another's child.

async fn list_workers(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let children = state.core.workers().active_for(&id);
    Json(serde_json::json!({ "workers": children }))
}

#[derive(serde::Deserialize)]
struct WorkerSteerBody {
    text: String,
}

async fn steer_worker(
    State(state): State<AppState>,
    Path((id, child)): Path<(String, String)>,
    Json(body): Json<WorkerSteerBody>,
) -> StatusCode {
    if state.core.workers().parent_of(&child).as_deref() != Some(id.as_str()) {
        return StatusCode::NOT_FOUND;
    }
    if state.core.workers().steer(&child, &body.text) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::CONFLICT
    }
}

async fn stop_worker(
    State(state): State<AppState>,
    Path((id, child)): Path<(String, String)>,
) -> StatusCode {
    if state.core.workers().parent_of(&child).as_deref() != Some(id.as_str()) {
        return StatusCode::NOT_FOUND;
    }
    if state.core.workers().stop(&child) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::CONFLICT
    }
}

#[derive(serde::Deserialize)]
struct ApprovalBody {
    approve: bool,
    /// "…and don't ask again for calls like this one". Derives the narrowest
    /// rule that covers this call and persists it to
    /// `.vak/permissions.local.toml`. Only meaningful alongside
    /// `approve: true` — remembering a refusal would be a deny rule, which
    /// is a different and much heavier decision than answering one gate.
    #[serde(default)]
    remember: bool,
}

#[derive(serde::Deserialize)]
struct OutcomeReviewBody {
    verdict: String,
    #[serde(default)]
    turn: Option<usize>,
    #[serde(default)]
    note: Option<String>,
}

/// Record an operator's review of a computed outcome. This is deliberately
/// separate from approval: reviewing a result never authorizes a tool call.
async fn record_outcome_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<OutcomeReviewBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !matches!(
        body.verdict.as_str(),
        "accepted" | "needs_work" | "rejected"
    ) {
        return (StatusCode::BAD_REQUEST, "invalid outcome review verdict").into_response();
    }
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(mut guard) = handle.session.lock() else {
        return StatusCode::CONFLICT.into_response();
    };
    let Some(session) = guard.as_mut() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let reviewed_turn = body.turn.or_else(|| {
        session
            .chain_to_root()
            .iter()
            .filter_map(|entry| match &entry.payload {
                vak_session::EntryPayload::Activity(activity)
                    if activity.label == "Outcome evaluation" =>
                {
                    activity.turn
                }
                _ => None,
            })
            .next_back()
    });
    if body.turn.is_some() && reviewed_turn.is_none() {
        return (StatusCode::NOT_FOUND, "outcome evaluation turn not found").into_response();
    }
    let activity = vak_session::ActivityRecord {
        activity_id: format!("outcome-review-{}", uuid::Uuid::now_v7()),
        turn: reviewed_turn,
        kind: vak_session::ActivityKind::Diagnostic,
        status: vak_session::ActivityStatus::Succeeded,
        label: "Outcome review".into(),
        detail: body.note,
        data: std::collections::BTreeMap::from([("verdict".into(), body.verdict)]),
    };
    if let Err(error) = session.append_activity(activity) {
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }
    Json(serde_json::json!({ "recorded": true })).into_response()
}

/// `POST /sessions/{id}/approvals/{req_id}` — answer one gate.
///
/// `remember` is the mechanism `docs/design/08-permissions.md` has described
/// since the engine shipped and nothing ever called: `Core::learn_allow_rule`
/// existed, was tested, wrote a validated file, and had no caller on any
/// surface, because the interactive approver its comment referred to was
/// never built. This is that caller.
///
/// Remembering never blocks the answer. The gate is resolved first; a
/// failure to derive or persist a rule is reported alongside a successful
/// approval, because the run is already waiting and a bookkeeping problem
/// must not become a denial.
async fn answer_approval(
    State(state): State<AppState>,
    Path((id, req_id)): Path<(String, String)>,
    Json(body): Json<ApprovalBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Look the request up in THIS session's pending map only: approvals
    // are never resolvable across sessions.
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(req) = handle
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&req_id)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let tool = req.tool.clone();
    let args_json = req.args_json.clone();
    req.respond(body.approve);

    let mut learned: Option<String> = None;
    let mut learn_error: Option<String> = None;
    if body.remember && body.approve {
        match serde_json::from_str::<serde_json::Value>(&args_json) {
            // The gate's own core, not the server's: a learned rule belongs
            // to the workspace whose call raised it.
            Ok(args) => match handle.core.learn_from_call(&tool, &args) {
                Ok(spec) => {
                    vak_core::security_events::record(
                        &state.core.sessions_home(),
                        vak_core::security_events::EventKind::ConfigChange,
                        "permission_rule_learned",
                        &format!("session={id} rule={spec}"),
                        None,
                    );
                    learned = Some(spec);
                }
                Err(error) => learn_error = Some(error.to_string()),
            },
            Err(error) => learn_error = Some(format!("unreadable tool arguments: {error}")),
        }
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "approved": body.approve,
            "learned_rule": learned,
            "learn_error": learn_error,
        })),
    )
        .into_response()
}

/// Dispatch forensics (docs/design/42-managed-work-contracts.md): the session's work
/// receipts, newest last.
async fn session_receipts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<vak_llm::WorkReceipt>>, StatusCode> {
    let Some(handle) = state.get(&id) else {
        return Err(StatusCode::NOT_FOUND);
    };
    let Ok(session) = handle.session.lock() else {
        return Err(StatusCode::NOT_FOUND);
    };
    match session.as_ref() {
        Some(log) => Ok(Json(log.receipts().into_iter().cloned().collect())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn session_work(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<vak_session::work::WorkProjection>>, StatusCode> {
    if let Some(handle) = state.get(&id) {
        let guard = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(session) = guard.as_ref() else {
            return Err(StatusCode::NOT_FOUND);
        };
        return session
            .work_projection()
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let Some(session) = open_historical_session(&state, &id) else {
        return Err(StatusCode::NOT_FOUND);
    };
    session
        .work_projection()
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[derive(serde::Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum WorkCommand {
    Transition {
        item_id: String,
        to: vak_session::types::WorkItemStatus,
        #[serde(default)]
        reason: String,
    },
    Cancel {
        #[serde(default)]
        reason: String,
    },
    Resume {
        #[serde(default)]
        reason: String,
    },
    Retry {
        item_id: String,
        #[serde(default)]
        reason: String,
    },
    ResolveAssumption {
        assumption_id: String,
        resolution: String,
    },
    AttachEvidence {
        item_id: String,
        evidence: vak_session::types::EvidenceRef,
    },
    Assign {
        item_id: String,
        owner: vak_session::types::WorkOwner,
        child_session_id: Option<String>,
    },
    Revise {
        contract: vak_session::types::WorkContract,
        #[serde(default)]
        reason: String,
    },
}

#[derive(serde::Deserialize)]
struct WorkReason {
    #[serde(default)]
    reason: String,
}

#[derive(serde::Deserialize)]
struct WorkReassign {
    owner: vak_session::types::WorkOwner,
    #[serde(default)]
    child_session_id: Option<String>,
}

async fn session_work_confirm(
    state: State<AppState>,
    path: Path<String>,
    body: Option<Json<WorkReason>>,
) -> axum::response::Response {
    session_work_command(
        state,
        path,
        Json(WorkCommand::Resume {
            reason: body
                .map(|body| body.0.reason)
                .unwrap_or_else(|| "confirmed by operator".into()),
        }),
    )
    .await
}

async fn session_work_revise(
    state: State<AppState>,
    path: Path<String>,
    Json(body): Json<WorkCommand>,
) -> axum::response::Response {
    session_work_command(state, path, Json(body)).await
}

async fn session_work_retry(
    state: State<AppState>,
    Path((id, item_id)): Path<(String, String)>,
    body: Option<Json<WorkReason>>,
) -> axum::response::Response {
    session_work_command(
        state,
        Path(id),
        Json(WorkCommand::Retry {
            item_id,
            reason: body
                .map(|body| body.0.reason)
                .unwrap_or_else(|| "retry requested by operator".into()),
        }),
    )
    .await
}

async fn session_work_cancel_item(
    state: State<AppState>,
    Path((id, item_id)): Path<(String, String)>,
    body: Option<Json<WorkReason>>,
) -> axum::response::Response {
    session_work_command(
        state,
        Path(id),
        Json(WorkCommand::Transition {
            item_id,
            to: vak_session::types::WorkItemStatus::Cancelled,
            reason: body
                .map(|body| body.0.reason)
                .unwrap_or_else(|| "cancelled by operator".into()),
        }),
    )
    .await
}

async fn session_work_reassign(
    state: State<AppState>,
    Path((id, item_id)): Path<(String, String)>,
    Json(body): Json<WorkReassign>,
) -> axum::response::Response {
    session_work_command(
        state,
        Path(id),
        Json(WorkCommand::Assign {
            item_id,
            owner: body.owner,
            child_session_id: body.child_session_id,
        }),
    )
    .await
}

/// Apply an operator/user work action to the same append-only ledger used by
/// the agent. The current projection supplies both the expected state and
/// revision, so stale clients receive a conflict instead of silently
/// overwriting a newer action.
async fn session_work_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(command): Json<WorkCommand>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut guard = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(session) = guard.as_mut() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "session is still running" })),
        )
            .into_response();
    };
    let Ok(Some(projection)) = session.work_projection() else {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "session has no valid work contract" })),
        )
            .into_response();
    };
    let contract_id = projection.contract.contract_id.clone();
    let revision = projection.contract.revision;
    let event_kind = match command {
        WorkCommand::Transition {
            item_id,
            to,
            reason,
        } => {
            let Some(item) = projection.items.get(&item_id) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("unknown work item '{item_id}'") })),
                )
                    .into_response();
            };
            if matches!(to, vak_session::types::WorkItemStatus::Succeeded) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "succeeded requires independent verification" })),
                )
                    .into_response();
            }
            vak_session::types::WorkEventKind::ItemStatusChanged {
                item_id,
                from: item.status.clone(),
                to,
                attempt: item.attempt,
                reason,
            }
        }
        WorkCommand::Cancel { reason } => {
            vak_session::types::WorkEventKind::ContractStatusChanged {
                from: projection.status,
                to: vak_session::types::WorkContractStatus::Cancelled,
                reason,
            }
        }
        WorkCommand::Resume { reason } => {
            if projection.status == vak_session::types::WorkContractStatus::AwaitingInput
                && projection.contract.assumptions.iter().any(|assumption| {
                    assumption.requires_confirmation && assumption.resolution.is_none()
                })
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "required assumptions must be resolved before resuming"
                    })),
                )
                    .into_response();
            }
            vak_session::types::WorkEventKind::ContractStatusChanged {
                from: projection.status,
                to: vak_session::types::WorkContractStatus::Active,
                reason,
            }
        }
        WorkCommand::Retry { item_id, reason } => {
            let Some(item) = projection.items.get(&item_id) else {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("unknown work item '{item_id}'") })),
                )
                    .into_response();
            };
            vak_session::types::WorkEventKind::ItemStatusChanged {
                item_id,
                from: item.status.clone(),
                to: vak_session::types::WorkItemStatus::Ready,
                attempt: item.attempt.saturating_add(1),
                reason,
            }
        }
        WorkCommand::ResolveAssumption {
            assumption_id,
            resolution,
        } => {
            if resolution.trim().is_empty()
                || !projection
                    .contract
                    .assumptions
                    .iter()
                    .any(|assumption| assumption.assumption_id == assumption_id)
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "unknown assumption or empty resolution" })),
                )
                    .into_response();
            }
            vak_session::types::WorkEventKind::AssumptionResolved {
                assumption_id,
                resolution,
            }
        }
        WorkCommand::AttachEvidence { item_id, evidence } => {
            if !projection.items.contains_key(&item_id) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("unknown work item '{item_id}'") })),
                )
                    .into_response();
            }
            if matches!(
                &evidence,
                vak_session::types::EvidenceRef::FlowNode { .. }
                    | vak_session::types::EvidenceRef::ChildSession { .. }
                    | vak_session::types::EvidenceRef::ExternalOperation { .. }
            ) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "runtime-owned evidence must be produced by its integration"
                    })),
                )
                    .into_response();
            }
            vak_session::types::WorkEventKind::EvidenceAttached { item_id, evidence }
        }
        WorkCommand::Assign {
            item_id,
            owner,
            child_session_id,
        } => {
            if !projection.items.contains_key(&item_id) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("unknown work item '{item_id}'") })),
                )
                    .into_response();
            }
            vak_session::types::WorkEventKind::ItemAssigned {
                item_id,
                owner,
                child_session_id,
            }
        }
        WorkCommand::Revise { contract, reason } => {
            if contract.contract_id != projection.contract.contract_id
                || contract.revision != revision.saturating_add(1)
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "revision must target the active contract and be exactly one greater" })),
                )
                    .into_response();
            }
            if revision >= state.core.effective_work().max_revisions {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": "maximum work revisions reached" })),
                )
                    .into_response();
            }
            if let Err(error) = vak_session::validate_contract_for_admission(&contract) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": error.to_string() })),
                )
                    .into_response();
            }
            if let Err(error) = vak_agent::validate_work_paths(&contract) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": error })),
                )
                    .into_response();
            }
            vak_session::types::WorkEventKind::ContractRevised {
                previous_revision: revision,
                contract,
                reason,
            }
        }
    };
    let event = vak_session::types::WorkEvent {
        contract_id,
        revision: if matches!(
            &event_kind,
            vak_session::types::WorkEventKind::ContractRevised { .. }
        ) {
            revision.saturating_add(1)
        } else {
            revision
        },
        kind: event_kind,
    };
    match session.append_work(event) {
        Ok(_) => {
            if let Ok(Some(updated)) = session.work_projection()
                && updated.status == vak_session::types::WorkContractStatus::AwaitingInput
                && updated
                    .contract
                    .assumptions
                    .iter()
                    .filter(|assumption| assumption.requires_confirmation)
                    .all(|assumption| assumption.resolution.is_some())
            {
                let _ = session.append_work(vak_session::types::WorkEvent {
                    contract_id: updated.contract.contract_id.clone(),
                    revision: updated.contract.revision,
                    kind: vak_session::types::WorkEventKind::ContractStatusChanged {
                        from: vak_session::types::WorkContractStatus::AwaitingInput,
                        to: vak_session::types::WorkContractStatus::Active,
                        reason: "all required assumptions resolved".into(),
                    },
                });
            }
            match session.work_projection() {
                Ok(Some(updated)) => Json(updated).into_response(),
                _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

/// Flow names discovered under `<sessions_home>/flow-runs` (docs/design/42-managed-work-contracts.mdG).
async fn flows_list(State(state): State<AppState>) -> Json<Vec<String>> {
    let root = state.core.sessions_home().join("flow-runs");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    Json(out)
}

/// Run ledger filenames for one flow, oldest first.
async fn flow_runs_list(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let dir = state.core.sessions_home().join("flow-runs").join(&name);
    let mut out = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for e in entries.flatten() {
                if e.path().extension().map(|x| x == "json").unwrap_or(false) {
                    out.push(e.file_name().to_string_lossy().into_owned());
                }
            }
            out.sort();
            Ok(Json(out))
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

/// Typed run-graph snapshot (delta+snapshot invariant 4): projection of a
/// single run ledger — statuses, layers, counts. No rendering opinions.
async fn flow_run_graph(
    State(state): State<AppState>,
    Path((name, run)): Path<(String, String)>,
) -> Result<Json<vak_flow::graph::RunGraph>, StatusCode> {
    // `run` is either the ledger filename or its stem.
    let run_file = if run.ends_with(".json") {
        run.clone()
    } else {
        format!("{run}.json")
    };
    let path = state
        .core
        .sessions_home()
        .join("flow-runs")
        .join(&name)
        .join(&run_file);
    match std::fs::read_to_string(&path) {
        Ok(body) => {
            let state: vak_flow::FlowState =
                serde_json::from_str(&body).map_err(|_| StatusCode::NOT_FOUND)?;
            Ok(Json(vak_flow::graph::graph_snapshot(&state)))
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

/// The `Last-Event-ID` a reconnecting client sent, if any.
///
/// Browsers resend it automatically on their own reconnect; the client also
/// passes it explicitly when it reopens a stream it tore down itself.
fn resume_from(headers: &axum::http::HeaderMap, uri: &axum::http::Uri) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .or_else(|| {
            uri.query()
                .and_then(|query| {
                    query.split('&').find_map(|part| {
                        let (key, value) = part.split_once('=')?;
                        (key == "last_event_id").then_some(value)
                    })
                })
                .and_then(|value| value.parse::<u64>().ok())
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod resume_cursor_tests {
    use super::resume_from;

    #[test]
    fn accepts_native_header_and_manual_reconnect_query_cursor() {
        let mut headers = axum::http::HeaderMap::new();
        let uri = "/sessions/s/events?last_event_id=17".parse().unwrap();
        assert_eq!(resume_from(&headers, &uri), Some(17));

        headers.insert("last-event-id", "23".parse().unwrap());
        assert_eq!(resume_from(&headers, &uri), Some(23));
    }
}

/// One SSE frame carrying its sequence number, so the client's next
/// reconnect can name where it got to.
fn seq_frame(framed: &events::SeqEvent) -> Event {
    let data = match serde_json::to_string(&framed.event) {
        Ok(data) => data,
        Err(error) => serde_json::json!({
            "error": "event serialization failed",
            "detail": error.to_string(),
        })
        .to_string(),
    };
    Event::default().id(framed.seq.to_string()).data(data)
}

async fn events_sse(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let resume = resume_from(&headers, &uri);
    let stream: std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
    > = match state.get(&id) {
        Some(h) => {
            // Subscribe BEFORE reading the replay ring, so an event
            // published between the two is received live rather than
            // falling into the gap between them. Duplicates are filtered
            // below by sequence number; a gap could not be recovered.
            let rx = h.events_tx.subscribe();

            // What the client missed while it was away. `None` means the
            // ring no longer reaches back that far, and the client is told
            // to rebuild from the durable transcript rather than being
            // handed a stream with a hole in it that it cannot see.
            let (replay, resync) = match resume {
                Some(seq) => match h.events_tx.replay_after(seq) {
                    Some(missed) => (missed, false),
                    None => (Vec::new(), true),
                },
                None => (Vec::new(), false),
            };
            let highest_replayed = replay.last().map(|e| e.seq).unwrap_or(0);

            h.subscribed.notify_one();
            h.events_tx.send(AgentEvent::StreamOpened);

            let resync_frame = resync.then(|| {
                Ok(Event::default()
                    .event("resync")
                    .data("{\"reason\":\"events older than the replay window\"}"))
            });
            let replayed = replay.into_iter().map(|framed| Ok(seq_frame(&framed)));
            let live = BroadcastStream::new(rx).filter_map(move |ev| match ev {
                // Anything at or below what the replay already delivered is
                // a duplicate of it, not new work.
                Ok(framed) if framed.seq <= highest_replayed => None,
                Ok(framed) => Some(Ok(seq_frame(&framed))),
                Err(_) => Some(Ok(Event::default()
                    .event("resync")
                    .data("{\"reason\":\"live event consumer lagged\"}"))),
            });
            Box::pin(
                tokio_stream::iter(resync_frame)
                    .chain(tokio_stream::iter(replayed))
                    .chain(live),
            )
        }
        None => Box::pin(tokio_stream::once(Ok(
            Event::default().data("{\"error\":\"unknown session\"}")
        ))),
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn presentation_snapshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Some(handle) = state.get(&id) {
        let guard = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(session) = guard.as_ref() {
            let planner = delivery::merged_presentation_planner(&handle.core);
            let mut timeline = match presentation_store(&state).load() {
                Ok(library) => crate::projection::snapshot_with_planner_and_library(
                    &id, session, &planner, &library,
                ),
                Err(_) => crate::projection::snapshot_with_planner(&id, session, &planner),
            };
            crate::projection::append_sandbox_artifacts(
                &mut timeline,
                &handle.core.sessions_home(),
                &id,
            );
            return Json(timeline).into_response();
        }
        let mut timeline = handle
            .presentation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        timeline.diagnostics.push("run in progress".into());
        return Json(timeline).into_response();
    }
    match open_historical_session(&state, &id) {
        Some(session) => {
            let mut timeline = crate::projection::snapshot(&id, &session);
            crate::projection::append_sandbox_artifacts(
                &mut timeline,
                &state.core.sessions_home(),
                &id,
            );
            Json(timeline).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response(),
    }
}

/// Fetch one immutable result from the typed presentation projection. This
/// keeps background notifications addressable without exposing transcript or
/// transport internals to the client.
async fn session_result(
    State(state): State<AppState>,
    Path((id, result_id)): Path<(String, String)>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let timeline = if let Some(handle) = state.get(&id) {
        let guard = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(session) = guard.as_ref() {
            let planner = delivery::merged_presentation_planner(&handle.core);
            match presentation_store(&state).load() {
                Ok(library) => crate::projection::snapshot_with_planner_and_library(
                    &id, session, &planner, &library,
                ),
                Err(_) => crate::projection::snapshot_with_planner(&id, session, &planner),
            }
        } else {
            handle
                .presentation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        }
    } else if let Some(session) = open_historical_session(&state, &id) {
        crate::projection::snapshot(&id, &session)
    } else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response();
    };
    match timeline.items.into_iter().find(|item| {
        item.id == result_id
            || item
                .outcome
                .as_ref()
                .is_some_and(|outcome| outcome.result_id == result_id)
    }) {
        Some(item) => Json(item).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown result" })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct PresentationFeedbackBody {
    choice: String,
    #[serde(default)]
    feedback: Option<String>,
    #[serde(default)]
    chain_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PresentationSelectionBody {
    #[serde(default)]
    spec_id: String,
    revision: u64,
    #[serde(default)]
    semantic_type: Option<String>,
    #[serde(default = "default_presentation_selection")]
    lifetime: String,
    #[serde(default)]
    scope: Option<vak_presentation::LibraryScope>,
    #[serde(default)]
    owner: Option<String>,
}

fn default_presentation_selection() -> String {
    "use_once".into()
}

async fn select_presentation_for_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PresentationSelectionBody>,
) -> axum::response::Response {
    if body.spec_id.len() > 256
        || body
            .semantic_type
            .as_deref()
            .is_some_and(|value| value.len() > 256)
        || !matches!(body.lifetime.as_str(), "use_once" | "remember")
        || (body.lifetime == "remember"
            && (body.scope.is_none()
                || body.owner.as_deref().unwrap_or_default().trim().is_empty()))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let store = presentation_store(&state);
    let library = match store.load() {
        Ok(library) => library,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response();
        }
    };
    let (spec_id, revision) = if body.spec_id.trim().is_empty() {
        let Some(semantic_type) = body
            .semantic_type
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let owner = state.core.cwd().to_string_lossy().into_owned();
        let Some(definition) = library.select_preferred(semantic_type, "builtin", &owner) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        (definition.spec.id.clone(), definition.spec.revision)
    } else {
        (body.spec_id.clone(), body.revision)
    };
    let Some(definition) = library.get(&spec_id, revision) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if body.lifetime == "remember" {
        let Some(scope) = body.scope else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let owner = body.owner.as_deref().unwrap_or_default();
        if definition.origin.scope != scope || definition.origin.owner != owner {
            return StatusCode::FORBIDDEN.into_response();
        }
        let result = store.load().and_then(|mut current| {
            current
                .activate(&spec_id, revision, scope, owner)
                .map_err(|error| {
                    vak_store::presentation::PresentationStoreError::Invalid(error.to_string())
                })?;
            store.save(&current)
        });
        if result.is_err() {
            return StatusCode::BAD_REQUEST.into_response();
        }
    }
    let mut data = std::collections::BTreeMap::new();
    data.insert("spec_id".into(), spec_id);
    data.insert("revision".into(), revision.to_string());
    data.insert("lifetime".into(), body.lifetime);
    let activity = vak_session::ActivityRecord {
        activity_id: format!("presentation-select-{}", uuid::Uuid::now_v7()),
        turn: None,
        kind: vak_session::ActivityKind::PresentationSelection,
        status: vak_session::ActivityStatus::Succeeded,
        label: "Presentation selected".into(),
        detail: None,
        data,
    };
    handle
        .activity_buffer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(activity);
    Json(serde_json::json!({ "selected": true })).into_response()
}

async fn presentation_feedback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PresentationFeedbackBody>,
) -> StatusCode {
    if body.choice.trim().is_empty() || body.choice.len() > 128 {
        return StatusCode::BAD_REQUEST;
    }
    if body
        .feedback
        .as_deref()
        .is_some_and(|text| text.len() > 32 * 1024)
    {
        return StatusCode::BAD_REQUEST;
    }
    if body
        .chain_id
        .as_deref()
        .is_some_and(|chain| chain.trim().is_empty() || chain.len() > 256)
    {
        return StatusCode::BAD_REQUEST;
    }
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    let feedback_denied = matches!(
        body.choice.trim().to_ascii_lowercase().as_str(),
        "keep_original" | "reject" | "dismiss"
    );
    let mut data = std::collections::BTreeMap::new();
    data.insert("choice".into(), body.choice);
    if let Some(chain_id) = body.chain_id {
        data.insert("chain_id".into(), chain_id);
    }
    if let Some(feedback) = body.feedback {
        data.insert("feedback".into(), feedback);
    }
    let activity = vak_session::ActivityRecord {
        activity_id: uuid::Uuid::now_v7().to_string(),
        turn: None,
        kind: vak_session::ActivityKind::PresentationFeedback,
        status: if feedback_denied {
            vak_session::ActivityStatus::Denied
        } else {
            vak_session::ActivityStatus::Succeeded
        },
        label: "Presentation feedback".into(),
        detail: None,
        data,
    };
    handle
        .activity_buffer
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(activity);
    StatusCode::ACCEPTED
}

async fn presentation_events_sse(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    // EventSource reconnects carry the last presentation sequence through
    // the same header/query contract as the primary session stream. The
    // presentation projection is snapshot-based here: the initial snapshot
    // is authoritative, and its id establishes the new durable cursor. Do
    // not manufacture delta replay from an unknown historical baseline.
    let _resume = resume_from(&headers, &uri);
    let stream: std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
    > = match state.get(&id) {
        Some(handle) => {
            let rx = handle.events_tx.subscribe();
            let initial = {
                let guard = handle
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard
                    .as_ref()
                    .map(|session| {
                        let mut timeline = crate::projection::snapshot(&id, session);
                        crate::projection::append_sandbox_artifacts(
                            &mut timeline,
                            &handle.core.sessions_home(),
                            &id,
                        );
                        timeline
                    })
                    .unwrap_or_else(|| {
                        handle
                            .presentation
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone()
                    })
            };
            let mut timeline = initial;
            let mut last_sequence = timeline
                .cursor
                .as_deref()
                .and_then(|cursor| cursor.strip_prefix("live:"))
                .and_then(|seq| seq.parse::<u64>().ok())
                .unwrap_or(0);
            let initial = vak_delivery::OutputStreamFrame {
                sequence: Some(last_sequence),
                delta: None,
                snapshot: timeline.clone(),
            };
            let initial = tokio_stream::once(Ok(Event::default()
                .id(last_sequence.to_string())
                .data(serde_json::to_string(&initial).unwrap_or_else(|error| {
                    serde_json::json!({
                        "error": "presentation serialization failed",
                        "detail": error.to_string(),
                    })
                    .to_string()
                }))));
            handle.subscribed.notify_one();
            let live = BroadcastStream::new(rx).filter_map(move |event| match event {
                Ok(framed) => {
                    if framed.seq <= last_sequence {
                        return None;
                    }
                    last_sequence = framed.seq;
                    crate::projection::project_frame(&mut timeline, framed).map(|frame| {
                        Ok(Event::default().id(last_sequence.to_string()).data(
                            serde_json::to_string(&frame).unwrap_or_else(|error| {
                                serde_json::json!({
                                    "error": "presentation serialization failed",
                                    "detail": error.to_string(),
                                })
                                .to_string()
                            }),
                        ))
                    })
                }
                Err(_) => {
                    if let Some(events) = handle.events_tx.replay_after(last_sequence) {
                        for framed in events {
                            last_sequence = framed.seq;
                            crate::projection::project_frame(&mut timeline, framed);
                        }
                    } else {
                        let guard = handle
                            .session
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        timeline = guard
                            .as_ref()
                            .map(|session| crate::projection::snapshot(&id, session))
                            .unwrap_or_else(|| {
                                handle
                                    .presentation
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .clone()
                            });
                        last_sequence = timeline
                            .cursor
                            .as_deref()
                            .and_then(|cursor| cursor.strip_prefix("live:"))
                            .and_then(|seq| seq.parse().ok())
                            .unwrap_or(last_sequence);
                        timeline
                            .diagnostics
                            .push("Presentation stream resynchronized after a gap.".into());
                    }
                    let frame = vak_delivery::OutputStreamFrame {
                        sequence: Some(last_sequence),
                        delta: None,
                        snapshot: timeline.clone(),
                    };
                    Some(Ok(Event::default()
                        .id(last_sequence.to_string())
                        .data(serde_json::to_string(&frame).unwrap_or_default())))
                }
            });
            Box::pin(initial.chain(live))
        }
        None => match open_historical_session(&state, &id) {
            Some(session) => {
                let event = vak_delivery::OutputStreamFrame {
                    sequence: None,
                    delta: None,
                    snapshot: crate::projection::snapshot(&id, &session),
                };
                Box::pin(tokio_stream::once(Ok(
                    Event::default().data(serde_json::to_string(&event).unwrap_or_default())
                )))
            }
            None => Box::pin(tokio_stream::once(Ok(
                Event::default().data("{\"error\":\"unknown session\"}")
            ))),
        },
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn transcript(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // `count` is the number of model-visible messages in the derived
    // projection (`derive_messages().len()`), not a raw ledger-entry count.
    // Since v3.0.17 the projection may legitimately exceed the exchanged
    // messages: the multi-turn continuity layer injects a synthetic
    // `<conversation_thread>` (and context compaction may add entries), so a
    // two-message exchange can project as five. Both numbers are
    // reconstructable from the append-only ledger; `count` describes exactly
    // what the model consumed.
    if let Some(handle) = state.get(&id) {
        let guard = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(s) = guard.as_ref() else {
            return Json(serde_json::json!({ "error": "run in progress" })).into_response();
        };
        let msgs = s.derive_messages();
        let contract = s.header().map(|header| header.contract.clone());
        return Json(serde_json::json!({
            "count": msgs.len(),
            "usage": s.total_usage(),
            "contract": contract,
            // Per-turn routing: every turn resolves provider/model from the
            // live effective_route(), so the header's initial contract snapshot
            // is no longer a mismatch indicator. Field kept for API compat.
            "configuration_mismatch": false,
            "messages": msgs,
        }))
        .into_response();
    }
    match open_historical_session(&state, &id) {
        Some(s) => {
            let msgs = s.derive_messages();
            let contract = s.header().map(|header| header.contract.clone());
            Json(serde_json::json!({
                "count": msgs.len(),
                "usage": s.total_usage(),
                "contract": contract,
                // Per-turn routing: every turn resolves provider/model from the
                // live effective_route(), so the header's initial contract snapshot
                // is no longer a mismatch indicator. Field kept for API compat.
                "configuration_mismatch": false,
                "messages": msgs,
            }))
            .into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response(),
    }
}

/// Markdown export over the same projection the JSON transcript serves.
/// One shared renderer with the TUI export — byte-identical output for the
/// same session (docs/design/29-personal-os.md P4).
async fn transcript_markdown(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let format_html = query.get("format").map(|s| s.as_str()) == Some("html");

    let md_opt = if let Some(handle) = state.get(&id) {
        let guard = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(s) = guard.as_ref() else {
            return Json(serde_json::json!({ "error": "run in progress" })).into_response();
        };
        Some(vak_core::transcript_md::render_markdown(
            &s.derive_messages(),
        ))
    } else {
        open_historical_session(&state, &id)
            .map(|s| vak_core::transcript_md::render_markdown(&s.derive_messages()))
    };

    match md_opt {
        Some(md) => {
            if format_html {
                html_response(vak_presentation::transcode_to_html(
                    &format!("Session {id}"),
                    &md,
                ))
            } else {
                markdown_response(md)
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response(),
    }
}

fn html_response(html: String) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        [(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        html,
    )
        .into_response()
}

fn markdown_response(md: String) -> axum::response::Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("text/markdown; charset=utf-8"),
        )],
        md,
    )
        .into_response()
}

/// Find a session log on disk across current sessions_home and all agent directories.
fn find_session_on_disk(core: &Core, id: &str) -> Option<vak_session::SessionLog> {
    let home = core.sessions_home();
    let path = home
        .join("sessions")
        .join(vak_core::memory::hash_cwd(core.cwd()))
        .join(format!("{id}.jsonl"));
    if let Ok(s) = vak_session::SessionLog::open_read_only(path) {
        return Some(s);
    }
    if let Ok(entries) = std::fs::read_dir(home.join("sessions")) {
        for entry in entries.flatten() {
            let candidate = entry.path().join(format!("{id}.jsonl"));
            if let Ok(s) = vak_session::SessionLog::open_read_only(candidate) {
                return Some(s);
            }
        }
    }
    let shared = core.shared_data_home();
    if let Ok(entries) = std::fs::read_dir(shared.join("sessions")) {
        for entry in entries.flatten() {
            let candidate = entry.path().join(format!("{id}.jsonl"));
            if let Ok(s) = vak_session::SessionLog::open_read_only(candidate) {
                return Some(s);
            }
        }
    }
    if let Ok(agents) = std::fs::read_dir(shared.join("agents")) {
        for agent in agents.flatten() {
            if let Ok(projects) = std::fs::read_dir(agent.path().join("sessions")) {
                for project in projects.flatten() {
                    let candidate = project.path().join(format!("{id}.jsonl"));
                    if let Ok(s) = vak_session::SessionLog::open_read_only(candidate) {
                        return Some(s);
                    }
                }
            }
        }
    }
    None
}

/// Historical sessions live on disk but not in the in-memory handle map
/// (a fresh server process starts with an empty map). Open read-only for
/// export/inspection without mutating run bookkeeping.
fn open_historical_session(state: &AppState, id: &str) -> Option<vak_session::SessionLog> {
    find_session_on_disk(&state.core, id)
}

/// Read only the immutable first header entry without acquiring the session's
/// writer lock. Admin forensics must still work when another local process is
/// actively serving the channel.
pub(crate) fn read_historical_header(
    state: &AppState,
    id: &str,
    workspace: Option<&std::path::Path>,
) -> Option<vak_session::types::SessionHeader> {
    fn read(path: &std::path::Path) -> Option<vak_session::types::SessionHeader> {
        use std::io::BufRead;
        let file = std::fs::File::open(path).ok()?;
        for line in std::io::BufReader::new(file).lines().take(4) {
            let entry: vak_session::types::Entry = serde_json::from_str(&line.ok()?).ok()?;
            if let vak_session::types::EntryPayload::Header(header) = entry.payload {
                return Some(header);
            }
        }
        None
    }

    if let Some(workspace) = workspace {
        let path =
            vak_session::SessionPath::new_session_file(&state.core.sessions_home(), workspace, id);
        if let Some(header) = read(&path) {
            return Some(header);
        }
    }
    if let Ok(entries) = std::fs::read_dir(state.core.sessions_home().join("sessions")) {
        for project in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let path = project.path().join(format!("{id}.jsonl"));
            if let Some(header) = read(&path) {
                return Some(header);
            }
        }
    }
    let shared = state.core.shared_data_home();
    if let Ok(entries) = std::fs::read_dir(shared.join("sessions")) {
        for project in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let path = project.path().join(format!("{id}.jsonl"));
            if let Some(header) = read(&path) {
                return Some(header);
            }
        }
    }
    if let Ok(agents) = std::fs::read_dir(shared.join("agents")) {
        for agent in agents.flatten().filter(|entry| entry.path().is_dir()) {
            if let Ok(projects) = std::fs::read_dir(agent.path().join("sessions")) {
                for project in projects.flatten().filter(|entry| entry.path().is_dir()) {
                    let path = project.path().join(format!("{id}.jsonl"));
                    if let Some(header) = read(&path) {
                        return Some(header);
                    }
                }
            }
        }
    }
    None
}

/// Reopen a session whose in-memory handle was consumed by a turn that
/// then failed: `run_turn_with` returns `Err(CoreError)` without the log,
/// but the append-only ledger file is durable — restore from it so the
/// session does not stay wedged as "run in progress" forever.
fn reopen_ledger(core: &vak_core::Core, id: &str) -> Option<vak_session::SessionLog> {
    let path = core
        .sessions_home()
        .join("sessions")
        .join(vak_core::memory::hash_cwd(core.cwd()))
        .join(format!("{id}.jsonl"));
    vak_session::SessionLog::open(path).ok()
}

// ---- Personal-OS surfaces (docs/design/29-personal-os.md P1–P4) -------------

#[derive(serde::Deserialize)]
struct DoctorQuery {
    #[serde(default)]
    session: Option<String>,
}

fn health_report_json(report: vak_core::health::HealthReport) -> serde_json::Value {
    let ok = report.failures == 0;
    let report_text = report
        .checks
        .iter()
        .map(|check| match &check.detail {
            Ok(detail) => format!("✓ {} — {detail}", check.label),
            Err(detail) => format!("✗ {} — {detail}", check.label),
        })
        .chain(report.facts.iter().map(|fact| format!("· {fact}")))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::json!({
        "ok": ok,
        "report": report_text,
        "failures": report.failures,
        "checks": report.checks.iter().map(|c| serde_json::json!({
            "label": c.label,
            "ok": c.detail.is_ok(),
            "detail": match &c.detail { Ok(d) => d, Err(e) => e },
        })).collect::<Vec<_>>(),
        "facts": report.facts,
        "ladder": report.ladder.map(|l| serde_json::json!({
            "legs": l.legs,
            "rendered": l.rendered,
            "objective": l.objective,
            "fallback_legs": l.fallback_legs,
            "annotations": l.annotations,
        })),
    })
}

/// `GET /onboarding` — the derived setup projection
/// (`docs/design/46-stabilization-install-and-onboarding.md` Part III).
///
/// The same `vak_core::onboarding::derive` the CLI renders, so web,
/// desktop, and terminal cannot disagree about what is configured. The
/// install manifest is probed by the CLI (which owns install layout) and
/// is therefore reported here as not-probed; services are probed from the
/// same service manager the ops endpoints already use.
async fn onboarding_state(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    // The service manager probe shells out, so it belongs on a blocking
    // worker rather than inside an async handler (invariant 26).
    let services = tokio::task::spawn_blocking(|| {
        let cfg = vak_ops::OpsConfig::detect();
        let probed: Vec<(String, bool)> = [vak_ops::Service::Gateway, vak_ops::Service::Bridges]
            .into_iter()
            .filter_map(|service| {
                let st = vak_ops::status(service, &cfg);
                (st != vak_ops::State::NotInstalled)
                    .then(|| (service.label().to_string(), st == vak_ops::State::Running))
            })
            .collect();
        (!probed.is_empty()).then_some(probed)
    })
    .await
    .unwrap_or(None);

    let awaiting_activation = service_control::activation_drift(&state.core)
        .await
        .map(|drift| drift.awaiting_activation)
        .unwrap_or_default();
    let projection = vak_core::onboarding::derive(
        &state.core,
        &vak_core::onboarding::ProbedFacts {
            services,
            install: None,
            awaiting_activation,
        },
    );
    Json(projection).into_response()
}

/// `POST /onboarding/seed` — install the Shared starter capabilities.
///
/// Idempotent: new standard skills/plugins are added, untouched shipped
/// content may advance on update, and edited or independently installed
/// content is preserved. Hooks and network defaults are seeded only when
/// their configuration layer is empty. Explicit because seeding is a setup
/// action, never an install side effect (doc 46 D6).
async fn onboarding_seed(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Touches the filesystem and the plugin store; not an async handler's
    // work (invariant 26).
    let outcome = tokio::task::spawn_blocking(vak_core::seed::seed_shared_capabilities).await;
    match outcome {
        Ok(Ok(())) => {
            state.hub.emit_config_changed("capabilities_seeded", "");
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("seeding did not complete: {e}") })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct WorkspacePath {
    path: String,
}

/// `POST /onboarding/workspace-review` — what a folder would ask for.
///
/// Reports privileged sections **without loading them** (doc 46, Step 2).
/// Describing a project's config by parsing it through the normal loader
/// would activate the very thing the operator is being asked about.
async fn onboarding_workspace_review(Json(body): Json<WorkspacePath>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = std::path::PathBuf::from(body.path.trim());
    if std::fs::read_dir(&path).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("cannot read {}", path.display()) })),
        )
            .into_response();
    }
    Json(serde_json::json!({
        "path": path,
        "git": path.join(".git").exists(),
        "requests_privilege": vak_core::trust::requests_privilege(&path),
        "privileges": vak_core::trust::requested_privileges(&path),
        "trusted": vak_core::trust::is_trusted(&path),
    }))
    .into_response()
}

/// `POST /onboarding/trust` — record an explicit trust decision.
///
/// Only ever *grants*: opening safely is the absence of a decision, and is
/// already the default, so there is nothing to write for it. Selecting a
/// folder is never itself consent (doc 46 security invariant 2).
async fn onboarding_trust(
    State(state): State<AppState>,
    Json(body): Json<WorkspacePath>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = std::path::PathBuf::from(body.path.trim());
    match vak_core::trust::record(&path) {
        Ok(()) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ConfigChange,
                "workspace_trusted",
                &path.display().to_string(),
                None,
            );
            Json(serde_json::json!({ "trusted": true, "path": path })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// The starter task. Read-only by construction, and deliberately not
/// something the caller supplies: a prompt this endpoint accepted would be
/// a way to run arbitrary work under the onboarding path.
const FIRST_TASK_PROMPT: &str = "Map this codebase and explain its architecture, key flows, \
     and highest-risk areas. Do not modify files or run any destructive command.";

/// `POST /onboarding/first-task` — create the guided starter session.
///
/// Capped to read-only **regardless of the workspace's configured mode**
/// (doc 46 security invariant 5). The cap is not advisory and not the
/// caller's to choose: the session is created against a `Core` resolved
/// through `CorePool` with a read-only override, which `capped_by` folds
/// against the workspace ceiling as a `min` — so the result is provably
/// never more permissive than the workspace, and never less strict than
/// read-only. The handle carries that `Core`, and every run path uses the
/// handle's `Core`, so the cap holds for the actual dispatch rather than
/// only at creation.
///
/// Returns the session id; the caller drives it through the normal run and
/// SSE endpoints, which is what makes its receipt an ordinary receipt.
async fn onboarding_first_task(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let workspace = state.core.cwd().clone();
    let capped = match state.gateway.core_pool.resolve_at(
        &workspace,
        Some(vak_config::PermissionMode::ReadOnly),
        std::time::Instant::now(),
    ) {
        Ok(core) => core,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };

    let session = match capped.start_session().await {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let id = session
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();
    register_handle(&state, id.clone(), session, workspace, capped.clone());
    state.hub.emit_session_created(&id, "");
    index_session_later(state.store.clone(), state.core.sessions_home(), id.clone());

    Json(serde_json::json!({
        "session_id": id,
        "prompt": FIRST_TASK_PROMPT,
        "permission_mode": format!("{:?}", capped.effective_permission_mode()),
    }))
    .into_response()
}

async fn doctor_report(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<DoctorQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // The optional session adds its frozen-ladder section; a live run owns
    // the ledger, in which case doctor reports without that section rather
    // than failing.
    let session_handle = q.session.and_then(|sid| state.get(&sid));
    let session_guard = session_handle.as_deref().map(|h| {
        h.session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    });
    let report =
        vak_core::health::collect(&state.core, session_guard.as_ref().and_then(|g| g.as_ref()));
    (StatusCode::OK, Json(health_report_json(report))).into_response()
}

#[derive(serde::Deserialize)]
struct BackupExportBody {
    dest_dir: String,
    #[serde(default)]
    include_secrets: bool,
}

#[derive(serde::Deserialize)]
struct BackupImportBody {
    src_dir: String,
    #[serde(default)]
    conflict: Option<String>,
}

/// Equality under canonicalization when both sides resolve; raw compare as
/// a fallback for paths that do not exist yet.
fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

async fn backup_export(
    State(state): State<AppState>,
    Json(body): Json<BackupExportBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let dest = std::path::PathBuf::from(body.dest_dir.trim());
    if dest.as_os_str().is_empty() || same_path(&dest, &home) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "backup destination must differ from the vak home itself"
            })),
        )
            .into_response();
    }
    match tokio::task::spawn_blocking(move || {
        vak_core::backup::export_to(&home, &dest, body.include_secrets)
    })
    .await
    {
        Ok(Ok(manifest)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "manifest": manifest,
                "included_secrets": body.include_secrets,
            })),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn backup_import(
    State(state): State<AppState>,
    Json(body): Json<BackupImportBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let src = std::path::PathBuf::from(body.src_dir.trim());
    if src.as_os_str().is_empty() || same_path(&src, &home) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "backup source must differ from the vak home itself"
            })),
        )
            .into_response();
    }
    let conflict = match body.conflict.as_deref() {
        None | Some("skip") => vak_core::backup::Conflict::Skip,
        Some("rename") => vak_core::backup::Conflict::Rename,
        Some(other) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("unknown conflict policy '{other}': expected \"skip\" or \"rename\"")
                })),
            )
                .into_response();
        }
    };
    match tokio::task::spawn_blocking(move || vak_core::backup::import_from(&src, &home, conflict))
        .await
    {
        Ok(Ok(report)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "copied": report.copied,
                "renamed": report.renamed,
                "skipped": report.skipped,
            })),
        )
            .into_response(),
        Ok(Err(e)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct DigestQuery {
    #[serde(default)]
    days: Option<u32>,
}

async fn digest_report(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<DigestQuery>,
) -> Json<vak_core::digest::DigestReport> {
    let days = q.days.unwrap_or(7).clamp(1, 90);
    Json(vak_core::digest::digest(&state.core.sessions_home(), days))
}

// ---- Intent kernel + commitments (docs/design/47-commitment-kernel.md) -----

#[derive(serde::Deserialize)]
struct IntentExplainQuery {
    prompt: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    act: Option<String>,
    #[serde(default)]
    horizon: Option<String>,
    #[serde(default)]
    stakes: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

/// Resolve a prompt without running it.
///
/// The same free tiers the runtime uses, so what this returns is what that
/// prompt would actually get. Costs nothing and dispatches nothing, which is
/// what makes it safe to call from a composer as the user types.
async fn intent_explain(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<IntentExplainQuery>,
) -> axum::response::Response {
    // A registered session already carries the exact Core it was opened
    // under (agent identity, isolated workspace) — resolving through it
    // keeps this preview consistent with what that session's own turns
    // would actually resolve, instead of always describing the default
    // "vak" workspace regardless of which Agent's composer called this.
    let core = q
        .session_id
        .as_deref()
        .and_then(|sid| state.get(sid))
        .map(|handle| handle.core.clone())
        .unwrap_or_else(|| state.core.clone());
    let mut declared = vak_intent::Declared::default();
    let mut bad = Vec::new();
    if let Some(raw) = &q.act {
        match vak_intent::Act::parse(raw) {
            Some(value) => declared.act = Some(value),
            None => bad.push(format!("act '{raw}'")),
        }
    }
    if let Some(raw) = &q.horizon {
        match vak_intent::Horizon::parse(raw) {
            Some(value) => declared.horizon = Some(value),
            None => bad.push(format!("horizon '{raw}'")),
        }
    }
    if let Some(raw) = &q.stakes {
        match vak_intent::Stakes::parse(raw) {
            Some(value) => declared.stakes = Some(value),
            None => bad.push(format!("stakes '{raw}'")),
        }
    }
    if let Some(raw) = &q.evidence {
        match vak_intent::Evidence::parse(raw) {
            Some(value) => declared.evidence = Some(value),
            None => bad.push(format!("evidence '{raw}'")),
        }
    }
    if !bad.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("unknown {}", bad.join(", ")) })),
        )
            .into_response();
    }

    let surface = match q.surface.as_deref() {
        None => core.surface().clone(),
        Some(raw) => match vak_intent::Surface::parse(raw) {
            Some(vak_intent::Surface::Cli) => vak_core::Surface::Cli,
            Some(vak_intent::Surface::Desktop) => vak_core::Surface::Desktop,
            Some(vak_intent::Surface::Server) => vak_core::Surface::Server,
            Some(vak_intent::Surface::Chat) => vak_core::Surface::Chat {
                channel: "chat".into(),
            },
            Some(vak_intent::Surface::Cron | vak_intent::Surface::Heartbeat) => {
                vak_core::Surface::Background
            }
            Some(vak_intent::Surface::Worker) => vak_core::Surface::Worker,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("unknown surface '{raw}'") })),
                )
                    .into_response();
            }
        },
    };

    let history = if let Some(sid) = q.session_id.as_deref() {
        match core.open_session(sid).await {
            Ok(session) => {
                let chain = session.chain_to_root();
                let previous_act = chain.iter().rev().find_map(|entry| match &entry.payload {
                    vak_session::EntryPayload::Intent(record) => Some(record.reading.act),
                    _ => None,
                });
                let turn_index = chain
                    .iter()
                    .filter(|entry| matches!(entry.payload, vak_session::EntryPayload::Message(_)))
                    .count();
                vak_intent::HistoryFacts {
                    previous_act,
                    turn_index,
                    commitment_open: session
                        .header()
                        .and_then(|header| header.contract_id.as_ref())
                        .is_some(),
                }
            }
            Err(_) => vak_intent::HistoryFacts::default(),
        }
    } else {
        vak_intent::HistoryFacts::default()
    };

    let resolution = vak_core::intent::resolve_turn(
        &q.prompt,
        &surface,
        &[],
        vak_core::intent::workspace_facts(core.cwd()),
        history.clone(),
        &declared,
        &core.turn_authority_for(&surface),
        &vak_core::intent::resolver_config(core.config()),
    );
    let escalation = match &resolution {
        vak_intent::Resolution::Escalate { reason, .. } => Some(reason.clone()),
        vak_intent::Resolution::Settled(_) => None,
    };
    let intent = resolution.intent();
    Json(serde_json::json!({
        "reading": intent.reading,
        "engagement": intent.engagement,
        "provenance": intent.provenance,
        "narrows": intent
            .engagement
            .limits
            .diff_from(&vak_intent::Limits::unrestricted()),
        "escalation_recommended": escalation,
        "model_visible": intent.model_visible(),
        "history": {
            "turn_index": history.turn_index,
            "previous_act": history.previous_act.map(|a| a.as_str()),
            "commitment_open": history.commitment_open,
        },
    }))
    .into_response()
}

/// The resolved intent and commitment policy for this workspace.
async fn intent_policy(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.core.config();
    Json(serde_json::json!({
        "intent": {
            "enabled": config.intent.enabled,
            "accept_confidence": config.intent.accept_confidence,
            "provisional_confidence": config.intent.provisional_confidence,
            "slice_capabilities": config.intent.slice_capabilities,
            "posture": config.intent.posture,
            "escalate": config.intent.escalate,
            "max_classify_usd": config.intent.max_classify_usd,
            "autonomy": config.intent.autonomy,
            "evidence_max_age_secs": config.intent.evidence_max_age_secs,
        },
        "commitment": {
            "enabled": config.commitment.enabled,
            "lifetime_budget_usd": config.commitment.lifetime_budget_usd,
            "stall_limit": config.commitment.stall_limit,
            "review_every_hours": config.commitment.review_every_hours,
            "default_ttl_days": config.commitment.default_ttl_days,
        },
    }))
}

#[derive(serde::Deserialize)]
struct CommitmentQuery {
    /// Include closed commitments.
    #[serde(default)]
    all: bool,
    #[serde(default)]
    agent: Option<String>,
}

/// The portfolio, in the order the scheduler would work it.
async fn list_commitments(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<CommitmentQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let ledger = vak_commit::CommitmentLedger::new(&core.sessions_home());
    let commitments = if q.all { ledger.all() } else { ledger.open() };
    let ranked = vak_commit::rank(&commitments, &vak_commit::SchedulerContext::default());
    Json(serde_json::json!({
        "commitments": commitments,
        // Priorities ride alongside rather than being baked into the rows:
        // the ordering is a scheduling opinion, and a UI should be able to
        // show why as well as what.
        "priorities": ranked,
    }))
    .into_response()
}

async fn get_commitment(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let ledger = vak_commit::CommitmentLedger::new(&core.sessions_home());
    match ledger.get(&id) {
        Ok(Some(commitment)) => Json(serde_json::json!({
            "commitment": commitment,
            "events": ledger.events_for(&id),
        }))
        .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no such commitment" })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct CloseCommitmentBody {
    verdict: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    agent: Option<String>,
}

async fn close_commitment(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<CloseCommitmentBody>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let ledger = vak_commit::CommitmentLedger::new(&core.sessions_home());
    let Ok(Some(commitment)) = ledger.get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no such commitment" })),
        )
            .into_response();
    };
    let verdict = match body.verdict.as_str() {
        "fulfilled" => vak_commit::Verdict::Fulfilled,
        "partial" => vak_commit::Verdict::Partial,
        "failed" => vak_commit::Verdict::Failed,
        "abandoned" => vak_commit::Verdict::Abandoned,
        "expired" => vak_commit::Verdict::Expired,
        "unknown" => vak_commit::Verdict::Unknown,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown verdict '{other}'") })),
            )
                .into_response();
        }
    };
    let strength = commitment.achieved_strength();
    match ledger.append(&vak_commit::Event::new(
        &commitment.commitment_id,
        vak_commit::EventKind::Closed {
            verdict,
            strength,
            evidence: Vec::new(),
            note: body.note,
        },
    )) {
        Ok(()) => {
            Json(serde_json::json!({ "ok": true, "verdict": verdict.as_str() })).into_response()
        }
        // A refused closure is a 409, not a 500: the request was well-formed
        // and the server is fine — the evidence simply does not support the
        // claim. The message says which evidence was missing.
        Err(error) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

// ---- Inbox (durable attention layer, docs/design/29-personal-os.md P6) ------

const DEFAULT_INBOX_LIMIT: usize = 200;

#[derive(serde::Deserialize)]
struct InboxQuery {
    #[serde(default)]
    limit: Option<usize>,
    /// Only entries without an ack tombstone.
    #[serde(default)]
    unread: bool,
}

/// Newest-first inbox entries plus the live unread total. The count always
/// reflects the full unfiltered set; `limit` bounds the returned window only.
async fn inbox_list(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<InboxQuery>,
) -> Json<serde_json::Value> {
    let home = state.core.shared_data_home();
    let unread_count = vak_core::inbox::unread_count(&home);
    let limit = q
        .limit
        .unwrap_or(DEFAULT_INBOX_LIMIT)
        .clamp(1, vak_core::inbox::MAX_SCAN);
    let entries = if q.unread {
        vak_core::inbox::unread(&home)
    } else {
        vak_core::inbox::list(&home, limit)
    }
    .into_iter()
    .take(limit)
    .collect::<Vec<_>>();
    let entries = entries
        .into_iter()
        .map(|entry| {
            let mut value = serde_json::to_value(&entry).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(session_id) = entry.session_id.as_deref() {
                let available = state.get(session_id).is_some()
                    || open_historical_session(&state, session_id).is_some();
                value["origin_state"] = serde_json::json!(if available {
                    "available"
                } else {
                    "unavailable"
                });
            }
            value
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "entries": entries, "unread_count": unread_count }))
}

async fn inbox_unread_count(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "count": vak_core::inbox::unread_count(&state.core.shared_data_home())
    }))
}

/// Idempotent read-state: a tombstone append via `inbox::ack`. An unknown id
/// is a 404; re-acking reports `{acked:false}` instead of writing twice.
async fn inbox_ack(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.shared_data_home();
    if !vak_core::inbox::list(&home, vak_core::inbox::MAX_SCAN)
        .iter()
        .any(|e| e.id == id)
    {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("unknown inbox entry '{id}'") })),
        )
            .into_response();
    }
    match vak_core::inbox::ack(&home, &id) {
        Ok(acked) => Json(serde_json::json!({ "acked": acked })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn git_output(cwd: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Workspace diff for the review pane. Untracked files appear in `status`
/// as `??` lines; patches are split per-file client-side.
async fn session_diff(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(handle) = state.get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response();
    };
    let cwd = handle.cwd.clone();
    let Some(status) = git_output(&cwd, &["status", "--porcelain"]).await else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({ "error": "not a git repository" })),
        )
            .into_response();
    };
    // `--no-color` is a `diff` option, not a global git flag — placed
    // before the subcommand (as this read for a long time) git rejects it
    // outright ("unknown option: --no-color", exit 129), and `git_output`
    // turns that failure into a silently empty string via
    // `unwrap_or_default()`. Every consumer of this endpoint — this
    // console's Worktree Diff tab, the desktop DiffPane, `openFileSmart`'s
    // diff-vs-editor routing — has been reading an empty diff regardless
    // of what actually changed.
    let diff = git_output(&cwd, &["diff", "--no-color", "--unified=3"])
        .await
        .unwrap_or_default();
    let staged = git_output(&cwd, &["diff", "--no-color", "--cached", "--unified=3"])
        .await
        .unwrap_or_default();
    Json(serde_json::json!({
        "root": cwd,
        "diff": diff,
        "staged_diff": staged,
        "status": status,
    }))
    .into_response()
}

// ---- checkpoints (time travel) ----------------------------------------------

async fn list_checkpoints(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    // The session id itself is enough to resolve the Agent that owns this
    // session's sessions_home (a registered session's own Core is reused
    // when it's still open; a checkpoint search for a closed session falls
    // back to the default "vak" Agent below, matching prior behaviour).
    let core = match resolve_scoped_core(&state, Some(&id), None) {
        Ok(core) => core,
        Err(response) => return response,
    };
    // A session with no snapshots yet has no directory; that's an empty
    // list, not an error.
    let list = match vak_core::checkpoints::list(&core.sessions_home(), &id) {
        Ok(list) if !list.is_empty() => list,
        _ => match vak_core::checkpoints::list(&core.shared_data_home(), &id) {
            Ok(list) => list,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        },
    };
    let checkpoints: Vec<serde_json::Value> = list
        .iter()
        .map(|cp| {
            serde_json::json!({
                "seq": cp.seq,
                "label": cp.label,
                "created_at": cp.created_at.to_rfc3339(),
                "files": cp.files.len(),
            })
        })
        .collect();
    Json(serde_json::json!({ "checkpoints": checkpoints })).into_response()
}

async fn restore_checkpoint(
    State(state): State<AppState>,
    Path((id, seq)): Path<(String, u32)>,
) -> axum::response::Response {
    // A live run must never have its workspace mutated underneath it.
    if let Some(handle) = state.get(&id)
        && handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
    {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "a run is active on this session" })),
        )
            .into_response();
    }
    let core = match resolve_scoped_core(&state, Some(&id), None) {
        Ok(core) => core,
        Err(response) => return response,
    };
    // Best-of-N children captured inside their worktrees; attached handles
    // know that cwd. Everything else restores into the workspace root.
    let cwd = state
        .get(&id)
        .map(|h| h.cwd.clone())
        .unwrap_or_else(|| core.cwd().clone());
    let cp = match vak_core::checkpoints::load(&core.sessions_home(), &id, seq)
        .or_else(|_| vak_core::checkpoints::load(&core.shared_data_home(), &id, seq))
    {
        Ok(cp) => cp,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": format!("checkpoint {seq} not found") })),
            )
                .into_response();
        }
    };
    match tokio::task::spawn_blocking(move || vak_core::checkpoints::restore(&cwd, &cp)).await {
        Ok(Ok((restored, deleted))) => Json(serde_json::json!({
            "restored": restored,
            "deleted": deleted,
            "seq": seq,
        }))
        .into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---- archive (sidebar visibility; ledgers stay untouched) --------------------

fn archive_path(core: &Core) -> PathBuf {
    core.shared_data_home().join("archive.json")
}

fn deleted_path(core: &Core) -> PathBuf {
    core.shared_data_home().join("deleted.json")
}

fn read_archive(core: &Core) -> HashMap<String, bool> {
    std::fs::read_to_string(archive_path(core))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn read_deleted(core: &Core) -> HashMap<String, bool> {
    std::fs::read_to_string(deleted_path(core))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_deleted(core: &Core, map: &HashMap<String, bool>) {
    if let Some(parent) = deleted_path(core).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = deleted_path(core).with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string(map).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, deleted_path(core));
    }
}

fn write_archive(core: &Core, map: &HashMap<String, bool>) {
    if let Some(parent) = archive_path(core).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = archive_path(core).with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_string(map).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, archive_path(core));
    }
}

fn find_session_in_cwd(core: &Core, id: &str) -> bool {
    let direct = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd())
        .join(format!("{id}.jsonl"));
    if direct.is_file() {
        return true;
    }
    let shared = core.shared_data_home();
    if let Ok(agents) = std::fs::read_dir(shared.join("agents")) {
        for agent in agents.flatten() {
            let candidate = vak_session::SessionPath::sessions_dir(&agent.path(), core.cwd())
                .join(format!("{id}.jsonl"));
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

#[derive(serde::Deserialize)]
struct ArchiveBody {
    archived: bool,
}

async fn set_archived(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ArchiveBody>,
) -> axum::response::Response {
    if !find_session_in_cwd(&state.core, &id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response();
    }
    let mut map = read_archive(&state.core);
    map.insert(id, body.archived);
    write_archive(&state.core, &map);
    Json(serde_json::json!({ "archived": body.archived })).into_response()
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    if !find_session_in_cwd(&state.core, &id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response();
    }
    if state.get(&id).is_some_and(|handle| {
        handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none()
    }) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "cannot delete a running task" })),
        )
            .into_response();
    }
    if !read_archive(&state.core).get(&id).copied().unwrap_or(false) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "only archived tasks can be deleted" })),
        )
            .into_response();
    }
    let mut deleted = read_deleted(&state.core);
    deleted.insert(id.clone(), true);
    write_deleted(&state.core, &deleted);
    // Drop the session's cached FinOps spend gate along with it (docs/design/42-managed-work-contracts.md// Phase D) — otherwise a long-running server accumulates one entry per
    // session ever seen, forever.
    state.core.forget_spend_gate(&id);
    Json(serde_json::json!({ "deleted": id })).into_response()
}

async fn delete_all_archived(State(state): State<AppState>) -> axum::response::Response {
    let archive = read_archive(&state.core);
    let local_archived: Vec<String> = archive
        .into_iter()
        .filter(|(id, archived)| *archived && find_session_in_cwd(&state.core, id))
        .map(|(id, _)| id)
        .collect();
    let running_archived = local_archived.iter().any(|id| {
        state.get(id).is_some_and(|handle| {
            handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        })
    });
    if running_archived {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "stop running archived tasks before deleting all" })),
        )
            .into_response();
    }
    let mut deleted = read_deleted(&state.core);
    let mut count = 0u64;
    for id in local_archived {
        if !deleted.get(&id).copied().unwrap_or(false) {
            deleted.insert(id.clone(), true);
            count += 1;
        }
        state.core.forget_spend_gate(&id);
    }
    write_deleted(&state.core, &deleted);
    Json(serde_json::json!({ "deleted": count })).into_response()
}

async fn list_skills(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    // `path` and `scope` tell the reader WHERE a skill came from. Discovery
    // reads two roots (`<cwd>/.vak/skills` then the Shared
    // `~/vak-home/.vak/skills` root), and a workspace skill is a very different
    // trust proposition from a Shared skill
    // one -- the admin console groups by this.
    let workspace_root = core.cwd().join(".vak/skills");
    let shared_root = vak_config::paths::default_workspace().join(".vak/skills");
    let skills: Vec<serde_json::Value> = core
        .skills_with_shadowed()
        .iter()
        .map(|s| {
            let scope = if s.path.starts_with(&shared_root) {
                "user"
            } else if s.path.starts_with(&workspace_root) {
                "workspace"
            } else {
                "user"
            };
            serde_json::json!({
                "name": s.name,
                "description": s.description,
                "path": s.path.display().to_string(),
                "scope": scope,
                "provenance": s.provenance,
                "shadowed": s.shadowed,
            })
        })
        .collect();
    Json(serde_json::json!({ "skills": skills })).into_response()
}

async fn list_commands(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    Json(
        serde_json::json!({ "commands": core.custom_commands().into_iter().map(|command| serde_json::json!({
        "name": command.name, "description": command.description, "source": command.source,
    })).collect::<Vec<_>>() }),
    )
    .into_response()
}

#[derive(Debug, serde::Deserialize)]
struct PluginMutation {
    path: Option<PathBuf>,
    #[serde(default)]
    scope: Option<InstallScope>,
    #[serde(default)]
    allow_unlicensed: bool,
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PluginSourceMutation {
    path: PathBuf,
    #[serde(default)]
    scope: Option<InstallScope>,
    #[serde(default)]
    label: String,
    #[serde(default = "default_marketplace_trust")]
    trust: MarketplaceTrust,
    key_id: Option<String>,
    public_key: Option<String>,
    signature: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

fn default_marketplace_trust() -> MarketplaceTrust {
    MarketplaceTrust::ManualReview
}

fn plugin_store(core: &vak_core::Core, scope: InstallScope) -> PluginStore {
    let root = match scope {
        InstallScope::User => vak_config::paths::default_workspace().join(".vak"),
        InstallScope::Workspace => core.cwd().join(".vak"),
    };
    PluginStore::new(root)
}

#[derive(Debug, serde::Deserialize)]
struct PluginScopeQuery {
    scope: Option<InstallScope>,
    #[serde(default)]
    agent: Option<String>,
}

async fn list_retired_plugins(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut retired = Vec::new();
    for scope in [InstallScope::User, InstallScope::Workspace] {
        if let Ok(flagged) = plugin_store(&core, scope).retired_plugins() {
            for (name, tools) in flagged {
                retired.push(serde_json::json!({
                    "name": name,
                    "retired_tools": tools,
                    "repair": "run `vak setup seed` to auto-remove, or `vak plugins remove <name>`",
                }));
            }
        }
    }
    Json(serde_json::json!({ "retired": retired })).into_response()
}

async fn remove_retired_plugins(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut removed = Vec::new();
    let mut errors = Vec::new();
    for scope in [InstallScope::User, InstallScope::Workspace] {
        let store = plugin_store(&core, scope);
        if let Ok(flagged) = store.retired_plugins() {
            for (name, _) in &flagged {
                match store.remove(name) {
                    Ok(_) => {
                        removed.push(name.clone());
                        let _ = vak_config::prune_plugins_network_allow(
                            &core.cwd().join(".vak/config.toml"),
                            name,
                        );
                    }
                    Err(e) => errors.push(format!("{name}: {e}")),
                }
            }
        }
    }
    if errors.is_empty() {
        (
            StatusCode::OK,
            Json(serde_json::json!({ "removed": removed })),
        )
            .into_response()
    } else {
        (
            StatusCode::PARTIAL_CONTENT,
            Json(serde_json::json!({ "removed": removed, "errors": errors })),
        )
            .into_response()
    }
}

#[derive(Debug, serde::Deserialize)]
struct PluginCatalogQuery {
    scope: Option<InstallScope>,
    q: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

fn requested_plugin_scopes(scope: Option<InstallScope>) -> Vec<InstallScope> {
    scope.map_or_else(
        || vec![InstallScope::User, InstallScope::Workspace],
        |scope| vec![scope],
    )
}

// Presentations are process-default-workspace scoped today, unlike the
// plugin store itself; out of scope for this per-Agent isolation pass
// (not one of the audited endpoints) and left untouched deliberately.
fn presentation_store(state: &AppState) -> vak_store::presentation::PresentationStore {
    vak_store::presentation::PresentationStore::new(
        state.core.sessions_home().join("presentations.json"),
    )
}

#[derive(Debug, serde::Deserialize)]
struct PresentationPackBody {
    records: Vec<vak_presentation::StoredPresentation>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PresentationExport {
    schema_version: u16,
    definitions: Vec<vak_presentation::StoredPresentation>,
    activations: Vec<vak_presentation::PresentationActivation>,
}

async fn list_presentations(State(state): State<AppState>) -> axum::response::Response {
    let store = presentation_store(&state);
    match store.load().and_then(|mut library| {
        let before = library.definitions().count();
        let mut changed = false;
        for seed in vak_presentation::seeds::built_in_seed_pack() {
            if let Some(existing) = library.get(&seed.spec.id, seed.spec.revision)
                && existing.digest != seed.digest
            {
                changed = true;
            }
            library.register(seed).map_err(|error| {
                vak_store::presentation::PresentationStoreError::Invalid(error.to_string())
            })?;
        }
        if changed || library.definitions().count() != before {
            store.save(&library)?;
        }
        Ok(library)
    }) {
        Ok(library) => Json(serde_json::json!({
            "definitions": library.definitions().collect::<Vec<_>>(),
            "activations": library.activations(),
        }))
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn export_presentations(State(state): State<AppState>) -> axum::response::Response {
    match presentation_store(&state).load() {
        Ok(library) => Json(PresentationExport {
            schema_version: 1,
            definitions: library.definitions().cloned().collect(),
            activations: library.activations().to_vec(),
        })
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn import_presentations(
    State(state): State<AppState>,
    Json(pack): Json<PresentationExport>,
) -> axum::response::Response {
    if pack.schema_version != 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "unsupported presentation pack schema" })),
        )
            .into_response();
    }
    let store = presentation_store(&state);
    let result = store.load().and_then(|mut library| {
        for mut definition in pack.definitions {
            // Pack import is always a preview operation. Never trust an
            // enabled bit from an external serialized projection.
            definition.enabled = false;
            library.register(definition).map_err(|error| {
                vak_store::presentation::PresentationStoreError::Invalid(error.to_string())
            })?;
        }
        store.save(&library)
    });
    match result {
        Ok(()) => Json(serde_json::json!({ "imported": true })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn list_presentation_primitives() -> Json<Vec<vak_presentation::Primitive>> {
    Json(vec![
        vak_presentation::Primitive::Stack,
        vak_presentation::Primitive::Row,
        vak_presentation::Primitive::Section,
        vak_presentation::Primitive::Text,
        vak_presentation::Primitive::Title,
        vak_presentation::Primitive::Badge,
        vak_presentation::Primitive::List,
        vak_presentation::Primitive::Table,
        vak_presentation::Primitive::KeyValue,
        vak_presentation::Primitive::Progress,
        vak_presentation::Primitive::LinkPreview,
        vak_presentation::Primitive::Image,
        vak_presentation::Primitive::Divider,
        vak_presentation::Primitive::Artifact,
        vak_presentation::Primitive::Map,
        vak_presentation::Primitive::Calendar,
        vak_presentation::Primitive::Board,
        vak_presentation::Primitive::Graph,
        vak_presentation::Primitive::Entity,
        vak_presentation::Primitive::Evidence,
        vak_presentation::Primitive::Form,
        vak_presentation::Primitive::Transaction,
        vak_presentation::Primitive::Alert,
        vak_presentation::Primitive::Conversation,
        vak_presentation::Primitive::Simulation,
    ])
}

async fn get_presentation_spec(
    State(state): State<AppState>,
    Path((id, revision)): Path<(String, u64)>,
) -> axum::response::Response {
    match presentation_store(&state).load() {
        Ok(library) => match library.get(&id, revision) {
            Some(definition) => Json(definition).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn register_presentations(
    State(state): State<AppState>,
    Json(body): Json<PresentationPackBody>,
) -> axum::response::Response {
    match presentation_store(&state).register_pack(body.records) {
        Ok(count) => Json(serde_json::json!({ "registered": count })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct PresentationScopeBody {
    scope: vak_presentation::LibraryScope,
    owner: String,
}

#[derive(Debug, serde::Deserialize)]
struct PresentationRevisionBody {
    request: vak_presentation::PresentationRevisionRequest,
    proposed: vak_presentation::PresentationSpec,
    origin: vak_presentation::PresentationOrigin,
    #[serde(default)]
    chain_id: Option<String>,
}

async fn propose_presentation_revision(
    State(state): State<AppState>,
    Json(body): Json<PresentationRevisionBody>,
) -> axum::response::Response {
    let store = presentation_store(&state);
    let result = store.load().and_then(|mut library| {
        let revision = library
            .register_revision(body.request, body.proposed, body.origin)
            .map_err(|error| {
                vak_store::presentation::PresentationStoreError::Invalid(error.to_string())
            })?;
        store.save(&library)?;
        Ok(revision)
    });
    match result {
        Ok(revision) => Json(revision).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn propose_session_presentation_revision(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PresentationRevisionBody>,
) -> axum::response::Response {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let chain_id = body
        .chain_id
        .clone()
        .unwrap_or_else(|| body.request.base_id.clone());
    if chain_id.trim().is_empty() || chain_id.len() > 256 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (attempts, rejected) = {
        let activities = handle
            .activity_buffer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        activities
            .iter()
            .fold((0_u8, 0_u8), |(attempts, rejected), activity| {
                if activity.data.get("chain_id") != Some(&chain_id) {
                    return (attempts, rejected);
                }
                match activity.kind {
                    vak_session::ActivityKind::PresentationProposal => (
                        attempts.saturating_add(1),
                        rejected.saturating_add(u8::from(
                            activity.status == vak_session::ActivityStatus::Failed,
                        )),
                    ),
                    vak_session::ActivityKind::PresentationFeedback
                        if activity.status == vak_session::ActivityStatus::Denied =>
                    {
                        (attempts, rejected.saturating_add(1))
                    }
                    _ => (attempts, rejected),
                }
            })
    };
    if attempts >= 2 || rejected >= 2 || body.request.attempt == 0 || body.request.attempt > 2 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({ "error": "presentation proposal chain exhausted" })),
        )
            .into_response();
    }
    let store = presentation_store(&state);
    let result = store.load().and_then(|mut library| {
        let revision = library
            .register_revision(body.request, body.proposed, body.origin)
            .map_err(|error| {
                vak_store::presentation::PresentationStoreError::Invalid(error.to_string())
            })?;
        store.save(&library)?;
        Ok(revision)
    });
    match result {
        Ok(revision) => {
            let mut data = std::collections::BTreeMap::new();
            data.insert("spec_id".into(), revision.proposed.id.clone());
            data.insert("revision".into(), revision.proposed.revision.to_string());
            data.insert("digest".into(), revision.digest.clone());
            data.insert("chain_id".into(), chain_id.clone());
            let activity = vak_session::ActivityRecord {
                activity_id: format!("presentation-proposal-{}", uuid::Uuid::now_v7()),
                turn: None,
                kind: vak_session::ActivityKind::PresentationProposal,
                status: vak_session::ActivityStatus::Succeeded,
                label: "Presentation proposal previewed".into(),
                detail: Some("immutable disabled revision".into()),
                data,
            };
            handle
                .activity_buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(activity);
            Json(revision).into_response()
        }
        Err(error) => {
            let mut data = std::collections::BTreeMap::new();
            data.insert("chain_id".into(), chain_id);
            data.insert("error".into(), error.to_string());
            handle
                .activity_buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(vak_session::ActivityRecord {
                    activity_id: format!("presentation-proposal-{}", uuid::Uuid::now_v7()),
                    turn: None,
                    kind: vak_session::ActivityKind::PresentationProposal,
                    status: vak_session::ActivityStatus::Failed,
                    label: "Presentation proposal rejected".into(),
                    detail: Some("invalid immutable revision".into()),
                    data,
                });
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response()
        }
    }
}

async fn activate_presentation(
    State(state): State<AppState>,
    Path((id, revision)): Path<(String, u64)>,
    Json(body): Json<PresentationScopeBody>,
) -> axum::response::Response {
    let store = presentation_store(&state);
    let result = store.load().and_then(|mut library| {
        let activation = library
            .activate(&id, revision, body.scope, &body.owner)
            .map_err(|error| {
                vak_store::presentation::PresentationStoreError::Invalid(error.to_string())
            })?;
        store.save(&library)?;
        Ok(activation)
    });
    match result {
        Ok(activation) => Json(activation).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn deactivate_presentation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PresentationScopeBody>,
) -> axum::response::Response {
    let store = presentation_store(&state);
    match store.load() {
        Ok(mut library) => {
            library.deactivate(&id, body.scope, &body.owner);
            match store.save(&library) {
                Ok(()) => Json(serde_json::json!({ "deactivated": true })).into_response(),
                Err(error) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": error.to_string() })),
                )
                    .into_response(),
            }
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn activate_all_presentations(
    State(state): State<AppState>,
    Json(body): Json<PresentationScopeBody>,
) -> axum::response::Response {
    let store = presentation_store(&state);
    let result = store.load().and_then(|mut library| {
        let mut latest_by_id: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();
        for def in library.definitions() {
            let entry = latest_by_id
                .entry(def.spec.id.clone())
                .or_insert(def.spec.revision);
            if def.spec.revision > *entry {
                *entry = def.spec.revision;
            }
        }
        let mut activated = 0;
        for (id, rev) in latest_by_id {
            if library.activate(&id, rev, body.scope, &body.owner).is_ok() {
                activated += 1;
            }
        }
        store.save(&library)?;
        Ok(activated)
    });
    match result {
        Ok(count) => Json(serde_json::json!({ "activated": count })).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn deactivate_all_presentations(
    State(state): State<AppState>,
    Json(body): Json<PresentationScopeBody>,
) -> axum::response::Response {
    let store = presentation_store(&state);
    match store.load() {
        Ok(mut library) => {
            let before = library.activations().len();
            let spec_ids: Vec<String> = library
                .activations()
                .iter()
                .filter(|a| a.scope == body.scope && a.owner == body.owner)
                .map(|a| a.spec_id.clone())
                .collect();
            for id in spec_ids {
                library.deactivate(&id, body.scope, &body.owner);
            }
            let removed = before.saturating_sub(library.activations().len());
            match store.save(&library) {
                Ok(()) => Json(serde_json::json!({ "deactivated": removed })).into_response(),
                Err(error) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": error.to_string() })),
                )
                    .into_response(),
            }
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn reset_presentation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PresentationScopeBody>,
) -> axum::response::Response {
    let store = presentation_store(&state);
    match store.load() {
        Ok(mut library) => {
            let restored = library.reset(&id, body.scope, &body.owner);
            match store.save(&library) {
                Ok(()) => {
                    Json(serde_json::json!({ "reset": true, "restored": restored })).into_response()
                }
                Err(error) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": error.to_string() })),
                )
                    .into_response(),
            }
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn revoke_presentations_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> axum::response::Response {
    match presentation_store(&state).revoke_plugin(&plugin_id) {
        Ok(removed) => Json(serde_json::json!({ "removed": removed })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn list_plugins(
    State(state): State<AppState>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut plugins = Vec::new();
    for scope in requested_plugin_scopes(query.scope) {
        if let Ok(items) = plugin_store(&core, scope).list() {
            let policy = core.effective_plugins();
            plugins.extend(
                items
                    .into_iter()
                    .map(|plugin| {
                        serde_json::json!({
                            "name": plugin.name,
                            "version": plugin.version,
                            "digest": plugin.digest,
                            "description": plugin.description,
                            "format": plugin.format,
                            "scope": plugin.scope,
                            "enabled": plugin.enabled,
                            "network_allowed": policy.is_network_allowed(&plugin.name),
                            "network_denied": policy.network_deny.contains(&plugin.name),
                            "network_allow": policy.network_allow,
                            "trace_id": plugin.trace_id,
                            "capabilities": plugin.capabilities,
                            "warnings": plugin.warnings,
                        })
                    })
                    .collect::<Vec<_>>(),
            );
        }
    }
    Json(serde_json::json!({ "plugins": plugins })).into_response()
}

async fn plugin_audit(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut events = Vec::new();
    for scope in [InstallScope::User, InstallScope::Workspace] {
        if let Ok(registry) = plugin_store(&core, scope).load() {
            events.extend(registry.audit);
        }
    }
    events.sort_by_key(|event| event.at_unix);
    Json(serde_json::json!({ "audit": events })).into_response()
}

async fn plugin_invocations(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut events = Vec::new();
    for scope in [InstallScope::User, InstallScope::Workspace] {
        if let Ok(items) = plugin_store(&core, scope).invocations() {
            events.extend(items);
        }
    }
    events.sort_by_key(|event| event.at_unix);
    Json(serde_json::json!({ "invocations": events })).into_response()
}

async fn plugin_sources(
    State(state): State<AppState>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let mut sources = Vec::new();
    for scope in requested_plugin_scopes(query.scope) {
        if let Ok(items) = plugin_store(&core, scope).list_sources() {
            sources.extend(items);
        }
    }
    Json(serde_json::json!({ "sources": sources })).into_response()
}

async fn plugin_catalog(
    State(state): State<AppState>,
    Query(query): Query<PluginCatalogQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let needle = query.q.as_deref().unwrap_or_default().trim().to_lowercase();
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for scope in requested_plugin_scopes(query.scope) {
        let store = plugin_store(&core, scope);
        let sources = match store.list_sources() {
            Ok(sources) => sources,
            Err(error) => {
                errors.push(serde_json::json!({ "scope": scope, "error": error.to_string() }));
                continue;
            }
        };
        for source in sources {
            let inspection = match vak_plugin::inspect_catalog(&source.root) {
                Ok(inspection) => inspection,
                Err(error) => {
                    errors.push(
                        serde_json::json!({ "source_id": source.id, "error": error.to_string() }),
                    );
                    continue;
                }
            };
            if inspection.digest != source.catalog_digest {
                errors.push(serde_json::json!({
                    "source_id": source.id,
                    "error": "catalog changed since registration",
                }));
                continue;
            }
            for entry in inspection.entries {
                let haystack = format!(
                    "{} {}",
                    entry.name,
                    entry.description.as_deref().unwrap_or_default()
                )
                .to_lowercase();
                if !needle.is_empty() && !haystack.contains(&needle) {
                    continue;
                }
                entries.push(serde_json::json!({
                    "source_id": source.id,
                    "source_label": source.label,
                    "source_enabled": source.enabled,
                    "source_scope": scope,
                    "catalog_digest": source.catalog_digest,
                    "name": entry.name,
                    "version": entry.version,
                    "description": entry.description,
                    "license": entry.license,
                }));
            }
        }
    }
    entries.sort_by(|a, b| {
        a["name"]
            .as_str()
            .cmp(&b["name"].as_str())
            .then_with(|| a["source_id"].as_str().cmp(&b["source_id"].as_str()))
    });
    Json(serde_json::json!({ "entries": entries, "errors": errors })).into_response()
}

async fn plugin_register_source(
    State(state): State<AppState>,
    Json(request): Json<PluginSourceMutation>,
) -> axum::response::Response {
    let evidence = match (request.key_id, request.public_key, request.signature) {
        (None, None, None) => None,
        (Some(key_id), Some(public_key), Some(signature)) => Some(SignatureEvidence {
            algorithm: "ed25519".into(),
            key_id,
            public_key,
            signature,
            verified: false,
            revoked: false,
        }),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "key_id, public_key, and signature must be supplied together",
            )
                .into_response();
        }
    };
    let scope = request.scope.unwrap_or(InstallScope::Workspace);
    let core = match resolve_scoped_core(&state, None, request.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(
        plugin_store(&core, scope).register_catalog_source_with_signature(
            &request.path,
            &request.label,
            request.trust,
            evidence,
        ),
    )
}

async fn plugin_source_enable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(
        plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace))
            .set_source_enabled(&id, true),
    )
}

async fn plugin_source_disable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(
        plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace))
            .set_source_enabled(&id, false),
    )
}

async fn plugin_key_revoke(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(
        plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace))
            .set_key_revoked(&id, true)
            .map(|_| serde_json::json!({"revoked": id})),
    )
}

async fn plugin_key_restore(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(
        plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace))
            .set_key_revoked(&id, false)
            .map(|_| serde_json::json!({"revoked": false, "key_id": id})),
    )
}

fn plugin_result(
    result: Result<impl serde::Serialize, vak_plugin::PluginError>,
) -> axum::response::Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn plugin_install(
    State(state): State<AppState>,
    Json(request): Json<PluginMutation>,
) -> axum::response::Response {
    let Some(path) = request.path else {
        return (StatusCode::BAD_REQUEST, "path is required").into_response();
    };
    let scope = request.scope.unwrap_or(InstallScope::Workspace);
    let core = match resolve_scoped_core(&state, None, request.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(plugin_store(&core, scope).install_local(
        &path,
        InstallOptions {
            allow_unlicensed: request.allow_unlicensed,
            scope,
        },
    ))
}

async fn plugin_update(
    State(state): State<AppState>,
    Json(request): Json<PluginMutation>,
) -> axum::response::Response {
    let Some(path) = request.path else {
        return (StatusCode::BAD_REQUEST, "path is required").into_response();
    };
    let scope = request.scope.unwrap_or(InstallScope::Workspace);
    let core = match resolve_scoped_core(&state, None, request.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(plugin_store(&core, scope).update_local(
        &path,
        InstallOptions {
            allow_unlicensed: request.allow_unlicensed,
            scope,
        },
    ))
}

async fn plugin_enable(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    plugin_result(plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace)).enable(&name))
}

async fn plugin_disable(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let result = plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace)).disable(&name);
    match result {
        Ok(value) => match presentation_store(&state).revoke_plugin(&name) {
            Ok(_) => (StatusCode::OK, Json(value)).into_response(),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("plugin disabled but presentation revocation failed: {error}") })),
            ).into_response(),
        },
        Err(error) => plugin_result(Err::<serde_json::Value, _>(error)),
    }
}

async fn plugin_rollback(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let result =
        plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace)).rollback(&name);
    match result {
        Ok(value) => match presentation_store(&state).revoke_plugin(&name) {
            Ok(_) => (StatusCode::OK, Json(value)).into_response(),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("plugin rolled back but presentation revocation failed: {error}") })),
            ).into_response(),
        },
        Err(error) => plugin_result(Err::<serde_json::Value, _>(error)),
    }
}

async fn plugin_remove(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let result = plugin_store(&core, query.scope.unwrap_or(InstallScope::Workspace)).remove(&name);
    match result {
        Ok(value) => match presentation_store(&state).revoke_plugin(&name) {
            Ok(_) => (StatusCode::OK, Json(value)).into_response(),
            Err(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("plugin removed but presentation revocation failed: {error}") })),
            ).into_response(),
        },
        Err(error) => plugin_result(Err::<serde_json::Value, _>(error)),
    }
}

#[derive(serde::Deserialize)]
struct FileQuery {
    path: String,
}

/// Resolve `input` (absolute or cwd-relative) inside the workspace root.
/// Symlinks are resolved for the existing portion; escapes are rejected.
fn confined_path(cwd: &std::path::Path, input: &str) -> Option<std::path::PathBuf> {
    let base = cwd.canonicalize().ok()?;
    let raw = std::path::PathBuf::from(input);
    let joined = if raw.is_absolute() {
        raw
    } else {
        base.join(raw)
    };
    let mut ancestor = joined.as_path();
    loop {
        match ancestor.canonicalize() {
            Ok(canonical) => {
                let tail = joined
                    .strip_prefix(ancestor)
                    .unwrap_or(std::path::Path::new(""));
                // join("") would append a trailing separator and break reads.
                let resolved = if tail.as_os_str().is_empty() {
                    canonical
                } else {
                    canonical.join(tail)
                };
                return if resolved.starts_with(&base) {
                    Some(resolved)
                } else {
                    None
                };
            }
            Err(_) => {
                ancestor = ancestor.parent()?;
            }
        }
    }
}

fn resolve_confined_file(state: &AppState, input: &str) -> Option<std::path::PathBuf> {
    let clean = input
        .trim()
        .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '\'', '"']);
    let active = state.active_core();

    // 1. Direct workspace check
    if let Some(p) =
        confined_path(active.cwd(), clean).or_else(|| confined_path(state.core.cwd(), clean))
        && p.exists()
    {
        return Some(p);
    }

    // 2. Quarantine scratch check: if file is in .vak/scratch/<subdirs>
    for cwd in [active.cwd(), state.core.cwd()] {
        let scratch_dir = cwd.join(".vak").join("scratch");
        if scratch_dir.is_dir() {
            let rel = clean
                .strip_prefix("./")
                .unwrap_or(clean)
                .strip_prefix(".vak/scratch/")
                .unwrap_or_else(|| clean.strip_prefix("scratch/").unwrap_or(clean));

            let direct = scratch_dir.join(rel);
            if direct.is_file()
                && let Some(canon) = confined_path(cwd, &direct.display().to_string())
            {
                return Some(canon);
            }

            // Search execution subdirectories under .vak/scratch (including agent-scoped .vak/scratch/<agent_id>/<exec_id>)
            if let Ok(entries) = std::fs::read_dir(&scratch_dir) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        let sub_path = entry.path().join(rel);
                        if sub_path.is_file()
                            && let Some(canon) = confined_path(cwd, &sub_path.display().to_string())
                        {
                            return Some(canon);
                        }
                        if let Ok(sub_entries) = std::fs::read_dir(entry.path()) {
                            for sub in sub_entries.flatten() {
                                if sub.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                    let nested = sub.path().join(rel);
                                    if nested.is_file()
                                        && let Some(canon) =
                                            confined_path(cwd, &nested.display().to_string())
                                    {
                                        return Some(canon);
                                    }
                                    if let Some(filename) = std::path::Path::new(rel).file_name() {
                                        let by_name = sub.path().join(filename);
                                        if by_name.is_file()
                                            && let Some(canon) =
                                                confined_path(cwd, &by_name.display().to_string())
                                        {
                                            return Some(canon);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(filename) = std::path::Path::new(rel).file_name() {
                            let by_name = entry.path().join(filename);
                            if by_name.is_file()
                                && let Some(canon) =
                                    confined_path(cwd, &by_name.display().to_string())
                            {
                                return Some(canon);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: standard confined_path even if not yet on disk (needed for write_file)
    confined_path(active.cwd(), clean).or_else(|| confined_path(state.core.cwd(), clean))
}

async fn read_file(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(path) = resolve_confined_file(&state, &q.path) else {
        return (StatusCode::FORBIDDEN, "path outside workspace").into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            // Never hand back lossily-decoded bytes: the editor can save what
            // it was given, and a lossy round trip would destroy the file.
            // Binary is reported as binary; images ride back as base64 so the
            // UI can render them.
            let kind = vak_core::files::classify(&path, &bytes);
            let mut body = serde_json::json!({
                "path": q.path,
                "kind": kind.as_str(),
                "bytes": bytes.len(),
            });
            match kind {
                vak_core::files::FileKind::Text => {
                    let mime = vak_core::files::mime_for(&path);
                    if mime == "image/svg+xml" {
                        use base64::Engine;
                        body["data_url"] = serde_json::json!(format!(
                            "data:{mime};base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(&bytes)
                        ));
                    }
                    body["content"] = serde_json::json!(String::from_utf8_lossy(&bytes));
                    body["editable"] = serde_json::json!(true);
                }
                vak_core::files::FileKind::Image => {
                    use base64::Engine;
                    body["data_url"] = serde_json::json!(format!(
                        "data:{};base64,{}",
                        vak_core::files::mime_for(&path),
                        base64::engine::general_purpose::STANDARD.encode(&bytes)
                    ));
                    body["editable"] = serde_json::json!(false);
                }
                vak_core::files::FileKind::Binary => {
                    // Browser-renderable binary formats still need a safe,
                    // authenticated representation. Keep the existing binary
                    // classification (never editable), but expose bounded
                    // data URLs for media/document viewers.
                    let mime = match path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str()
                    {
                        "pdf" => Some("application/pdf"),
                        "mp3" => Some("audio/mpeg"),
                        "wav" => Some("audio/wav"),
                        "ogg" => Some("audio/ogg"),
                        "mp4" => Some("video/mp4"),
                        "webm" => Some("video/webm"),
                        _ => None,
                    };
                    if let Some(mime) = mime
                        && bytes.len() <= 16 * 1024 * 1024
                    {
                        use base64::Engine;
                        body["data_url"] = serde_json::json!(format!(
                            "data:{mime};base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(&bytes)
                        ));
                    }
                    body["editable"] = serde_json::json!(false);
                }
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "file not found").into_response(),
    }
}

/// Authenticated raw artifact access for renderers that cannot consume a JSON
/// data URL (compound web apps, large media, PDFs, and browser-native formats).
/// The path is still workspace-confined and the response never exposes a
/// filesystem path outside the requested relative name.
async fn read_file_raw(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::response::IntoResponse;
    let Some(path) = resolve_confined_file(&state, &q.path) else {
        return (StatusCode::FORBIDDEN, "path outside workspace").into_response();
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::NOT_FOUND, "file not found").into_response(),
    };
    let mime = raw_mime_for(&path);
    let disposition = if mime.starts_with("text/")
        || mime == "image/svg+xml"
        || mime == "application/pdf"
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
    {
        "inline"
    } else {
        "attachment"
    };
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let headers = [
        (axum::http::header::CONTENT_TYPE, mime),
        (
            axum::http::header::CONTENT_DISPOSITION,
            &format!("{disposition}; filename=\"{filename}\""),
        ),
        (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (axum::http::header::CACHE_CONTROL, "no-store"),
    ];
    (headers, Body::from(bytes)).into_response()
}

/// Serve a workspace-confined artifact through a stable path so compound HTML
/// previews can resolve relative stylesheets, scripts, images, and imports.
/// The response is still sandboxed by CSP; it is never a general static-file
/// server.
async fn preview_file(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::response::IntoResponse;
    let Some(path) = resolve_confined_file(&state, &path) else {
        return (StatusCode::FORBIDDEN, "path outside workspace").into_response();
    };
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::NOT_FOUND, "file not found").into_response(),
    };
    let headers = [
        (axum::http::header::CONTENT_TYPE, raw_mime_for(&path)),
        (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (axum::http::header::CACHE_CONTROL, "no-store"),
        (
            axum::http::header::CONTENT_SECURITY_POLICY,
            "sandbox allow-scripts; default-src 'self'; object-src 'none'; connect-src 'none'; base-uri 'self'",
        ),
    ];
    (headers, Body::from(bytes)).into_response()
}

fn raw_mime_for(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "md" => "text/markdown; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

#[derive(serde::Deserialize)]
struct WriteBody {
    path: String,
    content: String,
}

async fn write_file(State(state): State<AppState>, Json(body): Json<WriteBody>) -> StatusCode {
    let Some(path) = resolve_confined_file(&state, &body.path) else {
        return StatusCode::FORBIDDEN;
    };
    // Refuse to overwrite a file this endpoint could never have rendered
    // faithfully: saving text over an image or binary destroys it.
    if let Ok(existing) = tokio::fs::read(&path).await
        && !vak_core::files::classify(&path, &existing).editable()
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE;
    }
    if let Some(parent) = path.parent()
        && tokio::fs::create_dir_all(parent).await.is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    match tokio::fs::write(&path, body.content.as_bytes()).await {
        Ok(()) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn sandbox_records_path(state: &AppState) -> std::path::PathBuf {
    state
        .core
        .cwd()
        .join(".vak")
        .join("sandbox")
        .join("records.jsonl")
}

fn session_sandbox_events_path(state: &AppState, session_id: &str) -> std::path::PathBuf {
    state
        .core
        .sessions_home()
        .join("sandbox")
        .join("executions")
        .join(format!("{session_id}.jsonl"))
}

async fn session_sandbox_executions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = session_sandbox_events_path(&state, &id);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": error.to_string()})),
            )
                .into_response();
        }
    };
    let events = text
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "session_id": id, "events": events })).into_response()
}

fn append_session_sandbox_event(home: &std::path::Path, session_id: &str, event: &AgentEvent) {
    let AgentEvent::Sandbox(sandbox) = event else {
        return;
    };
    let path = home
        .join("sandbox")
        .join("executions")
        .join(format!("{session_id}.jsonl"));
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let Ok(line) = serde_json::to_string(sandbox) else {
        return;
    };
    use std::io::Write;
    let _ = writeln!(file, "{line}");
}

async fn list_sandbox_records(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = sandbox_records_path(&state);
    match vak_sandbox::load_records(&path) {
        Ok(records) => Json(serde_json::json!({ "records": records })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn append_sandbox_record(
    State(state): State<AppState>,
    Json(record): Json<vak_sandbox::DurableRecord>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = sandbox_records_path(&state);
    match vak_sandbox::append_record(&path, &record) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "accepted": true })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct SandboxCandidateBody {
    candidate_id: String,
    source: String,
    #[serde(default)]
    destination: String,
}

async fn export_sandbox_candidate(
    State(state): State<AppState>,
    Json(body): Json<SandboxCandidateBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(source) = confined_path(state.core.cwd(), &body.source) else {
        return (StatusCode::FORBIDDEN, "candidate source outside workspace").into_response();
    };
    let destination_text = if body.destination.trim().is_empty() {
        ".".to_string()
    } else {
        body.destination
    };
    let Some(destination) = confined_path(state.core.cwd(), &destination_text) else {
        return (
            StatusCode::FORBIDDEN,
            "candidate destination outside workspace",
        )
            .into_response();
    };
    match vak_sandbox::candidate_manifest(&body.candidate_id, &source, &destination) {
        Ok(candidate) => Json(candidate).into_response(),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct SandboxPromotionBody {
    candidate: vak_sandbox::CandidateManifest,
    #[serde(default)]
    record_id: Option<String>,
}

async fn promote_sandbox_candidate(
    State(state): State<AppState>,
    Json(body): Json<SandboxPromotionBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Compare canonical paths: macOS temporary directories can be addressed
    // through `/var` or `/private/var`, which are the same workspace but do
    // not satisfy lexical `starts_with` checks.
    let workspace =
        std::fs::canonicalize(state.core.cwd()).unwrap_or_else(|_| state.core.cwd().to_path_buf());
    let source_root = std::fs::canonicalize(&body.candidate.source_root)
        .unwrap_or_else(|_| body.candidate.source_root.clone());
    let destination_root = std::fs::canonicalize(&body.candidate.destination_root)
        .unwrap_or_else(|_| body.candidate.destination_root.clone());
    let source_ok = source_root.starts_with(&workspace);
    let destination_ok = destination_root == workspace;
    if !source_ok || !destination_ok {
        return (
            StatusCode::FORBIDDEN,
            "candidate roots must remain inside the current workspace",
        )
            .into_response();
    }
    let receipt = match vak_sandbox::promote(&body.candidate) {
        Ok(receipt) => receipt,
        Err(error) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response();
        }
    };
    let record = vak_sandbox::DurableRecord::Promotion(vak_sandbox::PromotionRecord {
        record_id: body
            .record_id
            .unwrap_or_else(|| format!("promotion-{}", receipt.candidate_id)),
        candidate_id: receipt.candidate_id.clone(),
        receipt: receipt.clone(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    });
    if let Err(error) = vak_sandbox::append_record(&sandbox_records_path(&state), &record) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    (StatusCode::OK, Json(receipt)).into_response()
}

#[derive(serde::Deserialize)]
struct ModeBody {
    mode: String,
    #[serde(default)]
    agent: Option<String>,
}

/// Accepts every spelling clients use: config kebab-case (`workspace-write`)
/// and the Debug format surfaced by `/health` + `/config` (`WorkspaceWrite`).
pub(crate) fn parse_mode(raw: &str) -> Option<vak_config::PermissionMode> {
    vak_config::PermissionMode::deserialize_str(raw).or(match raw {
        "ReadOnly" => Some(vak_config::PermissionMode::ReadOnly),
        "WorkspaceWrite" => Some(vak_config::PermissionMode::WorkspaceWrite),
        "FullAccess" => Some(vak_config::PermissionMode::FullAccess),
        _ => None,
    })
}

pub(crate) fn parse_approval_mode(raw: &str) -> Option<vak_config::ApprovalMode> {
    vak_config::ApprovalMode::parse(raw).or(match raw {
        "Ask" => Some(vak_config::ApprovalMode::Ask),
        "ApproveSafe" => Some(vak_config::ApprovalMode::ApproveSafe),
        "AutoApprove" => Some(vak_config::ApprovalMode::AutoApprove),
        _ => None,
    })
}

async fn set_permission_mode(
    State(state): State<AppState>,
    Json(body): Json<ModeBody>,
) -> StatusCode {
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(_) => return StatusCode::NOT_FOUND,
    };
    match parse_mode(&body.mode) {
        Some(mode) => {
            let old = core.effective_permission_mode();
            if vak_config::persist_project_preferences(
                core.cwd(),
                None,
                None,
                None,
                Some(mode),
                None,
                None,
            )
            .is_err()
            {
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
            apply_permission_mode(&core, &state, mode, true);
            if old != mode {
                vak_core::security_events::record(
                    &core.sessions_home(),
                    vak_core::security_events::EventKind::ConfigChange,
                    "permission_mode_changed",
                    &format!("{old:?} -> {mode:?}"),
                    None,
                );
                state
                    .hub
                    .emit_config_changed("permission_mode", &format!("{mode:?}"));
            }
            StatusCode::OK
        }
        None => StatusCode::BAD_REQUEST,
    }
}

// ---- Gateway approval policy ----------------------------------------------

/// `GET /gateway/approvals` — the live policy plus the chats that could
/// answer a gate.
///
/// The candidate list is what makes this usable: `approver` is a
/// `<surface>:<chat>` target, and an operator has no way to type one
/// correctly from memory. Every allowed allowlist entry is offered,
/// normalized to the two-part shape `deliver_to` uses, because a
/// bot-scoped three-part key is an allowlist identity and not a delivery
/// address.
async fn get_gateway_approvals(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "mode": state.gateway.approvals_mode(),
        "approver": state.gateway.approver_target(),
        "timeout_secs": state.gateway.approval_timeout().as_secs(),
        "enabled": state.gateway.enabled,
        "forwarding": state.gateway.forward_mode(),
        "candidates": state.gateway.approver_candidates(),
    }))
}

#[derive(serde::Deserialize)]
struct GatewayApprovalsBody {
    /// "deny" or "forward".
    mode: String,
    /// `<surface>:<chat>`. Required for "forward"; ignored for "deny".
    #[serde(default)]
    approver: Option<String>,
    /// Seconds a forwarded gate waits before failing closed. Minimum 5,
    /// matching `vak_config`'s own floor.
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    scope: Option<ConfigScope>,
}

/// `PUT /gateway/approvals` — set the policy, live and on disk.
///
/// Validation happens here rather than being left to the config loader's
/// fallback: the loader's job is to make a bad file safe (it degrades
/// `forward` with no target to `deny` and warns), but an operator pressing
/// a button deserves a refusal that names the problem instead of a success
/// followed by a silently different setting.
async fn put_gateway_approvals(
    State(state): State<AppState>,
    Json(body): Json<GatewayApprovalsBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let bad = |msg: &str| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": msg })),
        )
            .into_response()
    };
    let mode = body.mode.trim();
    if !matches!(mode, "deny" | "forward") {
        return bad("mode must be \"deny\" or \"forward\"");
    }
    let approver = body
        .approver
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    if mode == "forward" {
        match approver {
            None => {
                return bad(
                    "forwarding needs an approver chat — the gate is announced there and \
                     answered with \"yes\" or \"no\"",
                );
            }
            Some(target) if !target.contains(':') => {
                return bad("approver must be \"<surface>:<chat>\", e.g. \"telegram:12345678\"");
            }
            Some(_) => {}
        }
    }
    if let Some(secs) = body.timeout_secs
        && !(5..=86_400).contains(&secs)
    {
        return bad("timeout must be between 5 and 86400 seconds");
    }

    let scope = body.scope.unwrap_or(ConfigScope::Workspace);
    let path = match scope.config_path(&state.core) {
        Ok(path) => path,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    // Persist first. A policy that applied live but never reached disk is
    // exactly the "I set it and it reverted" failure this endpoint exists
    // to end, and it is worse than one that failed loudly.
    if let Err(error) = vak_config::persist_gateway_approvals(
        path,
        Some(mode),
        // "deny" clears the target rather than leaving a stale one behind
        // that a later "forward" would silently reuse.
        Some(if mode == "forward" { approver } else { None }),
        body.timeout_secs,
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }

    let previous = state.gateway.approvals_mode();
    let installed = state
        .gateway
        .set_approval_policy(crate::gateway::ApprovalPolicy {
            approvals: mode.to_string(),
            approver: approver.map(str::to_string),
            timeout: std::time::Duration::from_secs(
                body.timeout_secs
                    .unwrap_or_else(|| state.gateway.approval_timeout().as_secs()),
            ),
        });

    if previous != installed.approvals || installed.approvals == "forward" {
        vak_core::security_events::record(
            &state.core.sessions_home(),
            vak_core::security_events::EventKind::ConfigChange,
            "gateway_approvals_changed",
            &format!(
                "{previous} -> {} approver={} scope={}",
                installed.approvals,
                installed.approver.as_deref().unwrap_or("<none>"),
                scope.label()
            ),
            None,
        );
        state
            .hub
            .emit_config_changed("gateway_approvals", &installed.approvals);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "mode": installed.approvals,
            "approver": installed.approver,
            "timeout_secs": installed.timeout.as_secs(),
            "forwarding": state.gateway.forward_mode(),
            // The gateway being off makes a forward policy inert. Say so
            // rather than reporting a grant the next inbound turn will not
            // honour, which is the same class of lie `reach` exists to end.
            "gateway_enabled": state.gateway.enabled,
        })),
    )
        .into_response()
}

// ---- Permission rules ------------------------------------------------------

/// `GET /config/permissions` — the effective rule lists the engine
/// evaluates, plus the selected layer's own, so a reader can tell an
/// inherited rule from one this scope set.
async fn get_permission_rules(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<OptionalScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let scope = q.scope.unwrap_or(ConfigScope::Workspace);
    // Recomputed from this resolved Core's own effective rules on every
    // request rather than relying on any process-pinned cache — a Core
    // resolved for a non-default Agent must not read the default Agent's
    // runtime-pinned overrides, and vice versa.
    let (allow, ask, deny) = core.effective_permission_rules();
    let layer = match scope
        .config_path(&core)
        .and_then(|path| read_config_layer(path.as_path()))
    {
        Ok(layer) => layer,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "scope": scope.label(),
            "effective": { "allow": allow, "ask": ask, "deny": deny },
            "layer": {
                "allow": layer.allow,
                "ask": layer.ask,
                "deny": layer.deny,
            },
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct PermissionRulesBody {
    /// Absent leaves that list alone; present replaces it wholesale.
    #[serde(default)]
    allow: Option<Vec<String>>,
    #[serde(default)]
    ask: Option<Vec<String>>,
    #[serde(default)]
    deny: Option<Vec<String>>,
    #[serde(default)]
    scope: Option<ConfigScope>,
    #[serde(default)]
    agent: Option<String>,
}

/// `PUT /config/permissions` — replace rule lists in one layer.
///
/// Every spec is parsed through the real `vak_permission::Rule::parse`
/// before anything is written, and the whole request is rejected if any
/// one of them fails. A half-applied rule set is a permission decision
/// nobody chose.
async fn put_permission_rules(
    State(state): State<AppState>,
    Json(body): Json<PermissionRulesBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    for (list_name, list) in [
        ("allow", &body.allow),
        ("ask", &body.ask),
        ("deny", &body.deny),
    ] {
        let Some(list) = list else { continue };
        for spec in list {
            if let Err(error) = vak_permission::Rule::parse(spec) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("{list_name}: {error}"),
                        "rule": spec,
                    })),
                )
                    .into_response();
            }
        }
    }
    let scope = body.scope.unwrap_or(ConfigScope::Workspace);
    let path = match scope.config_path(&core) {
        Ok(path) => path,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    if let Err(error) = vak_config::persist_permission_rules(
        path,
        body.allow.as_deref(),
        body.ask.as_deref(),
        body.deny.as_deref(),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    // Re-merge both layers against this resolved Agent's own Core. Written
    // rules are re-read from disk on every subsequent request through
    // `resolve_scoped_core` (option (b): no pinned-cache optimization for a
    // non-default Agent, since a freshly re-resolved Core would lose the
    // pin anyway) — pinning onto the runtime override is kept only for the
    // "vak" default/registered-session Core, where callers still read
    // `effective_permission_rules()` off the very same long-lived instance
    // within this same process.
    let merged = match vak_config::load_with_trust(core.cwd(), core.project_config_trusted()) {
        Ok(merged) => merged,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error.to_string() })),
            )
                .into_response();
        }
    };
    // Reject a set that the engine cannot compile, and do it BEFORE
    // pinning: individually valid rules are all that was checked above,
    // and the merge brings in the other layer's rules too.
    let (allow, ask, deny) = (
        merged.allow.clone(),
        merged.ask.clone(),
        merged.deny.clone(),
    );
    core.apply_persisted_permission_rules(allow.clone(), ask.clone(), deny.clone());
    if let Err(error) = core.build_permission_engine(&[]) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": format!("merged rule set does not compile: {error}")
            })),
        )
            .into_response();
    }
    vak_core::security_events::record(
        &core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "permission_rules_changed",
        &format!(
            "scope={} allow={} ask={} deny={}",
            scope.label(),
            allow.len(),
            ask.len(),
            deny.len()
        ),
        None,
    );
    state
        .hub
        .emit_config_changed("permission_rules", scope.label());
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "scope": scope.label(),
            "effective": { "allow": allow, "ask": ask, "deny": deny },
        })),
    )
        .into_response()
}

/// `core` is the Agent-scoped Core the mode is actually read from and
/// written to (so a PATCH scoped to a non-default Agent lands on that
/// Agent's own Core, not the process's default workspace); `state` is used
/// only for the process-wide safety fallout below — invalidating pooled
/// channel Cores and cancelling every live session — which is deliberately
/// global: a narrower permission ceiling must not leave an already-running
/// session anywhere holding a wider one.
fn apply_permission_mode(
    core: &vak_core::Core,
    state: &AppState,
    mode: vak_config::PermissionMode,
    persisted: bool,
) {
    if core.effective_permission_mode() == mode {
        return;
    }
    if persisted {
        core.apply_persisted_permission_mode(mode);
    } else {
        core.set_permission_mode(mode);
    }
    // Warm per-channel instances captured their ceiling when they were
    // built. Discard them so the next inbound message resolves a fresh one;
    // without this a narrowed mode reached chats only when their idle
    // window expired, which is up to half an hour of running under a
    // ceiling that had already been revoked.
    let dropped = state.gateway.core_pool.invalidate_pooled();
    if dropped > 0 {
        vak_core::security_events::record(
            &state.core.sessions_home(),
            vak_core::security_events::EventKind::ConfigChange,
            "core_pool_invalidated",
            &format!("permission_mode={mode:?} dropped={dropped}"),
            None,
        );
    }
    let handles: Vec<Arc<SessionHandle>> = state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .cloned()
        .collect();
    for handle in handles {
        handle
            .cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel();
        handle
            .side_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel();
        deny_pending_approvals(&handle);
        let _ = state.gateway.deny_pending_for_session(&handle.id);
    }
}

fn refresh_control_plane(state: &AppState) {
    let old_mode = state.core.effective_permission_mode();
    if let Ok(mode) = state.core.refresh_persisted_preferences()
        && !state.core.permission_mode_runtime_pinned()
        && mode != old_mode
    {
        apply_permission_mode(&state.core, state, mode, true);
        state
            .hub
            .emit_config_changed("permission_mode_refreshed", &format!("{mode:?}"));
    }
}

/// Picker data for provider/model UIs. Reports WHICH env var authenticates
/// each provider and whether it resolves right now — never the value.
async fn list_providers(State(state): State<AppState>) -> Json<serde_json::Value> {
    refresh_control_plane(&state);
    let route = state.core.effective_route();
    let mut providers = Vec::new();
    for name in state.core.provider_names() {
        let requires_key = name != "ollama";
        let configured = state.core.provider_configured(&name);
        let credential_ids = state.core.provider_credential_ids(&name);
        let (project_key, user_key, process_key) = state.core.provider_key_sources(&name);
        providers.push(serde_json::json!({
            "name": name,
            "env_var": Core::provider_env_var(&name),
            "pool_env_var": pool_env_var(&name),
            "pool_size": credential_ids.len(),
            "credential_ids": credential_ids,
            "requires_key": requires_key,
            "configured": configured,
            "key_in_project": project_key,
            "key_in_user": user_key,
            "key_in_process": process_key,
            "key_source": if project_key { "project" } else if user_key { "user" } else if process_key { "process" } else { "none" },
        }));
    }
    Json(serde_json::json!({
        "current": route.provider,
        "current_model": route.model,
        "current_provider_source": route.provider_source,
        "current_model_source": route.model_source,
        "route_revision": route.revision,
        "current_configured": state.core.provider_configured(&state.core.effective_provider()),
        "providers": providers,
    }))
}

fn pool_env_var(provider: &str) -> Option<&'static str> {
    Core::provider_pool_env_var(provider)
}

#[derive(serde::Deserialize)]
struct ProviderRef {
    provider: String,
    #[serde(default)]
    scope: Option<ConfigScope>,
}

/// Revoke a provider key. Reports when the variable is still set in the
/// real environment, since that keeps the provider authenticated and no
/// app-level action can change it.
async fn delete_provider_key(
    State(state): State<AppState>,
    Json(body): Json<ProviderRef>,
) -> axum::response::Response {
    let scope = body.scope.unwrap_or(ConfigScope::User);
    match state
        .core
        .remove_provider_key_scoped(&body.provider, scope.is_workspace())
    {
        Ok(removed) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "provider_key_removed",
                &format!(
                    "provider={} scope={} shadowed={}",
                    body.provider,
                    scope.label(),
                    removed.shadowed_by_env
                ),
                None,
            );
            state
                .hub
                .emit_config_changed("provider_key_removed", &body.provider);
            Json(serde_json::json!({
                "provider": body.provider,
                "env_var": removed.env_var,
                "configured": removed.shadowed_by_env,
                "shadowed_by_env": removed.shadowed_by_env,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Live model list for one provider, straight from its API using the key
/// currently configured for it. Reports the failure reason rather than
/// substituting a stale hard-coded list.
async fn discover_models(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    if !Core::provider_known(&name) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("unknown provider '{name}'") })),
        )
            .into_response();
    }
    match state.core.discover_models(&name).await {
        Ok(models) => {
            if name == "bedrock" {
                match state.core.bedrock_model_availability(&models).await {
                    Ok(availability) => Json(serde_json::json!({ "provider": name, "models": models, "availability": availability })).into_response(),
                    Err(e) => Json(serde_json::json!({ "provider": name, "models": models, "availability_error": e.to_string() })).into_response(),
                }
            } else {
                Json(serde_json::json!({ "provider": name, "models": models })).into_response()
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "provider": name, "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn model_availability(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    if name != "bedrock" {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error":"availability is only supported for bedrock"})),
        )
            .into_response();
    }
    let models = match state.core.discover_models("bedrock").await {
        Ok(models) => models,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"provider":name,"error":e.to_string()})),
            )
                .into_response();
        }
    };
    match state.core.bedrock_model_availability(&models).await {
        Ok(availability) => {
            Json(serde_json::json!({"provider":name,"models":availability})).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"provider":name,"error":e.to_string()})),
        )
            .into_response(),
    }
}

/// Read provider-published account metadata without returning credentials.
async fn provider_status(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> axum::response::Response {
    if !Core::provider_known(&name) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("unknown provider '{name}'") })),
        )
            .into_response();
    }
    match state.core.provider_status(&name).await {
        Ok(status) => Json(status).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "provider": name, "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ProviderKeyBody {
    provider: String,
    key: String,
    #[serde(default)]
    scope: Option<ConfigScope>,
}

/// Persists a credential to the Shared `~/vak-home/.env` secret store and makes it
/// effective immediately. The key is accepted once and never echoed back.
async fn put_provider_key(
    State(state): State<AppState>,
    Json(body): Json<ProviderKeyBody>,
) -> axum::response::Response {
    let scope = body.scope.unwrap_or(ConfigScope::User);
    match state
        .core
        .set_provider_key_scoped(&body.provider, &body.key, scope.is_workspace())
    {
        Ok(env_var) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "provider_key_set",
                &format!("provider={} scope={}", body.provider, scope.label()),
                None,
            );
            state
                .hub
                .emit_config_changed("provider_key_set", &body.provider);
            Json(serde_json::json!({
                "provider": body.provider,
                "env_var": env_var,
                "configured": true,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---- Distributed event bus (vak-bus, docs/design/53) -------------------

#[derive(serde::Deserialize)]
struct BusConfigBody {
    nats_url: Option<String>,
    /// NATS credentials JWT. Stored in the workspace .env file and read
    /// on next server start. Never returned by GET.
    #[serde(default)]
    nats_credentials_jwt: Option<String>,
    /// NATS nkey seed. Stored in the workspace .env file and read
    /// on next server start. Never returned by GET.
    #[serde(default)]
    nats_nkey_seed: Option<String>,
    /// Name of the env var holding the workspace encryption secret.
    #[serde(default)]
    workspace_secret_env: Option<String>,
}

/// GET /config/bus — bus status and configuration (non-secret fields only).
async fn get_bus_config(State(state): State<AppState>) -> axum::response::Response {
    let cfg = state.core.config().server.bus.clone();
    let status = state.hub.bus_status();
    Json(serde_json::json!({
        "nats_url": cfg.nats_url,
        "encrypted": cfg.workspace_secret.is_some(),
        "runtime": status,
    }))
    .into_response()
}

/// PUT /config/bus — store NATS credentials in the workspace .env file.
/// Takes effect on the next server restart (the ServerBus is initialized
/// at startup; runtime reconnection is a future enhancement).
/// Credentials are never returned by GET /config/bus once set.
async fn put_bus_config(
    State(state): State<AppState>,
    Json(body): Json<BusConfigBody>,
) -> axum::response::Response {
    if body.nats_url.is_none()
        && body.nats_credentials_jwt.is_none()
        && body.nats_nkey_seed.is_none()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "at least one of nats_url, nats_credentials_jwt, or nats_nkey_seed must be provided"
            })),
        )
            .into_response();
    }

    let env_updates: Vec<(String, String)> = [
        body.nats_url.map(|v| ("VAK_BUS_NATS_URL".to_string(), v)),
        body.nats_credentials_jwt
            .map(|v| ("VAK_BUS_NATS_CREDENTIALS_JWT".to_string(), v)),
        body.nats_nkey_seed
            .map(|v| ("VAK_BUS_NATS_NKEY_SEED".to_string(), v)),
        body.workspace_secret_env
            .map(|v| ("VAK_BUS_WORKSPACE_SECRET_ENV".to_string(), v)),
    ]
    .into_iter()
    .flatten()
    .collect();

    let env_path = state.core.cwd().join(".vak/env");
    let _ = std::fs::create_dir_all(state.core.cwd().join(".vak"));
    let existing = std::fs::read_to_string(&env_path).unwrap_or_default();
    let mut lines: Vec<String> = existing.lines().map(String::from).collect();

    for (key, val) in &env_updates {
        let pattern = format!("{key}=");
        if let Some(pos) = lines.iter().position(|l| l.starts_with(&pattern)) {
            lines[pos] = format!("{key}={val}");
        } else {
            lines.push(format!("{key}={val}"));
        }
    }

    let content = lines.join("\n") + "\n";
    if let Err(e) = std::fs::write(&env_path, &content) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("failed to write .vak/env: {e}") })),
        )
            .into_response();
    }
    for (key, _val) in &env_updates {
        state.hub.emit_config_changed("bus_credential_set", key);
    }

    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "bus_credential_set",
        &format!(
            "keys={}",
            env_updates
                .iter()
                .map(|(k, _)| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        None,
    );

    Json(serde_json::json!({
        "configured": true,
        "env_vars": env_updates.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
        "takes_effect": "on next server restart",
    }))
    .into_response()
}

/// DELETE /config/bus/credentials — remove NATS credentials from .vak/env.
async fn delete_bus_config(State(state): State<AppState>) -> axum::response::Response {
    let env_path = state.core.cwd().join(".vak/env");
    let existing = std::fs::read_to_string(&env_path).unwrap_or_default();
    let keys = [
        "VAK_BUS_NATS_URL",
        "VAK_BUS_NATS_CREDENTIALS_JWT",
        "VAK_BUS_NATS_NKEY_SEED",
        "VAK_BUS_WORKSPACE_SECRET_ENV",
    ];
    let filtered: Vec<String> = existing
        .lines()
        .filter(|line| !keys.iter().any(|k| line.starts_with(&format!("{k}="))))
        .map(String::from)
        .collect();
    let content = if filtered.is_empty() {
        String::new()
    } else {
        filtered.join("\n") + "\n"
    };
    let _ = std::fs::write(&env_path, &content);
    state
        .hub
        .emit_config_changed("bus_credential_cleared", "all");
    Json(serde_json::json!({ "configured": false })).into_response()
}

#[derive(serde::Deserialize)]
struct TelegramTokenBody {
    token: String,
}

// ---- Multi-bot (docs/design/34, multi-bot-per-channel) --------------------

fn bot_env_var(id: &str) -> String {
    // A dedicated env var per bot id, distinct from the legacy per-surface
    // slots (`TELEGRAM_BOT_TOKEN` etc.) so a second bot never overwrites
    // the first one's token in the shared `.env` file.
    format!("BOT_TOKEN__{}", id.to_uppercase().replace('-', "_"))
}

async fn list_bots(State(state): State<AppState>) -> Json<serde_json::Value> {
    // `token_configured` rather than the token: a bot's credential is
    // never returned once set (invariant 23). Every surface needs to know
    // *whether* a bot can authenticate — that is what "is this bot ready?"
    // means — without any of them being able to read the secret.
    let bots: Vec<serde_json::Value> = state
        .gateway
        .bots_snapshot()
        .into_iter()
        .map(|bot| {
            let configured = vak_config::get_var(&bot.token_env).is_some();
            let mut value = serde_json::to_value(&bot).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(map) = value.as_object_mut() {
                map.insert("token_configured".into(), serde_json::json!(configured));
            }
            value
        })
        .collect();
    Json(serde_json::json!({ "bots": bots }))
}

#[derive(serde::Deserialize)]
struct CreateBotBody {
    id: String,
    surface: String,
    label: String,
    #[serde(default)]
    agent_id: Option<String>,
}

async fn create_bot(
    State(state): State<AppState>,
    Json(body): Json<CreateBotBody>,
) -> axum::response::Response {
    let id = body.id.trim();
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "bot id must be non-empty alphanumeric/hyphen" })),
        )
            .into_response();
    }
    if !vak_core::Core::is_surface(&body.surface) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({ "error": format!("unknown chat surface '{}'", body.surface) }),
            ),
        )
            .into_response();
    }
    if state.gateway.bot_get(id).is_some() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": format!("bot '{id}' already exists") })),
        )
            .into_response();
    }
    let bot = crate::gateway::Bot {
        id: id.to_string(),
        surface: body.surface.clone(),
        label: if body.label.trim().is_empty() {
            id.to_string()
        } else {
            body.label.trim().to_string()
        },
        token_env: bot_env_var(id),
        agent_id: body.agent_id.and_then(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() || trimmed == "vak" {
                None
            } else {
                Some(trimmed.to_string())
            }
        }),
        ..Default::default()
    };
    state.gateway.bot_upsert(&state.core, bot.clone());
    state.hub.emit_config_changed("bot_created", id);
    // Creating a bot writes `bots.json`. It does not register an OS
    // service: activation is a separate, explicit act (see
    // `service_control`), so a routine edit never mutates the machine's
    // service manager behind the operator's back.
    Json(serde_json::json!({ "bot": bot, "activation_required": true })).into_response()
}

#[derive(serde::Deserialize, Default)]
struct UpdateBotBody {
    #[serde(default)]
    label: Option<String>,
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    agent_id: Option<Option<String>>,
    #[serde(default)]
    policy: Option<vak_config::ChannelPolicy>,
    /// Absent (field simply not sent) leaves the current mode alone;
    /// explicit `null` clears it back to "follows the project"; a string
    /// sets it. See `gateway::deserialize_present` for why the plain
    /// `Option<Option<T>>` shape needs the custom deserializer to make
    /// that distinction actually work.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    permission_mode: Option<Option<String>>,
    /// See `permission_mode` above.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    route: Option<Option<crate::gateway::AllowlistRoute>>,
    /// See `permission_mode` above.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    workspace: Option<Option<String>>,
    /// See `permission_mode` above: absent leaves the bot's voice alone,
    /// explicit `null` clears it back to inherit, a `VoiceConfig` object
    /// sets it.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    voice: Option<Option<vak_config::VoiceConfig>>,
    /// This bot's prompt tier (docs/design/45-prompt-layers.md). Absent
    /// leaves it alone; an object replaces it. Its `identity` block is also
    /// what drives this bot's spoken persona, so the two cannot drift.
    #[serde(default)]
    prompt: Option<vak_core::prompts::LayerContent>,
}

async fn update_bot(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<UpdateBotBody>,
) -> axum::response::Response {
    let Some(mut bot) = state.gateway.bot_get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("no bot '{id}'") })),
        )
            .into_response();
    };
    if let Some(label) = body.label {
        bot.label = label;
    }
    if let Some(agent_id) = body.agent_id {
        bot.agent_id = match agent_id {
            None => None,
            Some(raw) if raw.trim().is_empty() || raw.trim() == "vak" => None,
            Some(raw) => Some(raw.trim().to_string()),
        };
    }
    if let Some(policy) = body.policy {
        bot.policy = policy;
    }
    if let Some(raw) = body.permission_mode {
        match raw {
            None => bot.permission_mode = None,
            Some(raw) if raw.trim().is_empty() => bot.permission_mode = None,
            Some(raw) => match crate::parse_mode(raw.trim()) {
                Some(mode) => bot.permission_mode = Some(mode),
                None => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(
                            serde_json::json!({ "error": format!("unknown permission_mode '{raw}'") }),
                        ),
                    )
                        .into_response();
                }
            },
        }
    }
    if let Some(route) = body.route {
        bot.route = route;
    }
    if let Some(ws) = body.workspace {
        bot.workspace = match ws {
            None => None,
            Some(ws) if ws.trim().is_empty() => None,
            Some(ws) => Some(PathBuf::from(ws.trim())),
        };
    }
    if let Some(voice) = body.voice {
        bot.voice = voice;
    }
    if let Some(prompt) = body.prompt {
        bot.prompt = prompt;
    }
    state.gateway.bot_upsert(&state.core, bot.clone());
    state.hub.emit_config_changed("bot_updated", &id);
    Json(serde_json::json!({ "bot": bot })).into_response()
}

async fn delete_bot(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> StatusCode {
    if state.gateway.bot_remove(&state.core, &id) {
        // No service-manager call here. The bridge watches its own
        // credential (`surfaces::CredentialWatch`) and stops when it goes
        // away, so removal takes effect without a handler orchestrating
        // anything. Its unit is cleaned up at the next activation.
        state.hub.emit_config_changed("bot_deleted", &id);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

async fn put_bot_id_token(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<TelegramTokenBody>,
) -> axum::response::Response {
    let Some(bot) = state.gateway.bot_get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("no bot '{id}'") })),
        )
            .into_response();
    };
    match state.core.set_bot_token(&bot.token_env, &body.token) {
        Ok(env_var) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "bot_token_set",
                &id,
                None,
            );
            state.hub.emit_config_changed("bot_token_set", &id);
            // Storing a credential is configuration, not activation: it
            // registers no unit and restarts nothing. The operator
            // activates when they mean to, and `activation_required` is
            // how a surface knows to say so.
            Json(serde_json::json!({
                "id": id,
                "env_var": env_var,
                "configured": true,
                "activation_required": true,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn delete_bot_id_token(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> axum::response::Response {
    let Some(bot) = state.gateway.bot_get(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("no bot '{id}'") })),
        )
            .into_response();
    };
    match state.core.remove_bot_token(&bot.token_env) {
        Ok(removed) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "bot_token_removed",
                &format!("id={id} shadowed={}", removed.shadowed_by_env),
                None,
            );
            state.hub.emit_config_changed("bot_token_removed", &id);
            // Nothing to orchestrate: the bridge re-reads its credential
            // each poll cycle and stops using a cleared one on its own
            // (`surfaces::CredentialWatch`).
            Json(serde_json::json!({
                "id": id,
                "env_var": removed.env_var,
                "configured": removed.shadowed_by_env,
                "shadowed_by_env": removed.shadowed_by_env,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn get_config(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    refresh_control_plane(&state);
    let cfg = core.config();
    let work = core.effective_work();
    let route = core.effective_route();
    let permission_rules = core.effective_permission_rules();
    let project_path = vak_config::project_path(core.cwd());
    Json(serde_json::json!({
        "provider": route.provider,
        "model": route.model,
        "provider_source": route.provider_source,
        "model_source": route.model_source,
        "route_revision": route.revision,
        "max_tokens": cfg.max_tokens,
        "max_turns": core.effective_max_turns(),
        "intent_evidence_max_age_secs": cfg.intent.evidence_max_age_secs,
        "permission_mode": format!("{:?}", core.effective_permission_mode()),
        // How an `Ask` gets resolved, and what the rules say — both were
        // absent here, which is why the desktop app could set the permission
        // mode but had no way to show or change the approval behaviour, and
        // no way to show a rule at all.
        "approval_mode": core.effective_approval_mode().as_str(),
        "sandbox": core.effective_sandbox_name(),
        "permissions": {
            "allow": permission_rules.0,
            "ask": permission_rules.1,
            "deny": permission_rules.2,
        },
        "workers": core.effective_workers(),
        "max_retries": cfg.max_retries,
        "retry_base_backoff_ms": cfg.retry_base_backoff_ms,
        "request_timeout_secs": cfg.request_timeout_secs,
        "run_retry_attempts": cfg.run_retry_attempts,
        "run_retry_base_backoff_ms": cfg.run_retry_base_backoff_ms,
        "circuit_breaker_threshold": cfg.circuit_breaker_threshold,
        "circuit_breaker_cooldown_secs": cfg.circuit_breaker_cooldown_secs,
        "context_window": cfg.context_window,
        "theme": core.effective_theme(),
        "memory": {
            "search_enabled": core.effective_memory_search_enabled(),
            "write_enabled": core.effective_memory_write_enabled(),
            "reflection": core.effective_memory_reflection(),
            "skill_proposals": core.effective_memory_skill_proposals(),
        },
        "bell": cfg.ui.bell,
        "stop_policy": {
            "enabled": cfg.stop_policy.enabled,
            "marker_gate": cfg.stop_policy.marker_gate,
            "verify_gate": cfg.stop_policy.verify_gate,
            "max_blocks": cfg.stop_policy.max_blocks,
        },
        "work": {
            "enabled": work.enabled,
            "default_mode": work.default_mode,
            "max_items": work.max_items,
            "max_revisions": work.max_revisions,
            "max_parallel": work.max_parallel,
            "confirmation": work.confirmation,
        },
        "route": {
            "objective": cfg.route.objective,
            "fallback_models": cfg.route.fallback_models,
            "max_fallbacks": cfg.route.max_fallbacks,
            "quality_hints": cfg.route.quality_hints,
        },
        "integrations": {
            "mcp_servers": cfg.mcp.servers.keys().collect::<Vec<_>>(),
            "hooks": cfg.hooks.len(),
            "skills": core.skills().iter().map(|skill| skill.name.clone()).collect::<Vec<_>>(),
        },
        "capability_inheritance": {
            "mcp": core.effective_capability_inheritance().inherit_mcp,
            "hooks": core.effective_capability_inheritance().inherit_hooks,
            "skills": core.effective_capability_inheritance().inherit_skills,
            "commands": core.effective_capability_inheritance().inherit_commands,
            "plugins": core.effective_capability_inheritance().inherit_plugins,
        },
        // A surface is a transport, not a credential slot (invariant 23):
        // credentials belong to bots, and `GET /gateway/bots` reports them.
        "surfaces": vak_core::Core::SURFACES,
        "paths": {
            "project_config": project_path,
            "global_config": vak_config::global_path(),
            "sessions_home": core.sessions_home(),
            "cwd": core.cwd(),
        },
        "warnings": cfg.warnings,
    }))
    .into_response()
}

fn read_config_layer(path: &std::path::Path) -> Result<vak_config::FileConfig, String> {
    if !path.is_file() {
        return Ok(vak_config::FileConfig::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    toml::from_str(&raw).map_err(|error| error.to_string())
}

fn config_layer_response(
    core: &vak_core::Core,
    scope: ConfigScope,
) -> Result<serde_json::Value, String> {
    let path = scope.config_path(core)?;
    let layer = read_config_layer(&path)?;
    Ok(serde_json::json!({
        "scope": scope.label(),
        "path": path,
        "provider": layer.provider,
        "model": layer.model,
        "max_tokens": layer.max_tokens,
        "max_turns": layer.max_turns,
        "permission_mode": layer.permission_mode.map(|mode| format!("{mode:?}")),
        "approval_mode": layer.approval_mode.map(|mode| mode.as_str()),
        "profile": layer.profile,
        "workers": layer.workers,
        "theme": layer.ui.theme,
        "permissions": {
            "allow": layer.allow,
            "ask": layer.ask,
            "deny": layer.deny,
        },
        "memory": {
            "search_enabled": layer.memory.search_enabled,
            "write_enabled": layer.memory.write_enabled,
            "reflection": layer.memory.reflection,
            "skill_proposals": layer.memory.skill_proposals,
        },
        "work": {
            "enabled": layer.work.enabled,
            "default_mode": layer.work.default_mode,
            "max_items": layer.work.max_items,
            "max_revisions": layer.work.max_revisions,
            "max_parallel": layer.work.max_parallel,
            "confirmation": layer.work.confirmation,
        },
        "counts": {
            "mcp": layer.mcp.servers.len(),
            "hooks": layer.hooks.len(),
        },
        "capabilities": {
            "inherit_mcp": layer.capabilities.inherit_mcp,
            "inherit_hooks": layer.capabilities.inherit_hooks,
            "inherit_skills": layer.capabilities.inherit_skills,
            "inherit_commands": layer.capabilities.inherit_commands,
            "inherit_plugins": layer.capabilities.inherit_plugins,
        },
    }))
}

async fn get_global_config_layer(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    match config_layer_response(&core, ConfigScope::User) {
        Ok(layer) => Json(layer).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn get_workspace_config_layer(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    match config_layer_response(&core, ConfigScope::Workspace) {
        Ok(layer) => Json(layer).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct AgentNetworkCapabilityBody {
    workspace: String,
    enabled: bool,
    #[serde(default)]
    allowed_peers: Vec<String>,
    max_message_bytes: Option<usize>,
}

async fn agent_network_capability(
    State(state): State<AppState>,
    Json(body): Json<AgentNetworkCapabilityBody>,
) -> impl IntoResponse {
    let workspace = match std::path::Path::new(&body.workspace).canonicalize() {
        Ok(path) if path.is_dir() => path.display().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "workspace must be an existing directory"})),
            );
        }
    };
    let peers = match body
        .allowed_peers
        .into_iter()
        .map(|peer| {
            std::path::Path::new(&peer)
                .canonicalize()
                .ok()
                .filter(|path| path.is_dir())
                .map(|path| path.display().to_string())
        })
        .collect::<Option<Vec<_>>>()
    {
        Some(peers) => peers.into_iter().collect(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "every allowed peer must be an existing directory"}),
                ),
            );
        }
    };
    let policy = vak_core::agent_network::WorkspaceNetworkPolicy {
        enabled: body.enabled,
        allowed_peers: peers,
        max_message_bytes: body
            .max_message_bytes
            .unwrap_or(256 * 1024)
            .clamp(1, 256 * 1024),
    };
    let broker = state.core.agent_network_broker();
    let capability = broker.register(workspace.clone(), policy);
    if let Err(error) = broker.save_policies(
        &state
            .core
            .sessions_home()
            .join("agent-network/policies.json"),
    ) {
        let _ = broker.revoke(&capability);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error})),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "workspace": workspace,
            "capability": capability.token(),
            "expires_in_seconds": 900
        })),
    )
}

#[derive(Debug, serde::Deserialize)]
struct AgentNetworkSendBody {
    sender_workspace: String,
    capability: String,
    destination_workspace: String,
    body: String,
}

async fn agent_network_send(
    State(state): State<AppState>,
    Json(body): Json<AgentNetworkSendBody>,
) -> impl IntoResponse {
    use base64::Engine as _;
    let Ok(payload) = base64::engine::general_purpose::STANDARD.decode(body.body.trim()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "body must be standard base64"})),
        );
    };
    let sender = vak_core::agent_network::BrokerCapability::from_parts(
        body.sender_workspace,
        body.capability,
    );
    match state
        .core
        .agent_network_broker()
        .send_to(&sender, &body.destination_workspace, payload)
    {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"accepted": true})),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": error})),
        ),
    }
}

#[derive(Debug, serde::Deserialize)]
struct AgentNetworkReceiveQuery {
    workspace: String,
    capability: String,
}

async fn agent_network_receive(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<AgentNetworkReceiveQuery>,
) -> impl IntoResponse {
    use base64::Engine as _;
    let receiver =
        vak_core::agent_network::BrokerCapability::from_parts(query.workspace, query.capability);
    match state.core.agent_network_broker().receive(&receiver) {
        Ok(Some(message)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "from_workspace": message.from_workspace,
                "to_workspace": message.to_workspace,
                "body": base64::engine::general_purpose::STANDARD.encode(message.body)
            })),
        ),
        Ok(None) => (
            StatusCode::NO_CONTENT,
            Json(serde_json::json!({"message": null})),
        ),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": error})),
        ),
    }
}

#[derive(serde::Deserialize, Default)]
struct ConfigPatch {
    provider: Option<String>,
    model: Option<String>,
    max_turns: Option<usize>,
    permission_mode: Option<String>,
    approval_mode: Option<String>,
    theme: Option<String>,
    #[serde(default)]
    voice_enabled: Option<bool>,
    #[serde(default)]
    voice_max_session_secs: Option<u64>,
    #[serde(default)]
    voice_max_concurrent: Option<usize>,
    #[serde(default)]
    voice_max_audio_bytes: Option<u64>,
    #[serde(default)]
    voice_provider: Option<Option<String>>,
    #[serde(default)]
    voice_model: Option<Option<String>>,
    #[serde(default)]
    voice_transcription_model: Option<Option<String>>,
    #[serde(default)]
    voice_synthesis_model: Option<Option<String>>,
    #[serde(default)]
    voice_realtime_model: Option<Option<String>>,
    /// Whether worker delegation (the `task` tool) is available. Absent
    /// means "leave alone", same convention every field here uses. The
    /// `subagents` alias keeps a client sending the pre-rename field name
    /// (a saved script, an unrefreshed admin tab) from silently no-opping.
    #[serde(default, alias = "subagents")]
    workers: Option<bool>,
    /// `[memory]` toggles (docs/design/23-memory.md). Absent means "leave
    /// alone" — same convention every other field here already uses.
    #[serde(default)]
    memory_search_enabled: Option<bool>,
    #[serde(default)]
    memory_write_enabled: Option<bool>,
    #[serde(default)]
    memory_reflection: Option<bool>,
    #[serde(default)]
    memory_skill_proposals: Option<bool>,
    #[serde(default)]
    work_enabled: Option<bool>,
    #[serde(default)]
    work_default_mode: Option<String>,
    #[serde(default)]
    work_max_items: Option<usize>,
    #[serde(default)]
    work_max_revisions: Option<u32>,
    #[serde(default)]
    work_max_parallel: Option<usize>,
    #[serde(default)]
    work_confirmation: Option<String>,
    #[serde(default)]
    inherit_mcp: Option<bool>,
    #[serde(default)]
    inherit_hooks: Option<bool>,
    #[serde(default)]
    inherit_skills: Option<bool>,
    #[serde(default)]
    inherit_commands: Option<bool>,
    #[serde(default)]
    inherit_plugins: Option<bool>,
    /// `[plugins] network_allow` grants for the selected layer (docs/design/
    /// 39-plugin-ecosystem.md). `Some(names)` grants egress to those plugins;
    /// `Some(vec![])` clears the key (deny-by-default); absent leaves alone.
    /// Grants are privileged and refused for an untrusted project layer.
    #[serde(default)]
    plugins_network_allow: Option<Vec<String>>,
    #[serde(default)]
    agent: Option<String>,
}

async fn patch_config(
    State(state): State<AppState>,
    Json(body): Json<ConfigPatch>,
) -> axum::response::Response {
    patch_config_scope(state, body, false).await
}

async fn patch_global_config(
    State(state): State<AppState>,
    Json(body): Json<ConfigPatch>,
) -> axum::response::Response {
    patch_config_scope(state, body, true).await
}

#[derive(serde::Deserialize)]
struct EvidencePolicyBody {
    seconds: i64,
    #[serde(default)]
    scope: Option<String>,
}

async fn patch_evidence_policy(
    State(state): State<AppState>,
    Json(body): Json<EvidencePolicyBody>,
) -> axum::response::Response {
    let path = if body.scope.as_deref() == Some("user") {
        let Some(path) = vak_config::global_path() else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "user home unavailable").into_response();
        };
        path
    } else {
        vak_config::project_path(state.core.cwd())
    };
    if let Err(error) = vak_config::persist_evidence_max_age(path, body.seconds) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": error.to_string()})),
        )
            .into_response();
    }
    if state.core.refresh_persisted_preferences().is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not apply evidence policy",
        )
            .into_response();
    }
    Json(serde_json::json!({"saved": true, "seconds": body.seconds.max(0)})).into_response()
}

/// Which keys this write landed on disk but did NOT put into force, because
/// a narrower layer already pins them.
///
/// The project layer is merged after the global one, so a project
/// `permission_mode` wins. Writing the global value and then force-applying
/// it as a runtime override made the live process disagree with what the
/// files resolve to — right until the next restart, when the project pin
/// reasserted and the operator's change appeared to have been forgotten.
/// Persisting and then saying which keys are shadowed is honest; applying
/// them was not.
fn shadowed_by_project(core: &vak_core::Core, body: &ConfigPatch) -> Vec<&'static str> {
    let path = vak_config::project_path(core.cwd());
    // A workspace that IS the default workspace has one file serving as both
    // layers, and `load_with_trust` skips the project pass for exactly that
    // case. Comparing the file against itself made every global write on the
    // gateway's own workspace — the normal case — report as shadowed by a
    // project layer that does not independently exist, telling an operator
    // their change would not take effect when it would.
    if vak_config::global_path().is_some_and(|global| global == path) {
        return Vec::new();
    }
    let Ok(project) = read_config_layer(&path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if body.permission_mode.is_some() && project.permission_mode.is_some() {
        out.push("permission_mode");
    }
    if body.approval_mode.is_some() && project.approval_mode.is_some() {
        out.push("approval_mode");
    }
    if body.plugins_network_allow.is_some() && project.plugins.network_allow.is_some() {
        out.push("plugins_network_allow");
    }
    out
}

async fn patch_config_scope(
    state: AppState,
    body: ConfigPatch,
    global: bool,
) -> axum::response::Response {
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    if global
        && (body.inherit_mcp.is_some()
            || body.inherit_hooks.is_some()
            || body.inherit_skills.is_some()
            || body.inherit_commands.is_some()
            || body.inherit_plugins.is_some())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if body
        .provider
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
        || body
            .model
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        || body
            .max_turns
            .is_some_and(|value| !(1..=1000).contains(&value))
        || body
            .theme
            .as_deref()
            .is_some_and(|value| !matches!(value, "dark" | "light" | "plain"))
        || body
            .permission_mode
            .as_deref()
            .is_some_and(|value| parse_mode(value).is_none())
        || body
            .approval_mode
            .as_deref()
            .is_some_and(|value| parse_approval_mode(value).is_none())
        || body
            .work_default_mode
            .as_deref()
            .is_some_and(|value| !matches!(value, "direct" | "managed" | "auto"))
        || body
            .work_confirmation
            .as_deref()
            .is_some_and(|value| !matches!(value, "risk-based" | "always" | "never"))
        || body
            .work_max_items
            .is_some_and(|value| !(1..=128).contains(&value))
        || body
            .work_max_revisions
            .is_some_and(|value| !(1..=64).contains(&value))
        || body
            .work_max_parallel
            .is_some_and(|value| !(1..=32).contains(&value))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let permission_mode = body.permission_mode.as_deref().and_then(parse_mode);
    let approval_mode = body.approval_mode.as_deref().and_then(parse_approval_mode);
    if body
        .voice_max_session_secs
        .is_some_and(|v| !(1..=86_400).contains(&v))
        || body
            .voice_max_concurrent
            .is_some_and(|v| !(1..=64).contains(&v))
        || body
            .voice_max_audio_bytes
            .is_some_and(|v| !(1..=256 * 1024 * 1024).contains(&v))
        || body
            .voice_provider
            .as_ref()
            .and_then(|v| v.as_ref())
            .is_some_and(|v| v.trim().is_empty() || v.chars().count() > 256)
        || body
            .voice_model
            .as_ref()
            .and_then(|v| v.as_ref())
            .is_some_and(|v| v.trim().is_empty() || v.chars().count() > 256)
        || [
            body.voice_transcription_model.as_ref(),
            body.voice_synthesis_model.as_ref(),
            body.voice_realtime_model.as_ref(),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .any(|v| v.trim().is_empty() || v.chars().count() > 256)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if body.voice_enabled.is_some()
        || body.voice_max_session_secs.is_some()
        || body.voice_max_concurrent.is_some()
        || body.voice_max_audio_bytes.is_some()
        || body.voice_provider.is_some()
        || body.voice_model.is_some()
        || body.voice_transcription_model.is_some()
        || body.voice_synthesis_model.is_some()
        || body.voice_realtime_model.is_some()
    {
        let result = if global {
            vak_config::global_path().map_or_else(
                || {
                    Err(vak_config::ConfigError::Write {
                        path: std::path::PathBuf::from("<user-config>"),
                        source: std::io::Error::other("user home unavailable"),
                    })
                },
                |path| {
                    vak_config::persist_voice_settings_at_with_models(
                        path,
                        body.voice_enabled,
                        body.voice_max_session_secs,
                        body.voice_max_concurrent,
                        body.voice_max_audio_bytes,
                        body.voice_provider.clone(),
                        body.voice_model.clone(),
                        body.voice_transcription_model.clone(),
                        body.voice_synthesis_model.clone(),
                        body.voice_realtime_model.clone(),
                    )
                },
            )
        } else {
            vak_config::persist_voice_settings_at_with_models(
                vak_config::project_path(core.cwd()),
                body.voice_enabled,
                body.voice_max_session_secs,
                body.voice_max_concurrent,
                body.voice_max_audio_bytes,
                body.voice_provider.clone(),
                body.voice_model.clone(),
                body.voice_transcription_model.clone(),
                body.voice_synthesis_model.clone(),
                body.voice_realtime_model.clone(),
            )
        };
        if result.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        // Re-read the persisted layered value and publish it immediately;
        // new voice sessions observe the change without a restart.
        if let Ok(effective) =
            vak_config::load_with_trust(core.cwd(), core.project_config_trusted())
        {
            core.apply_persisted_voice(effective.voice);
        }
    }
    let current_route = core.effective_route();
    let route_change = body.provider.is_some() || body.model.is_some();
    let provider = route_change.then(|| {
        body.provider
            .as_deref()
            .map(str::trim)
            .unwrap_or(&current_route.provider)
            .to_string()
    });
    let model = route_change.then(|| {
        body.model
            .as_deref()
            .map(str::trim)
            .unwrap_or(&current_route.model)
            .to_string()
    });
    if (body.provider.is_some()
        || body.model.is_some()
        || body.max_turns.is_some()
        || permission_mode.is_some()
        || approval_mode.is_some()
        || body.theme.is_some())
        && (if global {
            vak_config::persist_global_preferences(
                provider.as_deref(),
                model.as_deref(),
                body.max_turns,
                permission_mode,
                approval_mode,
                body.theme.as_deref(),
            )
        } else {
            vak_config::persist_project_preferences(
                core.cwd(),
                provider.as_deref(),
                model.as_deref(),
                body.max_turns,
                permission_mode,
                approval_mode,
                body.theme.as_deref(),
            )
        })
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // Only a global write can be shadowed: the project layer is the last
    // one merged, so a project write is already the winner.
    let shadowed: Vec<&'static str> = if global {
        shadowed_by_project(&core, &body)
    } else {
        Vec::new()
    };
    let mut changes = Vec::new();
    if let (Some(provider), Some(model)) = (provider, model) {
        let effective = vak_config::load_with_trust(core.cwd(), core.project_config_trusted());
        let Ok(effective) = effective else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        core.apply_persisted_route(effective.provider, effective.model);
        changes.push(format!("route={provider}/{model}"));
    }
    if let Some(max_turns) = body.max_turns {
        if !(1..=1000).contains(&max_turns) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        core.apply_persisted_max_turns(max_turns);
        changes.push(format!("max_turns={max_turns}"));
    }
    if let Some(mode) = &body.permission_mode {
        let Some(mode) = parse_mode(mode) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        // Persisted above either way; only put into force when nothing
        // narrower already pins it, so the live value and the files agree.
        if shadowed.contains(&"permission_mode") {
            changes.push(format!("permission_mode={mode:?} (persisted, shadowed)"));
        } else {
            apply_permission_mode(&core, &state, mode, true);
            changes.push(format!("permission_mode={mode:?}"));
        }
    }
    if let Some(raw) = &body.approval_mode {
        let Some(mode) = parse_approval_mode(raw) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        if shadowed.contains(&"approval_mode") {
            changes.push(format!(
                "approval_mode={} (persisted, shadowed)",
                mode.as_str()
            ));
        } else {
            core.apply_persisted_approval_mode(mode);
            changes.push(format!("approval_mode={}", mode.as_str()));
        }
    }
    if let Some(theme) = body.theme {
        if !matches!(theme.as_str(), "dark" | "light" | "plain") {
            return StatusCode::BAD_REQUEST.into_response();
        }
        changes.push(format!("theme={theme}"));
        core.apply_persisted_theme(theme);
    }
    if let Some(workers) = body.workers {
        let persisted = if global {
            vak_config::persist_global_workers(workers)
        } else {
            vak_config::persist_project_workers(core.cwd(), workers)
        };
        if persisted.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        core.apply_persisted_workers(workers);
        changes.push(format!("workers={workers}"));
    }
    if body.memory_search_enabled.is_some()
        || body.memory_write_enabled.is_some()
        || body.memory_reflection.is_some()
        || body.memory_skill_proposals.is_some()
    {
        let persisted = if global {
            vak_config::persist_global_memory_prefs(
                body.memory_search_enabled,
                body.memory_write_enabled,
                body.memory_reflection,
                body.memory_skill_proposals,
            )
        } else {
            vak_config::persist_project_memory_prefs(
                core.cwd(),
                body.memory_search_enabled,
                body.memory_write_enabled,
                body.memory_reflection,
                body.memory_skill_proposals,
            )
        };
        if persisted.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        // `apply_persisted_memory` sets all four flags at once, so fields
        // this PATCH didn't mention keep their current effective value
        // rather than reverting to whatever was on disk before.
        core.apply_persisted_memory(
            body.memory_search_enabled
                .unwrap_or_else(|| core.effective_memory_search_enabled()),
            body.memory_write_enabled
                .unwrap_or_else(|| core.effective_memory_write_enabled()),
            body.memory_reflection
                .unwrap_or_else(|| core.effective_memory_reflection()),
            body.memory_skill_proposals
                .unwrap_or_else(|| core.effective_memory_skill_proposals()),
        );
        changes.push(format!(
            "memory(search={}, write={}, reflection={}, skill_proposals={})",
            core.effective_memory_search_enabled(),
            core.effective_memory_write_enabled(),
            core.effective_memory_reflection(),
            core.effective_memory_skill_proposals(),
        ));
    }
    if body.work_enabled.is_some()
        || body.work_default_mode.is_some()
        || body.work_max_items.is_some()
        || body.work_max_revisions.is_some()
        || body.work_max_parallel.is_some()
        || body.work_confirmation.is_some()
    {
        let path = if global {
            vak_config::global_path().ok_or(StatusCode::INTERNAL_SERVER_ERROR)
        } else {
            Ok(vak_config::project_path(core.cwd()))
        };
        let Ok(path) = path else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        if vak_config::persist_work_preferences(
            path,
            body.work_enabled,
            body.work_default_mode.as_deref(),
            body.work_max_items,
            body.work_max_revisions,
            body.work_max_parallel,
            body.work_confirmation.as_deref(),
        )
        .is_err()
        {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let resolved = vak_config::load_with_trust(core.cwd(), core.project_config_trusted());
        let Ok(resolved) = resolved else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        core.apply_persisted_work(resolved.work);
        changes.push(format!(
            "work(mode={}, enabled={})",
            core.effective_work().default_mode,
            core.effective_work().enabled
        ));
    }
    if let Some(grant) = &body.plugins_network_allow {
        if !grant.is_empty() && !global && !core.project_config_trusted() {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "plugins.network_allow is a privileged key: an untrusted \
                              workspace may not grant its execution plugins network \
                              egress. Set it from the Trusted (global) settings instead."
                })),
            )
                .into_response();
        }
        let path = if global {
            vak_config::global_path().ok_or(StatusCode::INTERNAL_SERVER_ERROR)
        } else {
            Ok(vak_config::project_path(core.cwd()))
        };
        let Ok(path) = path else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        if vak_config::persist_plugins_network_allow(&path, Some(grant.clone())).is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        if core.refresh_persisted_preferences().is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        if shadowed.contains(&"plugins_network_allow") {
            changes.push(format!(
                "plugins_network_allow={grant:?} (persisted, shadowed by project)"
            ));
        } else {
            changes.push(format!("plugins_network_allow={grant:?}"));
        }
    }
    if body.inherit_mcp.is_some()
        || body.inherit_hooks.is_some()
        || body.inherit_skills.is_some()
        || body.inherit_commands.is_some()
        || body.inherit_plugins.is_some()
    {
        let path = vak_config::project_path(core.cwd());
        if vak_config::persist_capability_inheritance(
            &path,
            body.inherit_mcp,
            body.inherit_hooks,
            body.inherit_skills,
            body.inherit_commands,
            body.inherit_plugins,
        )
        .is_err()
        {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let Ok(resolved) = vak_config::load_with_trust(core.cwd(), core.project_config_trusted())
        else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        core.apply_persisted_capability_inheritance(resolved.capabilities);
        core.apply_persisted_mcp_servers(resolved.mcp);
        core.apply_persisted_hooks(resolved.hooks);
        changes.push("capability_inheritance".into());
    }
    if !changes.is_empty() {
        vak_core::security_events::record(
            &core.sessions_home(),
            vak_core::security_events::EventKind::ConfigChange,
            "config_patched",
            &changes.join(", "),
            None,
        );
        state
            .hub
            .emit_config_changed("config_patched", &changes.join(", "));
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "applied": changes,
            // Named, so a client can tell the operator "saved, but this
            // project overrides it" instead of reporting a clean success
            // that the next restart quietly undoes.
            "shadowed_by_project": shadowed,
        })),
    )
        .into_response()
}

// ---- MCP server management --------------------------------------------------
//
// The desktop Settings page edits the MCP table here: GET reads the
// effective table; PUT validates, persists to the project config.toml
// ([mcp.servers]) and hot-applies into the running Core so the next turn
// picks it up without a backend restart. mcp.servers is a privileged key:
// this endpoint is only reachable through the bearer-token router of a
// locally trusted surface.

/// Project-only, deliberately not `core.effective_mcp()` (the merged
/// effective set, global layer included). `PUT /config/mcp` writes whatever
/// this reports straight into the *project* config — reporting the merged
/// set would silently fork every currently-inherited global MCP server
/// into the project file the moment any one server was added, edited, or
/// removed here, freezing that project's copy out of future changes to the
/// global definition. Unlike the hooks list (a plain `Vec` extended with no
/// dedup key), the merge for MCP servers is a name-keyed map — so this
/// couldn't duplicate or compound the way the hooks bug did, but it would
/// still quietly diverge project config from what the operator thought
/// they were changing. Mirrors `get_global_mcp_servers`, which has always
/// read its own file directly for the same reason.
async fn get_mcp_servers(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let path = vak_config::project_path(core.cwd());
    match read_mcp_config(&path) {
        Ok(mcp) => Json(serde_json::json!({ "servers": mcp.servers })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize, Clone)]
struct HookInput {
    event: String,
    #[serde(default)]
    matcher: Option<String>,
    command: String,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default = "default_hook_enabled")]
    enabled: bool,
    #[serde(default)]
    failure_mode: Option<String>,
}

fn default_hook_enabled() -> bool {
    true
}

#[derive(serde::Deserialize)]
struct HooksPutBody {
    hooks: Vec<HookInput>,
    #[serde(default)]
    agent: Option<String>,
}

/// Project-only, deliberately not `state.core.config().hooks` (the merged
/// effective list, global layer included — `vak_config::merge_into` extends
/// the project's hooks with the global ones on every load). `PUT
/// /config/hooks` replaces the *project* file's own `[[hooks]]` array with
/// whatever this endpoint reported; reporting the merged list would hand
/// back an inherited global hook, which the next save would then write into
/// the project file as if it were the project's own — duplicating it there,
/// and compounding on every subsequent edit as the merge re-extends over an
/// already-doubled list. Mirrors `get_global_hooks`, which has always read
/// its own file directly for the same reason.
/// Prompt layers (docs/design/45-prompt-layers.md).
///
/// `GET /config/prompts?scope=` reports **only** the selected layer's own
/// text, never the merged view — AGENTS.md rule 21: this GET seeds a
/// same-shape PUT, and returning inherited text would write a parent layer's
/// content into the child file on the next save. The effective composition
/// is a separate endpoint on purpose.
async fn get_prompt_layer(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let dir = vak_core::prompts::layer_dir(&query.scope.prompt_root(&core));
    let content = vak_core::prompts::read_layer(&dir);
    Json(serde_json::json!({
        "scope": query.scope.label(),
        "path": dir.display().to_string(),
        "layer": content,
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct PromptBlockBody {
    scope: ConfigScope,
    block: String,
    /// Absent or null resets the block and resumes inheritance.
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

async fn put_prompt_block(
    State(state): State<AppState>,
    Json(body): Json<PromptBlockBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let Some(block) = vak_core::prompts::PromptBlock::parse(&body.block) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unknown block '{}'", body.block),
                "editable": ["identity", "operating-rules", "guardrails"],
                "note": "the capability contract, Surface line, and skill/MCP lists are code-owned",
            })),
        )
            .into_response();
    };
    // A prompt is spent on every turn of every session, so an unbounded
    // editor is a permanent context tax rather than a one-off mistake.
    const MAX_BLOCK_BYTES: usize = 24_000;
    if let Some(text) = body.text.as_deref()
        && text.len() > MAX_BLOCK_BYTES
    {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": format!("{} is {}B; the cap is {MAX_BLOCK_BYTES}B", block.slug(), text.len()),
            })),
        )
            .into_response();
    }
    let dir = vak_core::prompts::layer_dir(&body.scope.prompt_root(&core));
    match vak_core::prompts::write_block(&dir, block, body.text.as_deref()) {
        Ok(()) => Json(serde_json::json!({
            "ok": true,
            "scope": body.scope.label(),
            "block": block.slug(),
            "reset": body.text.is_none(),
            // Prompts freeze into the session contract, so the UI must not
            // imply a running turn changed under the user.
            "applies": "new sessions",
        }))
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

/// The assembled prompt plus per-layer provenance — the right-hand pane.
async fn get_prompt_effective(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    Json(prompt_effective_payload(&state.core)).into_response()
}

async fn get_agents(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let root = match query
        .get("scope")
        .map(String::as_str)
        .unwrap_or("workspace")
    {
        "user" => vak_config::paths::default_workspace(),
        "workspace" => state.active_core().cwd().clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid agent scope"})),
            )
                .into_response();
        }
    };
    match agents::load(&root) {
        Ok(agents) => Json(serde_json::json!({ "agents": agents })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn list_agent_templates() -> axum::response::Response {
    use axum::response::IntoResponse;
    Json(serde_json::json!({
        "templates": agents::builtin_templates()
    }))
    .into_response()
}

#[derive(serde::Deserialize)]
struct InstantiateTemplateRequest {
    template_id: String,
    agent_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

async fn instantiate_agent_template(
    State(state): State<AppState>,
    Json(body): Json<InstantiateTemplateRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(template) = agents::find_template(&body.template_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("template '{}' not found", body.template_id) })),
        )
            .into_response();
    };

    let new_agent = template.to_agent_definition(&body.agent_id, body.name.as_deref());
    let root = match body.scope.as_deref().unwrap_or("workspace") {
        "user" => vak_config::paths::default_workspace(),
        _ => state.active_core().cwd().clone(),
    };

    let mut existing = agents::load(&root).unwrap_or_default();
    if existing.iter().any(|a| a.id == new_agent.id) {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({ "error": format!("agent '{}' already exists", new_agent.id) }),
            ),
        )
            .into_response();
    }
    existing.push(new_agent.clone());
    match agents::save(&root, &existing) {
        Ok(_) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "created": true, "agent": new_agent })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct UpdateScheduleRequest {
    cron_or_interval: String,
    prompt: String,
    #[serde(default = "default_schedule_enabled")]
    enabled: bool,
    #[serde(default)]
    scope: Option<String>,
}

fn default_schedule_enabled() -> bool {
    true
}

async fn update_agent_schedule_route(
    State(state): State<AppState>,
    axum::extract::Path(agent_id): axum::extract::Path<String>,
    Json(body): Json<UpdateScheduleRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let root = match body.scope.as_deref().unwrap_or("workspace") {
        "user" => vak_config::paths::default_workspace(),
        _ => state.active_core().cwd().clone(),
    };
    let sched = agents::AgentSchedule {
        cron_or_interval: body.cron_or_interval,
        prompt: body.prompt,
        enabled: body.enabled,
        last_run_at: None,
        last_status: None,
    };
    match agents::update_schedule(&root, &agent_id, Some(sched)) {
        Ok(agent) => Json(serde_json::json!({ "updated": true, "agent": agent })).into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct ListRunsQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    scope: Option<String>,
}

async fn list_agent_runs_route(
    State(state): State<AppState>,
    axum::extract::Path(agent_id): axum::extract::Path<String>,
    axum::extract::Query(query): axum::extract::Query<ListRunsQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let root = match query.scope.as_deref().unwrap_or("workspace") {
        "user" => vak_config::paths::default_workspace(),
        _ => state.active_core().cwd().clone(),
    };
    let limit = query.limit.unwrap_or(20).min(100);
    match agents::list_runs(&root, Some(&agent_id), limit) {
        Ok(runs) => Json(serde_json::json!({ "runs": runs })).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct CanvasPreviewRequest {
    #[serde(default)]
    title: Option<String>,
    content: String,
}

async fn canvas_preview(Json(body): Json<CanvasPreviewRequest>) -> axum::response::Response {
    let title = body.title.as_deref().unwrap_or("Outcome Canvas");
    let html = vak_presentation::transcode_to_html(title, &body.content);
    html_response(html)
}

async fn put_agents(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(raw) = body.get("agents") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "agents is required" })),
        )
            .into_response();
    };
    let Ok(agents) = serde_json::from_value::<Vec<agents::AgentDefinition>>(raw.clone()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "invalid agents" })),
        )
            .into_response();
    };
    let root = match body
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or("workspace")
    {
        "user" => vak_config::paths::default_workspace(),
        "workspace" => state.active_core().cwd().clone(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid agent scope"})),
            )
                .into_response();
        }
    };
    match agents::save(&root, &agents) {
        Ok(saved_agents) => {
            Json(serde_json::json!({ "saved": true, "agents": saved_agents })).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

fn prompt_effective_payload(core: &vak_core::Core) -> serde_json::Value {
    let resolution = core.resolve_prompt(&core.capability_descriptors());
    let (seed_content, _, _) = vak_core::prompts::seed(vak_core::APP_VERSION);
    let seed_blocks = serde_json::json!({
        "identity": seed_content.identity,
        "operating-rules": seed_content.operating_rules,
        "guardrails": seed_content.guardrails.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n"),
        "surface-note": seed_content.surface_notes.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n"),
    });
    serde_json::json!({
        "text": resolution.text,
        "fingerprint": resolution.fingerprint(),
        "estimated_tokens": resolution.text.len() / 4,
        "surface": core.surface().slug(),
        "layers": resolution.descriptors,
        "blocks": resolution.blocks,
        "seed_blocks": seed_blocks,
    })
}

#[derive(serde::Deserialize)]
struct PromptPreviewBody {
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

/// Render exactly what a chosen surface and role would receive. Composition
/// across seven tiers is not guessable, so an editor without this is asking
/// the operator to simulate the resolver in their head.
async fn preview_prompt(
    State(state): State<AppState>,
    Json(body): Json<PromptPreviewBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let raw_surface = body.surface.as_deref().map(str::trim).unwrap_or("");
    let surface = match raw_surface.to_ascii_lowercase().as_str() {
        "" | "unknown" => vak_core::Surface::Unknown,
        "cli" => vak_core::Surface::Cli,
        "terminal" => vak_core::Surface::Terminal,
        "desktop" => vak_core::Surface::Desktop,
        "server" => vak_core::Surface::Server,
        "web" => vak_core::Surface::Web,
        "background" => vak_core::Surface::Background,
        // "subagent" is kept for admin-console requests built against the
        // pre-rename surface name.
        "worker" | "subagent" => vak_core::Surface::Worker,
        _ => vak_core::Surface::Chat {
            channel: raw_surface.to_string(),
        },
    };
    if let Some(role) = body.role.as_deref()
        && !core.prompt_role_names().iter().any(|n| n == role)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("no role '{role}'") })),
        )
            .into_response();
    }
    let core = core.with_surface(surface).with_prompt_role(body.role);
    Json(prompt_effective_payload(&core)).into_response()
}

async fn list_prompt_roles(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    Json(serde_json::json!({ "roles": core.prompt_role_names() })).into_response()
}

async fn get_hooks(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, q.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let path = core.cwd().join(".vak/config.toml");
    let hooks = if path.is_file() {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str::<vak_config::FileConfig>(&raw).ok())
        {
            Some(config) => config.hooks,
            None => {
                return (StatusCode::BAD_REQUEST, "workspace config is invalid").into_response();
            }
        }
    } else {
        Vec::new()
    };
    let hooks = hooks
        .iter()
        .map(|h| {
            serde_json::json!({
                "event": h.event,
                "matcher": h.matcher,
                "command": h.command,
                "timeout_ms": h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS),
                "enabled": h.enabled,
                "failure_mode": h.failure_mode.as_deref().unwrap_or("open"),
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "hooks": hooks })).into_response()
}

async fn get_global_hooks() -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(path) = vak_config::global_path() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "user home unavailable").into_response();
    };
    let hooks = if path.is_file() {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str::<vak_config::FileConfig>(&raw).ok())
        {
            Some(config) => config.hooks,
            None => return (StatusCode::BAD_REQUEST, "user config is invalid").into_response(),
        }
    } else {
        Vec::new()
    };
    Json(serde_json::json!({ "scope": "global", "hooks": hooks.into_iter().map(|h| serde_json::json!({ "event": h.event, "matcher": h.matcher, "command": h.command, "timeout_ms": h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS), "enabled": h.enabled, "failure_mode": h.failure_mode.as_deref().unwrap_or("open") })).collect::<Vec<_>>() })).into_response()
}

async fn put_global_hooks(
    State(state): State<AppState>,
    Json(body): Json<HooksPutBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(path) = vak_config::global_path() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "user home unavailable").into_response();
    };
    let hooks = match validated_hook_configs(&body.hooks) {
        Ok(hooks) => hooks,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": message })),
            )
                .into_response();
        }
    };
    if let Err(error) = persist_hooks_to_config(&path, &hooks) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }
    if state.core.refresh_persisted_preferences().is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not apply user hooks",
        )
            .into_response();
    }
    Json(serde_json::json!({ "saved": true, "scope": "global", "count": hooks.len() }))
        .into_response()
}

fn validated_hook_configs(hooks: &[HookInput]) -> Result<Vec<vak_config::HookConfig>, String> {
    hooks
        .iter()
        .map(|hook| {
            if !matches!(
                hook.event.as_str(),
                "session_start"
                    | "session-start"
                    | "pre_tool_use"
                    | "pre-tool-use"
                    | "post_tool_use"
                    | "post-tool-use"
                    | "stop"
            ) {
                return Err(format!("unknown hook event '{}'", hook.event));
            }
            if hook.enabled && hook.command.trim().is_empty() {
                return Err("enabled hooks need a command".into());
            }
            let timeout_ms = hook.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS);
            if timeout_ms == 0 {
                return Err("hook timeout must be greater than zero".into());
            }
            let failure_mode = hook.failure_mode.as_deref().unwrap_or("open").trim();
            if !matches!(failure_mode, "open" | "closed") {
                return Err(format!("unknown hook failure mode '{failure_mode}'"));
            }
            Ok(vak_config::HookConfig {
                event: hook.event.clone(),
                matcher: hook
                    .matcher
                    .clone()
                    .filter(|matcher| !matcher.trim().is_empty()),
                command: hook.command.trim().to_string(),
                timeout_ms: Some(timeout_ms),
                enabled: hook.enabled,
                failure_mode: Some(failure_mode.to_string()),
            })
        })
        .collect()
}

fn persist_hooks_to_config(
    path: &std::path::Path,
    hooks: &[vak_config::HookConfig],
) -> Result<(), String> {
    let mut root = if path.is_file() {
        let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        toml::from_str::<toml::Value>(&raw).map_err(|error| error.to_string())?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let Some(table) = root.as_table_mut() else {
        return Err("config root is not a table".into());
    };
    table.insert(
        "hooks".into(),
        toml::Value::Array(
            hooks
                .iter()
                .map(|hook| {
                    let mut value = toml::map::Map::new();
                    value.insert("event".into(), toml::Value::String(hook.event.clone()));
                    value.insert("command".into(), toml::Value::String(hook.command.clone()));
                    if let Some(matcher) = &hook.matcher {
                        value.insert("match".into(), toml::Value::String(matcher.clone()));
                    }
                    value.insert(
                        "timeout_ms".into(),
                        toml::Value::Integer(
                            hook.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS) as i64,
                        ),
                    );
                    value.insert("enabled".into(), toml::Value::Boolean(hook.enabled));
                    value.insert(
                        "failure_mode".into(),
                        toml::Value::String(hook.failure_mode.as_deref().unwrap_or("open").into()),
                    );
                    toml::Value::Table(value)
                })
                .collect(),
        ),
    );
    let text = toml::to_string_pretty(&root).map_err(|error| error.to_string())?;
    let parent = path.parent().ok_or("config has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temp = parent.join(format!(".config.toml.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).map_err(|error| error.to_string())?;
    std::fs::rename(temp, path).map_err(|error| error.to_string())
}

async fn put_hooks(
    State(state): State<AppState>,
    Json(body): Json<HooksPutBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    for hook in &body.hooks {
        if !matches!(
            hook.event.as_str(),
            "session_start"
                | "session-start"
                | "pre_tool_use"
                | "pre-tool-use"
                | "post_tool_use"
                | "post-tool-use"
                | "stop"
        ) {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({ "error": format!("unknown hook event '{}'", hook.event) }),
                ),
            )
                .into_response();
        }
        if hook.enabled && hook.command.trim().is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "enabled hooks need a command" })),
            )
                .into_response();
        }
        if hook.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS) == 0 {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "hook timeout must be greater than zero" })),
            )
                .into_response();
        }
        if !matches!(
            hook.failure_mode.as_deref().unwrap_or("open").trim(),
            "open" | "closed"
        ) {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({ "error": "hook failure_mode must be 'open' or 'closed'" }),
                ),
            )
                .into_response();
        }
    }
    let path = core.cwd().join(".vak/config.toml");
    let mut root: toml::Value = if path.exists() {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str(&raw).ok())
        {
            Some(v) => v,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "workspace config is invalid" })),
                )
                    .into_response();
            }
        }
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    // A disabled hook is kept in config, not dropped — round-tripping the
    // toggle used to delete the definition outright (there was nowhere in
    // `[[hooks]]` to record "off"), which is not what a checkbox should do.
    let values = body
        .hooks
        .iter()
        .map(|h| {
            let mut t = toml::map::Map::new();
            t.insert("event".into(), toml::Value::String(h.event.clone()));
            t.insert(
                "command".into(),
                toml::Value::String(h.command.trim().into()),
            );
            if let Some(m) = h.matcher.as_ref().filter(|m| !m.trim().is_empty()) {
                t.insert("match".into(), toml::Value::String(m.clone()));
            }
            t.insert(
                "timeout_ms".into(),
                toml::Value::Integer(h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS) as i64),
            );
            t.insert("enabled".into(), toml::Value::Boolean(h.enabled));
            t.insert(
                "failure_mode".into(),
                toml::Value::String(h.failure_mode.as_deref().unwrap_or("open").trim().into()),
            );
            toml::Value::Table(t)
        })
        .collect::<Vec<_>>();
    let Some(table) = root.as_table_mut() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "config root is not a table" })),
        )
            .into_response();
    };
    table.insert("hooks".into(), toml::Value::Array(values));
    let out = match toml::to_string_pretty(&root) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("serialize config: {e}") })),
            )
                .into_response();
        }
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, out) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("write config: {e}") })),
        )
            .into_response();
    }
    // Disabled hooks are still handed to Core — `build_hooks_from` is what
    // skips them when it builds the live `HookDef` list — so the effective
    // set stays correct without this endpoint duplicating that filter.
    core.apply_persisted_hooks(
        body.hooks
            .iter()
            .map(|h| vak_config::HookConfig {
                event: h.event.clone(),
                matcher: h.matcher.clone().filter(|m| !m.trim().is_empty()),
                command: h.command.trim().to_string(),
                timeout_ms: Some(h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS)),
                enabled: h.enabled,
                failure_mode: h.failure_mode.clone(),
            })
            .collect(),
    );
    let enabled_count = body.hooks.iter().filter(|h| h.enabled).count();
    vak_core::security_events::record(
        &core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "hooks_updated",
        &format!("enabled={enabled_count} total={}", body.hooks.len()),
        None,
    );
    state.hub.emit_config_changed(
        "hooks_updated",
        &format!("enabled={enabled_count} total={}", body.hooks.len()),
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "saved": true, "count": enabled_count })),
    )
        .into_response()
}

#[derive(serde::Deserialize, Clone)]
struct McpServerInput {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    network: bool,
}

#[derive(serde::Deserialize)]
struct McpPutBody {
    servers: std::collections::BTreeMap<String, McpServerInput>,
    #[serde(default)]
    agent: Option<String>,
}

fn valid_server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn persist_mcp_to_project_config(
    cwd: &std::path::Path,
    servers: &std::collections::BTreeMap<String, McpServerInput>,
) -> Result<std::path::PathBuf, String> {
    let path = vak_config::project_path(cwd);
    let config = mcp_config_from_input(servers);
    vak_config::persist_mcp_servers(&path, &config.servers).map_err(|error| error.to_string())?;
    Ok(path)
}

fn mcp_config_from_input(
    servers: &std::collections::BTreeMap<String, McpServerInput>,
) -> vak_config::McpConfig {
    vak_config::McpConfig {
        servers: servers
            .iter()
            .map(|(name, server)| {
                (
                    name.clone(),
                    vak_config::McpServerConfig {
                        command: server.command.trim().to_string(),
                        args: server.args.clone(),
                        env: server.env.clone(),
                        network: server.network,
                        // Undeclared: the per-turn slice never narrows a
                        // capability that has not classified itself, so a
                        // server configured through the API stays reachable
                        // without the operator having to know about domains.
                        serves: Vec::new(),
                    },
                )
            })
            .collect(),
    }
}

fn read_mcp_config(path: &std::path::Path) -> Result<vak_config::McpConfig, String> {
    if !path.is_file() {
        return Ok(vak_config::McpConfig::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    toml::from_str::<vak_config::FileConfig>(&raw)
        .map(|config| config.mcp)
        .map_err(|error| error.to_string())
}

/// Which layer a setting belongs to.
///
/// One word for one concept: the config section is `workspace_roots`, the
/// path helper is `default_workspace`, the API is `/workspaces` — and this
/// said "project". Two names for the same thing is how a UI ends up
/// labelling one panel "This project" and its own store `workspace`.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum ConfigScope {
    User,
    Workspace,
}

impl ConfigScope {
    fn is_workspace(self) -> bool {
        matches!(self, Self::Workspace)
    }

    fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Workspace => "workspace",
        }
    }

    /// Root whose `.vak/prompts` directory this scope edits.
    fn prompt_root(self, core: &vak_core::Core) -> std::path::PathBuf {
        match self {
            Self::User => vak_config::paths::default_workspace(),
            Self::Workspace => core.cwd().clone(),
        }
    }

    fn config_path(self, core: &vak_core::Core) -> Result<std::path::PathBuf, String> {
        match self {
            Self::User => vak_config::global_path().ok_or_else(|| "user home unavailable".into()),
            Self::Workspace => Ok(vak_config::project_path(core.cwd())),
        }
    }
}

#[derive(serde::Deserialize)]
struct ScopeQuery {
    scope: ConfigScope,
    /// Which user-facing Agent's isolated workspace this config layer is
    /// rooted under. Absent means the built-in "vak" Agent, resolved
    /// through `resolve_scoped_core` exactly like the memory/proposal
    /// endpoints (see commit 15c9c256).
    #[serde(default)]
    agent: Option<String>,
}

/// A scope query where omitting the parameter is legal and means "workspace".
/// Kept separate from [`ScopeQuery`] so the endpoints that genuinely
/// require an explicit scope keep rejecting a request without one.
#[derive(serde::Deserialize)]
struct OptionalScopeQuery {
    #[serde(default)]
    scope: Option<ConfigScope>,
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Clone, Copy)]
struct IntegrationCatalogEntry {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    env_var: Option<&'static str>,
    key_required: bool,
    documentation_url: &'static str,
}

/// The curated integrations, **alphabetically**.
///
/// Order is not cosmetic here. Whatever sits first reads as the default,
/// and this list led with Tavily — which is how one connector came to look
/// like the real one and the others like extras (doc 46 D5). They are
/// peers: same shape, same status projection, same scoped read/write path.
const INTEGRATION_CATALOG: &[IntegrationCatalogEntry] = &[
    IntegrationCatalogEntry {
        id: "context7",
        label: "Context7",
        description: "Current library documentation and version-specific code examples.",
        command: "npx",
        args: &["-y", "@upstash/context7-mcp@latest"],
        env_var: Some("CONTEXT7_API_KEY"),
        key_required: false,
        documentation_url: "https://github.com/upstash/context7",
    },
    IntegrationCatalogEntry {
        id: "exa",
        label: "Exa",
        description: "Web, code, company, and research search with page retrieval.",
        command: "npx",
        args: &["-y", "exa-mcp-server"],
        env_var: Some("EXA_API_KEY"),
        key_required: true,
        documentation_url: "https://github.com/exa-labs/exa-mcp-server",
    },
    IntegrationCatalogEntry {
        id: "firecrawl",
        label: "Firecrawl",
        description: "Search, scrape, crawl, extract, and operate cloud browser sessions.",
        command: "npx",
        args: &["-y", "firecrawl-mcp"],
        env_var: Some("FIRECRAWL_API_KEY"),
        key_required: true,
        documentation_url: "https://github.com/firecrawl/firecrawl-mcp-server",
    },
    IntegrationCatalogEntry {
        id: "tavily",
        label: "Tavily",
        description: "Real-time web search, extraction, site maps, and crawling.",
        command: "npx",
        args: &["-y", "tavily-mcp@latest"],
        env_var: Some("TAVILY_API_KEY"),
        key_required: true,
        documentation_url: "https://github.com/tavily-ai/tavily-mcp",
    },
];

fn catalog_entry(id: &str) -> Option<IntegrationCatalogEntry> {
    INTEGRATION_CATALOG
        .iter()
        .copied()
        .find(|entry| entry.id == id)
}

fn catalog_server(entry: IntegrationCatalogEntry) -> vak_config::McpServerConfig {
    vak_config::McpServerConfig {
        command: entry.command.into(),
        args: entry.args.iter().map(|arg| (*arg).to_string()).collect(),
        env: entry
            .env_var
            .map(|name| {
                [(name.to_string(), format!("${{{name}}}"))]
                    .into_iter()
                    .collect()
            })
            .unwrap_or_default(),
        network: true,
        // Catalog entries stay undeclared for the same reason: a server the
        // operator just installed must be reachable immediately, and
        // declaring domains only ever narrows.
        serves: Vec::new(),
    }
}

fn integration_status(
    core: &vak_core::Core,
    scope: ConfigScope,
    entry: IntegrationCatalogEntry,
) -> Result<serde_json::Value, String> {
    let selected = read_mcp_config(&scope.config_path(core)?)?;
    let user = match vak_config::global_path() {
        Some(path) => read_mcp_config(&path)?,
        None => vak_config::McpConfig::default(),
    };
    let configured_here = selected.servers.contains_key(entry.id);
    let inherited = scope.is_workspace() && !configured_here && user.servers.contains_key(entry.id);
    let effective = core.effective_mcp().servers.contains_key(entry.id);
    let key_here = entry
        .env_var
        .is_some_and(|name| core.mcp_secret_at_scope(name, scope.is_workspace()));
    let key_inherited = scope.is_workspace()
        && !key_here
        && entry
            .env_var
            .is_some_and(|name| core.mcp_secret(name).is_some());
    Ok(serde_json::json!({
        "id": entry.id,
        "label": entry.label,
        "description": entry.description,
        "command": entry.command,
        "args": entry.args,
        "network": true,
        "env_var": entry.env_var,
        "key_required": entry.key_required,
        "documentation_url": entry.documentation_url,
        "scope": scope.label(),
        "configured_here": configured_here,
        "inherited": inherited,
        "effective": effective,
        "key_here": key_here,
        "key_inherited": key_inherited,
        "key_effective": entry.env_var.is_none_or(|name| core.mcp_secret(name).is_some()),
    }))
}

async fn get_integration_catalog(
    State(state): State<AppState>,
    Query(query): Query<ScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    match INTEGRATION_CATALOG
        .iter()
        .copied()
        .map(|entry| integration_status(&core, query.scope, entry))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(integrations) => Json(serde_json::json!({
            "scope": query.scope.label(),
            "integrations": integrations,
        }))
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn get_scoped_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(entry) = catalog_entry(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    match integration_status(&core, query.scope, entry) {
        Ok(status) => Json(status).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
struct IntegrationPutBody {
    scope: ConfigScope,
    key: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

fn apply_scoped_mcp_change(
    core: &vak_core::Core,
    scope: ConfigScope,
    id: &str,
    server: Option<vak_config::McpServerConfig>,
) -> Result<(), String> {
    let path = scope.config_path(core)?;
    let mut config = read_mcp_config(&path)?;
    match server {
        Some(server) => {
            config.servers.insert(id.to_string(), server);
        }
        None => {
            config.servers.remove(id);
        }
    }
    vak_config::persist_mcp_servers(&path, &config.servers).map_err(|error| error.to_string())?;
    let effective = vak_config::load_with_trust(core.cwd(), core.project_config_trusted())
        .map_err(|error| error.to_string())?;
    core.apply_persisted_mcp_servers(effective.mcp);
    Ok(())
}

async fn put_scoped_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<IntegrationPutBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(entry) = catalog_entry(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    if let Some(key) = body.key.as_deref()
        && let Err(error) = core.set_mcp_secret_scoped(
            entry.env_var.unwrap_or_default(),
            key,
            body.scope.is_workspace(),
        )
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    let key_available = entry
        .env_var
        .is_none_or(|name| core.mcp_secret(name).is_some());
    if entry.key_required && !key_available {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("{} requires {} at this scope or an inherited scope", entry.label, entry.env_var.unwrap_or("a key"))
            })),
        )
            .into_response();
    }
    if let Err(error) =
        apply_scoped_mcp_change(&core, body.scope, entry.id, Some(catalog_server(entry)))
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }
    state.hub.emit_config_changed(
        "integration_enabled",
        &format!("scope={} integration={}", body.scope.label(), entry.id),
    );
    match integration_status(&core, body.scope, entry) {
        Ok(status) => Json(status).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn delete_scoped_integration(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(entry) = catalog_entry(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let core = match resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    if let Some(env_var) = entry.env_var
        && let Err(error) = core.remove_mcp_secret_scoped(env_var, query.scope.is_workspace())
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    if let Err(error) = apply_scoped_mcp_change(&core, query.scope, entry.id, None) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }
    state.hub.emit_config_changed(
        "integration_removed",
        &format!("scope={} integration={}", query.scope.label(), entry.id),
    );
    match integration_status(&core, query.scope, entry) {
        Ok(status) => Json(status).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn get_global_mcp_servers() -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(path) = vak_config::global_path() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "user home unavailable").into_response();
    };
    match read_mcp_config(&path) {
        Ok(mcp) => {
            Json(serde_json::json!({ "scope": "global", "path": path, "servers": mcp.servers }))
                .into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn put_global_mcp_servers(
    State(state): State<AppState>,
    Json(body): Json<McpPutBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(error) = validate_mcp_servers(&body.servers) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }
    let Some(path) = vak_config::global_path() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "user home unavailable").into_response();
    };
    let config = mcp_config_from_input(&body.servers);
    if let Err(error) = vak_config::persist_mcp_servers(&path, &config.servers) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    if state.core.refresh_persisted_preferences().is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not apply user capability configuration",
        )
            .into_response();
    }
    state.hub.emit_config_changed(
        "global_mcp_servers_updated",
        &format!("count={}", body.servers.len()),
    );
    Json(serde_json::json!({ "saved": true, "scope": "global", "count": body.servers.len() }))
        .into_response()
}

fn validate_mcp_servers(
    servers: &std::collections::BTreeMap<String, McpServerInput>,
) -> Result<(), String> {
    for name in servers.keys() {
        if !valid_server_name(name) {
            return Err(format!("invalid server name '{name}'"));
        }
    }
    for (name, server) in servers {
        if server.command.trim().is_empty() {
            return Err(format!("server '{name}' needs a command"));
        }
    }
    Ok(())
}

async fn put_mcp_servers(
    State(state): State<AppState>,
    Json(body): Json<McpPutBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(error) = validate_mcp_servers(&body.servers) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }
    let core = match resolve_scoped_core(&state, None, body.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    match persist_mcp_to_project_config(core.cwd(), &body.servers) {
        Ok(_) => {}
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    }
    let cfg = vak_config::load_with_trust(core.cwd(), core.project_config_trusted())
        .map(|config| config.mcp);
    let Ok(cfg) = cfg else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    core.apply_persisted_mcp_servers(cfg);
    vak_core::security_events::record(
        &core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "mcp_servers_updated",
        &format!("count={}", body.servers.len()),
        None,
    );
    state.hub.emit_config_changed(
        "mcp_servers_updated",
        &format!("count={}", body.servers.len()),
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({ "saved": true, "count": body.servers.len() })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct TreeQuery {
    path: Option<String>,
    limit: Option<usize>,
}

/// Bounded recursive listing for @-mention autocomplete. Vendored/build
/// directories are skipped; results are cwd-relative and capped.
async fn fs_tree(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<TreeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    use walkdir::WalkDir;

    let limit = q.limit.unwrap_or(400).min(2000);
    let base = match confined_path(state.core.cwd(), q.path.as_deref().unwrap_or(".")) {
        Some(p) => p,
        None => return (StatusCode::FORBIDDEN, "path outside workspace").into_response(),
    };
    const SKIP: &[&str] = &[
        ".git",
        "target",
        "node_modules",
        "dist",
        "build",
        ".venv",
        "venv",
        "__pycache__",
        ".vak",
        ".next",
        ".cache",
        "coverage",
    ];
    let mut files: Vec<String> = Vec::new();
    for entry in WalkDir::new(&base)
        .max_depth(8)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.file_name()
                .to_str()
                .map(|n| !SKIP.contains(&n) || e.depth() == 0)
                .unwrap_or(true)
        })
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        // Strip against `base` (canonicalized): on macOS /var is a symlink
        // to /private/var, so prefixes against raw cwd never match.
        let Ok(rel) = entry.path().strip_prefix(&base) else {
            continue;
        };
        let mut text = rel.to_string_lossy().replace('\\', "/");
        if let Some(sub) = q
            .path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != ".")
        {
            text = format!("{}/{}", sub.trim_end_matches('/'), text);
        }
        files.push(text);
        if files.len() >= limit {
            break;
        }
    }
    files.sort_unstable();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "files": files, "truncated": files.len() >= limit })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
struct SideBody {
    question: String,
}

/// `/btw`: ask a question using the session's context WITHOUT landing it on
/// the main chain. Mechanics: append the Q + run the turn as a sibling
/// branch (parent = current main tail), then restore the tail so future
/// main turns continue exactly where they were. The side entries stay in
/// the ledger — reconstructable, never deleted.
async fn side_chat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SideBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(mut taken) = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return StatusCode::CONFLICT.into_response(); // main run active
    };
    if let Err(e) = state.core.provider() {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        return provider_unavailable(e);
    }

    let tail_main = taken.tail_id().cloned();
    if let Err(_e) = taken.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text(body.question.clone()),
        meta: None,
    }) {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let side_tx = handle.side_events_tx.clone();
    let side_activity = Arc::new(Mutex::new(Vec::new()));
    let approver: Arc<dyn Approver> = Arc::new(HttpApprover {
        events_tx: handle.events_tx.clone(),
        pending: handle.pending.clone(),
        session_id: handle.id.clone(),
        activity_buffer: side_activity.clone(),
        answerable: true,
    });
    let events = mpsc_to_broadcast(side_tx.clone());
    let cancel = handle
        .side_cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    // A side chat reads this session's context, so it runs under the same
    // ceiling the session was created with.
    let core = handle.core.clone();

    tokio::spawn(async move {
        // No steering on side chats by design: they are read-only Q&A over
        // the session context, not a second control surface.
        let outcome = core
            .run_turn_with(
                taken,
                &body.question,
                cancel,
                Some(approver),
                None,
                None,
                events,
            )
            .await;
        let (summary, is_error) = match &outcome {
            Ok((vak_agent::TurnOutcome::Completed { .. }, _)) => ("completed".to_string(), false),
            Ok((vak_agent::TurnOutcome::Aborted { .. }, _)) => ("aborted".to_string(), false),
            Ok((_, _)) => ("ended".to_string(), false),
            Err(e) => (format!("error: {e}"), true),
        };
        let turn_ok = matches!(&outcome, Ok((_, _)));
        if let Ok((_, mut restored)) = outcome {
            for activity in std::mem::take(
                &mut *side_activity
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ) {
                let _ = restored.append_activity(activity);
            }
            // Rewind the branch pointer to the main line: the side entries
            // remain in the ledger as a sibling branch — reconstructable via
            // their parent chain, invisible to derive_messages().
            if let Some(main_tail) = &tail_main {
                let _ = restored.branch_at(main_tail);
            }
            *handle
                .presentation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                crate::projection::snapshot(&id, &restored);
            *handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(restored);
        }
        let _ = side_tx.send(AgentEvent::RunFinished { summary, is_error });
        // On a failed turn the taken log is gone with the Err — reopen the
        // durable ledger so the session does not stay wedged as
        // "run in progress" forever (found by the v0.6 deployment gate).
        if !turn_ok && let Some(log) = reopen_ledger(&core, &id) {
            *handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(log);
        }
    });

    StatusCode::ACCEPTED.into_response()
}

async fn side_events_sse(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let stream: std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
    > = match state.get(&id) {
        Some(h) => {
            let mut rx = h.side_events_tx.subscribe();
            let _ = rx.try_recv();
            h.side_events_tx.send(AgentEvent::StreamOpened);
            Box::pin(BroadcastStream::new(rx).filter_map(|ev| match ev {
                Ok(framed) => Some(Ok(seq_frame(&framed))),
                Err(_) => Some(Ok(Event::default().data("{\"lagged\":true}"))),
            }))
        }
        None => Box::pin(tokio_stream::once(Ok(
            Event::default().data("{\"error\":\"unknown session\"}")
        ))),
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn side_cancel_run(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    handle
        .side_cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cancel();
    let _ = handle.side_events_tx.send(AgentEvent::RunFinished {
        summary: "cancelled by client".into(),
        is_error: false,
    });
    StatusCode::ACCEPTED
}

#[derive(serde::Deserialize)]
struct BestBody {
    prompt: String,
    n: Option<usize>,
}

/// Best-of-N: fan the same prompt across N isolated git worktrees, each with
/// its own session + event stream. Candidates are compared by diff; `keep`
/// merges a branch, `discard` drops it. Ledger-native: every run is a normal
/// session under the shared store.
async fn start_bestofn(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<BestBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let Some(anchor) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if anchor
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_none()
    {
        return StatusCode::CONFLICT.into_response();
    }
    let provider = match state.core.provider() {
        Ok(p) => p,
        Err(e) => return provider_unavailable(e),
    };

    let n = body.n.unwrap_or(2).clamp(1, 4);
    let repo = state.core.cwd().clone();
    if !vak_core::worktree::is_git_repo(&repo) {
        return StatusCode::CONFLICT.into_response();
    }

    // Create worktrees first; roll back everything on partial failure.
    let mut created: Vec<(String, vak_core::worktree::Worktree)> = Vec::new();
    for i in 0..n {
        // v7 shares its leading chars within one millisecond; disambiguate.
        let rid = format!("{}-{i}", &uuid::Uuid::now_v7().simple().to_string()[..12]);
        match vak_core::worktree::create(&repo, &rid) {
            Ok(wt) => created.push((rid, wt)),
            Err(e) => {
                for (_, wt) in &created {
                    let _ = vak_core::worktree::remove(&repo, wt);
                }
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("worktree create failed: {e}") })),
                )
                    .into_response();
            }
        }
    }

    let mut runs = Vec::new();
    for (rid, wt) in &created {
        match spawn_isolated_run(
            &state,
            provider.clone(),
            rid,
            wt,
            &body.prompt,
            None,
            None,
            None,
            true,
        )
        .await
        {
            Ok(child_id) => {
                state
                    .best_runs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(
                        child_id.clone(),
                        BestRunMeta {
                            repo: repo.clone(),
                            wt_path: wt.path.clone(),
                            branch: wt.branch.clone(),
                        },
                    );
                runs.push(serde_json::json!({
                    "session_id": child_id,
                    "branch": wt.branch,
                    "path": wt.path,
                }));
            }
            Err(e) => {
                for (_, w) in &created {
                    let _ = vak_core::worktree::remove(&repo, w);
                }
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e })),
                )
                    .into_response();
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({ "runs": runs }))).into_response()
}

/// One isolated run inside `wt`: child Core + session + registered handle +
/// fired turn. Shared by best-of-N and the task scheduler. `model_pin`
/// (docs/design/29-personal-os.md P2) overrides the child's provider/model
/// so BOTH main dispatches and any receipts carry the pinned id only — a
/// pinned task never escalates to another model.
#[allow(clippy::too_many_arguments)]
async fn spawn_isolated_run(
    state: &AppState,
    provider: Arc<dyn Provider>,
    rid: &str,
    wt: &vak_core::worktree::Worktree,
    prompt: &str,
    model_pin: Option<&str>,
    agent_id: Option<&str>,
    agent_revision: Option<u64>,
    start_turn: bool,
) -> Result<String, String> {
    let identity = if let Some(agent_id) = agent_id {
        let profiles = agents::effective(&state.active_core())?;
        let profile = profiles
            .iter()
            .find(|profile| profile.id == agent_id)
            .ok_or_else(|| format!("Agent '{agent_id}' no longer exists"))?;
        if !profile.is_admissible() {
            return Err(format!("Agent '{agent_id}' is paused or archived"));
        }
        if let Some(expected) = agent_revision
            && expected != profile.revision
        {
            return Err(format!(
                "Agent '{agent_id}' changed from revision {expected} to {}",
                profile.revision
            ));
        }
        Some(profile.identity())
    } else {
        None
    };
    let child_core = vak_core::Core::new_with_trust(wt.path.clone(), true)
        .map(|c| {
            c.with_agent_identity(identity)
                .with_surface(vak_core::Surface::Background)
                // Unattended, and stamped BEFORE `start_session` composes and
                // freezes the prompt. Stamping afterwards would be too late:
                // the prompt would already have advertised a gated capability
                // that this run can only ever be refused, which is the exact
                // mismatch `vak_core::reach` exists to remove. `begin_turn`
                // installs the matching approver.
                .with_approver_answerable(false)
        })
        .map_err(|e| format!("child core failed: {e}"))?;
    child_core.set_provider_instance(provider);
    child_core.set_sessions_home(state.core.sessions_home());
    if let Some(pin) = model_pin.map(str::trim).filter(|p| !p.is_empty()) {
        let (pin_provider, pin_model) = split_model_pin(pin, &child_core.effective_provider());
        child_core.set_route(pin_provider, pin_model);
    }

    let child_log = child_core
        .start_session()
        .await
        .map_err(|_| "child session failed to start".to_string())?;
    let Some(child_header) = child_log.header() else {
        return Err("child session has no header".to_string());
    };
    let child_id = format!("{}-{}", child_header.session_id, rid);
    let handle = register_handle(
        state,
        child_id.clone(),
        child_log,
        wt.path.clone(),
        child_core.clone(),
    );
    if start_turn {
        begin_turn(&handle, &child_core, prompt, false);
    }
    Ok(child_id)
}

/// Fire a single-turn agent run on a (usually fresh) session handle.
///
/// `attended` says whether anyone is watching this run's event stream. It is
/// not cosmetic: a scheduled routine and a best-of-N leg both arrive here,
/// nobody is subscribed to either, and an approval gate raised on one used
/// to emit an SSE event into the void and then block the run until the
/// process restarted. An unattended run gets an approver that says so, and
/// `Core::with_approver` carries that fact into the prompt so the model is
/// never offered a capability whose gate can only ever be refused.
fn begin_turn(handle: &Arc<SessionHandle>, core: &Core, prompt: &str, attended: bool) {
    let approver: Arc<dyn Approver> = Arc::new(HttpApprover {
        events_tx: handle.events_tx.clone(),
        pending: handle.pending.clone(),
        session_id: handle.id.clone(),
        activity_buffer: handle.activity_buffer.clone(),
        answerable: attended,
    });
    let core = core.clone().with_approver(approver.as_ref());
    let events = mpsc_to_broadcast(handle.events_tx.clone());
    let steering = Arc::new(SteeringQueues::new());
    let cancel = handle
        .cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(log) = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return; // busy — caller should have checked
    };
    let turn_session_id = log
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();
    let core = core.clone();
    let prompt = prompt.to_string();
    let h2 = handle.clone();
    tokio::spawn(async move {
        let outcome = core
            .run_turn_with(
                log,
                &prompt,
                cancel,
                Some(approver),
                None,
                Some(steering.clone()),
                events,
            )
            .await;
        *h2.cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = CancellationToken::new();
        match outcome {
            Ok((_, mut restored)) => {
                for activity in std::mem::take(
                    &mut *h2
                        .activity_buffer
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                ) {
                    let _ = restored.append_activity(activity);
                }
                *h2.presentation
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    crate::projection::snapshot(&turn_session_id, &restored);
                *h2.session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(restored);
                let _ = h2.events_tx.send(AgentEvent::RunFinished {
                    summary: "completed".into(),
                    is_error: false,
                });
            }
            Err(e) => {
                // Same leak class as side chats: restore from the durable
                // ledger so the handle is not wedged on "run in progress".
                if let Some(log) = reopen_ledger(&core, &turn_session_id) {
                    *h2.session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(log);
                }
                let _ = h2.events_tx.send(AgentEvent::RunFinished {
                    summary: format!("error: {e}"),
                    is_error: true,
                });
            }
        }
        drop(steering);
    });
}

async fn keep_best_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let meta = state
        .best_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&id)
        .cloned();
    let Some(meta) = meta else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Merge into the user's checkout. Conflicts/dirty trees surface as errors.
    let out = tokio::process::Command::new("git")
        .args([
            "merge",
            "--no-ff",
            "-m",
            &format!("best-of-n: merge {}", meta.branch),
        ])
        .arg(&meta.branch)
        .current_dir(&meta.repo)
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {
            cleanup_worktree(&state, &id, &meta);
            (StatusCode::OK, Json(serde_json::json!({"kept": id}))).into_response()
        }
        Ok(o) => {
            // Abort any conflicted merge so the tree is not left dirty.
            let _ = tokio::process::Command::new("git")
                .args(["merge", "--abort"])
                .current_dir(&meta.repo)
                .output()
                .await;
            (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "merge failed",
                    "stderr": String::from_utf8_lossy(&o.stderr),
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn discard_best_run(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let meta = state
        .best_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&id)
        .cloned();
    let Some(meta) = meta else {
        return StatusCode::NOT_FOUND.into_response();
    };
    cleanup_worktree(&state, &id, &meta);
    (StatusCode::OK, Json(serde_json::json!({"discarded": id}))).into_response()
}

fn cleanup_worktree(state: &AppState, child_id: &str, meta: &BestRunMeta) {
    let wt = vak_core::worktree::Worktree {
        path: meta.wt_path.clone(),
        branch: meta.branch.clone(),
    };
    let _ = vak_core::worktree::remove(&meta.repo, &wt);
    state
        .best_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(child_id);
}

// ---- PR monitoring (gh-backed) ---------------------------------------------

async fn gh_output(cwd: &std::path::Path, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("gh")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| format!("gh not available: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// One-shot PR status for the session workspace: current branch, the PR
/// attached to it (if any), and its check rollup. Tooling absence surfaces
/// as `{error}` — never a hang, never a panic.
async fn session_pr(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(handle) = state.get(&id) else {
        return Json(serde_json::json!({ "error": "unknown session" }));
    };
    let cwd = handle.cwd.clone();
    let Some(branch) = git_output(&cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).await else {
        return Json(serde_json::json!({ "error": "not a git repository" }));
    };
    let branch = branch.trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        return Json(serde_json::json!({ "error": "detached HEAD" }));
    }

    let raw = match gh_output(
        &cwd,
        &[
            "pr",
            "view",
            &branch,
            "--json",
            "number,title,url,state,mergeable,statusCheckRollup",
        ],
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            // No PR for this branch vs no gh at all — distinguish for UX.
            let msg = e.to_lowercase();
            let kind = if msg.contains("no pull requests") || msg.contains("no merges requested") {
                "no_pr"
            } else {
                "gh_unavailable"
            };
            return Json(serde_json::json!({
                "branch": branch,
                "pr": serde_json::Value::Null,
                "reason": kind,
                "error": e,
            }));
        }
    };

    let Ok(view) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Json(serde_json::json!({ "error": "unparsable gh output", "branch": branch }));
    };
    let mut pass = 0u32;
    let mut fail = 0u32;
    let mut pending = 0u32;
    if let Some(rollup) = view["statusCheckRollup"].as_array() {
        for c in rollup {
            let status = c["status"].as_str().unwrap_or("");
            let conclusion = c["conclusion"].as_str().unwrap_or("");
            match (status, conclusion) {
                (_, "SUCCESS") => pass += 1,
                (_, "FAILURE") | (_, "CANCELLED") | (_, "TIMED_OUT") => fail += 1,
                ("COMPLETED", _) => {}
                _ => pending += 1,
            }
        }
    }
    Json(serde_json::json!({
        "branch": branch,
        "pr": {
            "number": view["number"],
            "title": view["title"],
            "url": view["url"],
            "state": view["state"],
            "mergeable": view["mergeable"],
        },
        "checks": view["statusCheckRollup"],
        "summary": { "pass": pass, "fail": fail, "pending": pending },
    }))
}

#[derive(serde::Deserialize)]
struct PrMergeBody {
    number: u64,
    #[serde(default)]
    method: Option<String>,
}

/// Merge an open PR via gh. `--auto` honors branch protection: gh merges
/// when checks go green.
async fn pr_merge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PrMergeBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let method = body.method.unwrap_or_else(|| "squash".to_string());
    let flag = match method.as_str() {
        "merge" => "--merge",
        "rebase" => "--rebase",
        _ => "--squash",
    };
    let mut args = vec![
        "pr".to_string(),
        "merge".to_string(),
        body.number.to_string(),
        flag.to_string(),
        "--auto".to_string(),
    ];
    if method == "squash" {
        args.push("--delete-branch".to_string());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match gh_output(&handle.cwd, &arg_refs).await {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "merging": body.number })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

// ---- Scheduled tasks (local routines) --------------------------------------

/// Final assistant text of a session's active chain, if any. Used to give
/// routine runs a real answer instead of a status word.
fn last_assistant_text(handle: &SessionHandle) -> Option<String> {
    let guard = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let log = guard.as_ref()?;
    let text = log
        .derive_messages()
        .into_iter()
        .rev()
        .find(|m| m.role == vak_llm::Role::Assistant)
        .map(|m| m.text_content())?;
    let cleaned = crate::projection::clean_scaffolding(&text);
    if cleaned.trim().is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn tasks_file(core: &Core) -> PathBuf {
    let shared = vak_core::tasks::tasks_file(&core.shared_data_home());
    if shared.exists() || core.sessions_home() == core.shared_data_home() {
        shared
    } else {
        let session = vak_core::tasks::tasks_file(&core.sessions_home());
        if session.exists() { session } else { shared }
    }
}

/// Loads `tasks.json` and makes `state.tasks` match it exactly (inserts,
/// updates *and* removals) rather than merging insert-only. The whole
/// read-and-replace runs under `state.tasks`'s lock so it can never
/// interleave with `update_tasks`'s mutate-then-persist below: either this
/// runs entirely before a concurrent create/update/delete's persist, or
/// entirely after, never in the gap between that mutation's memory write
/// and its disk write. Previously an insert-only merge meant an external
/// delete (CLI, desktop app) — or even this process's own `delete_task`
/// racing a scheduler tick — could be silently undone the next time
/// anything called `update_tasks`, since the removed id would still be on
/// disk and get merged straight back into memory.
fn load_tasks(state: &AppState) {
    // The disk read itself must happen while holding the lock, not before
    // it: reading first and only acquiring the lock to apply the snapshot
    // leaves a gap where a concurrent `update_tasks` (create/update/delete)
    // can mutate memory *and* persist in between the read and the replace.
    // This function would then overwrite that fresh insert with the stale
    // pre-mutation snapshot it already had in hand, silently losing it —
    // exactly the kind of loss the merge-vs-replace note below was written
    // to prevent, just moved one step earlier.
    let mut map = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut tasks_vec = match vak_core::tasks::TaskStore::load(&state.core.shared_data_home()) {
        Ok(store) => store.all(),
        Err(_) => Vec::new(),
    };
    if state.core.sessions_home() != state.core.shared_data_home()
        && let Ok(store) = vak_core::tasks::TaskStore::load(&state.core.sessions_home())
    {
        for t in store.all() {
            if !tasks_vec.iter().any(|existing| existing.id == t.id) {
                tasks_vec.push(t);
            }
        }
    }
    *map = tasks_vec.into_iter().map(|t| (t.id.clone(), t)).collect();
    if recover_interrupted_tasks(&mut map) {
        write_tasks_file(state, &map);
    }
}

fn recover_interrupted_tasks(tasks: &mut HashMap<String, TaskDef>) -> bool {
    let mut recovered = false;
    for task in tasks.values_mut() {
        if task.last_run_status.as_deref() == Some("working") {
            task.last_run_status = Some("interrupted".into());
            task.last_delivery_state = Some("pending".into());
            recovered = true;
        }
    }
    recovered
}

/// fsyncs a directory so a prior rename into it is durable across a crash,
/// not just torn-write-free while running. No-op on non-unix, where the
/// rename itself is still atomic but directory fsync isn't a thing.
#[cfg(unix)]
fn sync_tasks_dir(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::File::open(path).and_then(|dir| dir.sync_all())
}
#[cfg(not(unix))]
fn sync_tasks_dir(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// Serializes `map` and writes it to `tasks.json` atomically and durably:
/// write-to-temp, fsync the temp file, rename over the real path, fsync the
/// directory. Mirrors `vak_core::tasks::TaskStore::save` (and the same
/// crash-durability fix) since this is a second, independent writer of the
/// same file — kept in sync here because `AppState.tasks` lives in the
/// server, not in a `TaskStore`.
fn write_tasks_file(state: &AppState, map: &HashMap<String, TaskDef>) {
    let mut list: Vec<TaskDef> = map.values().cloned().collect();
    list.sort_by_key(|t| t.created_at);
    let target = tasks_file(&state.core);
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&list)?;
        let tmp = target.with_extension("json.tmp");
        {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, &target)?;
        if let Some(parent) = target.parent() {
            sync_tasks_dir(parent)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("[scheduler] tasks file save failed: {e}");
    }
}

/// Mutates `state.tasks` and persists the result to disk under a single
/// hold of the lock, so no other reader/writer (in particular
/// `load_tasks`'s scheduler tick) can observe or race the gap between the
/// in-memory change and the on-disk write.
fn update_tasks<T>(state: &AppState, f: impl FnOnce(&mut HashMap<String, TaskDef>) -> T) -> T {
    let mut map = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = f(&mut map);
    write_tasks_file(state, &map);
    result
}

async fn list_tasks(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cwd = state.core.cwd().clone();
    let next_fire = state
        .next_fire
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut mine: Vec<TaskDef> = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .filter(|t| t.cwd == cwd)
        .cloned()
        .collect();
    mine.sort_by_key(|t| t.created_at);
    let now = chrono::Local::now();
    let tasks = mine
        .into_iter()
        .map(|task| {
            let next = task
                .schedule
                .as_deref()
                .and_then(|expr| {
                    next_fire
                        .get(&task.id)
                        .map(|at| at.with_timezone(&Utc))
                        .or_else(|| {
                            vak_core::tasks::cron_next_after(expr, now)
                                .ok()
                                .map(|at| at.with_timezone(&Utc))
                        })
                })
                .or_else(|| {
                    task.last_run_at
                        .map(|last| last + chrono::Duration::seconds(task.interval_secs as i64))
                })
                .or_else(|| Some(now.with_timezone(&Utc)));
            let mut value = serde_json::to_value(task).unwrap_or_else(|_| serde_json::json!({}));
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "next_run_at".into(),
                    next.map(|at| serde_json::Value::String(at.to_rfc3339()))
                        .unwrap_or(serde_json::Value::Null),
                );
                object.insert(
                    "timezone".into(),
                    serde_json::Value::String(now.offset().to_string()),
                );
            }
            value
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "tasks": tasks }))
}

#[derive(serde::Deserialize)]
struct TaskCreateBody {
    name: String,
    /// LLM turn instruction. Optional only for `script:` watchdog tasks.
    #[serde(default)]
    prompt: String,
    #[serde(default = "task_default_interval")]
    interval_secs: u64,
    #[serde(default)]
    deliver_to: Option<String>,
    /// 5-field cron (`m h dom mon dow`, local time) replacing interval ticks.
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    due_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Watchdog shell one-liner; XOR with `prompt`, never touches the LLM.
    #[serde(default)]
    script: Option<String>,
    /// Pin dispatches to one model id (`provider/model` or bare model id).
    #[serde(default)]
    model_pin: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_revision: Option<u64>,
}

fn task_default_interval() -> u64 {
    3600
}

/// Structural validation shared by POST and PATCH: TaskDef::validate owns
/// the prompt-XOR-script and cron-grammar rules; the server adds its
/// transport-shape rules on top. Returns a typed 400 payload on failure.
fn validate_task_fields(task: &TaskDef) -> Result<(), (StatusCode, serde_json::Value)> {
    if task.deliver_to.as_deref().is_some_and(|t| !t.contains(':')) {
        return Err((
            StatusCode::BAD_REQUEST,
            serde_json::json!({
                "error": "deliver_to must be '<surface>:<chat>', e.g. 'log:ops'"
            }),
        ));
    }
    task.validate().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "error": e.to_string() }),
        )
    })
}

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<TaskCreateBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let scheduled = body.schedule.is_some();
    if !scheduled && body.interval_secs < 60 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "interval must be >= 60s" })),
        )
            .into_response();
    }
    let task = TaskDef {
        id: uuid::Uuid::now_v7().to_string(),
        name: body.name,
        prompt: body.prompt,
        interval_secs: body.interval_secs,
        enabled: true,
        cwd: state.core.cwd().clone(),
        created_at: chrono::Utc::now(),
        last_run_at: None,
        last_session_id: None,
        last_summary: None,
        last_result_id: None,
        last_run_status: None,
        last_delivery_state: None,
        last_wt: None,
        deliver_to: body.deliver_to,
        schedule: body.schedule.filter(|s| !s.trim().is_empty()),
        timezone: body.timezone.filter(|s| !s.trim().is_empty()),
        due_at: body.due_at,
        script: body.script.filter(|s| !s.trim().is_empty()),
        model_pin: body.model_pin.filter(|m| !m.trim().is_empty()),
        agent_id: body.agent_id.filter(|m| !m.trim().is_empty()),
        agent_revision: body.agent_revision,
    };
    if let Err((status, payload)) = validate_task_fields(&task) {
        return (status, Json(payload)).into_response();
    }
    update_tasks(&state, |map| {
        map.insert(task.id.clone(), task);
    });
    (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response()
}

#[derive(serde::Deserialize)]
struct TaskPatchBody {
    enabled: Option<bool>,
    name: Option<String>,
    prompt: Option<String>,
    interval_secs: Option<u64>,
    deliver_to: Option<Option<String>>,
    /// Tri-state: absent = keep, null/empty = clear, string = set.
    #[serde(default)]
    schedule: OptionalStr,
    #[serde(default)]
    timezone: OptionalStr,
    #[serde(default)]
    due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    #[serde(default)]
    script: OptionalStr,
    #[serde(default)]
    model_pin: OptionalStr,
    /// Tri-state Agent selection: absent = keep, null/empty = clear, string = set.
    #[serde(default)]
    agent_id: OptionalStr,
    /// Absent = keep, null = clear, number = set.
    #[serde(default)]
    agent_revision: Option<Option<u64>>,
}

/// Distinguishes an absent JSON field from an explicit `null` (which plain
/// `Option<Option<T>>` cannot: both deserialize to outer `None`).
#[derive(Debug, Clone, Default)]
enum OptionalStr {
    #[default]
    Keep,
    Clear,
    Set(String),
}

impl<'de> serde::Deserialize<'de> for OptionalStr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Only reached when the key is present; null → None here.
        match Option::<String>::deserialize(deserializer)? {
            None => Ok(OptionalStr::Clear),
            Some(s) if s.trim().is_empty() => Ok(OptionalStr::Clear),
            Some(s) => Ok(OptionalStr::Set(s)),
        }
    }
}

async fn patch_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TaskPatchBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Some(Some(t)) = &body.deliver_to
        && !t.contains(':')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "deliver_to must be '<surface>:<chat>', e.g. 'log:ops'"
            })),
        )
            .into_response();
    }
    // Mutate, validate and (on success) persist under a single hold of the
    // tasks lock, so the patch can never be observed as committed in memory
    // but not yet on disk (or vice versa) by a concurrent `load_tasks` tick.
    // `update_tasks` can't itself carry an early `return` out of this async
    // fn, so the closure reports outcome via `Result` and the response is
    // built from that afterward.
    let outcome = update_tasks(
        &state,
        |map| -> Result<TaskDef, (StatusCode, serde_json::Value)> {
            let t = map.get_mut(&id).ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    serde_json::json!({ "error": format!("no task '{id}'") }),
                )
            })?;
            // Apply to a candidate and validate BEFORE committing so a
            // rejected patch never leaves half-mutated state behind.
            let mut candidate = t.clone();
            if let Some(v) = body.enabled {
                candidate.enabled = v;
            }
            if let Some(v) = body.name {
                candidate.name = v;
            }
            if let Some(v) = body.prompt {
                candidate.prompt = v;
            }
            if let Some(v) = body.interval_secs
                && v >= 60
            {
                candidate.interval_secs = v;
            }
            if let Some(v) = body.deliver_to {
                candidate.deliver_to = v;
            }
            match body.schedule {
                OptionalStr::Keep => {}
                OptionalStr::Clear => candidate.schedule = None,
                OptionalStr::Set(ref s) => candidate.schedule = Some(s.clone()),
            }
            match body.timezone {
                OptionalStr::Keep => {}
                OptionalStr::Clear => candidate.timezone = None,
                OptionalStr::Set(ref s) => candidate.timezone = Some(s.clone()),
            }
            if let Some(v) = body.due_at {
                candidate.due_at = v;
            }
            match body.script {
                OptionalStr::Keep => {}
                OptionalStr::Clear => candidate.script = None,
                OptionalStr::Set(ref s) => candidate.script = Some(s.clone()),
            }
            match body.model_pin {
                OptionalStr::Keep => {}
                OptionalStr::Clear => candidate.model_pin = None,
                OptionalStr::Set(ref s) => candidate.model_pin = Some(s.clone()),
            }
            match body.agent_id {
                OptionalStr::Keep => {}
                OptionalStr::Clear => {
                    candidate.agent_id = None;
                    candidate.agent_revision = None;
                }
                OptionalStr::Set(ref s) => candidate.agent_id = Some(s.clone()),
            }
            if let Some(revision) = body.agent_revision {
                candidate.agent_revision = revision;
            }
            if let Err((status, payload)) = validate_task_fields(&candidate) {
                return Err((status, payload));
            }
            *t = candidate.clone();
            // Re-enabling reschedules interval tasks from now; dropping the
            // cron marker makes the next tick recompute the schedule from
            // scratch.
            if body.enabled == Some(true) && t.schedule.is_none() {
                t.last_run_at = None;
            }
            Ok(candidate)
        },
    );
    let updated = match outcome {
        Ok(updated) => updated,
        // `update_tasks` still writes tasks.json on the Err path (it can't
        // see into the Result), but the write reproduces the same
        // unmodified map, so a rejected/missing patch persists nothing new.
        Err((status, payload)) => return (status, Json(payload)).into_response(),
    };
    state
        .next_fire
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
    (StatusCode::OK, Json(serde_json::json!(updated))).into_response()
}

async fn delete_task(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    // Remove-and-persist under one lock hold, closing the window where a
    // concurrent scheduler `load_tasks` tick could otherwise re-read the
    // not-yet-updated disk file and resurrect the task right after this
    // handler releases the lock but before it writes tasks.json.
    let removed = update_tasks(&state, |map| map.remove(&id));
    if let Some(task) = removed {
        let idle = task.last_session_id.as_deref().is_none_or(|sid| {
            state.get(sid).is_none_or(|h| {
                h.session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .is_some()
            })
        });
        if idle && let Some(wt) = &task.last_wt {
            let old = vak_core::worktree::Worktree {
                path: wt.path.clone(),
                branch: wt.branch.clone(),
            };
            let _ = vak_core::worktree::remove(&task.cwd, &old);
        }
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Fire a task immediately (also resets its schedule).
async fn run_task_now(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    if fire_task(&state, &id).await.is_some() {
        return StatusCode::ACCEPTED;
    }
    // A scheduler tick may hold the one-shot inflight slot for this script
    // task — the requested execution is happening at this very moment, so
    // report accepted rather than conflict (found by the 0.7 suite: the
    // tick raced run-now on freshly created interval tasks).
    let busy_elsewhere = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&id)
        .map(|t| {
            t.script
                .as_deref()
                .map(str::trim)
                .is_some_and(|s| !s.is_empty())
        })
        .unwrap_or(false)
        && state
            .script_inflight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&id);
    if busy_elsewhere {
        StatusCode::ACCEPTED
    } else {
        StatusCode::CONFLICT
    }
}

/// Replay pending deliveries for one task without executing the task again.
async fn retry_task_delivery(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let records = match delivery::outbox_records(&state.core) {
        Ok(records) => records,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response();
        }
    };
    let jobs = records
        .into_iter()
        .filter(|record| record.state == vak_delivery::outbox::OutboxState::Pending)
        .filter(|record| match &record.job.content {
            vak_delivery::DeliveryContent::Answer(answer) => {
                answer.metadata.get("vak_task_id").map(String::as_str) == Some(id.as_str())
            }
            _ => false,
        })
        .map(|record| record.job.job_id)
        .collect::<Vec<_>>();
    let mut replayed = 0usize;
    let mut failed = 0usize;
    for job_id in jobs {
        match delivery::replay_outbox_job(&state.core, &job_id).await {
            Ok(()) => replayed += 1,
            Err(_) => failed += 1,
        }
    }
    if replayed > 0 && failed == 0 {
        update_tasks(&state, |tasks| {
            if let Some(task) = tasks.get_mut(&id) {
                task.last_delivery_state = Some("delivered".into());
            }
        });
    }
    Json(serde_json::json!({ "replayed": replayed, "failed": failed })).into_response()
}

/// Split a `model_pin` into (provider, model). A bare model id pins only
/// the model and keeps this server's active provider.
pub(crate) fn split_model_pin(pin: &str, current_provider: &str) -> (String, String) {
    match pin.split_once('/') {
        Some((provider, model)) if !provider.trim().is_empty() && !model.trim().is_empty() => {
            (provider.trim().to_string(), model.trim().to_string())
        }
        _ => (current_provider.to_string(), pin.trim().to_string()),
    }
}

/// Spawn one isolated run for `task` if its previous run is idle. Returns
/// the child session id on success. Script tasks take the brokered-bash
/// branch instead: no provider dispatch, no worktree, no child session.
async fn fire_task(state: &AppState, id: &str) -> Option<String> {
    let snapshot = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
        .cloned()?;
    // Previous run still going?
    if let Some(prev) = snapshot.last_session_id.as_deref()
        && state.get(prev).is_some_and(|h| {
            h.session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none()
        })
    {
        return None;
    }
    let script = snapshot
        .script
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(script) = script {
        return fire_script_task(state, &snapshot, script).await;
    }
    let Ok(provider) = state.core.provider() else {
        eprintln!(
            "[scheduler] task '{}' skipped: no provider credential",
            snapshot.name
        );
        return None;
    };
    // Drop the previous worktree (latest-only retention).
    if let Some(wt) = &snapshot.last_wt {
        let old = vak_core::worktree::Worktree {
            path: wt.path.clone(),
            branch: wt.branch.clone(),
        };
        let _ = vak_core::worktree::remove(&snapshot.cwd, &old);
    }
    let rid = format!("task-{}", &uuid::Uuid::now_v7().simple().to_string()[..8]);
    let wt = vak_core::worktree::create(&snapshot.cwd, &rid).ok()?;
    let fired_at_utc = chrono::Utc::now();
    let scheduled_prompt = format!(
        "{}\n\n[Scheduled-run context: fired at UTC {}; local system time {}. Re-evaluate relative dates against this run time unless the request explicitly established a specific date.]",
        snapshot.prompt,
        fired_at_utc.to_rfc3339(),
        fired_at_utc.with_timezone(&chrono::Local).to_rfc3339(),
    );
    let child_id = spawn_isolated_run(
        state,
        provider.clone(),
        &rid,
        &wt,
        &scheduled_prompt,
        snapshot.model_pin.as_deref(),
        snapshot.agent_id.as_deref(),
        snapshot.agent_revision,
        false,
    )
    .await
    .ok()?;

    update_tasks(state, |map| {
        if let Some(t) = map.get_mut(id) {
            if t.due_at.is_some() {
                t.enabled = false;
            }
            t.last_run_at = Some(chrono::Utc::now());
            t.last_session_id = Some(child_id.clone());
            t.last_summary = None;
            t.last_result_id = None;
            t.last_run_status = Some("working".into());
            t.last_delivery_state = Some("pending".into());
            t.last_wt = Some(WtMeta {
                path: wt.path,
                branch: wt.branch,
            });
        }
    });

    // Watcher: record the run's final assistant text on the task when it
    // finishes, and push it out through the gateway when a deliver target
    // is set. The event's `summary` is a status word; the transcript holds
    // the actual answer a phone user should receive.
    if let Some(h) = state.get(&child_id) {
        let st = state.clone();
        let tid = id.to_string();
        let child_handle = h.clone();
        let child_session = child_id.clone();
        let task_name = snapshot.name.clone();
        let deliver_to = snapshot.deliver_to.clone();
        let rx = h.events_tx.subscribe();
        tokio::spawn(async move {
            use tokio_stream::StreamExt;
            use tokio_stream::wrappers::BroadcastStream;
            let mut stream = BroadcastStream::new(rx);
            while let Some(Ok(ev)) = stream.next().await {
                if let AgentEvent::RunFinished { summary, is_error } = ev.event {
                    let text =
                        last_assistant_text(&child_handle).unwrap_or_else(|| summary.clone());
                    let result_id = child_handle
                        .presentation
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .items
                        .iter()
                        .rev()
                        .find_map(|item| {
                            item.outcome
                                .as_ref()
                                .map(|outcome| outcome.result_id.clone())
                        });
                    update_tasks(&st, |map| {
                        if let Some(t) = map.get_mut(&tid) {
                            t.last_summary = Some(text.clone());
                            t.last_result_id = result_id.clone();
                        }
                    });
                    let delivery_state = if let Some(target) = &deliver_to {
                        // Delivery failure must not lose the recorded summary;
                        // it only means this transport could not be reached.
                        gateway::deliver_and_record_with_result(
                            &st.core,
                            target,
                            &format!("routine '{task_name}' finished:\n{text}"),
                            vak_core::inbox::Kind::TaskSummary,
                            format!("routine '{task_name}' finished"),
                            Some(&child_session),
                            Some(&tid),
                            result_id.as_deref(),
                        )
                        .await
                        .unwrap_or("pending")
                    } else {
                        let dedupe_key = Some(format!("inbox|{child_session}"));
                        let _ = vak_core::inbox::record_with_result_and_key(
                            &st.core.shared_data_home(),
                            vak_core::inbox::Kind::TaskSummary,
                            &format!("routine '{task_name}' finished"),
                            &format!("routine '{task_name}' finished:\n{text}"),
                            Some(&child_session),
                            Some(&tid),
                            result_id.as_deref(),
                            dedupe_key.as_deref(),
                        );
                        "inbox"
                    };
                    check_budget_alert(&st, &tid).await;
                    update_tasks(&st, |map| {
                        if let Some(task) = map.get_mut(&tid) {
                            task.last_run_status =
                                Some(if is_error { "failed" } else { "complete" }.into());
                            task.last_delivery_state = Some(delivery_state.into());
                        }
                    });
                    break;
                }
            }
        });
        // Subscribe the completion watcher before starting the turn so fast
        // scripted/provider responses cannot publish RunFinished into a void.
        tokio::task::yield_now().await;
        begin_turn(&h, &h.core, &scheduled_prompt, false);
    }
    Some(child_id)
}

// ---- Watchdog script tasks (docs/design/29-personal-os.md P2) ---------------
//
// A `script:` task NEVER reaches the LLM. The shell line runs through the
// exact brokered bash tool the agent loop uses (`Core::agent_tools()` →
// BrokeredTool → `__tool_worker`), so sandboxing, environment scrubbing,
// process-group isolation, and output caps are identical by construction.
// Zero provider dispatch is a property of the call graph: nothing here can
// name a Provider.

const SCRIPT_TIMEOUT_MS: u64 = 120_000;
/// Upper bound on one background reflection pass (docs/design/29 P1) so a
/// stuck auxiliary stream cannot pin a session handle indefinitely.
const REFLECTION_CALL_TIMEOUT: Duration = Duration::from_secs(120);
/// Fallback delivery surface for error alerts when a watchdog has no
/// `deliver_to`: failures are never silent.
pub(crate) const FALLBACK_ALERT_TARGET: &str = "log:vak";

/// Extract the stdout section from BashTool's combined report
/// ("[stdout]\n…\n[stderr]\n…" or "(no output)"). A literal "[stderr]"
/// inside the script's own stdout ends the section early — watchdogs that
/// print the marker get truncated delivery, never a misparse of stderr.
fn stdout_section(content: &str) -> &str {
    let stdout_part = if let Some(idx) = content.find("[stdout]\n") {
        &content[idx + "[stdout]\n".len()..]
    } else {
        return "";
    };
    match stdout_part.find("\n[stderr]") {
        Some(end) => &stdout_part[..end],
        None => stdout_part,
    }
}

struct ScriptOutcome {
    /// True when the brokered command exited zero within its watchdog.
    ok: bool,
    /// Trimmed stdout on success; combined failure detail otherwise.
    text: String,
}

async fn execute_script(core: &Core, cwd: &std::path::Path, script: &str) -> ScriptOutcome {
    let bash = core.agent_tools().into_iter().find(|t| t.name() == "bash");
    let Some(bash) = bash else {
        return ScriptOutcome {
            ok: false,
            text: "script task failed: no bash tool available".to_string(),
        };
    };
    let ctx = vak_tools::ToolContext {
        cwd: cwd.to_path_buf(),
        cancel: CancellationToken::new(),
        limits: vak_tools::OutputLimits::default(),
        sandbox: core.agent_sandbox(),
        sandbox_sink: None,
        agent_id: core.agent_identity().map(|a| a.id.clone()),
    };
    let args = serde_json::json!({ "command": script, "timeout_ms": SCRIPT_TIMEOUT_MS });
    let out = bash.execute(&args, &ctx).await;
    if out.is_error {
        ScriptOutcome {
            ok: false,
            text: format!("script failed: {}", out.content.trim()),
        }
    } else {
        ScriptOutcome {
            ok: true,
            text: stdout_section(&out.content).trim().to_string(),
        }
    }
}

/// Run one watchdog tick: execute, record, deliver. Empty stdout on
/// success stays silent (zero tokens, zero noise); any failure delivers a
/// typed error alert even without a configured target.
async fn fire_script_task(state: &AppState, task: &TaskDef, script: &str) -> Option<String> {
    // One execution at a time per watchdog: a scheduler tick and run-now
    // must never double-fire (or double-deliver) the same tick.
    if !state
        .script_inflight
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(task.id.clone())
    {
        return None;
    }
    update_tasks(state, |map| {
        if let Some(current) = map.get_mut(&task.id) {
            current.last_run_status = Some("working".into());
            current.last_delivery_state = Some("pending".into());
        }
    });
    let outcome = execute_script(&state.core, &task.cwd, script).await;
    let mut delivery_state = "inbox";
    // Deliver FIRST: once the summary is visible on the task, its delivery
    // attempt has already been made. With zero transports configured the
    // inbox itself is the sink (docs/design/29 P6): a watchdog summary is
    // never lost just because no chat channel exists.
    if outcome.ok {
        if !outcome.text.is_empty() {
            let title = format!("watchdog '{}'", task.name);
            match task.deliver_to.as_deref() {
                Some(target) => {
                    match gateway::deliver_and_record_with_result(
                        &state.core,
                        target,
                        &outcome.text,
                        vak_core::inbox::Kind::TaskSummary,
                        title,
                        None,
                        Some(&task.id),
                        None,
                    )
                    .await
                    {
                        Ok(state) => delivery_state = state,
                        Err(_) => delivery_state = "pending",
                    }
                }
                None => {
                    let _ = vak_core::inbox::record(
                        &state.core.shared_data_home(),
                        vak_core::inbox::Kind::TaskSummary,
                        &title,
                        &outcome.text,
                        None,
                        Some(&task.id),
                    );
                }
            }
        }
    } else {
        eprintln!(
            "[scheduler] watchdog '{}' failed: {}",
            task.name, outcome.text
        );
        let target = task.deliver_to.as_deref().unwrap_or(FALLBACK_ALERT_TARGET);
        match gateway::deliver_and_record_with_result(
            &state.core,
            target,
            &format!("watchdog '{}' alert:\n{}", task.name, outcome.text),
            vak_core::inbox::Kind::TaskSummary,
            format!("failure: watchdog '{}'", task.name),
            None,
            Some(&task.id),
            None,
        )
        .await
        {
            Ok(state) => delivery_state = state,
            Err(_) => delivery_state = "pending",
        }
    }
    update_tasks(state, |map| {
        if let Some(t) = map.get_mut(&task.id) {
            t.last_run_at = Some(chrono::Utc::now());
            t.last_summary = Some(if outcome.ok && outcome.text.is_empty() {
                "(silent tick)".to_string()
            } else {
                outcome.text.clone()
            });
            t.last_run_status = Some(if outcome.ok { "complete" } else { "failed" }.into());
            t.last_delivery_state = Some(delivery_state.into());
        }
    });
    check_budget_alert(state, &task.id).await;
    state
        .script_inflight
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&task.id);
    Some(task.id.clone())
}

// ---- Scheduler (intervals + cron + catch-up, docs/design/29 P2) -------------

/// True when the first scheduled slot STRICTLY AFTER `last_run_at` has
/// already arrived by `now` — i.e. a fire was skipped (typically while the
/// process was down). A run made after the latest slot (manual run-now)
/// covers it, so nothing is missed. Never-run tasks are decided by the
/// caller: without history there is nothing to catch up on, and a freshly
/// created task waits for its first computed slot. Pure; unit-tested
/// against fixed instants.
fn cron_slot_missed(
    expr: &str,
    last_run_at: chrono::DateTime<Utc>,
    now: chrono::DateTime<chrono::Local>,
) -> bool {
    let last_local = last_run_at.with_timezone(&chrono::Local);
    match vak_core::tasks::cron_next_after(expr, last_local) {
        // Instant comparison: correct across DST folds and gaps.
        Ok(next_due) => next_due <= now,
        Err(_) => false,
    }
}

/// Park an unparseable schedule's marker far in the future: validation
/// should have rejected it, so this only contains legacy/corrupt entries.
fn park_marker() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now() + chrono::Duration::days(366)
}

/// One scheduler pass over enabled tasks for this cwd: interval tasks use
/// their `last_run_at`; scheduled tasks consult their in-memory next-fire
/// marker, initializing it to the first future slot when absent (so newly
/// loaded/created tasks do not stampede on startup).
async fn scheduler_tick(state: &AppState) {
    let now_local = chrono::Local::now();
    feeds::scheduled_ingestion(state).await;
    // Reload from disk every tick: tasks.json is shared with the CLI and
    // desktop, so definitions added while the server runs must fire too
    // (found by the v0.6 live battery — CLI-added cron tasks never fired).
    load_tasks(state);
    let due: Vec<String> = {
        let tasks = state
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut markers = state
            .next_fire
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks
            .values()
            .filter(|t| t.enabled && t.cwd.as_path() == state.core.cwd().as_path())
            .filter(|t| match t.due_at {
                Some(due) => chrono::Utc::now() >= due,
                None => match t.schedule.as_deref() {
                    Some(expr) => {
                        if let Some(zone) = t.timezone.as_deref() {
                            let anchor = t.last_run_at.unwrap_or(t.created_at);
                            vak_core::tasks::cron_next_after_timezone(expr, anchor, zone)
                                .map(|next| chrono::Utc::now() >= next)
                                .unwrap_or(false)
                        } else {
                            let marker = markers.entry(t.id.clone()).or_insert_with(|| {
                                vak_core::tasks::cron_next_after(expr, now_local)
                                    .unwrap_or_else(|_| park_marker())
                            });
                            now_local >= *marker
                        }
                    }
                    None => t
                        .last_run_at
                        .map(|l| {
                            (now_local.with_timezone(&Utc) - l).num_seconds()
                                >= t.interval_secs as i64
                        })
                        .unwrap_or(true),
                },
            })
            .map(|t| t.id.clone())
            .collect()
    };
    for id in due {
        let _ = fire_task(state, &id).await;
        advance_marker(state, &id);
    }
}

fn advance_marker(state: &AppState, id: &str) {
    let expr = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
        .and_then(|t| t.schedule.clone());
    if let Some(expr) = expr {
        let next = vak_core::tasks::cron_next_after(&expr, chrono::Local::now())
            .unwrap_or_else(|_| park_marker());
        state
            .next_fire
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_string(), next);
    }
}

/// Startup catch-up (docs/design/29-personal-os.md P2): when enabled and a
/// scheduled task's most recent slot happened after its last run — a slot
/// missed while the process was down — fire it once immediately. Interval
/// tasks keep their self-healing `>= interval` behavior and need nothing.
async fn catch_up_missed_tasks(state: &AppState) {
    if !state.core.config().automation.catch_up_missed {
        return;
    }
    let now_local = chrono::Local::now();
    let due: Vec<String> = {
        let tasks = state
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks
            .values()
            .filter(|t| t.enabled && t.cwd.as_path() == state.core.cwd().as_path())
            .filter_map(|t| {
                let expr = t.schedule.as_deref()?;
                t.last_run_at
                    .is_some_and(|l| cron_slot_missed(expr, l, now_local))
                    .then(|| t.id.clone())
            })
            .collect()
    };
    for id in due {
        eprintln!("[scheduler] catch-up: firing missed slot for task '{id}'");
        let _ = fire_task(state, &id).await;
        advance_marker(state, &id);
    }
}

/// Background loop: evaluates due tasks every 20 seconds. Holds only weak
/// state via `state` clones living inside the router — when the server
/// shuts down the loop dies with the runtime.
pub fn start_scheduler(state: &AppState) {
    load_tasks(state);
    let st = state.clone();
    tokio::spawn(async move { catch_up_missed_tasks(&st).await });
    let st = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            scheduler_tick(&st).await;
        }
    });
    // Proactive heartbeat (docs/design/29-personal-os.md P7): its own
    // per-process timer alongside the task tick; the pass itself re-checks
    // the enabled flag every beat.
    if state.core.config().heartbeat.enabled {
        let st = state.clone();
        tokio::spawn(async move {
            let mut tick =
                tokio::time::interval(std::time::Duration::from_secs(heartbeat::TICK_SECS));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                heartbeat::heartbeat_tick(&st).await;
            }
        });
    }

    // Commitment upkeep runs on its own timer, deliberately NOT gated on
    // `heartbeat.enabled` (docs/design/47-commitment-kernel.md). Heartbeat is
    // an opt-in model pass that costs tokens; this is clock and filesystem
    // work that costs none. Tying durable work's upkeep to an opt-in prober
    // would mean a commitment stopped being durable the moment somebody
    // switched the prober off — and a suspended commitment nobody wakes is
    // indistinguishable from lost work.
    if state.core.config().commitment.enabled {
        let st = state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(COMMITMENT_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let report =
                    vak_core::commitments::maintain(&st.core.sessions_home(), st.core.cwd()).await;
                if report.is_empty() {
                    continue;
                }
                // A commitment waking, lapsing, or being abandoned by policy
                // is a thing that happened without anybody asking for it, so
                // it lands in the attention layer rather than only in a log.
                for id in report.expired.iter().chain(report.escalated.iter()) {
                    let _ = vak_core::inbox::record(
                        &st.core.shared_data_home(),
                        vak_core::inbox::Kind::TaskSummary,
                        "Commitment closed without you",
                        &format!("{id} reached the end of its window or escalation policy."),
                        None,
                        None,
                    );
                }
                for id in report.resumed.iter().chain(report.satisfied.iter()) {
                    eprintln!("[commit] {id} resumed");
                }
            }
        });
    }
}

/// Commitment upkeep cadence. Slower than the heartbeat tick because nothing
/// here is latency-sensitive: a scheduled wake a minute late is fine, and a
/// tighter loop would just re-read the ledger for nothing.
const COMMITMENT_TICK: std::time::Duration = std::time::Duration::from_secs(60);

// ---- Budget alerts (docs/design/29-personal-os.md P2) -----------------------

/// Proactive day-cap alerting, called after every task fire and exposed for
/// surfaces to invoke wherever day spend updates land. Fires at most once
/// per (level, UTC-day window): the audit row recorded by `record_alert`
/// doubles as the once-per-window marker. Delivery reuses the gateway
/// transports (`log:` / `webhook:` / telegram bridge); with no configured
/// targets it falls back to the log surface so an approaching cap is never
/// discovered at denial time.
pub async fn check_budget_alert(state: &AppState, session_id: &str) {
    let Some(cap) = state.core.effective_finops_max_day_usd() else {
        return;
    };
    // Read through the EFFECTIVE sessions home (an embedded server may
    // have relocated it); Core::spend_day_usd pins the constructed path.
    let mut day_total = vak_core::finops::FinOpsLedger::new(&state.core.shared_data_home())
        .day_total_usd(Utc::now());
    if state.core.sessions_home() != state.core.shared_data_home() {
        day_total += vak_core::finops::FinOpsLedger::new(&state.core.sessions_home())
            .day_total_usd(Utc::now());
    }
    let Some(level) = vak_core::finops::alert_level(day_total, cap) else {
        return;
    };
    let home = state.core.shared_data_home();
    if let Some(last) = vak_core::finops::last_alert(&home, level)
        && last.ts.with_timezone(&Utc).date_naive() == Utc::now().date_naive()
    {
        return; // this level already alerted inside the current day window
    }
    if let Err(e) = vak_core::finops::record_alert(&home, level, session_id) {
        eprintln!("[finops] budget-alert ledger write failed: {e}");
    }
    let text = format!(
        "budget alert [{}]: ${:.2} of ${:.2} daily cap",
        level.as_str(),
        day_total,
        cap
    );
    let title = format!("budget alert [{}]", level.as_str());
    let mut targets = configured_delivery_targets(state);
    if targets.is_empty() {
        targets.push(FALLBACK_ALERT_TARGET.to_string());
    }
    for target in targets {
        let _ = gateway::deliver_and_record(
            &state.core,
            &target,
            &text,
            vak_core::inbox::Kind::BudgetAlert,
            title.clone(),
            Some(session_id),
            None,
        )
        .await;
    }
}

/// Every distinct `deliver_to` routing target configured across all known
/// tasks — the server's vocabulary of delivery surfaces.
pub(crate) fn configured_delivery_targets(state: &AppState) -> Vec<String> {
    let mut targets: Vec<String> = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .filter_map(|t| t.deliver_to.clone())
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

// ---- Dev-server lifecycle (preview pane) -----------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LaunchConfig {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

struct ManagedProc {
    child: tokio::process::Child,
    logs: Arc<Mutex<std::collections::VecDeque<String>>>,
}

fn parse_launch_toml(cwd: &std::path::Path) -> Result<Vec<LaunchConfig>, String> {
    let path = cwd.join(".vak/launch.toml");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(rename = "server", default)]
        servers: Vec<LaunchConfig>,
    }
    let f: File = toml::from_str(&raw).map_err(|e| format!("launch.toml: {e}"))?;
    Ok(f.servers)
}

/// Sensible fallback when no launch.toml exists across supported runtimes:
/// Node/JS/TS (Vite, Next, Astro, React, Nuxt), Python (FastAPI/Uvicorn, Flask, Streamlit, Django),
/// Rust (cargo run), Go (go run .), or static HTML (http.server).
fn detect_launch(cwd: &std::path::Path) -> Vec<LaunchConfig> {
    let mut servers = Vec::new();

    // 1. JavaScript / TypeScript projects (package.json)
    let pkg = cwd.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(&pkg)
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw)
    {
        let scripts = &v["scripts"];
        let (script_name, dev_cmd) = if scripts["dev"].is_string() {
            ("dev", "dev")
        } else if scripts["start"].is_string() {
            ("start", "start")
        } else if scripts["serve"].is_string() {
            ("serve", "serve")
        } else {
            ("", "")
        };

        if !script_name.is_empty() {
            let pkg_manager = if cwd.join("pnpm-lock.yaml").exists() {
                ("pnpm", vec!["run".into(), dev_cmd.into()])
            } else if cwd.join("bun.lockb").exists() || cwd.join("bun.lock").exists() {
                ("bun", vec!["run".into(), dev_cmd.into()])
            } else if cwd.join("yarn.lock").exists() {
                ("yarn", vec![dev_cmd.into()])
            } else {
                ("npm", vec!["run".into(), dev_cmd.into()])
            };

            let raw_lower = raw.to_ascii_lowercase();
            let port = if raw_lower.contains("vite") {
                Some(5173)
            } else if raw_lower.contains("astro") {
                Some(4321)
            } else {
                Some(3000)
            };

            servers.push(LaunchConfig {
                name: script_name.into(),
                cmd: pkg_manager.0.into(),
                args: pkg_manager.1,
                port,
            });
        }
    }

    // 2. Python projects
    let manage_py = cwd.join("manage.py");
    if manage_py.exists() {
        servers.push(LaunchConfig {
            name: "django".into(),
            cmd: "python3".into(),
            args: vec!["manage.py".into(), "runserver".into(), "8000".into()],
            port: Some(8000),
        });
    }

    let main_py = cwd.join("main.py");
    let app_py = cwd.join("app.py");
    let py_entry = if main_py.exists() {
        Some(("main", "main.py"))
    } else if app_py.exists() {
        Some(("app", "app.py"))
    } else {
        None
    };

    if let Some((mod_name, file_name)) = py_entry {
        let content = std::fs::read_to_string(cwd.join(file_name))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if content.contains("fastapi") || content.contains("uvicorn") {
            servers.push(LaunchConfig {
                name: "fastapi".into(),
                cmd: "python3".into(),
                args: vec![
                    "-m".into(),
                    "uvicorn".into(),
                    format!("{mod_name}:app"),
                    "--reload".into(),
                    "--port".into(),
                    "8000".into(),
                ],
                port: Some(8000),
            });
        } else if content.contains("flask") {
            servers.push(LaunchConfig {
                name: "flask".into(),
                cmd: "python3".into(),
                args: vec![file_name.into()],
                port: Some(5000),
            });
        } else if content.contains("streamlit") {
            servers.push(LaunchConfig {
                name: "streamlit".into(),
                cmd: "streamlit".into(),
                args: vec![
                    "run".into(),
                    file_name.into(),
                    "--server.port".into(),
                    "8501".into(),
                ],
                port: Some(8501),
            });
        }
    }

    // 3. Rust projects
    let cargo_toml = cwd.join("Cargo.toml");
    if cargo_toml.exists() && (cwd.join("src/main.rs").exists() || cwd.join("src/bin").exists()) {
        servers.push(LaunchConfig {
            name: "cargo".into(),
            cmd: "cargo".into(),
            args: vec!["run".into()],
            port: Some(8080),
        });
    }

    // 4. Go projects
    let go_mod = cwd.join("go.mod");
    let main_go = cwd.join("main.go");
    if go_mod.exists() || main_go.exists() {
        servers.push(LaunchConfig {
            name: "go".into(),
            cmd: "go".into(),
            args: vec!["run".into(), ".".into()],
            port: Some(8080),
        });
    }

    // 5. Static HTML fallback
    if servers.is_empty() && cwd.join("index.html").exists() {
        servers.push(LaunchConfig {
            name: "static".into(),
            cmd: "python3".into(),
            args: vec!["-m".into(), "http.server".into(), "8080".into()],
            port: Some(8080),
        });
    }

    servers
}

async fn get_launch(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(handle) = state.get(&id) else {
        return Json(serde_json::json!({ "error": "unknown session" }));
    };
    let mut servers = match parse_launch_toml(&handle.cwd) {
        Ok(s) => s,
        Err(e) => return Json(serde_json::json!({ "error": e })),
    };
    if servers.is_empty() {
        servers = detect_launch(&handle.cwd);
    }
    let procs = state
        .procs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let list: Vec<serde_json::Value> = servers
        .into_iter()
        .map(|mut s| {
            let key = proc_key(&id, &s.name);
            let running = procs.contains_key(&key);
            if running && s.port.is_none() {
                s.port = None;
            }
            serde_json::json!({
                "name": s.name,
                "cmd": s.cmd,
                "args": s.args,
                "port": s.port,
                "running": running,
            })
        })
        .collect();
    Json(serde_json::json!({ "servers": list }))
}

fn proc_key(session: &str, name: &str) -> String {
    format!("{session}::{name}")
}

async fn wait_for_port(port: u16, timeout: std::time::Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    false
}

#[derive(serde::Deserialize)]
struct LaunchNameBody {
    name: String,
}

async fn start_launch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<LaunchNameBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut servers = match parse_launch_toml(&handle.cwd) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };
    if servers.is_empty() {
        servers = detect_launch(&handle.cwd);
    }
    let Some(cfg) = servers.iter().find(|s| s.name == body.name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown server name" })),
        )
            .into_response();
    };

    let key = proc_key(&id, &cfg.name);
    {
        let procs = state
            .procs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if procs.contains_key(&key) {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "error": "already running" })),
            )
                .into_response();
        }
    }

    let child = tokio::process::Command::new(&cfg.cmd)
        .args(&cfg.args)
        .current_dir(&handle.cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("spawn failed: {e}") })),
            )
                .into_response();
        }
    };

    let logs: Arc<Mutex<std::collections::VecDeque<String>>> =
        Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(500)));
    // Drain stdout+stderr into a bounded ring.
    if let Some(out) = child.stdout.take() {
        let logs_out = logs.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut reader = out;
            let mut buf = [0u8; 1024];
            let mut line = String::new();
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        line.push_str(&String::from_utf8_lossy(&buf[..n]));
                        while let Some(pos) = line.find('\n') {
                            let l: String = line.drain(..=pos).collect();
                            let mut g = logs_out
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if g.len() >= 500 {
                                g.pop_front();
                            }
                            g.push_back(l.trim_end().to_string());
                        }
                    }
                }
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let logs_err = logs.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut reader = err;
            let mut buf = [0u8; 1024];
            let mut line = String::new();
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        line.push_str(&String::from_utf8_lossy(&buf[..n]));
                        while let Some(pos) = line.find('\n') {
                            let l: String = line.drain(..=pos).collect();
                            let mut g = logs_err
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if g.len() >= 500 {
                                g.pop_front();
                            }
                            g.push_back(l.trim_end().to_string());
                        }
                    }
                }
            }
        });
    }

    state
        .procs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, ManagedProc { child, logs });

    // Give the server a moment to bind its port so the preview iframe works
    // immediately after start.
    let listening = match cfg.port {
        Some(p) => wait_for_port(p, std::time::Duration::from_secs(15)).await,
        None => false,
    };
    let _ = logs;

    (
        StatusCode::OK,
        Json(serde_json::json!({ "started": true, "listening": listening })),
    )
        .into_response()
}

async fn stop_launch(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<LaunchNameBody>,
) -> StatusCode {
    let removed = state
        .procs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&proc_key(&id, &body.name));
    match removed {
        Some(mut p) => {
            let _ = p.child.kill().await;
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

async fn launch_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::extract::Query(q): axum::extract::Query<LaunchNameBody>,
) -> Json<serde_json::Value> {
    let procs = state
        .procs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match procs.get(&proc_key(&id, &q.name)) {
        Some(p) => {
            let lines: Vec<String> = p
                .logs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .cloned()
                .collect();
            Json(serde_json::json!({ "lines": lines }))
        }
        None => Json(serde_json::json!({ "lines": [], "error": "not running" })),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod scheduler_pure_tests {
    use super::{TaskDef, cron_slot_missed, recover_interrupted_tasks, stdout_section};
    use chrono::TimeZone;
    use chrono::Utc;
    use std::collections::HashMap;

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .unwrap()
    }

    fn utc(dt: chrono::DateTime<chrono::Local>) -> chrono::DateTime<Utc> {
        dt.with_timezone(&Utc)
    }

    #[test]
    fn restart_recovery_marks_only_interrupted_tasks() {
        let make = |id: &str, status: Option<&str>| TaskDef {
            id: id.into(),
            name: id.into(),
            prompt: "check in".into(),
            interval_secs: 3600,
            enabled: true,
            cwd: std::path::PathBuf::from("/tmp"),
            created_at: Utc::now(),
            last_run_at: None,
            last_session_id: None,
            last_summary: None,
            last_result_id: None,
            last_run_status: status.map(str::to_owned),
            last_delivery_state: Some("pending".into()),
            last_wt: None,
            deliver_to: None,
            schedule: None,
            timezone: None,
            due_at: None,
            script: None,
            model_pin: None,
            agent_id: None,
            agent_revision: None,
        };
        let mut tasks = HashMap::from([
            ("running".into(), make("running", Some("working"))),
            ("done".into(), make("done", Some("complete"))),
        ]);
        assert!(recover_interrupted_tasks(&mut tasks));
        assert_eq!(
            tasks["running"].last_run_status.as_deref(),
            Some("interrupted")
        );
        assert_eq!(tasks["done"].last_run_status.as_deref(), Some("complete"));
    }

    #[test]
    fn missed_slot_matrix() {
        let every_min = "* * * * *";
        // Ran at the current slot → its next slot is in the future.
        assert!(!cron_slot_missed(
            every_min,
            utc(local(2026, 8, 24, 10, 30)),
            local(2026, 8, 24, 10, 30),
        ));
        // Ran yesterday; today's slot already passed → missed.
        assert!(cron_slot_missed(
            "0 12 * * *",
            utc(local(2026, 8, 23, 12, 0)),
            local(2026, 8, 24, 13, 0),
        ));
        // Ran after the latest slot (manual run-now covers it) → not missed.
        assert!(!cron_slot_missed(
            "0 12 * * *",
            utc(local(2026, 8, 24, 12, 30)),
            local(2026, 8, 24, 13, 0),
        ));
        // The slot exactly one step after the last run is due right now.
        assert!(cron_slot_missed(
            "*/15 * * * *",
            utc(local(2026, 8, 24, 10, 30)),
            local(2026, 8, 24, 10, 45),
        ));
        // Bad expression never reports a miss (parked markers handle it).
        assert!(!cron_slot_missed(
            "99 * * * *",
            utc(local(2026, 8, 23, 12, 0)),
            local(2026, 8, 24, 13, 0),
        ));
    }

    #[test]
    fn stdout_section_extracts_only_stdout() {
        assert_eq!(
            stdout_section("[stdout]\nhello\nworld\n\n[stderr]\noops\n"),
            "hello\nworld\n"
        );
        assert_eq!(
            stdout_section(
                "[working directory: /tmp]\n[file: /tmp/res.html]\n[stdout]\nhello\nworld\n\n[stderr]\noops\n"
            ),
            "hello\nworld\n"
        );
        assert_eq!(stdout_section("(no output)"), "");
        assert_eq!(stdout_section(""), "");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod configuration_control_tests {
    use super::*;

    fn control_state(dir: &std::path::Path) -> AppState {
        crate::pin_test_data_home();
        let core = Core::new(dir.to_path_buf()).unwrap();
        core.set_sessions_home(dir.join("home"));
        AppState::new(core)
    }

    // ---- remembering an approval (finding 02) ------------------------------

    /// Put a gate into a session's pending map the way `HttpApprover` does,
    /// so the answer path can be exercised without a provider.
    fn park_gate(handle: &Arc<SessionHandle>, tool: &str, args_json: &str) -> String {
        let id = uuid::Uuid::now_v7().to_string();
        let (respond, _rx) = oneshot::channel();
        handle
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.clone(),
                ApprovalRequest {
                    id: id.clone(),
                    tool: tool.into(),
                    args_json: args_json.into(),
                    reason: "needs approval".into(),
                    requested_at: chrono::Utc::now(),
                    respond: Arc::new(Mutex::new(Some(respond))),
                },
            );
        id
    }

    async fn answer_json(
        state: &AppState,
        session: &str,
        req: &str,
        body: ApprovalBody,
    ) -> serde_json::Value {
        let response = answer_approval(
            State(state.clone()),
            axum::extract::Path((session.to_string(), req.to_string())),
            Json(body),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The mechanism `08-permissions.md` has described since the engine
    /// shipped, and which had no caller on any surface until now.
    #[tokio::test]
    async fn remembering_an_approval_writes_a_scoped_rule_that_applies_at_once() {
        crate::pin_test_data_home();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let session = core.start_session().await.unwrap();
        let id = session.header().unwrap().session_id.clone();
        let handle = register_handle(
            &state,
            id.clone(),
            session,
            core.cwd().clone(),
            core.clone(),
        );

        let req = park_gate(&handle, "bash", r#"{"command":"cargo test --lib"}"#);
        let json = answer_json(
            &state,
            &id,
            &req,
            ApprovalBody {
                approve: true,
                remember: true,
            },
        )
        .await;
        assert_eq!(json["approved"], true);
        assert_eq!(json["learned_rule"], "+bash(cargo *)");
        assert!(json["learn_error"].is_null(), "{json}");

        // The next engine build sees it, with no restart.
        let engine = core
            .build_permission_engine(&core.extra_allow_snapshot())
            .unwrap();
        assert!(matches!(
            engine.evaluate(
                "bash",
                &serde_json::json!({ "command": "cargo build" }),
                vak_permission::Mode::WorkspaceWrite,
                core.cwd()
            ),
            vak_permission::Decision::Allow
        ));
    }

    /// A call that cannot be narrowed safely is still approved — the run is
    /// waiting on it — and simply not remembered, with the reason reported.
    #[tokio::test]
    async fn a_call_that_cannot_be_narrowed_is_approved_but_not_remembered() {
        crate::pin_test_data_home();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let session = core.start_session().await.unwrap();
        let id = session.header().unwrap().session_id.clone();
        let handle = register_handle(
            &state,
            id.clone(),
            session,
            core.cwd().clone(),
            core.clone(),
        );

        let req = park_gate(&handle, "bash", r#"{"command":"echo $(whoami)"}"#);
        let json = answer_json(
            &state,
            &id,
            &req,
            ApprovalBody {
                approve: true,
                remember: true,
            },
        )
        .await;
        assert_eq!(json["approved"], true, "the gate is still answered");
        assert!(json["learned_rule"].is_null());
        assert!(
            json["learn_error"]
                .as_str()
                .unwrap()
                .contains("cannot be narrowed"),
            "{json}"
        );
        assert!(core.extra_allow_snapshot().is_empty());
    }

    /// Remembering a refusal would be a deny rule, which is a different and
    /// much heavier decision than answering one gate.
    #[tokio::test]
    async fn a_refusal_is_never_remembered() {
        crate::pin_test_data_home();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let session = core.start_session().await.unwrap();
        let id = session.header().unwrap().session_id.clone();
        let handle = register_handle(
            &state,
            id.clone(),
            session,
            core.cwd().clone(),
            core.clone(),
        );

        let req = park_gate(&handle, "bash", r#"{"command":"rm -rf /"}"#);
        let json = answer_json(
            &state,
            &id,
            &req,
            ApprovalBody {
                approve: false,
                remember: true,
            },
        )
        .await;
        assert_eq!(json["approved"], false);
        assert!(json["learned_rule"].is_null());
        assert!(core.extra_allow_snapshot().is_empty());
    }

    // ---- unattended runs (findings 08 and 09) ------------------------------

    /// A gate raised where nobody is subscribed used to emit an SSE event
    /// into the void and then block on `rx.await` forever, holding the
    /// session handle open until the process restarted.
    #[tokio::test]
    async fn an_unattended_http_approver_refuses_instead_of_waiting() {
        let events_tx = events::EventBus::new();
        let approver = HttpApprover {
            events_tx,
            pending: Arc::new(Mutex::new(HashMap::new())),
            session_id: "s".into(),
            activity_buffer: Arc::new(Mutex::new(Vec::new())),
            answerable: false,
        };
        assert!(!Approver::answerable(&approver));
        // Returns immediately; without the guard this would block until the
        // 15-minute deadline, which the test would never reach.
        assert!(!approver.approve("bash", "{}", "needs approval").await);
    }

    /// `Core::approver_answerable` is stamped before a run and the approver
    /// is installed at dispatch. They used to be independent, with a comment
    /// asking hosts to keep them in step; the scheduler did not. A
    /// disagreement is now corrected in favour of the approver and recorded.
    #[tokio::test]
    async fn a_stamped_answerability_loses_to_the_installed_approver() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));

        // Default is "attended"; AutoDeny says otherwise.
        assert!(core.approver_answerable());
        let corrected = core.clone().with_approver(&vak_agent::AutoDeny);
        assert!(!corrected.approver_answerable());

        // And the other direction, for a surface that stamped false.
        let stamped = core.clone().with_approver_answerable(false);
        assert!(
            stamped
                .with_approver(&vak_agent::AutoApprove)
                .approver_answerable()
        );
    }

    // ---- gateway approval policy (finding 01) ------------------------------

    /// The setting was readable on three screens and writable nowhere, which
    /// is why every `capability_unreachable` in the audit log had a remedy
    /// no surface could perform.
    #[tokio::test]
    async fn forwarding_can_be_turned_on_and_survives_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let state = control_state(dir.path());
        assert_eq!(state.gateway.approvals_mode(), "deny");

        let response = put_gateway_approvals(
            State(state.clone()),
            Json(GatewayApprovalsBody {
                mode: "forward".into(),
                approver: Some("telegram:12345".into()),
                timeout_secs: Some(60),
                scope: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        // Live, without a restart.
        assert_eq!(state.gateway.approvals_mode(), "forward");
        assert_eq!(
            state.gateway.approver_target().as_deref(),
            Some("telegram:12345")
        );
        assert_eq!(state.gateway.approval_timeout().as_secs(), 60);

        // And on disk, so the next process starts the same way.
        let reloaded = vak_config::load_with_trust(dir.path(), true).unwrap();
        assert_eq!(reloaded.gateway.approvals, "forward");
        assert_eq!(reloaded.gateway.approver.as_deref(), Some("telegram:12345"));
    }

    /// The loader degrades an unbacked `forward` to `deny` with a warning,
    /// which is right for a bad file and wrong for a button press: the
    /// operator would see success and get the opposite setting.
    #[tokio::test]
    async fn forwarding_without_a_chat_is_refused_rather_than_silently_denied() {
        let dir = tempfile::tempdir().unwrap();
        let state = control_state(dir.path());
        for approver in [None, Some("not-a-chat-address".to_string())] {
            let response = put_gateway_approvals(
                State(state.clone()),
                Json(GatewayApprovalsBody {
                    mode: "forward".into(),
                    approver,
                    timeout_secs: None,
                    scope: None,
                }),
            )
            .await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        assert_eq!(state.gateway.approvals_mode(), "deny", "nothing changed");
    }

    /// Going back to `deny` must not leave the old target behind for a later
    /// `forward` to pick up silently.
    #[tokio::test]
    async fn returning_to_deny_clears_the_approver() {
        let dir = tempfile::tempdir().unwrap();
        let state = control_state(dir.path());
        let ok = |body| put_gateway_approvals(State(state.clone()), Json(body));
        assert_eq!(
            ok(GatewayApprovalsBody {
                mode: "forward".into(),
                approver: Some("telegram:1".into()),
                timeout_secs: None,
                scope: None,
            })
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            ok(GatewayApprovalsBody {
                mode: "deny".into(),
                approver: None,
                timeout_secs: None,
                scope: None,
            })
            .await
            .status(),
            StatusCode::OK
        );
        assert!(state.gateway.approver_target().is_none());
        let reloaded = vak_config::load_with_trust(dir.path(), true).unwrap();
        assert!(reloaded.gateway.approver.is_none());
    }

    // ---- permission rules (finding 02) -------------------------------------

    #[tokio::test]
    async fn rules_are_written_validated_and_applied_to_the_next_engine() {
        let dir = tempfile::tempdir().unwrap();
        let state = control_state(dir.path());
        let args = serde_json::json!({ "command": "rm -rf /" });

        let response = put_permission_rules(
            State(state.clone()),
            Json(PermissionRulesBody {
                allow: None,
                ask: None,
                deny: Some(vec!["Bash(rm *)".into()]),
                scope: None,
                agent: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let engine = state.core.build_permission_engine(&[]).unwrap();
        assert!(matches!(
            engine.evaluate(
                "bash",
                &args,
                vak_permission::Mode::FullAccess,
                state.core.cwd()
            ),
            vak_permission::Decision::Deny { .. }
        ));
    }

    /// A half-applied rule set is a permission decision nobody chose, so one
    /// bad spec rejects the whole request and writes nothing.
    #[tokio::test]
    async fn one_malformed_rule_rejects_the_whole_write() {
        let dir = tempfile::tempdir().unwrap();
        let state = control_state(dir.path());
        let response = put_permission_rules(
            State(state.clone()),
            Json(PermissionRulesBody {
                allow: None,
                ask: None,
                deny: Some(vec!["Bash(git *)".into(), "Bash((((".into()]),
                scope: None,
                agent: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let (_, _, deny) = state.core.effective_permission_rules();
        assert!(deny.is_empty(), "nothing may be written: {deny:?}");
    }

    // ---- global writes shadowed by a project pin (finding 07) --------------

    /// The project layer merges last, so a project pin wins. Applying the
    /// global value anyway made the running process disagree with what the
    /// files resolve to — until a restart put it back, which read as the
    /// operator's change being forgotten.
    /// Exclusive access to the process-shared global config layer.
    ///
    /// `VAK_HOME` has no scope smaller than the process, so the pinned test
    /// data home — and with it `global_path()` — is one file shared by every
    /// test in this binary. Restoring it on drop is not enough on its own:
    /// tests run in parallel, so a test that merely *reads* the global
    /// default can observe another test's write in the window before the
    /// restore. Both writers and readers take this guard, which serializes
    /// them and puts the file back afterwards.
    struct GlobalLayerGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        original: Option<String>,
    }

    impl GlobalLayerGuard {
        fn take() -> Self {
            static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
            let lock = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let original = vak_config::global_path().and_then(|p| std::fs::read_to_string(p).ok());
            GlobalLayerGuard {
                _lock: lock,
                original,
            }
        }
    }

    impl Drop for GlobalLayerGuard {
        fn drop(&mut self) {
            let Some(path) = vak_config::global_path() else {
                return;
            };
            match &self.original {
                Some(text) => {
                    let _ = std::fs::write(&path, text);
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// The gateway's own workspace IS the default workspace, so one file
    /// serves as both layers and `load_with_trust` skips the project pass.
    /// Reporting it as "shadowed" told an operator their change would not
    /// take effect when it would — observed live, on a real install, right
    /// after the shadow check shipped.
    #[tokio::test]
    async fn a_global_write_on_the_default_workspace_is_not_its_own_shadow() {
        crate::pin_test_data_home();
        let _restore = GlobalLayerGuard::take();
        let Some(global) = vak_config::global_path() else {
            return;
        };
        let workspace = global
            .parent()
            .and_then(|vak| vak.parent())
            .expect("global config sits under <workspace>/.vak/")
            .to_path_buf();
        std::fs::create_dir_all(global.parent().expect("parent")).unwrap();
        std::fs::write(&global, "permission_mode = \"read-only\"\n").unwrap();
        vak_core::trust::record(&workspace).unwrap();

        let core = Core::new_with_trust(workspace.clone(), true).unwrap();
        core.set_sessions_home(workspace.join(".sessions"));
        let state = AppState::new(core.clone());
        assert_eq!(
            vak_config::project_path(core.cwd()),
            global,
            "this test is only meaningful when the two layers are one file"
        );

        let response = patch_global_config(
            State(state.clone()),
            Json(ConfigPatch {
                permission_mode: Some("workspace-write".into()),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json["shadowed_by_project"],
            serde_json::json!([]),
            "one file cannot shadow itself: {json}"
        );
        assert_eq!(
            core.effective_permission_mode(),
            vak_config::PermissionMode::WorkspaceWrite,
            "and the change must actually be in force"
        );
    }

    #[tokio::test]
    async fn a_global_write_under_a_project_pin_persists_without_taking_effect() {
        crate::pin_test_data_home();
        let _restore = GlobalLayerGuard::take();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
        std::fs::write(
            dir.path().join(".vak/config.toml"),
            "permission_mode = \"read-only\"\n",
        )
        .unwrap();
        vak_core::trust::record(dir.path()).unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        assert_eq!(
            core.effective_permission_mode(),
            vak_config::PermissionMode::ReadOnly
        );

        let response = patch_global_config(
            State(state.clone()),
            Json(ConfigPatch {
                permission_mode: Some("full-access".into()),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(
            core.effective_permission_mode(),
            vak_config::PermissionMode::ReadOnly,
            "the project pin still decides what this process runs at"
        );
        let global = vak_config::load_with_trust(dir.path(), true).unwrap();
        assert_eq!(
            global.permission_mode,
            vak_config::PermissionMode::ReadOnly,
            "and what the files resolve to agrees"
        );
    }

    #[tokio::test]
    async fn cross_process_mode_refresh_revokes_live_capability_before_apply() {
        crate::pin_test_data_home();
        // Reads the default effective mode, which the shared global layer
        // decides — so it belongs under the same guard as the writers.
        let _global = GlobalLayerGuard::take();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let session = core.start_session().await.unwrap();
        let id = session.header().unwrap().session_id.clone();
        let handle = register_handle(&state, id, session, core.cwd().clone(), core.clone());
        assert!(!handle.cancel.lock().unwrap().is_cancelled());

        vak_config::persist_project_preferences(
            dir.path(),
            None,
            None,
            Some(19),
            Some(vak_config::PermissionMode::ReadOnly),
            None,
            None,
        )
        .unwrap();
        refresh_control_plane(&state);

        assert_eq!(core.effective_max_turns(), 19);
        assert_eq!(
            core.effective_permission_mode(),
            vak_config::PermissionMode::ReadOnly
        );
        assert!(handle.cancel.lock().unwrap().is_cancelled());
    }

    /// The bug this locks in: before `effective_memory_*` existed,
    /// `Core::config().memory.*` was read directly at every call site, so
    /// a live PATCH — or another process persisting a change to disk —
    /// silently did nothing until the process restarted. Mirrors
    /// `cross_process_mode_refresh_revokes_live_capability_before_apply`'s
    /// shape for the memory tier instead of permission mode.
    #[tokio::test]
    async fn cross_process_memory_refresh_takes_effect_without_restart() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        assert!(core.effective_memory_search_enabled(), "default is on");

        vak_config::persist_project_memory_prefs(dir.path(), Some(false), None, None, None)
            .unwrap();
        core.refresh_persisted_preferences().unwrap();

        assert!(
            !core.effective_memory_search_enabled(),
            "a disk change from another process must reach an already-running Core"
        );
        // Untouched flags keep their default, proving the write was
        // scoped to exactly the one field this call named.
        assert!(core.effective_memory_write_enabled());
    }

    /// `PATCH /config` end to end: persists to disk, applies live
    /// immediately (no restart), and a field the PATCH didn't mention
    /// keeps its current value rather than reverting to whatever was on
    /// disk before this call.
    #[tokio::test]
    async fn patch_config_memory_flags_apply_live_and_persist() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());

        let response = patch_config(
            State(state.clone()),
            Json(ConfigPatch {
                memory_write_enabled: Some(false),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        assert!(
            !core.effective_memory_write_enabled(),
            "must apply live without a restart"
        );
        assert!(
            core.effective_memory_search_enabled(),
            "a field this PATCH never mentioned must keep its value"
        );

        // Persisted to disk, not just the in-process override — a fresh
        // Core over the same cwd sees it too.
        let fresh = Core::new(dir.path().to_path_buf()).unwrap();
        fresh.set_sessions_home(dir.path().join("home"));
        assert!(!fresh.effective_memory_write_enabled());
    }

    /// Same live-without-restart guarantee as memory, for the `workers`
    /// toggle newly surfaced in the admin console's Settings page — it was
    /// previously read from `Core::config()` directly at both call sites,
    /// so a PATCH would have silently done nothing.
    #[tokio::test]
    async fn patch_config_workers_applies_live_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        assert!(core.effective_workers(), "default is on");

        let response = patch_config(
            State(state.clone()),
            Json(ConfigPatch {
                workers: Some(false),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            !core.effective_workers(),
            "must apply live without a restart"
        );

        let fresh = Core::new(dir.path().to_path_buf()).unwrap();
        fresh.set_sessions_home(dir.path().join("home"));
        assert!(!fresh.effective_workers(), "must be persisted to disk too");
    }

    /// `PATCH /finops` sets a cap live and persists it; an explicit `null`
    /// clears a previously-set cap rather than being indistinguishable
    /// from the field being absent (the exact bug `deserialize_present`
    /// exists to prevent, exercised here for a fresh field).
    #[tokio::test]
    async fn patch_finops_sets_and_clears_caps_live_and_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        assert_eq!(core.effective_finops_max_run_usd(), None);

        let status = patch_finops(
            State(state.clone()),
            Json(FinopsPatch {
                max_run_usd: Some(Some(5.0)),
                max_day_usd: None,
                agent: None,
            }),
        )
        .await;
        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(core.effective_finops_max_run_usd(), Some(5.0));

        let fresh = Core::new(dir.path().to_path_buf()).unwrap();
        fresh.set_sessions_home(dir.path().join("home"));
        assert_eq!(
            fresh.effective_finops_max_run_usd(),
            Some(5.0),
            "must be persisted to disk too"
        );

        // Explicit null clears it back to "no cap".
        let status = patch_finops(
            State(state.clone()),
            Json(FinopsPatch {
                max_run_usd: Some(None),
                max_day_usd: None,
                agent: None,
            }),
        )
        .await;
        assert_eq!(status.status(), StatusCode::OK);
        assert_eq!(
            core.effective_finops_max_run_usd(),
            None,
            "explicit null must clear the cap, not be a no-op"
        );
    }

    #[tokio::test]
    async fn patch_finops_rejects_negative_cap() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let status = patch_finops(
            State(state),
            Json(FinopsPatch {
                max_run_usd: Some(Some(-1.0)),
                max_day_usd: None,
                agent: None,
            }),
        )
        .await;
        assert_eq!(status.status(), StatusCode::BAD_REQUEST);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod voice_admission_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn rolling_voice_admission_enforces_limit_and_resets_window() {
        let requests = Arc::new(Mutex::new((Instant::now(), 0)));
        let state = requests;
        let mut window = state.lock().unwrap();
        assert!(window.1 < 2);
        window.1 += 1;
        assert!(window.1 < 2);
        window.1 += 1;
        assert!(window.1 >= 2);
        drop(window);

        // The production helper resets by elapsed time; exercise its exact
        // state shape without making the test sleep.
        let mut window = state.lock().unwrap();
        window.0 = Instant::now() - Duration::from_secs(61);
        drop(window);
        let state =
            AppState::new(Core::new(tempfile::tempdir().unwrap().path().to_path_buf()).unwrap());
        *state.voice_requests.lock().unwrap() = (Instant::now() - Duration::from_secs(61), 2);
        assert!(admit_voice_request(&state, 2));
        assert_eq!(state.voice_requests.lock().unwrap().1, 1);
    }

    #[test]
    fn zero_voice_limit_fails_closed() {
        let state =
            AppState::new(Core::new(tempfile::tempdir().unwrap().path().to_path_buf()).unwrap());
        assert!(!admit_voice_request(&state, 0));
        assert_eq!(state.voice_requests.lock().unwrap().1, 0);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod sandbox_promotion_tests {
    use super::*;

    #[tokio::test]
    async fn candidate_export_and_promotion_records_observed_verification() {
        crate::pin_test_data_home();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core);
        let scratch = dir.path().join(".vak/scratch/e1");
        tokio::fs::create_dir_all(&scratch).await.unwrap();
        tokio::fs::write(scratch.join("result.txt"), "candidate")
            .await
            .unwrap();

        let response = export_sandbox_candidate(
            State(state.clone()),
            Json(SandboxCandidateBody {
                candidate_id: "c1".into(),
                source: ".vak/scratch/e1".into(),
                destination: ".".into(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let candidate: vak_sandbox::CandidateManifest = serde_json::from_slice(&bytes).unwrap();

        let response = promote_sandbox_candidate(
            State(state.clone()),
            Json(SandboxPromotionBody {
                candidate,
                record_id: Some("p1".into()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let receipt: vak_sandbox::PromotionReceipt = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(receipt.verification.len(), 1);
        assert_eq!(
            tokio::fs::read_to_string(dir.path().join("result.txt"))
                .await
                .unwrap(),
            "candidate"
        );
        assert_eq!(
            vak_sandbox::load_records(&dir.path().join(".vak/sandbox/records.jsonl"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn sandbox_execution_ledger_survives_live_bus_loss() {
        let dir = tempfile::tempdir().unwrap();
        let start = AgentEvent::Sandbox(vak_tools::SandboxEvent::ExecutionStarted {
            execution_id: "child-exec".into(),
            owner_session_id: Some("child-session".into()),
            tool: "bash".into(),
            code_preview: "echo hi".into(),
            language: "bash".into(),
            scratch_dir: ".vak/scratch/child-exec".into(),
        });
        let finish = AgentEvent::Sandbox(vak_tools::SandboxEvent::ExecutionFinished {
            execution_id: "child-exec".into(),
            exit_code: 0,
            duration_ms: 42,
            artifacts: vec!["result.txt".into()],
        });
        append_session_sandbox_event(dir.path(), "parent-session", &start);
        append_session_sandbox_event(dir.path(), "parent-session", &finish);
        let path = dir.path().join("sandbox/executions/parent-session.jsonl");
        let lines = std::fs::read_to_string(path).unwrap();
        assert_eq!(lines.lines().count(), 2);
        assert!(lines.contains("child-session"));
        assert!(lines.contains("ExecutionFinished"));
    }

    #[test]
    fn detect_launch_identifies_vite_package_json() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = serde_json::json!({
            "scripts": { "dev": "vite" },
            "devDependencies": { "vite": "^5.0.0" }
        });
        std::fs::write(dir.path().join("package.json"), pkg.to_string()).unwrap();
        let found = detect_launch(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "dev");
        assert_eq!(found[0].cmd, "npm");
        assert_eq!(found[0].args, vec!["run", "dev"]);
        assert_eq!(found[0].port, Some(5173));
    }

    #[test]
    fn detect_launch_identifies_python_fastapi_and_flask() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("main.py"),
            "from fastapi import FastAPI\napp = FastAPI()\n",
        )
        .unwrap();
        let found = detect_launch(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "fastapi");
        assert_eq!(found[0].cmd, "python3");
        assert_eq!(
            found[0].args,
            vec!["-m", "uvicorn", "main:app", "--reload", "--port", "8000"]
        );
        assert_eq!(found[0].port, Some(8000));
    }

    #[test]
    fn detect_launch_identifies_cargo_and_go() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("go.mod"), "module demo\n").unwrap();

        let found = detect_launch(dir.path());
        let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"cargo"));
        assert!(names.contains(&"go"));
    }

    #[test]
    fn detect_launch_falls_back_to_static_html() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Test</h1>\n").unwrap();
        let found = detect_launch(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "static");
        assert_eq!(found[0].cmd, "python3");
        assert_eq!(found[0].args, vec!["-m", "http.server", "8080"]);
        assert_eq!(found[0].port, Some(8080));
    }

    #[tokio::test]
    async fn activate_all_and_deactivate_all_presentations() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            vak_store::presentation::PresentationStore::new(dir.path().join("presentations.json"));
        let mut library = vak_presentation::PresentationLibrary::default();
        for seed in vak_presentation::seeds::built_in_seed_pack() {
            library.register(seed).unwrap();
        }
        store.save(&library).unwrap();

        // Verify activate all
        let mut latest_by_id: std::collections::BTreeMap<String, u64> =
            std::collections::BTreeMap::new();
        for def in library.definitions() {
            let entry = latest_by_id
                .entry(def.spec.id.clone())
                .or_insert(def.spec.revision);
            if def.spec.revision > *entry {
                *entry = def.spec.revision;
            }
        }
        let mut activated = 0;
        for (id, rev) in latest_by_id {
            if library
                .activate(&id, rev, vak_presentation::LibraryScope::User, "user")
                .is_ok()
            {
                activated += 1;
            }
        }
        assert_eq!(activated, 72);
        assert_eq!(library.activations().len(), 72);

        // Verify deactivate all
        let spec_ids: Vec<String> = library
            .activations()
            .iter()
            .filter(|a| a.scope == vak_presentation::LibraryScope::User && a.owner == "user")
            .map(|a| a.spec_id.clone())
            .collect();
        for id in spec_ids {
            library.deactivate(&id, vak_presentation::LibraryScope::User, "user");
        }
        assert_eq!(library.activations().len(), 0);
    }
}
