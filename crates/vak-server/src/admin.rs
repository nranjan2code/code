//! Admin console API endpoints (docs/design/29-personal-os.md): session
//! catalog, transcripts, live event SSE, security audit log, and store
//! management. Mounted under `/admin/api` in `router_with_state`.

use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
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
            let contract = state
                .get(&session_id)
                .and_then(|handle| {
                    handle
                        .session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .as_ref()
                        .and_then(|session| session.header().map(|header| header.contract.clone()))
                })
                .or_else(|| {
                    crate::open_historical_session(&state, &session_id)
                        .and_then(|session| session.header().map(|header| header.contract.clone()))
                });
            let configuration_mismatch = contract.as_ref().is_some_and(|contract| {
                contract.provider != state.core.effective_provider()
                    || contract.model != state.core.effective_model()
            });
            Json(serde_json::json!({
                "session_id": session_id,
                "entries": page,
                "offset": offset,
                "total": total,
                "has_more": has_more,
                "contract": contract,
                "configuration_mismatch": configuration_mismatch,
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
    crate::refresh_control_plane(&state);
    let route = state.core.effective_route();
    Json(serde_json::json!({
        "provider": route.provider,
        "model": route.model,
        "provider_source": route.provider_source,
        "model_source": route.model_source,
        "route_revision": route.revision,
        "max_turns": state.core.effective_max_turns(),
        "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
        "theme": state.core.effective_theme(),
    }))
}

// ---- GET /admin/api/gateway/status ----------------------------------------

pub(crate) async fn gateway_status_admin(State(state): State<AppState>) -> Json<serde_json::Value> {
    let gw = &state.gateway;
    crate::refresh_control_plane(&state);
    let default_route = state.core.effective_route();
    let mut bindings = Vec::new();
    for (target, binding) in gw.bindings_snapshot() {
        let channel_override = binding.provider.clone().zip(binding.model.clone());
        let (provider, model, source, revision) = match &channel_override {
            Some((provider, model)) => (
                provider.clone(),
                model.clone(),
                "channel_override",
                format!("channel:{provider}:{model}"),
            ),
            None => (
                default_route.provider.clone(),
                default_route.model.clone(),
                "workspace_default",
                default_route.revision.clone(),
            ),
        };
        let contract = binding.session_id.as_ref().and_then(|session_id| {
            state
                .get(session_id)
                .and_then(|handle| {
                    handle
                        .session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .as_ref()
                        .and_then(|session| session.header().cloned())
                })
                .or_else(|| {
                    crate::read_historical_header(&state, session_id, binding.workspace.as_deref())
                })
        });
        let stale_reasons = contract
            .as_ref()
            .map(|header| {
                let mut reasons = Vec::new();
                if header.cwd != *state.core.cwd() {
                    reasons.push("workspace_changed");
                }
                if header.contract.provider != provider {
                    reasons.push("provider_changed");
                }
                if header.contract.model != model {
                    reasons.push("model_changed");
                }
                reasons
            })
            .unwrap_or_else(|| {
                binding
                    .session_id
                    .as_ref()
                    .map(|_| vec!["session_missing"])
                    .unwrap_or_default()
            });
        bindings.push(serde_json::json!({
            "target": target,
            "session_id": binding.session_id,
            "workspace": state.core.cwd(),
            "configured_workspace": binding.workspace,
            "override": channel_override.map(|(provider, model)| serde_json::json!({
                "provider": provider,
                "model": model,
            })),
            "effective_route": {
                "provider": provider,
                "model": model,
                "source": source,
                "revision": revision,
            },
            "session_contract": contract.map(|header| serde_json::json!({
                "provider": header.contract.provider,
                "model": header.contract.model,
                "workspace": header.cwd,
                "app_version": header.contract.app_version,
            })),
            "stale": !stale_reasons.is_empty(),
            "stale_reasons": stale_reasons,
        }));
    }
    bindings.sort_by(|a, b| a["target"].as_str().cmp(&b["target"].as_str()));
    Json(serde_json::json!({
        "enabled": gw.enabled,
        "workspace": state.core.cwd(),
        "default_route": default_route,
        "bindings": bindings,
        "chat_allowlist": state.core.config().gateway.chat_allowlist,
        "chat_allowlist_open": state.core.config().gateway.chat_allowlist_open,
    }))
}

#[derive(serde::Deserialize)]
pub(crate) struct GatewayRoutePatch {
    provider: Option<String>,
    model: Option<String>,
}

pub(crate) async fn patch_gateway_binding(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<GatewayRoutePatch>,
) -> StatusCode {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST;
    }
    let route = match (body.provider, body.model) {
        (None, None) => None,
        (Some(provider), Some(model))
            if !provider.trim().is_empty() && !model.trim().is_empty() =>
        {
            Some((provider.trim().to_string(), model.trim().to_string()))
        }
        _ => return StatusCode::BAD_REQUEST,
    };
    if route.as_ref().is_some_and(|(provider, _)| {
        !state
            .core
            .provider_names()
            .iter()
            .any(|name| name == provider)
    }) {
        return StatusCode::BAD_REQUEST;
    }
    state
        .gateway
        .set_route_override(&state.core, key.clone(), route);
    state.hub.emit_config_changed("gateway_binding_route", &key);
    StatusCode::OK
}

pub(crate) async fn rotate_gateway_binding(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> StatusCode {
    if state.gateway.rotate(&state.core, &key) {
        state
            .hub
            .emit_config_changed("gateway_binding_rotated", &key);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

pub(crate) async fn delete_gateway_binding_admin(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> StatusCode {
    if state.gateway.unbind(&state.core, &key) {
        state
            .hub
            .emit_config_changed("gateway_binding_deleted", &key);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- Allowlist (docs/design/34-channel-onboarding.md) ---------------------

fn allowlist_entry_json(e: &crate::gateway::AllowlistEntry) -> serde_json::Value {
    serde_json::json!({
        "key": e.key,
        "status": e.status,
        "workspace": e.workspace,
        "route": e.route,
        "added_at": e.added_at,
        "added_by": e.added_by,
        "first_seen_text": e.first_seen_text,
    })
}

pub(crate) async fn list_gateway_allowlist(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let entries: Vec<serde_json::Value> = state
        .gateway
        .allowlist_snapshot()
        .iter()
        .map(allowlist_entry_json)
        .collect();
    Json(serde_json::json!({ "entries": entries }))
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct AllowlistApproveBody {
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    route: Option<GatewayRoutePatch>,
}

pub(crate) async fn approve_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let body: AllowlistApproveBody = if body.is_empty() {
        AllowlistApproveBody::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    };
    let workspace = match body.workspace {
        Some(w) if !w.trim().is_empty() => PathBuf::from(w.trim()),
        _ => state.core.cwd().clone(),
    };
    let route = match body.route {
        Some(GatewayRoutePatch {
            provider: Some(provider),
            model: Some(model),
        }) if !provider.trim().is_empty() && !model.trim().is_empty() => {
            Some(crate::gateway::AllowlistRoute {
                provider: provider.trim().to_string(),
                model: model.trim().to_string(),
            })
        }
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
        None => None,
    };
    let entry = state
        .gateway
        .allowlist_approve(&state.core, &key, workspace, route, "admin");
    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::ChatApproved,
        "chat_approved",
        &format!("key={key}"),
        None,
    );
    state
        .hub
        .emit_config_changed("gateway_allowlist_approved", &key);
    (StatusCode::OK, Json(allowlist_entry_json(&entry))).into_response()
}

pub(crate) async fn deny_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Response {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let entry = state.gateway.allowlist_deny(&state.core, &key, "admin");
    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::ChatDenied,
        "chat_denied",
        &format!("key={key}"),
        None,
    );
    state
        .hub
        .emit_config_changed("gateway_allowlist_denied", &key);
    (StatusCode::OK, Json(allowlist_entry_json(&entry))).into_response()
}

pub(crate) async fn revoke_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> StatusCode {
    if state.gateway.allowlist_revoke(&state.core, &key) {
        vak_core::security_events::record(
            &state.core.sessions_home(),
            vak_core::security_events::EventKind::ChatRevoked,
            "chat_revoked",
            &format!("key={key}"),
            None,
        );
        state
            .hub
            .emit_config_changed("gateway_allowlist_revoked", &key);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- Admin route mounter --------------------------------------------------

pub(crate) fn routes() -> axum::Router<AppState> {
    use axum::routing::{get, patch, post};
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
        .route(
            "/admin/api/gateway/bindings/{key}",
            patch(patch_gateway_binding).delete(delete_gateway_binding_admin),
        )
        .route(
            "/admin/api/gateway/bindings/{key}/rotate",
            post(rotate_gateway_binding),
        )
        .route("/admin/api/gateway/allowlist", get(list_gateway_allowlist))
        .route(
            "/admin/api/gateway/allowlist/{key}/approve",
            post(approve_gateway_allowlist),
        )
        .route(
            "/admin/api/gateway/allowlist/{key}/deny",
            post(deny_gateway_allowlist),
        )
        .route(
            "/admin/api/gateway/allowlist/{key}",
            axum::routing::delete(revoke_gateway_allowlist),
        )
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
        // Without this, `sessions_home()` falls back to the developer's
        // real $XDG_DATA_HOME/vak — any test that persists something
        // (gateway bindings, the allowlist store) would leak state across
        // test runs and across the machine. Every other test module in
        // this crate isolates it the same way.
        core.set_sessions_home(dir.path().join("home"));
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

    // ---- Allowlist admin routes (docs/design/34-channel-onboarding.md) ----

    #[tokio::test]
    async fn allowlist_list_returns_entries_array() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/allowlist")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        // Not asserting emptiness: a developer's own global config.toml may
        // legitimately seed `gateway.chat_allowlist` entries on load, and
        // that is not this test's concern — only that the route responds
        // with the documented shape.
        assert!(json["entries"].is_array());
    }

    #[tokio::test]
    async fn allowlist_list_requires_auth() {
        let state = test_state();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/allowlist")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn allowlist_approve_defaults_workspace_to_core_cwd_and_shows_it() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A42/approve")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["status"], "allowed");
        // The point of this endpoint: the workspace is explicit in the
        // response even when the caller didn't supply one.
        let ws = json["workspace"].as_str().expect("workspace must be shown");
        assert_eq!(std::path::Path::new(ws), state.core.cwd().as_path());
    }

    #[tokio::test]
    async fn allowlist_approve_with_explicit_workspace_and_route() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A43/approve")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"workspace":"/tmp/somewhere","route":{"provider":"anthropic","model":"sonnet"}}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["workspace"], "/tmp/somewhere");
        assert_eq!(json["route"]["provider"], "anthropic");
        assert_eq!(json["route"]["model"], "sonnet");
    }

    #[tokio::test]
    async fn allowlist_deny_then_revoke_of_denied_is_404() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A44/deny")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["status"], "denied");

        // Revoke only applies to *allowed* entries; a denied one 404s.
        let app2 = authed_app(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/admin/api/gateway/allowlist/telegram%3A44")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn allowlist_revoke_unknown_key_is_404() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/admin/api/gateway/allowlist/telegram%3Anever-seen")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn allowlist_approve_then_revoke_roundtrip() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A45/approve")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app2 = authed_app(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/admin/api/gateway/allowlist/telegram%3A45")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app3 = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/allowlist")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app3.oneshot(req).await.unwrap();
        let json = body_json(resp).await;
        let has_key = json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["key"] == "telegram:45");
        assert!(!has_key, "revoked entry must not remain in the list");
    }
}
