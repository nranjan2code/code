#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message, Role, Usage};
use vak_session::types::{
    EntryPayload, FrozenContract, MessageMeta, MessageRecord, SessionHeader, TurnCapabilitiesBound,
    WorkContract, WorkEvent, WorkEventKind, WorkItemDefinition, WorkOwner,
};
use vak_session::{ActivityKind, ActivityRecord, ActivityStatus, SessionLog, SessionPath};

fn header() -> SessionHeader {
    SessionHeader {
        session_id: "s-test".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
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
        turn: Some(1),
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

#[test]
fn active_work_is_reconstructed_into_model_context() {
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
    let messages = log.derive_messages();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].text_content().contains("preserve active work"));
    assert!(messages[0].text_content().contains("inspect"));
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
    let boundary = log.append_message(user_msg("keep this")).unwrap();
    log.compact("old summary".into(), boundary.id, 9000)
        .unwrap();
    assert!(
        log.derive_messages()
            .iter()
            .any(|message| message.text_content().contains("survive compaction"))
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
    contract.items[0].owner = WorkOwner::Subagent;
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
            owner: WorkOwner::Subagent,
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
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    log.append_message(user_msg("a")).unwrap();
    let fork_point = log.append_message(user_msg("b")).unwrap();
    log.append_message(user_msg("c")).unwrap();

    log.branch_at(&fork_point.id).unwrap();
    log.append_message(user_msg("d")).unwrap();

    let texts: Vec<String> = log
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect();
    assert_eq!(texts, vec!["a", "b", "d"]);
}

#[test]
fn compaction_replaces_prefix_keeps_suffix() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    let _e1 = log.append_message(user_msg("old-1")).unwrap();
    let _e2 = log.append_message(user_msg("old-2")).unwrap();
    let e3 = log.append_message(user_msg("kept")).unwrap();

    log.compact("summary of old turns".into(), e3.id.clone(), 9000)
        .unwrap();
    log.append_message(user_msg("after")).unwrap();

    let texts: Vec<String> = log
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect();
    assert_eq!(
        texts,
        vec![
            "<context_summary>\nsummary of old turns\n</context_summary>",
            "kept",
            "after"
        ]
    );
}

#[test]
fn compaction_keeps_entries_between_marker_and_compaction_point() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();
    log.append_message(user_msg("dropped")).unwrap();
    let e2 = log.append_message(user_msg("kept-mid")).unwrap();
    log.append_message(user_msg("kept-late")).unwrap();

    log.compact("s".into(), e2.id.clone(), 100).unwrap();

    let texts: Vec<String> = log
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect();
    assert_eq!(
        texts,
        vec![
            "<context_summary>\ns\n</context_summary>",
            "kept-mid",
            "kept-late"
        ]
    );
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

#[test]
fn second_compaction_summarizes_the_prior_summary() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();

    // Seed 8 messages; compact down to last 2.
    for i in 0..8 {
        log.append_message(user_msg(&format!("m{i}"))).unwrap();
    }
    let plan1 = log.plan_compaction(2).expect("plan 1");
    // Older segment = first 6 messages.
    assert_eq!(plan1.older.len(), 6);
    log.apply_compaction(&plan1, "summary-one".into(), 9000)
        .unwrap();

    // Projection: [summary-one, m6, m7].
    let msgs = log.derive_messages();
    assert_eq!(msgs.len(), 3);
    assert!(msgs[0].text_content().contains("summary-one"));

    // Second cycle: grow past again, then compact once more.
    log.append_message(user_msg("m8")).unwrap();
    log.append_message(user_msg("m9")).unwrap();
    let plan2 = log.plan_compaction(2).expect("plan 2");
    // The new older segment must START with the prior summary message —
    // raw pre-compaction history must NOT reappear.
    assert!(
        plan2.older[0].text_content().contains("<context_summary>"),
        "repeated compaction must summarize the prior summary"
    );
    assert_eq!(plan2.older.len(), 3); // summary-one, m7, m8 (m6 stays verbatim)

    log.apply_compaction(&plan2, "summary-two".into(), 400)
        .unwrap();
    let final_msgs = log.derive_messages();
    assert!(
        final_msgs[0].text_content().contains("summary-two"),
        "latest summary wins"
    );
    assert!(
        !serde_like_contains(&final_msgs, "summary-one"),
        "old summary must be folded away"
    );
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
        .open(&path)
        .unwrap();
    write!(f, "{{\"id\":\"torn\", \"pay").unwrap();
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
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    log.append_turn_capabilities(TurnCapabilitiesBound {
        epoch: 7,
        capability_ids: vec!["Tool:read".into()],
        excluded_ids: vec!["Tool:bash".into()],
        system_prompt: "system".into(),
        tool_schemas: vec![serde_json::json!({"name":"read"})],
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

#[test]
fn conversation_thread_projects_across_multi_turn_drifts() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("thread.jsonl"), header()).unwrap();

    // Turn 1
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 1,
        relation: vak_intent::GoalRelation::New,
        request: "initial research on WEF".into(),
        supersedes_revision: None,
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
    })
    .unwrap();
    log.append_message(user_msg("now evaluate global GDP past 5 years"))
        .unwrap();

    let messages = log.derive_messages();
    let thread_msg = messages
        .iter()
        .find(|m| m.text_content().contains("<conversation_thread"));
    assert!(
        thread_msg.is_some(),
        "expected <conversation_thread> projection"
    );
    let thread_text = thread_msg.unwrap().text_content();
    assert!(thread_text.contains("revision=\"3\""));
    assert!(thread_text.contains("initial research on WEF"));
    assert!(thread_text.contains("use python sandbox"));
    assert!(thread_text.contains("now evaluate global GDP past 5 years"));
    assert!(thread_text.contains("Follow the user's intent across conversational drifts"));
    assert!(thread_text.contains("Conversational drift across turns is expected: follow along smoothly and adapt immediately."));
    assert!(thread_text.contains("If genuinely confused, ask a brief clarification, but NEVER use asking clarification as an exception-handling escape hatch"));

    // The thread must appear before the latest user message
    let last_user_idx = messages
        .iter()
        .rposition(|m| m.text_content().contains("now evaluate global GDP"))
        .unwrap();
    let thread_idx = messages
        .iter()
        .position(|m| m.text_content().contains("<conversation_thread"))
        .unwrap();
    assert!(
        thread_idx < last_user_idx,
        "thread must precede the active user turn"
    );
}

#[test]
fn historical_tool_results_are_pruned_in_projection() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("prune.jsonl"), header()).unwrap();

    // Turn 1: Huge search result (2500 chars)
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
    let giant_output = "A".repeat(2500);
    log.append_message(MessageRecord {
        message: Message {
            role: vak_llm::Role::User,
            content: vec![vak_llm::ContentBlock::tool_result(
                call_id,
                giant_output.clone(),
            )],
        },
        meta: None,
    })
    .unwrap();
    log.append_message(assistant_msg("found results")).unwrap();

    // Turn 2: Active turn with tool result (2500 chars)
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
                call_id_2,
                giant_output.clone(),
            )],
        },
        meta: None,
    })
    .unwrap();

    let messages = log.derive_messages();

    // Find Turn 1's tool result: it should be pruned
    let turn1_result = messages
        .iter()
        .find(|m| {
            m.content.iter().any(|b| match b {
                vak_llm::ContentBlock::ToolResult { tool_use_id, .. } => tool_use_id == "call-1",
                _ => false,
            })
        })
        .unwrap();

    let turn1_text = match &turn1_result.content[0] {
        vak_llm::ContentBlock::ToolResult { content, .. } => content,
        _ => unreachable!(),
    };
    assert!(
        turn1_text.len() < 600,
        "historical tool result must be truncated: was {}",
        turn1_text.len()
    );
    assert!(turn1_text.contains("historical tool output trimmed; total was 2500 chars"));

    // Find Turn 2's active tool result: it MUST remain full-size verbatim (2500 chars)
    let turn2_result = messages
        .iter()
        .find(|m| {
            m.content.iter().any(|b| match b {
                vak_llm::ContentBlock::ToolResult { tool_use_id, .. } => tool_use_id == "call-2",
                _ => false,
            })
        })
        .unwrap();

    let turn2_text = match &turn2_result.content[0] {
        vak_llm::ContentBlock::ToolResult { content, .. } => content,
        _ => unreachable!(),
    };
    assert_eq!(
        turn2_text.len(),
        2500,
        "active turn tool result must be 100% verbatim"
    );
}
