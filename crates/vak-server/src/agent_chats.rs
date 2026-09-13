use crate::{AppState, agents, register_handle};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use vak_session::types::{
    AgentIdentity, ConversationContext, ConversationOrigin, Entry, EntryPayload, SessionHeader,
};

pub(crate) fn header(path: &std::path::Path) -> Result<SessionHeader, String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let line = std::io::BufReader::new(file)
        .lines()
        .next()
        .ok_or("empty session ledger")?
        .map_err(|e| e.to_string())?;
    match serde_json::from_str::<Entry>(&line)
        .map_err(|e| e.to_string())?
        .payload
    {
        EntryPayload::Header(header) => Ok(header),
        _ => Err("session ledger has no header".into()),
    }
}

fn error(status: StatusCode, message: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({"error": message.to_string()})),
    )
        .into_response()
}

pub(crate) async fn list(State(state): State<AppState>) -> Response {
    match agents::effective(&state.active_core()) {
        Ok(agents) => Json(serde_json::json!({"agents": agents})).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// Opening an agent is idempotent across reloads and clients. The immutable
/// header is the ownership record; browser storage has no routing authority.
pub(crate) async fn open(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let core = state.active_core();
    let identity = if id == "vak" {
        AgentIdentity {
            id,
            revision: 1,
            name: "Vak".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
        }
    } else {
        let profiles = match agents::effective(&core) {
            Ok(profiles) => profiles,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        let Some(profile) = profiles.into_iter().find(|p| p.id == id) else {
            return error(StatusCode::NOT_FOUND, "This agent is no longer available.");
        };
        if !profile.is_admissible() {
            return error(StatusCode::CONFLICT, "This Agent is paused or archived.");
        }
        profile.identity()
    };
    let dir = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd());
    // The desktop surface has one durable conversation per selected Agent.
    // This is deliberately derived from the Agent identity, not from browser
    // storage or a transient session id, so reopening the same Agent resumes
    // the same conversation while another Agent gets an independent ledger.
    let conversation = ConversationContext {
        conversation_id: format!("agent:{}:local", identity.id),
        audience_id: "local".into(),
        origin: Some(ConversationOrigin {
            surface: "desktop".into(),
            address: "local".into(),
            bot_id: None,
        }),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return error(StatusCode::INTERNAL_SERVER_ERROR, e);
    }
    // One short cross-process admission lock per workspace. A contending
    // request retries explicitly; it cannot create a second conversation.
    let lock = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("agent-admission.lock"))
    {
        Ok(file) => file,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    if lock.try_lock().is_err() {
        return error(StatusCode::CONFLICT, "An agent is opening. Try again.");
    }
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        if entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let h = match header(&entry.path()) {
            Ok(h) => h,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
        };
        if h.cwd == *core.cwd()
            && h.parent_session_id.is_none()
            && h.agent.as_ref().is_some_and(|a| a.id == identity.id)
            && h.conversation.as_ref() == Some(&conversation)
        {
            candidates.push(h);
        }
    }
    candidates.sort_by_key(|h| h.created_at);
    if let Some(h) = candidates.first() {
        let sid = h.session_id.clone();
        if state.get(&sid).is_none() {
            let session = match core.open_session(&sid).await {
                Ok(session) => session,
                Err(e) => return error(StatusCode::CONFLICT, e),
            };
            register_handle(
                &state,
                sid.clone(),
                session,
                core.cwd().clone(),
                core.clone(),
            );
        }
        return Json(serde_json::json!({"session_id": sid, "agent": h.agent, "cwd": core.cwd()}))
            .into_response();
    }
    let core = core
        .with_agent_identity(Some(identity.clone()))
        .with_conversation_context(Some(conversation));
    let session = match core.start_session().await {
        Ok(session) => session,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let Some(h) = session.header() else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "Session has no header");
    };
    let sid = h.session_id.clone();
    register_handle(
        &state,
        sid.clone(),
        session,
        core.cwd().clone(),
        core.clone(),
    );
    Json(serde_json::json!({"session_id": sid, "agent": identity, "cwd": core.cwd()}))
        .into_response()
}
