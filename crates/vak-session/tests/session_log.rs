#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use tempfile::tempdir;
use vak_llm::Message;
use vak_session::{FrozenContract, MessageRecord, SessionHeader, SessionLog};

fn header(id: &str) -> SessionHeader {
    SessionHeader {
        session_id: id.into(),
        project_id: "project-a".into(),
        created_at: chrono::Utc::now(),
        project_root: PathBuf::from("/tmp/project-a"),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0.10.0".into(),
            provider: "test".into(),
            model: "model".into(),
            system_prompt: "system".into(),
            tools: vec!["read".into()],
            permission_mode: "read-only".into(),
        },
    }
}

#[test]
fn uses_project_and_session_layout_and_derives_messages() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create_at(dir.path(), "project-a", header("session-1")).unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("hello"),
    })
    .unwrap();
    assert_eq!(
        log.path(),
        dir.path().join("sessions/project-a/session-1.jsonl")
    );
    assert_eq!(log.derive_messages()[0].text_content(), "hello");
}

#[test]
fn branch_projection_uses_parent_chain() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create_at(dir.path(), "project-a", header("s")).unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("a"),
    })
    .unwrap();
    let fork = log
        .append_message(MessageRecord {
            message: Message::user_text("b"),
        })
        .unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("c"),
    })
    .unwrap();
    log.branch_at(&fork.id).unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("d"),
    })
    .unwrap();
    let texts: Vec<_> = log
        .derive_messages()
        .iter()
        .map(Message::text_content)
        .collect();
    assert_eq!(texts, ["a", "b", "d"]);
}

#[test]
fn torn_final_line_is_recoverable_but_interior_corruption_is_not_silently_hidden() {
    use std::io::Write;
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut log = SessionLog::create(path.clone(), header("s")).unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("safe"),
    })
    .unwrap();
    drop(log);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    write!(file, "{{\"id\":\"torn\",\"kind\":").unwrap();
    drop(file);
    let recovered = SessionLog::open(path).unwrap();
    assert_eq!(recovered.derive_messages()[0].text_content(), "safe");
    assert_eq!(recovered.warnings().len(), 1);
}

#[test]
fn exclusive_lock_and_identifier_validation() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let log = SessionLog::create(path.clone(), header("s")).unwrap();
    assert!(SessionLog::open(path).is_err());
    drop(log);
    assert!(SessionLog::create_at(dir.path(), "../escape", header("x")).is_err());
}
