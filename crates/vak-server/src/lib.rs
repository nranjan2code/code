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

mod channels;
mod gateway;
mod heartbeat;
pub mod telegram;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;

use axum::extract::{Path, State};
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
use vak_session::SessionLog;

pub(crate) struct SessionHandle {
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
    sessions: Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>,
    /// Live best-of-N runs keyed by child session id.
    best_runs: Arc<Mutex<HashMap<String, BestRunMeta>>>,
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
        AppState {
            core,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            best_runs: Arc::new(Mutex::new(HashMap::new())),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            next_fire: Arc::new(Mutex::new(HashMap::new())),
            script_inflight: Arc::new(Mutex::new(std::collections::HashSet::new())),
            procs: Arc::new(Mutex::new(HashMap::new())),
            gateway,
            heartbeat: Arc::new(heartbeat::HeartbeatRuntime::new()),
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
}

#[derive(Clone)]
pub struct ApprovalRequest {
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
                    respond: Arc::new(Mutex::new(Some(respond))),
                },
            );
        let _ = self.events_tx.send(AgentEvent::ApprovalRequested {
            id: id.clone(),
            tool: tool.to_string(),
            args_json: args_json.to_string(),
            reason: reason.to_string(),
        });
        let approved = rx.await.unwrap_or(false);
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
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
        .route("/config/mode", post(set_permission_mode))
        .route("/config/mcp", get(get_mcp_servers).put(put_mcp_servers))
        .route("/config/hooks", get(get_hooks).put(put_hooks))
        .route(
            "/config/key",
            put(put_provider_key).delete(delete_provider_key),
        )
        .route("/providers", get(list_providers))
        .route("/providers/{name}/models", get(discover_models))
        .route("/search", get(search_sessions))
        .route("/ops/status", get(ops_status))
        .route("/ops/{service}/{action}", post(ops_action))
        .route("/ops/diagnostics", get(ops_diagnostics))
        .route("/finops", get(finops_status))
        .route("/memory", get(list_memory).post(append_memory))
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

async fn ops_status(State(_state): State<AppState>) -> Json<serde_json::Value> {
    let cfg = vak_ops::OpsConfig::detect();
    Json(ops_payload(&cfg))
}

/// Read-only operational projection for desktop/TUI surfaces. This keeps
/// service, gateway, flow and health state in one refreshable payload without
/// exposing credentials or implementation paths.
async fn ops_diagnostics(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg = vak_ops::OpsConfig::detect();
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
    Json(serde_json::json!({
        "health": {
            "status": "ok",
            "provider": state.core.effective_provider(),
            "model": state.core.effective_model(),
            "sandbox": state.core.effective_sandbox_name(),
            "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
            "warnings": state.core.config().warnings,
        },
        "services": ops_payload(&cfg),
        "gateway": {
            "enabled": state.gateway.enabled,
            "bindings": gateway.into_iter().map(|(target, session_id)| serde_json::json!({ "target": target, "session_id": session_id })).collect::<Vec<_>>(),
            "approvals": {
                "mode": state.gateway.approvals_mode(),
                "approver": state.gateway.approver_target(),
                "pending": state.gateway.pending_approval_count(),
            },
        },
        "flows": flows,
    }))
}

/// FinOps projection from the append-only cost ledger. Unknown-priced rows are
/// retained as `unknown_rows`; they are never reported as zero spend.
async fn finops_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg = &state.core.config().finops;
    let path = state.core.sessions_home().join("cost-log.jsonl");
    let mut rows = Vec::new();
    if let Ok(body) = std::fs::read_to_string(path) {
        for line in body.lines() {
            if let Ok(row) = serde_json::from_str::<vak_core::finops::CostRow>(line) {
                rows.push(row);
            }
        }
    }
    let now = chrono::Utc::now();
    let day_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Utc).single());
    let day_rows = rows
        .iter()
        .filter(|r| day_start.is_some_and(|start| r.ts >= start));
    let day_usd: f64 = day_rows.clone().filter_map(|r| r.usd).sum();
    let unknown_rows = day_rows.filter(|r| r.usd.is_none()).count();
    let mut by_provider = std::collections::BTreeMap::<String, (f64, u64)>::new();
    let mut by_model = std::collections::BTreeMap::<String, (f64, u64)>::new();
    for row in rows
        .iter()
        .filter(|r| day_start.is_some_and(|start| r.ts >= start))
    {
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
    Json(serde_json::json!({
        "day_usd": day_usd,
        "run_cap_usd": cfg.max_run_usd,
        "day_cap_usd": cfg.max_day_usd,
        "unknown_rows": unknown_rows,
        "total_rows": rows.len(),
        "by_provider": rollup(by_provider),
        "by_model": rollup(by_model),
    }))
}

#[derive(serde::Deserialize)]
struct OpsActionQuery {
    #[serde(default)]
    port: Option<u16>,
}

async fn ops_action(
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
    if let Some(port) = q.port {
        cfg.port = port;
    }
    let result = match action.as_str() {
        "start" => {
            vak_ops::start(svc, &cfg);
            serde_json::json!({ "ok": true, "action": "start" })
        }
        "stop" => {
            vak_ops::stop(svc, &cfg);
            serde_json::json!({ "ok": true, "action": "stop" })
        }
        "restart" => {
            vak_ops::restart(svc, &cfg);
            serde_json::json!({ "ok": true, "action": "restart" })
        }
        "install" => match vak_ops::install(svc, &cfg) {
            Ok(()) => serde_json::json!({ "ok": true, "action": "install" }),
            Err(e) => serde_json::json!({ "ok": false, "error": e }),
        },
        "uninstall" => match vak_ops::uninstall(svc, &cfg) {
            Ok(()) => serde_json::json!({ "ok": true, "action": "uninstall" }),
            Err(e) => serde_json::json!({ "ok": false, "error": e }),
        },
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("unknown action '{other}'") })),
            )
                .into_response();
        }
    };
    (StatusCode::OK, Json(result)).into_response()
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
        Ok(bytes) => (
            StatusCode::OK,
            Json(serde_json::json!({ "forgotten": note_id, "bytes": bytes })),
        )
            .into_response(),
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
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "amended": note_id })),
        )
            .into_response(),
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
    match tokio::task::spawn_blocking(move || {
        // Both hit shapes are Serialize; the workspace path keeps its flat
        // SessionHit wire shape, cross-project adds the project_hash wrapper.
        let searched = if all {
            vak_session::search_all(&home, &query, limit, exclude.as_deref())
                .map(|hits| serde_json::to_value(&hits).map_err(|e| e.to_string()))
        } else {
            vak_session::search(&home, &cwd, &query, limit, exclude.as_deref())
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
/// Token selection: when `VAKCODER_GATEWAY_TOKEN` is set in the
/// environment, it is used verbatim so service-managed bridges and other
/// long-lived clients can survive process restarts. Otherwise a fresh
/// per-process token is minted as before. The variable is never logged.
pub fn secured_router_with(core: Core, force_gateway: bool) -> (Router, String) {
    let token = std::env::var("VAKCODER_GATEWAY_TOKEN")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| format!("vk_{}", uuid::Uuid::now_v7()));
    // Webview origins: tauri://localhost (macOS/Linux), https://tauri.localhost
    // (Windows), plus vite dev servers.
    let origins = [
        "tauri://localhost",
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
    if force_gateway {
        state.enable_gateway();
    }
    let app = router_with_state(state.clone())
        .layer(axum::middleware::from_fn_with_state(
            token.clone(),
            require_bearer,
        ))
        .layer(cors);
    // Local routines: fires due scheduled tasks while this server lives.
    start_scheduler(&state);
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
    let (app, token) = secured_router_with(core, force_gateway);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("VakCoder server listening on http://{addr}");
    if std::env::var("VAKCODER_GATEWAY_TOKEN").is_ok_and(|t| !t.trim().is_empty()) {
        eprintln!("auth token: (pinned via VAKCODER_GATEWAY_TOKEN)");
    } else {
        eprintln!("auth token: {token}");
        eprintln!("clients must send 'Authorization: Bearer {token}' (or ?token=)");
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

async fn require_bearer(
    State(token): State<String>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if req.uri().path() == "/health" {
        return next.run(req).await;
    }
    let provided = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(String::from)
        .or_else(|| {
            req.uri()
                .query()
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
                .map(String::from)
        });
    if provided.as_deref() == Some(token.as_str()) {
        next.run(req).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "provider": state.core.effective_provider(),
        "model": state.core.effective_model(),
        "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
        "sandbox": state.core.effective_sandbox_name(),
        "context_window": state.core.config().context_window,
        "cwd": state.core.cwd(),
        "warnings": state.core.config().warnings,
    }))
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
    let handle = Arc::new(SessionHandle {
        cwd,
        session: Arc::new(Mutex::new(Some(session))),
        steering: Arc::new(SteeringQueues::new()),
        cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
        events_tx,
        pending: Arc::new(Mutex::new(HashMap::new())),
        subscribed: Arc::new(tokio::sync::Notify::new()),
        side_events_tx,
        side_cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
    });
    state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id, handle.clone());
    handle
}

async fn create_session(State(state): State<AppState>) -> Json<serde_json::Value> {
    let session = match state.core.start_session().await {
        Ok(s) => s,
        Err(e) => return Json(serde_json::json!({ "error": e.to_string() })),
    };
    let id = session
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();
    register_handle(&state, id.clone(), session, state.core.cwd().clone());

    Json(serde_json::json!({ "session_id": id }))
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

async fn run_prompt(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RunBody>,
) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    let Some(taken) = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return StatusCode::CONFLICT; // run already active
    };
    if state.core.provider().is_err() {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    // Give SSE consumers a moment to attach so terminal events are seen.
    let _ = tokio::time::timeout(Duration::from_secs(2), handle.subscribed.notified()).await;

    let approver: Arc<dyn Approver> = Arc::new(HttpApprover {
        events_tx: handle.events_tx.clone(),
        pending: handle.pending.clone(),
    });
    let events = mpsc_to_broadcast(handle.events_tx.clone());
    let steering = handle.steering.clone();
    let cancel = handle
        .cancel
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let core = state.core.clone();

    let prompt_message = if body.attachments.is_empty() {
        None
    } else {
        let mut blocks = vec![vak_llm::ContentBlock::text(body.prompt.clone())];
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
        return StatusCode::BAD_REQUEST;
    }
    // Goal mode (Phase H): captured before the spawn consumes `body`.
    let goal_pair = body.goal.clone().map(|g| (g, body.criteria.clone()));
    let run_id = id.clone();

    tokio::spawn(async move {
        let outcome = if let Some((objective, criteria)) = goal_pair {
            core.run_goal_turn_with(
                taken,
                &body.prompt,
                &objective,
                criteria,
                cancel.clone(),
                Some(approver.clone()),
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
                &body.prompt,
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
            Ok((o, session_log)) => {
                let (summary, is_error) = match &o {
                    vak_agent::TurnOutcome::Completed { .. } => ("completed".to_string(), false),
                    vak_agent::TurnOutcome::Aborted { .. } => ("aborted".to_string(), false),
                    vak_agent::TurnOutcome::Failed { error } => (format!("failed: {error}"), true),
                    vak_agent::TurnOutcome::MaxTurnsReached => ("max_turns".to_string(), true),
                };
                let _ = handle
                    .events_tx
                    .send(AgentEvent::RunFinished { summary, is_error });
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
                if let Some(restored) = reopen_ledger(&core, &run_id) {
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

    StatusCode::ACCEPTED
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
        return Json(serde_json::json!({
            "count": msgs.len(),
            "usage": s.total_usage(),
            "messages": msgs,
        }))
        .into_response();
    }
    match open_historical_session(&state, &id) {
        Some(s) => {
            let msgs = s.derive_messages();
            Json(serde_json::json!({
                "count": msgs.len(),
                "usage": s.total_usage(),
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
    serde_json::json!({
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
                "error": "backup destination must differ from the vakcoder home itself"
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
                "error": "backup source must differ from the vakcoder home itself"
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
) -> Json<serde_json::Value> {
    let Some(handle) = state.get(&id) else {
        return Json(serde_json::json!({ "error": "unknown session" }));
    };
    let cwd = handle.cwd.clone();
    let Some(status) = git_output(&cwd, &["status", "--porcelain"]).await else {
        return Json(serde_json::json!({ "error": "not a git repository" }));
    };
    let diff = git_output(&cwd, &["--no-color", "diff", "--unified=3"])
        .await
        .unwrap_or_default();
    let staged = git_output(&cwd, &["--no-color", "diff", "--cached", "--unified=3"])
        .await
        .unwrap_or_default();
    Json(serde_json::json!({
        "root": cwd,
        "diff": diff,
        "staged_diff": staged,
        "status": status,
    }))
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
    Json(serde_json::json!({ "deleted": id })).into_response()
}

async fn delete_all_archived(State(state): State<AppState>) -> axum::response::Response {
    let archive = read_archive(&state.core);
    let running_archived = archive.iter().any(|(id, archived)| {
        *archived
            && state.get(id).is_some_and(|handle| {
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
    for (id, archived) in archive {
        if archived && !deleted.get(&id).copied().unwrap_or(false) {
            deleted.insert(id, true);
            count += 1;
        }
    }
    write_deleted(&state.core, &deleted);
    Json(serde_json::json!({ "deleted": count })).into_response()
}

async fn list_skills(State(state): State<AppState>) -> Json<serde_json::Value> {
    let skills: Vec<serde_json::Value> = state
        .core
        .skills()
        .iter()
        .map(|s| serde_json::json!({ "name": s.name, "description": s.description }))
        .collect();
    Json(serde_json::json!({ "skills": skills }))
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
fn parse_mode(raw: &str) -> Option<vak_config::PermissionMode> {
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
            apply_permission_mode(&state, mode);
            StatusCode::OK
        }
        None => StatusCode::BAD_REQUEST,
    }
}

fn apply_permission_mode(state: &AppState, mode: vak_config::PermissionMode) {
    if state.core.effective_permission_mode() == mode {
        return;
    }
    state.core.set_permission_mode(mode);
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

/// Picker data for provider/model UIs. Reports WHICH env var authenticates
/// each provider and whether it resolves right now — never the value.
async fn list_providers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut providers = Vec::new();
    for name in state.core.provider_names() {
        let requires_key = name != "ollama";
        let configured = state.core.provider_configured(&name);
        providers.push(serde_json::json!({
            "name": name,
            "env_var": Core::provider_env_var(&name),
            "requires_key": requires_key,
            "configured": configured,
        }));
    }
    Json(serde_json::json!({
        "current": state.core.effective_provider(),
        "current_model": state.core.effective_model(),
        "current_configured": state.core.provider_configured(&state.core.effective_provider()),
        "providers": providers,
    }))
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
        Ok(removed) => Json(serde_json::json!({
            "provider": body.provider,
            "env_var": removed.env_var,
            "configured": removed.shadowed_by_env,
            "shadowed_by_env": removed.shadowed_by_env,
        }))
        .into_response(),
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
        Ok(env_var) => Json(serde_json::json!({
            "provider": body.provider,
            "env_var": env_var,
            "configured": true,
        }))
        .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn get_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg = state.core.config();
    let project_path = vak_config::project_path(state.core.cwd());
    Json(serde_json::json!({
        "provider": state.core.effective_provider(),
        "model": state.core.effective_model(),
        "max_tokens": cfg.max_tokens,
        "max_turns": state.core.effective_max_turns(),
        "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
        "subagents": cfg.subagents,
        "max_retries": cfg.max_retries,
        "retry_base_backoff_ms": cfg.retry_base_backoff_ms,
        "request_timeout_secs": cfg.request_timeout_secs,
        "run_retry_attempts": cfg.run_retry_attempts,
        "run_retry_base_backoff_ms": cfg.run_retry_base_backoff_ms,
        "circuit_breaker_threshold": cfg.circuit_breaker_threshold,
        "circuit_breaker_cooldown_secs": cfg.circuit_breaker_cooldown_secs,
        "context_window": cfg.context_window,
        "theme": state.core.effective_theme(),
        "bell": cfg.ui.bell,
        "stop_policy": {
            "enabled": cfg.stop_policy.enabled,
            "marker_gate": cfg.stop_policy.marker_gate,
            "verify_gate": cfg.stop_policy.verify_gate,
            "max_blocks": cfg.stop_policy.max_blocks,
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
}

async fn patch_config(State(state): State<AppState>, Json(body): Json<ConfigPatch>) -> StatusCode {
    if let Some(provider) = body.provider {
        if provider.trim().is_empty() {
            return StatusCode::BAD_REQUEST;
        }
        state.core.set_provider(provider.trim().to_string());
    }
    if let Some(model) = body.model {
        if model.trim().is_empty() {
            return StatusCode::BAD_REQUEST;
        }
        state.core.set_model(model.trim().to_string());
    }
    if let Some(max_turns) = body.max_turns {
        if !(1..=1000).contains(&max_turns) {
            return StatusCode::BAD_REQUEST;
        }
        state.core.set_max_turns(max_turns);
    }
    if let Some(mode) = body.permission_mode {
        let Some(mode) = parse_mode(&mode) else {
            return StatusCode::BAD_REQUEST;
        };
        apply_permission_mode(&state, mode);
    }
    if let Some(theme) = body.theme {
        if !matches!(theme.as_str(), "dark" | "light" | "plain") {
            return StatusCode::BAD_REQUEST;
        }
        state.core.set_theme(theme);
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

async fn get_mcp_servers(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mcp = state.core.effective_mcp();
    Json(serde_json::json!({ "servers": mcp.servers }))
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
}

fn default_hook_enabled() -> bool {
    true
}

#[derive(serde::Deserialize)]
struct HooksPutBody {
    hooks: Vec<HookInput>,
}

async fn get_hooks(State(state): State<AppState>) -> Json<serde_json::Value> {
    let hooks = state
        .core
        .config()
        .hooks
        .iter()
        .map(|h| {
            serde_json::json!({
                "event": h.event,
                "matcher": h.matcher,
                "command": h.command,
                "timeout_ms": h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS),
                "enabled": true,
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "hooks": hooks }))
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
    }
    let path = state.core.cwd().join(".vakcoder/config.toml");
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
    let values = body
        .hooks
        .iter()
        .filter(|h| h.enabled)
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
    state.core.set_hooks(
        body.hooks
            .iter()
            .filter(|h| h.enabled)
            .map(|h| vak_config::HookConfig {
                event: h.event.clone(),
                matcher: h.matcher.clone().filter(|m| !m.trim().is_empty()),
                command: h.command.trim().to_string(),
                timeout_ms: Some(h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS)),
            })
            .collect(),
    );
    (StatusCode::OK, Json(serde_json::json!({ "saved": true, "count": body.hooks.iter().filter(|h| h.enabled).count() }))).into_response()
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
    let path = cwd.join(".vakcoder/config.toml");
    let mut root: toml::Value = if path.exists() {
        let raw =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        // A config we cannot parse is never silently replaced.
        toml::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let mut servers_table = toml::map::Map::new();
    for (name, s) in servers {
        let mut entry = toml::map::Map::new();
        entry.insert("command".into(), toml::Value::String(s.command.clone()));
        entry.insert(
            "args".into(),
            toml::Value::Array(s.args.iter().cloned().map(toml::Value::String).collect()),
        );
        if !s.env.is_empty() {
            entry.insert(
                "env".into(),
                toml::Value::Table(
                    s.env
                        .iter()
                        .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
                        .collect(),
                ),
            );
        }
        if s.network {
            entry.insert("network".into(), toml::Value::Boolean(true));
        }
        servers_table.insert(name.clone(), toml::Value::Table(entry));
    }
    let mut mcp_table = toml::map::Map::new();
    mcp_table.insert("servers".into(), toml::Value::Table(servers_table));
    root.as_table_mut()
        .ok_or("config root is not a table")?
        .insert("mcp".into(), toml::Value::Table(mcp_table));
    let out = toml::to_string_pretty(&root).map_err(|e| format!("serialize config: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, out).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

async fn put_mcp_servers(
    State(state): State<AppState>,
    Json(body): Json<McpPutBody>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    for name in body.servers.keys() {
        if !valid_server_name(name) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("invalid server name '{name}'") })),
            )
                .into_response();
        }
    }
    for (name, s) in &body.servers {
        if s.command.trim().is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("server '{name}' needs a command") })),
            )
                .into_response();
        }
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
    let cfg = vak_config::McpConfig {
        servers: body
            .servers
            .iter()
            .map(|(name, s)| {
                (
                    name.clone(),
                    vak_config::McpServerConfig {
                        command: s.command.trim().to_string(),
                        args: s.args.clone(),
                        env: s.env.clone(),
                        network: s.network,
                    },
                )
            })
            .collect(),
    };
    state.core.set_mcp_servers(cfg);
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
        ".vakcoder",
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
) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    let Some(mut taken) = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    else {
        return StatusCode::CONFLICT; // main run active
    };
    if state.core.provider().is_err() {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
        return StatusCode::SERVICE_UNAVAILABLE;
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
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    let side_tx = handle.side_events_tx.clone();
    let approver: Arc<dyn Approver> = Arc::new(HttpApprover {
        events_tx: handle.events_tx.clone(),
        pending: handle.pending.clone(),
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
            // Rewind the branch pointer to the main line: the side entries
            // remain in the ledger as a sibling branch — reconstructable via
            // their parent chain, invisible to derive_messages().
            if let Some(main_tail) = &tail_main {
                let _ = restored.branch_at(main_tail);
            }
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

    StatusCode::ACCEPTED
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
    let Ok(provider) = state.core.provider() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
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
        child_core.set_provider(pin_provider);
        child_core.set_model(pin_model);
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
            Ok((_, restored)) => {
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

fn load_tasks(state: &AppState) {
    match vak_core::tasks::TaskStore::load(&state.core.sessions_home()) {
        Ok(store) => {
            let mut map = state
                .tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for t in store.all() {
                map.insert(t.id.clone(), t);
            }
        }
        // A corrupt tasks file is surfaced loudly, never silently dropped:
        // those definitions represent real automation the user expects.
        Err(e) => eprintln!("[scheduler] tasks file unreadable, ignoring: {e}"),
    }
}

fn save_tasks(state: &AppState) {
    // The in-memory map is authoritative; the persisted array is rewritten
    // wholesale in the exact wire shape TaskStore uses.
    let list: Vec<TaskDef> = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .values()
        .cloned()
        .collect();
    let _ = std::fs::create_dir_all(state.core.sessions_home());
    let target = tasks_file(&state.core);
    let tmp = target.with_extension("json.tmp");
    match serde_json::to_string_pretty(&list)
        .ok()
        .filter(|json| std::fs::write(&tmp, json).is_ok())
    {
        Some(_) => {
            let _ = std::fs::rename(&tmp, &target);
        }
        None => eprintln!("[scheduler] tasks file save failed"),
    }
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
    state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(task.id.clone(), task);
    save_tasks(&state);
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
    let updated = {
        let mut map = state
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match map.get_mut(&id) {
            Some(t) => {
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
                    return (status, Json(payload)).into_response();
                }
                *t = candidate.clone();
                candidate
            }
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": format!("no task '{id}'") })),
                )
                    .into_response();
            }
        }
    };
    // Re-enabling reschedules interval tasks from now; dropping the cron
    // marker makes the next tick recompute the schedule from scratch.
    if body.enabled == Some(true) {
        let mut map = state
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = map.get_mut(&id)
            && t.schedule.is_none()
        {
            t.last_run_at = None;
        }
    }
    state
        .next_fire
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
    save_tasks(&state);
    (StatusCode::OK, Json(serde_json::json!(updated))).into_response()
}

async fn delete_task(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let removed = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
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
        save_tasks(&state);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Fire a task immediately (also resets its schedule).
async fn run_task_now(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    let fired = fire_task(&state, &id).await.is_some();
    if fired {
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

    let mut map = state
        .tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(t) = map.get_mut(id) {
        t.last_run_at = Some(chrono::Utc::now());
        t.last_session_id = Some(child_id.clone());
        t.last_summary = None;
        t.last_wt = Some(WtMeta {
            path: wt.path,
            branch: wt.branch,
        });
    }
    drop(map);
    save_tasks(state);

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
                    if let Some(t) = st
                        .tasks
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get_mut(&tid)
                    {
                        t.last_summary = Some(text.clone());
                    }
                    save_tasks(&st);
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
pub(crate) const FALLBACK_ALERT_TARGET: &str = "log:vakcoder";

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
    {
        let mut map = state
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(t) = map.get_mut(&task.id) {
            t.last_run_at = Some(chrono::Utc::now());
            t.last_summary = Some(if outcome.ok && outcome.text.is_empty() {
                "(silent tick)".to_string()
            } else {
                outcome.text.clone()
            });
        }
    }
    save_tasks(state);
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
    let Some(cap) = state.core.config().finops.max_day_usd else {
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
    let path = cwd.join(".vakcoder/launch.toml");
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
