#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message, Role, Usage};
use vak_session::SessionLog;
use vak_session::types::{EntryPayload, FrozenContract, MessageMeta, MessageRecord, SessionHeader};

fn header() -> SessionHeader {
    SessionHeader {
        session_id: "s-test".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            route_ladder: Vec::new(),
            system_prompt: "system prompt v1".into(),
            tools: vec!["read".into(), "bash".into()],
            permission_mode: "workspace-write".into(),
            skills: Vec::new(),
        },
    }
}

fn user_msg(text: &str) -> MessageRecord {
    MessageRecord {
        message: Message::user_text(text),
        meta: None,
    }
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
