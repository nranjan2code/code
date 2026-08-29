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
    /// From the shared `archive.json` (keyed by session id, not scoped to a
    /// workspace — the same map `/sessions/{id}/archive` reads and writes).
    pub archived: bool,
}

pub(crate) async fn list_sessions_admin(
    State(state): State<AppState>,
    Query(q): Query<SessionListQuery>,
) -> Response {
    let Some(store) = &state.store else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "store not available" })),
        )
            .into_response();
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    // Both maps live under the shared vak home (`sessions_home`), not a
    // per-workspace directory, so they apply across every project this
    // store indexes — unlike archive/delete *mutation*, which only reaches
    // a ledger file under this process's own workspace (see
    // `workspace_project_hash` on `/admin/api/config`).
    let archive_map = crate::read_archive(&state.core);
    let deleted_map = crate::read_deleted(&state.core);
    match store.list_sessions() {
        Ok(all) => {
            let visible = all.into_iter().filter(|s| {
                q.project.as_ref().is_none_or(|p| &s.project_hash == p)
                    && !deleted_map.get(&s.session_id).copied().unwrap_or(false)
            });
            let visible: Vec<_> = visible.collect();
            let total = visible.len();
            let items: Vec<SessionListItem> = visible
                .into_iter()
                .take(limit)
                .map(|s| SessionListItem {
                    archived: archive_map.get(&s.session_id).copied().unwrap_or(false),
                    session_id: s.session_id,
                    project_hash: s.project_hash,
                    entry_count: s.entry_count,
                    first_ts: s.first_ts,
                    last_ts: s.last_ts,
                })
                .collect();
            Json(serde_json::json!({ "sessions": items, "total": total })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
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
    let cfg = state.core.config();
    Json(serde_json::json!({
        "provider": route.provider,
        "model": route.model,
        "provider_source": route.provider_source,
        "model_source": route.model_source,
        "route_revision": route.revision,
        "max_turns": state.core.effective_max_turns(),
        "permission_mode": format!("{:?}", state.core.effective_permission_mode()),
        "theme": state.core.effective_theme(),
        "memory": {
            "search_enabled": cfg.memory.search_enabled,
            "write_enabled": cfg.memory.write_enabled,
            "skill_proposals": cfg.memory.skill_proposals,
            "reflection": cfg.memory.reflection,
        },
        // The session list at `/admin/api/sessions` spans every project the
        // store indexes, but archive/delete/run/steer on a session only
        // reach a ledger file under *this* process's own workspace
        // (`sessions_home/sessions/<hash(cwd)>/`). This is that same hash,
        // matching `project_hash` on each session row — the console uses it
        // to tell which rows those actions can actually reach.
        "workspace_project_hash": vak_core::memory::hash_cwd(state.core.cwd()),
        // The resolved rule lists the permission engine actually evaluates
        // (vak_permission::Rule syntax: `Tool`, `Tool(glob)`, with a
        // `+`/`?`/`-` prefix for allow/ask/deny). The admin console shows
        // these verbatim and derives per-extension scope from them, so a
        // reader can see what an MCP server or a hook is permitted to do
        // rather than only that it is configured.
        "permissions": {
            "allow": cfg.allow,
            "ask": cfg.ask,
            "deny": cfg.deny,
        },
    }))
}

// ---- GET /admin/api/gateway/status ----------------------------------------

pub(crate) async fn gateway_status_admin(State(state): State<AppState>) -> Json<serde_json::Value> {
    let gw = &state.gateway;
    crate::refresh_control_plane(&state);
    let default_route = state.core.effective_route();
    let mut bindings = Vec::new();
    let mut bound_targets = std::collections::HashSet::new();
    for (target, binding) in gw.bindings_snapshot() {
        bound_targets.insert(target.clone());
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
    // An approved channel does not acquire a runtime binding until its first
    // accepted message creates a session. Keep that approved-but-cold channel
    // visible in the same table so the Admin UI does not claim it disappeared.
    for entry in gw.allowlist_snapshot() {
        if entry.status != crate::gateway::AllowlistStatus::Allowed
            || bound_targets.contains(&entry.key)
        {
            continue;
        }
        let channel_override = entry
            .route
            .as_ref()
            .map(|route| (route.provider.clone(), route.model.clone()));
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
        let workspace = entry
            .workspace
            .clone()
            .unwrap_or_else(|| state.core.cwd().to_path_buf());
        bindings.push(serde_json::json!({
            "target": entry.key,
            "session_id": null,
            "workspace": workspace,
            "configured_workspace": entry.workspace,
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
            "session_contract": null,
            "stale": false,
            "stale_reasons": [],
        }));
    }
    bindings.sort_by(|a, b| a["target"].as_str().cmp(&b["target"].as_str()));
    // docs/design/34 Phase 2: which workspaces currently have a pooled Core
    // running (warm) vs. cold (will lazily start on next inbound message),
    // so the workspace picker in the approve flow isn't guessing.
    let now = std::time::Instant::now();
    let core_pool: Vec<serde_json::Value> = gw
        .core_pool
        .snapshot_at(now)
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
        .collect();
    Json(serde_json::json!({
        "enabled": gw.enabled,
        "workspace": state.core.cwd(),
        "default_route": default_route,
        "bindings": bindings,
        "chat_allowlist": state.core.config().gateway.chat_allowlist,
        "chat_allowlist_open": state.core.config().gateway.chat_allowlist_open,
        "core_pool": {
            "max": state.core.config().gateway.core_pool_max,
            "idle_secs": state.core.config().gateway.core_pool_idle_secs,
            "entries": core_pool,
        },
        // docs/design/34 open question 4: the workspace field is a picker
        // of workspaces vak has actually run in, not unconstrained free
        // text — a typo'd path is caught at entry time by offering
        // known-good options first.
        "known_workspaces": known_workspaces(&state),
    }))
}

/// Workspaces vak has session ledgers for, newest-first, plus the
/// gateway's own cwd and any currently pooled workspace.
///
/// Sessions are stored per-cwd (`<home>/sessions/<hash>/<id>.jsonl`) and
/// the real path lives in each ledger's header, so this reads only the
/// first line of one file per project directory — no store rebuild, no
/// `Core` start.
fn known_workspaces(state: &AppState) -> Vec<String> {
    use std::io::BufRead;
    let mut seen: Vec<String> = vec![state.core.cwd().display().to_string()];
    let mut push = |path: String| {
        if !path.is_empty() && !seen.contains(&path) && !is_scratch_workspace(&path) {
            seen.push(path);
        }
    };
    for entry in state
        .gateway
        .core_pool
        .snapshot_at(std::time::Instant::now())
    {
        push(entry.workspace.display().to_string());
    }
    let root = state.core.sessions_home().join("sessions");
    let Ok(projects) = std::fs::read_dir(&root) else {
        return seen;
    };
    for project in projects.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else {
            continue;
        };
        // Any ledger in the directory names the same cwd, so the first
        // readable header is enough.
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(handle) = std::fs::File::open(&path) else {
                continue;
            };
            let mut first = String::new();
            if std::io::BufReader::new(handle)
                .read_line(&mut first)
                .is_err()
            {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&first)
                && let Some(cwd) = v["cwd"].as_str()
            {
                push(cwd.to_string());
                break;
            }
        }
    }
    seen
}

/// True for a path that's almost certainly test/build scratch rather than
/// a real project — OS temp dirs and the `tempfile` crate's `.tmpXXXXXX`
/// directory naming convention (used throughout this workspace's own test
/// suite, which is exactly what was polluting `known_workspaces` with
/// dozens of one-shot `cargo test` tempdirs on a dev machine). A path
/// under a real project that happens to be named `tmp` is not excluded by
/// this — only OS scratch roots and the tempfile-style random suffix are.
fn is_scratch_workspace(path: &str) -> bool {
    if path == "/tmp"
        || path.starts_with("/tmp/")
        || path == "/private/tmp"
        || path.starts_with("/private/tmp/")
        || path.starts_with("/var/folders/")
        || path.starts_with("/private/var/folders/")
    {
        return true;
    }
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with(".tmp"))
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
    // One source of truth for "what does this channel route to"
    // (docs/design/34 "Editing an already-allowed entry"): when an
    // `allowed` allowlist entry exists for this key, the route lives there
    // and this pre-existing surface writes through to it instead of
    // parking a second, divergent value on the binding.
    let mirrored = state.gateway.allowlist_patch_route_if_allowed(
        &state.core,
        &key,
        route
            .clone()
            .map(|(provider, model)| crate::gateway::AllowlistRoute { provider, model }),
    );
    state
        .gateway
        .set_route_override(&state.core, key.clone(), route);
    if mirrored {
        state
            .hub
            .emit_config_changed("gateway_allowlist_patched", &key);
    }
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
    // Resolve the effective permission mode the same way "Effective route"
    // is surfaced: the console must show what the channel actually gets,
    // not just what was requested, so a capped override is visible rather
    // than mistaken for a live grant.
    let resolved = e
        .workspace
        .as_ref()
        .map(|w| crate::gateway::resolve_channel_permission(w, e.permission_mode));
    serde_json::json!({
        "key": e.key,
        "status": e.status,
        "workspace": e.workspace,
        "route": e.route,
        "permission_mode": e.permission_mode,
        "workspace_permission_mode": resolved.as_ref().map(|r| r.workspace_mode),
        "effective_permission_mode": resolved.as_ref().map(|r| r.effective),
        "permission_capped": resolved.as_ref().is_some_and(|r| r.was_capped()),
        "policy": e.policy,
        "bot_id": e.bot_id,
        "inherit_bot_policy": e.inherit_bot_policy,
        "added_at": e.added_at,
        "added_by": e.added_by,
        "first_seen_text": e.first_seen_text,
    })
}

/// Parse an optional `permission_mode` field from an approve/PATCH body.
/// `Ok(None)` = inherit (field absent, null, or empty string); `Err(())` =
/// present but unparseable, which is a 400 rather than a silent inherit —
/// a typo'd mode must never quietly widen or narrow a channel's access.
fn parse_permission_mode_field(
    raw: Option<&str>,
) -> Result<Option<vak_config::PermissionMode>, ()> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) => crate::parse_mode(s).map(Some).ok_or(()),
    }
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
    /// Optional per-channel permission mode, riding along in the same
    /// request as `route` rather than on an endpoint of its own.
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    policy: Option<vak_config::ChannelPolicy>,
    /// Bind this chat to a bot identity (multi-bot-per-channel). `null`/
    /// absent/`""` unbinds it — the chat then resolves purely against the
    /// workspace, same as before bots existed.
    #[serde(default)]
    bot_id: Option<String>,
    /// Whether to inherit the bound bot's policy/permission_mode/route as a
    /// tier below this chat's own. Defaults to `true`; ignored when no
    /// `bot_id` is set.
    #[serde(default = "crate::gateway::default_true")]
    inherit_bot_policy: bool,
}

/// Audit an override that the workspace's own boundary will cap down, at
/// the moment the operator sets it — so the reduction is visible in the
/// security log immediately, not only when the channel's `Core` is first
/// started at dispatch (where `CorePool` records the enforcement itself).
fn record_permission_cap(state: &AppState, key: &str, entry: &crate::gateway::AllowlistEntry) {
    let Some(workspace) = entry.workspace.as_ref() else {
        return;
    };
    let resolved = crate::gateway::resolve_channel_permission(workspace, entry.permission_mode);
    if !resolved.was_capped() {
        return;
    }
    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::PermissionCapped,
        "permission_capped",
        &format!(
            "key={key} workspace={} requested={} capped_to={}",
            workspace.display(),
            resolved
                .requested
                .map(|m| m.as_str())
                .unwrap_or("(inherit)"),
            resolved.effective.as_str()
        ),
        None,
    );
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
    let permission_mode = match parse_permission_mode_field(body.permission_mode.as_deref()) {
        Ok(m) => m,
        Err(()) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let bot_id = body
        .bot_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let entry = state.gateway.allowlist_approve(
        &state.core,
        &key,
        workspace,
        route,
        permission_mode,
        body.policy.unwrap_or_default(),
        bot_id,
        body.inherit_bot_policy,
        "admin",
    );
    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::ChatApproved,
        "chat_approved",
        &format!("key={key}"),
        None,
    );
    record_permission_cap(&state, &key, &entry);
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

#[derive(serde::Deserialize, Default)]
pub(crate) struct AllowlistPatchBody {
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    route: Option<GatewayRoutePatch>,
    /// Absent / null / `""` clears the pin (inherit the workspace default),
    /// mirroring how an empty `route` object clears a pinned route.
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    policy: Option<vak_config::ChannelPolicy>,
    /// See `AllowlistApproveBody::bot_id`. `None` here (field simply
    /// absent) means "leave whatever bot binding is already set" — unlike
    /// approve, patch is an edit-in-place and must not silently unbind a
    /// chat just because a caller's PATCH body didn't mention bots at all.
    /// Send an explicit `null` to unbind.
    #[serde(default)]
    bot_id: Option<Option<String>>,
    #[serde(default)]
    inherit_bot_policy: Option<bool>,
}

/// `PATCH /admin/api/gateway/allowlist/{key}` (docs/design/34 "Editing an
/// already-allowed entry"): re-point an `allowed` channel's workspace
/// and/or pinned route without revoking and re-approving it, which would
/// lose `added_at`/`added_by` provenance and momentarily 403 the channel.
///
/// Only `allowed` entries are editable — pending/denied ones move through
/// approve/deny, not here (404 otherwise, same as an unknown key).
///
/// The edit is *not* a second way to change a channel's effective route:
/// the entry is the one source of truth `binding_route` already reads, and
/// the binding's cached route revision is invalidated here so the change
/// takes effect through the exact stale-session-rotation path
/// `PATCH .../bindings/{key}` uses — the next inbound message rotates to a
/// fresh frozen session, preserving the old append-only ledger.
pub(crate) async fn patch_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let body: AllowlistPatchBody = if body.is_empty() {
        AllowlistPatchBody::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    };
    let workspace = body
        .workspace
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(PathBuf::from);
    let route = match body.route {
        Some(GatewayRoutePatch {
            provider: Some(provider),
            model: Some(model),
        }) if !provider.trim().is_empty() && !model.trim().is_empty() => {
            if !state
                .core
                .provider_names()
                .iter()
                .any(|name| name == provider.trim())
            {
                return StatusCode::BAD_REQUEST.into_response();
            }
            Some(crate::gateway::AllowlistRoute {
                provider: provider.trim().to_string(),
                model: model.trim().to_string(),
            })
        }
        // An explicitly empty route object clears the pin (inherit the
        // workspace default) — the same shape `PATCH .../bindings` uses.
        Some(GatewayRoutePatch {
            provider: None,
            model: None,
        })
        | None => None,
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let permission_mode = match parse_permission_mode_field(body.permission_mode.as_deref()) {
        Ok(m) => m,
        Err(()) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let existing = state.gateway.allowlist_get(&key);
    let bot_id = body.bot_id.map(|inner| {
        inner
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    let Some(entry) = state.gateway.allowlist_patch(
        &state.core,
        &key,
        workspace,
        route,
        permission_mode,
        body.policy
            .or_else(|| existing.as_ref().map(|e| e.policy.clone()))
            .unwrap_or_default(),
        bot_id,
        body.inherit_bot_policy,
    ) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record_permission_cap(&state, &key, &entry);
    // Same stale-detection seam as the binding route editor: drop the
    // cached revision (never the ledger) so the next message re-derives
    // the effective route and rotates only if it really changed.
    state.gateway.invalidate_binding_revision(&state.core, &key);
    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::ConfigChange,
        "chat_edited",
        &format!(
            "key={key} workspace={} permission_mode={}",
            entry
                .workspace
                .as_ref()
                .map(|w| w.display().to_string())
                .unwrap_or_else(|| "(inherit)".into()),
            entry
                .permission_mode
                .map(|m| m.as_str())
                .unwrap_or("(inherit)")
        ),
        None,
    );
    state
        .hub
        .emit_config_changed("gateway_allowlist_patched", &key);
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
            patch(patch_gateway_allowlist).delete(revoke_gateway_allowlist),
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

    // ---- PATCH .../allowlist/{key} (docs/design/34 follow-up) ----------

    async fn approve(app: &axum::Router, token: &str, key: &str, body: &str) {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/admin/api/gateway/allowlist/{key}/approve"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Approve and return the entry JSON, for the permission-mode tests.
    async fn approve_json(
        app: &axum::Router,
        token: &str,
        key: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/admin/api/gateway/allowlist/{key}/approve"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        if status != StatusCode::OK {
            return (status, serde_json::Value::Null);
        }
        (status, body_json(resp).await)
    }

    /// A workspace directory whose own config fixes `mode` — the ceiling
    /// a channel override is capped against.
    fn workspace_dir_with_mode(mode: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let vak = dir.path().join(".vak");
        std::fs::create_dir_all(&vak).unwrap();
        std::fs::write(
            vak.join("config.toml"),
            format!("permission_mode = \"{mode}\"\n"),
        )
        .unwrap();
        dir
    }

    #[tokio::test]
    async fn approve_without_permission_mode_inherits_the_workspace() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("read-only");
        let (status, json) = approve_json(
            &app,
            &token,
            "telegram%3A60",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // No pin stored, and the effective mode is simply the workspace's.
        assert!(json["permission_mode"].is_null());
        assert_eq!(json["effective_permission_mode"], "read-only");
        assert_eq!(json["permission_capped"], false);
    }

    #[tokio::test]
    async fn approve_with_permission_mode_at_or_below_workspace_is_kept() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("full-access");
        let (status, json) = approve_json(
            &app,
            &token,
            "telegram%3A61",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "read-only" })
                .to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["permission_mode"], "read-only");
        assert_eq!(json["workspace_permission_mode"], "full-access");
        assert_eq!(json["effective_permission_mode"], "read-only");
        assert_eq!(json["permission_capped"], false);
    }

    /// The security-relevant admin path: an operator asking for more than
    /// the workspace itself allows gets the workspace's mode, not theirs,
    /// and the reduction lands in the security log.
    #[tokio::test]
    async fn approve_with_over_broad_permission_mode_is_capped_and_audited() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("read-only");
        let (status, json) = approve_json(
            &app,
            &token,
            "telegram%3A62",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "full-access" })
                .to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // The request is recorded verbatim...
        assert_eq!(json["permission_mode"], "full-access");
        // ...but what the channel actually gets is the workspace's ceiling.
        assert_eq!(json["effective_permission_mode"], "read-only");
        assert_eq!(json["permission_capped"], true);

        let events = vak_core::security_events::list(&state.core.sessions_home(), 50);
        assert!(
            events.iter().any(
                |e| e.kind == vak_core::security_events::EventKind::PermissionCapped
                    && e.detail.contains("requested=full-access")
                    && e.detail.contains("capped_to=read-only")
            ),
            "a capped grant must be visible in the audit log, got {events:?}"
        );
    }

    #[tokio::test]
    async fn approve_rejects_an_unparseable_permission_mode() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let (status, _) = approve_json(
            &app,
            &token,
            "telegram%3A63",
            r#"{"permission_mode":"god-mode"}"#,
        )
        .await;
        // A typo must be a hard 400, never a silent inherit.
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_sets_and_clears_a_permission_pin() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("full-access");
        approve(
            &app,
            &token,
            "telegram%3A64",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;

        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A64",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "workspace-write" })
                .to_string(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["permission_mode"], "workspace-write");
        assert_eq!(json["effective_permission_mode"], "workspace-write");

        // An omitted field clears the pin back to inherit, the same way an
        // empty route object clears a pinned route.
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A64",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;
        let json = body_json(resp).await;
        assert!(json["permission_mode"].is_null());
        assert_eq!(json["effective_permission_mode"], "full-access");
    }

    #[tokio::test]
    async fn patch_with_over_broad_permission_mode_is_capped() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("workspace-write");
        approve(
            &app,
            &token,
            "telegram%3A65",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A65",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "full-access" })
                .to_string(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["effective_permission_mode"], "workspace-write");
        assert_eq!(json["permission_capped"], true);
    }

    /// Editing a route from the *binding* surface must not silently drop a
    /// channel's permission pin — that surface never mentions permissions.
    #[tokio::test]
    async fn binding_route_write_through_preserves_the_permission_pin() {
        let state = test_state();
        let ws = workspace_dir_with_mode("full-access");
        state.gateway.allowlist_approve(
            &state.core,
            "telegram:66",
            ws.path().to_path_buf(),
            None,
            Some(vak_config::PermissionMode::ReadOnly),
            vak_config::ChannelPolicy::default(),
            None,
            true,
            "admin",
        );
        state.gateway.allowlist_patch_route_if_allowed(
            &state.core,
            "telegram:66",
            Some(crate::gateway::AllowlistRoute {
                provider: "anthropic".into(),
                model: "sonnet".into(),
            }),
        );
        let entry = state.gateway.allowlist_get("telegram:66").unwrap();
        assert_eq!(
            entry.permission_mode,
            Some(vak_config::PermissionMode::ReadOnly)
        );
        assert_eq!(entry.route.unwrap().model, "sonnet");
    }

    async fn patch_allowlist(
        app: &axum::Router,
        token: &str,
        key: &str,
        body: &str,
    ) -> axum::response::Response {
        let req = Request::builder()
            .method("PATCH")
            .uri(format!("/admin/api/gateway/allowlist/{key}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn allowlist_patch_edits_workspace_in_place_keeping_provenance() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(&app, &token, "telegram%3A50", r#"{"workspace":"/tmp/one"}"#).await;
        let before = state.gateway.allowlist_get("telegram:50").unwrap();

        let resp =
            patch_allowlist(&app, &token, "telegram%3A50", r#"{"workspace":"/tmp/two"}"#).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["workspace"], "/tmp/two");
        // A re-point is not a re-approval: added_at/added_by must survive.
        assert_eq!(json["added_at"], before.added_at);
        assert_eq!(json["added_by"], before.added_by);
        assert_eq!(json["status"], "allowed");
    }

    #[tokio::test]
    async fn allowlist_patch_clears_a_pinned_route_with_an_empty_route_object() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(
            &app,
            &token,
            "telegram%3A51",
            r#"{"workspace":"/tmp/one","route":{"provider":"anthropic","model":"sonnet"}}"#,
        )
        .await;
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A51",
            r#"{"workspace":"/tmp/one","route":{}}"#,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            state
                .gateway
                .allowlist_get("telegram:51")
                .unwrap()
                .route
                .is_none()
        );
    }

    #[tokio::test]
    async fn allowlist_patch_rotates_through_the_existing_stale_detection_path() {
        // docs/design/34: an edited workspace/route must take effect the
        // way a stale binding already does — by dropping the cached route
        // revision so the next inbound message re-derives it, never by
        // mutating state the binding/session layer doesn't know about.
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(&app, &token, "telegram%3A52", r#"{"workspace":"/tmp/one"}"#).await;
        state.gateway.set_route_override(
            &state.core,
            "telegram:52".into(),
            Some(("anthropic".into(), "sonnet".into())),
        );
        // A bound session with a frozen revision, as dispatch would leave it.
        state
            .gateway
            .bind_for_test("telegram:52", "sess-1", "channel:anthropic:sonnet");
        assert!(
            state
                .gateway
                .route_revision_for_test("telegram:52")
                .is_some()
        );

        let resp =
            patch_allowlist(&app, &token, "telegram%3A52", r#"{"workspace":"/tmp/two"}"#).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            state
                .gateway
                .route_revision_for_test("telegram:52")
                .is_none()
        );
        // The ledger binding itself is preserved — rotation replaces the
        // session on the next message, it never deletes history.
        assert_eq!(
            state.gateway.session_id_for_test("telegram:52").as_deref(),
            Some("sess-1")
        );
    }

    #[tokio::test]
    async fn allowlist_patch_is_404_for_unknown_and_non_allowed_keys() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3Anever-seen",
            r#"{"workspace":"/tmp"}"#,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // Denied entries move through approve/deny, never through edit.
        state
            .gateway
            .allowlist_deny(&state.core, "telegram:53", "admin");
        let resp = patch_allowlist(&app, &token, "telegram%3A53", r#"{"workspace":"/tmp"}"#).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn allowlist_patch_rejects_a_key_that_is_not_surface_chat() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let resp = patch_allowlist(&app, &token, "nocolon", r#"{"workspace":"/tmp"}"#).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn binding_route_patch_writes_through_to_the_allowed_entry() {
        // One source of truth: changing the route from the pre-existing
        // binding editor must not leave the allowlist entry holding a
        // different, stale answer to "what does this channel route to".
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(
            &app,
            &token,
            "telegram%3A54",
            r#"{"workspace":"/tmp/one","route":{"provider":"anthropic","model":"sonnet"}}"#,
        )
        .await;
        let req = Request::builder()
            .method("PATCH")
            .uri("/admin/api/gateway/bindings/telegram%3A54")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let entry = state.gateway.allowlist_get("telegram:54").unwrap();
        assert!(
            entry.route.is_none(),
            "entry kept a route the binding cleared"
        );
        // The workspace is untouched by a route-only edit.
        assert_eq!(entry.workspace.unwrap().to_string_lossy(), "/tmp/one");
    }

    #[test]
    fn scratch_workspace_detection() {
        assert!(super::is_scratch_workspace("/tmp/foo"));
        assert!(super::is_scratch_workspace("/private/tmp/foo"));
        assert!(super::is_scratch_workspace(
            "/var/folders/0g/xyz/T/.tmpAbC123"
        ));
        assert!(super::is_scratch_workspace(
            "/private/var/folders/0g/xyz/T/.tmpAbC123"
        ));
        assert!(super::is_scratch_workspace("/Users/x/anywhere/.tmpZZZZZZ"));
        assert!(!super::is_scratch_workspace("/Users/x/Projects/vakcoder"));
        assert!(!super::is_scratch_workspace("/Users/x/Projects/tmp-tool"));
    }

    #[tokio::test]
    async fn gateway_status_lists_known_workspaces_including_the_gateway_cwd() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/status")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let json = body_json(app.oneshot(req).await.unwrap()).await;
        let known = json["known_workspaces"].as_array().unwrap();
        assert!(
            known
                .iter()
                .any(|w| w.as_str() == Some(&state.core.cwd().display().to_string())),
            "the gateway's own workspace must always be offered"
        );
    }

    #[tokio::test]
    async fn gateway_status_lists_allowed_channel_before_first_message() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(&app, &token, "telegram%3A67", r#"{}"#).await;
        let req = Request::builder()
            .uri("/admin/api/gateway/status")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let json = body_json(app.oneshot(req).await.unwrap()).await;
        let binding = json["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|binding| binding["target"] == "telegram:67")
            .expect("approved cold channel must be visible");
        assert!(binding["session_id"].is_null());
        assert_eq!(binding["stale"], false);
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
