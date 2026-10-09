//! Admin console API endpoints (docs/design/29-personal-os.md): session
//! catalog, transcripts, live event SSE, security audit log, and store
//! management. Mounted under `/admin/api` in `router_with_state`.

use std::path::PathBuf;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::AppState;

pub(crate) const SESSION_COOKIE: &str = "vak_session";

pub(crate) async fn admin_traffic_status() -> Json<vak_llm::TrafficSnapshot> {
    Json(vak_llm::RateLimitGate::traffic_snapshot())
}

// ---- GET /admin/api/sessions ----------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct SessionListQuery {
    pub limit: Option<usize>,
    pub space: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SessionListItem {
    pub session_id: String,
    pub space_id: String,
    pub entry_count: usize,
    pub first_ts: String,
    pub last_ts: String,
    /// From the shared `archive.json` (keyed by session id, not scoped to a
    /// workspace — the same map `/sessions/{id}/archive` reads and writes).
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

pub(crate) async fn list_sessions_admin(
    State(state): State<AppState>,
    Query(q): Query<SessionListQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    // The archive is keyed by session id across every project; the trash
    // hides a session everywhere (`vak_core::trash`).
    let archive_map = crate::read_archive(&state.core);
    let core = state.core.clone();
    let listed = tokio::task::spawn_blocking(move || {
        let catalog = core.catalog()?;
        catalog.catch_up()?;
        let audience = vak_catalog::Audience {
            exclude_sessions: vak_core::trash::trashed(&core.shared_scope()),
            ..Default::default()
        };
        Ok::<_, vak_core::CoreError>(catalog.list("session", &audience, 10_000)?)
    })
    .await;
    let sessions = match listed {
        Ok(Ok(sessions)) => sessions,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let mut visible: Vec<vak_catalog::Node> = sessions
        .into_iter()
        .filter(|node| {
            let agent = node.agent_name.as_deref().unwrap_or("vak");
            q.space
                .as_ref()
                .is_none_or(|space| node.space.as_ref() == Some(space))
                && q.agent.as_ref().is_none_or(|a| a == "all" || agent == a)
        })
        .collect();
    visible.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let total = visible.len();
    let items: Vec<SessionListItem> = visible
        .into_iter()
        .take(limit)
        .map(|node| {
            let session_id = node.id.trim_start_matches("ses_").to_string();
            SessionListItem {
                archived: archive_map.get(&session_id).copied().unwrap_or(false),
                session_id,
                space_id: node.space.unwrap_or_default(),
                entry_count: node.size.unwrap_or_default().max(0) as usize,
                first_ts: node.created_at.unwrap_or_default(),
                last_ts: node.updated_at.unwrap_or_default(),
                agent_id: Some(node.agent_name.unwrap_or_else(|| "vak".into())),
            }
        })
        .collect();
    Json(serde_json::json!({
        "sessions": items,
        "total": total,
        // The project this server opened, so the console can tell its
        // sessions from other projects'.
        "workspace_space_id": vak_config::spaces::key(state.core.cwd()),
    }))
    .into_response()
}

// ---- GET /admin/api/approvals ----------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct PendingApproval {
    pub session_id: String,
    pub request_id: String,
    pub tool: String,
    pub args_json: String,
    pub reason: String,
    pub requested_at: String,
}

/// Aggregated pending approval gates across every live session. Answering
/// still goes through the per-session endpoint — listing never crosses a
/// session's approval scope, only displays it.
pub(crate) async fn list_pending_approvals(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let mut items: Vec<PendingApproval> = Vec::new();
    for handle in state.live_handles() {
        let pending = handle
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for req in pending.values() {
            items.push(PendingApproval {
                session_id: handle.id.clone(),
                request_id: req.id.clone(),
                tool: req.tool.clone(),
                args_json: truncate_chars(&req.args_json, 400),
                reason: req.reason.clone(),
                requested_at: req.requested_at.to_rfc3339(),
            });
        }
    }
    // Oldest first — the gate that has been waiting longest is the most urgent.
    items.sort_by(|a, b| a.requested_at.cmp(&b.requested_at));
    let total = items.len();
    Json(serde_json::json!({ "approvals": items, "total": total }))
}

// ---- GET /admin/api/questions ----------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct PendingWorkerQuestion {
    pub session_id: String,
    pub question_id: String,
    pub worker: String,
    pub question: String,
    pub options: Vec<String>,
    pub asked_at: String,
}

/// Worker questions waiting on an answer, across every live session. Read
/// only: a question is answered in its own conversation, by the approver chat
/// or at the terminal (docs/design/84-worker-questions-and-control.md §4.5),
/// so this lists and links and never answers.
pub(crate) async fn list_pending_questions(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let mut items: Vec<PendingWorkerQuestion> = Vec::new();
    for handle in state.live_handles() {
        for question in handle.core.workers().questions().pending(&handle.id) {
            items.push(PendingWorkerQuestion {
                session_id: handle.id.clone(),
                question_id: question.id,
                worker: question.label,
                question: question.question,
                options: question.options,
                asked_at: question.asked_at.to_rfc3339(),
            });
        }
    }
    // Oldest first: a worker that has waited longest is the most blocked.
    items.sort_by(|a, b| a.asked_at.cmp(&b.asked_at));
    let total = items.len();
    Json(serde_json::json!({ "questions": items, "total": total }))
}

// ---- GET /admin/api/sessions/:id/transcript --------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct TranscriptQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub kind: Option<String>,
    pub role: Option<String>,
}

/// Byte-safe truncation: never splits a multi-byte UTF-8 sequence.
fn truncate_chars(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Which ledger entries of a session are runtime-authored control messages,
/// by entry id (empty when the session cannot be read).
fn control_kinds_by_entry(
    state: &AppState,
    session_id: &str,
) -> std::collections::HashMap<String, vak_intent::control::ControlKind> {
    let from = |session: &vak_session::SessionLog| {
        session
            .derive_transcript()
            .into_iter()
            .filter_map(|item| item.control.map(|kind| (item.entry_id, kind)))
            .collect()
    };
    if let Some(handle) = state.get(session_id)
        && let Some(session) = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
    {
        return from(session);
    }
    crate::open_historical_session(state, session_id)
        .map(|session| from(&session))
        .unwrap_or_default()
}

pub(crate) async fn session_transcript_admin(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(q): Query<TranscriptQuery>,
) -> Json<serde_json::Value> {
    if vak_core::trash::is_trashed(&state.core.shared_scope(), &session_id) {
        return Json(serde_json::json!({ "error": "session is in the trash" }));
    }
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let offset = q.offset.unwrap_or(0);
    let Some(ledger) = crate::session_ledger_dir(&state, &session_id).await else {
        return Json(serde_json::json!({ "error": "no such session" }));
    };
    // The ledger is read as it is on disk, without its writer's lock, so a
    // conversation another process is serving can still be inspected.
    let (kind, role) = (q.kind.clone(), q.role.clone());
    let rows = tokio::task::spawn_blocking(move || {
        let mut rows = Vec::new();
        let mut calls = std::collections::HashMap::new();
        vak_session::SessionLog::scan(&ledger, |entry| {
            if let Some(row) = entry.and_then(|entry| forensics_row(entry, &mut calls))
                && kind.as_deref().is_none_or(|kind| row.kind == kind)
                && role
                    .as_deref()
                    .is_none_or(|role| row.role.as_deref() == Some(role))
            {
                rows.push(row);
            }
            true
        });
        rows
    })
    .await
    .unwrap_or_default();
    let total = rows.len();
    let has_more = offset.saturating_add(limit) < total;
    // A runtime-authored row is tagged from the ledger's own typed marker.
    let controls = control_kinds_by_entry(&state, &session_id);
    let page: Vec<serde_json::Value> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|row| {
            serde_json::json!({
                "control": controls.get(&row.entry_id),
                "entry_id": row.entry_id,
                "ts": row.ts,
                "kind": row.kind,
                "role": row.role,
                "tool_name": row.tool_name,
                "is_error": row.is_error,
                "content": truncate_chars(&row.content, 16000),
            })
        })
        .collect();
    let contract = state
        .get(&session_id)
        .and_then(|handle| {
            handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .and_then(|session| session.header().map(|header| header.contract.clone()))
        })
        .or_else(|| {
            crate::open_historical_session(&state, &session_id)
                .and_then(|session| session.header().map(|header| header.contract.clone()))
        });
    Json(serde_json::json!({
        "session_id": session_id,
        "entries": page,
        "offset": offset,
        "total": total,
        "has_more": has_more,
        "contract": contract,
        // Per-turn routing: a contract snapshot is never a mismatch.
        "configuration_mismatch": false,
    }))
}

/// One ledger entry as the forensics transcript lists it.
struct ForensicsRow {
    entry_id: String,
    ts: String,
    kind: &'static str,
    role: Option<String>,
    tool_name: Option<String>,
    content: String,
    is_error: bool,
}

/// What the forensics transcript shows of `entry`, or `None` for entries
/// it does not list (capability bindings, evidence bodies, presentations,
/// call effects). `calls` remembers each call's tool, so a result row
/// names the tool that produced it.
fn forensics_row(
    entry: &vak_session::Entry,
    calls: &mut std::collections::HashMap<String, String>,
) -> Option<ForensicsRow> {
    use vak_session::EntryPayload;
    let row = |kind: &'static str, role: Option<&str>, content: String| ForensicsRow {
        entry_id: entry.id.clone(),
        ts: entry.ts.to_rfc3339(),
        kind,
        role: role.map(str::to_string),
        tool_name: None,
        content,
        is_error: false,
    };
    Some(match &entry.payload {
        EntryPayload::Header(_) => row("header", None, String::new()),
        EntryPayload::Message(record) => {
            // A runtime-authored nudge is listed under its own role so it
            // is never read as something the user said.
            let role = match record.message.role {
                _ if record.control_kind().is_some() => "control",
                vak_llm::Role::User => "user",
                vak_llm::Role::Assistant => "assistant",
            };
            let mut parts = Vec::new();
            let mut tool_name = None;
            let mut is_error = false;
            for block in &record.message.content {
                match block {
                    vak_llm::ContentBlock::Text { text } => parts.push(text.clone()),
                    vak_llm::ContentBlock::Thinking { text, .. } => {
                        parts.push(format!("[thinking] {text}"))
                    }
                    vak_llm::ContentBlock::ToolUse { id, name, input } => {
                        calls.insert(id.clone(), name.clone());
                        tool_name = Some(name.clone());
                        match input.get("command").and_then(|value| value.as_str()) {
                            Some(command) => parts.push(format!("[tool:{name}] {command}")),
                            None => parts.push(format!("[tool:{name}]")),
                        }
                    }
                    vak_llm::ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error: error,
                    } => {
                        is_error = *error;
                        if let Some(name) = calls.get(tool_use_id) {
                            tool_name = Some(name.clone());
                        }
                        if !content.trim().is_empty() {
                            parts.push(format!("[result] {content}"));
                        }
                    }
                    _ => {}
                }
            }
            ForensicsRow {
                tool_name,
                is_error,
                ..row("message", Some(role), parts.join("\n"))
            }
        }
        EntryPayload::Compaction(compaction) => row("compaction", None, compaction.summary.clone()),
        EntryPayload::Receipt(_) => row("receipt", None, String::new()),
        EntryPayload::Goal(goal) => row(
            "goal",
            None,
            format!("{} {}", goal.objective, goal.criteria.join(" ")),
        ),
        EntryPayload::GoalUpdate(update) => row(
            "goal",
            Some("system"),
            format!("{:?} {}", update.relation, update.request),
        ),
        EntryPayload::Activity(activity) => ForensicsRow {
            tool_name: activity.data.get("tool").cloned(),
            is_error: matches!(
                activity.status,
                vak_session::ActivityStatus::Failed | vak_session::ActivityStatus::Denied
            ),
            ..row(
                "activity",
                Some("system"),
                format!(
                    "{} {}",
                    activity.label,
                    activity.detail.as_deref().unwrap_or_default()
                ),
            )
        },
        EntryPayload::Work(work) => row(
            "work",
            Some("system"),
            serde_json::to_string(&work.kind).unwrap_or_default(),
        ),
        EntryPayload::TurnCard(record) => row(
            "turn_card",
            Some("turn"),
            format!("{} {}", record.card.asked, record.card.answered.narration),
        ),
        EntryPayload::Intent(record) => row(
            "intent",
            Some("system"),
            record.model_visible.clone().unwrap_or_default(),
        ),
        _ => return None,
    })
}

// ---- GET /admin/api/events (SSE) ------------------------------------------

pub(crate) async fn admin_events_sse(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::wrappers::BroadcastStream;

    let rx = state.hub.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(event) => {
            let json = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().data(json)))
        }
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            let lagged = serde_json::json!({
                "type": "Lagged",
                "data": { "missed": n }
            });
            let json = serde_json::to_string(&lagged).unwrap_or_default();
            Some(Ok(Event::default().data(json)))
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---- GET /admin/api/security -----------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct SecurityQuery {
    pub limit: Option<usize>,
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct SecurityEventEntry {
    ts: String,
    kind: String,
    label: String,
    detail: String,
    ip: Option<String>,
}

pub(crate) async fn list_security_events(
    State(state): State<AppState>,
    Query(q): Query<SecurityQuery>,
) -> Json<serde_json::Value> {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let home = state.core.scope().into_root();
    let events = vak_core::security_events::list(&vak_config::scope::AgentScope::new(&home), limit);
    let filtered: Vec<SecurityEventEntry> = events
        .into_iter()
        .filter(|e| {
            q.kind.as_deref().is_none_or(|k| {
                serde_json::to_value(&e.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .as_deref()
                    == Some(k)
            })
        })
        .map(|e| SecurityEventEntry {
            ts: e.ts.to_rfc3339(),
            kind: serde_json::to_value(&e.kind)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "unknown".into()),
            label: e.label,
            detail: e.detail,
            ip: e.ip,
        })
        .collect();
    Json(serde_json::json!({
        "events": filtered,
        "total": filtered.len(),
    }))
}

// ---- GET /admin/api/bestofn ------------------------------------------------

#[derive(Debug, Serialize)]
struct BestOfNRun {
    session_id: String,
    repo: String,
    branch: String,
}

/// Active best-of-N candidate runs keyed by child session.
pub(crate) async fn list_bestofn(State(state): State<AppState>) -> Json<serde_json::Value> {
    let map = state
        .best_runs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let runs: Vec<BestOfNRun> = map
        .iter()
        .map(|(id, meta)| BestOfNRun {
            session_id: id.clone(),
            repo: meta.repo.display().to_string(),
            branch: meta.branch.clone(),
        })
        .collect();
    let total = runs.len();
    Json(serde_json::json!({ "runs": runs, "total": total }))
}

// ---- GET /admin/api/config ------------------------------------------------

pub(crate) async fn get_config_admin(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<crate::AgentScopeQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    // Report the selected Agent's effective settings when the admin scope is
    // agent-specific. With no agent, retain the shared server Core view.
    let core = if let Some(agent) = query.agent.as_deref() {
        match crate::resolve_scoped_core(&state, None, Some(agent)) {
            Ok(core) => core,
            Err(response) => return response,
        }
    } else {
        crate::refresh_control_plane(&state);
        state.core.clone()
    };
    let route = core.effective_route();
    let cfg = core.config();
    let work = core.effective_work();
    // The lists the engine actually evaluates, not the ones loaded at
    // startup: `PUT /config/permissions` changes them without a restart, and
    // a console showing the stale set would be reporting rules no run uses.
    let permission_rules = core.effective_permission_rules();
    Json(serde_json::json!({
        "provider": route.provider,
        "model": route.model,
        "provider_source": route.provider_source,
        "model_source": route.model_source,
        "route_revision": route.revision,
        "max_turns": core.effective_max_turns(),
        "permission_mode": format!("{:?}", core.effective_permission_mode()),
        // These three were consumed by the console and never sent. The
        // console's `ConfigInfo` declared all of them, so nothing caught it:
        // the approval-mode picker could not show which mode was in force,
        // "Effective sandbox" rendered its loading placeholder forever, and
        // the workers toggle rendered unchecked whatever the real value
        // was — so the first click wrote the opposite of what was displayed.
        "approval_mode": core.effective_approval_mode().as_str(),
        "sandbox": core.effective_sandbox_name(),
        "workers": core.effective_workers(),
        "theme": core.effective_theme(),
        "voice": {
            "enabled": core.effective_voice().enabled,
            "max_session_secs": core.effective_voice().max_session_secs,
            "max_concurrent": core.effective_voice().max_concurrent,
            "max_audio_bytes": core.effective_voice().max_audio_bytes,
            // Keep quota semantics explicit for operators. These values are
            // the live workspace admission limits; narrower bot/chat pins
            // are reported by their binding endpoints and never merged here.
            "quota": {
                "session_seconds": core.effective_voice().max_session_secs,
                "concurrent_sessions": core.effective_voice().max_concurrent,
                "inbound_audio_bytes": core.effective_voice().max_audio_bytes,
                "scope": "workspace",
                "source": "effective",
            },
            "source": "effective",
        },
        "work": {
            "enabled": work.enabled,
            "default_mode": work.default_mode,
            "max_items": work.max_items,
            "max_revisions": work.max_revisions,
            "max_parallel": work.max_parallel,
            "confirmation": work.confirmation,
        },
        "memory": {
            "search_enabled": cfg.memory.search_enabled,
            "write_enabled": cfg.memory.write_enabled,
            "skill_proposals": cfg.memory.skill_proposals,
            "reflection": cfg.memory.reflection,
        },
        // The session list at `/admin/api/sessions` spans every project the
        // store indexes, but archive/delete/run/steer on a session only
        // reach a ledger file under *this* process's own workspace
        // (`sessions_home/sessions/<hash(cwd)>/`). This is that same hash,
        // matching `space_id` on each session row — the console uses it
        // to tell which rows those actions can actually reach.
        "workspace_space_id": vak_config::spaces::key(core.cwd()),
        // The resolved rule lists the permission engine actually evaluates
        // (vak_permission::Rule syntax: `Tool`, `Tool(glob)`, with a
        // `+`/`?`/`-` prefix for allow/ask/deny). The admin console shows
        // these verbatim and derives per-extension scope from them, so a
        // reader can see what an MCP server or a hook is permitted to do
        // rather than only that it is configured.
        "permissions": {
            "allow": permission_rules.0,
            "ask": permission_rules.1,
            "deny": permission_rules.2,
        },
    }))
    .into_response()
}

// ---- GET /admin/api/gateway/status ----------------------------------------

pub(crate) async fn gateway_status_admin(State(state): State<AppState>) -> Json<serde_json::Value> {
    let gw = &state.gateway;
    crate::refresh_control_plane(&state);
    let default_route = state.core.effective_route();
    let mut bindings = Vec::new();
    let mut bound_targets = std::collections::HashSet::new();
    for (target, binding) in gw.bindings_snapshot() {
        bound_targets.insert(target.clone());
        let configured_workspace = gw
            .configured_space(&target)
            .and_then(|space| crate::gateway::space_folder(&space));
        let effective_workspace = configured_workspace
            .clone()
            .or_else(|| {
                binding
                    .space
                    .as_deref()
                    .and_then(crate::gateway::space_folder)
            })
            .unwrap_or_else(|| state.core.cwd().to_path_buf());
        let channel_override = binding.provider.clone().zip(binding.model.clone());
        let (provider, model, source, revision) = match &channel_override {
            Some((provider, model)) => (
                provider.clone(),
                model.clone(),
                "channel_override",
                format!("channel:{provider}:{model}"),
            ),
            None => (
                default_route.provider.clone(),
                default_route.model.clone(),
                "workspace_default",
                default_route.revision.clone(),
            ),
        };
        let contract = binding.session_id.as_ref().and_then(|session_id| {
            state
                .get(session_id)
                .and_then(|handle| {
                    handle
                        .session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .as_ref()
                        .and_then(|session| session.header().cloned())
                })
                .or_else(|| crate::read_historical_header(&state, session_id))
        });
        let stale_reasons = contract
            .as_ref()
            .map(|header| {
                let mut reasons = Vec::new();
                if header.cwd != effective_workspace {
                    reasons.push("workspace_changed");
                }
                // Per-turn routing: provider/model are resolved fresh each turn
                // from effective_route(), so the header's initial snapshot no
                // longer constitutes a stale reason. WorkReceipt records actual
                // per-turn dispatch for audit.
                reasons
            })
            .unwrap_or_else(|| {
                binding
                    .session_id
                    .as_ref()
                    .map(|_| vec!["session_missing"])
                    .unwrap_or_default()
            });
        let paused = binding
            .session_id
            .as_deref()
            .and_then(|session_id| state.get(session_id))
            .is_some_and(|handle| handle.steering.is_paused());
        bindings.push(serde_json::json!({
            "target": target,
            "session_id": binding.session_id,
            "workspace": effective_workspace,
            "configured_workspace": configured_workspace,
            "override": channel_override.map(|(provider, model)| serde_json::json!({
                "provider": provider,
                "model": model,
            })),
            "effective_route": {
                "provider": provider,
                "model": model,
                "source": source,
                "revision": revision,
            },
            "session_contract": contract.map(|header| serde_json::json!({
                "provider": header.contract.provider,
                "model": header.contract.model,
                "workspace": header.cwd,
                "app_version": header.contract.app_version,
            })),
            "stale": !stale_reasons.is_empty(),
            "stale_reasons": stale_reasons,
            "paused": paused,
        }));
    }
    // An approved channel does not acquire a runtime binding until its first
    // accepted message creates a session. Keep that approved-but-cold channel
    // visible in the same table so the Admin UI does not claim it disappeared.
    for entry in gw.allowlist_snapshot() {
        if entry.status != crate::gateway::AllowlistStatus::Allowed
            || bound_targets.contains(&entry.key)
        {
            continue;
        }
        let channel_override = entry
            .route
            .as_ref()
            .map(|route| (route.provider.clone(), route.model.clone()));
        let (provider, model, source, revision) = match &channel_override {
            Some((provider, model)) => (
                provider.clone(),
                model.clone(),
                "channel_override",
                format!("channel:{provider}:{model}"),
            ),
            None => (
                default_route.provider.clone(),
                default_route.model.clone(),
                "workspace_default",
                default_route.revision.clone(),
            ),
        };
        let workspace = gw.workspace_for_entry(&state.core, &entry.key);
        bindings.push(serde_json::json!({
            "target": entry.key,
            "session_id": null,
            "workspace": workspace.as_ref().ok(),
            "workspace_error": workspace.as_ref().err(),
            "configured_workspace": crate::gateway::space_folder_json(gw.configured_space(&entry.key).as_deref()),
            "override": channel_override.map(|(provider, model)| serde_json::json!({
                "provider": provider,
                "model": model,
            })),
            "effective_route": {
                "provider": provider,
                "model": model,
                "source": source,
                "revision": revision,
            },
            "session_contract": null,
            "stale": false,
            "stale_reasons": [],
        }));
    }
    bindings.sort_by(|a, b| a["target"].as_str().cmp(&b["target"].as_str()));
    // docs/design/34 Phase 2: which workspaces currently have a pooled Core
    // running (warm) vs. cold (will lazily start on next inbound message),
    // so the workspace picker in the approve flow isn't guessing.
    let now = std::time::Instant::now();
    let core_pool: Vec<serde_json::Value> = gw
        .core_pool
        .snapshot_at(now)
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "workspace": entry.workspace,
                "is_default": entry.is_default,
                "state": "warm",
                "idle_secs": entry.idle_secs,
                "permission_override": entry.permission_override,
                "effective_permission_mode": entry.effective_permission_mode,
            })
        })
        .collect();
    Json(serde_json::json!({
        "enabled": gw.enabled,
        "workspace": state.core.cwd(),
        "canonical_default_workspace": vak_config::paths::default_workspace(),
        "default_route": default_route,
        "bindings": bindings,
        "chat_allowlist": state.core.config().gateway.chat_allowlist,
        "chat_allowlist_open": state.core.config().gateway.chat_allowlist_open,
        "core_pool": {
            "max": state.core.config().gateway.core_pool_max,
            "idle_secs": state.core.config().gateway.core_pool_idle_secs,
            "entries": core_pool,
        },
        // docs/design/34 open question 4: the workspace field is a picker
        // of workspaces vak has actually run in, not unconstrained free
        // text — a typo'd path is caught at entry time by offering
        // known-good options first.
        "known_workspaces": known_workspaces(&state),
        "workspace_catalog": workspace_catalog(&state),
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct GatewayWorkspaceBody {
    pub workspace: Option<String>,
}

pub(crate) async fn patch_gateway_workspace(
    State(state): State<AppState>,
    Json(body): Json<GatewayWorkspaceBody>,
) -> Response {
    let workspace = body
        .workspace
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let selected = match workspace {
        None => None,
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() || !path.is_dir() {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "workspace must be an existing absolute directory"
                    })),
                )
                    .into_response();
            }
            match std::fs::canonicalize(path) {
                Ok(path) => Some(path),
                Err(error) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({ "error": error.to_string() })),
                    )
                        .into_response();
                }
            }
        }
    };
    let data_home = state.core.scope().into_root();
    if let Err(error) =
        vak_config::paths::persist_gateway_workspace_at(&data_home, selected.as_deref())
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response();
    }
    let effective = vak_config::paths::gateway_workspace_at(
        &data_home,
        &vak_config::paths::default_workspace(),
    );
    state.hub.emit_config_changed(
        "gateway_workspace_changed",
        &effective.display().to_string(),
    );
    Json(serde_json::json!({
        "workspace": effective,
        "restart_required": effective.as_path() != state.core.cwd().as_path(),
    }))
    .into_response()
}

/// Configure › Projects (doc 74 A13): every space this machine knows, with
/// its folder here, its name and trust. "Project" is the everyday word for
/// a space (doc 75 §7).
async fn list_projects() -> Json<serde_json::Value> {
    let projects: Vec<serde_json::Value> = vak_config::spaces::all()
        .into_iter()
        .map(|space| {
            let name = space.name.clone().or_else(|| {
                space
                    .folder
                    .as_deref()
                    .and_then(std::path::Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
            });
            serde_json::json!({
                "id": space.id,
                "name": name,
                "folder": space.folder,
                "folder_here": space.folder.as_deref().is_some_and(std::path::Path::is_dir),
                "trusted": space.folder.as_deref().is_some_and(vak_core::trust::is_trusted),
                "hidden": space.forgotten,
                "last_opened": space.last_opened,
            })
        })
        .collect();
    Json(serde_json::json!({ "projects": projects }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ProjectPatch {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hidden: Option<bool>,
    /// The project's folder on this machine, for one made on another.
    #[serde(default)]
    pub folder: Option<String>,
}

async fn patch_project(
    Path(id): Path<String>,
    Json(body): Json<ProjectPatch>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if let Some(name) = body.name.as_deref() {
        let name = name.trim();
        if name.is_empty() || name.len() > 80 {
            return Err((
                StatusCode::BAD_REQUEST,
                "a project name is 1–80 characters".into(),
            ));
        }
        vak_config::spaces::rename(&id, name).map_err(|e| (StatusCode::NOT_FOUND, e))?;
    }
    if let Some(hidden) = body.hidden {
        vak_config::spaces::set_forgotten(&id, hidden).map_err(|e| (StatusCode::NOT_FOUND, e))?;
    }
    if let Some(folder) = body.folder.as_deref() {
        vak_core::workspaces::place_project(&id, std::path::Path::new(folder.trim()))
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    }
    Ok(Json(serde_json::json!({ "id": id })))
}

/// The workspaces the console offers: the default, the gateway's own, the
/// current one, every pooled one, then every space this machine has opened
/// (`vak_config::spaces`), most recent first, minus forgotten ones.
fn known_workspaces(state: &AppState) -> Vec<String> {
    let mut seen: Vec<String> = vec![vak_config::paths::default_workspace().display().to_string()];
    let gateway_workspace = vak_config::paths::gateway_workspace_at(
        &state.core.scope().into_root(),
        &vak_config::paths::default_workspace(),
    );
    let gateway_workspace_text = gateway_workspace.display().to_string();
    if gateway_workspace_text != seen[0] {
        seen.push(gateway_workspace_text);
    }
    let current = state.core.cwd().display().to_string();
    if !seen.contains(&current) {
        seen.push(current);
    }
    let mut push = |path: String| {
        if !path.is_empty() && !seen.contains(&path) && !is_scratch_workspace(&path) {
            seen.push(path);
        }
    };
    for entry in state
        .gateway
        .core_pool
        .snapshot_at(std::time::Instant::now())
    {
        push(entry.workspace.display().to_string());
    }
    for space in vak_config::spaces::all() {
        if !space.forgotten
            && let Some(folder) = space.folder
        {
            push(folder.display().to_string());
        }
    }
    seen
}

pub(crate) fn workspace_catalog(state: &AppState) -> Vec<serde_json::Value> {
    known_workspaces(state)
        .into_iter()
        .map(|path| {
            let folder = std::path::Path::new(&path);
            let named = vak_config::spaces::bound_space(folder).and_then(|id| {
                vak_config::spaces::all()
                    .into_iter()
                    .find(|space| space.id == id)
                    .and_then(|space| space.name)
            });
            let fallback = folder
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(&path)
                .to_string();
            serde_json::json!({
                "path": path,
                "name": named.unwrap_or(fallback),
            })
        })
        .collect()
}

/// True for a path that's almost certainly test/build scratch rather than
/// a real project — OS temp dirs and the `tempfile` crate's `.tmpXXXXXX`
/// directory naming convention (used throughout this workspace's own test
/// suite, which is exactly what was polluting `known_workspaces` with
/// dozens of one-shot `cargo test` tempdirs on a dev machine). A path
/// under a real project that happens to be named `tmp` is not excluded by
/// this — only OS scratch roots and the tempfile-style random suffix are.
fn is_scratch_workspace(path: &str) -> bool {
    if path == "/tmp"
        || path.starts_with("/tmp/")
        || path == "/private/tmp"
        || path.starts_with("/private/tmp/")
        || path.starts_with("/var/folders/")
        || path.starts_with("/private/var/folders/")
    {
        return true;
    }
    // A user-created Agent's workspace lives in the tenant tree (see
    // `vak_config::paths::agent_workspace`): an implementation detail, not a
    // project a person would recognize or want to switch into.
    if vak_config::paths::agent_workspace_space(std::path::Path::new(path)).is_some() {
        return true;
    }
    // Windows temp dirs: %TEMP%, %TMP%, C:\Windows\Temp, C:\Temp.
    // `std::env::temp_dir()` returns the OS canonical temp on every platform,
    // so checking it catches redirected/user-specific temp roots that a
    // literal path match would miss (e.g. on a managed Windows account).
    #[cfg(windows)]
    {
        let temp = std::env::temp_dir();
        if let Ok(temp_str) = temp.into_os_string().into_string() {
            if path == temp_str || path.starts_with(&format!("{}\\", temp_str)) {
                return true;
            }
        }
        if path.eq_ignore_ascii_case(r"C:\Windows\Temp")
            || path.starts_with(r"C:\Windows\Temp\")
            || path.eq_ignore_ascii_case(r"C:\Temp")
            || path.starts_with(r"C:\Temp\")
        {
            return true;
        }
    }
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with(".tmp"))
}

#[derive(serde::Deserialize)]
pub(crate) struct GatewayRoutePatch {
    provider: Option<String>,
    model: Option<String>,
}

pub(crate) async fn patch_gateway_binding(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<GatewayRoutePatch>,
) -> StatusCode {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST;
    }
    let route = match (body.provider, body.model) {
        (None, None) => None,
        (Some(provider), Some(model))
            if !provider.trim().is_empty() && !model.trim().is_empty() =>
        {
            Some((provider.trim().to_string(), model.trim().to_string()))
        }
        _ => return StatusCode::BAD_REQUEST,
    };
    if route.as_ref().is_some_and(|(provider, _)| {
        !state
            .core
            .provider_names()
            .iter()
            .any(|name| name == provider)
    }) {
        return StatusCode::BAD_REQUEST;
    }
    // One source of truth for "what does this channel route to"
    // (docs/design/34 "Editing an already-allowed entry"): when an
    // `allowed` allowlist entry exists for this key, the route lives there
    // and this pre-existing surface writes through to it instead of
    // parking a second, divergent value on the binding.
    let mirrored = state.gateway.allowlist_patch_route_if_allowed(
        &state.core,
        &key,
        route
            .clone()
            .map(|(provider, model)| crate::gateway::AllowlistRoute { provider, model }),
    );
    state
        .gateway
        .set_route_override(&state.core, key.clone(), route);
    if mirrored {
        state
            .hub
            .emit_config_changed("gateway_allowlist_patched", &key);
    }
    state.hub.emit_config_changed("gateway_binding_route", &key);
    StatusCode::OK
}

pub(crate) async fn rotate_gateway_binding(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> StatusCode {
    if state.gateway.rotate(&state.core, &key) {
        state
            .hub
            .emit_config_changed("gateway_binding_rotated", &key);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

pub(crate) async fn delete_gateway_binding_admin(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> StatusCode {
    if state.gateway.unbind(&state.core, &key) {
        state
            .hub
            .emit_config_changed("gateway_binding_deleted", &key);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- Allowlist (docs/design/34-channel-onboarding.md) ---------------------

/// Resolve a chat's permission exactly as `core_for_entry` does, including
/// the tiers the entry does not carry itself.
///
/// Two things were missing and both made the console read WIDER than
/// dispatch. The bot pin was never consulted, so a bot narrowing its chats
/// was invisible. And the whole resolution was skipped whenever the entry
/// had no workspace of its own — the common case, since a chat inherits the
/// gateway's workspace unless an operator pins one — which serialized
/// `effective_permission_mode` as `null` and left the console's mode picker
/// falling back to a hardcoded guess with no ceiling to clamp against.
fn resolve_entry_permission(
    state: &AppState,
    e: &crate::gateway::AllowlistEntry,
) -> crate::gateway::ResolvedPermission {
    let bot = e
        .inherit_bot_policy
        .then_some(e.bot_id.as_deref())
        .flatten()
        .and_then(|id| state.gateway.bot_get(id));
    // Same precedence `core_for_entry` uses: the chat's own workspace, else
    // its bot's, else the gateway's default.
    let workspace = e
        .space
        .clone()
        .or_else(|| bot.as_ref().and_then(|b| b.space.clone()))
        .and_then(|space| crate::gateway::space_folder(&space))
        .unwrap_or_else(|| state.core.cwd().clone());
    crate::gateway::resolve_channel_permission(
        &workspace,
        e.permission_mode,
        bot.and_then(|b| b.permission_mode),
    )
}

fn allowlist_entry_json(state: &AppState, e: &crate::gateway::AllowlistEntry) -> serde_json::Value {
    // Resolve the effective permission mode the same way "Effective route"
    // is surfaced: the console must show what the channel actually gets,
    // not just what was requested, so a capped override is visible rather
    // than mistaken for a live grant.
    let resolved = resolve_entry_permission(state, e);
    let bot = if e.inherit_bot_policy {
        e.bot_id.as_deref().and_then(|id| state.gateway.bot_get(id))
    } else {
        None
    };
    let effective_agent_id = if let Some(ref aid) = e.agent_id {
        if aid != "vak" || !e.inherit_bot_policy {
            aid.clone()
        } else {
            bot.as_ref()
                .and_then(|b| b.agent_id.clone())
                .unwrap_or_else(|| "vak".into())
        }
    } else {
        bot.as_ref()
            .and_then(|b| b.agent_id.clone())
            .unwrap_or_else(|| "vak".into())
    };
    serde_json::json!({
        "key": e.key,
        "status": e.status,
        "space": e.space,
        "workspace": crate::gateway::space_folder_json(e.space.as_deref()),
        "agent_id": e.agent_id,
        "effective_agent_id": effective_agent_id,
        "route": e.route,
        "permission_mode": e.permission_mode,
        "workspace_permission_mode": resolved.workspace_mode,
        // The bot tier, named, so a reader can tell "the project caps this"
        // from "the bot caps this" instead of only seeing the result.
        "bot_permission_mode": resolved.bot_mode,
        "effective_permission_mode": resolved.effective,
        "permission_capped": resolved.was_capped(),
        "policy": e.policy,
        "bot_id": e.bot_id,
        "inherit_bot_policy": e.inherit_bot_policy,
        "voice": e.voice,
        "added_at": e.added_at,
        "added_by": e.added_by,
        "first_seen_text": e.first_seen_text,
    })
}

/// Parse an optional `permission_mode` field from an approve/PATCH body.
/// `Ok(None)` = inherit (field absent, null, or empty string); `Err(())` =
/// present but unparseable, which is a 400 rather than a silent inherit —
/// a typo'd mode must never quietly widen or narrow a channel's access.
fn parse_permission_mode_field(
    raw: Option<&str>,
) -> Result<Option<vak_config::PermissionMode>, ()> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) => crate::parse_mode(s).map(Some).ok_or(()),
    }
}

pub(crate) async fn list_gateway_allowlist(
    State(state): State<AppState>,
) -> Json<serde_json::Value> {
    let entries: Vec<serde_json::Value> = state
        .gateway
        .allowlist_snapshot()
        .iter()
        .map(|entry| allowlist_entry_json(&state, entry))
        .collect();
    Json(serde_json::json!({ "entries": entries }))
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct AllowlistApproveBody {
    #[serde(default)]
    workspace: Option<String>,
    /// Optional Agent slug. Omitting it binds the endpoint to built-in Vak.
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    route: Option<GatewayRoutePatch>,
    /// Optional per-channel permission mode, riding along in the same
    /// request as `route` rather than on an endpoint of its own.
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    policy: Option<vak_config::ChannelPolicy>,
    /// Bind this chat to a bot identity (multi-bot-per-channel). `null`/
    /// absent/`""` unbinds it — the chat then resolves purely against the
    /// workspace, same as before bots existed.
    #[serde(default)]
    bot_id: Option<String>,
    /// Whether to inherit the bound bot's policy/permission_mode/route as a
    /// tier below this chat's own. Defaults to `true`; ignored when no
    /// `bot_id` is set.
    #[serde(default = "crate::gateway::default_true")]
    inherit_bot_policy: bool,
}

/// Audit an override that the workspace's own boundary will cap down, at
/// the moment the operator sets it — so the reduction is visible in the
/// security log immediately, not only when the channel's `Core` is first
/// started at dispatch (where `CorePool` records the enforcement itself).
fn record_permission_cap(state: &AppState, key: &str, entry: &crate::gateway::AllowlistEntry) {
    let resolved = resolve_entry_permission(state, entry);
    if !resolved.was_capped() {
        return;
    }
    // Name the ceiling that actually did the capping. "Reduced to read-only"
    // is not actionable on its own — an operator needs to know whether to
    // widen the project's config or the bot's pin.
    let ceiling = match resolved.bot_mode {
        Some(bot) if bot.capped_by(resolved.workspace_mode) == resolved.effective => "bot",
        _ => "workspace",
    };
    vak_core::security_events::record(
        &state.core.scope(),
        vak_core::security_events::EventKind::PermissionCapped,
        "permission_capped",
        &format!(
            "key={key} workspace={} requested={} capped_to={} by={ceiling}",
            entry
                .space
                .as_deref()
                .and_then(crate::gateway::space_folder)
                .unwrap_or_else(|| state.core.cwd().clone())
                .display(),
            resolved
                .requested
                .map(|m| m.as_str())
                .unwrap_or("(inherit)"),
            resolved.effective.as_str()
        ),
        None,
    );
}

/// Record the operator's trust decision for a workspace they just pointed a
/// channel at through the console.
///
/// Pinning a workspace for a channel IS the decision `vak_core::trust`
/// records: the operator is saying "run turns here, with this project's own
/// configuration". Writing the marker keeps that decision in the one store
/// every surface reads, so the terminal and the server agree — instead of
/// the server assuming trust, which is what it used to do.
///
/// Best-effort: an unwritable data home means the workspace loads
/// untrusted, which is the safe direction. It is never the reason an
/// approval fails.
fn note_workspace_trust(state: &AppState, workspace: Option<&std::path::Path>) {
    let Some(workspace) = workspace else { return };
    if vak_core::trust::is_trusted(workspace) {
        return;
    }
    match vak_core::trust::record(workspace) {
        Ok(()) => {
            vak_core::security_events::record(
                &state.core.scope(),
                vak_core::security_events::EventKind::ConfigChange,
                "workspace_trusted",
                &format!("workspace={} by=admin", workspace.display()),
                None,
            );
        }
        Err(error) => {
            tracing::warn!(error_kind = %vak_telemetry::error_kind(&error), "a trust decision was not recorded")
        }
    }
}

pub(crate) async fn approve_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let body: AllowlistApproveBody = if body.is_empty() {
        AllowlistApproveBody::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    };
    let workspace = match body.workspace {
        Some(w) if !w.trim().is_empty() => PathBuf::from(w.trim()),
        _ => state.core.cwd().clone(),
    };
    let route = match body.route {
        Some(GatewayRoutePatch {
            provider: Some(provider),
            model: Some(model),
        }) if !provider.trim().is_empty() && !model.trim().is_empty() => {
            Some(crate::gateway::AllowlistRoute {
                provider: provider.trim().to_string(),
                model: model.trim().to_string(),
            })
        }
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
        None => None,
    };
    let permission_mode = match parse_permission_mode_field(body.permission_mode.as_deref()) {
        Ok(m) => m,
        Err(()) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let bot_id = body
        .bot_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let space = match vak_config::spaces::bind(&workspace) {
        Ok(space) => space,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let entry = state.gateway.allowlist_approve(
        &state.core,
        &key,
        space,
        body.agent_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        route,
        permission_mode,
        body.policy.unwrap_or_default(),
        bot_id,
        body.inherit_bot_policy,
        "admin",
    );
    vak_core::security_events::record(
        &state.core.scope(),
        vak_core::security_events::EventKind::ChatApproved,
        "chat_approved",
        &format!("key={key}"),
        None,
    );
    note_workspace_trust(&state, Some(&workspace));
    record_permission_cap(&state, &key, &entry);
    state
        .hub
        .emit_config_changed("gateway_allowlist_approved", &key);
    (StatusCode::OK, Json(allowlist_entry_json(&state, &entry))).into_response()
}

pub(crate) async fn deny_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Response {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let entry = state.gateway.allowlist_deny(&state.core, &key, "admin");
    vak_core::security_events::record(
        &state.core.scope(),
        vak_core::security_events::EventKind::ChatDenied,
        "chat_denied",
        &format!("key={key}"),
        None,
    );
    state
        .hub
        .emit_config_changed("gateway_allowlist_denied", &key);
    (StatusCode::OK, Json(allowlist_entry_json(&state, &entry))).into_response()
}

#[derive(serde::Deserialize, Default)]
pub(crate) struct AllowlistPatchBody {
    #[serde(default)]
    workspace: Option<String>,
    /// Optional agent binding. Send null/empty to reset to built-in vak.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    pub(crate) agent_id: Option<Option<String>>,
    #[serde(default)]
    route: Option<GatewayRoutePatch>,
    /// Absent / null / `""` clears the pin (inherit the workspace default),
    /// mirroring how an empty `route` object clears a pinned route.
    #[serde(default)]
    permission_mode: Option<String>,
    #[serde(default)]
    policy: Option<vak_config::ChannelPolicy>,
    /// See `AllowlistApproveBody::bot_id`. `None` here (field simply
    /// absent) means "leave whatever bot binding is already set" — unlike
    /// approve, patch is an edit-in-place and must not silently unbind a
    /// chat just because a caller's PATCH body didn't mention bots at all.
    /// Send an explicit `null` to unbind. See
    /// `crate::gateway::deserialize_present` for why the plain
    /// `Option<Option<T>>` shape alone can't tell "absent" from "present
    /// as null" apart — without it this `null` would silently do nothing.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    bot_id: Option<Option<String>>,
    #[serde(default)]
    inherit_bot_policy: Option<bool>,
    /// Absent leaves this chat's voice alone; explicit `null` clears it
    /// back to inherit (bot tier, then no voice); a `VoiceConfig` object
    /// pins this chat's own override. Same `deserialize_present` shape as
    /// `bot_id` above, for the same reason.
    #[serde(default, deserialize_with = "crate::gateway::deserialize_present")]
    voice: Option<Option<vak_config::VoiceConfig>>,
    /// This chat's prompt tier (docs/design/45-prompt-layers.md). Absent
    /// leaves it alone; an object replaces it. Restrictive by construction:
    /// its guardrails add to the chain and its identity can only lose to a
    /// narrower layer, never reach the code-owned blocks.
    #[serde(default)]
    prompt: Option<vak_core::prompts::LayerContent>,
}

/// `PATCH /admin/api/gateway/allowlist/{key}` (docs/design/34 "Editing an
/// already-allowed entry"): re-point an `allowed` channel's workspace
/// and/or pinned route without revoking and re-approving it, which would
/// lose `added_at`/`added_by` provenance and momentarily 403 the channel.
///
/// Only `allowed` entries are editable — pending/denied ones move through
/// approve/deny, not here (404 otherwise, same as an unknown key).
///
/// The edit is *not* a second way to change a channel's effective route:
/// the entry is the one source of truth `binding_route` already reads, and
/// the binding's cached route revision is invalidated here so the change
/// takes effect through the exact stale-session-rotation path
/// `PATCH .../bindings/{key}` uses — the next inbound message rotates to a
/// fresh frozen session, preserving the old append-only ledger.
pub(crate) async fn patch_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if key.trim().is_empty() || !key.contains(':') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let body: AllowlistPatchBody = if body.is_empty() {
        AllowlistPatchBody::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(b) => b,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        }
    };
    let workspace = body
        .workspace
        .as_deref()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(PathBuf::from);
    let route = match body.route {
        Some(GatewayRoutePatch {
            provider: Some(provider),
            model: Some(model),
        }) if !provider.trim().is_empty() && !model.trim().is_empty() => {
            if !state
                .core
                .provider_names()
                .iter()
                .any(|name| name == provider.trim())
            {
                return StatusCode::BAD_REQUEST.into_response();
            }
            Some(crate::gateway::AllowlistRoute {
                provider: provider.trim().to_string(),
                model: model.trim().to_string(),
            })
        }
        // An explicitly empty route object clears the pin (inherit the
        // workspace default) — the same shape `PATCH .../bindings` uses.
        Some(GatewayRoutePatch {
            provider: None,
            model: None,
        })
        | None => None,
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let permission_mode = match parse_permission_mode_field(body.permission_mode.as_deref()) {
        Ok(m) => m,
        Err(()) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let existing = state.gateway.allowlist_get(&key);
    let agent_id = body.agent_id.map(|inner| {
        inner
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    let bot_id = body.bot_id.map(|inner| {
        inner
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });
    if let Some(Some(Err(error))) = body
        .voice
        .as_ref()
        .map(|voice| voice.as_ref().map(crate::voice::check_tier))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": error })),
        )
            .into_response();
    }
    let space = match workspace
        .as_deref()
        .map(vak_config::spaces::bind)
        .transpose()
    {
        Ok(space) => space,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let Some(entry) = state.gateway.allowlist_patch(
        &state.core,
        &key,
        space,
        agent_id,
        route,
        permission_mode,
        body.policy
            .or_else(|| existing.as_ref().map(|e| e.policy.clone()))
            .unwrap_or_default(),
        bot_id,
        body.inherit_bot_policy,
        body.voice,
        body.prompt,
    ) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record_permission_cap(&state, &key, &entry);
    // Same stale-detection seam as the binding route editor: drop the
    // cached revision (never the ledger) so the next message re-derives
    // the effective route and rotates only if it really changed.
    state.gateway.invalidate_binding_revision(&state.core, &key);
    vak_core::security_events::record(
        &state.core.scope(),
        vak_core::security_events::EventKind::ConfigChange,
        "chat_edited",
        &format!(
            "key={key} workspace={} permission_mode={}",
            workspace
                .as_ref()
                .map(|w| w.display().to_string())
                .unwrap_or_else(|| "(inherit)".into()),
            entry
                .permission_mode
                .map(|m| m.as_str())
                .unwrap_or("(inherit)")
        ),
        None,
    );
    note_workspace_trust(&state, workspace.as_deref());
    state
        .hub
        .emit_config_changed("gateway_allowlist_patched", &key);
    (StatusCode::OK, Json(allowlist_entry_json(&state, &entry))).into_response()
}

pub(crate) async fn revoke_gateway_allowlist(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> StatusCode {
    if state.gateway.allowlist_revoke(&state.core, &key) {
        vak_core::security_events::record(
            &state.core.scope(),
            vak_core::security_events::EventKind::ChatRevoked,
            "chat_revoked",
            &format!("key={key}"),
            None,
        );
        state
            .hub
            .emit_config_changed("gateway_allowlist_revoked", &key);
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- Admin route mounter --------------------------------------------------

pub(crate) fn routes() -> axum::Router<AppState> {
    use axum::routing::{get, patch, post};
    axum::Router::new()
        .route("/admin/api/sessions", get(list_sessions_admin))
        .route(
            "/admin/api/sessions/{id}/transcript",
            get(session_transcript_admin),
        )
        .route("/admin/api/approvals", get(list_pending_approvals))
        .route("/admin/api/questions", get(list_pending_questions))
        .route("/admin/api/bestofn", get(list_bestofn))
        .route("/admin/api/events", get(admin_events_sse))
        .route("/admin/api/security", get(list_security_events))
        // Mutations are POST: crawlers/prefetchers only ever issue GETs.
        .route("/admin/api/config", get(get_config_admin))
        .route("/admin/api/gateway/status", get(gateway_status_admin))
        .route("/admin/api/traffic", get(admin_traffic_status))
        .route(
            "/admin/api/gateway/workspace",
            axum::routing::patch(patch_gateway_workspace),
        )
        .route("/admin/api/projects", get(list_projects))
        .route("/admin/api/projects/{id}", patch(patch_project))
        .route(
            "/admin/api/gateway/bindings/{key}",
            patch(patch_gateway_binding).delete(delete_gateway_binding_admin),
        )
        .route(
            "/admin/api/gateway/bindings/{key}/rotate",
            post(rotate_gateway_binding),
        )
        .route("/admin/api/gateway/allowlist", get(list_gateway_allowlist))
        .route(
            "/admin/api/gateway/allowlist/{key}/approve",
            post(approve_gateway_allowlist),
        )
        .route(
            "/admin/api/gateway/allowlist/{key}/deny",
            post(deny_gateway_allowlist),
        )
        .route(
            "/admin/api/gateway/allowlist/{key}",
            patch(patch_gateway_allowlist).delete(revoke_gateway_allowlist),
        )
}

// ---- Tests ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use crate::AppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> AppState {
        // Isolates the trust-marker store as well as the session ledger:
        // approving a channel records a trust decision, and that write must
        // not reach the developer's real data home.
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        let core = vak_core::Core::new(cwd).unwrap();
        // Without this, `scope()` falls back to the developer's
        // real $XDG_DATA_HOME/vak — any test that persists something
        // (gateway bindings, the allowlist store) would leak state across
        // test runs and across the machine. Every other test module in
        // this crate isolates it the same way.
        core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
        AppState::new(core)
    }

    fn authed_app(state: &AppState) -> axum::Router {
        crate::router_with_state(state.clone()).layer(axum::middleware::from_fn_with_state(
            crate::AuthPolicy {
                token: (*state.auth_token).clone(),
                home: state.core.scope().into_root(),
                shared: state.core.shared_scope().into_root(),
                trusted_hosts: Vec::new(),
                public_url: None,
                browser_sessions: state.browser_sessions.clone(),
            },
            crate::require_bearer,
        ))
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn list_sessions_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/sessions")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["sessions"].is_array());
    }

    #[tokio::test]
    async fn search_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/search?q=hello&all=true")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["hits"].is_array());
    }

    #[tokio::test]
    async fn security_events_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/security")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn catalog_rebuild_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/catalog/rebuild")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["ok"].as_bool().unwrap_or(false));
    }

    #[tokio::test]
    async fn rebuild_via_get_is_rejected() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/catalog/rebuild")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    /// One login for every browser surface: the operations console and
    /// the workspace client share `/auth/login` and one cookie.
    /// `/admin/login` was REMOVED rather than kept alongside it — two
    /// endpoints against one cookie is two contracts that must agree
    /// forever (invariant 30).
    #[tokio::test]
    async fn login_sets_cookie_and_it_authenticates() {
        use axum::http::header;
        let state = test_state();
        let token = (*state.auth_token).clone();

        // Login itself is auth-exempt; plain router is fine for it.
        let plain = crate::router_with_state(state.clone());

        // Wrong token → 401.
        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"token":"wrong"}"#))
            .unwrap();
        let resp = plain.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // Correct token → 200 + cookie.
        let req = Request::builder()
            .method("POST")
            .uri("/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"token":"{token}"}}"#)))
            .unwrap();
        let resp = plain.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let set_cookie = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(set_cookie.starts_with("vak_session="), "cookie must be set");
        assert!(set_cookie.contains("HttpOnly"));

        // Cookie authenticates a protected admin endpoint with no header.
        let cookie_pair = &set_cookie[..set_cookie.find(';').unwrap_or(set_cookie.len())];
        let authed = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/sessions")
            .header(header::COOKIE, cookie_pair)
            .body(Body::empty())
            .unwrap();
        let resp = authed.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unauthenticated_admin_api_is_401() {
        let state = test_state();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/sessions")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// `EventSource` cannot set request headers, and the desktop app
    /// never performs the `/auth/login` cookie exchange -- that is the
    /// browser console's flow. `?token=` is the only channel its SSE
    /// streams can authenticate on, and `openEventStream` /
    /// `openSideStream` have always used it. The middleware accepted
    /// only the header and the cookie, so every desktop event stream
    /// was rejected 401 and the app received not one agent event: runs
    /// completed and were durably logged while the UI showed no reply,
    /// "Working" forever, and `0 in / 0 out`. Nothing surfaced in the
    /// console either, because a 401 on an EventSource is just a bare
    /// `onerror`.
    ///
    /// Found by opening the real SSE endpoint with curl and getting
    /// zero bytes back, after three unrelated "fixes" shipped on
    /// theory without ever testing this path.
    #[tokio::test]
    async fn sse_authenticates_with_the_token_query_parameter() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let mut req = Request::builder()
            .uri(format!("/sessions/does-not-exist/events?token={token}"))
            .header(axum::http::header::HOST, "127.0.0.1")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50_000,
            ))));
        let resp = app.oneshot(req).await.unwrap();
        // The session id is bogus, so any status is acceptable EXCEPT
        // 401: this asserts the request got past authentication, which
        // is the thing that was broken. Asserting 200 would instead
        // pin unrelated session-lookup behaviour.
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "?token= must authenticate -- it is the only channel EventSource has"
        );
    }

    #[tokio::test]
    async fn a_wrong_token_query_parameter_is_still_rejected() {
        let state = test_state();
        let app = authed_app(&state);
        let mut req = Request::builder()
            .uri("/sessions/does-not-exist/events?token=vk_not-the-real-token")
            .header(axum::http::header::HOST, "127.0.0.1:41783")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                41783,
            ))));
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn pending_approvals_lists_empty_without_gates() {
        let state = test_state();
        let token = (*state.auth_token).clone();

        // Create a live session so the aggregation path has something to walk.
        {
            let core = state.core.clone();
            let s = core.start_session().await.unwrap();
            let id = s.header().map(|h| h.session_id.clone()).unwrap_or_default();
            crate::register_handle(&state, id, s, state.core.cwd().clone(), state.core.clone());
        }

        let app =
            crate::router_with_state(state.clone()).layer(axum::middleware::from_fn_with_state(
                crate::AuthPolicy {
                    token: (*state.auth_token).clone(),
                    home: state.core.scope().into_root(),
                    shared: state.core.shared_scope().into_root(),
                    trusted_hosts: Vec::new(),
                    public_url: None,
                    browser_sessions: state.browser_sessions.clone(),
                },
                crate::require_bearer,
            ));
        let req = Request::builder()
            .uri("/admin/api/approvals")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["total"], 0);
        assert!(json["approvals"].is_array());
    }

    #[tokio::test]
    async fn pending_questions_lists_a_waiting_workers_question_read_only() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let id = {
            let core = state.core.clone();
            let s = core.start_session().await.unwrap();
            let id = s.header().map(|h| h.session_id.clone()).unwrap_or_default();
            crate::register_handle(
                &state,
                id.clone(),
                s,
                state.core.cwd().clone(),
                state.core.clone(),
            );
            id
        };
        let _waiting = state
            .core
            .workers()
            .questions()
            .open(vak_agent::PendingQuestion {
                id: "q-admin".into(),
                worker_id: "child-1".into(),
                label: "Totals".into(),
                parent_session_id: id.clone(),
                question: "Which fiscal year?".into(),
                options: vec!["2025".into(), "2026".into()],
                asked_at: chrono::Utc::now(),
            })
            .unwrap();

        let app =
            crate::router_with_state(state.clone()).layer(axum::middleware::from_fn_with_state(
                crate::AuthPolicy {
                    token: (*state.auth_token).clone(),
                    home: state.core.scope().into_root(),
                    shared: state.core.shared_scope().into_root(),
                    trusted_hosts: Vec::new(),
                    public_url: None,
                    browser_sessions: state.browser_sessions.clone(),
                },
                crate::require_bearer,
            ));
        let get = |uri: &str, method: &str| {
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap()
        };
        let resp = app
            .clone()
            .oneshot(get("/admin/api/questions", "GET"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["total"], 1, "{json}");
        let row = &json["questions"][0];
        assert_eq!(row["session_id"], id.as_str());
        assert_eq!(row["worker"], "Totals");
        assert_eq!(row["question"], "Which fiscal year?");
        assert_eq!(row["options"], serde_json::json!(["2025", "2026"]));

        // Read only: the admin path offers no way to answer.
        let resp = app
            .oneshot(get("/admin/api/questions/q-admin", "POST"))
            .await
            .unwrap();
        assert!(
            matches!(
                resp.status(),
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{}",
            resp.status()
        );
        assert!(
            state.core.workers().questions().is_open("q-admin"),
            "listing and probing never answer"
        );
    }

    #[tokio::test]
    async fn outcome_review_rejects_unknown_verdict() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/sessions/missing/outcome-review")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"verdict":"maybe"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// These three were read by the console and never sent. The console's
    /// own `ConfigInfo` declared all of them, so nothing caught it: the
    /// approval picker could not show what was in force, "Effective sandbox"
    /// rendered its loading placeholder forever, and the workers toggle
    /// rendered unchecked whatever the real value was.
    #[tokio::test]
    async fn config_endpoint_reports_approval_mode_sandbox_and_workers() {
        let state = test_state();
        let response = crate::admin::get_config_admin(
            axum::extract::State(state),
            axum::extract::Query(crate::AgentScopeQuery::default()),
        )
        .await;
        let json = body_json(response).await;
        assert!(json["approval_mode"].is_string(), "{json}");
        assert!(json["sandbox"].is_string(), "{json}");
        assert!(json["workers"].is_boolean(), "{json}");
    }

    #[tokio::test]
    async fn config_endpoint_returns_ok() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/config")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert!(json["provider"].is_string());
    }

    // ---- Allowlist admin routes (docs/design/34-channel-onboarding.md) ----

    #[tokio::test]
    async fn allowlist_list_returns_entries_array() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/allowlist")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        // Not asserting emptiness: a developer's own global config.toml may
        // legitimately seed `gateway.chat_allowlist` entries on load, and
        // that is not this test's concern — only that the route responds
        // with the documented shape.
        assert!(json["entries"].is_array());
    }

    #[tokio::test]
    async fn allowlist_list_requires_auth() {
        let state = test_state();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/allowlist")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn allowlist_approve_defaults_workspace_to_core_cwd_and_shows_it() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A42/approve")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["status"], "allowed");
        // The point of this endpoint: the workspace is explicit in the
        // response even when the caller didn't supply one.
        let ws = json["workspace"].as_str().expect("workspace must be shown");
        assert_eq!(
            std::path::Path::new(ws),
            state.core.cwd().canonicalize().unwrap(),
            "a binding is the canonical folder"
        );
    }

    #[tokio::test]
    async fn allowlist_approve_with_explicit_workspace_and_route() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A43/approve")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"workspace":"/tmp/somewhere","route":{"provider":"anthropic","model":"sonnet"}}"#,
            ))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["workspace"], "/tmp/somewhere");
        assert_eq!(json["route"]["provider"], "anthropic");
        assert_eq!(json["route"]["model"], "sonnet");
    }

    // ---- PATCH .../allowlist/{key} (docs/design/34 follow-up) ----------

    async fn approve(app: &axum::Router, token: &str, key: &str, body: &str) {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/admin/api/gateway/allowlist/{key}/approve"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    /// Approve and return the entry JSON, for the permission-mode tests.
    async fn approve_json(
        app: &axum::Router,
        token: &str,
        key: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/admin/api/gateway/allowlist/{key}/approve"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        if status != StatusCode::OK {
            return (status, serde_json::Value::Null);
        }
        (status, body_json(resp).await)
    }

    /// A workspace directory whose own config fixes `mode` — the ceiling
    /// a channel override is capped against.
    fn workspace_dir_with_mode(mode: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let vak = dir.path().join(".vak");
        std::fs::create_dir_all(&vak).unwrap();
        std::fs::write(
            vak.join("config.toml"),
            format!("permission_mode = \"{mode}\"\n"),
        )
        .unwrap();
        dir
    }

    #[tokio::test]
    async fn approve_without_permission_mode_inherits_the_workspace() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("read-only");
        let (status, json) = approve_json(
            &app,
            &token,
            "telegram%3A60",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // No pin stored, and the effective mode is simply the workspace's.
        assert!(json["permission_mode"].is_null());
        assert_eq!(json["effective_permission_mode"], "read-only");
        assert_eq!(json["permission_capped"], false);
    }

    #[tokio::test]
    async fn approve_with_permission_mode_at_or_below_workspace_is_kept() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("full-access");
        let (status, json) = approve_json(
            &app,
            &token,
            "telegram%3A61",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "read-only" })
                .to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["permission_mode"], "read-only");
        assert_eq!(json["workspace_permission_mode"], "full-access");
        assert_eq!(json["effective_permission_mode"], "read-only");
        assert_eq!(json["permission_capped"], false);
    }

    /// The security-relevant admin path: an operator asking for more than
    /// the workspace itself allows gets the workspace's mode, not theirs,
    /// and the reduction lands in the security log.
    #[tokio::test]
    async fn approve_with_over_broad_permission_mode_is_capped_and_audited() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("read-only");
        let (status, json) = approve_json(
            &app,
            &token,
            "telegram%3A62",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "full-access" })
                .to_string(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // The request is recorded verbatim...
        assert_eq!(json["permission_mode"], "full-access");
        // ...but what the channel actually gets is the workspace's ceiling.
        assert_eq!(json["effective_permission_mode"], "read-only");
        assert_eq!(json["permission_capped"], true);

        let events = vak_core::security_events::list(&state.core.scope(), 50);
        assert!(
            events.iter().any(
                |e| e.kind == vak_core::security_events::EventKind::PermissionCapped
                    && e.detail.contains("requested=full-access")
                    && e.detail.contains("capped_to=read-only")
            ),
            "a capped grant must be visible in the audit log, got {events:?}"
        );
    }

    #[tokio::test]
    async fn approve_rejects_an_unparseable_permission_mode() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let (status, _) = approve_json(
            &app,
            &token,
            "telegram%3A63",
            r#"{"permission_mode":"god-mode"}"#,
        )
        .await;
        // A typo must be a hard 400, never a silent inherit.
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_sets_and_clears_a_permission_pin() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("full-access");
        approve(
            &app,
            &token,
            "telegram%3A64",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;

        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A64",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "workspace-write" })
                .to_string(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["permission_mode"], "workspace-write");
        assert_eq!(json["effective_permission_mode"], "workspace-write");

        // An omitted field clears the pin back to inherit, the same way an
        // empty route object clears a pinned route.
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A64",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;
        let json = body_json(resp).await;
        assert!(json["permission_mode"].is_null());
        assert_eq!(json["effective_permission_mode"], "full-access");
    }

    #[tokio::test]
    async fn patch_with_over_broad_permission_mode_is_capped() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let ws = workspace_dir_with_mode("workspace-write");
        approve(
            &app,
            &token,
            "telegram%3A65",
            &serde_json::json!({ "workspace": ws.path() }).to_string(),
        )
        .await;
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A65",
            &serde_json::json!({ "workspace": ws.path(), "permission_mode": "full-access" })
                .to_string(),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["effective_permission_mode"], "workspace-write");
        assert_eq!(json["permission_capped"], true);
    }

    /// Editing a route from the *binding* surface must not silently drop a
    /// channel's permission pin — that surface never mentions permissions.
    #[tokio::test]
    async fn binding_route_write_through_preserves_the_permission_pin() {
        let state = test_state();
        let ws = workspace_dir_with_mode("full-access");
        state.gateway.allowlist_approve(
            &state.core,
            "telegram:66",
            vak_config::spaces::bind(ws.path()).unwrap(),
            None,
            None,
            Some(vak_config::PermissionMode::ReadOnly),
            vak_config::ChannelPolicy::default(),
            None,
            true,
            "admin",
        );
        state.gateway.allowlist_patch_route_if_allowed(
            &state.core,
            "telegram:66",
            Some(crate::gateway::AllowlistRoute {
                provider: "anthropic".into(),
                model: "sonnet".into(),
            }),
        );
        let entry = state.gateway.allowlist_get("telegram:66").unwrap();
        assert_eq!(
            entry.permission_mode,
            Some(vak_config::PermissionMode::ReadOnly)
        );
        assert_eq!(entry.route.unwrap().model, "sonnet");
    }

    async fn patch_allowlist(
        app: &axum::Router,
        token: &str,
        key: &str,
        body: &str,
    ) -> axum::response::Response {
        let req = Request::builder()
            .method("PATCH")
            .uri(format!("/admin/api/gateway/allowlist/{key}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn allowlist_patch_edits_workspace_in_place_keeping_provenance() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(&app, &token, "telegram%3A50", r#"{"workspace":"/tmp/one"}"#).await;
        let before = state.gateway.allowlist_get("telegram:50").unwrap();

        let resp =
            patch_allowlist(&app, &token, "telegram%3A50", r#"{"workspace":"/tmp/two"}"#).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["workspace"], "/tmp/two");
        // A re-point is not a re-approval: added_at/added_by must survive.
        assert_eq!(json["added_at"], before.added_at);
        assert_eq!(json["added_by"], before.added_by);
        assert_eq!(json["status"], "allowed");
    }

    #[tokio::test]
    async fn allowlist_patch_clears_a_pinned_route_with_an_empty_route_object() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(
            &app,
            &token,
            "telegram%3A51",
            r#"{"workspace":"/tmp/one","route":{"provider":"anthropic","model":"sonnet"}}"#,
        )
        .await;
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3A51",
            r#"{"workspace":"/tmp/one","route":{}}"#,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            state
                .gateway
                .allowlist_get("telegram:51")
                .unwrap()
                .route
                .is_none()
        );
    }

    #[tokio::test]
    async fn allowlist_patch_rotates_through_the_existing_stale_detection_path() {
        // docs/design/34: an edited workspace/route must take effect the
        // way a stale binding already does — by dropping the cached route
        // revision so the next inbound message re-derives it, never by
        // mutating state the binding/session layer doesn't know about.
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(&app, &token, "telegram%3A52", r#"{"workspace":"/tmp/one"}"#).await;
        state.gateway.set_route_override(
            &state.core,
            "telegram:52".into(),
            Some(("anthropic".into(), "sonnet".into())),
        );
        // A bound session with a frozen revision, as dispatch would leave it.
        state
            .gateway
            .bind_for_test("telegram:52", "sess-1", "channel:anthropic:sonnet");
        assert!(
            state
                .gateway
                .route_revision_for_test("telegram:52")
                .is_some()
        );

        let resp =
            patch_allowlist(&app, &token, "telegram%3A52", r#"{"workspace":"/tmp/two"}"#).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            state
                .gateway
                .route_revision_for_test("telegram:52")
                .is_none()
        );
        // The ledger binding itself is preserved — rotation replaces the
        // session on the next message, it never deletes history.
        assert_eq!(
            state.gateway.session_id_for_test("telegram:52").as_deref(),
            Some("sess-1")
        );
    }

    #[tokio::test]
    async fn allowlist_patch_is_404_for_unknown_and_non_allowed_keys() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let resp = patch_allowlist(
            &app,
            &token,
            "telegram%3Anever-seen",
            r#"{"workspace":"/tmp"}"#,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // Denied entries move through approve/deny, never through edit.
        state
            .gateway
            .allowlist_deny(&state.core, "telegram:53", "admin");
        let resp = patch_allowlist(&app, &token, "telegram%3A53", r#"{"workspace":"/tmp"}"#).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn allowlist_patch_rejects_a_key_that_is_not_surface_chat() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let resp = patch_allowlist(&app, &token, "nocolon", r#"{"workspace":"/tmp"}"#).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn binding_route_patch_writes_through_to_the_allowed_entry() {
        // One source of truth: changing the route from the pre-existing
        // binding editor must not leave the allowlist entry holding a
        // different, stale answer to "what does this channel route to".
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(
            &app,
            &token,
            "telegram%3A54",
            r#"{"workspace":"/tmp/one","route":{"provider":"anthropic","model":"sonnet"}}"#,
        )
        .await;
        let req = Request::builder()
            .method("PATCH")
            .uri("/admin/api/gateway/bindings/telegram%3A54")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let entry = state.gateway.allowlist_get("telegram:54").unwrap();
        assert!(
            entry.route.is_none(),
            "entry kept a route the binding cleared"
        );
        // The workspace is untouched by a route-only edit.
        let folder = crate::gateway::space_folder(&entry.space.unwrap()).unwrap();
        assert_eq!(
            folder,
            std::path::Path::new("/tmp/one")
                .canonicalize()
                .unwrap_or_else(|_| "/tmp/one".into())
        );
    }

    #[test]
    fn scratch_workspace_detection() {
        assert!(super::is_scratch_workspace("/tmp/foo"));
        assert!(super::is_scratch_workspace("/private/tmp/foo"));
        assert!(super::is_scratch_workspace(
            "/var/folders/0g/xyz/T/.tmpAbC123"
        ));
        assert!(super::is_scratch_workspace(
            "/private/var/folders/0g/xyz/T/.tmpAbC123"
        ));
        assert!(super::is_scratch_workspace("/Users/x/anywhere/.tmpZZZZZZ"));
        assert!(!super::is_scratch_workspace("/Users/x/Projects/vakcoder"));
        assert!(!super::is_scratch_workspace("/Users/x/Projects/tmp-tool"));
    }

    #[tokio::test]
    async fn gateway_status_lists_known_workspaces_including_the_gateway_cwd() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/status")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let json = body_json(app.oneshot(req).await.unwrap()).await;
        let known = json["known_workspaces"].as_array().unwrap();
        assert!(
            known
                .iter()
                .any(|w| w.as_str() == Some(&state.core.cwd().display().to_string())),
            "the gateway's own workspace must always be offered"
        );
    }

    #[tokio::test]
    async fn gateway_status_lists_allowed_channel_before_first_message() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(&app, &token, "telegram%3A67", r#"{}"#).await;
        let req = Request::builder()
            .uri("/admin/api/gateway/status")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let json = body_json(app.oneshot(req).await.unwrap()).await;
        let binding = json["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|binding| binding["target"] == "telegram:67")
            .expect("approved cold channel must be visible");
        assert!(binding["session_id"].is_null());
        assert_eq!(binding["stale"], false);
    }

    #[tokio::test]
    async fn gateway_status_reports_an_existing_bindings_configured_workspace() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        approve(
            &app,
            &token,
            "telegram%3A68",
            r#"{"workspace":"/tmp/chat-workspace"}"#,
        )
        .await;
        state
            .gateway
            .set_route_override(&state.core, "telegram:68".into(), None);

        let req = Request::builder()
            .uri("/admin/api/gateway/status")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let json = body_json(app.oneshot(req).await.unwrap()).await;
        let binding = json["bindings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|binding| binding["target"] == "telegram:68")
            .expect("existing binding must be visible");
        assert_eq!(binding["workspace"], "/tmp/chat-workspace");
        assert_eq!(binding["configured_workspace"], "/tmp/chat-workspace");
        assert_eq!(binding["stale"], false);
    }

    #[tokio::test]
    async fn allowlist_deny_then_revoke_of_denied_is_404() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A44/deny")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["status"], "denied");

        // Revoke only applies to *allowed* entries; a denied one 404s.
        let app2 = authed_app(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/admin/api/gateway/allowlist/telegram%3A44")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn allowlist_revoke_unknown_key_is_404() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/admin/api/gateway/allowlist/telegram%3Anever-seen")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn allowlist_approve_then_revoke_roundtrip() {
        let state = test_state();
        let token = (*state.auth_token).clone();
        let app = authed_app(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/admin/api/gateway/allowlist/telegram%3A45/approve")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app2 = authed_app(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/admin/api/gateway/allowlist/telegram%3A45")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let app3 = authed_app(&state);
        let req = Request::builder()
            .uri("/admin/api/gateway/allowlist")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app3.oneshot(req).await.unwrap();
        let json = body_json(resp).await;
        let has_key = json["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["key"] == "telegram:45");
        assert!(!has_key, "revoked entry must not remain in the list");
    }
}
