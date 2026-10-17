//! `PUT /sessions/{id}/turns/{turn}/aside`: a person sets a turn aside, or
//! restores it (docs/design/99-turns-that-do-not-poison.md §4 F). Each is an
//! entry naming the turn; nothing is removed, and no later turn sees a turn
//! set aside. Only the owner does this: a guest is refused here, behind
//! `require_bearer` and `enforce_participant_audience`.

use crate::{AppState, AuthenticatedPrincipal};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use vak_session::SessionLog;
use vak_session::turn_marks::{TurnMarkRecord, set_aside_turns};

#[derive(serde::Deserialize)]
pub(crate) struct AsideBody {
    aside: bool,
}

enum Refused {
    NoTurn,
    Open,
    Write,
}

/// The turns set aside on the session's chain, oldest first.
pub(crate) fn set_aside_list(session: &SessionLog) -> Vec<String> {
    let aside = set_aside_turns(session.chain_to_root());
    vak_session::TurnIndex::from_log(session)
        .turns
        .into_iter()
        .filter(|turn| aside.contains(&turn.id))
        .map(|turn| turn.id)
        .collect()
}

/// Appends the mark unless the turn is already in that state.
fn mark(session: &mut SessionLog, turn_id: &str, aside: bool) -> Result<(), Refused> {
    let index = vak_session::TurnIndex::from_log(session);
    let turn = index.turn_by_id(turn_id).ok_or(Refused::NoTurn)?;
    if !turn.closed {
        return Err(Refused::Open);
    }
    if turn.set_aside == aside {
        return Ok(());
    }
    session
        .append_turn_mark(
            aside,
            TurnMarkRecord {
                turn: turn_id.to_string(),
                trace: None,
                actor: Some(vak_session::trace::local::local_owner()),
            },
        )
        .map(|_| ())
        .map_err(|_| Refused::Write)
}

pub(crate) async fn put(
    State(state): State<AppState>,
    Path((id, turn)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Json(body): Json<AsideBody>,
) -> Response {
    if !matches!(principal, AuthenticatedPrincipal::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let handle = match crate::ensure_session_handle(&state, &id).await {
        Ok((_, handle)) => handle,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let Ok(mut guard) = handle.session.lock() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Some(session) = guard.as_mut() else {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"error": "Agent is working; try again after this turn settles"}),
            ),
        )
            .into_response();
    };
    match mark(session, &turn, body.aside) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "set_aside": set_aside_list(session) })),
        )
            .into_response(),
        Err(Refused::NoTurn) => StatusCode::NOT_FOUND.into_response(),
        Err(Refused::Open) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "This turn is still going; it can be set aside once it ends"})),
        )
            .into_response(),
        Err(Refused::Write) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
