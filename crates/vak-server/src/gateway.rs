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

use vak_agent::{AgentEvent, AutoDeny, SteeringQueues};
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
    pending_approvals: Mutex<std::collections::BTreeMap<String, PendingGate>>,
}

struct PendingGate {
    session_id: String,
    tx: oneshot::Sender<bool>,
}

/// What a resolved gate was, so a bare yes/no is never silent about which
/// session's tool run it just decided.
pub(crate) struct ResolvedGate {
    pub id: String,
    pub session_id: String,
    pub remaining: usize,
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

    /// Register a gate for `session_id` and hand back the reply receiver.
    /// The sender must be stored before the request is announced so an
    /// instant reply cannot race a missing entry.
    pub(crate) fn register_gate(&self, id: &str, session_id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.to_string(),
                PendingGate {
                    session_id: session_id.to_string(),
                    tx,
                },
            );
        rx
    }

    /// Resolve a gate. With an id prefix, only that exact gate resolves —
    /// a reply meant for one session can never approve another's tool run.
    /// Without one, the globally oldest gate resolves and is reported so
    /// the approver surface can see what their bare yes/no did.
    pub(crate) fn resolve_gate(
        &self,
        approve: bool,
        id_prefix: Option<&str>,
    ) -> Result<ResolvedGate, ()> {
        let mut map = self
            .pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = match id_prefix {
            Some(prefix) => match map.keys().find(|k| k.starts_with(prefix)).cloned() {
                Some(k) => k,
                None => return Err(()),
            },
            None => map.keys().next().cloned().ok_or(())?,
        };
        let (_, gate) = map.remove_entry(&key).ok_or(())?;
        let _ = gate.tx.send(approve);
        Ok(ResolvedGate {
            id: key,
            session_id: gate.session_id,
            remaining: map.len(),
        })
    }

    pub(crate) fn snapshot(&self) -> Vec<(String, String)> {
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
    /// Who sent this, when the adapter knows (a Telegram @user, a webhook
    /// identity). Typed since G1 groundwork: it is recorded on deliveries
    /// and attributed on queued turns instead of being silently dropped.
    #[serde(default)]
    sender: Option<String>,
    text: String,
    #[serde(default)]
    wait: bool,
    /// Base64 images appended to the prompt as vision content.
    #[serde(default)]
    attachments: Vec<InboundAttachment>,
}

#[derive(serde::Deserialize)]
struct InboundAttachment {
    /// MIME type; defaults to image/png for Telegram-style senders.
    #[serde(default = "default_image_mime")]
    mime: String,
    data: String,
}

fn default_image_mime() -> String {
    "image/png".into()
}

/// Compose the prompt message: text plus any vision blocks. The ledger
/// stores exactly what the model will see (invariant 1).
fn compose_prompt(text: &str, attachments: &[InboundAttachment]) -> vak_llm::Message {
    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(vak_llm::ContentBlock::text(text));
    }
    for a in attachments {
        if a.data.trim().is_empty() {
            continue;
        }
        blocks.push(vak_llm::ContentBlock::image_base64(
            a.mime.clone(),
            a.data.trim().to_string(),
        ));
    }
    vak_llm::Message {
        role: vak_llm::Role::User,
        content: blocks,
    }
}

// ---- Approval forwarding (G2) ----------------------------------------------

/// Approver for gateway-driven turns. In `deny` mode this behaves like
/// `AutoDeny`. In `forward` mode the gate is announced on the approver
/// surface and resolved by a yes/no reply; timeout or silence fails closed.
struct GatewayApprover {
    events_tx: tokio::sync::broadcast::Sender<AgentEvent>,
    state: Arc<GatewayState>,
    core: Core,
    session_id: String,
}

#[async_trait::async_trait]
impl vak_agent::Approver for GatewayApprover {
    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool {
        if !self.state.forward_mode() {
            return false;
        }
        let id = uuid::Uuid::now_v7().to_string();
        let rx = self.state.register_gate(&id, &self.session_id);
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
            let _ = self.state.resolve_gate(false, Some(&id));
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

/// "yes"/"no" vocabulary for chat replies, optionally addressed to one
/// gate: "yes ab12cd34". Deliberately small and strict — casual chatter
/// from the approver chat must not resolve gates. Returns the verdict and
/// the gate-id prefix when one was supplied.
fn parse_verdict(text: &str) -> Option<(bool, Option<String>)> {
    let mut tokens = text.split_whitespace();
    let head = tokens.next()?.to_lowercase();
    let verdict = match head.as_str() {
        "y" | "yes" | "approve" | "approved" | "ok" | "allow" => true,
        "n" | "no" | "deny" | "denied" | "block" => false,
        _ => return None,
    };
    // Extra prose after a bare verdict is ignored; exactly one short token
    // is treated as a gate id.
    let id = match tokens.next() {
        Some(t)
            if tokens.next().is_none()
                && t.len() >= 4
                && t.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            Some(t.to_lowercase())
        }
        _ => None,
    };
    Some((verdict, id))
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
    let has_image = body.attachments.iter().any(|a| !a.data.trim().is_empty());
    if body.surface.trim().is_empty()
        || body.chat.trim().is_empty()
        || (text.is_empty() && !has_image)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "surface, chat and text are required"})),
        )
            .into_response();
    }
    let key = format!("{}:{}", body.surface.trim(), body.chat.trim());

    // Approval replies from the designated approver surface resolve the
    // addressed gate (or the oldest one) instead of becoming conversation
    // input. Any non-verdict text from that chat falls through to normal
    // routing.
    if state.gateway.forward_mode()
        && Some(key.as_str()) == state.gateway.approver_target()
        && let Some((verdict, gate_id)) = parse_verdict(&text)
    {
        return match state.gateway.resolve_gate(verdict, gate_id.as_deref()) {
            Ok(resolved) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "state": "approval_resolved",
                    "approved": verdict,
                    "gate": resolved.id,
                    "session_id": resolved.session_id,
                    "remaining": resolved.remaining,
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
    // The full composed message (text + images) is queued so nothing the
    // sender supplied is degraded to bare text.
    let busy = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_none();
    if busy {
        let attributed = match body.sender.as_deref().map(str::trim) {
            Some(who) if !who.is_empty() => format!("[from {who}] {text}"),
            _ => text.clone(),
        };
        handle
            .steering
            .push_steering_message(compose_prompt(&attributed, &body.attachments));
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
    let prompt = compose_prompt(&text, &body.attachments);
    start_turn_chain(&state, handle, prompt, want_reply.then_some(reply_tx));

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
    prompt: vak_llm::Message,
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
    mut prompt: vak_llm::Message,
    mut reply: Option<oneshot::Sender<String>>,
) {
    loop {
        let taken = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(taken) = taken else {
            // Lost the race with another writer mid-chain; hand our full
            // prompt (text + images) to the winner as steering instead of
            // dropping it.
            handle.steering.push_steering_message(prompt.clone());
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
                session_id: session_id.clone(),
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
            .run_turn_with_message(
                taken,
                prompt.clone(),
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

        // Reflection stage (docs/design/26-learning.md L1): after a clean
        // completion, optionally propose durable notes/skills. Detached and
        // best-effort — reflection failures must never touch the reply or
        // the session.
        if core.config().memory.reflection && !is_error {
            let rcore = core.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                match reflection_tail(&rcore, &sid).await {
                    Ok((notes, skill)) => {
                        if notes > 0 || skill {
                            eprintln!(
                                "[gateway] reflection: {notes} note(s) persisted, skill queued: {skill}"
                            );
                        }
                    }
                    Err(e) => eprintln!("[gateway] reflection skipped: {e}"),
                }
            });
        }

        let queued = steering.drain(vak_agent::DrainMode::All);
        match SteeringQueues::merge_prompt(queued) {
            None => return,
            Some(merged) => prompt = merged,
        }
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
    // One formatted buffer + ONE write_all: O_APPEND makes a single write
    // atomic, whereas `writeln!` emits several syscalls that two concurrent
    // deliveries can interleave mid-line.
    let mut buf = line.to_string();
    buf.push('\n');
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open deliveries log: {e}"))?;
    f.write_all(buf.as_bytes())
        .map_err(|e| format!("append deliveries log: {e}"))
}

/// Transient webhook failures retry with bounded exponential backoff.
/// 4xx (except 429) are the receiver's permanent answer and return at once;
/// network errors, timeouts, 429 and 5xx are retried.
const WEBHOOK_ATTEMPTS: u32 = 3;

fn webhook_retryable(status: Option<u16>) -> bool {
    match status {
        None => true,
        Some(429) => true,
        Some(c) => c >= 500,
    }
}

async fn deliver_webhook(core: &Core, name: &str, text: &str) -> Result<(), String> {
    let hook = core.config().gateway.webhooks.get(name).ok_or_else(|| {
        let known: Vec<&String> = core.config().gateway.webhooks.keys().collect();
        format!("unknown webhook '{name}'; configured: {known:?}")
    })?;
    // Fail closed: a configured credential that is missing must not turn
    // into an unauthenticated post of agent output.
    let token = match &hook.token_env {
        Some(env_name) => Some(
            vak_config::get_var(env_name)
                .ok_or_else(|| format!("webhook '{name}' token_env '{env_name}' is not set"))?,
        ),
        None => None,
    };
    let payload = serde_json::json!({
        "target": format!("webhook:{name}"),
        "text": text,
        "ts": chrono::Utc::now().to_rfc3339(),
    });

    let mut last_error = String::new();
    for attempt in 0..WEBHOOK_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400u64 << (attempt - 1))).await;
        }
        let mut req = http_client().post(&hook.url).json(&payload);
        if let Some(token) = &token {
            req = req.bearer_auth(token);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(());
                }
                last_error = format!("webhook '{name}' returned {status}");
                if !webhook_retryable(Some(status.as_u16())) {
                    return Err(last_error);
                }
            }
            Err(e) => {
                last_error = format!("webhook '{name}' post failed: {e}");
            }
        }
    }
    Err(last_error)
}

/// Reflection helper for the gateway: pulls the bound session's transcript
/// tail, runs the auxiliary proposal call through the core's provider, and
/// applies deduped writes. Returns (notes written, skill queued).
pub(crate) async fn reflection_tail(
    core: &Core,
    session_id: &str,
) -> Result<(usize, bool), String> {
    let provider = core
        .provider()
        .map_err(|e| format!("reflection provider: {e}"))?;
    // Read the transcript tail straight from disk: the live ledger may be
    // owned by the agent at reflection time.
    let path =
        vak_session::SessionPath::new_session_file(&core.sessions_home(), core.cwd(), session_id);
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read session for reflection: {e}"))?;
    let mut tail = String::new();
    for line in text.lines() {
        if let Ok(e) = serde_json::from_str::<serde_json::Value>(line)
            && e["kind"] == "message"
        {
            let role = e["message"]["role"].as_str().unwrap_or("?");
            let txt = e["message"]["content"]
                .as_array()
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            tail.push_str(&format!("{role}: {txt}\n"));
        }
    }

    let model = core.effective_model();
    let proposals = vak_core::reflection::propose(
        provider,
        &model,
        &tail,
        tokio_util::sync::CancellationToken::new(),
    )
    .await?;
    if proposals.notes.is_empty() && proposals.skill.is_none() {
        return Ok((0, false));
    }
    let home_dir = core.sessions_home();
    let cwd = core.cwd().clone();
    vak_core::reflection::apply(home_dir.as_path(), cwd.as_path(), session_id, &proposals)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn bare_verdicts_have_no_gate_id() {
        assert_eq!(parse_verdict("yes"), Some((true, None)));
        assert_eq!(parse_verdict("  NO "), Some((false, None)));
        assert_eq!(parse_verdict("approve"), Some((true, None)));
    }

    #[test]
    fn addressed_verdict_extracts_single_short_token() {
        assert_eq!(
            parse_verdict("yes ab12cd34"),
            Some((true, Some("ab12cd34".into())))
        );
        assert_eq!(
            parse_verdict("no deadbeef"),
            Some((false, Some("deadbeef".into())))
        );
    }

    #[test]
    fn prose_after_verdict_is_never_an_id() {
        assert_eq!(parse_verdict("yes please do it now"), Some((true, None)));
        assert_eq!(parse_verdict("no way"), Some((false, None)));
    }

    #[test]
    fn chatter_is_not_a_verdict() {
        assert_eq!(parse_verdict("sure thing"), None);
        assert_eq!(parse_verdict(""), None);
        assert_eq!(parse_verdict("approved!"), None);
    }

    #[test]
    fn webhook_retry_matrix() {
        assert!(webhook_retryable(None), "network error retries");
        assert!(webhook_retryable(Some(429)));
        assert!(webhook_retryable(Some(503)));
        assert!(!webhook_retryable(Some(401)), "auth is permanent");
        assert!(!webhook_retryable(Some(404)));
        assert!(!webhook_retryable(Some(200)));
    }
}
