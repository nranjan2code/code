//! vak-server: HTTP+SSE wrapper around vak-core. The TUI, web clients, and
//! IDE extensions are all just consumers of these endpoints — one headless
//! agent core, many surfaces.
//!
//! Endpoints:
//! - `POST /sessions`                     → {session_id}
//! - `POST /sessions/:id/run` {prompt}    → 202 (events stream on SSE)
//! - `POST /sessions/:id/steering` {text} → 202
//! - `POST /sessions/:id/approvals/:rid` {approve} → resolve a pending gate
//! - `GET  /sessions/:id/events`          → SSE of AgentEvent JSON
//! - `GET  /sessions/:id/transcript`      → derived messages + usage
//! - `GET  /health`

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, Approver, SteeringQueues};
use vak_core::Core;
use vak_session::SessionLog;

struct SessionHandle {
    session: Arc<Mutex<Option<SessionLog>>>,
    steering: Arc<SteeringQueues>,
    /// Cancel for the CURRENT run only; replaced with a fresh token when a
    /// run ends so one `/cancel` doesn't poison every later run.
    cancel: Arc<std::sync::Mutex<CancellationToken>>,
    events_tx: broadcast::Sender<AgentEvent>,
    /// Pending approval gates scoped to THIS session — a client holding
    /// session A can never resolve session B's approvals.
    pending: Arc<Mutex<HashMap<String, ApprovalRequest>>>,
    /// Notified when an SSE consumer attaches, so runs don't start (and
    /// finish) before anyone is listening.
    subscribed: Arc<tokio::sync::Notify>,
}

#[derive(Clone)]
pub struct AppState {
    pub core: Core,
    sessions: Arc<Mutex<HashMap<String, Arc<SessionHandle>>>>,
}

impl AppState {
    pub fn new(core: Core) -> Self {
        AppState {
            core,
            sessions: Arc::new(Mutex::new(HashMap::new())),
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
    async fn approve(&self, tool: &str, reason: &str) -> bool {
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
    let state = AppState::new(core);
    Router::new()
        .route("/health", get(health))
        .route("/sessions", post(create_session))
        .route("/sessions/{id}/run", post(run_prompt))
        .route("/sessions/{id}/steering", post(send_steering))
        .route("/sessions/{id}/cancel", post(cancel_run))
        .route("/sessions/{id}/approvals/{req_id}", post(answer_approval))
        .route("/sessions/{id}/events", get(events_sse))
        .route("/sessions/{id}/transcript", get(transcript))
        .with_state(state)
}

pub async fn serve(core: Core, addr: std::net::SocketAddr) -> std::io::Result<()> {
    // Local-only does not mean safe-by-default: any local process could
    // reach an unauthenticated agent and drive arbitrary tool execution
    // plus self-approval. Every serve() instance gets a per-process
    // bearer token; /health stays open.
    let token = format!("vk_{}", uuid::Uuid::now_v7());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("vakcoder server listening on http://{addr}");
    eprintln!("auth token: {token}");
    eprintln!("clients must send 'Authorization: Bearer {token}' (or ?token=)");
    let app = router(core).layer(axum::middleware::from_fn_with_state(
        token.clone(),
        require_bearer,
    ));
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
        "warnings": state.core.config().warnings,
    }))
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
    let (events_tx, _) = broadcast::channel(1024);

    state
        .sessions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            id.clone(),
            Arc::new(SessionHandle {
                session: Arc::new(Mutex::new(Some(session))),
                steering: Arc::new(SteeringQueues::new()),
                cancel: Arc::new(std::sync::Mutex::new(CancellationToken::new())),
                events_tx,
                pending: Arc::new(Mutex::new(HashMap::new())),
                subscribed: Arc::new(tokio::sync::Notify::new()),
            }),
        );

    Json(serde_json::json!({ "session_id": id }))
}

#[derive(serde::Deserialize)]
struct RunBody {
    prompt: String,
}

fn mpsc_to_broadcast(tx: broadcast::Sender<AgentEvent>) -> mpsc::Sender<AgentEvent> {
    let (tx_in, mut rx) = mpsc::channel::<AgentEvent>(512);
    tokio::spawn(async move {
        // Forward into the BROADCAST channel (sync send). Forwarding into
        // tx_in would feed the channel back into itself.
        while let Some(ev) = rx.recv().await {
            if tx.send(ev).is_err() {
                break;
            }
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

    tokio::spawn(async move {
        let outcome = core
            .run_turn_with(taken, &body.prompt, cancel, Some(approver), None, events)
            .await;
        // Reset the token so the next run on this session is not born
        // already-cancelled.
        *handle
            .cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = CancellationToken::new();
        match outcome {
            Ok((o, session_log)) => {
                // Return the ledger so transcript stays available.
                *handle
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session_log);
                let summary = match &o {
                    vak_agent::TurnOutcome::Completed { .. } => "completed".to_string(),
                    vak_agent::TurnOutcome::Aborted { .. } => "aborted".to_string(),
                    vak_agent::TurnOutcome::Failed { error } => format!("failed: {error}"),
                    vak_agent::TurnOutcome::MaxTurnsReached => "max_turns".to_string(),
                };
                let _ = handle.events_tx.send(AgentEvent::RunFinished { summary });
            }
            Err(e) => {
                let _ = handle.events_tx.send(AgentEvent::RunFinished {
                    summary: format!("error: {e}"),
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
}

async fn send_steering(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SteeringBody>,
) -> StatusCode {
    let Some(handle) = state.get(&id) else {
        return StatusCode::NOT_FOUND;
    };
    handle.steering.push_steering(body.text);
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
    let _ = handle.events_tx.send(AgentEvent::RunFinished {
        summary: "cancelled by client".into(),
    });
    StatusCode::ACCEPTED
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
) -> Json<serde_json::Value> {
    let Some(handle) = state.get(&id) else {
        return Json(serde_json::json!({ "error": "unknown session" }));
    };
    let guard = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(s) = guard.as_ref() else {
        return Json(serde_json::json!({ "error": "run in progress" }));
    };
    let msgs = s.derive_messages();
    Json(serde_json::json!({
        "count": msgs.len(),
        "usage": s.total_usage(),
        "messages": msgs,
    }))
}
