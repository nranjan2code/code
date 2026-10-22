#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A turn a person set aside is not cited and not sent, and is again once
//! restored; nothing is removed (docs/design/99-turns-that-do-not-poison.md
//! §4 F).

use tempfile::tempdir;
use vak_llm::{ContentBlock, Message, Role};
use vak_session::turn_marks::TurnMarkRecord;
use vak_session::types::{FrozenContract, MessageRecord, SessionHeader};
use vak_session::{Fidelity, SessionLog, TurnIndex, WorkingSetPlan};

fn open(dir: &std::path::Path) -> SessionLog {
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-aside".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "scripted".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: String::new(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    SessionLog::create(dir.join("s.jsonl"), header).unwrap()
}

fn append(log: &mut SessionLog, message: Message) -> String {
    log.append_message(MessageRecord {
        message,
        meta: None,
    })
    .unwrap()
    .id
}

#[test]
fn a_turn_set_aside_is_neither_cited_nor_sent_until_restored() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let first = append(&mut log, Message::user_text("find a route"));
    append(
        &mut log,
        Message::assistant(vec![ContentBlock::ToolUse {
            id: "c1".into(),
            name: "webfetch".into(),
            input: serde_json::json!({"url": "https://example.org/route"}),
        }]),
    );
    append(
        &mut log,
        Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result(
                "c1",
                "https://example.org/made-up",
            )],
        },
    );
    append(
        &mut log,
        Message::assistant(vec![ContentBlock::text("The route is made up.")]),
    );
    append(&mut log, Message::user_text("try again"));

    let cites = |log: &SessionLog| log.evidence_text(|_, _| true).contains("made-up");
    // Even a plan that asks for the turn in full does not get it.
    let asks = WorkingSetPlan {
        per_turn: vec![(first.clone(), Fidelity::Full)],
        ..Default::default()
    };
    let sent = |log: &SessionLog| {
        log.derive_with_plan(&asks)
            .iter()
            .any(|message| message.text_content().contains("find a route"))
    };
    let mark = || TurnMarkRecord {
        turn: first.clone(),
        trace: None,
        actor: None,
    };
    assert!(cites(&log));
    assert!(sent(&log));

    log.append_turn_mark(true, mark()).unwrap();
    assert!(TurnIndex::from_log(&log).turns[0].set_aside);
    assert!(!cites(&log));
    assert!(!sent(&log));
    // Nothing is removed: the person still reads the turn.
    assert!(
        log.derive_transcript()
            .iter()
            .any(|item| item.message.text_content().contains("find a route"))
    );

    log.append_turn_mark(false, mark()).unwrap();
    assert!(!TurnIndex::from_log(&log).turns[0].set_aside);
    assert!(cites(&log));
    assert!(sent(&log));
}

/// Retrying a turn starts a branch before it: the turn sent again follows
/// the turn before, the retried turn and everything after it leave the
/// chain, and the branch is what a reopened ledger follows (doc 99 §4 F).
#[test]
fn a_retried_turn_is_answered_again_on_a_new_branch() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let first = append(&mut log, Message::user_text("name the codename"));
    append(
        &mut log,
        Message::assistant(vec![ContentBlock::text("It is Heron.")]),
    );
    let second = append(&mut log, Message::user_text("what is the route?"));
    append(
        &mut log,
        Message::assistant(vec![ContentBlock::text("The route is made up.")]),
    );
    append(&mut log, Message::user_text("and after that?"));
    append(
        &mut log,
        Message::assistant(vec![ContentBlock::text("Nothing after.")]),
    );

    let directive = log.retry_turn(&second, None).unwrap();
    assert_eq!(directive.message.text_content(), "what is the route?");
    let turns = |log: &SessionLog| -> Vec<String> {
        TurnIndex::from_log(log)
            .turns
            .iter()
            .map(|turn| turn.directive.text_content())
            .collect()
    };
    assert_eq!(turns(&log), ["name the codename"]);
    let retried = append(&mut log, directive.message);
    assert_ne!(retried, second);
    append(
        &mut log,
        Message::assistant(vec![ContentBlock::text("The route is the river path.")]),
    );
    assert_eq!(turns(&log), ["name the codename", "what is the route?"]);
    let spoken = |log: &SessionLog| -> String {
        log.derive_transcript()
            .iter()
            .map(|item| item.message.text_content())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(spoken(&log).contains("river path"));
    assert!(!spoken(&log).contains("made up"));
    assert!(!spoken(&log).contains("Nothing after"));
    assert!(
        log.derive_messages()
            .iter()
            .all(|m| !m.text_content().contains("made up"))
    );

    let path = log.path().to_path_buf();
    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(
        turns(&reopened),
        ["name the codename", "what is the route?"]
    );
    assert_eq!(TurnIndex::from_log(&reopened).turns[1].id, retried);
    assert!(!spoken(&reopened).contains("made up"));

    // The first turn retried leaves only the header behind it.
    let mut reopened = reopened;
    reopened.retry_turn(&first, None).unwrap();
    assert!(turns(&reopened).is_empty());
    assert!(reopened.retry_turn(&second, None).is_err());
}
