//! Admin console API endpoints (docs/design/29-personal-os.md): session
//! catalog, transcripts, live event SSE, security audit log, and store
//! management. Mounted under `/admin/api` in `router_with_state`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::AppState;

pub(crate) const SESSION_COOKIE: &str = "vak_session";

// ---- Auth: POST /admin/login, POST /admin/logout ---------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct LoginBody {
    pub token: String,
}

/// Constant-time token check → HttpOnly session cookie. Browsers need this
/// because EventSource cannot send Authorization headers.
///
/// No `Secure` flag on purpose: this server is loopback-first and plain
/// http://localhost would silently drop Secure cookies.
pub(crate) async fn login(State(state): State<AppState>, Json(body): Json<LoginBody>) -> Response {
    use subtle::ConstantTimeEq;
    let ok: bool = body
        .token
        .as_bytes()
        .ct_eq(state.auth_token.as_bytes())
        .into();
    if !ok {
        let ip = None;
        vak_core::security_events::record(
            &state.core.sessions_home(),
            vak_core::security_events::EventKind::AuthFailure,
            "admin_login_failed",
            "invalid token on /admin/login",
            ip,
        );
        state.hub.emit_security("AuthFailure", "admin_login_failed");
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "invalid token"
            })),
        )
            .into_response();
    }
    (
        [(
            axum::http::header::SET_COOKIE,
            format!(
                "{}={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=604800",
                SESSION_COOKIE, body.token
            ),
        )],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

pub(crate) async fn logout() -> Response {
    (
        [(
            axum::http::header::SET_COOKIE,
            format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"),
        )],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

// ---- GET /admin/api/sessions ----------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct SessionListQuery {
    pub limit: Option<usize>,
    pub project: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionListItem {
    pub session_id: String,
    pub project_hash: String,
    pub entry_count: usize,
    pub first_ts: String,
    pub last_ts: String,
}

pub(crate) async fn list_sessions_admin(
    State(state): State<AppState>,
    Query(q): Query<SessionListQuery>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return Json(serde_json::json!({ "error": "store not available" }));
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    match store.list_sessions() {
        Ok(all) => {
            let items: Vec<SessionListItem> = all
                .into_iter()
                .filter(|s| q.project.as_ref().is_none_or(|p| &s.project_hash == p))
                .take(limit)
                .map(|s| SessionListItem {
                    session_id: s.session_id,
                    project_hash: s.project_hash,
                    entry_count: s.entry_count,
                    first_ts: s.first_ts,
                    last_ts: s.last_ts,
                })
                .collect();
            Json(serde_json::json!({ "sessions": items }))
        }
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// ---- GET /admin/api/approvals ----------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct PendingApproval {
    pub session_id: String,
    pub request_id: String,
    pub tool: String,
    pub args_json: String,
    pub reason: String,
    pub requested_at: String,
}

/// Aggregated pending approval gates across every live session. Answering
/// still goes through the per-session endpoint — listing never crosses a
/// session's approval scope, only displays it.
pub(crate) async fn list_pending_approvals(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let mut items: Vec<PendingApproval> = Vec::new();
    for handle in state.live_handles() {
        let pending = handle
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for req in pending.values() {
            items.push(PendingApproval {
                session_id: handle.id.clone(),
                request_id: req.id.clone(),
                tool: req.tool.clone(),
                args_json: truncate_chars(&req.args_json, 400),
                reason: req.reason.clone(),
                requested_at: req.requested_at.to_rfc3339(),
            });
        }
    }
    // Oldest first — the gate that has been waiting longest is the most urgent.
    items.sort_by(|a, b| a.requested_at.cmp(&b.requested_at));
    let total = items.len();
    Json(serde_json::json!({ "approvals": items, "total": total }))
}

// ---- GET /admin/api/sessions/:id/transcript --------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub kind: Option<String>,
    pub role: Option<String>,
    /// Re-import this session's JSONL into the index before reading, so
    /// entries appended by an active run become visible immediately.
    pub refresh: Option<bool>,
}

/// Byte-safe truncation: never splits a multi-byte UTF-8 sequence.
fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

pub(crate) async fn session_transcript_admin(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return Json(serde_json::json!({ "error": "store not available" }));
    };
    // Fetch offset+limit so we can report whether more pages exist.
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let offset = q.offset.unwrap_or(0);
    if q.refresh == Some(true) {
        crate::import_session_sync(store, &state.core.sessions_home(), &session_id);
    }
    let filter = vak_store::query::SearchFilter {
        session_id: Some(session_id.clone()),
        kind: q.kind,
        role: q.role,
        ..Default::default()
    };
    match store.query_page(&filter, limit, offset, true) {
        Ok((entries, total)) => {
            let has_more = offset.saturating_add(entries.len()) < total;
            let page: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "entry_id": e.entry_id,
                        "ts": e.ts,
                        "kind": e.kind.as_str(),
                        "role": e.role,
                        "tool_name": e.tool_name,
                        "is_error": e.is_error,
                        "content": truncate_chars(&e.content_text, 16000),
                    })
                })
                .collect();
            Json(serde_json::json!({
                "session_id": session_id,
                "entries": page,
                "offset": offset,
                "total": total,
                "has_more": has_more,
            }))
        }
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// ---- GET /admin/api/search -------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct SearchQuery {
    pub q: String,
    pub limit: Option<usize>,
    pub project: Option<String>,
    pub role: Option<String>,
    pub kind: Option<String>,
    pub exclude_session: Option<String>,
}

pub(crate) async fn search_admin(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return Json(serde_json::json!({ "error": "store not available" }));
    };
    let limit = q.limit.unwrap_or(20).clamp(1, 100);
    let filter = vak_store::query::SearchFilter {
        project_hash: q.project,
        role: q.role,
        kind: q.kind,
        exclude_session: q.exclude_session,
        ..Default::default()
    };
    match store.search(&q.q, limit, &filter) {
        Ok(result) => Json(serde_json::json!({
            "hits": result.entries,
            "total": result.total,
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// ---- GET /admin/api/events (SSE) ------------------------------------------

pub(crate) async fn admin_events_sse(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::wrappers::BroadcastStream;

    let rx = state.hub.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let json = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().data(json)))
        }
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            let lagged = serde_json::json!({
                "type": "Lagged",
                "data": { "missed": n }
            });
            let json = serde_json::to_string(&lagged).unwrap_or_default();
            Some(Ok(Event::default().data(json)))
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---- GET /admin/api/security -----------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct SecurityQuery {
    pub limit: Option<usize>,
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct SecurityEventEntry {
    ts: String,
    kind: String,
    label: String,
    detail: String,
    ip: Option<String>,
}

pub(crate) async fn list_security_events(
    State(state): State<AppState>,
    Query(q): Query<SecurityQuery>,
) -> Json<serde_json::Value> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let home = state.core.sessions_home();
    let events = vak_core::security_events::list(&home, limit);
    let filtered: Vec<SecurityEventEntry> = events
        .into_iter()
        .filter(|e| {
            q.kind.as_deref().is_none_or(|k| {
                serde_json::to_value(&e.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .as_deref()
                    == Some(k)
            })
        })
        .map(|e| SecurityEventEntry {
            ts: e.ts.to_rfc3339(),
            kind: serde_json::to_value(&e.kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "unknown".into()),
            label: e.label,
            detail: e.detail,
            ip: e.ip,
        })
        .collect();
    Json(serde_json::json!({
        "events": filtered,
        "total": filtered.len(),
    }))
}

// ---- POST /admin/api/store/rebuild ----------------------------------------

pub(crate) async fn rebuild_store(State(state): State<AppState>) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return Json(serde_json::json!({ "ok": false, "error": "store not available" }));
    };
    let home = state.core.sessions_home();
    match store.rebuild(&home) {
        Ok(stats) => Json(serde_json::json!({
            "ok": true,
            "files_scanned": stats.files_scanned,
            "entries_indexed": stats.entries_indexed,
            "fts_rows": stats.fts_rows,
        })),
        Err(e) => Json(serde_json::json!({
            "ok": false,
            "error": e.to_string(),
        })),
    }
}

// ---- GET /admin/api/store/import/:session_id -------------------------------

pub(crate) async fn import_session_store(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return Json(serde_json::json!({ "ok": false, "error": "store not available" }));
    };
    let home = state.core.sessions_home();
    let sessions_dir = home.join("sessions");
    if !sessions_dir.exists() {
        return Json(serde_json::json!({ "error": "no sessions directory" }));
    }
    for entry in walkdir::WalkDir::new(&sessions_dir)
        .min_depth(2)
        .max_depth(2)
        .into_iter()
        .filter_entry(|e| e.file_type().is_file())
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
            && path.file_stem().and_then(|s| s.to_str()) == Some(&session_id)
        {
            match store.import_session(&home, path) {
                Ok(stats) => {
                    return Json(serde_json::json!({
                        "ok": true,
                        "entries_indexed": stats.entries_indexed,
                        "fts_rows": stats.fts_rows,
                        "skipped": stats.skipped,
                    }));
                }
                Err(e) => {
                    return Json(serde_json::json!({
                        "ok": false,
                        "error": e.to_string(),
                    }));
                }
            }
        }
    }
    Json(serde_json::json!({ "error": format!("session {session_id} not found") }))
}

// ---- GET /admin/api/bestofn ------------------------------------------------

#[derive(Debug, Serialize)]
struct BestOfNRun {
    session_id: String,
    repo: String,
    branch: String,
}

/// Active best-of-N candidate runs keyed by child session.
pub(crate) async fn list_bestofn(State(state): State<AppState>) -> Json<serde_json::Value> {
    let map = state
        .best_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let runs: Vec<BestOfNRun> = map
        .iter()
        .map(|(id, meta)| BestOfNRun {
            session_id: id.clone(),
            repo: meta.repo.display().to_string(),
            branch: meta.branch.clone(),
        })
        .collect();
    let total = runs.len();
    Json(serde_json::json!({ "runs": runs, "total": total }))
}

// ---- GET /admin/api/config ------------------------------------------------

pub(crate) async fn get_config_admin(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Report the EFFECTIVE provider, model, turns, mode, and theme so runtime
    // overrides applied by PATCH /config and POST /config/mode are accurately returned.
    Json(serde_json::json!({
        "provider": state.core.effective_provider(),
        "model": state.core.effective_model(),
        "provider_source": state.core.provider_source(),
        "model_source": state.core.model_source(),
        "max_turns": state.core.effective_max_turns(),
        "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
        "theme": state.core.effective_theme(),
    }))
}

// ---- GET /admin/api/gateway/status ----------------------------------------

pub(crate) async fn gateway_status_admin(State(state): State<AppState>) -> Json<serde_json::Value> {
    let gw = &state.gateway;
    let bindings: Vec<String> = gw
        .bindings_snapshot()
        .into_iter()
        .map(|(k, _v)| k)
        .collect();
    Json(serde_json::json!({
        "enabled": gw.enabled,
        "bindings": bindings,
        "chat_allowlist": state.core.config().gateway.chat_allowlist,
    }))
}

// ---- Admin route mounter --------------------------------------------------

pub(crate) fn routes() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/admin/login", post(login))
        .route("/admin/logout", post(logout))
        .route("/admin/api/sessions", get(list_sessions_admin))
        .route(
            "/admin/api/sessions/{id}/transcript",
            get(session_transcript_admin),
        )
        .route("/admin/api/approvals", get(list_pending_approvals))
        .route("/admin/api/bestofn", get(list_bestofn))
        .route("/admin/api/search", get(search_admin))
        .route("/admin/api/events", get(admin_events_sse))
        .route("/admin/api/security", get(list_security_events))
        // Mutations are POST: crawlers/prefetchers only ever issue GETs.
        .route("/admin/api/store/rebuild", post(rebuild_store))
        .route(
            "/admin/api/store/import/{session_id}",
            post(import_session_store),
        )
        .route("/admin/api/config", get(get_config_admin))
        .route("/admin/api/gateway/status", get(gateway_status_admin))
}

// ---- Tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> AppState {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        let core = vak_core::Core::new(cwd).unwrap();
        AppState::new(core)
    }

    fn authed_app(state: &AppState) -> axum::Router {
        crate::router_with_state(state.clone()).layer(axum::middleware::from_fn_with_state(
            ((*state.auth_token).clone(), state.core.sessions_home()),
            crate::require_bearer,
        ))
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn list_sessions_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/sessions")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["sessions"].is_array());
    }

    #[tokio::test]
    async fn search_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/search?q=hello")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json.get("total").is_some());
    }

    #[tokio::test]
    async fn security_events_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/security")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn store_rebuild_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/store/rebuild")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["ok"].as_bool().unwrap_or(false));
    }

    #[tokio::test]
    async fn rebuild_via_get_is_rejected() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/store/rebuild")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn login_sets_cookie_and_it_authenticates() {
        use axum::http::header;
        let state = test_state();
        let token = (*state.auth_token).clone();

        // Login itself is auth-exempt; plain router is fine for it.
        let plain = crate::router_with_state(state.clone());

        // Wrong token → 401.
        let req = Request::builder()
            .method("POST")
            .uri("/admin/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"token":"wrong"}"#))
            .unwrap();
        let resp = plain.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Correct token → 200 + cookie.
        let req = Request::builder()
            .method("POST")
            .uri("/admin/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"token":"{token}"}}"#)))
            .unwrap();
        let resp = plain.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let set_cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(set_cookie.starts_with("vak_session="), "cookie must be set");
        assert!(set_cookie.contains("HttpOnly"));

        // Cookie authenticates a protected admin endpoint with no header.
        let cookie_pair = &set_cookie[..set_cookie.find(';').unwrap_or(set_cookie.len())];
        let authed = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/sessions")
            .header(header::COOKIE, cookie_pair)
            .body(Body::empty())
            .unwrap();
        let resp = authed.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unauthenticated_admin_api_is_401() {
        let state = test_state();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// `EventSource` cannot set request headers, and the desktop app
    /// never performs the `/admin/login` cookie exchange -- that is the
    /// browser console's flow. `?token=` is the only channel its SSE
    /// streams can authenticate on, and `openEventStream` /
    /// `openSideStream` have always used it. The middleware accepted
    /// only the header and the cookie, so every desktop event stream
    /// was rejected 401 and the app received not one agent event: runs
    /// completed and were durably logged while the UI showed no reply,
    /// "Working" forever, and `0 in / 0 out`. Nothing surfaced in the
    /// console either, because a 401 on an EventSource is just a bare
    /// `onerror`.
    ///
    /// Found by opening the real SSE endpoint with curl and getting
    /// zero bytes back, after three unrelated "fixes" shipped on
    /// theory without ever testing this path.
    #[tokio::test]
    async fn sse_authenticates_with_the_token_query_parameter() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri(format!("/sessions/does-not-exist/events?token={token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // The session id is bogus, so any status is acceptable EXCEPT
        // 401: this asserts the request got past authentication, which
        // is the thing that was broken. Asserting 200 would instead
        // pin unrelated session-lookup behaviour.
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "?token= must authenticate -- it is the only channel EventSource has"
        );
    }

    #[tokio::test]
    async fn a_wrong_token_query_parameter_is_still_rejected() {
        let state = test_state();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/sessions/does-not-exist/events?token=vk_not-the-real-token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn pending_approvals_lists_empty_without_gates() {
        let state = test_state();
        let token = (*state.auth_token).clone();

        // Create a live session so the aggregation path has something to walk.
        {
            let core = state.core.clone();
            let s = core.start_session().await.unwrap();
            let id = s.header().map(|h| h.session_id.clone()).unwrap_or_default();
            crate::register_handle(&state, id, s, state.core.cwd().clone());
        }

        let app =
            crate::router_with_state(state.clone()).layer(axum::middleware::from_fn_with_state(
                ((*state.auth_token).clone(), state.core.sessions_home()),
                crate::require_bearer,
            ));
        let req = Request::builder()
            .uri("/admin/api/approvals")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["total"], 0);
        assert!(json["approvals"].is_array());
    }

    #[tokio::test]
    async fn config_endpoint_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/config")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["provider"].is_string());
    }
}
