use crate::{AppState, agents, register_handle};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::io::BufRead;
use vak_session::types::{
    AgentIdentity, ConversationContext, ConversationOrigin, Entry, EntryPayload, SessionHeader,
};

pub(crate) fn header(path: &std::path::Path) -> Result<SessionHeader, String> {
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

/// Resolve an Agent id to its identity and its own isolated `Core` — the
/// single place this resolution happens, so every endpoint that needs "this
/// Agent's own data" (workspace, sessions_home, runtime safety pins) goes
/// through the same logic `open` uses, rather than each one re-deriving or
/// (worse) silently falling back to the process's default workspace. Any
/// new endpoint scoped to a specific Agent should call this rather than
/// reading `state.core`/`state.active_core()` directly.
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_agent_core(
    state: &AppState,
    id: &str,
) -> Result<(AgentIdentity, vak_core::Core), Response> {
    let active = state.active_core();
    let identity = if id == "vak" {
        AgentIdentity {
            id: id.to_string(),
            revision: 1,
            name: "Vak".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
        }
    } else {
        let profiles =
            agents::effective(&active).map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let Some(profile) = profiles.into_iter().find(|p| p.id == id) else {
            return Err(error(
                StatusCode::NOT_FOUND,
                "This agent is no longer available.",
            ));
        };
        if !profile.is_admissible() {
            return Err(error(
                StatusCode::CONFLICT,
                "This Agent is paused or archived.",
            ));
        }
        profile.identity()
    };
    // Each user-created Agent gets its own isolated project workspace (files,
    // tool access, permissions) rather than sharing the process's default
    // workspace — resolved through the same `CorePool` a channel/gateway
    // workspace switch uses, so trust/permission/sandbox resolution is
    // identical to a local run rooted there. The built-in "vak" agent keeps
    // the process's own workspace for backward compatibility.
    //
    // The base workspace must match whichever root `agents::save` used to
    // persist this profile (`agents.rs` saves "user"-scope agents under
    // `default_workspace()`, "workspace"-scope under the saving request's
    // own cwd) — otherwise, whenever the active core points somewhere other
    // than `default_workspace()` (a browser workspace switch, a gateway
    // channel), a "user"-scope agent's precreated directory and its actual
    // runtime workspace would silently diverge. `agents::effective` already
    // gives the workspace layer precedence over the shared layer for a
    // duplicate id, so mirror that precedence here.
    let default_root = vak_config::paths::default_workspace();
    let base = if identity.id == "vak" {
        active.cwd().clone()
    } else {
        let is_workspace_scoped = active.cwd() != &default_root
            && agents::load(active.cwd())
                .unwrap_or_default()
                .iter()
                .any(|p| p.id == identity.id);
        if is_workspace_scoped {
            active.cwd().clone()
        } else {
            default_root
        }
    };
    let workspace = vak_config::paths::agent_workspace(&base, &identity.id);
    // `agents::save` already creates this directory once at agent-creation
    // time; opening an agent is idempotent and hit repeatedly (reload, tab
    // switch, reconnect), so skip the mkdir once it's confirmed to exist
    // rather than paying the syscalls on every open.
    if !workspace.is_dir()
        && let Err(e) = std::fs::create_dir_all(&workspace)
    {
        return Err(error(StatusCode::INTERNAL_SERVER_ERROR, e));
    }
    // Backstop for an Agent whose workspace predates `agents::save` carrying
    // trust forward (or was created by some other path this fix missed):
    // without a trust marker here, `CorePool::resolve_at` below treats it as
    // untrusted and silently strips its own `permission_mode`, `hooks`,
    // `mcp.servers`, and other privileged config forever — the same gap
    // `agents::save` closes at creation time, applied retroactively the
    // first time this Agent is opened from a trusted context.
    if identity.id != "vak"
        && active.project_config_trusted()
        && !vak_core::trust::is_trusted(&workspace)
    {
        let _ = vak_core::trust::mark_trusted(&workspace);
    }
    let core = if workspace == *active.cwd() {
        active
    } else {
        let resolved =
            match state
                .gateway
                .core_pool
                .resolve_at(&workspace, None, std::time::Instant::now())
            {
                Ok(core) => core,
                // `resolve_at` failing (a transient permission-ceiling recheck
                // error on a cache hit, or `Core::new_with_trust`'s own IO/config
                // error) is not itself a trust decision — falling back to an
                // unconditional `true` here would let an operator-declined
                // workspace's hooks/MCP servers/`.env` apply anyway, exactly the
                // bypass `vak_core::trust` exists to close. Recompute trust the
                // same way `resolve_at` does rather than assuming it.
                Err(_) => match vak_core::Core::new_with_trust(
                    workspace.clone(),
                    vak_core::trust::is_trusted(&workspace),
                ) {
                    Ok(core) => core,
                    Err(e) => return Err(error(StatusCode::INTERNAL_SERVER_ERROR, e)),
                },
            };
        // A freshly-resolved Core has its own default sessions/data home
        // (real on-disk `data_home()`), which would silently diverge from
        // wherever this app/process's data actually lives if the active
        // Core was pointed at a non-default root (test isolation, or a
        // future custom data-home setting). Every agent's data must live
        // under the *same* root, just in its own agent-scoped subdirectory
        // (`Core::sessions_home` already layers that on top).
        resolved.set_sessions_home(active.shared_data_home());
        if let Some(provider) = active.provider_instance_override() {
            resolved.set_provider_instance(provider);
        }
        // A user-pinned safety ceiling (e.g. read-only mode, or a hardened
        // sandbox backend) is a this-session/this-app control, not a
        // per-project-directory config value — it must not silently loosen
        // the moment a different Agent's Core is resolved from that
        // workspace's own on-disk config. Carry the pin forward the same
        // way a persisted config value already is via `sessions_home`.
        if let Some(mode) = active.permission_mode_override_value() {
            // Cap against this workspace's own resolved ceiling, the same
            // way a per-channel override is capped in `core_pool.rs` — a
            // pin from a more-permissive workspace must never grant more
            // access than this agent's own config already allows.
            let ceiling = resolved.effective_permission_mode();
            resolved.set_permission_mode(mode.capped_by(ceiling));
        }
        if let Some(backend) = active.sandbox_backend_override_value() {
            resolved.set_sandbox_backend(Some(backend));
        }
        resolved
    };
    let core = core.with_agent_identity(Some(identity.clone()));
    Ok((identity, core))
}

/// Opening an agent is idempotent across reloads and clients. The immutable
/// header is the ownership record; browser storage has no routing authority.
pub(crate) async fn open(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let (identity, core) = match resolve_agent_core(&state, &id) {
        Ok(pair) => pair,
        Err(response) => return response,
    };
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
    let core = core.with_conversation_context(Some(conversation.clone()));
    let dir = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd());
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
            let has_content = std::fs::File::open(entry.path())
                .ok()
                .map(|f| std::io::BufReader::new(f).lines().count() > 1)
                .unwrap_or(false);
            // Per-turn routing: provider/model are resolved fresh each turn,
            // so every session for this conversation is a valid candidate
            // regardless of its initial frozen contract values.
            let route_matches = true;
            candidates.push((h, has_content, route_matches));
        }
    }
    candidates.sort_by_key(|(h, has_content, route_matches)| {
        (*has_content, *route_matches, h.created_at)
    });
    if let Some((h, _, _)) = candidates.last() {
        let sid = h.session_id.clone();
        if state.get(&sid).is_none() {
            let session = match core.open_session(&sid).await {
                Ok(session) => Some(session),
                Err(vak_core::CoreError::Session(vak_session::SessionError::Locked(_))) => {
                    core.open_session_read_only(&sid).await.ok()
                }
                Err(e) => return error(StatusCode::CONFLICT, e),
            };
            if let Some(session) = session {
                register_handle(
                    &state,
                    sid.clone(),
                    session,
                    core.cwd().clone(),
                    core.clone(),
                );
            }
        }
        return Json(serde_json::json!({"session_id": sid, "agent": h.agent, "cwd": core.cwd()}))
            .into_response();
    }
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
