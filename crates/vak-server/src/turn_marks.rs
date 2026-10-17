//! `PUT /sessions/{id}/turns/{turn}/aside`: a person sets a turn aside, or
//! restores it; `POST /sessions/{id}/turns/{turn}/retry`: a person asks for
//! a turn again, on a new branch from the turn before it
//! (docs/design/99-turns-that-do-not-poison.md §4 F). Each is an entry
//! naming the turn; nothing is removed, and no later turn sees a turn set
//! aside or retried. Only the owner does this: a guest is refused here,
//! behind `require_bearer` and `enforce_participant_audience`.

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

#[derive(serde::Deserialize)]
pub(crate) struct RetryBody {
    /// Whether the caller draws a worker's questions, as on `/run`
    /// (`RunBody::can_show_questions`).
    #[serde(default)]
    can_show_questions: bool,
}

/// Starts the turn again: the session must be idle and the turn ended. The
/// branch starts with `TurnRetried`, then the turn's directive, as the
/// person sent it, is sent again as a new turn and answered like any other.
pub(crate) async fn retry(
    State(state): State<AppState>,
    Path((id, turn)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Json(body): Json<RetryBody>,
) -> Response {
    if !matches!(principal, AuthenticatedPrincipal::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !state.core.sync_may_work() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": vak_core::CoreError::StandingBy.to_string(),
                "reason": "standing_by",
            })),
        )
            .into_response();
    }
    let handle = match crate::ensure_session_handle(&state, &id).await {
        Ok((_, handle)) => handle,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let taken = {
        let mut slot = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(session) = slot.as_ref() else {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Agent is working; try again after this turn settles"})),
            )
                .into_response();
        };
        let index = vak_session::TurnIndex::from_log(session);
        match index.turn_by_id(&turn) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(found) if !found.closed => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": "This turn is still going; it can be tried again once it ends"})),
                )
                    .into_response();
            }
            Some(_) => {}
        }
        if let Err(error) = handle.core.provider() {
            return crate::provider_unavailable(error);
        }
        match slot.take() {
            Some(taken) => taken,
            None => return StatusCode::CONFLICT.into_response(),
        }
    };
    let mut taken = match crate::writable(&handle, taken) {
        Ok(taken) => taken,
        Err(refused) => return *refused,
    };
    let directive = match taken.retry_turn(&turn, Some(vak_session::trace::local::local_owner())) {
        Ok(directive) => directive,
        Err(_) => {
            *handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(taken);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let prompt = directive.message.text_content();
    crate::start_http_turn(
        &state,
        handle,
        taken,
        crate::TurnStart::Message(Box::new(directive)),
        &prompt,
        None,
        body.can_show_questions,
    )
    .await
}
