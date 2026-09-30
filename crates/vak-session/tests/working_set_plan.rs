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
        selected_records: None,
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
        selected_records: None,
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
fn a_packet_range_with_no_stored_packet_renders_as_cards_never_as_nothing() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);

    // Only the newest turn is planned; the older two are a packet range
    // that no packet covers yet (the agent normally writes it before
    // projecting). The no-cut invariant still holds: those turns ride as
    // card lines, never as their raw records and never as nothing.
    let plan = WorkingSetPlan {
        selected_records: None,
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
    assert!(!joined.contains("<context_summary>"));
    // One <turns> block carries both, and neither turn's own two-message
    // record is present: "first question" appears only inside that block.
    assert_eq!(joined.matches("<turns>").count(), 1);
    assert!(
        joined.contains("#1 asked:") && joined.contains("#2 asked:"),
        "{joined}"
    );
    let standalone: Vec<&Message> = messages
        .iter()
        .filter(|m| !m.text_content().starts_with("<turns>"))
        .collect();
    assert!(
        standalone
            .iter()
            .all(|m| !m.text_content().contains("first question")),
        "{joined}"
    );
    assert!(joined.contains("third question") && joined.contains("third answer"));
}

/// A packet never moves a boundary: the plan-free projection (all-Full)
/// is untouched by it, and the plan that asked for it renders it — before
/// and after a reopen from disk.
#[test]
fn incremental_compaction_writes_a_packet_that_survives_reopen_and_hides_nothing() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    let ids = three_closed_turns(&mut log);

    log.append_incremental_compaction(
        &ids[0],
        &ids[1],
        "small-model",
        "summary of turns 1-2".to_string(),
        1_234,
    )
    .unwrap();

    let joined: String = log
        .derive_messages()
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("<context_summary>"), "{joined}");
    assert!(joined.contains("first question") && joined.contains("second question"));
    assert!(joined.contains("third question") && joined.contains("third answer"));

    let plan = WorkingSetPlan {
        selected_records: None,
        per_turn: vec![(ids[2].clone(), Fidelity::Full)],
        packet_range: Some((ids[0].clone(), ids[1].clone())),
        retrieved: Vec::new(),
        budget: 100,
        spent: 50,
    };
    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    let joined_after_reopen: String = reopened
        .derive_with_plan(&plan)
        .iter()
        .map(Message::text_content)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined_after_reopen.contains("summary of turns 1-2"));
    assert!(!joined_after_reopen.contains("first question"));
    assert!(joined_after_reopen.contains("third question"));
    let packet = reopened
        .packet_for(&ids[0], &ids[1])
        .expect("packet stored");
    assert_eq!(packet.model, "small-model");
}

#[test]
fn packet_needs_compaction_until_an_incremental_entry_covers_it() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);

    assert!(
        log.packet_needs_compaction(&ids[0], &ids[1]),
        "nothing compacted yet, so the range still needs it"
    );
    let (transcript, _) = log.packet_transcript(&ids[0], &ids[1]);
    assert!(transcript.contains("#1 asked:") && transcript.contains("#2 asked:"));
    assert!(
        !transcript.contains("third question"),
        "the transcript must stop at last_turn_id, not run to the end"
    );

    log.append_incremental_compaction(
        &ids[0],
        &ids[1],
        "m",
        "summary of turns 1-2".to_string(),
        999,
    )
    .unwrap();
    assert!(
        !log.packet_needs_compaction(&ids[0], &ids[1]),
        "a packet now covers exactly this range"
    );
    assert!(
        log.packet_needs_compaction(&ids[0], &ids[2]),
        "a different range is a different packet"
    );
    assert!(
        log.packet_needs_compaction(&ids[1], &ids[1]),
        "a packet is matched on its whole range, never on a sub-range"
    );
}

#[test]
fn packet_transcript_carries_the_prior_summary_forward() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);
    log.append_incremental_compaction(&ids[0], &ids[0], "m", "FIRST-SUMMARY".to_string(), 100)
        .unwrap();

    assert!(log.packet_needs_compaction(&ids[0], &ids[1]));
    let (transcript, _) = log.packet_transcript(&ids[0], &ids[1]);
    assert!(
        transcript.contains("FIRST-SUMMARY"),
        "the prior summary must carry forward: {transcript}"
    );
    assert!(
        !transcript.contains("#1 asked:"),
        "seeded turn re-read: {transcript}"
    );
    assert!(transcript.contains("#2 asked:"));
}

#[test]
fn a_packet_over_an_unknown_turn_is_rejected() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);
    assert!(
        log.append_incremental_compaction(&ids[0], "no-such-turn", "m", "bad".to_string(), 0)
            .is_err()
    );
}

#[test]
fn addressed_projection_contains_only_selected_history_and_open_directive() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);
    let index = TurnIndex::from_log(&log);
    let closing = index
        .turn_by_id(&ids[0])
        .unwrap()
        .closing_entry_id
        .clone()
        .unwrap();
    log.append_message(user_msg("continue the first question"))
        .unwrap();
    let plan = WorkingSetPlan {
        selected_records: Some(vec![(ids[0].clone(), closing)]),
        per_turn: vec![(ids[0].clone(), Fidelity::Card)],
        ..Default::default()
    };
    let messages = log.derive_selected_checked(&plan).unwrap();
    let encoded = serde_json::to_string(&messages).unwrap();
    assert!(encoded.contains(&format!("[turn_id:{}]", ids[0])));
    assert!(encoded.contains("first question"));
    assert!(encoded.contains("continue the first question"));
    assert!(!encoded.contains("second question"));
    assert!(!encoded.contains("third question"));
    assert!(!encoded.contains("#1"));
    let fresh = WorkingSetPlan {
        selected_records: Some(vec![]),
        ..Default::default()
    };
    let messages = log.derive_selected_checked(&fresh).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0],
        Message::user_text("continue the first question")
    );
}

#[test]
fn addressed_projection_rejects_missing_mismatched_and_duplicate_sources() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let ids = three_closed_turns(&mut log);
    let index = TurnIndex::from_log(&log);
    let closing = index
        .turn_by_id(&ids[0])
        .unwrap()
        .closing_entry_id
        .clone()
        .unwrap();
    for sources in [
        vec![(ids[0].clone(), "missing".into())],
        vec![(ids[1].clone(), closing.clone())],
        vec![
            (ids[0].clone(), closing.clone()),
            (ids[0].clone(), closing.clone()),
        ],
    ] {
        let plan = WorkingSetPlan {
            per_turn: sources
                .iter()
                .map(|(id, _)| (id.clone(), Fidelity::Full))
                .collect(),
            selected_records: Some(sources),
            ..Default::default()
        };
        assert!(log.derive_selected_checked(&plan).is_err());
    }
}

#[test]
fn selection_manifest_replays_at_its_leaf_after_future_turns_and_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    let ids = three_closed_turns(&mut log);
    let index = TurnIndex::from_log(&log);
    let closing = index
        .turn_by_id(&ids[0])
        .unwrap()
        .closing_entry_id
        .clone()
        .unwrap();
    log.append_message(user_msg("follow up first question"))
        .unwrap();
    let plan = WorkingSetPlan {
        selected_records: Some(vec![(ids[0].clone(), closing)]),
        per_turn: vec![(ids[0].clone(), Fidelity::Full)],
        ..Default::default()
    };
    let expected = log.derive_selected_checked(&plan).unwrap();
    let manifest = log.append_context_selection(plan).unwrap();
    log.append_message(assistant_msg("future response must not enter replay"))
        .unwrap();
    log.append_message(user_msg("future unrelated question"))
        .unwrap();
    assert_eq!(
        log.replay_context_selection(&manifest.id).unwrap(),
        expected
    );
    assert!(
        !serde_json::to_string(&log.derive_messages())
            .unwrap()
            .contains("policy_version")
    );
    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(
        reopened.replay_context_selection(&manifest.id).unwrap(),
        expected
    );
}
