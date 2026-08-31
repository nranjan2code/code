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
//! - `GET  /gateway/status`           → gateway enabled flag + binding table
//! - `DELETE /gateway/bindings/:key`  → unbind a surface from its session

mod admin;
mod admin_ui;
mod channels;
mod core_pool;
mod delivery;
pub mod discord;
mod events;
mod feeds;
pub mod gateway;
mod heartbeat;
mod operations;
mod presentation;
mod rate_limit;
pub mod slack;
pub mod telegram;

use std::collections::HashMap;
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

pub(crate) struct SessionHandle {
    pub(crate) id: String,
    /// Workspace this session's tools/diffs operate in (main cwd, or a
    /// best-of-N worktree).
    pub(crate) cwd: PathBuf,
    pub(crate) session: Arc<Mutex<Option<SessionLog>>>,
    pub(crate) steering: Arc<SteeringQueues>,
    /// Cancel for the CURRENT run only; replaced with a fresh token when a
    /// run ends so one `/cancel` doesn't poison every later run.
    pub(crate) cancel: Arc<std::sync::Mutex<CancellationToken>>,
    pub(crate) events_tx: broadcast::Sender<AgentEvent>,
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
    pub(crate) side_events_tx: broadcast::Sender<AgentEvent>,
    pub(crate) side_cancel: Arc<std::sync::Mutex<CancellationToken>>,
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
        }
    }

    /// Force-enable the gateway (`serve --gateway`) before the state is
    /// shared; the config gate alone governs every other entry point.
    pub fn enable_gateway(&mut self) {
        if let Some(gw) = Arc::get_mut(&mut self.gateway) {
            gw.set_enabled(true);
        }
    }

    fn get(&self, id: &str) -> Option<Arc<SessionHandle>> {
        self.sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
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

struct HttpApprover {
    events_tx: broadcast::Sender<AgentEvent>,
    pending: Arc<Mutex<HashMap<String, ApprovalRequest>>>,
    /// Owning session, so admin-console surfaces can attribute gates.
    session_id: String,
    activity_buffer: Arc<Mutex<Vec<vak_session::ActivityRecord>>>,
}

#[async_trait::async_trait]
impl Approver for HttpApprover {
    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool {
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
        let approved = rx.await.unwrap_or(false);
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
        .route("/sessions/{id}/launch", get(get_launch))
        .route("/sessions/{id}/launch/start", post(start_launch))
        .route("/sessions/{id}/launch/stop", post(stop_launch))
        .route("/sessions/{id}/launch/logs", get(launch_logs))
        .route("/sessions/{id}/run", post(run_prompt))
        .route("/sessions/{id}/steering", post(send_steering))
        .route("/sessions/{id}/cancel", post(cancel_run))
        .route("/sessions/{id}/subagents", get(list_subagents))
        .route(
            "/sessions/{id}/subagents/{child}/steer",
            post(steer_subagent),
        )
        .route("/sessions/{id}/subagents/{child}/stop", post(stop_subagent))
        .route("/sessions/{id}/approvals/{req_id}", post(answer_approval))
        .route("/sessions/{id}/events", get(events_sse))
        .route("/sessions/{id}/presentation", get(presentation_snapshot))
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
        .route("/fs/tree", get(fs_tree))
        .route("/config", get(get_config).patch(patch_config))
        .route("/config/global", axum::routing::patch(patch_global_config))
        .route("/config/mode", post(set_permission_mode))
        .route("/config/mcp", get(get_mcp_servers).put(put_mcp_servers))
        .route(
            "/config/mcp/global",
            get(get_global_mcp_servers).put(put_global_mcp_servers),
        )
        .route(
            "/config/integrations/tavily",
            get(get_tavily).put(put_tavily),
        )
        .route("/config/integrations/tavily/disable", post(disable_tavily))
        .route("/config/hooks", get(get_hooks).put(put_hooks))
        .route(
            "/config/hooks/global",
            get(get_global_hooks).put(put_global_hooks),
        )
        .route(
            "/config/key",
            put(put_provider_key).delete(delete_provider_key),
        )
        .route(
            "/config/telegram-token",
            put(put_telegram_token).delete(delete_telegram_token),
        )
        // Per-surface form of the same thing (docs/design/34 Phase 3);
        // the telegram-specific route above stays for compatibility.
        .route(
            "/config/bot-token/{surface}",
            put(put_bot_token).delete(delete_bot_token),
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
        .route("/finops", get(finops_status).patch(patch_finops))
        .route("/voice/speak", post(voice_speak))
        .route("/memory", get(list_memory).post(append_memory))
        .route("/memory/cleanup", post(cleanup_memory))
        .route(
            "/memory/{note_id}",
            axum::routing::patch(amend_memory_note).delete(forget_memory_note),
        )
        .route("/doctor", get(doctor_report))
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
        .merge(admin_ui::routes())
        .with_state(state)
}

/// Service-control plane over vak-ops: lets TUI/desktop/tray agree on the
/// same truth (docs/design/28-operations.md).
fn ops_payload(cfg: &vak_ops::OpsConfig) -> serde_json::Value {
    let st = |svc| vak_ops::status(svc, cfg);
    serde_json::json!({
        "gateway": { "state": st(vak_ops::Service::Gateway).to_string() },
        "telegram": { "state": st(vak_ops::Service::Telegram).to_string() },
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
    let telegram = vak_ops::status(vak_ops::Service::Telegram, cfg);
    serde_json::json!({
        "gateway": { "state": gateway.to_string() },
        "telegram": { "state": telegram.to_string() },
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
            Some(serde_json::json!({
                "session_id": handle.id,
                "workspace": handle.cwd,
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
    let mut bound_targets = std::collections::HashSet::new();
    let mut bindings = gateway
        .into_iter()
        .map(|(target, binding)| {
            bound_targets.insert(target.clone());
            serde_json::json!({
                "target": target,
                "session_id": binding.session_id,
                "workspace": binding.workspace,
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
    for entry in state.gateway.allowlist_snapshot() {
        if entry.status != gateway::AllowlistStatus::Allowed || bound_targets.contains(&entry.key) {
            continue;
        }
        bindings.push(serde_json::json!({
            "target": entry.key,
            "session_id": null,
            "workspace": state.gateway.workspace_for_entry(&state.core, &entry.key),
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
        "gateway": {
            "enabled": state.gateway.enabled,
            "approvals": { "pending": approvals, "mode": state.gateway.approvals_mode(), "approver": state.gateway.approver_target() },
            "bindings": bindings,
        },
        "pool": {
            "max": state.core.config().gateway.core_pool_max,
            "idle_secs": state.core.config().gateway.core_pool_idle_secs,
            "entries": pool,
        },
        "runs": runs,
        "tasks": operation_tasks(&state),
        "outbox": { "pending": outbox_pending, "dead_letter": outbox_dead, "records": outbox, "error": outbox_error },
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
            let after = delivery::outbox_records(&state.core)
                .ok()
                .and_then(|records| {
                    records
                        .into_iter()
                        .find(|record| record.job.job_id == job_id)
                })
                .map(|record| state_label(record.state).to_string())
                .unwrap_or_else(|| "not found".to_string());
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
const DEFAULT_VOICE_NAME: &str = "Kore";

#[derive(serde::Deserialize)]
struct VoiceSpeakBody {
    text: String,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    chat_key: Option<String>,
    #[serde(default)]
    voice_override: Option<vak_config::VoiceConfig>,
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

    // Resolution order: explicit override > resolved chat/bot voice > a
    // built-in default. The chat lookup mirrors `core_for_entry`'s own
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
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_VOICE_NAME.to_string());
    let persona = resolved.as_ref().and_then(|v| v.persona.clone());

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

    let config = vak_llm::google_live::GoogleLiveConfig::new(api_key);
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
        Some(voice_name.as_str()),
        &cancel,
    )
    .await;

    match result {
        Ok(wav) => {
            receipt.record(
                vak_llm::AttemptReason::Initial,
                vak_llm::FailureDomain::Unknown,
                vak_llm::Settlement::Ok,
                started.elapsed().as_millis() as u64,
                None,
                None,
            );
            (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "audio/wav")],
                wav,
            )
                .into_response()
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

/// FinOps projection from the append-only cost ledger. Unknown-priced rows
/// are retained as `unknown_rows`; they are never reported as zero spend.
/// Caps are read through the live-effective accessors, not `Core::config()`
/// directly, so a PATCH from `patch_finops` (below) is reflected
/// immediately rather than only after a restart.
async fn finops_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let ledger = vak_core::finops::FinOpsLedger::new(&state.core.sessions_home());
    let rows = ledger.all_rows();
    let now = chrono::Utc::now();
    let day_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Utc).single());
    let day_rows: Vec<&vak_core::finops::CostRow> = rows
        .iter()
        .filter(|r| day_start.is_some_and(|start| r.ts >= start))
        .collect();
    let day_usd: f64 = day_rows.iter().filter_map(|r| r.usd).sum();
    let unknown_rows = day_rows.iter().filter(|r| r.usd.is_none()).count();
    let mut by_provider = std::collections::BTreeMap::<String, (f64, u64)>::new();
    let mut by_model = std::collections::BTreeMap::<String, (f64, u64)>::new();
    for row in &day_rows {
        let usd = row.usd.unwrap_or(0.0);
        let p = by_provider.entry(row.provider.clone()).or_default();
        p.0 += usd;
        p.1 += 1;
        let m = by_model.entry(row.model.clone()).or_default();
        m.0 += usd;
        m.1 += 1;
    }
    let rollup =
        |source: std::collections::BTreeMap<String, (f64, u64)>| -> Vec<serde_json::Value> {
            source.into_iter().map(|(name, (usd, calls))| serde_json::json!({ "name": name, "usd": usd, "calls": calls })).collect()
        };
    let daily: Vec<serde_json::Value> = ledger
        .daily_totals(now, FINOPS_TREND_DAYS)
        .into_iter()
        .map(|(date, usd)| serde_json::json!({ "date": date.to_string(), "usd": usd }))
        .collect();
    Json(serde_json::json!({
        "day_usd": day_usd,
        "run_cap_usd": state.core.effective_finops_max_run_usd(),
        "day_cap_usd": state.core.effective_finops_max_day_usd(),
        "unknown_rows": unknown_rows,
        "total_rows": rows.len(),
        "by_provider": rollup(by_provider),
        "by_model": rollup(by_model),
        "daily": daily,
        "recent_alerts": recent_budget_alerts(&state.core.sessions_home(), 10),
    }))
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
    if vak_config::persist_project_finops_caps(state.core.cwd(), body.max_run_usd, body.max_day_usd)
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    state
        .core
        .apply_persisted_finops_caps(body.max_run_usd, body.max_day_usd);
    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "finops_caps_patched",
        &format!(
            "run={:?} day={:?}",
            state.core.effective_finops_max_run_usd(),
            state.core.effective_finops_max_day_usd()
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

async fn ops_action(
    State(state): State<AppState>,
    Path((service, action)): Path<(String, String)>,
    axum::extract::Query(q): axum::extract::Query<OpsActionQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let svc = match service.as_str() {
        "gateway" => Some(vak_ops::Service::Gateway),
        "telegram" => Some(vak_ops::Service::Telegram),
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
    let before = vak_ops::status(svc, &cfg).to_string();
    let requested_at = Utc::now();
    let (result, succeeded) = match action.as_str() {
        "start" => {
            let ok = vak_ops::start(svc, &cfg);
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
            let ok = vak_ops::stop(svc, &cfg);
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
            let ok = vak_ops::restart(svc, &cfg);
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
    let after = vak_ops::status(svc, &cfg).to_string();
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

async fn list_memory(State(state): State<AppState>) -> Json<serde_json::Value> {
    let home = state.core.sessions_home();
    let mut blocks: Vec<serde_json::Value> = vak_core::memory::list_notes(&home, state.core.cwd())
        .iter()
        .map(|n| note_payload(n, "workspace"))
        .collect();
    blocks.extend(
        vak_core::memory::list_profile_notes(&home)
            .iter()
            .map(|n| note_payload(n, "profile")),
    );
    Json(serde_json::json!({ "notes": blocks }))
}

async fn cleanup_memory(State(state): State<AppState>) -> Json<serde_json::Value> {
    let report = vak_core::memory::cleanup_artifacts(
        &state.core.sessions_home(),
        std::time::Duration::from_secs(86_400),
    );
    vak_core::security_events::record(
        &state.core.sessions_home(),
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
}

/// Append a note to either tier. Keeps gateway/desktop/CLI symmetric —
/// every surface writes through the same validated core API.
async fn append_memory(
    State(state): State<AppState>,
    Json(body): Json<AppendMemoryBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
    let scope = body.scope.unwrap_or(MemoryScope::Workspace);
    let kind = body.kind.unwrap_or_else(|| "fact".to_string());
    let tag = body.tag.unwrap_or_default();
    let session = body.session_id.unwrap_or_else(|| "http".to_string());
    let result = match scope {
        MemoryScope::Workspace => vak_core::memory::append_note(
            &home,
            state.core.cwd(),
            &kind,
            &tag,
            &session,
            &body.text,
        ),
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

fn memory_store_path(state: &AppState, scope: MemoryScope) -> PathBuf {
    let home = state.core.sessions_home();
    match scope {
        MemoryScope::Workspace => home
            .join("memory")
            .join(vak_core::memory::hash_cwd(state.core.cwd()))
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
    let path = memory_store_path(&state, q.scope.unwrap_or_default());
    match vak_core::memory::forget_note(&path, &note_id) {
        Ok(bytes) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
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
}

#[derive(serde::Deserialize, Default)]
struct MemoryScopeQuery {
    #[serde(default)]
    scope: Option<MemoryScope>,
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
    let path = memory_store_path(&state, body.scope.unwrap_or_default());
    match vak_core::memory::amend_note(&path, &note_id, &body.text) {
        Ok(()) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
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
    vak_core::learning::list_proposals(&core.sessions_home(), core.cwd())
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

async fn list_proposals_route(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "proposals": proposals_payload(&state.core) }))
}

async fn promote_proposal(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match vak_core::learning::promote(&state.core.sessions_home(), state.core.cwd(), &id) {
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
) -> axum::response::Response {
    use axum::response::IntoResponse;
    match vak_core::learning::reject(&state.core.sessions_home(), state.core.cwd(), &id) {
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
            (token.clone(), state.core.sessions_home()),
            require_bearer,
        ))
        .layer(cors);
    // Local routines: fires due scheduled tasks while this server lives.
    start_scheduler(&state);
    delivery::start_replay(&state.core);
    // Start MCP tool discovery now, not at the first session's first turn:
    // a process sits idle through real human seconds before any message
    // arrives, so this spends that idle time on the same background
    // warm-up `system_prompt()` would otherwise trigger far too late to
    // matter for a single-turn task (see `Core::warm_mcp`).
    state.core.warm_mcp();
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

pub async fn serve(core: Core, addr: std::net::SocketAddr) -> std::io::Result<()> {
    serve_with(core, addr, false).await
}

/// `force_gateway` mirrors `serve --gateway`: enable routing regardless of
/// the (untrusted-stripped) project config.
pub async fn serve_with(
    core: Core,
    addr: std::net::SocketAddr,
    force_gateway: bool,
) -> std::io::Result<()> {
    // Local-only does not mean safe-by-default: any local process could
    // reach an unauthenticated agent and drive arbitrary tool execution
    // plus self-approval. Every serve() instance gets a per-process
    // bearer token; /health stays open.
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let actual_addr = listener.local_addr()?;
    let (app, token) = secured_router_with_port(core, force_gateway, actual_addr.port());
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

/// Paths that must be reachable without a token: health probe, the SPA
/// shell (static assets carry no data), and the login endpoint itself.
fn auth_exempt_path(path: &str) -> bool {
    path == "/health"
        || path == "/admin"
        || path == "/admin/"
        || path == "/admin/login"
        || path == "/admin/favicon.svg"
        || path == "/admin/vak-icon.png"
        || path.starts_with("/admin/assets/")
        || path == "/favicon.ico"
        || path == "/favicon.svg"
}

#[cfg(test)]
mod auth_exempt_path_tests {
    use super::auth_exempt_path;

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

pub(crate) async fn require_bearer(
    State((token, home)): State<(String, std::path::PathBuf)>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
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
    // Browser surfaces authenticate once via /admin/login which sets an
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
    // performs the `/admin/login` cookie exchange -- that is the admin
    // console's browser flow, not the desktop's. The query parameter is
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
    let query_token = req.uri().query().and_then(|q| {
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
    });
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
        "sandbox": state.core.effective_sandbox_name(),
        "context_window": state.core.config().context_window,
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
) -> Arc<SessionHandle> {
    let (events_tx, _) = broadcast::channel(1024);
    let (side_events_tx, _) = broadcast::channel(1024);
    let presentation = Arc::new(Mutex::new(crate::presentation::snapshot(&id, &session)));
    let mut presentation_rx = events_tx.subscribe();
    let presentation_state = presentation.clone();
    let presentation_activities = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(SessionHandle {
        id: id.clone(),
        cwd,
        session: Arc::new(Mutex::new(Some(session))),
        steering: Arc::new(SteeringQueues::new()),
        cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
        events_tx,
        pending: Arc::new(Mutex::new(HashMap::new())),
        activity_buffer: presentation_activities.clone(),
        presentation,
        subscribed: Arc::new(tokio::sync::Notify::new()),
        side_events_tx,
        side_cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
    });
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let observer_id = id.clone();
        runtime.spawn(async move {
            loop {
                match presentation_rx.recv().await {
                    Ok(event) => {
                        if let Some(projected) =
                            crate::presentation::live_event(&observer_id, event.clone())
                        {
                            crate::presentation::apply_stream_event(
                                &mut presentation_state
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                                projected,
                            );
                        }
                        let activity = match event {
                            AgentEvent::SubagentStarted { label } => {
                                Some(vak_session::ActivityRecord {
                                    activity_id: format!("subagent-{label}"),
                                    turn: None,
                                    kind: vak_session::ActivityKind::Subagent,
                                    status: vak_session::ActivityStatus::Running,
                                    label,
                                    detail: Some("Subagent started".into()),
                                    data: std::collections::BTreeMap::new(),
                                })
                            }
                            AgentEvent::SubagentFinished {
                                label,
                                is_error,
                                elapsed_ms,
                            } => Some(vak_session::ActivityRecord {
                                activity_id: format!("subagent-{label}"),
                                turn: None,
                                kind: vak_session::ActivityKind::Subagent,
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
    handle
}

async fn create_session(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    refresh_control_plane(&state);
    let session = match state.core.start_session().await {
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
    register_handle(&state, id.clone(), session, state.core.cwd().clone());

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
    let Ok(read) = std::fs::read_dir(&dir) else {
        return false;
    };
    for project in read.flatten() {
        let candidate = project.path().join(format!("{session_id}.jsonl"));
        if candidate.is_file()
            && let Ok(stats) = store.import_session(home, &candidate)
        {
            return stats.entries_indexed > 0 || stats.skipped > 0;
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
    match state.core.open_session(&body.session_id).await {
        Ok(session) => {
            let id = session
                .header()
                .map(|h| h.session_id.clone())
                .unwrap_or_else(|| body.session_id.clone());
            // The header id can differ from the requested one; if that handle
            // is already live, keep it rather than replacing it.
            if state.get(&id).is_none() {
                register_handle(&state, id.clone(), session, state.core.cwd().clone());
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
    let dir = vak_session::SessionPath::sessions_dir(&state.core.sessions_home(), state.core.cwd());
    let archive_map = read_archive(&state.core);
    let deleted_map = read_deleted(&state.core);
    let mut sessions = Vec::new();
    let Ok(read) = std::fs::read_dir(&dir) else {
        return Json(serde_json::json!({ "sessions": sessions }));
    };
    for entry in read.flatten() {
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
        let (created_at, title, entries, cwd) = summarize_jsonl(&path);
        // Header-only sessions are abandoned drafts (for example, creating a
        // task and immediately switching away). Keep the ledger append-only,
        // but do not let empty drafts accumulate in the task switcher.
        if entries <= 1 {
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
            "entries": entries,
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
) -> (Option<String>, Option<String>, u64, Option<String>) {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else {
        return (None, None, 0, None);
    };
    let mut reader = std::io::BufReader::new(file);
    let mut created_at = None;
    let mut title = None;
    let mut cwd = None;
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
                        vak_session::EntryPayload::Activity(_) => {}
                        vak_session::EntryPayload::Work(_) => {}
                    }
                }
                if title.is_some() && entries > 400 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    (created_at, title, entries, cwd)
}

#[derive(serde::Deserialize)]
struct RunBody {
    prompt: String,
    /// Optional run-scoped work profile. `managed` creates and persists a
    /// work contract before the agent can execute tools.
    #[serde(default)]
    work_mode: Option<String>,
    /// Optional base64 images appended to the prompt as vision content
    /// (docs/design/22-gateway.md media passthrough).
    #[serde(default)]
    attachments: Vec<RunAttachment>,
    /// Goal mode (docs/design/27 Phase H): durable objective; completion
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

pub(crate) fn mpsc_to_broadcast(tx: broadcast::Sender<AgentEvent>) -> mpsc::Sender<AgentEvent> {
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
    let Some(taken) = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return StatusCode::CONFLICT.into_response(); // run already active
    };
    if let Err(e) = state.core.provider() {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        return provider_unavailable(e);
    }

    // Give SSE consumers a moment to attach so terminal events are seen.
    let _ = tokio::time::timeout(Duration::from_secs(2), handle.subscribed.notified()).await;

    let approver: Arc<dyn Approver> = Arc::new(HttpApprover {
        events_tx: handle.events_tx.clone(),
        pending: handle.pending.clone(),
        session_id: handle.id.clone(),
        activity_buffer: handle.activity_buffer.clone(),
    });
    let events = mpsc_to_broadcast(handle.events_tx.clone());
    let steering = handle.steering.clone();
    let cancel = handle
        .cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let core = state.core.clone();

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
                    let _ = session_log.append_activity(activity);
                }
                *handle
                    .presentation
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    crate::presentation::snapshot(&run_id, &session_log);
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
                        crate::presentation::snapshot(&run_id, &restored);
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
        drop(steering);
    });

    StatusCode::ACCEPTED.into_response()
}

#[derive(serde::Deserialize)]
struct SteeringBody {
    text: String,
    /// Optional base64 images appended to the steered prompt, mirroring
    /// /run so queued input is never degraded to bare text.
    #[serde(default)]
    attachments: Vec<RunAttachment>,
}

async fn send_steering(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SteeringBody>,
) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
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
    StatusCode::ACCEPTED
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
    deny_pending_approvals(&handle);
    let _ = handle.events_tx.send(AgentEvent::RunFinished {
        summary: "cancelled by client".into(),
        is_error: false,
    });
    StatusCode::ACCEPTED
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

// ---- Subagent control plane -------------------------------------------------
//
// Children already stream lifecycle/tool events into the parent session's
// SSE channel; these endpoints add the missing half: listing, steering, and
// stopping from a remote surface. Scope-checked against the parent so one
// session can never touch another's child.

async fn list_subagents(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let children = state.core.subagents().active_for(&id);
    Json(serde_json::json!({ "subagents": children }))
}

#[derive(serde::Deserialize)]
struct SubagentSteerBody {
    text: String,
}

async fn steer_subagent(
    State(state): State<AppState>,
    Path((id, child)): Path<(String, String)>,
    Json(body): Json<SubagentSteerBody>,
) -> StatusCode {
    if state.core.subagents().parent_of(&child).as_deref() != Some(id.as_str()) {
        return StatusCode::NOT_FOUND;
    }
    if state.core.subagents().steer(&child, &body.text) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::CONFLICT
    }
}

async fn stop_subagent(
    State(state): State<AppState>,
    Path((id, child)): Path<(String, String)>,
) -> StatusCode {
    if state.core.subagents().parent_of(&child).as_deref() != Some(id.as_str()) {
        return StatusCode::NOT_FOUND;
    }
    if state.core.subagents().stop(&child) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::CONFLICT
    }
}

#[derive(serde::Deserialize)]
struct ApprovalBody {
    approve: bool,
}

async fn answer_approval(
    State(state): State<AppState>,
    Path((id, req_id)): Path<(String, String)>,
    Json(body): Json<ApprovalBody>,
) -> StatusCode {
    // Look the request up in THIS session's pending map only: approvals
    // are never resolvable across sessions.
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    match handle
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&req_id)
    {
        Some(req) => {
            req.respond(body.approve);
            StatusCode::OK
        }
        None => StatusCode::NOT_FOUND,
    }
}

/// Dispatch forensics (docs/design/27 Phase A): the session's work
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

/// Flow names discovered under `<sessions_home>/flow-runs` (doc 27 G).
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

async fn events_sse(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let stream: std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
    > = match state.get(&id) {
        Some(h) => {
            // Subscribe BEFORE notifying so no early events are missed,
            // then mark the stream open so clients can safely trigger a run.
            let mut rx = h.events_tx.subscribe();
            let _ = rx.try_recv();
            h.subscribed.notify_one();
            let _ = h.events_tx.send(AgentEvent::StreamOpened);
            Box::pin(BroadcastStream::new(rx).filter_map(|ev| match ev {
                Ok(agent_event) => Some(Ok(
                    Event::default().data(serde_json::to_string(&agent_event).unwrap_or_default()),
                )),
                Err(_) => Some(Ok(Event::default().data("{\"lagged\":true}"))),
            }))
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
            return Json(crate::presentation::snapshot(&id, session)).into_response();
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
        Some(session) => Json(crate::presentation::snapshot(&id, &session)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response(),
    }
}

async fn presentation_events_sse(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

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
                    .map(|session| crate::presentation::snapshot(&id, session))
                    .unwrap_or_else(|| {
                        handle
                            .presentation
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone()
                    })
            };
            let initial = vak_delivery::OutputStreamEvent::Snapshot { timeline: initial };
            let initial = tokio_stream::once(Ok(
                Event::default().data(serde_json::to_string(&initial).unwrap_or_default())
            ));
            handle.subscribed.notify_one();
            let live_id = id.clone();
            let live = BroadcastStream::new(rx).filter_map(move |event| match event {
                Ok(event) => {
                    crate::presentation::live_event(&live_id, event).map(|projected| {
                        Ok(Event::default()
                            .data(serde_json::to_string(&projected).unwrap_or_default()))
                    })
                }
                Err(_) => Some(Ok(Event::default().data("{\"lagged\":true}"))),
            });
            Box::pin(initial.chain(live))
        }
        None => match open_historical_session(&state, &id) {
            Some(session) => {
                let event = vak_delivery::OutputStreamEvent::Snapshot {
                    timeline: crate::presentation::snapshot(&id, &session),
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
            "configuration_mismatch": s.header().is_some_and(|header| {
                header.contract.provider != state.core.effective_provider()
                    || header.contract.model != state.core.effective_model()
            }),
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
                "configuration_mismatch": s.header().is_some_and(|header| {
                    header.contract.provider != state.core.effective_provider()
                        || header.contract.model != state.core.effective_model()
                }),
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
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Some(handle) = state.get(&id) {
        let guard = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(s) = guard.as_ref() else {
            return Json(serde_json::json!({ "error": "run in progress" })).into_response();
        };
        let md = vak_core::transcript_md::render_markdown(&s.derive_messages());
        return markdown_response(md);
    }
    match open_historical_session(&state, &id) {
        Some(s) => markdown_response(vak_core::transcript_md::render_markdown(
            &s.derive_messages(),
        )),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "unknown session" })),
        )
            .into_response(),
    }
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

/// Historical sessions live on disk but not in the in-memory handle map
/// (a fresh server process starts with an empty map). Open read-only for
/// export/inspection without mutating run bookkeeping.
fn open_historical_session(state: &AppState, id: &str) -> Option<vak_session::SessionLog> {
    let path = state
        .core
        .sessions_home()
        .join("sessions")
        .join(vak_core::memory::hash_cwd(state.core.cwd()))
        .join(format!("{id}.jsonl"));
    vak_session::SessionLog::open(path).ok()
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
    let root = state.core.sessions_home().join("sessions");
    let entries = std::fs::read_dir(root).ok()?;
    for project in entries.flatten().filter(|entry| entry.path().is_dir()) {
        let path = project.path().join(format!("{id}.jsonl"));
        if let Some(header) = read(&path) {
            return Some(header);
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
    let home = state.core.sessions_home();
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
    Json(serde_json::json!({ "entries": entries, "unread_count": unread_count }))
}

async fn inbox_unread_count(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "count": vak_core::inbox::unread_count(&state.core.sessions_home())
    }))
}

/// Idempotent read-state: a tombstone append via `inbox::ack`. An unknown id
/// is a 404; re-acking reports `{acked:false}` instead of writing twice.
async fn inbox_ack(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let home = state.core.sessions_home();
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
    // A session with no snapshots yet has no directory; that's an empty
    // list, not an error.
    let list = match vak_core::checkpoints::list(&state.core.sessions_home(), &id) {
        Ok(list) => list,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
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
    // Best-of-N children captured inside their worktrees; attached handles
    // know that cwd. Everything else restores into the workspace root.
    let cwd = state
        .get(&id)
        .map(|h| h.cwd.clone())
        .unwrap_or_else(|| state.core.cwd().clone());
    let cp = match vak_core::checkpoints::load(&state.core.sessions_home(), &id, seq) {
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
    core.sessions_home().join("archive.json")
}

fn deleted_path(core: &Core) -> PathBuf {
    core.sessions_home().join("deleted.json")
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

#[derive(serde::Deserialize)]
struct ArchiveBody {
    archived: bool,
}

async fn set_archived(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ArchiveBody>,
) -> axum::response::Response {
    let dir = vak_session::SessionPath::sessions_dir(&state.core.sessions_home(), state.core.cwd());
    if !dir.join(format!("{id}.jsonl")).is_file() {
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
    let dir = vak_session::SessionPath::sessions_dir(&state.core.sessions_home(), state.core.cwd());
    if !dir.join(format!("{id}.jsonl")).is_file() {
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
    // Drop the session's cached FinOps spend gate along with it (docs/design/27
    // Phase D) — otherwise a long-running server accumulates one entry per
    // session ever seen, forever.
    state.core.forget_spend_gate(&id);
    Json(serde_json::json!({ "deleted": id })).into_response()
}

async fn delete_all_archived(State(state): State<AppState>) -> axum::response::Response {
    // `archive.json`/`deleted.json` are shared, global-by-session-id maps —
    // not scoped to a workspace — but a ledger file only ever lives under
    // *this* process's own `sessions_dir(sessions_home, cwd)`. Single-item
    // delete already respects that boundary by checking the file exists
    // there before acting; this bulk form iterated every archived id in the
    // global map with no such check, so running it from one workspace
    // could soft-delete archived sessions that belong to a completely
    // different project.
    let dir = vak_session::SessionPath::sessions_dir(&state.core.sessions_home(), state.core.cwd());
    let archive = read_archive(&state.core);
    let local_archived: Vec<String> = archive
        .into_iter()
        .filter(|(id, archived)| *archived && dir.join(format!("{id}.jsonl")).is_file())
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

async fn list_skills(State(state): State<AppState>) -> Json<serde_json::Value> {
    // `path` and `scope` tell the reader WHERE a skill came from. Discovery
    // reads two roots (`<cwd>/.vak/skills` then `<sessions_home>/skills`), and
    // a workspace skill is a very different trust proposition from a user-wide
    // one -- the admin console groups by this.
    let workspace_root = state.core.cwd().join(".vak/skills");
    let skills: Vec<serde_json::Value> = state
        .core
        .skills_with_shadowed()
        .iter()
        .map(|s| {
            let scope = if s.path.starts_with(&workspace_root) {
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
    Json(serde_json::json!({ "skills": skills }))
}

async fn list_commands(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({ "commands": state.core.custom_commands().into_iter().map(|command| serde_json::json!({
        "name": command.name, "description": command.description, "source": command.source,
    })).collect::<Vec<_>>() }),
    )
}

#[derive(Debug, serde::Deserialize)]
struct PluginMutation {
    path: Option<PathBuf>,
    #[serde(default)]
    scope: Option<InstallScope>,
    #[serde(default)]
    allow_unlicensed: bool,
}

#[derive(Debug, serde::Deserialize)]
struct PluginSourceMutation {
    path: PathBuf,
    #[serde(default)]
    label: String,
    #[serde(default = "default_marketplace_trust")]
    trust: MarketplaceTrust,
    key_id: Option<String>,
    public_key: Option<String>,
    signature: Option<String>,
}

fn default_marketplace_trust() -> MarketplaceTrust {
    MarketplaceTrust::ManualReview
}

fn plugin_store(state: &AppState, scope: InstallScope) -> PluginStore {
    let root = match scope {
        InstallScope::User => state.core.sessions_home(),
        InstallScope::Workspace => state.core.cwd().join(".vak"),
    };
    PluginStore::new(root)
}

#[derive(Debug, serde::Deserialize)]
struct PluginScopeQuery {
    scope: Option<InstallScope>,
}

#[derive(Debug, serde::Deserialize)]
struct PluginCatalogQuery {
    scope: Option<InstallScope>,
    q: Option<String>,
}

fn requested_plugin_scopes(scope: Option<InstallScope>) -> Vec<InstallScope> {
    scope.map_or_else(
        || vec![InstallScope::User, InstallScope::Workspace],
        |scope| vec![scope],
    )
}

async fn list_plugins(
    State(state): State<AppState>,
    Query(query): Query<PluginScopeQuery>,
) -> Json<serde_json::Value> {
    let mut plugins = Vec::new();
    for scope in requested_plugin_scopes(query.scope) {
        if let Ok(items) = plugin_store(&state, scope).list() {
            plugins.extend(items.into_iter().map(|plugin| {
                serde_json::json!({
                    "name": plugin.name,
                    "version": plugin.version,
                    "digest": plugin.digest,
                    "description": plugin.description,
                    "format": plugin.format,
                    "scope": plugin.scope,
                    "enabled": plugin.enabled,
                    "trace_id": plugin.trace_id,
                    "capabilities": plugin.capabilities,
                    "warnings": plugin.warnings,
                })
            }));
        }
    }
    Json(serde_json::json!({ "plugins": plugins }))
}

async fn plugin_audit(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut events = Vec::new();
    for scope in [InstallScope::User, InstallScope::Workspace] {
        if let Ok(registry) = plugin_store(&state, scope).load() {
            events.extend(registry.audit);
        }
    }
    events.sort_by_key(|event| event.at_unix);
    Json(serde_json::json!({ "audit": events }))
}

async fn plugin_invocations(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut events = Vec::new();
    for scope in [InstallScope::User, InstallScope::Workspace] {
        if let Ok(items) = plugin_store(&state, scope).invocations() {
            events.extend(items);
        }
    }
    events.sort_by_key(|event| event.at_unix);
    Json(serde_json::json!({ "invocations": events }))
}

async fn plugin_sources(
    State(state): State<AppState>,
    Query(query): Query<PluginScopeQuery>,
) -> Json<serde_json::Value> {
    let mut sources = Vec::new();
    for scope in requested_plugin_scopes(query.scope) {
        if let Ok(items) = plugin_store(&state, scope).list_sources() {
            sources.extend(items);
        }
    }
    Json(serde_json::json!({ "sources": sources }))
}

async fn plugin_catalog(
    State(state): State<AppState>,
    Query(query): Query<PluginCatalogQuery>,
) -> Json<serde_json::Value> {
    let needle = query.q.as_deref().unwrap_or_default().trim().to_lowercase();
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for scope in requested_plugin_scopes(query.scope) {
        let store = plugin_store(&state, scope);
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
    Json(serde_json::json!({ "entries": entries, "errors": errors }))
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
    plugin_result(
        plugin_store(&state, InstallScope::Workspace).register_catalog_source_with_signature(
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
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace))
            .set_source_enabled(&id, true),
    )
}

async fn plugin_source_disable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace))
            .set_source_enabled(&id, false),
    )
}

async fn plugin_key_revoke(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace))
            .set_key_revoked(&id, true)
            .map(|_| serde_json::json!({"revoked": id})),
    )
}

async fn plugin_key_restore(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace))
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
    plugin_result(plugin_store(&state, scope).install_local(
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
    plugin_result(plugin_store(&state, scope).update_local(
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
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace)).enable(&name),
    )
}

async fn plugin_disable(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace)).disable(&name),
    )
}

async fn plugin_rollback(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace)).rollback(&name),
    )
}

async fn plugin_remove(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<PluginScopeQuery>,
) -> axum::response::Response {
    plugin_result(
        plugin_store(&state, query.scope.unwrap_or(InstallScope::Workspace)).remove(&name),
    )
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

async fn read_file(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(path) = confined_path(state.core.cwd(), &q.path) else {
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
                    body["editable"] = serde_json::json!(false);
                }
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "file not found").into_response(),
    }
}

#[derive(serde::Deserialize)]
struct WriteBody {
    path: String,
    content: String,
}

async fn write_file(State(state): State<AppState>, Json(body): Json<WriteBody>) -> StatusCode {
    let Some(path) = confined_path(state.core.cwd(), &body.path) else {
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

#[derive(serde::Deserialize)]
struct ModeBody {
    mode: String,
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

async fn set_permission_mode(
    State(state): State<AppState>,
    Json(body): Json<ModeBody>,
) -> StatusCode {
    match parse_mode(&body.mode) {
        Some(mode) => {
            let old = state.core.effective_permission_mode();
            if vak_config::persist_project_preferences(
                state.core.cwd(),
                None,
                None,
                None,
                Some(mode),
                None,
            )
            .is_err()
            {
                return StatusCode::INTERNAL_SERVER_ERROR;
            }
            apply_permission_mode(&state, mode, true);
            if old != mode {
                vak_core::security_events::record(
                    &state.core.sessions_home(),
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

fn apply_permission_mode(state: &AppState, mode: vak_config::PermissionMode, persisted: bool) {
    if state.core.effective_permission_mode() == mode {
        return;
    }
    if persisted {
        state.core.apply_persisted_permission_mode(mode);
    } else {
        state.core.set_permission_mode(mode);
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
    }
}

fn refresh_control_plane(state: &AppState) {
    let old_mode = state.core.effective_permission_mode();
    if let Ok(mode) = state.core.refresh_persisted_preferences()
        && !state.core.permission_mode_runtime_pinned()
        && mode != old_mode
    {
        apply_permission_mode(state, mode, true);
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
        providers.push(serde_json::json!({
            "name": name,
            "env_var": Core::provider_env_var(&name),
            "pool_env_var": pool_env_var(&name),
            "pool_size": credential_ids.len(),
            "credential_ids": credential_ids,
            "requires_key": requires_key,
            "configured": configured,
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
}

/// Revoke a provider key. Reports when the variable is still set in the
/// real environment, since that keeps the provider authenticated and no
/// app-level action can change it.
async fn delete_provider_key(
    State(state): State<AppState>,
    Json(body): Json<ProviderRef>,
) -> axum::response::Response {
    match state.core.remove_provider_key(&body.provider) {
        Ok(removed) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "provider_key_removed",
                &format!(
                    "provider={} shadowed={}",
                    body.provider, removed.shadowed_by_env
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
            Json(serde_json::json!({ "provider": name, "models": models })).into_response()
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "provider": name, "error": e.to_string() })),
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
}

/// Persists a credential to the user-level secret store and makes it
/// effective immediately. The key is accepted once and never echoed back.
async fn put_provider_key(
    State(state): State<AppState>,
    Json(body): Json<ProviderKeyBody>,
) -> axum::response::Response {
    match state.core.set_provider_key(&body.provider, &body.key) {
        Ok(env_var) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "provider_key_set",
                &format!("provider={}", body.provider),
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

#[derive(serde::Deserialize)]
struct TelegramTokenBody {
    token: String,
}

/// Persists the Telegram bot token to the same user-level `.env` the
/// provider keys use, then kicks the bridge service so the new token takes
/// effect right away — it self-sources `.env` at launch, it doesn't
/// inherit this process's environment or the runtime override. The token
/// is accepted once and never echoed back.
async fn put_telegram_token(
    State(state): State<AppState>,
    Json(body): Json<TelegramTokenBody>,
) -> axum::response::Response {
    match state.core.set_telegram_token(&body.token) {
        Ok(env_var) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "telegram_token_set",
                "telegram",
                None,
            );
            state
                .hub
                .emit_config_changed("telegram_token_set", "telegram");
            // Best-effort: kickstart -k (macOS) / systemctl restart (linux)
            // reads the fresh .env on the way back up. If the bridge isn't
            // installed as a service yet, this is a harmless no-op — the
            // caller still gets `restarted: false` to reflect that.
            let mut cfg = vak_ops::OpsConfig::detect();
            cfg.port = state.ops_port;
            let restarted = vak_ops::restart(vak_ops::Service::Telegram, &cfg);
            Json(serde_json::json!({
                "env_var": env_var,
                "configured": true,
                "restarted": restarted,
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

/// Revoke the stored Telegram bot token. Reports when the variable is
/// still set in the real environment, since that keeps the bridge
/// authenticated and no app-level action can change it.
async fn delete_telegram_token(State(state): State<AppState>) -> axum::response::Response {
    match state.core.remove_telegram_token() {
        Ok(removed) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "telegram_token_removed",
                &format!("shadowed={}", removed.shadowed_by_env),
                None,
            );
            state
                .hub
                .emit_config_changed("telegram_token_removed", "telegram");
            // Same best-effort kick as on save, so a removed token doesn't
            // keep serving off a stale in-memory credential.
            let mut cfg = vak_ops::OpsConfig::detect();
            cfg.port = state.ops_port;
            let restarted = vak_ops::restart(vak_ops::Service::Telegram, &cfg);
            Json(serde_json::json!({
                "env_var": removed.env_var,
                "configured": removed.shadowed_by_env,
                "shadowed_by_env": removed.shadowed_by_env,
                "restarted": restarted,
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

/// Per-surface bot token, stored exactly like the Telegram one: user-level
/// `.env`, owner-only, accepted once and never echoed back
/// (docs/design/34 Phase 3 "a bot-token field per surface").
///
/// Only Telegram has a managed service unit today, so `restarted` is true
/// only for that surface; Discord/Slack bridges are started by hand
/// (`vak discord --server ...`) and the caller says so.
async fn put_bot_token(
    State(state): State<AppState>,
    axum::extract::Path(surface): axum::extract::Path<String>,
    Json(body): Json<TelegramTokenBody>,
) -> axum::response::Response {
    let Some(env) = vak_core::Core::bot_token_env(&surface) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("unknown chat surface '{surface}'") })),
        )
            .into_response();
    };
    match state.core.set_bot_token(env, &body.token) {
        Ok(env_var) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "bot_token_set",
                &surface,
                None,
            );
            state.hub.emit_config_changed("bot_token_set", &surface);
            let mut cfg = vak_ops::OpsConfig::detect();
            cfg.port = state.ops_port;
            let restarted =
                surface == "telegram" && vak_ops::restart(vak_ops::Service::Telegram, &cfg);
            Json(serde_json::json!({
                "surface": surface,
                "env_var": env_var,
                "configured": true,
                "restarted": restarted,
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

async fn delete_bot_token(
    State(state): State<AppState>,
    axum::extract::Path(surface): axum::extract::Path<String>,
) -> axum::response::Response {
    let Some(env) = vak_core::Core::bot_token_env(&surface) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("unknown chat surface '{surface}'") })),
        )
            .into_response();
    };
    match state.core.remove_bot_token(env) {
        Ok(removed) => {
            vak_core::security_events::record(
                &state.core.sessions_home(),
                vak_core::security_events::EventKind::ProviderKeyChange,
                "bot_token_removed",
                &format!("surface={surface} shadowed={}", removed.shadowed_by_env),
                None,
            );
            state.hub.emit_config_changed("bot_token_removed", &surface);
            let mut cfg = vak_ops::OpsConfig::detect();
            cfg.port = state.ops_port;
            let restarted =
                surface == "telegram" && vak_ops::restart(vak_ops::Service::Telegram, &cfg);
            Json(serde_json::json!({
                "surface": surface,
                "env_var": removed.env_var,
                "configured": removed.shadowed_by_env,
                "shadowed_by_env": removed.shadowed_by_env,
                "restarted": restarted,
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

// ---- Multi-bot (docs/design/34, multi-bot-per-channel) --------------------

/// Reconcile per-bot bridge units after any `bots.json` change (create,
/// delete, or rename) so the admin console's Bots list never gets ahead of
/// what's actually running. Mirrors the legacy single-bot
/// `vak_ops::restart(Service::Telegram, ...)` calls above, generalized to a
/// dynamic per-bot unit — see docs/design/34 and `vak-ops::services`.
///
/// `std::env::current_exe()` is deliberately used instead of resolving an
/// install manifest: this handler runs inside the gateway process itself
/// (`vak serve --gateway --trust`), so the currently-executing binary path
/// *is* the correct one to launch bridge processes from.
fn sync_bot_units(core: &vak_core::Core, port: u16) {
    let Ok(bin_path) = std::env::current_exe() else {
        return;
    };
    let gateway_url = vak_ops::OpsConfig { port }.base_url();
    let _ = vak_ops::sync_bots(
        &bin_path,
        &core.sessions_home(),
        &gateway_url,
        &vak_ops::Paths::default(),
        &vak_ops::SystemRunner,
    );
}

fn bot_env_var(id: &str) -> String {
    // A dedicated env var per bot id, distinct from the legacy per-surface
    // slots (`TELEGRAM_BOT_TOKEN` etc.) so a second bot never overwrites
    // the first one's token in the shared `.env` file.
    format!("BOT_TOKEN__{}", id.to_uppercase().replace('-', "_"))
}

async fn list_bots(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "bots": state.gateway.bots_snapshot() }))
}

#[derive(serde::Deserialize)]
struct CreateBotBody {
    id: String,
    surface: String,
    label: String,
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
    if vak_core::Core::bot_token_env(&body.surface).is_none() {
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
        ..Default::default()
    };
    state.gateway.bot_upsert(&state.core, bot.clone());
    sync_bot_units(&state.core, state.ops_port);
    state.hub.emit_config_changed("bot_created", id);
    Json(serde_json::json!({ "bot": bot })).into_response()
}

#[derive(serde::Deserialize, Default)]
struct UpdateBotBody {
    #[serde(default)]
    label: Option<String>,
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
    state.gateway.bot_upsert(&state.core, bot.clone());
    state.hub.emit_config_changed("bot_updated", &id);
    Json(serde_json::json!({ "bot": bot })).into_response()
}

async fn delete_bot(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> StatusCode {
    if state.gateway.bot_remove(&state.core, &id) {
        sync_bot_units(&state.core, state.ops_port);
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
            // A rotate needs no new unit, only a bounce so the process
            // re-reads its env var (see `restart_bot_unit`'s doc comment).
            // A first-time token set has no unit yet — `sync_bot_units`
            // creates and starts it, and `restart_bot_unit` then no-ops.
            sync_bot_units(&state.core, state.ops_port);
            let restarted = vak_ops::restart_bot_unit(&bot.surface, &id, &vak_ops::SystemRunner);
            Json(serde_json::json!({
                "id": id,
                "env_var": env_var,
                "configured": true,
                "restarted": restarted,
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
            // Bounce the running process so it stops using the now-cleared
            // token immediately, instead of continuing on the one it read
            // at its last start.
            let restarted = vak_ops::restart_bot_unit(&bot.surface, &id, &vak_ops::SystemRunner);
            Json(serde_json::json!({
                "id": id,
                "env_var": removed.env_var,
                "configured": removed.shadowed_by_env,
                "shadowed_by_env": removed.shadowed_by_env,
                "restarted": restarted,
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

/// Every chat bridge's credential state in one shape, so Settings renders
/// all three surfaces from one list instead of three copies of the
/// Telegram block (docs/design/34 Phase 3).
fn chat_surface_status() -> Vec<serde_json::Value> {
    ["telegram", "discord", "slack"]
        .iter()
        .map(|surface| {
            serde_json::json!({
                "surface": surface,
                "env_var": vak_core::Core::bot_token_env(surface),
                "configured": vak_core::Core::bot_token_configured(surface),
                // Only Telegram has a managed service unit today; the
                // others are started by hand and the UI must not promise
                // otherwise.
                "managed_service": *surface == "telegram",
            })
        })
        .collect()
}

async fn get_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    refresh_control_plane(&state);
    let cfg = state.core.config();
    let work = state.core.effective_work();
    let route = state.core.effective_route();
    let project_path = vak_config::project_path(state.core.cwd());
    Json(serde_json::json!({
        "provider": route.provider,
        "model": route.model,
        "provider_source": route.provider_source,
        "model_source": route.model_source,
        "route_revision": route.revision,
        "max_tokens": cfg.max_tokens,
        "max_turns": state.core.effective_max_turns(),
        "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
        "subagents": state.core.effective_subagents(),
        "max_retries": cfg.max_retries,
        "retry_base_backoff_ms": cfg.retry_base_backoff_ms,
        "request_timeout_secs": cfg.request_timeout_secs,
        "run_retry_attempts": cfg.run_retry_attempts,
        "run_retry_base_backoff_ms": cfg.run_retry_base_backoff_ms,
        "circuit_breaker_threshold": cfg.circuit_breaker_threshold,
        "circuit_breaker_cooldown_secs": cfg.circuit_breaker_cooldown_secs,
        "context_window": cfg.context_window,
        "theme": state.core.effective_theme(),
        "memory": {
            "search_enabled": state.core.effective_memory_search_enabled(),
            "write_enabled": state.core.effective_memory_write_enabled(),
            "reflection": state.core.effective_memory_reflection(),
            "skill_proposals": state.core.effective_memory_skill_proposals(),
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
            "skills": state.core.skills().iter().map(|skill| skill.name.clone()).collect::<Vec<_>>(),
        },
        "telegram": {
            "env_var": Core::TELEGRAM_TOKEN_ENV,
            "configured": state.core.telegram_configured(),
        },
        // docs/design/34 Phase 3: same shape per surface, so Settings can
        // render all three chat bridges from one list.
        "chat_surfaces": chat_surface_status(),
        "paths": {
            "project_config": project_path,
            "global_config": vak_config::global_path(),
            "sessions_home": state.core.sessions_home(),
            "cwd": state.core.cwd(),
        },
        "warnings": cfg.warnings,
    }))
}

#[derive(serde::Deserialize, Default)]
struct ConfigPatch {
    provider: Option<String>,
    model: Option<String>,
    max_turns: Option<usize>,
    permission_mode: Option<String>,
    theme: Option<String>,
    /// Whether sub-agent delegation (the `task` tool) is available. Absent
    /// means "leave alone", same convention every field here uses.
    #[serde(default)]
    subagents: Option<bool>,
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
}

async fn patch_config(State(state): State<AppState>, Json(body): Json<ConfigPatch>) -> StatusCode {
    patch_config_scope(state, body, false).await
}

async fn patch_global_config(
    State(state): State<AppState>,
    Json(body): Json<ConfigPatch>,
) -> StatusCode {
    patch_config_scope(state, body, true).await
}

async fn patch_config_scope(state: AppState, body: ConfigPatch, global: bool) -> StatusCode {
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
        return StatusCode::BAD_REQUEST;
    }
    let permission_mode = body.permission_mode.as_deref().and_then(parse_mode);
    let current_route = state.core.effective_route();
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
        || body.theme.is_some())
        && (if global {
            vak_config::persist_global_preferences(
                provider.as_deref(),
                model.as_deref(),
                body.max_turns,
                permission_mode,
                body.theme.as_deref(),
            )
        } else {
            vak_config::persist_project_preferences(
                state.core.cwd(),
                provider.as_deref(),
                model.as_deref(),
                body.max_turns,
                permission_mode,
                body.theme.as_deref(),
            )
        })
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    let mut changes = Vec::new();
    if let (Some(provider), Some(model)) = (provider, model) {
        let effective =
            vak_config::load_with_trust(state.core.cwd(), state.core.project_config_trusted());
        let Ok(effective) = effective else {
            return StatusCode::INTERNAL_SERVER_ERROR;
        };
        state
            .core
            .apply_persisted_route(effective.provider, effective.model);
        changes.push(format!("route={provider}/{model}"));
    }
    if let Some(max_turns) = body.max_turns {
        if !(1..=1000).contains(&max_turns) {
            return StatusCode::BAD_REQUEST;
        }
        state.core.apply_persisted_max_turns(max_turns);
        changes.push(format!("max_turns={max_turns}"));
    }
    if let Some(mode) = body.permission_mode {
        let Some(mode) = parse_mode(&mode) else {
            return StatusCode::BAD_REQUEST;
        };
        apply_permission_mode(&state, mode, true);
        changes.push(format!("permission_mode={mode:?}"));
    }
    if let Some(theme) = body.theme {
        if !matches!(theme.as_str(), "dark" | "light" | "plain") {
            return StatusCode::BAD_REQUEST;
        }
        changes.push(format!("theme={theme}"));
        state.core.apply_persisted_theme(theme);
    }
    if let Some(subagents) = body.subagents {
        let persisted = if global {
            vak_config::persist_global_subagents(subagents)
        } else {
            vak_config::persist_project_subagents(state.core.cwd(), subagents)
        };
        if persisted.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
        state.core.apply_persisted_subagents(subagents);
        changes.push(format!("subagents={subagents}"));
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
                state.core.cwd(),
                body.memory_search_enabled,
                body.memory_write_enabled,
                body.memory_reflection,
                body.memory_skill_proposals,
            )
        };
        if persisted.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
        // `apply_persisted_memory` sets all four flags at once, so fields
        // this PATCH didn't mention keep their current effective value
        // rather than reverting to whatever was on disk before.
        state.core.apply_persisted_memory(
            body.memory_search_enabled
                .unwrap_or_else(|| state.core.effective_memory_search_enabled()),
            body.memory_write_enabled
                .unwrap_or_else(|| state.core.effective_memory_write_enabled()),
            body.memory_reflection
                .unwrap_or_else(|| state.core.effective_memory_reflection()),
            body.memory_skill_proposals
                .unwrap_or_else(|| state.core.effective_memory_skill_proposals()),
        );
        changes.push(format!(
            "memory(search={}, write={}, reflection={}, skill_proposals={})",
            state.core.effective_memory_search_enabled(),
            state.core.effective_memory_write_enabled(),
            state.core.effective_memory_reflection(),
            state.core.effective_memory_skill_proposals(),
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
            Ok(vak_config::project_path(state.core.cwd()))
        };
        let Ok(path) = path else {
            return StatusCode::INTERNAL_SERVER_ERROR;
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
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
        let resolved =
            vak_config::load_with_trust(state.core.cwd(), state.core.project_config_trusted());
        let Ok(resolved) = resolved else {
            return StatusCode::INTERNAL_SERVER_ERROR;
        };
        state.core.apply_persisted_work(resolved.work);
        changes.push(format!(
            "work(mode={}, enabled={})",
            state.core.effective_work().default_mode,
            state.core.effective_work().enabled
        ));
    }
    if !changes.is_empty() {
        vak_core::security_events::record(
            &state.core.sessions_home(),
            vak_core::security_events::EventKind::ConfigChange,
            "config_patched",
            &changes.join(", "),
            None,
        );
        state
            .hub
            .emit_config_changed("config_patched", &changes.join(", "));
    }
    StatusCode::OK
}

// ---- MCP server management --------------------------------------------------
//
// The desktop Settings page edits the MCP table here: GET reads the
// effective table; PUT validates, persists to the project config.toml
// ([mcp.servers]) and hot-applies into the running Core so the next turn
// picks it up without a backend restart. mcp.servers is a privileged key:
// this endpoint is only reachable through the bearer-token router of a
// locally trusted surface.

/// Project-only, deliberately not `state.core.effective_mcp()` (the merged
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
async fn get_mcp_servers(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = vak_config::project_path(state.core.cwd());
    match read_mcp_config(&path) {
        Ok(mcp) => Json(serde_json::json!({ "servers": mcp.servers })).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn get_tavily(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mcp = state.core.effective_mcp();
    let configured = vak_config::get_var("TAVILY_API_KEY").is_some_and(|v| !v.trim().is_empty());
    let enabled = mcp.servers.get("tavily").is_some_and(|server| {
        server.command == "npx"
            && server.args == ["-y", "tavily-mcp"]
            && server.network
            && server.env.get("TAVILY_API_KEY") == Some(&"${TAVILY_API_KEY}".to_string())
    });
    Json(serde_json::json!({
        "enabled": enabled,
        "key_present": configured,
        "network": enabled,
        "env_var": "TAVILY_API_KEY"
    }))
}

#[derive(serde::Deserialize)]
struct TavilyPutBody {
    key: String,
}

async fn put_tavily(
    State(state): State<AppState>,
    Json(body): Json<TavilyPutBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Err(e) = state.core.set_mcp_secret("TAVILY_API_KEY", &body.key) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    let mut servers = state
        .core
        .effective_mcp()
        .servers
        .into_iter()
        .map(|(name, server)| {
            (
                name,
                McpServerInput {
                    command: server.command,
                    args: server.args,
                    env: server.env,
                    network: server.network,
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    servers.insert(
        "tavily".into(),
        McpServerInput {
            command: "npx".into(),
            args: vec!["-y".into(), "tavily-mcp".into()],
            env: [("TAVILY_API_KEY".into(), "${TAVILY_API_KEY}".into())]
                .into_iter()
                .collect(),
            network: true,
        },
    );
    if let Err(e) = persist_mcp_to_project_config(state.core.cwd(), &servers) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response();
    }
    state
        .core
        .apply_persisted_mcp_servers(vak_config::McpConfig {
            servers: servers
                .into_iter()
                .map(|(name, server)| {
                    (
                        name,
                        vak_config::McpServerConfig {
                            command: server.command,
                            args: server.args,
                            env: server.env,
                            network: server.network,
                        },
                    )
                })
                .collect(),
        });
    state.hub.emit_config_changed("tavily_updated", "tavily");
    Json(serde_json::json!({ "enabled": true, "key_present": true })).into_response()
}

async fn disable_tavily(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let mut servers = state
        .core
        .effective_mcp()
        .servers
        .into_iter()
        .map(|(name, server)| {
            (
                name,
                McpServerInput {
                    command: server.command,
                    args: server.args,
                    env: server.env,
                    network: server.network,
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    servers.remove("tavily");
    if let Err(e) = persist_mcp_to_project_config(state.core.cwd(), &servers) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e })),
        )
            .into_response();
    }
    state
        .core
        .apply_persisted_mcp_servers(vak_config::McpConfig {
            servers: servers
                .into_iter()
                .map(|(name, server)| {
                    (
                        name,
                        vak_config::McpServerConfig {
                            command: server.command,
                            args: server.args,
                            env: server.env,
                            network: server.network,
                        },
                    )
                })
                .collect(),
        });
    state.hub.emit_config_changed("tavily_disabled", "tavily");
    Json(serde_json::json!({ "enabled": false })).into_response()
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
async fn get_hooks(State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = state.core.cwd().join(".vak/config.toml");
    let hooks = if path.is_file() {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str::<vak_config::FileConfig>(&raw).ok())
        {
            Some(config) => config.hooks,
            None => return (StatusCode::BAD_REQUEST, "project config is invalid").into_response(),
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
    Json(serde_json::json!({ "scope": "user", "hooks": hooks.into_iter().map(|h| serde_json::json!({ "event": h.event, "matcher": h.matcher, "command": h.command, "timeout_ms": h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS), "enabled": h.enabled, "failure_mode": h.failure_mode.as_deref().unwrap_or("open") })).collect::<Vec<_>>() })).into_response()
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
    Json(serde_json::json!({ "saved": true, "scope": "user", "count": hooks.len() }))
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
    let path = state.core.cwd().join(".vak/config.toml");
    let mut root: toml::Value = if path.exists() {
        match std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str(&raw).ok())
        {
            Some(v) => v,
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": "project config is invalid" })),
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
    state.core.apply_persisted_hooks(
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
        &state.core.sessions_home(),
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

async fn get_global_mcp_servers() -> axum::response::Response {
    use axum::response::IntoResponse;
    let Some(path) = vak_config::global_path() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "user home unavailable").into_response();
    };
    match read_mcp_config(&path) {
        Ok(mcp) => {
            Json(serde_json::json!({ "scope": "user", "path": path, "servers": mcp.servers }))
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
    Json(serde_json::json!({ "saved": true, "scope": "user", "count": body.servers.len() }))
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
    match persist_mcp_to_project_config(state.core.cwd(), &body.servers) {
        Ok(_) => {}
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    }
    let cfg = vak_config::load_with_trust(state.core.cwd(), state.core.project_config_trusted())
        .map(|config| config.mcp);
    let Ok(cfg) = cfg else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    state.core.apply_persisted_mcp_servers(cfg);
    vak_core::security_events::record(
        &state.core.sessions_home(),
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
    });
    let events = mpsc_to_broadcast(side_tx.clone());
    let cancel = handle
        .side_cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let core = state.core.clone();

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
                crate::presentation::snapshot(&id, &restored);
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
            let _ = h.side_events_tx.send(AgentEvent::StreamOpened);
            Box::pin(BroadcastStream::new(rx).filter_map(|ev| match ev {
                Ok(agent_event) => Some(Ok(
                    Event::default().data(serde_json::to_string(&agent_event).unwrap_or_default()),
                )),
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
        match spawn_isolated_run(&state, provider.clone(), rid, wt, &body.prompt, None).await {
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
async fn spawn_isolated_run(
    state: &AppState,
    provider: Arc<dyn Provider>,
    rid: &str,
    wt: &vak_core::worktree::Worktree,
    prompt: &str,
    model_pin: Option<&str>,
) -> Result<String, String> {
    let child_core = vak_core::Core::new_with_trust(wt.path.clone(), true)
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
    let handle = register_handle(state, child_id.clone(), child_log, wt.path.clone());
    begin_turn(&handle, &child_core, prompt);
    Ok(child_id)
}

/// Fire a single-turn agent run on a (usually fresh) session handle.
fn begin_turn(handle: &Arc<SessionHandle>, core: &Core, prompt: &str) {
    let approver: Arc<dyn Approver> = Arc::new(HttpApprover {
        events_tx: handle.events_tx.clone(),
        pending: handle.pending.clone(),
        session_id: handle.id.clone(),
        activity_buffer: handle.activity_buffer.clone(),
    });
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
                    crate::presentation::snapshot(&turn_session_id, &restored);
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
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn tasks_file(core: &Core) -> PathBuf {
    vak_core::tasks::tasks_file(&core.sessions_home())
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
    match vak_core::tasks::TaskStore::load(&state.core.sessions_home()) {
        Ok(store) => {
            *map = store.all().into_iter().map(|t| (t.id.clone(), t)).collect();
        }
        // A corrupt tasks file is surfaced loudly, never silently dropped:
        // those definitions represent real automation the user expects.
        Err(e) => eprintln!("[scheduler] tasks file unreadable, ignoring: {e}"),
    }
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
    let mut mine: Vec<TaskDef> = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .filter(|t| t.cwd == cwd)
        .cloned()
        .collect();
    mine.sort_by_key(|t| t.created_at);
    Json(serde_json::json!({ "tasks": mine }))
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
    /// Watchdog shell one-liner; XOR with `prompt`, never touches the LLM.
    #[serde(default)]
    script: Option<String>,
    /// Pin dispatches to one model id (`provider/model` or bare model id).
    #[serde(default)]
    model_pin: Option<String>,
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
        last_wt: None,
        deliver_to: body.deliver_to,
        schedule: body.schedule.filter(|s| !s.trim().is_empty()),
        script: body.script.filter(|s| !s.trim().is_empty()),
        model_pin: body.model_pin.filter(|m| !m.trim().is_empty()),
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
    script: OptionalStr,
    #[serde(default)]
    model_pin: OptionalStr,
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
    let child_id = spawn_isolated_run(
        state,
        provider.clone(),
        &rid,
        &wt,
        &snapshot.prompt,
        snapshot.model_pin.as_deref(),
    )
    .await
    .ok()?;

    update_tasks(state, |map| {
        if let Some(t) = map.get_mut(id) {
            t.last_run_at = Some(chrono::Utc::now());
            t.last_session_id = Some(child_id.clone());
            t.last_summary = None;
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
                if let AgentEvent::RunFinished { summary, .. } = ev {
                    let text =
                        last_assistant_text(&child_handle).unwrap_or_else(|| summary.clone());
                    update_tasks(&st, |map| {
                        if let Some(t) = map.get_mut(&tid) {
                            t.last_summary = Some(text.clone());
                        }
                    });
                    if let Some(target) = &deliver_to {
                        // Delivery failure must not lose the recorded summary;
                        // it only means this transport could not be reached.
                        let _ = gateway::deliver_and_record(
                            &st.core,
                            target,
                            &format!("routine '{task_name}' finished:\n{text}"),
                            vak_core::inbox::Kind::TaskSummary,
                            format!("routine '{task_name}' finished"),
                            Some(&child_session),
                            Some(&tid),
                        )
                        .await;
                    }
                    check_budget_alert(&st, &tid).await;
                    break;
                }
            }
        });
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
    match content.strip_prefix("[stdout]\n") {
        Some(rest) => match rest.find("\n[stderr]") {
            Some(end) => &rest[..end],
            None => rest,
        },
        None => "",
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
    let outcome = execute_script(&state.core, &task.cwd, script).await;
    // Deliver FIRST: once the summary is visible on the task, its delivery
    // attempt has already been made. With zero transports configured the
    // inbox itself is the sink (docs/design/29 P6): a watchdog summary is
    // never lost just because no chat channel exists.
    if outcome.ok {
        if !outcome.text.is_empty() {
            let title = format!("watchdog '{}'", task.name);
            match task.deliver_to.as_deref() {
                Some(target) => {
                    let _ = gateway::deliver_and_record(
                        &state.core,
                        target,
                        &outcome.text,
                        vak_core::inbox::Kind::TaskSummary,
                        title,
                        None,
                        Some(&task.id),
                    )
                    .await;
                }
                None => {
                    let _ = vak_core::inbox::record(
                        &state.core.sessions_home(),
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
        let _ = gateway::deliver_and_record(
            &state.core,
            target,
            &format!("watchdog '{}' alert:\n{}", task.name, outcome.text),
            vak_core::inbox::Kind::TaskSummary,
            format!("failure: watchdog '{}'", task.name),
            None,
            Some(&task.id),
        )
        .await;
    }
    update_tasks(state, |map| {
        if let Some(t) = map.get_mut(&task.id) {
            t.last_run_at = Some(chrono::Utc::now());
            t.last_summary = Some(if outcome.ok && outcome.text.is_empty() {
                "(silent tick)".to_string()
            } else {
                outcome.text.clone()
            });
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
            .filter(|t| match t.schedule.as_deref() {
                Some(expr) => {
                    let marker = markers.entry(t.id.clone()).or_insert_with(|| {
                        vak_core::tasks::cron_next_after(expr, now_local)
                            .unwrap_or_else(|_| park_marker())
                    });
                    now_local >= *marker
                }
                None => t
                    .last_run_at
                    .map(|l| {
                        (now_local.with_timezone(&Utc) - l).num_seconds() >= t.interval_secs as i64
                    })
                    .unwrap_or(true),
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
}

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
    let day_total =
        vak_core::finops::FinOpsLedger::new(&state.core.sessions_home()).day_total_usd(Utc::now());
    let Some(level) = vak_core::finops::alert_level(day_total, cap) else {
        return;
    };
    let home = state.core.sessions_home();
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

/// Sensible fallback when no launch.toml exists: a package.json dev script.
fn detect_launch(cwd: &std::path::Path) -> Vec<LaunchConfig> {
    let pkg = cwd.join("package.json");
    let Ok(raw) = std::fs::read_to_string(pkg) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    if v["scripts"]["dev"].is_string() {
        vec![LaunchConfig {
            name: "dev".into(),
            cmd: "npm".into(),
            args: vec!["run".into(), "dev".into()],
            port: None,
        }]
    } else {
        Vec::new()
    }
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
    use super::{cron_slot_missed, stdout_section};
    use chrono::TimeZone;
    use chrono::Utc;

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
        assert_eq!(stdout_section("(no output)"), "");
        assert_eq!(stdout_section(""), "");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod configuration_control_tests {
    use super::*;

    #[tokio::test]
    async fn cross_process_mode_refresh_revokes_live_capability_before_apply() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let session = core.start_session().await.unwrap();
        let id = session.header().unwrap().session_id.clone();
        let handle = register_handle(&state, id, session, core.cwd().clone());
        assert!(!handle.cancel.lock().unwrap().is_cancelled());

        vak_config::persist_project_preferences(
            dir.path(),
            None,
            None,
            Some(19),
            Some(vak_config::PermissionMode::ReadOnly),
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

        let status = patch_config(
            State(state.clone()),
            Json(ConfigPatch {
                memory_write_enabled: Some(false),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

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

    /// Same live-without-restart guarantee as memory, for the `subagents`
    /// toggle newly surfaced in the admin console's Settings page — it was
    /// previously read from `Core::config()` directly at both call sites,
    /// so a PATCH would have silently done nothing.
    #[tokio::test]
    async fn patch_config_subagents_applies_live_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        assert!(core.effective_subagents(), "default is on");

        let status = patch_config(
            State(state.clone()),
            Json(ConfigPatch {
                subagents: Some(false),
                ..Default::default()
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            !core.effective_subagents(),
            "must apply live without a restart"
        );

        let fresh = Core::new(dir.path().to_path_buf()).unwrap();
        fresh.set_sessions_home(dir.path().join("home"));
        assert!(
            !fresh.effective_subagents(),
            "must be persisted to disk too"
        );
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
            }),
        )
        .await;
        assert_eq!(status.status(), StatusCode::BAD_REQUEST);
    }
}
