//! Turning rows into nodes, edges and text. Every write is an upsert, so
//! taking a row twice (a crash between a source's rows and its cursor) is
//! harmless; a source's rows and its cursor commit together anyway.

use crate::CatalogError;
use crate::sources::Source;
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;
use vak_session::tail::{self, Position};
use vak_session::types::{CallEffect, EntryPayload};

/// The node id of a session from its plain or `ses_` id.
pub(crate) fn session_node(id: &str) -> String {
    if id.starts_with("ses_") {
        id.to_string()
    } else {
        format!("ses_{id}")
    }
}

fn turn_node(id: &str) -> String {
    if id.starts_with("trn_") {
        id.to_string()
    } else {
        format!("trn_{id}")
    }
}

fn call_node(session: &str, call: &str) -> String {
    format!("call:{session}:{call}")
}

/// An Agent's catalog id: its `AgentId`, whatever form the record names it
/// in.
fn agent_id(agent: &str) -> String {
    if agent.starts_with("agt_") {
        agent.to_string()
    } else {
        vak_session::trace::local::agent(agent).to_string()
    }
}

/// Fields of a node; `None` leaves what is stored.
#[derive(Default)]
struct Upsert<'a> {
    id: &'a str,
    kind: &'a str,
    space: Option<String>,
    agent: Option<String>,
    agent_name: Option<String>,
    session: Option<String>,
    turn: Option<String>,
    run: Option<String>,
    actor: Option<String>,
    cause: Option<String>,
    audience: Option<String>,
    title: Option<String>,
    status: Option<String>,
    created_at: Option<String>,
    locator: Option<String>,
}

fn upsert(tx: &Transaction<'_>, node: Upsert<'_>) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO nodes (id, kind, space, agent, session, turn, run, actor, cause, audience,
                            title, status, created_at, locator, agent_name)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(id) DO UPDATE SET
            space = IFNULL(excluded.space, space),
            agent = IFNULL(excluded.agent, agent),
            agent_name = IFNULL(excluded.agent_name, agent_name),
            session = IFNULL(excluded.session, session),
            turn = IFNULL(excluded.turn, turn),
            run = IFNULL(excluded.run, run),
            actor = IFNULL(excluded.actor, actor),
            cause = IFNULL(excluded.cause, cause),
            audience = IFNULL(excluded.audience, audience),
            title = IFNULL(excluded.title, title),
            status = IFNULL(excluded.status, status),
            created_at = IFNULL(created_at, excluded.created_at),
            locator = IFNULL(excluded.locator, locator)",
        params![
            node.id,
            node.kind,
            node.space,
            node.agent,
            node.session,
            node.turn,
            node.run,
            node.actor,
            node.cause,
            node.audience,
            node.title,
            node.status,
            node.created_at,
            node.locator,
            node.agent_name
        ],
    )?;
    Ok(())
}

fn edge(tx: &Transaction<'_>, src: &str, kind: &str, dst: &str) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO edges (src, kind, dst) VALUES (?1, ?2, ?3)",
        params![src, kind, dst],
    )?;
    Ok(())
}

/// Adds `text` to a node's searchable body.
fn append_text(tx: &Transaction<'_>, node: &str, text: &str) -> rusqlite::Result<()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO texts (node, body) VALUES (?1, ?2)
         ON CONFLICT(node) DO UPDATE SET body = body || char(10) || excluded.body",
        params![node, text],
    )?;
    Ok(())
}

/// Replaces a node's searchable body.
fn set_text(tx: &Transaction<'_>, node: &str, text: &str) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO texts (node, body) VALUES (?1, ?2)
         ON CONFLICT(node) DO UPDATE SET body = excluded.body",
        params![node, text.trim()],
    )?;
    Ok(())
}

/// Takes `source`'s rows after `from`; returns where it stopped and how
/// many rows it took.
pub(crate) fn source(
    tx: &Transaction<'_>,
    source: &Source,
    from: Position,
) -> Result<(Position, usize), CatalogError> {
    let mut rows = 0;
    let mut failed: Option<rusqlite::Error> = None;
    let mut walk =
        |dir: &std::path::Path,
         rows: &mut usize,
         ingest: &mut dyn FnMut(Position, &[u8]) -> rusqlite::Result<()>| {
            tail::tail(dir, from, |after, bytes| {
                let at = Position {
                    segment: after.segment,
                    frames: after.frames.saturating_sub(1),
                };
                match ingest(at, bytes) {
                    Ok(()) => {
                        *rows += 1;
                        true
                    }
                    Err(error) => {
                        failed = Some(error);
                        false
                    }
                }
            })
        };
    let to = match source {
        Source::Session(dir) => {
            let mut session = SessionContext::load(tx, dir)?;
            walk(dir, &mut rows, &mut |at, bytes| {
                session_entry(tx, &mut session, dir, at, bytes)
            })
        }
        Source::Runs(dir) => walk(dir, &mut rows, &mut |_, bytes| run_row(tx, bytes)),
        Source::Effects(dir) => walk(dir, &mut rows, &mut |_, bytes| effect_row(tx, bytes)),
        Source::Commitments { dir, agent } => walk(dir, &mut rows, &mut |_, bytes| {
            commitment_row(tx, agent, bytes)
        }),
        Source::Trigger(path) => {
            let versions = vak_session::documents::version_count(path) as u64;
            if versions > from.frames {
                if let Ok(Some(content)) = vak_session::documents::read(path) {
                    trigger_document(tx, path, &content)?;
                }
                rows += 1;
            }
            Position {
                segment: 0,
                frames: versions.max(from.frames),
            }
        }
        Source::Intake { dir, tenant } => {
            let objects = vak_session::objects::TenantObjects::for_tenant(tenant).ok();
            walk(dir, &mut rows, &mut |_, bytes| {
                intake_row(tx, objects.as_deref(), bytes)
            })
        }
        Source::Artifacts(dir) => walk(dir, &mut rows, &mut |_, bytes| artifact_row(tx, bytes)),
        Source::Grants(dir) => walk(dir, &mut rows, &mut |_, bytes| grant_row(tx, bytes)),
        Source::IntakeSource(path) => {
            let versions = vak_session::documents::version_count(path) as u64;
            if versions > from.frames {
                match vak_session::documents::read(path) {
                    Ok(Some(content)) => source_document(tx, path, &content)?,
                    _ => forget_source(tx, path)?,
                }
                rows += 1;
            }
            Position {
                segment: 0,
                frames: versions.max(from.frames),
            }
        }
        Source::Memory { path, agent } | Source::Entity { path, agent } => {
            let memory = matches!(source, Source::Memory { .. });
            let versions = vak_session::documents::version_count(path) as u64;
            if versions > from.frames {
                match vak_session::documents::read(path) {
                    Ok(Some(content)) if memory => memory_document(tx, path, agent, &content)?,
                    Ok(Some(content)) => entity_document(tx, path, agent, &content)?,
                    // Forgotten: it leaves search.
                    _ => forget_document(tx, path, memory)?,
                }
                rows += 1;
            }
            Position {
                segment: 0,
                frames: versions.max(from.frames),
            }
        }
    };
    if let Some(error) = failed {
        return Err(error.into());
    }
    Ok((to, rows))
}

/// What every entry of one session inherits: the session's node and the
/// fields its turns and calls copy from it.
struct SessionContext {
    node: Option<String>,
    agent: Option<String>,
    space: Option<String>,
    audience: Option<String>,
}

impl SessionContext {
    fn load(tx: &Transaction<'_>, dir: &std::path::Path) -> rusqlite::Result<Self> {
        let Some(id) = vak_config::scope::ledger_session_id(dir) else {
            return Ok(Self {
                node: None,
                agent: None,
                space: None,
                audience: None,
            });
        };
        let node = session_node(&id);
        let stored = tx
            .query_row(
                "SELECT agent, space, audience FROM nodes WHERE id = ?1",
                [&node],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (agent, space, audience) = stored.unwrap_or((None, None, None));
        Ok(Self {
            node: Some(node),
            agent,
            space,
            audience,
        })
    }
}

fn session_entry(
    tx: &Transaction<'_>,
    session: &mut SessionContext,
    dir: &std::path::Path,
    at: Position,
    bytes: &[u8],
) -> rusqlite::Result<()> {
    let Some(entry) = vak_session::SessionLog::decode(bytes) else {
        return Ok(());
    };
    let Some(session_id) = session.node.clone() else {
        return Ok(());
    };
    let created = entry.ts.to_rfc3339();
    crate::history::index_entry(tx, &session_id, at, &entry)?;
    tx.execute(
        "UPDATE nodes SET size = IFNULL(size, 0) + 1, updated_at = ?2 WHERE id = ?1",
        params![session_id, created],
    )?;
    if let EntryPayload::Header(header) = &entry.payload {
        let name = header
            .agent
            .as_ref()
            .map_or("vak", |agent| agent.id.as_str())
            .to_string();
        let agent = agent_id(&name);
        // A ledger lives at `sessions/<space>/<session>`: its directory
        // names the space when the header does not.
        let space = header.space.map(|space| space.to_string()).or_else(|| {
            dir.parent()
                .and_then(|parent| parent.file_name())
                .map(|name| name.to_string_lossy().into_owned())
        });
        let audience = header
            .conversation
            .as_ref()
            .map(|conversation| conversation.audience_id.clone());
        upsert(
            tx,
            Upsert {
                id: &session_id,
                kind: "session",
                space: space.clone(),
                agent: Some(agent.clone()),
                agent_name: Some(name),
                session: Some(session_id.clone()),
                run: header.run.map(|run| run.to_string()),
                cause: header
                    .cause
                    .as_ref()
                    .map(|cause| vak_session::runs::cause_kind(cause).to_string()),
                audience: audience.clone(),
                created_at: Some(created.clone()),
                locator: Some(dir.display().to_string()),
                ..Default::default()
            },
        )?;
        tx.execute(
            "UPDATE nodes SET size = 1, updated_at = ?2 WHERE id = ?1",
            params![session_id, created],
        )?;
        if let Some(run) = header.run {
            edge(tx, &session_id, "caused_by", &run.to_string())?;
        }
        if let Some(parent) = &header.parent_session_id {
            edge(tx, &session_id, "caused_by", &session_node(parent))?;
        }
        session.agent = Some(agent);
        session.space = space;
        session.audience = audience;
        return Ok(());
    }
    let Some(turn) = entry.at_turn.as_deref().map(turn_node) else {
        // Text written outside any turn belongs to the session itself.
        if let EntryPayload::Message(record) = &entry.payload
            && record.control_kind().is_none()
        {
            for block in &record.message.content {
                if let vak_llm::types::ContentBlock::Text { text } = block {
                    append_text(tx, &session_id, text)?;
                }
            }
        }
        return Ok(());
    };
    upsert(
        tx,
        Upsert {
            id: &turn,
            kind: "turn",
            space: session.space.clone(),
            agent: session.agent.clone(),
            session: Some(session_id.clone()),
            turn: Some(turn.clone()),
            audience: session.audience.clone(),
            created_at: Some(created.clone()),
            ..Default::default()
        },
    )?;
    edge(tx, &turn, "part_of", &session_id)?;
    let call = |tx: &Transaction<'_>, id: &str, tool: Option<&str>| -> rusqlite::Result<String> {
        let node = call_node(&session_id, id);
        upsert(
            tx,
            Upsert {
                id: &node,
                kind: "call",
                space: session.space.clone(),
                agent: session.agent.clone(),
                session: Some(session_id.clone()),
                turn: Some(turn.clone()),
                audience: session.audience.clone(),
                title: tool.map(str::to_string),
                created_at: Some(created.clone()),
                ..Default::default()
            },
        )?;
        edge(tx, &node, "produced_by", &turn)?;
        Ok(node)
    };
    match &entry.payload {
        EntryPayload::Message(record) => {
            if record.control_kind().is_some() {
                return Ok(());
            }
            for block in &record.message.content {
                match block {
                    vak_llm::types::ContentBlock::Text { text } => append_text(tx, &turn, text)?,
                    vak_llm::types::ContentBlock::ToolUse { id, name, input } => {
                        let node = call(tx, id, Some(name))?;
                        tx.execute(
                            "INSERT INTO calls (node, tool, input) VALUES (?1, ?2, ?3)
                             ON CONFLICT(node) DO NOTHING",
                            params![node, name, input.to_string()],
                        )?;
                    }
                    vak_llm::types::ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        let node = call(tx, tool_use_id, None)?;
                        let (tool, input): (String, String) = tx
                            .query_row(
                                "SELECT tool, input FROM calls WHERE node = ?1",
                                [&node],
                                |row| Ok((row.get(0)?, row.get(1)?)),
                            )
                            .optional()?
                            .unwrap_or_default();
                        let evidence = vak_session::turns::Evidence {
                            tool,
                            input: serde_json::from_str(&input).unwrap_or(Value::Null),
                            content: content.clone(),
                            is_error: *is_error,
                        };
                        let digest = vak_session::turns::evidence_digest(tool_use_id, &evidence);
                        append_text(tx, &node, &digest)?;
                    }
                    _ => {}
                }
            }
        }
        EntryPayload::CallEffect(record) => {
            let node = call(tx, &record.tool_use_id, None)?;
            let (path, kind) = match &record.effect {
                CallEffect::FileWrite { path, .. } => (path, "produced_by"),
                CallEffect::FileRead { path, .. } => (path, "references"),
                _ => return Ok(()),
            };
            let file = format!(
                "file:{}:{path}",
                session.space.as_deref().unwrap_or("unbound")
            );
            upsert(
                tx,
                Upsert {
                    id: &file,
                    kind: "file",
                    space: session.space.clone(),
                    agent: session.agent.clone(),
                    audience: session.audience.clone(),
                    title: Some(path.clone()),
                    created_at: Some(created),
                    ..Default::default()
                },
            )?;
            if kind == "produced_by" {
                edge(tx, &file, "produced_by", &node)?;
            } else {
                edge(tx, &node, "references", &file)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn run_row(tx: &Transaction<'_>, bytes: &[u8]) -> rusqlite::Result<()> {
    use vak_session::runs::{RunEvent, RunStep};
    let Ok(event) = serde_json::from_slice::<RunEvent>(bytes) else {
        return Ok(());
    };
    let run = event.run.to_string();
    let at = event.at.to_rfc3339();
    match event.step {
        RunStep::Opened { trigger, .. } => {
            let trace = event.trace.as_ref();
            upsert(
                tx,
                Upsert {
                    id: &run,
                    kind: "run",
                    space: trace.map(|trace| trace.space.to_string()),
                    agent: trace.map(|trace| trace.agent.to_string()),
                    session: trace.and_then(|trace| trace.session.map(|s| s.to_string())),
                    turn: trace.and_then(|trace| trace.turn.map(|t| t.to_string())),
                    run: Some(run.clone()),
                    actor: event
                        .actor
                        .or(trace.and_then(|trace| trace.actor))
                        .map(|a| a.to_string()),
                    cause: trace
                        .map(|trace| vak_session::runs::cause_kind(&trace.cause).to_string()),
                    status: Some("running".into()),
                    created_at: Some(at),
                    locator: Some("runs".into()),
                    ..Default::default()
                },
            )?;
            if let Some(trigger) = trigger {
                edge(tx, &run, "caused_by", &trigger.to_string())?;
            }
            if let Some(trace) = trace {
                if let vak_session::trace::Cause::Delegation { parent_run, .. } = &trace.cause {
                    edge(tx, &run, "caused_by", &parent_run.to_string())?;
                }
                if let Some(turn) = trace.turn {
                    edge(tx, &turn.to_string(), "produced_by", &run)?;
                }
            }
        }
        RunStep::Session { session_id } => {
            edge(tx, &session_node(&session_id), "caused_by", &run)?;
        }
        step => {
            let status = serde_json::to_value(&step)
                .ok()
                .and_then(|value| {
                    value
                        .get("outcome")
                        .or_else(|| value.get("step"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "settled".into());
            upsert(
                tx,
                Upsert {
                    id: &run,
                    kind: "run",
                    status: Some(status),
                    ..Default::default()
                },
            )?;
        }
    }
    Ok(())
}

fn effect_row(tx: &Transaction<'_>, bytes: &[u8]) -> rusqlite::Result<()> {
    use vak_session::effects::{EffectEvent, EffectStep};
    let Ok(event) = serde_json::from_slice::<EffectEvent>(bytes) else {
        return Ok(());
    };
    let effect = event.effect.to_string();
    let status = match &event.step {
        EffectStep::Prepared { hold, .. } => {
            if hold.is_some() {
                "held"
            } else {
                "queued"
            }
        }
        EffectStep::Dispatched { .. } => "sending",
        EffectStep::Accepted { .. } | EffectStep::Confirmed { .. } => "sent",
        _ => {
            let value = serde_json::to_value(&event.step).unwrap_or(Value::Null);
            return upsert(
                tx,
                Upsert {
                    id: &effect,
                    kind: "effect",
                    status: value
                        .get("step")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    ..Default::default()
                },
            );
        }
    };
    if let EffectStep::Prepared {
        kind,
        run,
        supersedes,
        ..
    } = &event.step
    {
        let trace = event.trace.as_ref();
        let run = run
            .or(trace.map(|trace| trace.run))
            .map(|run| run.to_string());
        upsert(
            tx,
            Upsert {
                id: &effect,
                kind: "effect",
                space: trace.map(|trace| trace.space.to_string()),
                agent: trace.map(|trace| trace.agent.to_string()),
                run: run.clone(),
                actor: event.actor.map(|actor| actor.to_string()),
                title: Some(kind.name().to_string()),
                status: Some(status.into()),
                created_at: Some(event.at.to_rfc3339()),
                locator: Some("effects".into()),
                ..Default::default()
            },
        )?;
        if let Some(run) = run {
            edge(tx, &effect, "produced_by", &run)?;
        }
        if let Some(previous) = supersedes {
            edge(tx, &effect, "supersedes", &previous.to_string())?;
        }
    } else {
        upsert(
            tx,
            Upsert {
                id: &effect,
                kind: "effect",
                status: Some(status.into()),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

fn commitment_row(tx: &Transaction<'_>, agent: &str, bytes: &[u8]) -> rusqlite::Result<()> {
    let Ok(mut event) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(());
    };
    if !matches!(
        vak_session::content::restore(&mut event),
        Ok(vak_session::content::Restored::Whole)
    ) {
        return Ok(());
    }
    let Some(id) = event.get("commitment_id").and_then(Value::as_str) else {
        return Ok(());
    };
    let node = format!("commitment:{id}");
    let kind = event
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let trace = event
        .get("trace")
        .cloned()
        .and_then(|trace| serde_json::from_value::<vak_session::trace::TraceKey>(trace).ok());
    if kind == "opened" {
        let objective = event
            .pointer("/spec/objective")
            .and_then(Value::as_str)
            .unwrap_or_default();
        upsert(
            tx,
            Upsert {
                id: &node,
                kind: "commitment",
                space: trace.as_ref().map(|trace| trace.space.to_string()),
                agent: Some(agent_id(agent)),
                run: trace.as_ref().map(|trace| trace.run.to_string()),
                title: Some(objective.to_string()),
                status: Some("open".into()),
                created_at: event.get("ts").and_then(Value::as_str).map(str::to_string),
                locator: Some("commitments".into()),
                ..Default::default()
            },
        )?;
        set_text(tx, &node, objective)?;
        if let Some(turn) = trace.as_ref().and_then(|trace| trace.turn) {
            edge(tx, &node, "produced_by", &turn.to_string())?;
        } else if let Some(trace) = &trace {
            edge(tx, &node, "produced_by", &trace.run.to_string())?;
        }
    } else if kind == "closed" {
        upsert(
            tx,
            Upsert {
                id: &node,
                kind: "commitment",
                status: Some("closed".into()),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

fn trigger_document(
    tx: &Transaction<'_>,
    path: &std::path::Path,
    content: &str,
) -> rusqlite::Result<()> {
    let Ok(trigger) = serde_json::from_str::<Value>(content) else {
        return Ok(());
    };
    let Some(id) = trigger.get("id").and_then(Value::as_str) else {
        return Ok(());
    };
    let name = trigger
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    upsert(
        tx,
        Upsert {
            id,
            kind: "trigger",
            space: trigger
                .get("space")
                .and_then(Value::as_str)
                .map(str::to_string),
            agent: trigger.get("agent").and_then(Value::as_str).map(agent_id),
            title: Some(name.to_string()),
            status: Some(
                if trigger.get("enabled").and_then(Value::as_bool) == Some(false) {
                    "paused"
                } else {
                    "enabled"
                }
                .into(),
            ),
            created_at: trigger
                .get("created_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            locator: Some(path.display().to_string()),
            ..Default::default()
        },
    )?;
    set_text(tx, id, name)
}

/// A memory note: one Document, `## <time> [<kind>] tag=… session=…
/// turn=…` and then its text. Notes in the profile tier (`user/`) belong to
/// the owner's local audience; workspace notes to their Agent.
fn memory_document(
    tx: &Transaction<'_>,
    path: &std::path::Path,
    agent: &str,
    content: &str,
) -> rusqlite::Result<()> {
    let node = format!("memory:{}", path.display());
    let (header, text) = content.split_once('\n').unwrap_or(("", content));
    let header = header.trim_start_matches("## ");
    let created = header.split_whitespace().next().map(str::to_string);
    let mut kind = "note".to_string();
    let mut tag = String::new();
    let mut session = None;
    let mut turn = None;
    for word in header.split_whitespace().skip(1) {
        if let Some(inner) = word.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
            kind = inner.to_string();
        } else if let Some(value) = word.strip_prefix("tag=") {
            tag = value.to_string();
        } else if let Some(id) = word.strip_prefix("session=") {
            session = Some(session_node(id));
        } else if let Some(id) = word.strip_prefix("turn=") {
            turn = Some(turn_node(id));
        }
    }
    // `…/memory/<space or user>/<tier>/<note id>`.
    let parts: Vec<String> = path
        .iter()
        .rev()
        .take(3)
        .map(|part| part.to_string_lossy().into_owned())
        .collect();
    let profile = parts.get(2).is_some_and(|tier| tier == "user");
    // A workspace note stays in the audience of the conversation it came
    // from; a profile note is the owner's, read locally.
    let audience: Option<String> = if profile {
        Some("local".into())
    } else {
        match &session {
            Some(session) => tx
                .query_row(
                    "SELECT audience FROM nodes WHERE id = ?1",
                    [session],
                    |row| row.get(0),
                )
                .optional()?
                .flatten(),
            None => None,
        }
    };
    upsert(
        tx,
        Upsert {
            id: &node,
            kind: "memory",
            space: if profile { None } else { parts.get(2).cloned() },
            agent: Some(agent_id(agent)),
            agent_name: Some(agent.to_string()),
            session: session.clone(),
            turn: turn.clone(),
            audience,
            title: parts.first().cloned(),
            status: Some(if profile { "profile" } else { "workspace" }.into()),
            created_at: created,
            locator: Some(path.display().to_string()),
            ..Default::default()
        },
    )?;
    if let Some(turn) = turn {
        edge(tx, &node, "produced_by", &turn)?;
    } else if let Some(session) = session {
        edge(tx, &node, "produced_by", &session)?;
    }
    let label = if tag.is_empty() {
        kind
    } else {
        format!("{kind} {tag}")
    };
    set_text(tx, &node, &format!("[{label}] {}", text.trim()))
}

/// An entity: one Document of JSON with its name, type and summary.
fn entity_document(
    tx: &Transaction<'_>,
    path: &std::path::Path,
    agent: &str,
    content: &str,
) -> rusqlite::Result<()> {
    let Ok(entity) = serde_json::from_str::<Value>(content) else {
        return Ok(());
    };
    let node = format!("entity:{}", path.display());
    let field = |name: &str| entity.get(name).and_then(Value::as_str).unwrap_or_default();
    let attributes = entity
        .get("attributes")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(key, value)| format!("{key}: {}", value.as_str().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    // `…/entities/<space or global>/ENTITIES.jsonl/<id>`.
    let space = path
        .iter()
        .rev()
        .nth(2)
        .map(|part| part.to_string_lossy().into_owned())
        .filter(|space| space != "global");
    upsert(
        tx,
        Upsert {
            id: &node,
            kind: "entity",
            space,
            agent: Some(agent_id(agent)),
            agent_name: Some(agent.to_string()),
            title: Some(field("name").to_string()),
            status: Some(field("entity_type").to_string()),
            created_at: entity
                .get("updated_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            locator: Some(path.display().to_string()),
            ..Default::default()
        },
    )?;
    set_text(
        tx,
        &node,
        &format!(
            "[entity:{}] {} — {} {attributes}",
            field("entity_type"),
            field("name"),
            field("summary")
        ),
    )
}

/// A forgotten memory note or entity leaves search and the catalog.
fn forget_document(
    tx: &Transaction<'_>,
    path: &std::path::Path,
    memory: bool,
) -> rusqlite::Result<()> {
    let kind = if memory { "memory" } else { "entity" };
    let node = format!("{kind}:{}", path.display());
    tx.execute("DELETE FROM texts WHERE node = ?1", [&node])?;
    tx.execute("DELETE FROM edges WHERE src = ?1 OR dst = ?1", [&node])?;
    tx.execute("DELETE FROM nodes WHERE id = ?1", [&node])?;
    Ok(())
}

/// One row of the `intake/` chain (plan M6.5): a `taken` row makes the
/// item's node, its lineage and its text (title and body, read from its
/// object); a person's `released` or `quarantined` row moves its status.
fn intake_row(
    tx: &Transaction<'_>,
    objects: Option<&vak_session::objects::TenantObjects>,
    bytes: &[u8],
) -> rusqlite::Result<()> {
    use vak_session::objects::Objects;
    let Ok(mut row) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(());
    };
    if !matches!(
        vak_session::content::restore(&mut row),
        Ok(vak_session::content::Restored::Whole)
    ) {
        return Ok(());
    }
    let Some(item) = row.get("item").and_then(Value::as_str) else {
        return Ok(());
    };
    let text = |key: &str| row.get(key).and_then(Value::as_str).map(str::to_string);
    match row.get("step").and_then(Value::as_str) {
        Some("taken") => {
            let trace = row.get("trace");
            let field = |key: &str| {
                trace
                    .and_then(|trace| trace.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            let run = field("run");
            let source = text("source");
            let title = text("title").unwrap_or_default();
            upsert(
                tx,
                Upsert {
                    id: item,
                    kind: "item",
                    agent: text("agent").as_deref().map(agent_id),
                    agent_name: text("agent"),
                    run: run.clone(),
                    actor: text("actor"),
                    cause: trace
                        .and_then(|trace| trace.get("cause"))
                        .and_then(|cause| cause.get("kind"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    title: Some(title.clone()),
                    status: text("disposition"),
                    created_at: text("published").or_else(|| text("at")),
                    locator: text("link"),
                    ..Default::default()
                },
            )?;
            if let Some(run) = &run {
                edge(tx, item, "produced_by", run)?;
            }
            if let Some(source) = &source {
                edge(tx, item, "derived_from", source)?;
            }
            let body = row
                .get("object")
                .and_then(|object| serde_json::from_value(object.clone()).ok())
                .zip(objects)
                .zip(source.as_ref())
                .and_then(|((object, objects), source)| {
                    objects.get(&object, &format!("intake:{source}")).ok()
                })
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|body| body.get("text").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            set_text(tx, item, &format!("{title}\n{body}"))
        }
        Some(step @ ("released" | "quarantined")) => upsert(
            tx,
            Upsert {
                id: item,
                kind: "item",
                status: Some(
                    if step == "released" {
                        "accepted"
                    } else {
                        "quarantined"
                    }
                    .into(),
                ),
                ..Default::default()
            },
        ),
        _ => Ok(()),
    }
}

/// An intake source Document.
fn source_document(
    tx: &Transaction<'_>,
    path: &std::path::Path,
    content: &str,
) -> rusqlite::Result<()> {
    let Ok(source) = serde_json::from_str::<Value>(content) else {
        return Ok(());
    };
    let Some(id) = source.get("id").and_then(Value::as_str) else {
        return Ok(());
    };
    let name = source
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let agent = source.get("agent").and_then(Value::as_str);
    upsert(
        tx,
        Upsert {
            id,
            kind: "source",
            agent: agent.map(agent_id),
            agent_name: agent.map(str::to_string),
            actor: source
                .get("created_by")
                .and_then(Value::as_str)
                .map(str::to_string),
            title: Some(name.to_string()),
            created_at: source
                .get("created_at")
                .and_then(Value::as_str)
                .map(str::to_string),
            locator: Some(path.display().to_string()),
            ..Default::default()
        },
    )?;
    if let Some(trigger) = source.get("trigger").and_then(Value::as_str) {
        edge(tx, trigger, "references", id)?;
    }
    set_text(tx, id, name)
}

/// A removed source leaves search; its items stay until erasure.
fn forget_source(tx: &Transaction<'_>, path: &std::path::Path) -> rusqlite::Result<()> {
    let locator = path.display().to_string();
    tx.execute(
        "DELETE FROM texts WHERE node IN (SELECT id FROM nodes WHERE kind = 'source' AND locator = ?1)",
        [&locator],
    )?;
    tx.execute(
        "DELETE FROM nodes WHERE kind = 'source' AND locator = ?1",
        [&locator],
    )?;
    Ok(())
}

/// One row of the `artifacts/` chain (plan M8): a declaration makes the
/// artifact's node and text; a version made by a call is produced by that
/// call and its run; a rename or archive moves its title or status.
fn artifact_row(tx: &Transaction<'_>, bytes: &[u8]) -> rusqlite::Result<()> {
    let Ok(mut row) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(());
    };
    if !matches!(
        vak_session::content::restore(&mut row),
        Ok(vak_session::content::Restored::Whole)
    ) {
        return Ok(());
    }
    let Some(artifact) = row.get("artifact").and_then(Value::as_str) else {
        return Ok(());
    };
    let text = |key: &str| row.get(key).and_then(Value::as_str).map(str::to_string);
    let run = row
        .pointer("/trace/run")
        .and_then(Value::as_str)
        .map(str::to_string);
    match row.get("step").and_then(Value::as_str) {
        Some("declared") => {
            let path = text("path").unwrap_or_default();
            let title = text("title");
            upsert(
                tx,
                Upsert {
                    id: artifact,
                    kind: "artifact",
                    space: text("space"),
                    agent: text("agent").as_deref().map(agent_id),
                    agent_name: text("agent"),
                    run: run.clone(),
                    actor: text("actor"),
                    title: title.clone().or_else(|| Some(path.clone())),
                    status: Some("active".into()),
                    created_at: text("at"),
                    locator: Some(path.clone()),
                    ..Default::default()
                },
            )?;
            let body = [
                title.unwrap_or_default(),
                text("summary").unwrap_or_default(),
                path,
            ];
            append_text(tx, artifact, &body.join("\n"))
        }
        Some("versioned") => {
            if let (Some("call"), Some(session), Some(call)) = (
                row.get("from").and_then(Value::as_str),
                row.get("session").and_then(Value::as_str),
                row.get("call").and_then(Value::as_str),
            ) && !session.is_empty()
            {
                edge(
                    tx,
                    artifact,
                    "produced_by",
                    &call_node(&session_node(session), call),
                )?;
            }
            if let Some(run) = &run {
                edge(tx, artifact, "produced_by", run)?;
            }
            Ok(())
        }
        Some("renamed") => upsert(
            tx,
            Upsert {
                id: artifact,
                kind: "artifact",
                title: text("title"),
                ..Default::default()
            },
        ),
        Some("archived") => upsert(
            tx,
            Upsert {
                id: artifact,
                kind: "artifact",
                status: Some(
                    if row.get("on").and_then(Value::as_bool) == Some(true) {
                        "archived"
                    } else {
                        "active"
                    }
                    .into(),
                ),
                ..Default::default()
            },
        ),
        _ => Ok(()),
    }
}

/// The catalog node a grant's object names.
fn granted_node(object: &Value) -> Option<String> {
    let id = object.get("id").and_then(Value::as_str)?;
    match object.get("kind").and_then(Value::as_str)? {
        "artifact" => Some(id.to_string()),
        "conversation" => Some(session_node(id)),
        _ => None,
    }
}

/// One row of the `grants/` chain (plan M8.2): what it opens, to whom,
/// until when, and which objects no longer inherit.
fn grant_row(tx: &Transaction<'_>, bytes: &[u8]) -> rusqlite::Result<()> {
    let Ok(row) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(());
    };
    match row.get("step").and_then(Value::as_str) {
        Some("granted") => {
            let Some(grant) = row.get("grant") else {
                return Ok(());
            };
            let field = |key: &str| grant.get(key).and_then(Value::as_str);
            let (Some(id), Some(principal), Some(node)) = (
                field("id"),
                field("principal"),
                grant.get("object").and_then(granted_node),
            ) else {
                return Ok(());
            };
            // In the format SQLite's strftime produces, so expiry compares.
            let expires = field("expires_at")
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .map(|at| {
                    at.with_timezone(&chrono::Utc)
                        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                        .to_string()
                });
            tx.execute(
                "INSERT OR IGNORE INTO grants (grant_id, node, principal, role, expires)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    node,
                    principal,
                    field("role").unwrap_or("viewer"),
                    expires
                ],
            )?;
        }
        Some("revoked") => {
            if let Some(id) = row.get("grant").and_then(Value::as_str) {
                tx.execute("UPDATE grants SET revoked = 1 WHERE grant_id = ?1", [id])?;
            }
        }
        Some("inheritance_broken") => {
            if let Some(node) = row.get("object").and_then(granted_node) {
                tx.execute("INSERT OR IGNORE INTO broken (node) VALUES (?1)", [node])?;
            }
        }
        Some("inheritance_restored") => {
            if let Some(node) = row.get("object").and_then(granted_node) {
                tx.execute("DELETE FROM broken WHERE node = ?1", [node])?;
            }
        }
        _ => {}
    }
    Ok(())
}
