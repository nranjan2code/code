#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;
use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message, Role, Usage};
use vak_session::types::{
    EntryPayload, FrozenContract, MessageMeta, MessageRecord, SessionHeader, TurnCapabilitiesBound,
    WorkContract, WorkEvent, WorkEventKind, WorkItemDefinition, WorkOwner,
};
use vak_session::{ActivityKind, ActivityRecord, ActivityStatus, SessionLog};

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

fn user_msg(text: &str) -> MessageRecord {
    MessageRecord {
        message: Message::user_text(text),
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
