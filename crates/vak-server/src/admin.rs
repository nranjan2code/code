//! Admin console API endpoints (docs/design/29-personal-os.md): session
//! catalog, transcripts, live event SSE, security audit log, and store
//! management. Mounted under `/admin/api` in `router_with_state`.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::AppState;

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
    let filter = vak_store::query::SearchFilter {
        project_hash: q.project,
        ..Default::default()
    };
    match store.query(&filter, limit) {
        Ok(entries) => {
            let mut map: std::collections::BTreeMap<String, SessionAgg> =
                std::collections::BTreeMap::new();
            for e in &entries {
                let agg = map
                    .entry(e.session_id.clone())
                    .or_insert_with(|| SessionAgg {
                        session_id: e.session_id.clone(),
                        project_hash: e.project_hash.clone(),
                        count: 0,
                        first_ts: e.ts.clone(),
                        last_ts: e.ts.clone(),
                    });
                agg.count += 1;
                if e.ts < agg.first_ts {
                    agg.first_ts = e.ts.clone();
                }
                if e.ts > agg.last_ts {
                    agg.last_ts = e.ts.clone();
                }
            }
            let items: Vec<SessionListItem> = map
                .into_values()
                .map(|a| SessionListItem {
                    session_id: a.session_id,
                    project_hash: a.project_hash,
                    entry_count: a.count,
                    first_ts: a.first_ts,
                    last_ts: a.last_ts,
                })
                .collect();
            Json(serde_json::json!({ "sessions": items }))
        }
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

#[derive(Debug)]
struct SessionAgg {
    session_id: String,
    project_hash: String,
    count: usize,
    first_ts: String,
    last_ts: String,
}

// ---- GET /admin/api/sessions/:id/transcript --------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptQuery {
    pub limit: Option<usize>,
    pub kind: Option<String>,
    pub role: Option<String>,
}

pub(crate) async fn session_transcript_admin(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return Json(serde_json::json!({ "error": "store not available" }));
    };
    let limit = q.limit.unwrap_or(500).clamp(1, 2000);
    let filter = vak_store::query::SearchFilter {
        session_id: Some(session_id.clone()),
        kind: q.kind,
        role: q.role,
        ..Default::default()
    };
    match store.query(&filter, limit) {
        Ok(entries) => {
            let items: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "entry_id": e.entry_id,
                        "ts": e.ts,
                        "kind": e.kind.as_str(),
                        "role": e.role,
                        "tool_name": e.tool_name,
                        "is_error": e.is_error,
                        "content": if e.content_text.len() > 4000 {
                            format!("{}…", &e.content_text[..4000])
                        } else {
                            e.content_text.clone()
                        },
                    })
                })
                .collect();
            Json(serde_json::json!({
                "session_id": session_id,
                "entries": items,
                "total": items.len(),
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

// ---- GET /admin/api/config ------------------------------------------------

pub(crate) async fn get_config_admin(State(state): State<AppState>) -> Json<serde_json::Value> {
    let cfg = state.core.config();
    Json(serde_json::json!({
        "provider": cfg.provider,
        "model": cfg.model,
        "max_turns": cfg.max_turns,
        "permission_mode": format!("{:?}", cfg.permission_mode),
        "theme": cfg.ui.theme,
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
    use axum::routing::get;
    axum::Router::new()
        .route("/admin/api/sessions", get(list_sessions_admin))
        .route(
            "/admin/api/sessions/{id}/transcript",
            get(session_transcript_admin),
        )
        .route("/admin/api/search", get(search_admin))
        .route("/admin/api/events", get(admin_events_sse))
        .route("/admin/api/security", get(list_security_events))
        .route("/admin/api/store/rebuild", get(rebuild_store))
        .route(
            "/admin/api/store/import/{session_id}",
            get(import_session_store),
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

    #[tokio::test]
    async fn list_sessions_returns_ok() {
        let state = test_state();
        let app = crate::router_with_state(state);
        let req = Request::builder()
            .uri("/admin/api/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["sessions"].is_array());
    }

    #[tokio::test]
    async fn search_returns_ok() {
        let state = test_state();
        let app = crate::router_with_state(state);
        let req = Request::builder()
            .uri("/admin/api/search?q=hello")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("total").is_some(), "response should have total field");
    }

    #[tokio::test]
    async fn security_events_returns_ok() {
        let state = test_state();
        let app = crate::router_with_state(state);
        let req = Request::builder()
            .uri("/admin/api/security")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn store_rebuild_returns_ok() {
        let state = test_state();
        let app = crate::router_with_state(state);
        let req = Request::builder()
            .uri("/admin/api/store/rebuild")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["ok"].as_bool().unwrap_or(false));
    }

    #[tokio::test]
    async fn config_endpoint_returns_ok() {
        let state = test_state();
        let app = crate::router_with_state(state);
        let req = Request::builder()
            .uri("/admin/api/config")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["provider"].is_string());
    }
}
