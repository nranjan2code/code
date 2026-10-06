//! Turning rows into nodes, edges and text. Every write is an upsert, so
//! taking a row twice (a crash between a source's rows and its cursor) is
//! harmless; a source's rows and its cursor commit together anyway.

use crate::CatalogError;
use crate::sources::Source;
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;
use vak_session::tail::{self, Position};
use vak_session::types::{CallEffect, Entry, EntryPayload};

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
                            title, status, created_at, locator)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(id) DO UPDATE SET
            space = IFNULL(excluded.space, space),
            agent = IFNULL(excluded.agent, agent),
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
            node.locator
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
    let mut walk = |dir: &std::path::Path,
                    rows: &mut usize,
                    ingest: &mut dyn FnMut(&[u8]) -> rusqlite::Result<()>| {
        tail::tail(dir, from, |_, bytes| match ingest(bytes) {
            Ok(()) => {
                *rows += 1;
                true
            }
            Err(error) => {
                failed = Some(error);
                false
            }
        })
    };
    let to = match source {
        Source::Session(dir) => {
            let mut session = SessionContext::load(tx, dir)?;
            walk(dir, &mut rows, &mut |bytes| {
                session_entry(tx, &mut session, dir, bytes)
            })
        }
        Source::Runs(dir) => walk(dir, &mut rows, &mut |bytes| run_row(tx, bytes)),
        Source::Effects(dir) => walk(dir, &mut rows, &mut |bytes| effect_row(tx, bytes)),
        Source::Commitments { dir, agent } => walk(dir, &mut rows, &mut |bytes| {
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
        Source::Memory { path, agent } => {
            let versions = vak_session::documents::version_count(path) as u64;
            if versions > from.frames {
                if let Ok(Some(content)) = vak_session::documents::read(path) {
                    memory_document(tx, path, agent, &content)?;
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
    bytes: &[u8],
) -> rusqlite::Result<()> {
    let Ok(entry) = serde_json::from_slice::<Entry>(bytes) else {
        return Ok(());
    };
    let Some(session_id) = session.node.clone() else {
        return Ok(());
    };
    let created = entry.ts.to_rfc3339();
    if let EntryPayload::Header(header) = &entry.payload {
        let agent = agent_id(
            header
                .agent
                .as_ref()
                .map_or("vak", |agent| agent.id.as_str()),
        );
        let space = header.space.map(|space| space.to_string());
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
                session: Some(session_id.clone()),
                run: header.run.map(|run| run.to_string()),
                cause: header
                    .cause
                    .as_ref()
                    .map(|cause| vak_session::runs::cause_kind(cause).to_string()),
                audience: audience.clone(),
                created_at: Some(created),
                locator: Some(dir.display().to_string()),
                ..Default::default()
            },
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
    let Ok(event) = serde_json::from_slice::<Value>(bytes) else {
        return Ok(());
    };
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

fn memory_document(
    tx: &Transaction<'_>,
    path: &std::path::Path,
    agent: &str,
    content: &str,
) -> rusqlite::Result<()> {
    let node = format!("memory:{}", path.display());
    upsert(
        tx,
        Upsert {
            id: &node,
            kind: "memory",
            agent: Some(agent_id(agent)),
            title: path
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string),
            locator: Some(path.display().to_string()),
            ..Default::default()
        },
    )?;
    set_text(tx, &node, content)
}
