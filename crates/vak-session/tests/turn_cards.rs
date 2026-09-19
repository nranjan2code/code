#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `EntryPayload::TurnCard` (docs/design/68-context-engine.md §10): written
//! once at turn close, never model-visible raw, and a closed turn's
//! `derive_messages` slice must equal its own `full_record` exactly.

use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message, Role};
use vak_session::types::{
    EntryPayload, FrozenContract, MessageRecord, SessionHeader, TurnCardRecord,
};
use vak_session::{SessionLog, TurnIndex};

fn header() -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: "s-test".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "system prompt v1".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn user_msg(text: &str) -> MessageRecord {
    MessageRecord {
        message: Message::user_text(text),
        meta: None,
    }
}

fn assistant_msg(text: &str) -> MessageRecord {
    MessageRecord {
        message: Message::assistant(vec![ContentBlock::text(text)]),
        meta: None,
    }
}

#[test]
fn a_turn_card_entry_round_trips_through_jsonl() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    let turn_id = log
        .append_message(user_msg("how tall is Everest"))
        .unwrap()
        .id;
    log.append_message(assistant_msg("8,849 meters")).unwrap();

    let index = TurnIndex::from_log(&log);
    let turn = index.turn_by_id(&turn_id).unwrap();
    let card = turn.build_card("completed", "8,849 meters".to_string(), &|s| {
        s.len() as u64 / 4
    });
    log.append_turn_card(TurnCardRecord {
        turn_id: turn_id.clone(),
        card: card.clone(),
    })
    .unwrap();
    drop(log);

    let reopened = SessionLog::open(path).unwrap();
    let cards = reopened.turn_cards();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].0, turn_id);
    assert_eq!(cards[0].1.outcome, "completed");
    assert_eq!(cards[0].1.answered.narration, "8,849 meters");
}

#[test]
fn derive_messages_skips_the_turn_card_entry() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let turn_id = log.append_message(user_msg("hi")).unwrap().id;
    log.append_message(assistant_msg("hello")).unwrap();
    let index = TurnIndex::from_log(&log);
    let card =
        index
            .turn_by_id(&turn_id)
            .unwrap()
            .build_card("completed", "hello".to_string(), &|s| s.len() as u64 / 4);
    log.append_turn_card(TurnCardRecord {
        turn_id: turn_id.clone(),
        card,
    })
    .unwrap();

    let joined: String = log
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !joined.contains("outcome") && !joined.contains("tokens_full"),
        "the raw card entry must never be model-visible"
    );
    assert!(
        log.chain_to_root()
            .iter()
            .any(|e| matches!(e.payload, EntryPayload::TurnCard(_))),
        "the entry is still on the ledger, just not projected"
    );
}

fn tool_call(id: &str, name: &str) -> MessageRecord {
    MessageRecord {
        message: Message::assistant(vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input: serde_json::json!({}),
        }]),
        meta: None,
    }
}

fn tool_result(id: &str, content: &str) -> MessageRecord {
    MessageRecord {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result(id, content)],
        },
        meta: None,
    }
}

/// No-cut invariant (docs/design/68-context-engine.md, Verification): every
/// tool result recorded in the ledger is either verbatim (the still-open
/// turn), a trace line naming its evidence id (a closed turn in the working
/// set), or covered by a compaction packet — never silently absent.
#[test]
fn every_tool_result_is_verbatim_traced_or_covered_by_a_packet() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();

    // Turn A: closed, will be compacted away.
    log.append_message(user_msg("turn a")).unwrap();
    log.append_message(tool_call("ev-a", "search")).unwrap();
    log.append_message(tool_result("ev-a", "result a")).unwrap();
    log.append_message(assistant_msg("a done")).unwrap();

    // Turn B: closed, stays in the working set as a full record.
    log.append_message(user_msg("turn b")).unwrap();
    log.append_message(tool_call("ev-b", "search")).unwrap();
    log.append_message(tool_result("ev-b", "result b")).unwrap();
    log.append_message(assistant_msg("b done")).unwrap();

    // Turn C: still open, stays fully verbatim.
    log.append_message(user_msg("turn c")).unwrap();
    log.append_message(tool_call("ev-c", "search")).unwrap();
    log.append_message(tool_result("ev-c", "result c")).unwrap();

    // Compact turn A away, keeping the last 2 turns (B and the open C).
    let plan = log.plan_compaction(2).expect("a plan over three turns");
    log.apply_compaction(&plan, "turn a summarized".into(), 100)
        .unwrap();

    let messages = log.derive_messages();
    let joined: String = messages
        .iter()
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n");
    let has_verbatim = |id: &str| {
        messages.iter().any(|m| {
            m.content.iter().any(
                |b| matches!(b, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == id),
            )
        })
    };

    // ev-a: dropped by compaction — covered by the packet summary, never
    // verbatim and never traced.
    assert!(!has_verbatim("ev-a"));
    assert!(!joined.contains("ev:ev-a"));
    assert!(joined.contains("<context_summary>"));

    // ev-b: closed and kept — a trace line, not a raw block.
    assert!(!has_verbatim("ev-b"));
    assert!(joined.contains("ev:ev-b"));

    // ev-c: still open — verbatim.
    assert!(has_verbatim("ev-c"));
}

#[test]
fn a_closed_turns_derive_messages_slice_equals_its_full_record() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    log.append_message(user_msg("first")).unwrap();
    log.append_message(assistant_msg("first reply")).unwrap();
    log.append_message(user_msg("second")).unwrap();
    log.append_message(assistant_msg("second reply")).unwrap();

    let index = TurnIndex::from_log(&log);
    let first_turn = &index.turns[0];
    assert!(first_turn.closed);
    let full_record = first_turn.full_record();

    let projected = log.derive_messages();
    assert_eq!(&projected[0..2], &full_record[..]);
}
