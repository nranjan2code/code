//! The data catalog over HTTP (plan M6): what a thing is, what caused it,
//! and the catalog's own state. Search is `/search` in `lib.rs`. Every
//! answer leaves out what the trash hides.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::AppState;

fn failed(error: impl std::fmt::Display) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": error.to_string() })),
    )
        .into_response()
}

/// Whether `node` lies in a session the trash hides.
fn trashed(state: &AppState, node: Option<&vak_catalog::Node>) -> bool {
    node.and_then(|node| node.session.as_deref())
        .is_some_and(|session| {
            vak_core::trash::is_trashed(
                &state.core.shared_scope(),
                session.trim_start_matches("ses_"),
            )
        })
}

/// `GET /nodes/{id}`: one thing the catalog knows.
pub(crate) async fn node(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let core = state.core.clone();
    let found = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        catalog.catch_up()?;
        Ok::<_, vak_core::CoreError>(catalog.open_node(&id)?)
    })
    .await;
    match found {
        Ok(Ok(Some(node))) if !trashed(&state, Some(&node)) => Json(node).into_response(),
        Ok(Ok(_)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

/// `GET /lineage/{id}`: what caused a thing, from it up to its run and
/// cause.
pub(crate) async fn lineage(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let core = state.core.clone();
    let found = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        catalog.catch_up()?;
        Ok::<_, vak_core::CoreError>(catalog.lineage(&id)?)
    })
    .await;
    match found {
        Ok(Ok(Some(lineage))) if !lineage.path.iter().any(|node| trashed(&state, Some(node))) => {
            Json(lineage).into_response()
        }
        Ok(Ok(_)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

/// `GET /catalog/sessions/{id}/nodes`: what a session holds, oldest first,
/// for the Lineage tab to trace one of them.
pub(crate) async fn session_nodes(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if vak_core::trash::is_trashed(&state.core.shared_scope(), id.trim_start_matches("ses_")) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let core = state.core.clone();
    let found = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        catalog.catch_up()?;
        Ok::<_, vak_core::CoreError>(catalog.in_session(&id, 2_000)?)
    })
    .await;
    match found {
        Ok(Ok(nodes)) => Json(serde_json::json!({ "nodes": nodes })).into_response(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

/// `GET /catalog`: whether the catalog has taken everything, and how much
/// of each kind it holds.
pub(crate) async fn status(State(state): State<AppState>) -> Response {
    let core = state.core.clone();
    let found = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        Ok::<_, vak_core::CoreError>((catalog.stale()?, catalog.counts()?))
    })
    .await;
    match found {
        Ok(Ok((stale, counts))) => {
            Json(serde_json::json!({ "stale": stale, "counts": counts })).into_response()
        }
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}

/// `POST /catalog/rebuild`: drops every row and takes every record again.
pub(crate) async fn rebuild(State(state): State<AppState>) -> Response {
    let core = state.core.clone();
    let rebuilt = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        let taken = catalog.rebuild()?;
        Ok::<_, vak_core::CoreError>((taken, catalog.counts()?))
    })
    .await;
    match rebuilt {
        Ok(Ok((taken, counts))) => Json(serde_json::json!({
            "ok": true,
            "sources": taken.sources,
            "rows": taken.rows,
            "counts": counts,
        }))
        .into_response(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}
