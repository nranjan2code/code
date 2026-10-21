use std::path::PathBuf;

use vak_catalog::Node;
use vak_core::Core;
use vak_session::trace::TraceKey;
use vak_session::work_threads::{TopicId, TopicSnapshot, TopicStore};

use crate::cli::TopicsAction;

const PAGE: usize = 200;

pub(crate) async fn run_topics(
    cwd: PathBuf,
    session: Option<String>,
    agent: Option<String>,
    expected_revision: Option<u64>,
    action: TopicsAction,
) -> i32 {
    match run(cwd, session, agent, expected_revision, action).await {
        Ok(output) => {
            if !output.is_empty() {
                println!("{output}");
            }
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

async fn run(
    cwd: PathBuf,
    session: Option<String>,
    agent: Option<String>,
    expected_revision: Option<u64>,
    action: TopicsAction,
) -> Result<String, String> {
    let core = Core::new(cwd).map_err(|error| error.to_string())?;
    let catalog = core.catalog().map_err(|error| error.to_string())?;
    let _ = catalog.catch_up().map_err(|error| error.to_string())?;
    match action {
        TopicsAction::List {
            query,
            offset,
            limit,
        } => {
            let limit = limit.clamp(1, 100);
            let query = query.map(|query| query.to_lowercase());
            let agent = agent
                .as_deref()
                .map(|filter| resolve_agent_filter(&core, filter))
                .transpose()?;
            if let Some(session_id) = session.as_deref() {
                let (node, store) = session_store(&core, &catalog, session_id)?;
                let snapshot = store.snapshot().map_err(|error| error.to_string())?;
                let mut matching = snapshot
                    .topics
                    .iter()
                    .filter(|topic| {
                        query
                            .as_ref()
                            .is_none_or(|query| topic.title.to_lowercase().contains(query))
                    })
                    .collect::<Vec<_>>();
                matching.sort_by(|left, right| {
                    right
                        .pinned
                        .cmp(&left.pinned)
                        .then_with(|| right.updated_at.cmp(&left.updated_at))
                        .then_with(|| right.id.cmp(&left.id))
                });
                let total = matching.len();
                let topics = matching
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|topic| {
                        let count = snapshot
                            .placements
                            .iter()
                            .filter(|placement| placement.topic_ids.contains(&topic.id))
                            .count();
                        serde_json::json!({
                            "id": topic.id,
                            "title": topic.title,
                            "archived": topic.archived,
                            "pinned": topic.pinned,
                            "placement_count": count
                        })
                    })
                    .collect::<Vec<_>>();
                return render_page(
                    topics,
                    offset,
                    limit,
                    total,
                    Some(snapshot.automatic),
                    Some(snapshot.revision),
                    Some(node.agent_name.unwrap_or_else(|| "vak".into())),
                );
            }
            let audience = core.catalog_audience();
            let mut before: Option<(String, String)> = None;
            let mut matches = Vec::new();
            loop {
                let page = catalog
                    .topic_page(
                        &audience,
                        before.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
                        PAGE,
                    )
                    .map_err(|error| error.to_string())?;
                if page.is_empty() {
                    break;
                }
                for node in &page {
                    if query.as_ref().is_some_and(|query| {
                        !node
                            .title
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(query)
                    }) {
                        continue;
                    }
                    if agent.as_ref().is_some_and(|agent| {
                        !node.agent_name.as_deref().is_some_and(|name| name == agent)
                    }) {
                        continue;
                    }
                    matches.push(serde_json::json!({
                        "id": node.id.strip_prefix("topic:").unwrap_or(&node.id),
                        "title": node.title,
                        "archived": node.status.as_deref() == Some("archived"),
                        "agent": node.agent_name,
                        "session": node.session.as_deref().unwrap_or_default().strip_prefix("ses_").unwrap_or(node.session.as_deref().unwrap_or_default())
                    }));
                }
                let last = page.last().ok_or("catalog page was empty")?;
                let created_at = last.created_at.clone().unwrap_or_default();
                before = Some((created_at, last.id.clone()));
                if page.len() < PAGE {
                    break;
                }
            }
            let total = matches.len();
            let topics = matches.into_iter().skip(offset).take(limit).collect();
            render_page(topics, offset, limit, total, None, None, agent)
        }
        TopicsAction::Open { id, offset, limit } => {
            let (session_id, node) = if let Some(session_id) = session.as_deref() {
                let (node, _) = session_store(&core, &catalog, session_id)?;
                (session_id.to_owned(), node)
            } else {
                let node = find_topic_node(&core, &catalog, &id)?;
                let session_id = node
                    .session
                    .as_deref()
                    .and_then(|session| session.strip_prefix("ses_").or(Some(session)))
                    .ok_or("topic has no source conversation")?
                    .to_owned();
                (session_id, node)
            };
            let (_, store) = session_store(&core, &catalog, &session_id)?;
            let snapshot = store.snapshot().map_err(|error| error.to_string())?;
            let topic_id = TopicId::parse(&id).map_err(|_| "invalid Topic id")?;
            let topic = snapshot
                .topics
                .iter()
                .find(|topic| topic.id == topic_id)
                .ok_or("Topic was not found in its source conversation")?;
            let mut all = snapshot
                .placements
                .iter()
                .filter(|placement| placement.topic_ids.contains(&topic_id))
                .collect::<Vec<_>>();
            all.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| right.turn_id.cmp(&left.turn_id))
                    .then_with(|| left.outcome.cmp(&right.outcome))
            });
            let limit = limit.clamp(1, 100);
            let placements = all
                .iter()
                .skip(offset)
                .take(limit)
                .map(|placement| {
                    serde_json::json!({
                        "address": format!("turn:{}", placement.turn_id),
                        "turn_id": placement.turn_id,
                        "outcome": placement.outcome,
                        "source": placement.source
                    })
                })
                .collect::<Vec<_>>();
            pretty(&serde_json::json!({
                "topic": {"id": topic.id, "title": topic.title, "archived": topic.archived, "pinned": topic.pinned},
                "agent": node.agent_name,
                "session": session_id,
                "placements": placements,
                "offset": offset,
                "limit": limit,
                "total": all.len(),
                "has_more": offset.saturating_add(limit) < all.len(),
                "automatic": snapshot.automatic,
                "revision": snapshot.revision,
                "note": "Turn addresses identify membership; use recall to read a turn. Membership does not load turn contents."
            }))
        }
        action => {
            let session_id = session
                .as_deref()
                .ok_or("this action requires --session <uuid>")?;
            let (node, store) = session_store(&core, &catalog, session_id)?;
            let snapshot = store.snapshot().map_err(|error| error.to_string())?;
            if expected_revision.is_some_and(|expected| expected != snapshot.revision) {
                return Err(format!(
                    "Topics revision changed (current revision {})",
                    snapshot.revision
                ));
            }
            if let TopicsAction::Assign { turn, .. } = &action {
                let frame = catalog
                    .history_frame(session_id, turn)
                    .map_err(|error| error.to_string())?;
                if frame.is_none_or(|frame| !frame.directive) {
                    return Err("this request is not in this conversation".into());
                }
            }
            let source_agent = node.agent_name.as_deref().unwrap_or("vak");
            let trace = cli_trace(&core, session_id, source_agent)?;
            let span = vak_session::runs::span(&trace);
            let _entered = span.enter();
            let mut run = core
                .runs()
                .begin(&trace, None, None)
                .map_err(|error| error.to_string())?;
            core.runs()
                .session(run.id(), session_id)
                .map_err(|error| error.to_string())?;
            let result = mutate(&store, &snapshot, &trace, session_id, action);
            match &result {
                Ok(_) => run.settle_with(vak_session::runs::RunOutcome::Completed, None),
                Err(_) => run.settle_with(
                    vak_session::runs::RunOutcome::Failed {
                        reason: "Topics mutation failed".into(),
                    },
                    None,
                ),
            }
            result?;
            Ok(format!(
                "Topics updated for {}.",
                node.agent_name.unwrap_or_else(|| "vak".into())
            ))
        }
    }
}

fn mutate(
    store: &TopicStore,
    snapshot: &TopicSnapshot,
    trace: &TraceKey,
    session_id: &str,
    action: TopicsAction,
) -> Result<(), String> {
    use TopicsAction as A;
    let store = store.clone().at_revision(snapshot.revision);
    match action {
        A::Create { title, pinned } => store
            .create(&title, pinned, trace)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        A::Rename { id, title } => store
            .rename(&topic_id(&id)?, &title, trace)
            .map_err(|error| error.to_string()),
        A::Pin { id, pinned } => store
            .set_pinned(&topic_id(&id)?, pinned, trace)
            .map_err(|error| error.to_string()),
        A::Archive { id } => store
            .archive(&topic_id(&id)?, trace)
            .map_err(|error| error.to_string()),
        A::Reopen { id } => store
            .reopen(&topic_id(&id)?, trace)
            .map_err(|error| error.to_string()),
        A::Automatic { enabled } => store
            .set_automatic(enabled, trace)
            .map_err(|error| error.to_string()),
        A::Assign {
            turn,
            outcome,
            topics,
            whole_turn,
        } => {
            let ids = topics
                .iter()
                .map(|id| topic_id(id))
                .collect::<Result<Vec<_>, _>>()?;
            let turn_trace = trace.clone().in_turn(session_id, &turn);
            let changed = if whole_turn {
                store.assign_turn_human(&turn, ids, &turn_trace)
            } else {
                store.assign_human(&turn, &outcome, ids, &turn_trace)
            };
            changed.map(|_| ()).map_err(|error| error.to_string())
        }
        A::List { .. } | A::Open { .. } => Err("read action reached the mutation handler".into()),
    }
}

fn topic_id(id: &str) -> Result<TopicId, String> {
    TopicId::parse(id).map_err(|_| "invalid Topic id".into())
}

fn session_store(
    core: &Core,
    catalog: &vak_catalog::Catalog,
    session_id: &str,
) -> Result<(Node, TopicStore), String> {
    let audience = core.catalog_audience();
    let mut before: Option<(String, String)> = None;
    let mut found = None;
    loop {
        let page = catalog
            .session_page(
                &audience,
                before.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
                PAGE,
            )
            .map_err(|error| error.to_string())?;
        if page.is_empty() {
            break;
        }
        if let Some(node) = page.iter().find(|node| {
            node.session
                .as_deref()
                .and_then(|id| id.strip_prefix("ses_").or(Some(id)))
                == Some(session_id)
        }) {
            found = Some(node.clone());
            break;
        }
        let last = page.last().ok_or("catalog page was empty")?;
        before = Some((
            last.updated_at
                .clone()
                .or(last.created_at.clone())
                .unwrap_or_default(),
            last.id.clone(),
        ));
        if page.len() < PAGE {
            break;
        }
    }
    let node = found
        .ok_or_else(|| format!("conversation '{session_id}' is not visible in the catalog"))?;
    if vak_core::trash::state(&core.shared_scope(), session_id)
        .map_err(|_| "conversation lifecycle state is unavailable")?
        .hidden()
    {
        return Err("conversation is in the trash or has been erased".into());
    }
    let agent = node.agent_name.as_deref().unwrap_or("vak");
    let shared = core.shared_scope();
    let scope = shared.agent(agent);
    Ok((
        node,
        TopicStore::new(&scope, session_id).map_err(|error| error.to_string())?,
    ))
}

fn find_topic_node(core: &Core, catalog: &vak_catalog::Catalog, id: &str) -> Result<Node, String> {
    let target = format!("topic:{id}");
    let audience = core.catalog_audience();
    let mut before: Option<(String, String)> = None;
    loop {
        let page = catalog
            .topic_page(
                &audience,
                before.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
                PAGE,
            )
            .map_err(|error| error.to_string())?;
        if page.is_empty() {
            break;
        }
        if let Some(node) = page.iter().find(|node| node.id == target) {
            return Ok(node.clone());
        }
        let last = page.last().ok_or("catalog page was empty")?;
        before = Some((last.created_at.clone().unwrap_or_default(), last.id.clone()));
        if page.len() < PAGE {
            break;
        }
    }
    Err("Topic was not found in the visible catalog".into())
}

fn cli_trace(core: &Core, session_id: &str, agent_id: &str) -> Result<TraceKey, String> {
    let mut trace = core.mint_trace(None);
    let uuid = uuid::Uuid::parse_str(session_id).map_err(|_| "invalid conversation id")?;
    trace.session = vak_session::ids::SessionId::from_uuid(uuid);
    if trace.session.is_none() {
        return Err("invalid conversation id".into());
    }
    trace.agent = vak_session::trace::local::agent(agent_id);
    Ok(trace)
}

fn resolve_agent_filter(core: &Core, filter: &str) -> Result<String, String> {
    let profiles = vak_core::agent_definitions::effective(core).unwrap_or_default();
    Ok(profiles
        .iter()
        .find(|profile| agent_alias_matches(filter, &profile.id, &profile.name))
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| filter.to_string()))
}

fn agent_alias_matches(filter: &str, id: &str, name: &str) -> bool {
    id.eq_ignore_ascii_case(filter) || name.eq_ignore_ascii_case(filter)
}

fn render_page(
    topics: Vec<serde_json::Value>,
    offset: usize,
    limit: usize,
    total: usize,
    automatic: Option<bool>,
    revision: Option<u64>,
    agent: Option<String>,
) -> Result<String, String> {
    let has_more = offset.saturating_add(topics.len()) < total;
    pretty(
        &serde_json::json!({"topics":topics,"offset":offset,"limit":limit,"total":total,"has_more":has_more,"automatic":automatic,"revision":revision,"agent":agent}),
    )
}

fn pretty(value: &serde_json::Value) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::agent_alias_matches;

    #[test]
    fn agent_filter_accepts_id_or_display_name() {
        assert!(agent_alias_matches(
            "scout-id",
            "scout-id",
            "Research Scout"
        ));
        assert!(agent_alias_matches(
            "research scout",
            "scout-id",
            "Research Scout"
        ));
        assert!(!agent_alias_matches("other", "scout-id", "Research Scout"));
    }
}
