//! Human topic controls over the same sealed records the Agent uses.
//! A topic is a view of addressed turns, never a new conversation or grant.
#![allow(clippy::result_large_err)]

use crate::AppState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use vak_session::work_threads::{TopicError, TopicId, TopicStore};

fn failed(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

fn storage_error(error: TopicError) -> Response {
    let status = match &error {
        TopicError::Invalid(_) => StatusCode::BAD_REQUEST,
        TopicError::Store(_) => StatusCode::SERVICE_UNAVAILABLE,
        TopicError::Conflict => StatusCode::CONFLICT,
        TopicError::AutomaticDisabled => StatusCode::CONFLICT,
    };
    failed(status, &error.to_string())
}

/// Resolve only a real, visible source conversation; never manufacture a
/// store from a caller's Agent or path. Participant routes also pass the live
/// audience middleware. Mutations are owner-only in the route allowlist.
pub(crate) fn source(state: &AppState, id: &str) -> Result<(TopicStore, vak_core::Core), Response> {
    let lifecycle = vak_core::trash::state(&state.core.shared_scope(), id).map_err(|_| {
        failed(
            StatusCode::SERVICE_UNAVAILABLE,
            "Conversation state is unavailable.",
        )
    })?;
    if lifecycle.hidden() {
        return Err(StatusCode::NOT_FOUND.into_response());
    }
    let header = crate::read_historical_header(state, id)
        .filter(|header| header.session_id == id)
        .ok_or_else(|| StatusCode::NOT_FOUND.into_response())?;
    let core = crate::resolve_core_for_header(state, &header);
    let store =
        TopicStore::new(&crate::session_agent_scope(state, id), id).map_err(storage_error)?;
    Ok((store, core))
}

pub(crate) fn focus(
    state: &AppState,
    session: &str,
    id: &str,
) -> Result<vak_session::types::TopicFocus, Response> {
    let (store, _) = source(state, session)?;
    let topic = store
        .get(&TopicId::parse(id).map_err(storage_error)?)
        .map_err(storage_error)?;
    if topic.archived {
        return Err(failed(
            StatusCode::CONFLICT,
            "Reopen this topic before continuing its work.",
        ));
    }
    Ok(vak_session::types::TopicFocus {
        id: topic.id.as_str().to_string(),
        title: topic.title,
        block: 0,
    })
}

pub(crate) async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    match tokio::task::spawn_blocking(move || {
        let (store, core) = source(&state, &id)?;
        snapshot_response(&store, &core)
    })
    .await
    {
        Ok(Ok(response)) | Ok(Err(response)) => response,
        Err(_) => failed(StatusCode::SERVICE_UNAVAILABLE, "Topics could not be read."),
    }
}

fn snapshot_response(store: &TopicStore, core: &vak_core::Core) -> Result<Response, Response> {
    let snapshot = store.snapshot().map_err(storage_error)?;
    let mut pending = false;
    let mut failed = false;
    let mut value = serde_json::to_value(&snapshot).map_err(|_| failed_response())?;
    let runs = core.runs().list().map_err(|_| failed_response())?;
    for (index, name) in snapshot.turn_names.iter().enumerate() {
        let last = runs.iter().find(|run| run.trigger == Some(name.trigger));
        let status = match last.map(|run| run.status) {
            Some(vak_session::runs::RunStatus::Completed) if name.response.is_some() => "complete",
            Some(vak_session::runs::RunStatus::Running) => "pending",
            None if vak_core::triggers::get(&core.shared_scope(), &name.trigger.to_string())
                .map_err(|_| failed_response())?
                .is_some() =>
            {
                "pending"
            }
            _ => "failed",
        };
        pending |= status == "pending";
        failed |= status == "failed";
        value["turn_names"][index]["status"] = json!(status);
    }
    value["organisation_pending"] = json!(pending);
    value["organisation_failed"] = json!(failed);
    Ok(Json(value).into_response())
}

fn failed_response() -> Response {
    failed(
        StatusCode::SERVICE_UNAVAILABLE,
        "Conversation organisation is unavailable.",
    )
}

pub(crate) async fn open(
    State(state): State<AppState>,
    Path((id, topic)): Path<(String, String)>,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        let (store, _) = source(&state, &id)?;
        let topic = TopicId::parse(&topic).map_err(storage_error)?;
        let snapshot = store.snapshot().map_err(storage_error)?;
        let found = snapshot
            .topics
            .iter()
            .find(|found| found.id == topic)
            .ok_or_else(|| StatusCode::NOT_FOUND.into_response())?;
        let placements: Vec<_> = snapshot
            .placements
            .iter()
            .filter(|placement| placement.topic_ids.contains(&topic))
            .collect();
        Ok::<_, Response>(
            Json(json!({"topic": found, "placements": placements,
            "session_id": id, "revision": snapshot.revision}))
            .into_response(),
        )
    })
    .await
    {
        Ok(Ok(response)) | Ok(Err(response)) => response,
        Err(_) => failed(
            StatusCode::SERVICE_UNAVAILABLE,
            "This topic could not be read.",
        ),
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Create {
        title: String,
        #[serde(default)]
        pinned: bool,
    },
    Rename {
        topic_id: String,
        title: String,
    },
    Pin {
        topic_id: String,
        pinned: bool,
    },
    Archive {
        topic_id: String,
    },
    Reopen {
        topic_id: String,
    },
    Assign {
        turn_id: String,
        outcome: String,
        topic_ids: Vec<String>,
        #[serde(default)]
        whole_turn: bool,
    },
    RenameTurn {
        turn_id: String,
        title: String,
    },
    Organise {
        turn_ids: Vec<String>,
        #[serde(default)]
        retry: bool,
    },
    Automatic {
        automatic: bool,
    },
}

#[derive(Deserialize)]
pub(crate) struct Write {
    revision: u64,
    #[serde(flatten)]
    action: Action,
}

pub(crate) async fn change(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(write): Json<Write>,
) -> Response {
    // An authenticated human made this request. Mint and open the real run
    // before moving any Document ref, and retain its guard across all paths.
    if let Action::Organise { turn_ids, retry } = &write.action {
        let (store, core) = match source(&state, &id) {
            Ok(value) => value,
            Err(response) => return response,
        };
        match store.snapshot() {
            Ok(snapshot) if snapshot.revision == write.revision => {}
            Ok(_) => return storage_error(TopicError::Conflict),
            Err(error) => return storage_error(error),
        }
        let core = core.with_run_admission(
            vak_core::admission::RunAdmission::default().actor(crate::request_actor(&state)),
        );
        let mut trace = core.mint_trace(None);
        trace.session = uuid::Uuid::parse_str(&id)
            .ok()
            .and_then(vak_session::ids::SessionId::from_uuid);
        let mut run = match core.runs().begin(&trace, None, None) {
            Ok(run) => run,
            Err(_) => return failed_response(),
        };
        let span = vak_session::runs::span(&trace);
        let result = tracing::Instrument::instrument(
            async {
                core.runs()
                    .session(trace.run, &id)
                    .map_err(|_| failed_response())?;
                let (_, handle) = crate::ensure_session_handle(&state, &id)
                    .await
                    .map_err(|_| failed_response())?;
                let mut guard = handle.session.lock().map_err(|_| failed_response())?;
                let session = guard.as_mut().ok_or_else(|| {
                    failed(
                        StatusCode::CONFLICT,
                        "Wait for the current answer to finish.",
                    )
                })?;
                let mut session = crate::writing(session).map_err(|_| {
                    failed(
                        StatusCode::CONFLICT,
                        "This conversation is being changed elsewhere.",
                    )
                })?;
                if turn_ids
                    .iter()
                    .any(|turn| session.visible_settled_turn(turn).is_none())
                {
                    return Err(StatusCode::NOT_FOUND.into_response());
                }
                for turn in turn_ids {
                    core.queue_turn_organisation(&mut session, turn, *retry, &trace)
                        .map_err(|_| failed_response())?;
                }
                snapshot_response(&store, &core)
            },
            span,
        )
        .await;
        run.settle_with(
            if result.is_ok() {
                vak_session::runs::RunOutcome::Completed
            } else {
                vak_session::runs::RunOutcome::Failed {
                    reason: "Organisation could not be queued".into(),
                }
            },
            None,
        );
        return match result {
            Ok(response) | Err(response) => response,
        };
    }
    let actor = crate::request_actor(&state);
    match tokio::task::spawn_blocking(move || {
        let (store, core) = source(&state, &id)?;
        if let Action::Assign { turn_id, .. } = &write.action {
            let catalog = core
                .catalog()
                .map_err(|_| failed(StatusCode::SERVICE_UNAVAILABLE, "History is unavailable."))?;
            catalog
                .catch_up()
                .map_err(|_| failed(StatusCode::SERVICE_UNAVAILABLE, "History is unavailable."))?;
            let frame = catalog
                .history_frame(&id, turn_id)
                .map_err(|_| failed(StatusCode::SERVICE_UNAVAILABLE, "History is unavailable."))?;
            if frame.is_none_or(|frame| !frame.directive) {
                return Err(failed(
                    StatusCode::NOT_FOUND,
                    "This request is not in this conversation.",
                ));
            }
        }
        let core =
            core.with_run_admission(vak_core::admission::RunAdmission::default().actor(actor));
        let mut trace = core.mint_trace(None);
        trace.session = uuid::Uuid::parse_str(&id)
            .ok()
            .and_then(vak_session::ids::SessionId::from_uuid);
        let span = vak_session::runs::span(&trace);
        let _entered = span.enter();
        let mut run = core.runs().begin(&trace, None, None).map_err(|_| {
            failed(
                StatusCode::SERVICE_UNAVAILABLE,
                "This change could not start.",
            )
        })?;
        let store = store.at_revision(write.revision);
        let result = apply(&store, write.action, &trace);
        match result {
            Ok(created) => {
                let snapshot = store.snapshot().map_err(storage_error)?;
                run.settle_with(vak_session::runs::RunOutcome::Completed, None);
                let mut value = serde_json::to_value(snapshot)
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
                if let Some(created) = created {
                    value["created_topic_id"] = json!(created);
                }
                Ok(Json(value).into_response())
            }
            Err(error) => Err(storage_error(error)),
        }
    })
    .await
    {
        Ok(Ok(response)) | Ok(Err(response)) => response,
        Err(_) => failed(
            StatusCode::SERVICE_UNAVAILABLE,
            "Topics could not be updated.",
        ),
    }
}

fn apply(
    store: &TopicStore,
    action: Action,
    trace: &vak_session::trace::TraceKey,
) -> Result<Option<TopicId>, TopicError> {
    match action {
        Action::RenameTurn { turn_id, title } => store.rename_turn(&turn_id, &title, trace)?,
        Action::Organise { .. } => {
            return Err(TopicError::Invalid(
                "organisation must be queued asynchronously".into(),
            ));
        }
        Action::Create { title, pinned } => return store.create(&title, pinned, trace).map(Some),
        Action::Rename { topic_id, title } => {
            store.rename(&TopicId::parse(&topic_id)?, &title, trace)?
        }
        Action::Pin { topic_id, pinned } => {
            store.set_pinned(&TopicId::parse(&topic_id)?, pinned, trace)?
        }
        Action::Archive { topic_id } => store.archive(&TopicId::parse(&topic_id)?, trace)?,
        Action::Reopen { topic_id } => store.reopen(&TopicId::parse(&topic_id)?, trace)?,
        Action::Assign {
            turn_id,
            outcome,
            topic_ids,
            whole_turn,
        } => {
            let ids = topic_ids
                .iter()
                .map(|id| TopicId::parse(id))
                .collect::<Result<Vec<_>, _>>()?;
            if whole_turn {
                store.assign_turn_human(&turn_id, ids, trace)?;
            } else {
                store.assign_human(&turn_id, &outcome, ids, trace)?;
            }
        }
        Action::Automatic { automatic } => store.set_automatic(automatic, trace)?,
    }
    Ok(None)
}

#[derive(Default, Deserialize)]
pub(crate) struct ListQuery {
    before: Option<String>,
}

/// Reuse the catalog's paged conversation discovery. A topic is never found
/// by walking ledgers, and its contents retain the source audience boundary.
pub(crate) async fn for_agent(
    State(state): State<AppState>,
    Path(agent): Path<String>,
    Query(query): Query<ListQuery>,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        let page = crate::conversation_history::list(&state, false, query.before.as_deref(), None)
            .map_err(IntoResponse::into_response)?;
        let mut topics = Vec::new();
        if let Some(sessions) = page["sessions"].as_array() {
            for session in sessions {
                let source_agent = session["agent"]["id"].as_str().unwrap_or("vak");
                if source_agent != agent {
                    continue;
                }
                let Some(id) = session["session_id"].as_str() else {
                    continue;
                };
                let (store, _) = source(&state, id)?;
                let snapshot = store.snapshot().map_err(storage_error)?;
                for topic in snapshot.topics {
                    let placements: Vec<_> = snapshot
                        .placements
                        .iter()
                        .filter(|placement| placement.topic_ids.contains(&topic.id))
                        .collect();
                    let mut value = serde_json::to_value(&topic)
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
                    value["session_id"] = json!(id);
                    value["placements"] = json!(placements);
                    topics.push(value);
                }
            }
        }
        Ok::<_, Response>(Json(json!({"topics": topics, "next": page["older"]})).into_response())
    })
    .await
    {
        Ok(Ok(response)) | Ok(Err(response)) => response,
        Err(_) => failed(
            StatusCode::SERVICE_UNAVAILABLE,
            "Topics could not be listed.",
        ),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::Router;
    use tower::ServiceExt;

    const OWNER: &str = "topics-owner-secret";

    fn setup() -> (tempfile::TempDir, crate::AppState, Router, String) {
        let dir = tempfile::tempdir().unwrap();
        vak_config::paths::isolate_home_for_tests();
        let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path()));
        let state = crate::AppState::new(core);
        let app = Router::new()
            .route(
                "/sessions/{id}/topics",
                axum::routing::get(super::get).post(super::change),
            )
            .route(
                "/sessions/{id}/topics/{topic}",
                axum::routing::get(super::open),
            )
            .route(
                "/agents/{agent}/topics",
                axum::routing::get(super::for_agent),
            )
            .with_state(state.clone())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::enforce_participant_audience,
            ))
            .layer(axum::middleware::from_fn_with_state(
                crate::AuthPolicy {
                    token: OWNER.into(),
                    home: dir.path().into(),
                    shared: dir.path().into(),
                    trusted_hosts: Vec::new(),
                    public_url: None,
                    browser_sessions: crate::web::BrowserSessions::default(),
                },
                crate::require_bearer,
            ));
        (dir, state, app, OWNER.into())
    }

    fn seed_session(state: &crate::AppState) -> (String, String) {
        let core = &state.core;
        let id = uuid::Uuid::now_v7().to_string();
        let path = vak_session::SessionPath::new_session_file(core.scope().root(), core.cwd(), &id);
        let header = vak_session::types::SessionHeader {
            agent: Some(vak_core::vak_agent_identity()),
            session_id: id.clone(),
            created_at: chrono::Utc::now(),
            cwd: core.cwd().to_path_buf(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
            conversation: Some(vak_session::types::ConversationContext::local(
                &id,
                "topics-test",
            )),
            space: None,
            run: None,
            cause: None,
            contract: vak_session::types::FrozenContract {
                app_version: "test".into(),
                provider: "test".into(),
                model: "test".into(),
                route_ladder: Vec::new(),
                route_objective: String::new(),
                route_annotations: Vec::new(),
                system_prompt: String::new(),
                permission_mode: "workspace-write".into(),
                capabilities: Vec::new(),
                prompt_layers: Vec::new(),
            },
        };
        let mut log = vak_session::SessionLog::create(path, header).unwrap();
        let directive = log
            .append_message(vak_session::types::MessageRecord {
                message: vak_llm::Message::user_text("Organize this request"),
                meta: None,
            })
            .unwrap();
        (id, directive.id)
    }

    fn request(
        bearer: &str,
        method: axum::http::Method,
        uri: String,
        body: Option<serde_json::Value>,
    ) -> axum::http::Request<axum::body::Body> {
        let builder = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {bearer}"),
            )
            .header(axum::http::header::CONTENT_TYPE, "application/json");
        builder
            .body(axum::body::Body::from(
                body.map_or_else(String::new, |v| v.to_string()),
            ))
            .unwrap()
    }

    async fn value(response: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap_or_default()
    }

    fn invite(state: &crate::AppState, session: &str, token: &str, audience: &str) {
        crate::coworking::invite(
            &crate::coworking::grants(state),
            crate::coworking::AudienceGrant {
                grant_id: crate::coworking::test_grant_id(&format!("grant-{token}")),
                principal_id: format!("person-{token}"),
                display_name: "Guest".into(),
                conversation_id: session.into(),
                audience_id: audience.into(),
                capabilities: vec!["read".into()],
                token_hash: crate::coworking::token_hash(token),
                created_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                trace: None,
                actor: None,
            },
        )
        .unwrap();
    }

    #[tokio::test]
    async fn owner_can_queue_backfill_on_the_attached_idle_ledger() {
        let (_dir, state, app, owner) = setup();
        let (id, turn) = seed_session(&state);
        let mut log = state.core.open_session(&id).await.unwrap();
        log.append_message(vak_session::types::MessageRecord {
            message: vak_llm::Message::assistant(vec![vak_llm::ContentBlock::text(
                "The supplied notes are reviewed.",
            )]),
            meta: None,
        })
        .unwrap();
        let index = log.current_turn_index();
        let card = index.turn_by_id(&turn).unwrap().build_card(
            "answered",
            "The supplied notes are reviewed.".into(),
            &|text| text.len() as u64,
        );
        log.append_turn_card(vak_session::types::TurnCardRecord {
            turn_id: turn.clone(),
            card,
        })
        .unwrap();
        let handle = crate::register_handle(
            &state,
            id.clone(),
            log,
            state.core.cwd().clone(),
            state.core.clone(),
        );
        let response = app
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{id}/topics"),
                Some(serde_json::json!({"revision":0,"action":"organise","turn_ids":[turn]})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            value(response).await["turn_names"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            handle
                .session
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .organisation_requests()
                .unwrap()
                .len(),
            1,
            "backfill must append through the existing live handle"
        );
    }

    #[tokio::test]
    async fn owner_can_correct_topic_membership_and_lifecycle_with_revision_checks() {
        let (_dir, state, app, owner) = setup();
        let (session, turn) = seed_session(&state);
        let (other_session, other_turn) = seed_session(&state);

        let create = |revision| {
            request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": revision,
                    "action": "create",
                    "title": "Planning",
                    "pinned": false
                })),
            )
        };
        let first = app.clone().oneshot(create(0)).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let first = value(first).await;
        let first_id = first["created_topic_id"].as_str().unwrap().to_string();
        assert_eq!(first["revision"], 1);

        let second = app.clone().oneshot(create(1)).await.unwrap();
        assert_eq!(second.status(), StatusCode::OK);
        let second = value(second).await;
        let second_id = second["created_topic_id"].as_str().unwrap().to_string();

        let assign = app
            .clone()
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": second["revision"],
                    "action": "assign",
                    "turn_id": turn,
                    "outcome": "Research notes",
                    "topic_ids": [first_id, second_id]
                })),
            ))
            .await
            .unwrap();
        assert_eq!(assign.status(), StatusCode::OK);
        let assigned = value(assign).await;
        let placement = assigned["placements"].as_array().unwrap().last().unwrap();
        assert_eq!(placement["topic_ids"].as_array().unwrap().len(), 2);

        let mismatch = app
            .clone()
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": assigned["revision"],
                    "action": "assign",
                    "turn_id": other_turn,
                    "outcome": "Foreign turn",
                    "topic_ids": []
                })),
            ))
            .await
            .unwrap();
        assert_eq!(mismatch.status(), StatusCode::NOT_FOUND);
        assert_ne!(session, other_session);

        let archive = app
            .clone()
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": assigned["revision"],
                    "action": "archive",
                    "topic_id": first["created_topic_id"]
                })),
            ))
            .await
            .unwrap();
        assert_eq!(archive.status(), StatusCode::OK);
        let archived = value(archive).await;
        assert_eq!(archived["topics"][0]["archived"], true);

        let stale = app
            .clone()
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": assigned["revision"],
                    "action": "automatic",
                    "automatic": false
                })),
            ))
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::CONFLICT);

        let reopen = app
            .clone()
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": archived["revision"],
                    "action": "reopen",
                    "topic_id": first["created_topic_id"]
                })),
            ))
            .await
            .unwrap();
        assert_eq!(reopen.status(), StatusCode::OK);
        let reopened = value(reopen).await;
        assert_eq!(reopened["topics"][0]["archived"], false);

        let automatic = app
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": reopened["revision"],
                    "action": "automatic",
                    "automatic": false
                })),
            ))
            .await
            .unwrap();
        assert_eq!(automatic.status(), StatusCode::OK);
        assert_eq!(value(automatic).await["automatic"], false);
    }

    #[tokio::test]
    async fn participant_reads_only_its_audience_and_trash_hides_topics() {
        let (_dir, state, app, owner) = setup();
        let (session, _) = seed_session(&state);
        let created = app
            .clone()
            .oneshot(request(
                &owner,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": 0,
                    "action": "create",
                    "title": "Only this conversation"
                })),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::OK);

        let guest = "topic-guest-secret";
        invite(&state, &session, guest, "local");
        invite(&state, &session, "topic-wrong-audience", "other-room");

        let visible = app
            .clone()
            .oneshot(request(
                guest,
                axum::http::Method::GET,
                format!("/sessions/{session}/topics"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(visible.status(), StatusCode::OK);
        assert_eq!(value(visible).await["topics"].as_array().unwrap().len(), 1);

        let wrong = app
            .clone()
            .oneshot(request(
                "topic-wrong-audience",
                axum::http::Method::GET,
                format!("/sessions/{session}/topics"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

        let mutation = app
            .clone()
            .oneshot(request(
                guest,
                axum::http::Method::POST,
                format!("/sessions/{session}/topics"),
                Some(serde_json::json!({
                    "revision": 1,
                    "action": "automatic",
                    "automatic": false
                })),
            ))
            .await
            .unwrap();
        assert_eq!(mutation.status(), StatusCode::FORBIDDEN);

        for action in [
            serde_json::json!({"action":"organise","turn_ids":["not-an-authority"],"retry":true}),
            serde_json::json!({"action":"rename_turn","turn_id":"not-an-authority","title":"Guest change"}),
        ] {
            let mut body = action;
            body["revision"] = serde_json::json!(1);
            let refused = app
                .clone()
                .oneshot(request(
                    guest,
                    axum::http::Method::POST,
                    format!("/sessions/{session}/topics"),
                    Some(body),
                ))
                .await
                .unwrap();
            assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        }

        let aggregate = app
            .clone()
            .oneshot(request(
                guest,
                axum::http::Method::GET,
                "/agents/vak/topics".into(),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(aggregate.status(), StatusCode::FORBIDDEN);

        crate::coworking::revoke(
            &crate::coworking::grants(&state),
            &crate::coworking::test_grant_id(&format!("grant-{guest}")).to_string(),
            None,
        )
        .unwrap();
        let revoked = app
            .clone()
            .oneshot(request(
                guest,
                axum::http::Method::GET,
                format!("/sessions/{session}/topics"),
                None,
            ))
            .await
            .unwrap();
        assert!(
            matches!(
                revoked.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ),
            "revoked readers cannot reopen Topic records"
        );

        vak_core::trash::set(
            &state.core.shared_scope(),
            std::slice::from_ref(&session),
            true,
        )
        .unwrap();
        let hidden = app
            .oneshot(request(
                &owner,
                axum::http::Method::GET,
                format!("/sessions/{session}/topics"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND);
    }
}
