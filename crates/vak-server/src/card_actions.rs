//! `POST /sessions/{id}/card-actions`: what a person did on a card
//! (docs/design/96-answer-projection-gaps.md, Q3). A choice is recorded and
//! starts a turn whose directive names it; a tick is recorded and starts
//! none. Only the owner acts on cards: a guest is refused here, behind
//! `require_bearer` and `enforce_participant_audience`.

use crate::{AppState, AuthenticatedPrincipal, RunBody};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use vak_session::card_actions::{CardActionKind, CardActionRecord, CardActionValues};
use vak_session::{EntryPayload, SessionLog};

#[derive(serde::Deserialize)]
pub(crate) struct CardActionBody {
    presentation_id: String,
    action: CardActionKind,
    values: CardActionValues,
}

/// Why an action does not fit the card it names.
fn refusal(message: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// The card's title, when the action fits the card: a choice names one of a
/// comparison's options, a tick one item of a checkable list by its place
/// and label.
fn check(payload: &Value, block: &str, body: &CardActionBody) -> Result<String, &'static str> {
    let title = payload["title"].as_str().unwrap_or_default().to_string();
    match (&body.action, &body.values) {
        (CardActionKind::Choose, CardActionValues::Option { label }) => {
            let named = |list: &Value, key: &str| {
                list.as_array().is_some_and(|options| {
                    options
                        .iter()
                        .any(|option| option[key].as_str() == Some(label.as_str()))
                })
            };
            // A comparison's options, by name; a steps card's choices on
            // its steps, by label.
            let offered = match block {
                "comparison" => named(&payload["options"], "name"),
                "steps" => payload["steps"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().any(|step| named(&step["choices"], "label"))),
                _ => false,
            };
            if offered {
                Ok(title)
            } else {
                Err("this card offers no option by that name")
            }
        }
        (CardActionKind::Mark, CardActionValues::Item { index, label, .. }) => {
            let item = (block == "list" && payload["checkable"] == true)
                .then(|| payload["items"].get(*index))
                .flatten();
            if item.is_some_and(|item| item["label"].as_str() == Some(label.as_str())) {
                Ok(title)
            } else {
                Err("this card has no item to tick at that place")
            }
        }
        _ => Err("a choice names an option; a tick names an item"),
    }
}

pub(crate) async fn post(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Json(body): Json<CardActionBody>,
) -> Response {
    if !matches!(principal, AuthenticatedPrincipal::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let handle = match crate::ensure_session_handle(&state, &id).await {
        Ok((_, handle)) => handle,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let recorded = {
        let Ok(mut guard) = handle.session.lock() else {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        };
        let Some(session) = guard.as_mut() else {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "Agent is working; try again after this turn settles"})),
            )
                .into_response();
        };
        record(session, &body)
    };
    let (entry_id, title, state_now) = match recorded {
        Ok(recorded) => recorded,
        Err(Refused::NoCard) => return StatusCode::NOT_FOUND.into_response(),
        Err(Refused::Mismatch(message)) => return refusal(message),
        Err(Refused::Write) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    match (&body.action, &body.values) {
        (CardActionKind::Choose, CardActionValues::Option { label }) => {
            let directive = vak_session::card_actions::choice_text(&title, label);
            let Ok(mut run) = serde_json::from_value::<RunBody>(serde_json::json!({
                "prompt": directive,
            })) else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            run.card_action = Some(entry_id);
            let started = crate::run_prompt(State(state), Path(id), Json(run)).await;
            if !started.status().is_success() {
                return started;
            }
            // The directive the turn started with, so a client shows the
            // person's choice as their message.
            (
                started.status(),
                Json(serde_json::json!({ "directive": directive })),
            )
                .into_response()
        }
        _ => (
            StatusCode::OK,
            Json(serde_json::json!({ "state": state_now })),
        )
            .into_response(),
    }
}

enum Refused {
    NoCard,
    Mismatch(&'static str),
    Write,
}

/// Records the action on the card it names, returning the entry, the card's
/// title and the card's state after it.
fn record(
    session: &mut SessionLog,
    body: &CardActionBody,
) -> Result<(String, String, vak_session::card_actions::CardState), Refused> {
    let (payload, block) = session
        .chain_to_root()
        .into_iter()
        .find_map(|entry| match &entry.payload {
            EntryPayload::Presentation(record) if entry.id == body.presentation_id => Some((
                record.payload.clone(),
                record
                    .block
                    .clone()
                    .unwrap_or_else(|| record.semantic_type.clone()),
            )),
            _ => None,
        })
        .ok_or(Refused::NoCard)?;
    let title = check(&payload, &block, body).map_err(Refused::Mismatch)?;
    let entry = session
        .append_card_action(CardActionRecord {
            presentation_id: body.presentation_id.clone(),
            action: body.action,
            values: body.values.clone(),
            trace: None,
            actor: Some(vak_session::trace::local::local_owner()),
        })
        .map_err(|_| Refused::Write)?;
    let state_now = vak_session::card_actions::card_states(session.chain_to_root())
        .remove(&body.presentation_id)
        .unwrap_or_default();
    Ok((entry.id, title, state_now))
}
