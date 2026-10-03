use crate::{AppState, agents, register_handle};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use vak_session::types::{AgentIdentity, ConversationContext, ConversationOrigin, SessionHeader};

pub(crate) fn header(path: &std::path::Path) -> Result<SessionHeader, String> {
    vak_session::SessionLog::read_header(path).map_err(|e| e.to_string())
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

/// Identity lookup plus admissibility check by id alone, without deriving a
/// workspace path. Shared by [`resolve_agent_core`] (a fresh or looked-up
/// conversation, which still needs to compute a workspace) and
/// `crate::resolve_core_for_header` (an existing session, which already
/// knows its own workspace from its header and must never let a re-derived
/// path disagree with it — see that function's doc comment).
#[allow(clippy::result_large_err)]
pub(crate) fn resolve_agent_identity(
    active: &vak_core::Core,
    id: &str,
) -> Result<AgentIdentity, Response> {
    if id == "vak" {
        return Ok(AgentIdentity {
            id: id.to_string(),
            revision: 1,
            name: "Vakyartha".into(),
            character: "vak".into(),
            personality: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
        });
    }
    let profiles =
        agents::effective(active).map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
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
    Ok(profile.identity())
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
    let identity = resolve_agent_identity(&active, id)?;
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
    let core = pinned_core_for_workspace(state, &active, &identity, &workspace)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((identity, core))
}

/// Resolve a `Core` pinned exactly the way `/agents/{id}/open` pins one:
/// runtime permission-mode cap, sandbox backend override, provider instance
/// override, and shared sessions_home carried forward from `active`, plus
/// the one-time trust backstop for a workspace that predates `agents::save`
/// recording trust.
///
/// Takes the target `workspace` directly rather than re-deriving it from
/// `active.cwd()` (finding 6): `resolve_agent_core` still has to compute a
/// workspace path for a fresh-or-looked-up conversation, but a session that
/// already exists knows its own workspace from its header, and re-deriving
/// one from whatever happens to be the CURRENT active workspace can
/// disagree with it once the active workspace has moved on. Before this,
/// `/attach`, `/run` and the SSE endpoints resolved a plain pooled `Core`
/// with none of these pins, so the same session's security ceiling
/// depended on which endpoint touched it first.
pub(crate) fn pinned_core_for_workspace(
    state: &AppState,
    active: &vak_core::Core,
    identity: &AgentIdentity,
    workspace: &std::path::Path,
) -> Result<vak_core::Core, vak_core::CoreError> {
    // Backstop for an Agent whose workspace predates `agents::save` carrying
    // trust forward (or was created by some other path this fix missed):
    // without a trust marker here, `CorePool::resolve_at` below treats it as
    // untrusted and silently strips its own `permission_mode`, `hooks`,
    // `mcp.servers`, and other privileged config forever — the same gap
    // `agents::save` closes at creation time, applied retroactively the
    // first time this Agent is opened from a trusted context.
    if identity.id != "vak"
        && active.project_config_trusted()
        && !vak_core::trust::is_trusted(workspace)
    {
        let _ = vak_core::trust::mark_trusted(workspace);
    }
    let core = if workspace == active.cwd().as_path() {
        active.clone()
    } else {
        let resolved =
            match state
                .gateway
                .core_pool
                .resolve_at(workspace, None, std::time::Instant::now())
            {
                Ok(core) => core,
                // `resolve_at` failing (a transient permission-ceiling recheck
                // error on a cache hit, or `Core::new_with_trust`'s own IO/config
                // error) is not itself a trust decision — falling back to an
                // unconditional `true` here would let an operator-declined
                // workspace's hooks/MCP servers/secret scope apply anyway, exactly the
                // bypass `vak_core::trust` exists to close. Recompute trust the
                // same way `resolve_at` does rather than assuming it.
                Err(_) => vak_core::Core::new_with_trust(
                    workspace.to_path_buf(),
                    vak_core::trust::is_trusted(workspace),
                )?,
            };
        // A freshly-resolved Core has its own default sessions/data home
        // (real on-disk `data_home()`), which would silently diverge from
        // wherever this app/process's data actually lives if the active
        // Core was pointed at a non-default root (test isolation, or a
        // future custom data-home setting). Every agent's data must live
        // under the *same* root, just in its own agent-scoped subdirectory
        // (`Core::sessions_home` already layers that on top).
        resolved.set_shared_scope(vak_config::scope::SharedScope::new(
            active.shared_scope().into_root(),
        ));
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
    Ok(core.with_agent_identity(Some(identity.clone())))
}

#[derive(Deserialize, Default)]
pub(crate) struct OpenAgentRequest {
    #[serde(default)]
    create_new: bool,
}

/// One resolved conversation, cached so a repeated open (every reload, tab
/// switch, reconnect — this endpoint is hit constantly) can skip the
/// admission lock and directory scan entirely.
#[derive(Clone)]
struct CachedAgentSession {
    session_id: String,
    ledger_path: std::path::PathBuf,
    agent: AgentIdentity,
}

/// Keyed by (workspace, conversation id) rather than just the Agent id: the
/// built-in `vak` agent alone can have one conversation per workspace, and
/// `create_new` mints a fresh conversation id every time so it never
/// collides with — or evicts — the durable one. Revalidated by file
/// existence on every read (below), so a stale entry pointing at a deleted
/// ledger is never trusted, only discarded.
static AGENT_SESSION_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<(std::path::PathBuf, String), CachedAgentSession>>,
> = std::sync::OnceLock::new();

fn agent_session_cache() -> &'static std::sync::Mutex<
    std::collections::HashMap<(std::path::PathBuf, String), CachedAgentSession>,
> {
    AGENT_SESSION_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Whether a ledger has any entry beyond its header; stops at the second.
fn has_entries_beyond_header(path: &std::path::Path) -> bool {
    let mut seen = 0;
    vak_session::SessionLog::scan(path, |_| {
        seen += 1;
        seen < 2
    }) >= 2
}

/// The directory scan that used to run inline on the async handler
/// (blocking `std::fs` calls, one `header()` read per ledger). Runs inside
/// `spawn_blocking`; an unreadable or empty ledger is skipped with a
/// warning rather than failing the whole request — a fleet of conversations
/// must not go dark because one file next to them is corrupt.
fn scan_candidates(
    dir: std::path::PathBuf,
    cwd: std::path::PathBuf,
    agent_id: String,
    conversation: ConversationContext,
    trashed: std::collections::HashSet<String>,
) -> Vec<(SessionHeader, bool)> {
    let mut candidates = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return candidates;
    };
    for entry in entries.flatten() {
        if vak_config::scope::ledger_session_id(&entry.path()).is_none() {
            continue;
        }
        let h = match header(&entry.path()) {
            Ok(h) => h,
            Err(e) => {
                eprintln!(
                    "[agents] skipping unreadable session ledger {}: {e}",
                    entry.path().display()
                );
                continue;
            }
        };
        if h.cwd == cwd
            && !trashed.contains(&h.session_id)
            && h.parent_session_id.is_none()
            && h.agent.as_ref().is_some_and(|a| a.id == agent_id)
            && h.conversation.as_ref() == Some(&conversation)
        {
            let has_content = has_entries_beyond_header(&entry.path());
            candidates.push((h, has_content));
        }
    }
    candidates
}

fn opened_response(session_id: &str, agent: &AgentIdentity, core: &vak_core::Core) -> Response {
    Json(serde_json::json!({"session_id": session_id, "agent": agent, "cwd": core.cwd()}))
        .into_response()
}

/// Reopen an already-known session into the live handle map if it is not
/// there already. `Err` means the ledger could not actually be (re)opened
/// even though its file exists (e.g. removed a moment ago, or held
/// exclusively elsewhere in a way `open_session_read_only` also refuses) —
/// callers either fall back to a fresh scan or surface the reason, but
/// never silently answer 200 for a session that was not actually attached.
async fn ensure_registered(
    state: &AppState,
    core: &vak_core::Core,
    session_id: &str,
) -> Result<(), String> {
    if state.get(session_id).is_some() {
        return Ok(());
    }
    let session = match core.open_session(session_id).await {
        Ok(session) => session,
        Err(vak_core::CoreError::Session(vak_session::SessionError::Locked(_))) => {
            match core.open_session_read_only(session_id).await {
                Ok(session) => session,
                Err(e) => return Err(format!("session is locked elsewhere: {e}")),
            }
        }
        Err(e) => return Err(e.to_string()),
    };
    register_handle(
        state,
        session_id.to_string(),
        session,
        core.cwd().clone(),
        core.clone(),
    );
    Ok(())
}

/// Opening an existing Agent conversation is idempotent. An explicit new
/// conversation gets its own identity and append-only session ledger.
pub(crate) async fn open(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<OpenAgentRequest>,
) -> Response {
    let (identity, core) = match resolve_agent_core(&state, &id) {
        Ok(pair) => pair,
        Err(response) => return response,
    };
    // Desktop and web share one durable local conversation per selected
    // Agent. This is deliberately derived from the Agent identity, not from
    // browser storage or a transient session id, so either local client
    // resumes the same ledger while another Agent gets an independent one.
    let conversation = ConversationContext {
        conversation_id: if request.create_new {
            format!("agent:{}:local:{}", identity.id, uuid::Uuid::now_v7())
        } else {
            format!("agent:{}:local", identity.id)
        },
        audience_id: "local".into(),
        origin: Some(ConversationOrigin {
            surface: "desktop".into(),
            address: "local".into(),
            bot_id: None,
        }),
    };
    let core = core.with_conversation_context(Some(conversation.clone()));
    let cache_key = (core.cwd().clone(), conversation.conversation_id.clone());

    let cached = agent_session_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&cache_key)
        .cloned();
    if let Some(cached) = cached
        && cached.ledger_path.exists()
    {
        if ensure_registered(&state, &core, &cached.session_id)
            .await
            .is_ok()
        {
            return opened_response(&cached.session_id, &cached.agent, &core);
        }
        // The ledger existed a moment ago but could not actually be
        // (re)opened; drop the stale entry and fall through to a fresh scan.
        agent_session_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&cache_key);
    }

    let dir = vak_session::SessionPath::sessions_dir(&core.scope().into_root(), core.cwd());
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
    let mut candidates = match tokio::task::spawn_blocking({
        let dir = dir.clone();
        let cwd = core.cwd().clone();
        let agent_id = identity.id.clone();
        let conversation = conversation.clone();
        let trashed = vak_core::trash::trashed(&core.shared_scope());
        move || scan_candidates(dir, cwd, agent_id, conversation, trashed)
    })
    .await
    {
        Ok(candidates) => candidates,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    candidates.sort_by_key(|(h, has_content)| (*has_content, h.created_at));
    if let Some((h, _)) = candidates.last() {
        let sid = h.session_id.clone();
        if let Err(e) = ensure_registered(&state, &core, &sid).await {
            return error(StatusCode::CONFLICT, e);
        }
        // The identity as admitted, for display: a turn resolves the current
        // definition itself (`Core::live_agent_identity`), so editing an
        // Agent reaches its next turn, while this header snapshot records
        // what the conversation was admitted with (AGENTS.md invariant 37).
        let admitted = h.agent.clone().unwrap_or_else(|| identity.clone());
        agent_session_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                cache_key,
                CachedAgentSession {
                    session_id: sid.clone(),
                    ledger_path: vak_config::scope::session_ledger(&dir, &sid),
                    agent: admitted.clone(),
                },
            );
        return opened_response(&sid, &admitted, &core);
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
    agent_session_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            cache_key,
            CachedAgentSession {
                session_id: sid.clone(),
                ledger_path: vak_config::scope::session_ledger(&dir, &sid),
                agent: identity.clone(),
            },
        );
    opened_response(&sid, &identity, &core)
}
