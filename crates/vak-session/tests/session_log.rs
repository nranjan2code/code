// A test's output is for the person running it.
#![allow(clippy::disallowed_macros)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message, Role, Usage};
use vak_session::types::{
    EntryPayload, FrozenContract, MessageMeta, MessageRecord, SessionHeader, TurnBinding,
    WorkContract, WorkEvent, WorkEventKind, WorkItemDefinition, WorkOwner,
};
use vak_session::{
    ActivityKind, ActivityRecord, ActivityStatus, Fidelity, SessionLog, SessionPath, WorkingSetPlan,
};

fn header() -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
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

#[test]
fn successful_tool_receipts_retain_replay_timestamp() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("receipts.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    log.append_message(MessageRecord {
        message: Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tool-1".into(),
                name: "read".into(),
                input: serde_json::json!({"path":"notes.txt"}),
            }],
        },
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result("tool-1", "notes")],
        },
        meta: None,
    })
    .unwrap();
    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    let receipts = reopened.successful_tool_receipts();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].0, "tool-1");
    assert!(receipts[0].1 <= chrono::Utc::now());
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

fn work_contract(id: &str) -> WorkContract {
    WorkContract {
        contract_id: id.into(),
        revision: 0,
        source_entry_id: "prompt".into(),
        objective: "test work".into(),
        constraints: Vec::new(),
        assumptions: Vec::new(),
        criteria: Vec::new(),
        items: vec![WorkItemDefinition {
            item_id: "one".into(),
            title: "one".into(),
            instructions: "one".into(),
            dependencies: Vec::new(),
            owner: WorkOwner::ParentAgent,
            required: true,
            readonly: false,
            path_claims: Vec::new(),
            criterion_ids: Vec::new(),
        }],
    }
}

#[test]
fn invalid_work_event_is_rejected_without_poisoning_the_ledger() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("work.jsonl"), header()).unwrap();
    log.append_work(WorkEvent {
        contract_id: "work-atomic".into(),
        revision: 0,
        kind: WorkEventKind::ContractCreated {
            contract: work_contract("work-atomic"),
        },
    })
    .unwrap();
    log.append_work(WorkEvent {
        contract_id: "work-atomic".into(),
        revision: 0,
        kind: WorkEventKind::ContractStatusChanged {
            from: vak_session::types::WorkContractStatus::Draft,
            to: vak_session::types::WorkContractStatus::Active,
            reason: "test".into(),
        },
    })
    .unwrap();
    let before = log.len();
    assert!(
        log.append_work(WorkEvent {
            contract_id: "work-atomic".into(),
            revision: 0,
            kind: WorkEventKind::ItemStatusChanged {
                item_id: "one".into(),
                from: vak_session::types::WorkItemStatus::Proposed,
                to: vak_session::types::WorkItemStatus::Ready,
                attempt: 0,
                reason: "duplicate".into(),
            },
        })
        .is_err()
    );
    assert_eq!(log.len(), before);
    assert!(log.work_projection().unwrap().is_some());
}

#[test]
fn append_and_derive_roundtrip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();

    log.append_message(user_msg("first")).unwrap();
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text("second")]),
        meta: Some(MessageMeta {
            model: Some("claude-sonnet-4-5".into()),
            stop_reason: Some("end_turn".into()),
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            }),
            ..Default::default()
        }),
    })
    .unwrap();

    // The create handle holds the ledger's exclusive lock; release it
    // before reopening.
    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(reopened.len(), 3);
    let msgs = reopened.derive_messages();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].text_content(), "first");
    assert_eq!(msgs[1].role, Role::Assistant);
    assert_eq!(reopened.total_usage().output_tokens, 5);
    assert_eq!(
        reopened.header().unwrap().contract.model,
        "claude-sonnet-4-5"
    );
}

#[test]
fn activity_roundtrips_without_entering_model_context() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("activity.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    log.append_message(user_msg("keep me visible")).unwrap();
    log.append_activity(ActivityRecord {
        activity_id: "retry-1".into(),
        kind: ActivityKind::Retry,
        status: ActivityStatus::Succeeded,
        label: "Recovered after retry".into(),
        detail: Some("provider timeout".into()),
        data: [("attempt".into(), "1".into())].into(),
    })
    .unwrap();
    assert_eq!(log.derive_messages().len(), 1);
    drop(log);

    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(reopened.derive_messages().len(), 1);
    let activities = reopened.activities();
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].2.activity_id, "retry-1");
}

#[test]
fn voice_activity_helpers_record_transcript_and_playback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("voice.jsonl");
    let mut log = SessionLog::create(path, header()).unwrap();
    let transcript = log
        .append_voice_transcript("u1", "open the build log", true)
        .unwrap();
    let playback = log.append_voice_playback("p1", 640, true).unwrap();
    let EntryPayload::Activity(a) = transcript.payload else {
        panic!("expected transcript activity")
    };
    assert_eq!(a.kind, ActivityKind::VoiceTranscript);
    assert_eq!(a.data.get("finalized"), Some(&"true".to_string()));
    let EntryPayload::Activity(a) = playback.payload else {
        panic!("expected playback activity")
    };
    assert_eq!(a.kind, ActivityKind::VoicePlayback);
    assert_eq!(a.status, ActivityStatus::Partial);
}

/// Active work state reaches the model through the request tail
/// (docs/design/68-context-engine.md §6/§10), not spliced into the
/// projection.
#[test]
fn active_work_is_reconstructed_into_the_tail() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("work.jsonl"), header()).unwrap();
    let contract = WorkContract {
        contract_id: "work-ctx".into(),
        revision: 0,
        source_entry_id: "user".into(),
        objective: "preserve active work".into(),
        constraints: Vec::new(),
        assumptions: Vec::new(),
        criteria: Vec::new(),
        items: vec![WorkItemDefinition {
            item_id: "inspect".into(),
            title: "Inspect".into(),
            instructions: "inspect".into(),
            dependencies: Vec::new(),
            owner: WorkOwner::ParentAgent,
            required: true,
            readonly: true,
            path_claims: Vec::new(),
            criterion_ids: Vec::new(),
        }],
    };
    log.append_work(WorkEvent {
        contract_id: "work-ctx".into(),
        revision: 0,
        kind: WorkEventKind::ContractCreated { contract },
    })
    .unwrap();
    assert!(
        log.derive_messages().is_empty(),
        "work state must not be spliced into the projection"
    );
    let work_contract = log
        .tail_sections(None)
        .work_contract
        .expect("active work contract present in the tail");
    assert!(work_contract.contains("preserve active work"));
    assert!(work_contract.contains("inspect"));
}

#[test]
fn active_work_survives_compaction() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("work-compaction.jsonl"), header()).unwrap();
    log.append_message(user_msg("old context")).unwrap();
    let contract = WorkContract {
        contract_id: "work-compaction".into(),
        revision: 0,
        source_entry_id: "user".into(),
        objective: "survive compaction".into(),
        constraints: Vec::new(),
        assumptions: Vec::new(),
        criteria: Vec::new(),
        items: vec![WorkItemDefinition {
            item_id: "verify".into(),
            title: "Verify evidence".into(),
            instructions: "verify".into(),
            dependencies: Vec::new(),
            owner: WorkOwner::ParentAgent,
            required: true,
            readonly: true,
            path_claims: Vec::new(),
            criterion_ids: Vec::new(),
        }],
    };
    log.append_work(WorkEvent {
        contract_id: "work-compaction".into(),
        revision: 0,
        kind: WorkEventKind::ContractCreated { contract },
    })
    .unwrap();
    log.append_message(user_msg("keep this")).unwrap();
    log.append_handoff_reset("old summary".into(), 9000, false)
        .unwrap();
    assert!(
        log.tail_sections(None)
            .work_contract
            .expect("active work contract present in the tail")
            .contains("survive compaction")
    );
}

#[test]
fn restart_reconciles_orphaned_running_work_without_replaying_it() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("work-recovery.jsonl"), header()).unwrap();
    let contract = WorkContract {
        contract_id: "work-recovery".into(),
        revision: 0,
        source_entry_id: "user".into(),
        objective: "recover safely".into(),
        constraints: Vec::new(),
        assumptions: Vec::new(),
        criteria: Vec::new(),
        items: vec![WorkItemDefinition {
            item_id: "mutate".into(),
            title: "Mutate safely".into(),
            instructions: "perform one operation".into(),
            dependencies: Vec::new(),
            owner: WorkOwner::ParentAgent,
            required: true,
            readonly: false,
            path_claims: Vec::new(),
            criterion_ids: Vec::new(),
        }],
    };
    log.append_work(WorkEvent {
        contract_id: "work-recovery".into(),
        revision: 0,
        kind: WorkEventKind::ContractCreated { contract },
    })
    .unwrap();
    log.append_work(WorkEvent {
        contract_id: "work-recovery".into(),
        revision: 0,
        kind: WorkEventKind::ContractStatusChanged {
            from: vak_session::types::WorkContractStatus::Draft,
            to: vak_session::types::WorkContractStatus::Active,
            reason: "test".into(),
        },
    })
    .unwrap();
    log.append_work(WorkEvent {
        contract_id: "work-recovery".into(),
        revision: 0,
        kind: WorkEventKind::ItemStatusChanged {
            item_id: "mutate".into(),
            from: vak_session::types::WorkItemStatus::Ready,
            to: vak_session::types::WorkItemStatus::Running,
            attempt: 1,
            reason: "test".into(),
        },
    })
    .unwrap();

    assert_eq!(log.reconcile_running_work(&HashSet::new()).unwrap(), 1);
    let projection = log.work_projection().unwrap().unwrap();
    assert_eq!(
        projection.items["mutate"].status,
        vak_session::types::WorkItemStatus::Interrupted
    );
    assert!(projection.items["mutate"].blocker.is_some());
}

#[test]
fn restart_attaches_completed_child_for_verification_without_marking_it_succeeded() {
    let dir = tempdir().unwrap();
    let child_path =
        SessionPath::new_session_file(dir.path(), PathBuf::from("/tmp/proj").as_path(), "child-1");
    let mut child = SessionLog::create(
        child_path,
        SessionHeader {
            space: None,
            run: None,
            cause: None,
            agent: None,
            session_id: "child-1".into(),
            ..header()
        },
    )
    .unwrap();
    child
        .append_child_run_status(vak_session::types::ChildRunStatus::Completed)
        .unwrap();
    drop(child);

    let mut log = SessionLog::create(dir.path().join("parent.jsonl"), header()).unwrap();
    let mut contract = work_contract("child-recovery");
    contract.items[0].owner = WorkOwner::Worker;
    log.append_work(WorkEvent {
        contract_id: "child-recovery".into(),
        revision: 0,
        kind: WorkEventKind::ContractCreated { contract },
    })
    .unwrap();
    log.append_work(WorkEvent {
        contract_id: "child-recovery".into(),
        revision: 0,
        kind: WorkEventKind::ContractStatusChanged {
            from: vak_session::types::WorkContractStatus::Draft,
            to: vak_session::types::WorkContractStatus::Active,
            reason: "delegating".into(),
        },
    })
    .unwrap();
    log.append_work(WorkEvent {
        contract_id: "child-recovery".into(),
        revision: 0,
        kind: WorkEventKind::ItemStatusChanged {
            item_id: "one".into(),
            from: vak_session::types::WorkItemStatus::Ready,
            to: vak_session::types::WorkItemStatus::Running,
            attempt: 1,
            reason: "delegated".into(),
        },
    })
    .unwrap();
    log.append_work(WorkEvent {
        contract_id: "child-recovery".into(),
        revision: 0,
        kind: WorkEventKind::ItemAssigned {
            item_id: "one".into(),
            owner: WorkOwner::Worker,
            child_session_id: Some("child-1".into()),
        },
    })
    .unwrap();
    assert_eq!(
        log.reconcile_running_work_with_child_ledgers(&HashSet::new(), Some(dir.path()))
            .unwrap(),
        1
    );
    let state = log.work_projection().unwrap().unwrap().items["one"].clone();
    assert_eq!(
        state.status,
        vak_session::types::WorkItemStatus::ReadyForVerification
    );
    assert_eq!(state.evidence.len(), 1);
}

#[test]
fn branching_derives_only_active_path() {
    // Each user/assistant pair is a whole turn (docs/design/68-context-
    // engine.md principle 3); the projection now renders a closed turn as
    // its two-message full record, so the fixture needs a reply per turn
    // to exercise that rather than the old bare-message-per-turn shape.
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    log.append_message(user_msg("a")).unwrap();
    log.append_message(assistant_msg("a-reply")).unwrap();
    log.append_message(user_msg("b")).unwrap();
    let fork_point = log.append_message(assistant_msg("b-reply")).unwrap();
    log.append_message(user_msg("c")).unwrap();
    log.append_message(assistant_msg("c-reply")).unwrap();

    log.branch_at(&fork_point.id).unwrap();
    log.append_message(user_msg("d")).unwrap();
    log.append_message(assistant_msg("d-reply")).unwrap();

    let texts: Vec<String> = log
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect();
    assert_eq!(texts, vec!["a", "a-reply", "b", "b-reply", "d", "d-reply"]);
}

/// A packet is rendered only when the plan's `packet_range` is exactly the
/// range it covers (docs/design/68-context-engine.md §4): the packet is a
/// cache of summariser work, never a boundary in the ledger.
#[test]
fn packet_is_rendered_only_for_the_exact_range_the_plan_asks_for() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let old1 = log.append_message(user_msg("old-1")).unwrap().id;
    log.append_message(assistant_msg("old-1-reply")).unwrap();
    let old2 = log.append_message(user_msg("old-2")).unwrap().id;
    log.append_message(assistant_msg("old-2-reply")).unwrap();
    let kept = log.append_message(user_msg("kept")).unwrap().id;
    log.append_message(assistant_msg("kept-reply")).unwrap();
    log.append_packet(
        &old1,
        &old2,
        "small-model",
        "summary of old turns".into(),
        9000,
    )
    .unwrap();
    let after = log.append_message(user_msg("after")).unwrap().id;
    log.append_message(assistant_msg("after-reply")).unwrap();

    let texts = |messages: Vec<vak_llm::Message>| -> Vec<String> {
        messages.iter().map(|m| m.text_content()).collect()
    };

    // The plan that asked for this packet sees it in place of the turns.
    let packeting = WorkingSetPlan {
        per_turn: vec![
            (kept.clone(), Fidelity::Full),
            (after.clone(), Fidelity::Full),
        ],
        packet_range: Some((old1.clone(), old2.clone())),
        ..WorkingSetPlan::default()
    };
    assert_eq!(
        texts(log.derive_with_plan(&packeting)),
        vec![
            "<context_summary>\nSummary of turns 1-2; recall({ turn: N }) reopens one.\nsummary of old turns\n</context_summary>",
            "kept",
            "kept-reply",
            "after",
            "after-reply",
        ]
    );

    // A plan that wants every turn at Full gets every turn at Full: the
    // packet hides nothing.
    let everything = WorkingSetPlan {
        per_turn: [&old1, &old2, &kept, &after]
            .into_iter()
            .map(|id| (id.clone(), Fidelity::Full))
            .collect(),
        ..WorkingSetPlan::default()
    };
    assert_eq!(
        texts(log.derive_with_plan(&everything)),
        vec![
            "old-1",
            "old-1-reply",
            "old-2",
            "old-2-reply",
            "kept",
            "kept-reply",
            "after",
            "after-reply",
        ]
    );
    // ...and so does the plan-free projection.
    assert_eq!(texts(log.derive_messages()).len(), 8);

    // A plan that packets a different range finds no packet: those turns
    // render as cards rather than vanishing (no-cut), and no stale summary
    // is substituted.
    let narrower = WorkingSetPlan {
        per_turn: vec![
            (old2.clone(), Fidelity::Full),
            (kept.clone(), Fidelity::Full),
            (after.clone(), Fidelity::Full),
        ],
        packet_range: Some((old1.clone(), old1.clone())),
        ..WorkingSetPlan::default()
    };
    let rendered = texts(log.derive_with_plan(&narrower));
    assert!(
        !rendered.iter().any(|t| t.contains("<context_summary>")),
        "{rendered:?}"
    );
    assert!(
        rendered
            .last()
            .is_some_and(|t| t.starts_with("<turns>") && t.contains("old-1")),
        "the card block follows the Full turns: {rendered:?}"
    );
    assert!(log.packet_needs_compaction(&old1, &old1));
    assert!(!log.packet_needs_compaction(&old1, &old2));
}

#[test]
fn unknown_parent_rejected() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let orphan = EntryPayload::Message(user_msg("x"));
    let mut entry = vak_session::Entry::new(Some("nope".into()), orphan);
    entry.parent_id = Some("nope".into());
    assert!(log.append(entry).is_err());
}

/// Growing a packet reuses the stored one as its seed (docs/design/68
/// §4): the summariser input for a wider range starts with the longest
/// stored packet over the same first turn and continues with the cards of
/// the turns after it — raw pre-packet history is never re-read — and the
/// wider packet, once stored, is what the wider plan renders.
#[test]
fn a_wider_packet_seeds_from_the_stored_prefix_packet() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();

    let mut ids = Vec::new();
    for i in 0..8 {
        ids.push(log.append_message(user_msg(&format!("m{i}"))).unwrap().id);
        log.append_message(assistant_msg(&format!("m{i}-reply")))
            .unwrap();
    }
    log.append_packet(&ids[0], &ids[5], "m", "summary-one".into(), 9000)
        .unwrap();

    // The plan now wants m0..=m6 packeted: the transcript is seeded from
    // summary-one and adds only m6.
    let (transcript, _) = log.packet_transcript(&ids[0], &ids[6]);
    assert!(transcript.starts_with("summary-one"), "{transcript}");
    assert!(transcript.contains("m6"), "{transcript}");
    assert!(
        !transcript.contains("m0-reply"),
        "raw history re-read: {transcript}"
    );
    assert!(log.packet_needs_compaction(&ids[0], &ids[6]));

    log.append_packet(&ids[0], &ids[6], "m", "summary-two".into(), 400)
        .unwrap();
    let plan = WorkingSetPlan {
        per_turn: vec![(ids[7].clone(), Fidelity::Full)],
        packet_range: Some((ids[0].clone(), ids[6].clone())),
        ..WorkingSetPlan::default()
    };
    let final_msgs = log.derive_with_plan(&plan);
    assert!(
        final_msgs[0].text_content().contains("summary-two"),
        "the packet for the asked range wins"
    );
    assert!(
        !serde_like_contains(&final_msgs, "summary-one"),
        "the narrower packet is not rendered alongside"
    );
    // The narrower packet is still there for a plan that asks for it.
    assert!(!log.packet_needs_compaction(&ids[0], &ids[5]));
}

#[test]
fn proactive_retrieval_finds_relevant_older_entries() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();

    log.append_message(user_msg("discuss Kubernetes cluster deployment"))
        .unwrap();
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text(
            "the Kubernetes cluster uses Docker containers",
        )]),
        meta: None,
    })
    .unwrap();
    log.append_message(user_msg("how to configure PostgreSQL database settings"))
        .unwrap();
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text(
            "PostgreSQL needs shared_buffers tuning",
        )]),
        meta: None,
    })
    .unwrap();
    log.append_message(user_msg("tell me about network firewalls"))
        .unwrap();

    // Retrieve the most relevant entries for a Kubernetes query.
    let retrieved = log.retrieve_relevant_entries(
        "Kubernetes deployment",
        5,
        0, // don't exclude any tail for this test
    );
    assert!(
        retrieved.len() >= 2,
        "should retrieve at least Kubernetes-related entries"
    );

    // The first retrieved entry should be about Kubernetes.
    let first_text = retrieved[0].1.text_content();
    assert!(
        first_text.contains("Kubernetes") || first_text.contains("kubernetes"),
        "first retrieved should be Kubernetes-related, got: {first_text}"
    );
}

fn serde_like_contains(msgs: &[vak_llm::Message], needle: &str) -> bool {
    msgs.iter().any(|m| m.text_content().contains(needle))
}

#[test]
fn torn_trailing_line_is_skipped_not_fatal() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    log.append_message(user_msg("safe")).unwrap();
    drop(log);

    // Simulate a crash mid-append: a partial JSON line at EOF.
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path.join("seg-00000001.log"))
        .unwrap();
    f.write_all(&[9, 0, 0, 0, 1, 2]).unwrap();
    drop(f);

    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(reopened.len(), 2);
    assert!(!reopened.warnings().is_empty(), "skip must be surfaced");
    let msgs = reopened.derive_messages();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].text_content(), "safe");
}

#[test]
fn second_handle_on_same_file_is_locked_out() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let log = SessionLog::create(path.clone(), header()).unwrap();
    assert!(
        SessionLog::open(path.clone()).is_err(),
        "concurrent open must fail while a handle is alive"
    );
    drop(log);
    assert!(SessionLog::open(path).is_ok());
}

#[test]
fn open_read_only_succeeds_while_handle_is_locked() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut writer = SessionLog::create(path.clone(), header()).unwrap();
    writer
        .append_message(user_msg("hello from writer"))
        .unwrap();

    let mut reader = SessionLog::open_read_only(path.clone()).expect("read only open must succeed");
    assert!(reader.is_read_only());
    assert_eq!(reader.derive_messages().len(), 1);
    assert!(
        reader
            .append_message(user_msg("write should fail"))
            .is_err(),
        "appending to read-only session must fail"
    );

    drop(writer);
}

#[test]
fn create_on_existing_nonempty_file_refuses() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let log = SessionLog::create(path.clone(), header()).unwrap();
    drop(log);
    assert!(
        SessionLog::create(path, header()).is_err(),
        "must not append a second header to an existing ledger"
    );
}

#[test]
fn total_usage_counts_active_chain_only() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();

    let usage_msg = |text: &str, out: u64| MessageRecord {
        message: Message::user_text(text),
        meta: Some(MessageMeta {
            usage: Some(Usage {
                input_tokens: 100,
                output_tokens: out,
                ..Default::default()
            }),
            ..Default::default()
        }),
    };

    let fork = log.append_message(usage_msg("branch-a", 50)).unwrap().id;
    log.append_message(usage_msg("branch-a-2", 70)).unwrap();

    // Abandon that branch: rewind to the first entry and grow elsewhere.
    log.branch_at(&fork).unwrap();
    log.append_message(usage_msg("active", 10)).unwrap();

    // Old code summed every entry (100/130 from the abandoned branch).
    let u = log.total_usage();
    assert_eq!(u.input_tokens, 200);
    assert_eq!(u.output_tokens, 60);
}

#[test]
fn turn_capability_binding_roundtrips_without_entering_context() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header())
        .unwrap()
        .with_objects(std::sync::Arc::new(
            vak_session::objects::MemoryObjects::default(),
        ));
    log.append_turn_capabilities(TurnBinding {
        epoch: 7,
        capability_ids: vec!["Tool:read".into()],
        excluded_ids: vec!["Tool:bash".into()],
        system_prompt: "system".into(),
        tool_schemas: vec![serde_json::json!({"name":"read"})],
        core_tool_names: vec!["read".into()],
        deferred_tool_names: Vec::new(),
        tool_index: String::new(),
        tool_domains: Default::default(),
    })
    .unwrap();
    assert!(log.derive_messages().is_empty());
    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    assert!(
        reopened
            .chain_to_root()
            .iter()
            .any(|entry| matches!(entry.payload, EntryPayload::TurnCapabilitiesBound(_)))
    );
}

/// A turn bound to the same interface as the last one records a reference,
/// not the whole system prompt and schemas again; a changed interface is
/// written in full. Prints the bytes a turn's binding costs either way.
#[test]
fn unchanged_capabilities_not_rewritten() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header())
        .unwrap()
        .with_objects(std::sync::Arc::new(
            vak_session::objects::MemoryObjects::default(),
        ));
    let bound = |epoch: u64, prompt: &str| TurnBinding {
        epoch,
        capability_ids: vec!["Tool:read".into()],
        excluded_ids: Vec::new(),
        system_prompt: prompt.repeat(400),
        tool_schemas: vec![serde_json::json!({"name": "read", "description": "x".repeat(2000)})],
        core_tool_names: vec!["read".into()],
        deferred_tool_names: Vec::new(),
        tool_index: String::new(),
        tool_domains: Default::default(),
    };
    let size = || {
        SessionLog::segment_files(&path)
            .iter()
            .map(|segment| std::fs::metadata(segment).unwrap().len())
            .sum::<u64>()
    };

    let before = size();
    let first = log.append_turn_capabilities(bound(1, "system ")).unwrap();
    let full = size() - before;
    let before = size();
    let second = log.append_turn_capabilities(bound(2, "system ")).unwrap();
    let referenced = size() - before;
    println!("capability binding: {full} bytes in full, {referenced} bytes by reference");

    match &second.payload {
        EntryPayload::TurnCapabilitiesRef(reference) => {
            assert_eq!(reference.entry, first.id);
            assert_eq!(reference.epoch, 2, "the epoch is still recorded");
        }
        other => panic!("an unchanged binding was rewritten: {other:?}"),
    }
    // The interface is an object, so neither write carries the prompt.
    assert!(referenced < full, "{referenced} vs {full}");
    assert!(!SessionLog::text(&path).contains("system system"));

    let third = log
        .append_turn_capabilities(bound(2, "a different system prompt "))
        .unwrap();
    assert!(matches!(
        third.payload,
        EntryPayload::TurnCapabilitiesBound(_)
    ));
    assert!(log.derive_messages().is_empty());
}

/// The conversation thread is a tail section (docs/design/68-context-engine.md
/// §6/§10), not spliced into the projection, and it lists only directives no
/// longer verbatim among `derive_messages()` — one still present in the
/// working set needs no restating (§6 "one source per fact").
#[test]
fn conversation_thread_lists_only_directives_dropped_by_compaction() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("thread.jsonl"), header()).unwrap();

    // Turn 1
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 1,
        relation: vak_intent::GoalRelation::New,
        request: "initial research on WEF".into(),
        supersedes_revision: None,
        explicit: false,
    })
    .unwrap();
    log.append_message(user_msg("initial research on WEF"))
        .unwrap();
    log.append_message(assistant_msg("here are findings"))
        .unwrap();

    // Turn 2
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 2,
        relation: vak_intent::GoalRelation::AddsTo,
        request: "use python sandbox".into(),
        supersedes_revision: None,
        explicit: false,
    })
    .unwrap();
    log.append_message(user_msg("use python sandbox")).unwrap();
    log.append_message(assistant_msg("running in sandbox"))
        .unwrap();

    // Turn 3: User drifts to GDP task
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 3,
        relation: vak_intent::GoalRelation::AddsTo,
        request: "now evaluate global GDP past 5 years".into(),
        supersedes_revision: None,
        explicit: false,
    })
    .unwrap();
    log.append_message(user_msg("now evaluate global GDP past 5 years"))
        .unwrap();

    // While every directive is still verbatim in the working set, the
    // projection carries none of them (they moved to the tail) and the
    // tail itself has nothing to add — restating a directive already in
    // `derive_messages()` would duplicate a fact already sent.
    assert!(
        log.derive_messages()
            .iter()
            .all(|m| !m.text_content().contains("<conversation_thread")),
        "the thread must never be spliced into the projection"
    );
    assert!(
        log.tail_sections(None).thread.is_none(),
        "nothing is dropped from the working set yet, so the thread has nothing to add"
    );

    // Packet turns 1 and 2 away; only turn 3's directive stays verbatim.
    let index = vak_session::TurnIndex::from_log(&log);
    let (t1, t2) = (index.turns[0].id.clone(), index.turns[1].id.clone());
    log.append_packet(&t1, &t2, "m", "summary of turns 1-2".into(), 999)
        .unwrap();
    let plan = WorkingSetPlan {
        packet_range: Some((t1, t2)),
        ..WorkingSetPlan::default()
    };

    let joined = log
        .derive_with_plan(&plan)
        .iter()
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !joined.contains("<conversation_thread"),
        "the thread must never be spliced into the projection"
    );

    let thread_text = log
        .tail_sections(Some(&plan))
        .thread
        .expect("packeted-away directives surface in the thread");
    assert!(thread_text.contains("revision=\"3\""));
    assert!(thread_text.contains("initial research on WEF"));
    assert!(thread_text.contains("use python sandbox"));
    // Turn 3's directive is still verbatim in the working set (it was kept,
    // not compacted), so restating it in the thread would duplicate it.
    assert!(!thread_text.contains("now evaluate global GDP past 5 years"));
    assert!(thread_text.contains("Follow the user's intent across conversational drifts"));
    assert!(thread_text.contains("Conversational drift across turns is expected: follow along smoothly and adapt immediately."));
    assert!(thread_text.contains("If genuinely confused, ask a brief clarification, but NEVER use asking clarification as an exception-handling escape hatch"));
}

/// Replaces the old character-count trim: a closed turn's historical tool
/// result is never truncated in the ledger, and never appears as a raw
/// `ToolResult` block in the projection at all — it becomes a trace line
/// naming its evidence id, and the full content is recoverable via
/// `SessionLog::evidence` (docs/design/68-context-engine.md §3, §10).
#[test]
fn historical_tool_result_projects_as_a_trace_line_and_evidence_returns_it_whole() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("prune.jsonl"), header()).unwrap();

    // Turn 1: closed, with a 5,000-char search result.
    log.append_message(user_msg("search the web")).unwrap();
    let call_id = "call-1".to_string();
    log.append_message(MessageRecord {
        message: Message {
            role: vak_llm::Role::Assistant,
            content: vec![vak_llm::ContentBlock::ToolUse {
                id: call_id.clone(),
                name: "search".into(),
                input: serde_json::json!({}),
            }],
        },
        meta: None,
    })
    .unwrap();
    let giant_output = "A".repeat(5_000);
    log.append_message(MessageRecord {
        message: Message {
            role: vak_llm::Role::User,
            content: vec![vak_llm::ContentBlock::tool_result(
                call_id.clone(),
                giant_output.clone(),
            )],
        },
        meta: None,
    })
    .unwrap();
    log.append_message(assistant_msg("found results")).unwrap();

    // Turn 2: still open, with its own tool result.
    log.append_message(user_msg("now run python")).unwrap();
    let call_id_2 = "call-2".to_string();
    log.append_message(MessageRecord {
        message: Message {
            role: vak_llm::Role::Assistant,
            content: vec![vak_llm::ContentBlock::ToolUse {
                id: call_id_2.clone(),
                name: "python".into(),
                input: serde_json::json!({}),
            }],
        },
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message {
            role: vak_llm::Role::User,
            content: vec![vak_llm::ContentBlock::tool_result(
                call_id_2.clone(),
                "small output".to_string(),
            )],
        },
        meta: None,
    })
    .unwrap();

    let messages = log.derive_messages();

    // Turn 1 is closed: its tool_use/tool_result pair is still there (the
    // call pattern is what a later turn imitates), but the result is the
    // schema-driven digest naming the evidence id — never the 5,000 raw
    // characters and never a character-count trim marker.
    let closed_result = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|b| match b {
            vak_llm::ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } if tool_use_id == &call_id => Some(content.clone()),
            _ => None,
        })
        .expect("the closed turn keeps its tool_use/tool_result pair");
    assert!(
        closed_result.contains("[evidence:call-1"),
        "the digest must name the evidence id: {closed_result}"
    );
    assert!(
        !closed_result.contains(&giant_output),
        "a closed turn's tool result must not appear verbatim"
    );
    assert!(
        !closed_result.contains("trimmed"),
        "no character-count trim marker anywhere"
    );

    // Turn 2 is still open: its tool result stays verbatim.
    let turn2_result = messages
        .iter()
        .find(|m| {
            m.content.iter().any(|b| {
                matches!(b, vak_llm::ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == &call_id_2)
            })
        })
        .expect("the open turn's tool result stays a raw block");
    let turn2_text = match &turn2_result.content[0] {
        vak_llm::ContentBlock::ToolResult { content, .. } => content,
        _ => unreachable!(),
    };
    assert_eq!(turn2_text, "small output");

    // The full content is still recoverable whole, via evidence().
    let evidence = log
        .evidence(&call_id)
        .expect("evidence for a closed turn's call");
    assert_eq!(evidence.content.len(), 5_000);
    assert_eq!(evidence.tool, "search");
    assert!(!evidence.is_error);
}

/// Sealing a segment changes how its frames are stored, never what the
/// model is sent: the projection is the same before the seal, after it,
/// after reopening, and with entries appended to the next segment.
#[test]
fn derive_messages_identical_across_seal() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("sealed");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    for n in 0..4 {
        log.append_message(MessageRecord {
            message: vak_llm::Message::user_text(format!("question {n}")),
            meta: None,
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: vak_llm::Message::assistant(vec![vak_llm::ContentBlock::text(format!(
                "answer {n}"
            ))]),
            meta: None,
        })
        .unwrap();
    }
    let projected = |log: &SessionLog| serde_json::to_string(&log.derive_messages()).unwrap();
    let before = projected(&log);

    log.seal_segment().unwrap();
    assert_eq!(projected(&log), before, "sealing changed the projection");
    drop(log);

    let mut reopened = SessionLog::open(path.clone()).unwrap();
    assert_eq!(
        projected(&reopened),
        before,
        "a sealed ledger reads back differently"
    );
    assert!(
        SessionLog::segment_files(&path)
            .iter()
            .any(|segment| segment.extension().is_some_and(|ext| ext == "sealed")),
        "the first segment is sealed"
    );

    reopened
        .append_message(MessageRecord {
            message: vak_llm::Message::user_text("after the seal"),
            meta: None,
        })
        .unwrap();
    let extended = projected(&reopened);
    drop(reopened);
    assert_eq!(projected(&SessionLog::open(path).unwrap()), extended);
}
