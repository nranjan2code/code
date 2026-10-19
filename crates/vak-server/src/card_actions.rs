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
fn refusal(message: String) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// What a controls card's values come to: each setting as the card shows it
/// and each output the host computes from them (docs/design/96, P1).
fn controls_directive(
    payload: &Value,
    title: &str,
    values: &std::collections::BTreeMap<String, String>,
) -> Result<String, String> {
    let inputs = payload["inputs"].as_array().cloned().unwrap_or_default();
    let with_unit = |shown: String, item: &Value| match item["unit"].as_str() {
        Some(unit) if !unit.trim().is_empty() => format!("{shown} {}", unit.trim()),
        _ => shown,
    };
    let mut set = std::collections::BTreeMap::new();
    let mut said = Vec::new();
    if let Some(unknown) = values.keys().find(|name| {
        !inputs
            .iter()
            .any(|input| input["name"].as_str() == Some(name.as_str()))
    }) {
        return Err(format!("this card has no input named `{unknown}`"));
    }
    // In the card's own order.
    for input in &inputs {
        let name = input["name"].as_str().unwrap_or_default();
        let Some(written) = values.get(name) else {
            continue;
        };
        let label = input["label"].as_str().unwrap_or(name);
        let written = written.trim();
        let (value, shown) = match input["kind"].as_str().unwrap_or_default() {
            "toggle" => match written {
                "true" => (1.0, "on".to_string()),
                "false" => (0.0, "off".to_string()),
                _ => return Err(format!("`{label}` is on or off")),
            },
            "date" => (
                vak_delivery::controls::days(written)
                    .ok_or_else(|| format!("`{label}` is a date, YYYY-MM-DD"))?,
                written.to_string(),
            ),
            kind => {
                let number: f64 = written
                    .parse()
                    .ok()
                    .filter(|number: &f64| number.is_finite())
                    .ok_or_else(|| format!("`{label}` is a number"))?;
                let below = input["min"].as_f64().is_some_and(|min| number < min);
                let above = input["max"].as_f64().is_some_and(|max| number > max);
                if below || above {
                    return Err(format!("`{label}` is outside the card's range"));
                }
                let shown = if kind == "choice" {
                    input["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|option| option["value"].as_f64() == Some(number))
                        .and_then(|option| option["label"].as_str())
                        .map(str::to_string)
                        .ok_or_else(|| format!("`{label}` has no option with that value"))?
                } else {
                    written.to_string()
                };
                (number, shown)
            }
        };
        set.insert(name.to_string(), value);
        said.push(format!("{label} to {}", with_unit(shown, input)));
    }
    let computed = vak_delivery::controls::compute(payload, &set)?;
    let outputs = payload["outputs"].as_array().cloned().unwrap_or_default();
    let results = outputs
        .iter()
        .zip(computed)
        .map(|(output, (name, value))| {
            let decimals = output["decimals"].as_u64().unwrap_or(0);
            let label = output["label"].as_str().unwrap_or(&name).to_string();
            format!(
                "{label} {}",
                with_unit(vak_delivery::controls::shown(value, decimals), output)
            )
        })
        .collect::<Vec<_>>();
    Ok(vak_session::card_actions::values_text(
        title, &said, &results,
    ))
}

/// The card's title, when the action fits the card: a choice names one of a
/// comparison's options, a tick one item of a checkable list by its place
/// and label.
/// For values set on a controls card, the directive their turn starts with.
fn check(
    payload: &Value,
    block: &str,
    body: &CardActionBody,
) -> Result<(String, Option<String>), String> {
    let title = payload["title"].as_str().unwrap_or_default().to_string();
    match (&body.action, &body.values) {
        (CardActionKind::Choose, CardActionValues::Values { values }) if block == "controls" => {
            let directive = controls_directive(payload, &title, values)?;
            Ok((title, Some(directive)))
        }
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
                Ok((title, None))
            } else {
                Err("this card offers no option by that name".into())
            }
        }
        (CardActionKind::Mark, CardActionValues::Item { index, label, .. }) => {
            let item = (block == "list" && payload["checkable"] == true)
                .then(|| payload["items"].get(*index))
                .flatten();
            if item.is_some_and(|item| item["label"].as_str() == Some(label.as_str())) {
                Ok((title, None))
            } else {
                Err("this card has no item to tick at that place".into())
            }
        }
        _ => Err(
            "a choice names an option, a tick an item, and values a controls card's inputs".into(),
        ),
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
        let mut session = match crate::writing(session) {
            Ok(session) => session,
            Err(error) => {
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": crate::writer_refused(error)})),
                )
                    .into_response();
            }
        };
        record(&mut session, &body)
    };
    let recorded = match recorded {
        Ok(recorded) => recorded,
        Err(Refused::NoCard) => return StatusCode::NOT_FOUND.into_response(),
        Err(Refused::Mismatch(message)) => return refusal(message),
        Err(Refused::Write) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Some(directive) = recorded.directive else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "state": recorded.state_now })),
        )
            .into_response();
    };
    let Ok(mut run) = serde_json::from_value::<RunBody>(serde_json::json!({
        "prompt": directive,
    })) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    run.card_action = Some(recorded.entry_id);
    let started = crate::run_prompt(State(state), Path(id), Json(run)).await;
    if !started.status().is_success() {
        return started;
    }
    // The directive the turn started with, so a client shows the person's
    // choice as their message.
    (
        started.status(),
        Json(serde_json::json!({ "directive": directive })),
    )
        .into_response()
}

struct Recorded {
    entry_id: String,
    /// The directive of the turn the action starts, when it starts one.
    directive: Option<String>,
    state_now: vak_session::card_actions::CardState,
}

enum Refused {
    NoCard,
    Mismatch(String),
    Write,
}

/// Records the action on the card it names, returning the entry, the card's
/// title and the card's state after it.
fn record(session: &mut SessionLog, body: &CardActionBody) -> Result<Recorded, Refused> {
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
    let (title, directive) = check(&payload, &block, body).map_err(Refused::Mismatch)?;
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
    let directive = match &body.values {
        CardActionValues::Option { label } => {
            Some(vak_session::card_actions::choice_text(&title, label))
        }
        _ => directive,
    };
    Ok(Recorded {
        entry_id: entry.id,
        directive,
        state_now,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::controls_directive;
    use std::collections::BTreeMap;

    /// What a person set is checked against the card, and the outputs are
    /// the host's own computation, never the client's.
    #[test]
    fn values_set_on_a_controls_card_start_a_turn_the_host_computes() {
        let card = serde_json::json!({"title": "Split the bill", "inputs": [
            {"name": "bill", "label": "Bill", "kind": "number", "value": 1200, "min": 0, "unit": "₹"},
            {"name": "tip", "label": "Tip", "kind": "choice", "value": 15,
             "options": [{"label": "None", "value": 0}, {"label": "Usual", "value": 15}]},
            {"name": "people", "label": "People", "kind": "range", "value": 3, "min": 1, "max": 12}],
            "outputs": [{"name": "each", "label": "Each pays", "expression": "bill * (1 + tip / 100) / people",
             "equals": 460, "decimals": 0, "unit": "₹"}]});
        let set = BTreeMap::from([
            ("bill".to_string(), "1500".to_string()),
            ("tip".to_string(), "0".to_string()),
            ("people".to_string(), "4".to_string()),
        ]);
        assert_eq!(
            controls_directive(&card, "Split the bill", &set).expect("fits"),
            "I set Bill to 1500 ₹, Tip to None, People to 4 on “Split the bill”, which gives Each pays 375 ₹."
        );
        let outside = BTreeMap::from([("people".to_string(), "40".to_string())]);
        assert!(controls_directive(&card, "", &outside).is_err());
        let unknown = BTreeMap::from([("guests".to_string(), "2".to_string())]);
        assert!(controls_directive(&card, "", &unknown).is_err());
        let off_menu = BTreeMap::from([("tip".to_string(), "12".to_string())]);
        assert!(controls_directive(&card, "", &off_menu).is_err());
    }
}
