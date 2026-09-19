#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `SessionLog::derive_with_plan` (docs/design/68-context-engine.md §4/§10):
//! a `WorkingSetPlan`'s fidelity choice — not a fixed policy — decides
//! whether a closed turn projects as a full record, a `<turns>` card line,
//! or nothing (left to an existing/incremental `Compaction` entry).

use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message};
use vak_session::types::{FrozenContract, MessageRecord, SessionHeader, TurnCardRecord};
use vak_session::{Fidelity, SessionLog, TurnIndex, WorkingSetPlan};

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

/// Three closed turns, each carrying a `TurnCard` built the same way the
/// real turn-close hook does.
fn three_closed_turns(log: &mut SessionLog) -> Vec<String> {
    let mut ids = Vec::new();
    for (q, a) in [
        ("first question", "first answer"),
        ("second question", "second answer"),
        ("third question", "third answer"),
    ] {
        let id = log.append_message(user_msg(q)).unwrap().id;
        log.append_message(assistant_msg(a)).unwrap();
        let card = TurnIndex::from_log(log)
            .turn_by_id(&id)
            .unwrap()
            .build_card("completed", a.to_string(), &|s| s.len() as u64 / 4);
        log.append_turn_card(TurnCardRecord {
            turn_id: id.clone(),
            card,
        })
        .unwrap();
        ids.push(id);
    }
    ids
}

#[test]
fn full_fidelity_projects_the_two_message_record() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);

    let plan = WorkingSetPlan {
        per_turn: ids.iter().map(|id| (id.clone(), Fidelity::Full)).collect(),
        packet_range: None,
        retrieved: Vec::new(),
        budget: 10_000,
        spent: 0,
    };
    let messages = log.derive_with_plan(&plan);
    // 3 turns x 2 messages each, no <turns> block, no compaction summary.
    assert_eq!(messages.len(), 6);
    let joined: String = messages
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("first question") && joined.contains("first answer"));
    assert!(joined.contains("third question") && joined.contains("third answer"));
    assert!(!joined.contains("<turns>"));
}

#[test]
fn card_fidelity_collapses_into_one_turns_block() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);

    // Newest turn stays Full; the first two are cards.
    let plan = WorkingSetPlan {
        per_turn: vec![
            (ids[0].clone(), Fidelity::Card),
            (ids[1].clone(), Fidelity::Card),
            (ids[2].clone(), Fidelity::Full),
        ],
        packet_range: None,
        retrieved: Vec::new(),
        budget: 10_000,
        spent: 0,
    };
    let messages = log.derive_with_plan(&plan);
    // One <turns> block message plus the Full turn's own two messages
    // (directive + trace/answer) — the card turns never get a directive
    // message of their own.
    assert_eq!(messages.len(), 3);
    let joined: String = messages
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(joined.matches("<turns>").count(), 1);
    assert!(joined.contains("#1 asked:") && joined.contains("#2 asked:"));
    assert!(joined.contains("third question") && joined.contains("third answer"));
}

#[test]
fn packet_turns_and_omitted_turns_contribute_nothing_directly() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);

    // Only the newest turn is planned; the older two are a packet range
    // (no Compaction entry written yet in this test, exercising that
    // `derive_with_plan` renders nothing for them rather than leaking raw
    // text under a missing-fidelity default).
    let plan = WorkingSetPlan {
        per_turn: vec![(ids[2].clone(), Fidelity::Full)],
        packet_range: Some((ids[0].clone(), ids[1].clone())),
        retrieved: Vec::new(),
        budget: 100,
        spent: 50,
    };
    let messages = log.derive_with_plan(&plan);
    let joined: String = messages
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("first question"));
    assert!(!joined.contains("second question"));
    assert!(joined.contains("third question") && joined.contains("third answer"));
}

#[test]
fn incremental_compaction_extends_the_boundary_and_the_summary_survives_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    let ids = three_closed_turns(&mut log);

    log.append_incremental_compaction(&ids[1], "summary of turns 1-2".to_string(), 1_234)
        .unwrap();

    // The plan-free projection (all-Full) must now skip the two compacted
    // turns and carry the summary instead.
    let joined: String = log
        .derive_messages()
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("<context_summary>"));
    assert!(joined.contains("summary of turns 1-2"));
    assert!(!joined.contains("first question"));
    assert!(!joined.contains("second question"));
    assert!(joined.contains("third question") && joined.contains("third answer"));

    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    let joined_after_reopen: String = reopened
        .derive_messages()
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined_after_reopen.contains("summary of turns 1-2"));
}

#[test]
fn incremental_compaction_rejects_a_range_with_no_following_turn() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);
    // The newest turn has no turn after it (only the ledger's end) — must
    // fail rather than silently compact everything, which would drop the
    // still-open turn's own history out from under it.
    assert!(
        log.append_incremental_compaction(&ids[2], "bad".to_string(), 0)
            .is_err()
    );
}
