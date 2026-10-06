//! Intake over HTTP (plan M6.5, docs/design/76-intake-and-knowledge.md):
//! the sources an Agent follows, the run each poll is, and what the polls
//! took. A source is made, changed and removed here with the trigger that
//! polls it; the generic automation endpoints refuse to touch that trigger.

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::Utc;
use vak_core::intake::{self, Connector, Intake, IntakeError, IntakeStep, Source, Trust};
use vak_core::triggers::{self, Schedule, Trigger, TriggerAction, TriggerKind};
use vak_session::ids::{SourceId, TriggerId};
use vak_session::runs::{OpenRun, RunOutcome};

use crate::AppState;

/// The shortest interval a source may be polled at.
const MIN_EVERY_MINUTES: u64 = 5;
const DEFAULT_EVERY_MINUTES: u64 = 60;

fn shared(state: &AppState) -> vak_config::scope::SharedScope {
    state.core.shared_scope()
}

fn tenant_home(state: &AppState) -> std::path::PathBuf {
    vak_config::paths::tenant_home_at(
        &state.core.shared_scope().into_root(),
        vak_config::paths::LOCAL_TENANT,
    )
}

fn intake_of(state: &AppState) -> Intake {
    Intake::at(&shared(state), tenant_home(state))
}

fn cursors_of(state: &AppState) -> vak_session::cursors::Cursors {
    vak_session::cursors::Cursors::at(shared(state).cursors(), tenant_home(state))
}

fn error_response(status: StatusCode, error: impl std::fmt::Display) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": error.to_string() })),
    )
        .into_response()
}

fn intake_error(error: IntakeError) -> Response {
    let status = match &error {
        IntakeError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
        IntakeError::NotFound(_) => StatusCode::NOT_FOUND,
        IntakeError::Busy => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    error_response(status, error)
}

/// Does a claimed poll's work and settles its run with how it went.
pub(crate) async fn fire_poll(
    state: &AppState,
    trigger: &Trigger,
    source: SourceId,
    mut run: OpenRun,
    trace: vak_session::trace::TraceKey,
) {
    let found = match intake::get(&shared(state), &source.to_string()) {
        Ok(Some(found)) => found,
        Ok(None) => {
            run.settle_with(
                RunOutcome::Failed {
                    reason: "the source was removed".into(),
                },
                None,
            );
            return;
        }
        Err(error) => {
            run.settle_with(
                RunOutcome::Failed {
                    reason: error.to_string(),
                },
                None,
            );
            return;
        }
    };
    let polled = intake::poll(
        &intake_of(state),
        &cursors_of(state),
        &found,
        &trace,
        &state.core.tool_worker_exe(),
    )
    .await;
    match polled {
        Ok(polled) => {
            tracing::info!(
                trigger = %trigger.id,
                count = polled.taken,
                outcome = if polled.unchanged { "unchanged" } else { "taken" },
                "a source was polled"
            );
            run.settle_with(RunOutcome::Completed, None);
        }
        Err(error) => {
            tracing::warn!(
                trigger = %trigger.id,
                error_kind = %vak_telemetry::error_kind(&error),
                "a source poll failed"
            );
            run.settle_with(
                RunOutcome::Failed {
                    reason: error.to_string(),
                },
                None,
            );
        }
    }
}

/// A source with what its poll's trigger says.
fn projection(source: &Source, trigger: Option<&Trigger>) -> serde_json::Value {
    let every_minutes = trigger.and_then(|trigger| match trigger.schedule() {
        Some(Schedule::Interval { every_secs, .. }) => Some(every_secs / 60),
        _ => None,
    });
    let mut value = serde_json::json!(source);
    value["kind"] = serde_json::json!(source.connector.kind());
    value["url"] = serde_json::json!(source.connector.url());
    value["enabled"] = serde_json::json!(trigger.is_some_and(|trigger| trigger.enabled));
    value["every_minutes"] = serde_json::json!(every_minutes);
    value
}

fn trigger_of(state: &AppState, source: &Source) -> Option<Trigger> {
    triggers::get(&shared(state), &source.trigger.to_string())
        .ok()
        .flatten()
}

/// `GET /intake/sources`
async fn list_sources(State(state): State<AppState>) -> Response {
    match intake::list(&shared(&state)) {
        Ok(sources) => Json(serde_json::json!({
            "sources": sources
                .iter()
                .map(|source| projection(source, trigger_of(&state, source).as_ref()))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => intake_error(error),
    }
}

#[derive(serde::Deserialize)]
struct SourceDraft {
    name: String,
    #[serde(default)]
    agent: Option<String>,
    connector: Connector,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    trust: Trust,
    #[serde(default)]
    every_minutes: Option<u64>,
    #[serde(default)]
    enabled: Option<bool>,
}

/// A poll interval in seconds, or why it is refused.
fn every(minutes: Option<u64>) -> Result<u64, &'static str> {
    let minutes = minutes.unwrap_or(DEFAULT_EVERY_MINUTES);
    if !(MIN_EVERY_MINUTES..=7 * 24 * 60).contains(&minutes) {
        return Err("a source is polled every 5 minutes to 7 days");
    }
    Ok(minutes * 60)
}

/// `POST /intake/sources`: the source and the trigger that polls it.
async fn create_source(State(state): State<AppState>, Json(draft): Json<SourceDraft>) -> Response {
    let every_secs = match every(draft.every_minutes) {
        Ok(secs) => secs,
        Err(reason) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, reason),
    };
    let actor = crate::request_actor(&state);
    let agent = draft
        .agent
        .filter(|agent| !agent.trim().is_empty())
        .unwrap_or_else(|| "vak".into());
    let id = SourceId::new();
    let now = Utc::now();
    let trigger = Trigger {
        id: TriggerId::new(),
        name: format!("Poll {}", draft.name.trim()),
        agent: agent.clone(),
        agent_revision: None,
        space: vak_config::spaces::key(state.core.cwd()),
        enabled: draft.enabled.unwrap_or(true),
        kind: TriggerKind::Schedule {
            schedule: Schedule::Interval {
                every_secs,
                anchor: now,
            },
        },
        action: TriggerAction::SourcePoll { source: id },
        deliver_to: None,
        on_crash: Default::default(),
        scope: None,
        created_at: now,
        created_by: Some(actor),
    };
    let source = Source {
        id,
        name: draft.name.trim().to_string(),
        agent,
        connector: draft.connector,
        tags: draft.tags,
        trust: draft.trust,
        trigger: trigger.id,
        created_at: now,
        created_by: Some(actor),
    };
    if let Err(error) = source.validate() {
        return intake_error(error);
    }
    if let Err(error) = triggers::create(&shared(&state), &trigger) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, error);
    }
    if let Err(error) = intake::create(&shared(&state), &source) {
        let _ = triggers::delete(&shared(&state), &trigger.id.to_string());
        return intake_error(error);
    }
    (
        StatusCode::CREATED,
        Json(projection(&source, Some(&trigger))),
    )
        .into_response()
}

/// `GET /intake/sources/{id}`
async fn get_source(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match intake::get(&shared(&state), &id) {
        Ok(Some(source)) => {
            Json(projection(&source, trigger_of(&state, &source).as_ref())).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => intake_error(error),
    }
}

#[derive(serde::Deserialize)]
struct SourcePatch {
    name: Option<String>,
    connector: Option<Connector>,
    tags: Option<Vec<String>>,
    trust: Option<Trust>,
    every_minutes: Option<u64>,
    enabled: Option<bool>,
}

/// `PATCH /intake/sources/{id}`: the source, and its poll's schedule and
/// whether it runs.
async fn update_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(patch): Json<SourcePatch>,
) -> Response {
    let every_secs = match patch
        .every_minutes
        .map(|minutes| every(Some(minutes)))
        .transpose()
    {
        Ok(secs) => secs,
        Err(reason) => return error_response(StatusCode::UNPROCESSABLE_ENTITY, reason),
    };
    let source = match intake::update(&shared(&state), &id, |source| {
        if let Some(name) = &patch.name {
            source.name = name.trim().to_string();
        }
        if let Some(connector) = &patch.connector {
            source.connector = connector.clone();
        }
        if let Some(tags) = &patch.tags {
            source.tags = tags.clone();
        }
        if let Some(trust) = patch.trust {
            source.trust = trust;
        }
        Ok(())
    }) {
        Ok(source) => source,
        Err(error) => return intake_error(error),
    };
    let trigger = triggers::update(&shared(&state), &source.trigger.to_string(), |trigger| {
        trigger.name = format!("Poll {}", source.name);
        if let Some(enabled) = patch.enabled {
            trigger.enabled = enabled;
        }
        if let Some(every_secs) = every_secs {
            trigger.kind = TriggerKind::Schedule {
                schedule: Schedule::Interval {
                    every_secs,
                    anchor: Utc::now(),
                },
            };
        }
        Ok(())
    });
    match trigger {
        Ok(trigger) => Json(projection(&source, Some(&trigger))).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

/// `DELETE /intake/sources/{id}`: the source and its poll. What it took
/// stays until erasure (M7a).
async fn delete_source(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let source = match intake::get(&shared(&state), &id) {
        Ok(Some(source)) => source,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => return intake_error(error),
    };
    if let Err(error) = intake::delete(&shared(&state), &id) {
        return intake_error(error);
    }
    if let Err(error) = triggers::delete(&shared(&state), &source.trigger.to_string()) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /intake/sources/{id}/poll`: polls now, through the trigger's
/// claim like any Run now.
async fn poll_source(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let source = match intake::get(&shared(&state), &id) {
        Ok(Some(source)) => source,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => return intake_error(error),
    };
    let Some(trigger) = trigger_of(&state, &source) else {
        return error_response(StatusCode::CONFLICT, "the source has no poll");
    };
    let event = uuid::Uuid::now_v7().to_string();
    match crate::automations::claim_and_fire(&state, trigger, Utc::now(), Some(event)).await {
        Ok(_) => StatusCode::ACCEPTED.into_response(),
        Err(crate::automations::NotFired::Busy) => error_response(
            StatusCode::CONFLICT,
            "It is already polling, so this run was skipped.",
        ),
        Err(crate::automations::NotFired::Refused) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "It could not poll; the inbox says why.",
        ),
    }
}

#[derive(serde::Deserialize)]
struct ItemsQuery {
    source: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
}

/// `GET /intake/items`: what the polls took, newest first.
async fn list_items(State(state): State<AppState>, Query(query): Query<ItemsQuery>) -> Response {
    let core = state.core.clone();
    let found = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        catalog.catch_up()?;
        Ok::<_, vak_core::CoreError>(catalog.list(
            "item",
            &vak_catalog::Audience::default(),
            10_000,
        )?)
    })
    .await;
    let nodes = match found {
        Ok(Ok(nodes)) => nodes,
        Ok(Err(error)) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    let prefix = query.source.map(|source| format!("itm:{source}:"));
    let items: Vec<_> = nodes
        .into_iter()
        .filter(|node| {
            prefix
                .as_ref()
                .is_none_or(|prefix| node.id.starts_with(prefix.as_str()))
        })
        .filter(|node| {
            query
                .status
                .as_deref()
                .is_none_or(|status| node.status.as_deref() == Some(status))
        })
        .take(query.limit.unwrap_or(100).clamp(1, 1_000))
        .collect();
    Json(serde_json::json!({ "items": items })).into_response()
}

/// `GET /intake/items/{id}`: an item, its body and what detection said.
async fn get_item(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let intake = intake_of(&state);
    let core = state.core.clone();
    let lookup = id.clone();
    let node = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        catalog.catch_up()?;
        Ok::<_, vak_core::CoreError>(catalog.open_node(&lookup)?)
    })
    .await;
    let node = match node {
        Ok(Ok(Some(node))) if node.kind == "item" => node,
        Ok(Ok(_)) => return StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
        Err(error) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, error),
    };
    let Some(taken) = intake.taken(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let IntakeStep::Taken {
        source,
        object,
        labels,
        evidence,
        ..
    } = &taken.step
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let body = match intake.body(source, object) {
        Ok(body) => body,
        Err(error) => return intake_error(error),
    };
    Json(serde_json::json!({
        "item": node,
        "body": body,
        "labels": labels,
        "evidence": evidence,
    }))
    .into_response()
}

#[derive(serde::Deserialize, Default)]
struct HoldBody {
    reason: Option<String>,
}

async fn decide(state: &AppState, id: &str, step: IntakeStep) -> Response {
    let intake = intake_of(state);
    if intake.taken(id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    match intake.decide(id, crate::request_actor(state), step) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => intake_error(error),
    }
}

/// `POST /intake/items/{id}/release`: lets a held item reach the Agent.
async fn release_item(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    decide(&state, &id, IntakeStep::Released).await
}

/// `POST /intake/items/{id}/quarantine`: holds an item back from the Agent.
async fn quarantine_item(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<HoldBody>>,
) -> Response {
    let reason = body
        .and_then(|Json(body)| body.reason)
        .filter(|reason| !reason.trim().is_empty());
    decide(&state, &id, IntakeStep::Quarantined { reason }).await
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/intake/sources", get(list_sources).post(create_source))
        .route(
            "/intake/sources/{id}",
            get(get_source).patch(update_source).delete(delete_source),
        )
        .route("/intake/sources/{id}/poll", post(poll_source))
        .route("/intake/items", get(list_items))
        .route("/intake/items/{id}", get(get_item))
        .route("/intake/items/{id}/release", post(release_item))
        .route("/intake/items/{id}/quarantine", post(quarantine_item))
}
