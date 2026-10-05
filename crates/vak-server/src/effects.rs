//! The effects API (plan M4.5): what Vak did outside itself, read from the
//! `effects/` chain, and the two things a person may do about one whose
//! outcome is open: send it again, or say whether it was sent. Each of
//! those appends a before/after receipt to the operations actions chain
//! (invariant 26).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use vak_session::effects::{EffectRecord, EffectStatus, Reconciled};
use vak_session::ids::{EffectId, RunId};

use crate::AppState;
use crate::operations;

#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct EffectsQuery {
    run: Option<String>,
    state: Option<EffectStatus>,
    limit: Option<usize>,
}

fn error(status: StatusCode, message: impl std::fmt::Display) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": message.to_string() })),
    )
        .into_response()
}

fn parse(id: &str) -> Result<EffectId, Box<Response>> {
    EffectId::parse(id).map_err(|_| Box::new(error(StatusCode::BAD_REQUEST, "not an effect id")))
}

pub(crate) async fn list(
    State(state): State<AppState>,
    Query(query): Query<EffectsQuery>,
) -> Response {
    let run = match query.run.as_deref().map(RunId::parse).transpose() {
        Ok(run) => run,
        Err(_) => return error(StatusCode::BAD_REQUEST, "not a run id"),
    };
    match state.core.effects().list() {
        Ok(effects) => {
            let effects: Vec<EffectRecord> = effects
                .into_iter()
                .filter(|effect| run.is_none_or(|run| effect.run == Some(run)))
                .filter(|effect| query.state.is_none_or(|status| effect.status == status))
                .take(query.limit.unwrap_or(200).min(1000))
                .collect();
            Json(serde_json::json!({ "effects": effects })).into_response()
        }
        Err(failure) => error(StatusCode::INTERNAL_SERVER_ERROR, failure),
    }
}

pub(crate) async fn detail(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match parse(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match state.core.effects().get(id) {
        Ok(Some(effect)) => Json(effect).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(failure) => error(StatusCode::INTERNAL_SERVER_ERROR, failure),
    }
}

fn status_of(state: &AppState, id: EffectId) -> String {
    match state.core.effects().get(id) {
        Ok(Some(effect)) => serde_json::to_value(effect.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default(),
        Ok(None) => "not found".into(),
        Err(_) => "unreadable".into(),
    }
}

fn record_receipt(
    state: &AppState,
    id: EffectId,
    action: &str,
    (before, after): (String, String),
    requested_at: chrono::DateTime<Utc>,
    succeeded: bool,
    detail: String,
) -> operations::ActionReceipt {
    let mut receipt = operations::ActionReceipt {
        receipt_id: format!("OP-{}", uuid::Uuid::now_v7().simple()),
        service: format!("effect:{id}"),
        action: action.to_string(),
        requested_at,
        completed_at: Utc::now(),
        succeeded,
        verification: operations::ActionVerification {
            status: if after == "sent" {
                "verified"
            } else {
                "pending"
            }
            .to_string(),
            before,
            after,
            detail,
        },
        persisted: false,
        trace: Some(crate::request_trace(state, &format!("effect-{action}"))),
        actor: Some(crate::request_actor(state)),
    };
    receipt.persisted = operations::record_action(&state.core.shared_scope(), &receipt).is_ok();
    receipt
}

/// Sends an effect again (`crate::delivery::resend` says how).
pub(crate) async fn resend(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let id = match parse(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    let before = status_of(&state, id);
    let requested_at = Utc::now();
    let outcome = crate::delivery::resend(&state.core, id).await;
    let sent_as = match &outcome {
        Ok(crate::delivery::Resent::Same(sent) | crate::delivery::Resent::New(sent)) => *sent,
        Err(_) => id,
    };
    let after = status_of(&state, sent_as);
    let receipt = record_receipt(
        &state,
        id,
        "resend",
        (before, after.clone()),
        requested_at,
        outcome.is_ok(),
        match &outcome {
            Ok(crate::delivery::Resent::Same(_)) => "Sent again as the same action.".into(),
            Ok(crate::delivery::Resent::New(next)) => {
                format!("Sent again as a new action, {next}, which replaces it.")
            }
            Err(reason) => format!("Not sent again: {reason}."),
        },
    );
    match outcome {
        Ok(resent) => Json(serde_json::json!({
            "ok": true,
            "effect": sent_as.to_string(),
            "new": matches!(resent, crate::delivery::Resent::New(_)),
            "status": after,
            "receipt_id": receipt.receipt_id,
            "receipt_persisted": receipt.persisted,
        }))
        .into_response(),
        Err(reason) => error(StatusCode::CONFLICT, reason),
    }
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ReconcileBody {
    outcome: Reconciled,
}

/// The owner says whether an effect whose outcome is open was sent.
pub(crate) async fn reconcile(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ReconcileBody>,
) -> Response {
    let id = match parse(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    if let Err(fenced) = vak_session::fence::check() {
        return error(StatusCode::SERVICE_UNAVAILABLE, fenced);
    }
    let before = status_of(&state, id);
    let requested_at = Utc::now();
    let outcome =
        state
            .core
            .effects()
            .reconcile(id, body.outcome, None, crate::request_actor(&state));
    let after = status_of(&state, id);
    let receipt = record_receipt(
        &state,
        id,
        "reconcile",
        (before, after.clone()),
        requested_at,
        outcome.is_ok(),
        match (&outcome, body.outcome) {
            (Ok(_), Reconciled::Sent) => "The owner said it was sent.".into(),
            (Ok(_), Reconciled::NotSent) => "The owner said it was not sent.".into(),
            (Err(failure), _) => format!("Not settled: {failure}."),
        },
    );
    match outcome {
        Ok(effect) => Json(serde_json::json!({
            "ok": true,
            "effect": effect,
            "receipt_id": receipt.receipt_id,
            "receipt_persisted": receipt.persisted,
        }))
        .into_response(),
        Err(failure) => error(StatusCode::CONFLICT, failure),
    }
}

/// The Operations Center's view of effects: the newest 200, and how many
/// are waiting, unknown or failed.
pub(crate) fn operations_view(state: &AppState) -> serde_json::Value {
    match state.core.effects().list() {
        Ok(effects) => {
            let count = |want: &[EffectStatus]| {
                effects
                    .iter()
                    .filter(|effect| want.contains(&effect.status))
                    .count()
            };
            serde_json::json!({
                "waiting": count(&[EffectStatus::Queued, EffectStatus::Sending, EffectStatus::Retrying]),
                "held": count(&[EffectStatus::Held]),
                "unknown": count(&[EffectStatus::Unknown]),
                "failed": count(&[EffectStatus::Failed]),
                "records": effects.into_iter().take(200).collect::<Vec<_>>(),
                "error": null,
            })
        }
        Err(failure) => serde_json::json!({
            "waiting": 0, "held": 0, "unknown": 0, "failed": 0,
            "records": [],
            "error": failure.to_string(),
        }),
    }
}
