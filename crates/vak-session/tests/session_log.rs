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
            system_prompt: "system prompt v1".into(),
            tools: vec!["read".into(), "bash".into()],
            permission_mode: "workspace-write".into(),
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
