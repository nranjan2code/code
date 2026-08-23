//! Gateway: route always-on surfaces (chat adapters, webhooks, cron) to
//! persistent agent sessions. See docs/design/22-gateway.md.
//!
//! A surface speaks HTTP: `POST /gateway/inbound` carries `{surface, chat,
//! sender, text}` and either returns immediately or long-polls the final
//! assistant text (`wait`). Bindings persist `(surface:chat) -> session_id`
//! under `<home>/gateway/bindings.json` so conversations survive restarts.
//!
//! Unattended turns fail closed: approval gates are auto-denied and the
//! denial is fed back to the model as a tool error. Messages arriving while
//! a turn runs are queued as logged steering input and consumed between
//! model steps — nothing typed while busy is ever dropped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router};
use tokio::sync::oneshot;

use vak_agent::{AgentEvent, AutoDeny};
use vak_core::Core;

use crate::{AppState, SessionHandle};

/// Long-poll ceiling for `wait: true` inbound messages.
const WAIT_TIMEOUT: Duration = Duration::from_secs(240);

fn bindings_path(home: &std::path::Path) -> PathBuf {
    home.join("gateway").join("bindings.json")
}

fn deliveries_path(home: &std::path::Path) -> PathBuf {
    home.join("gateway").join("deliveries.jsonl")
}

pub struct GatewayState {
    pub enabled: bool,
    bindings: Mutex<HashMap<String, String>>,
    /// Resolved approval policy (docs/design/22-gateway.md G2).
    approvals: String,
    approver: Option<String>,
    approval_timeout: Duration,
    /// Forwarded gates awaiting a yes/no from the approver surface,
    /// oldest first (uuidv7 keys sort by insertion time).
    pending_approvals: Mutex<std::collections::BTreeMap<String, oneshot::Sender<bool>>>,
}

impl GatewayState {
    /// Load persisted bindings; `force` overrides the config gate
    /// (`serve --gateway`).
    pub fn load(core: &Core, force: bool) -> Self {
        let mut bindings = HashMap::new();
        if let Ok(raw) = std::fs::read_to_string(bindings_path(&core.sessions_home()))
            && let Ok(map) = serde_json::from_str::<HashMap<String, String>>(&raw)
        {
            bindings = map;
        }
        let gw = &core.config().gateway;
        let forward_ok = gw.approvals == "forward" && gw.approver.is_some();
        GatewayState {
            enabled: force || gw.enabled,
            bindings: Mutex::new(bindings),
            approvals: if forward_ok {
                "forward".into()
            } else {
                "deny".into()
            },
            approver: if forward_ok {
                gw.approver.clone()
            } else {
                None
            },
            approval_timeout: Duration::from_secs(gw.approval_timeout_secs),
            pending_approvals: Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// True when forwarded gates are active.
    pub(crate) fn forward_mode(&self) -> bool {
        self.enabled && self.approvals == "forward" && self.approver.is_some()
    }

    pub(crate) fn approver_target(&self) -> Option<&str> {
        self.approver.as_deref()
    }

    pub(crate) fn approval_timeout(&self) -> Duration {
        self.approval_timeout
    }

    pub(crate) fn approvals_mode(&self) -> &str {
        &self.approvals
    }

    pub(crate) fn pending_approval_count(&self) -> usize {
        self.pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Register a gate and hand back the reply receiver. The sender must be
    /// stored before the request is announced so an instant reply cannot
    /// race a missing entry.
    pub(crate) fn register_gate(&self, id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_string(), tx);
        rx
    }

    /// Resolve the oldest outstanding gate. Returns remaining count.
    pub(crate) fn resolve_oldest_gate(&self, approve: bool) -> Result<usize, ()> {
        let sender = self
            .pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_first();
        match sender {
            Some((_, tx)) => {
                let _ = tx.send(approve);
                Ok(self.pending_approval_count())
            }
            None => Err(()),
        }
    }

    fn snapshot(&self) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        pairs.sort();
        pairs
    }

    fn bind(&self, core: &Core, key: String, session_id: String) {
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, session_id);
        persist_bindings(core, self);
    }

    fn unbind(&self, core: &Core, key: &str) -> bool {
        let removed = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key)
            .is_some();
        if removed {
            persist_bindings(core, self);
        }
        removed
    }
}

fn persist_bindings(core: &Core, gw: &GatewayState) {
    let map = gw
        .bindings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let path = bindings_path(&core.sessions_home());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(path, json);
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/gateway/inbound", axum::routing::post(gateway_inbound))
        .route("/gateway/status", axum::routing::get(gateway_status))
        .route(
            "/gateway/bindings/{key}",
            axum::routing::delete(gateway_unbind),
        )
}

// ---- Inbound ----------------------------------------------------------------

#[derive(serde::Deserialize)]
struct InboundBody {
    surface: String,
    chat: String,
    // `sender` is accepted on the wire for adapter compatibility (group
    // routing lands with G1) but deliberately untyped here until then;
    // serde ignores unknown fields.
    text: String,
    #[serde(default)]
    wait: bool,
}

// ---- Approval forwarding (G2) ----------------------------------------------

/// Approver for gateway-driven turns. In `deny` mode this behaves like
/// `AutoDeny`. In `forward` mode the gate is announced on the approver
/// surface and resolved by a yes/no reply; timeout or silence fails closed.
struct GatewayApprover {
    events_tx: tokio::sync::broadcast::Sender<AgentEvent>,
    state: Arc<GatewayState>,
    core: Core,
}

#[async_trait::async_trait]
impl vak_agent::Approver for GatewayApprover {
    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool {
        if !self.state.forward_mode() {
            return false;
        }
        let id = uuid::Uuid::now_v7().to_string();
        let rx = self.state.register_gate(&id);
        let _ = self.events_tx.send(AgentEvent::ApprovalRequested {
            id: id.clone(),
            tool: tool.to_string(),
            args_json: args_json.to_string(),
            reason: reason.to_string(),
        });
        let short = &id[..8];
        let announce = format!(
            "Approval requested [{short}]\nTool: {tool}\nArgs: {args_json}\nReason: {reason}\nReply 'yes' or 'no' to decide."
        );
        if let Err(e) = deliver(
            &self.core,
            self.state.approver_target().unwrap_or(""),
            &announce,
        )
        .await
        {
            eprintln!("[gateway] approval announcement failed: {e}");
            self.state.resolve_oldest_gate(false).ok();
            return false;
        }
        match tokio::time::timeout(self.state.approval_timeout(), rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => false, // gate dropped (run cancelled)
            Err(_) => {
                // Timed out: remove our own entry so a late reply resolves
                // nothing.
                self.state
                    .pending_approvals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&id);
                eprintln!("[gateway] approval {short} timed out; denied");
                false
            }
        }
    }
}

/// "yes"/"no" vocabulary for chat replies. Deliberately small and strict —
/// casual chatter from the approver chat must not resolve gates.
fn parse_verdict(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "y" | "yes" | "approve" | "approved" | "ok" | "allow" => Some(true),
        "n" | "no" | "deny" | "denied" | "block" => Some(false),
        _ => None,
    }
}

async fn gateway_inbound(
    State(state): State<AppState>,
    Json(body): Json<InboundBody>,
) -> axum::response::Response {
    if !state.gateway.enabled {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "gateway disabled: set [gateway] enabled = true (trusted config) or pass serve --gateway"
            })),
        )
            .into_response();
    }
    let text = body.text.trim().to_string();
    if body.surface.trim().is_empty() || body.chat.trim().is_empty() || text.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "surface, chat and text are required"})),
        )
            .into_response();
    }
    let key = format!("{}:{}", body.surface.trim(), body.chat.trim());

    // Approval replies from the designated approver surface resolve the
    // oldest forwarded gate instead of becoming conversation input. Any
    // non-verdict text from that chat falls through to normal routing.
    if state.gateway.forward_mode() && Some(key.as_str()) == state.gateway.approver_target() {
        if let Some(verdict) = parse_verdict(&text) {
            return match state.gateway.resolve_oldest_gate(verdict) {
                Ok(remaining) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "state": "approval_resolved",
                        "approved": verdict,
                        "remaining": remaining,
                    })),
                )
                    .into_response(),
                Err(()) => (
                    StatusCode::OK,
                    Json(serde_json::json!({ "state": "no_pending_approvals" })),
                )
                    .into_response(),
            };
        }
    }

    let handle = match resolve_session(&state, &key).await {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
    };

    // Busy? Queue as logged steering input; the running loop consumes it
    // between model steps, and any leftovers run as a continuation turn.
    let busy = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_none();
    if busy {
        handle.steering.push_steering(text);
        return (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "state": "steering_queued",
                "session_id": binding_session(&state, &key).unwrap_or_default(),
            })),
        )
            .into_response();
    }

    if state.core.provider().is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no provider credential configured"})),
        )
            .into_response();
    }

    let (reply_tx, reply_rx) = oneshot::channel::<String>();
    let want_reply = body.wait;
    start_turn_chain(&state, handle, text, want_reply.then_some(reply_tx));

    if !want_reply {
        return (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "state": "started" })),
        )
            .into_response();
    }
    match tokio::time::timeout(WAIT_TIMEOUT, reply_rx).await {
        Ok(Ok(text)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "state": "completed",
                "text": text,
                "session_id": binding_session(&state, &key),
            })),
        )
            .into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "turn chain ended without a reply"})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({
                "error": "turn did not finish in time; poll /sessions/{id}/transcript"
            })),
        )
            .into_response(),
    }
}

async fn gateway_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let bindings: Vec<serde_json::Value> = state
        .gateway
        .snapshot()
        .into_iter()
        .map(|(k, sid)| serde_json::json!({"target": k, "session_id": sid}))
        .collect();
    Json(serde_json::json!({
        "enabled": state.gateway.enabled,
        "cwd": state.core.cwd(),
        "bindings": bindings,
        "approvals": {
            "mode": state.gateway.approvals_mode(),
            "approver": state.gateway.approver_target(),
            "pending": state.gateway.pending_approval_count(),
        },
    }))
}

async fn gateway_unbind(State(state): State<AppState>, Path(key): Path<String>) -> StatusCode {
    if state.gateway.unbind(&state.core, &key) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- Session resolution -----------------------------------------------------

fn binding_session(state: &AppState, key: &str) -> Option<String> {
    state
        .gateway
        .bindings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .cloned()
}

/// Attach-or-create the session bound to `key`. Stale bindings (ledger
/// deleted through the normal endpoint) rebind to a fresh session.
async fn resolve_session(state: &AppState, key: &str) -> Result<Arc<SessionHandle>, String> {
    if let Some(sid) = binding_session(state, key) {
        if let Some(handle) = state.get(&sid) {
            return Ok(handle);
        }
        match state.core.open_session(&sid).await {
            Ok(session) => {
                let id = session
                    .header()
                    .map(|h| h.session_id.clone())
                    .unwrap_or_else(|| sid.clone());
                return Ok(crate::register_handle(
                    state,
                    id,
                    session,
                    state.core.cwd().clone(),
                ));
            }
            Err(_) => {
                state.gateway.unbind(&state.core, key);
            }
        }
    }
    let session = state
        .core
        .start_session()
        .await
        .map_err(|e| format!("start session: {e}"))?;
    let id = session
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();
    let handle = crate::register_handle(state, id.clone(), session, state.core.cwd().clone());
    // Two racing first-messages could each mint a session; last bind wins
    // and the loser stays a hidden header-only draft.
    state.gateway.bind(&state.core, key.to_string(), id);
    Ok(handle)
}

// ---- Turn execution ---------------------------------------------------------

/// Run a turn chain: prompt, then any steering left queued by concurrent
/// inbound messages, until the queue is dry. Sends one synthesized
/// `RunFinished` per turn so SSE consumers see normal terminal markers.
fn start_turn_chain(
    state: &AppState,
    handle: Arc<SessionHandle>,
    prompt: String,
    reply: Option<oneshot::Sender<String>>,
) {
    let core = state.core.clone();
    let gw = state.gateway.clone();
    tokio::spawn(execute_turn_chain(core, gw, handle, prompt, reply));
}

async fn execute_turn_chain(
    core: Core,
    gw: Arc<GatewayState>,
    handle: Arc<SessionHandle>,
    mut prompt: String,
    mut reply: Option<oneshot::Sender<String>>,
) {
    loop {
        let taken = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(taken) = taken else {
            // Lost the race with another writer mid-chain; hand our prompt
            // to the winner as steering instead of dropping it.
            handle.steering.push_steering(prompt);
            return;
        };
        let session_id = taken
            .header()
            .map(|h| h.session_id.clone())
            .unwrap_or_default();

        // Unattended policy: deny by default, forward to the approver
        // surface when configured (G2).
        let approver: Arc<dyn vak_agent::Approver> = if gw.forward_mode() {
            Arc::new(GatewayApprover {
                events_tx: handle.events_tx.clone(),
                state: gw.clone(),
                core: core.clone(),
            })
        } else {
            Arc::new(AutoDeny)
        };
        let events = crate::mpsc_to_broadcast(handle.events_tx.clone());
        let steering = handle.steering.clone();
        let cancel = handle
            .cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        let outcome = core
            .run_turn_with(
                taken,
                &prompt,
                cancel,
                Some(approver),
                None,
                Some(steering.clone()),
                events,
            )
            .await;

        *handle
            .cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            tokio_util::sync::CancellationToken::new();

        let (reply_text, is_error) = match outcome {
            Ok((o, log)) => {
                *handle
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(log);
                (outcome_text(&o), outcome_is_error(&o))
            }
            Err(e) => {
                recover_ledger(&core, handle.clone(), &session_id).await;
                (format!("error: {e}"), true)
            }
        };
        let _ = handle.events_tx.send(AgentEvent::RunFinished {
            summary: short_summary(&reply_text),
            is_error,
        });

        if let Some(tx) = reply.take() {
            let _ = tx.send(reply_text.clone());
        }

        let queued = steering.drain(vak_agent::DrainMode::All);
        if queued.is_empty() {
            return;
        }
        prompt = queued.join("\n\n");
    }
}

/// A `CoreError` loses the ledger handle (the agent consumed it); reopen the
/// JSONL so the session stays usable in this long-lived process.
async fn recover_ledger(core: &Core, handle: Arc<SessionHandle>, session_id: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    if let Ok(log) = core.open_session(session_id).await {
        *handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(log);
        return true;
    }
    false
}

fn outcome_text(o: &vak_agent::TurnOutcome) -> String {
    use vak_agent::TurnOutcome;
    match o {
        TurnOutcome::Completed { response } => {
            let t = response.text_content();
            if t.trim().is_empty() {
                "(no text)".into()
            } else {
                t
            }
        }
        TurnOutcome::Aborted { partial } => partial
            .as_ref()
            .map(|m| m.text_content())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| "(aborted)".into()),
        TurnOutcome::Failed { error } => format!("error: {error}"),
        TurnOutcome::MaxTurnsReached => "(stopped at max turns)".into(),
    }
}

fn outcome_is_error(o: &vak_agent::TurnOutcome) -> bool {
    matches!(
        o,
        vak_agent::TurnOutcome::Failed { .. } | vak_agent::TurnOutcome::MaxTurnsReached
    )
}

fn short_summary(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let mut s: String = first.chars().take(80).collect();
    if first.chars().count() > 80 {
        s.push('…');
    }
    if s.is_empty() { "completed".into() } else { s }
}

// ---- Outbound delivery ------------------------------------------------------

fn http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// Deliver `text` to a routing target. Transports:
/// - `log:<chat>`    — append-only journal at `<home>/gateway/deliveries.jsonl`
/// - `webhook:<name>`— POST JSON to the configured `[gateway.outbound.webhooks.<name>]`
pub async fn deliver(core: &Core, target: &str, text: &str) -> Result<(), String> {
    let Some((kind, chat)) = target.split_once(':') else {
        return Err(format!(
            "invalid deliver target '{target}': expected '<surface>:<chat>'"
        ));
    };
    match kind {
        "log" => deliver_log(core, target, text),
        "webhook" => deliver_webhook(core, chat, text).await,
        other => Err(format!("unsupported gateway surface '{other}'")),
    }
}

fn deliver_log(core: &Core, target: &str, text: &str) -> Result<(), String> {
    let path = deliveries_path(&core.sessions_home());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let line = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "target": target,
        "text": text,
    });
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open deliveries log: {e}"))?;
    writeln!(f, "{line}").map_err(|e| format!("append deliveries log: {e}"))
}

async fn deliver_webhook(core: &Core, name: &str, text: &str) -> Result<(), String> {
    let hook = core.config().gateway.webhooks.get(name).ok_or_else(|| {
        let known: Vec<&String> = core.config().gateway.webhooks.keys().collect();
        format!("unknown webhook '{name}'; configured: {known:?}")
    })?;
    let mut req = http_client().post(&hook.url).json(&serde_json::json!({
        "target": format!("webhook:{name}"),
        "text": text,
        "ts": chrono::Utc::now().to_rfc3339(),
    }));
    // Fail closed: a configured credential that is missing must not turn
    // into an unauthenticated post of agent output.
    if let Some(env_name) = &hook.token_env {
        let token = vak_config::get_var(env_name)
            .ok_or_else(|| format!("webhook '{name}' token_env '{env_name}' is not set"))?;
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("webhook '{name}' post failed: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("webhook '{name}' returned {status}"));
    }
    Ok(())
}
