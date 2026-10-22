//! Human history pages over catalog-addressed canonical ledger frames.
//! A page pairs transcript and presentation at one branch revision; the
//! catalog is an address index, never an authorization or content source.

use crate::AppState;
use axum::Json;
use axum::extract::{Path as RoutePath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use vak_catalog::{Catalog, HistoryFrame};
use vak_session::{Entry, EntryPayload, SessionLog};

// Transport budgets, independent of the model context/retention policies.
pub(crate) const PAGE_TURNS: usize = 40;
const PAGE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Default, Deserialize)]
pub(crate) struct PageQuery {
    pub before: Option<String>,
    pub since: Option<String>,
    pub around: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    session: String,
    leaf: String,
    end: String,
}

fn cursor(session: &str, leaf: &str, end: &str) -> Result<String, StatusCode> {
    let bytes = serde_json::to_vec(&Cursor {
        session: session.into(),
        leaf: leaf.into(),
        end: end.into(),
    })
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

fn decode(raw: &str) -> Result<Cursor, StatusCode> {
    if raw.len() > 2048 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)
}

pub(crate) fn list(
    state: &AppState,
    trash: bool,
    before: Option<&str>,
    visible: Option<&str>,
) -> Result<serde_json::Value, StatusCode> {
    let catalog = state
        .core
        .catalog()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    {
        let mut discovered = state
            .catalog_discovered
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if !*discovered {
            catalog
                .catch_up()
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            *discovered = true;
        }
    }
    let active = state.active_core();
    let mut boundary: Option<(String, String)> = match before {
        Some(raw) => {
            if raw.len() > 2048 {
                return Err(StatusCode::BAD_REQUEST);
            }
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(raw)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            Some(serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?)
        }
        None => None,
    };
    let summary = |node: &vak_catalog::Node| -> Result<Option<serde_json::Value>, StatusCode> {
        let id = node.id.strip_prefix("ses_").unwrap_or(&node.id);
        let lifecycle = vak_core::trash::state(&state.core.shared_scope(), id)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if lifecycle.erased_at.is_some() || lifecycle.trashed_at.is_some() != trash {
            return Ok(None);
        }
        let Some(path) = &node.locator else {
            return Ok(None);
        };
        let header = match SessionLog::read_header(Path::new(path)) {
            Ok(header) => header,
            Err(_) => return Ok(None),
        };
        if header.session_id != id
            || (header.agent.as_ref().is_none_or(|agent| agent.id == "vak")
                && header.contract_cwd() != *active.cwd())
        {
            return Ok(None);
        }
        let handle = state.get(id);
        if node.size.unwrap_or(0) <= 1 && handle.is_none() {
            return Ok(None);
        }
        let running = handle
            .as_ref()
            .is_some_and(|handle| handle.session.lock().map_or(true, |log| log.is_none()));
        Ok(Some(
            serde_json::json!({"session_id": id, "cwd": header.contract_cwd(), "created_at": header.created_at,
            "updated_at": node.updated_at, "entries": node.size, "title": node.title, "agent": header.agent,
            "conversation": header.conversation, "running": running, "archived": lifecycle.archived,
            "trashed_at": lifecycle.trashed_at, "erase_on": lifecycle.trashed_at.map(|at| at + vak_core::lifecycle::trash_window()),
            "held": lifecycle.trashed_at.is_some() && state.core.conversation_held(id)}),
        ))
    };
    let mut sessions = Vec::new();
    let mut more = false;
    loop {
        let rows = catalog
            .session_page(
                &vak_catalog::Audience::default(),
                boundary.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
                PAGE_TURNS,
            )
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if rows.is_empty() {
            break;
        }
        for node in &rows {
            boundary = Some((
                node.updated_at
                    .as_ref()
                    .or(node.created_at.as_ref())
                    .cloned()
                    .unwrap_or_default(),
                node.id.clone(),
            ));
            if let Some(value) = summary(node)? {
                sessions.push(value);
            }
            if sessions.len() >= PAGE_TURNS {
                more = true;
                break;
            }
        }
        if more || rows.len() < PAGE_TURNS {
            break;
        }
    }
    let older = if more {
        boundary
            .map(|at| {
                serde_json::to_vec(&at)
                    .map(|bytes| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
            })
            .transpose()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        None
    };
    // Visible/running sessions retain targeted status recovery even when
    // they sit outside the current summary page.
    let mut extra: HashSet<String> = state
        .live_handles()
        .into_iter()
        .filter(|handle| handle.session.lock().map_or(true, |log| log.is_none()))
        .map(|handle| handle.id.clone())
        .collect();
    if let Some(visible) = visible {
        if visible.len() > 256 {
            return Err(StatusCode::BAD_REQUEST);
        }
        extra.extend(
            visible
                .split(',')
                .take(2)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        );
    }
    for id in extra {
        if sessions.iter().any(|item| item["session_id"] == id) {
            continue;
        }
        if let Some(node) = catalog
            .open_node(&id)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            && let Some(value) = summary(&node)?
        {
            sessions.push(value);
        }
    }
    Ok(serde_json::json!({"sessions": sessions, "older": older}))
}

fn frame(catalog: &Catalog, session: &str, id: &str) -> Result<HistoryFrame, StatusCode> {
    catalog
        .history_frame(session, id)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::CONFLICT)
}

fn load(path: &Path, address: &HistoryFrame) -> Result<Entry, StatusCode> {
    let entry = SessionLog::read_indexed_frame(
        path,
        address.segment,
        address.offset,
        address.length,
        &address.id,
        &address.digest,
    )
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if entry.parent_id != address.parent {
        return Err(StatusCode::CONFLICT);
    }
    Ok(entry)
}

fn ancestor(catalog: &Catalog, session: &str, id: &str, leaf: &str) -> Result<bool, StatusCode> {
    catalog
        .is_ancestor(session, id, leaf)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

/// Content-free fingerprint. Checking it reads file metadata and the
/// immutable header/key only, never the transcript/presentation bodies.
pub(crate) fn revision(state: &AppState, path: &Path) -> Result<String, StatusCode> {
    fn stamp(path: &Path) -> String {
        let mut stamp = String::new();
        for segment in SessionLog::segment_files(path) {
            if let Ok(meta) = std::fs::metadata(&segment) {
                let at = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |time| time.as_nanos());
                use std::fmt::Write;
                let _ = write!(stamp, "{}:{at};", meta.len());
            }
        }
        stamp
    }
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(stamp(path));
    hasher.update(stamp(&state.core.shared_scope().erasures()));
    Ok(format!("{:x}", hasher.finalize()))
}

pub(crate) fn canonical_turns(timeline: &mut vak_delivery::OutputTimeline, session: &SessionLog) {
    let mut turns = HashMap::new();
    for item in &timeline.items {
        if item.role == vak_delivery::OutputRole::User
            && let Some(id) = item.provenance.as_ref().and_then(|p| p.entry_id.as_ref())
        {
            turns.insert(item.turn_id.clone(), id.clone());
        }
    }
    let entry_turns: HashMap<_, _> = session
        .chain_to_root()
        .into_iter()
        .filter_map(|entry| {
            entry
                .at_turn
                .as_ref()
                .map(|turn| (entry.id.as_str(), turn.as_str()))
        })
        .collect();
    for item in &mut timeline.items {
        if let Some(turn) = turns.get(&item.turn_id) {
            item.turn_id = turn.clone();
        } else if let Some(turn) = item
            .provenance
            .as_ref()
            .and_then(|p| p.entry_id.as_deref())
            .and_then(|id| entry_turns.get(id))
        {
            item.turn_id = (*turn).into();
        }
    }
}

fn build(state: &AppState, id: &str, query: PageQuery) -> Result<serde_json::Value, StatusCode> {
    if vak_core::trash::state(&state.core.shared_scope(), id)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .hidden()
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let path: PathBuf = crate::locate_session(&state.core, id).ok_or(StatusCode::NOT_FOUND)?;
    let header = SessionLog::read_header(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    if header.session_id != id {
        return Err(StatusCode::NOT_FOUND);
    }
    let before_revision = revision(state, &path)?;
    let erasure_revision = revision(state, &state.core.shared_scope().erasures())?;
    let running = state
        .get(id)
        .is_some_and(|handle| handle.session.lock().map_or(true, |log| log.is_none()));
    if query.around.is_none() && query.since.as_deref() == Some(&before_revision) {
        return Ok(
            serde_json::json!({"session_id": id, "revision": before_revision, "not_modified": true, "running": running}),
        );
    }
    let catalog = state
        .core
        .catalog()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    catalog
        .catch_up_session(&path)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let leaf = catalog
        .history_head(id)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let head = frame(&catalog, id, &leaf)?;
    let end = if let Some(raw) = &query.before {
        let cursor = decode(raw)?;
        if cursor.session != id
            || !ancestor(&catalog, id, &cursor.leaf, &leaf)?
            || !ancestor(&catalog, id, &cursor.end, &cursor.leaf)?
        {
            return Err(StatusCode::CONFLICT);
        }
        cursor.end
    } else if let Some(turn) = &query.around {
        if !ancestor(&catalog, id, turn, &leaf)? {
            return Err(StatusCode::CONFLICT);
        }
        // A turn's final canonical row, before its next directive.
        catalog
            .history_turn_end(id, turn, &leaf)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::NOT_FOUND)?
    } else {
        leaf.clone()
    };
    let header_frame = catalog
        .history_header(id)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .ok_or(StatusCode::CONFLICT)?;
    let mut addresses = Vec::new();
    let mut at = Some(end.clone());
    let mut turns = 0;
    let mut bytes = 0;
    while let Some(current) = at {
        let row = frame(&catalog, id, &current)?;
        at = row.parent.clone();
        bytes += row.bytes;
        turns += usize::from(row.directive);
        let boundary = row.directive && (turns >= PAGE_TURNS || bytes >= PAGE_BYTES);
        addresses.push(row);
        if boundary {
            break;
        }
    }
    let older = at
        .as_deref()
        .filter(|id| *id != header_frame.id)
        .map(|end| cursor(id, &leaf, end))
        .transpose()?;
    addresses.reverse();
    let mut entries: Vec<Entry> = addresses
        .iter()
        .map(|address| load(&path, address))
        .collect::<Result<_, _>>()?;
    if entries
        .first()
        .is_none_or(|entry| entry.id != header_frame.id)
    {
        entries.insert(0, load(&path, &header_frame)?);
    }
    let targets: HashSet<String> = entries
        .iter()
        .filter(|entry| {
            entry.is_directive() || matches!(entry.payload, EntryPayload::Presentation(_))
        })
        .map(|entry| entry.id.clone())
        .collect();

    let mut known: HashSet<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let mut overlays = Vec::new();
    for target in targets {
        for address in catalog
            .history_overlays(id, &target)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        {
            if known.insert(address.id.clone()) && ancestor(&catalog, id, &address.id, &leaf)? {
                overlays.push((address.segment, address.offset, load(&path, &address)?));
            }
        }
    }
    let mut goal = head.goal_head.clone();
    while let Some(id) = goal {
        let address = frame(&catalog, &header.session_id, &id)?;
        goal = address.goal_parent.clone();
        if known.insert(id) {
            overlays.push((address.segment, address.offset, load(&path, &address)?));
        }
    }
    for global in [
        head.goal_revision.as_deref(),
        head.selection_head.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let address = frame(&catalog, id, global)?;
        if known.insert(global.to_string()) {
            overlays.push((address.segment, address.offset, load(&path, &address)?));
        }
    }
    overlays.sort_by_key(|(segment, offset, _)| (*segment, *offset));
    entries.extend(overlays.into_iter().map(|(_, _, entry)| entry));
    let core = state
        .get(id)
        .map(|handle| handle.core.clone())
        .unwrap_or_else(|| crate::resolve_core_for_header(state, &header));
    let mut view = SessionLog::read_view(path.clone(), entries)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if let Ok(objects) = core.objects() {
        view = view.with_objects(objects);
    }
    let mut presentation = crate::project_presentation(&core, id, &view);
    canonical_turns(&mut presentation, &view);
    presentation.cursor = Some(leaf.clone());
    let mut transcript = crate::transcript_json(&view);
    let newer = catalog
        .history_page_end_after(id, &end, &leaf, PAGE_TURNS as u64)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .as_deref()
        .map(|end| cursor(id, &leaf, end))
        .transpose()?;
    transcript["usage"] =
        serde_json::json!({"input_tokens": head.input_tokens, "output_tokens": head.output_tokens});
    // Appends cannot change this committed branch prefix. Keep its starting
    // revision so the next conditional read observes any concurrent append.
    // Erasure can revoke already opened content and must fail closed.
    if revision(state, &state.core.shared_scope().erasures())? != erasure_revision
        || vak_core::trash::state(&state.core.shared_scope(), id)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .hidden()
    {
        return Err(StatusCode::CONFLICT);
    }
    Ok(
        serde_json::json!({"session_id": id, "cursor": leaf, "revision": before_revision,
        "running": running, "transcript": transcript, "presentation": presentation,
        "page": {"request": query.before, "older": older, "end": end, "first_turn": addresses.iter().find(|row| row.directive).map(|row| row.turns), "total_turns": head.turns,
            "newer": newer}}),
    )
}

pub(crate) async fn snapshot(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<String>,
    Query(query): Query<PageQuery>,
) -> Response {
    match tokio::task::spawn_blocking(move || build(&state, &id, query)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(status)) => (status, Json(serde_json::json!({"error": if status == StatusCode::CONFLICT { "Conversation changed; reload this page." } else { "Conversation history is unavailable." }}))).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vak_session::types::{FrozenContract, MessageRecord, SessionHeader};

    fn fixture(turns: usize) -> (tempfile::TempDir, AppState, String, SessionLog, Vec<String>) {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let id = uuid::Uuid::now_v7().to_string();
        let path = vak_session::SessionPath::new_session_file(core.scope().root(), core.cwd(), &id);
        let mut log = SessionLog::create(
            path,
            SessionHeader {
                session_id: id.clone(),
                created_at: chrono::Utc::now(),
                cwd: core.cwd().clone(),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: Some(vak_session::types::ConversationContext::local(&id, "test")),
                space: None,
                run: None,
                cause: None,
                agent: None,
                contract: FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            },
        )
        .unwrap();
        let mut directives = Vec::new();
        for number in 0..turns {
            let entry = log
                .append_message(MessageRecord {
                    message: vak_llm::Message::user_text(format!("Question {number}")),
                    meta: None,
                })
                .unwrap();
            directives.push(entry.id);
            log.append_message(MessageRecord {
                message: vak_llm::Message::assistant(vec![vak_llm::ContentBlock::text(format!(
                    "Answer {number}"
                ))]),
                meta: None,
            })
            .unwrap();
            if number % PAGE_TURNS == PAGE_TURNS - 1 {
                log.seal_segment().unwrap();
            }
        }
        core.catalog()
            .unwrap()
            .catch_up_session(log.path())
            .unwrap();
        (dir, AppState::new(core), id, log, directives)
    }

    #[test]
    fn recent_and_older_pages_preserve_every_entry_and_canonical_turn() {
        let (_dir, state, id, log, directives) = fixture(121);
        let mut page = build(&state, &id, PageQuery::default()).unwrap();
        assert_eq!(page["transcript"]["messages"].as_array().unwrap().len(), 80);
        assert_eq!(page["page"]["total_turns"], 121);
        assert!(
            state.live_handles().is_empty(),
            "reading never attaches the runtime"
        );
        let revision = page["revision"].as_str().unwrap().to_string();
        let unchanged = build(
            &state,
            &id,
            PageQuery {
                since: Some(revision),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(unchanged["not_modified"], true);
        assert!(unchanged.get("transcript").is_none());
        let mut entries = HashSet::new();
        let mut turn_ids = HashSet::new();
        loop {
            for entry in page["transcript"]["entries"].as_array().unwrap() {
                assert!(entries.insert(entry["entry_id"].as_str().unwrap().to_string()));
            }
            for item in page["presentation"]["items"].as_array().unwrap() {
                if item["role"] == "user" {
                    turn_ids.insert(item["turn_id"].as_str().unwrap().to_string());
                }
            }
            let Some(before) = page["page"]["older"].as_str() else {
                break;
            };
            page = build(
                &state,
                &id,
                PageQuery {
                    before: Some(before.into()),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert_eq!(entries.len(), 242);
        assert_eq!(turn_ids, directives.into_iter().collect());
        assert!(log.tail_id().is_some());
    }

    #[test]
    fn indexed_recent_reads_do_not_open_an_unrelated_history_segment() {
        let (_dir, state, id, log, directives) = fixture(121);
        let page = build(&state, &id, PageQuery::default()).unwrap();
        // Index catch-up is complete. The middle segment is outside both
        // the immutable header and the requested recent page.
        let segment = log.path().join("seg-00000000000000000002.sealed");
        let actual = SessionLog::segment_files(log.path())
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "sealed"))
            .nth(1)
            .unwrap_or(segment);
        std::fs::write(actual, b"unreadable historical segment").unwrap();
        let recent = build(&state, &id, PageQuery::default()).unwrap();
        assert_eq!(
            recent["transcript"]["entries"],
            page["transcript"]["entries"]
        );
        let historical = build(
            &state,
            &id,
            PageQuery {
                around: Some(directives[50].clone()),
                ..Default::default()
            },
        );
        assert_eq!(historical.unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn cursors_are_session_and_branch_scoped_and_late_marks_reach_old_pages() {
        let (_dir, state, id, mut log, directives) = fixture(90);
        let recent = build(&state, &id, PageQuery::default()).unwrap();
        let before = recent["page"]["older"].as_str().unwrap();
        let mut wrong = decode(before).unwrap();
        wrong.session = "another-session".into();
        let wrong = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&wrong).unwrap());
        assert_eq!(
            build(
                &state,
                &id,
                PageQuery {
                    before: Some(wrong),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            StatusCode::CONFLICT
        );
        log.append_turn_mark(
            true,
            vak_session::turn_marks::TurnMarkRecord {
                turn: directives[20].clone(),
                trace: None,
                actor: None,
            },
        )
        .unwrap();
        let old = build(
            &state,
            &id,
            PageQuery {
                before: Some(before.into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            old["transcript"]["set_aside"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(directives[20]))
        );
        let newer = old["page"]["newer"].as_str().unwrap();
        let next = build(
            &state,
            &id,
            PageQuery {
                before: Some(newer.into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            next["transcript"]["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message.to_string().contains("Question 89"))
        );
    }
    #[test]
    fn a_shorter_retry_branch_rejects_old_cursors_and_counts_its_own_turns() {
        let (_dir, state, id, mut log, directives) = fixture(90);
        let recent = build(&state, &id, PageQuery::default()).unwrap();
        let old_cursor = recent["page"]["older"].as_str().unwrap().to_string();
        let prompt = log.retry_turn(&directives[20], None).unwrap();
        log.append_message(prompt).unwrap();
        let current = build(&state, &id, PageQuery::default()).unwrap();
        assert_eq!(current["page"]["total_turns"], 21);
        assert!(
            !current["transcript"]["messages"]
                .to_string()
                .contains("Question 89")
        );
        assert_eq!(
            build(
                &state,
                &id,
                PageQuery {
                    before: Some(old_cursor),
                    ..Default::default()
                }
            )
            .unwrap_err(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    #[ignore = "explicit history scale acceptance"]
    #[allow(clippy::disallowed_macros)] // Explicit test measurements, never runtime output.
    fn history_scale_keeps_warm_pages_bounded() {
        for turns in [100, 1_000, 10_000] {
            let started = std::time::Instant::now();
            let (_dir, state, id, _log, _) = fixture(turns);
            let prepared = started.elapsed();
            let started = std::time::Instant::now();
            let recent = build(&state, &id, PageQuery::default()).unwrap();
            let elapsed = started.elapsed();
            let bytes = serde_json::to_vec(&recent).unwrap().len();
            assert_eq!(
                recent["transcript"]["messages"].as_array().unwrap().len(),
                80
            );
            assert!(bytes < 150_000, "page grew with history: {bytes}");
            let before = recent["page"]["older"].as_str().unwrap().to_string();
            let previous = build(
                &state,
                &id,
                PageQuery {
                    before: Some(before),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                previous["transcript"]["messages"].as_array().unwrap().len(),
                80
            );
            let unchanged = build(
                &state,
                &id,
                PageQuery {
                    since: Some(recent["revision"].as_str().unwrap().into()),
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(serde_json::to_vec(&unchanged).unwrap().len() < 300);
            println!(
                "turns={turns} fixture_and_index_ms={} recent_ms={} bytes={bytes}",
                prepared.as_millis(),
                elapsed.as_millis()
            );
        }
    }
}
