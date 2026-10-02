#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A reset-with-handoff replaces what came before it, but the turn still being
//! worked keeps its directive and every step after the reset
//! (docs/design/68-context-engine.md §4).

use tempfile::tempdir;
use vak_llm::{ContentBlock, Message, Role};
use vak_session::types::{FrozenContract, MessageRecord, SessionHeader};
use vak_session::{SessionLog, TurnIndex, WorkingSetPlan};

fn open(dir: &std::path::Path) -> SessionLog {
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-reset".into(),
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

fn rec(message: Message) -> MessageRecord {
    MessageRecord {
        message,
        meta: None,
    }
}

fn call(id: &str) -> Message {
    Message::assistant(vec![ContentBlock::ToolUse {
        id: id.into(),
        name: "read".into(),
        input: serde_json::json!({"path": "a.md"}),
    }])
}

fn result(id: &str, text: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![ContentBlock::tool_result(id, text)],
    }
}

fn text(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|m| {
            m.content
                .iter()
                .map(|b| match b {
                    ContentBlock::Text { text } => text.clone(),
                    ContentBlock::ToolResult { content, .. } => content.clone(),
                    ContentBlock::ToolUse { input, .. } => input.to_string(),
                    _ => String::new(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_open_turn_keeps_its_directive_and_later_steps_across_a_reset() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let settled = log
        .append_message(rec(Message::user_text("an earlier question")))
        .unwrap()
        .id;
    log.append_message(rec(Message::assistant(vec![ContentBlock::text(
        "an earlier answer",
    )])))
    .unwrap();
    log.append_message(rec(Message::user_text("build the quarterly report")))
        .unwrap();
    log.append_message(rec(call("t1"))).unwrap();
    log.append_message(rec(result("t1", "BEFORE-RESET-RESULT")))
        .unwrap();
    log.append_handoff_reset("HANDOFF SUMMARY".into(), 1_000, true)
        .unwrap();
    log.append_message(rec(call("t2"))).unwrap();
    log.append_message(rec(result("t2", "AFTER-RESET-RESULT")))
        .unwrap();

    let (messages, directive_at) = log.derive_with_plan_and_directive(&WorkingSetPlan::default());
    let seen = text(&messages);
    assert!(seen.contains("HANDOFF SUMMARY"));
    assert!(
        seen.contains("build the quarterly report"),
        "the task stays: {seen}"
    );
    assert!(
        seen.contains("AFTER-RESET-RESULT"),
        "later steps stay: {seen}"
    );
    assert!(
        !seen.contains("BEFORE-RESET-RESULT"),
        "the summary stands in for it"
    );
    assert!(!seen.contains("an earlier question"));
    assert_eq!(
        messages[directive_at].text_content(),
        "build the quarterly report",
        "the tail attaches to the directive"
    );
    assert_eq!(messages.last().unwrap().role, Role::User);

    let index = TurnIndex::from_log(&log);
    assert!(index.turn_by_id(&settled).unwrap().behind_reset);
    assert!(!index.turns.last().unwrap().behind_reset);
    assert_eq!(
        log.open_turn_verbatim().len(),
        3,
        "directive plus the two steps after the reset"
    );
}

#[test]
fn a_reset_before_the_first_step_leaves_the_directive_visible() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    log.append_message(rec(Message::user_text("summarise the contract")))
        .unwrap();
    log.append_handoff_reset("HANDOFF SUMMARY".into(), 1_000, true)
        .unwrap();
    let (messages, _) = log.derive_with_plan_and_directive(&WorkingSetPlan::default());
    let seen = text(&messages);
    assert!(seen.contains("HANDOFF SUMMARY") && seen.contains("summarise the contract"));
}

#[test]
fn a_finished_turn_is_hidden_by_a_later_reset() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    log.append_message(rec(Message::user_text("old question")))
        .unwrap();
    log.append_message(rec(Message::assistant(vec![ContentBlock::text(
        "old answer",
    )])))
    .unwrap();
    log.append_handoff_reset("HANDOFF SUMMARY".into(), 1_000, true)
        .unwrap();
    let (messages, _) = log.derive_with_plan_and_directive(&WorkingSetPlan::default());
    let seen = text(&messages);
    assert!(!seen.contains("old question") && !seen.contains("old answer"));
    assert!(TurnIndex::from_log(&log).turns[0].behind_reset);
}

#[test]
fn a_reset_that_does_not_keep_the_open_turn_hides_it_for_the_summary_to_stand_in() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    log.append_message(rec(Message::user_text("a directive too large to send")))
        .unwrap();
    log.append_message(rec(call("t1"))).unwrap();
    log.append_message(rec(result("t1", "EARLY-RESULT")))
        .unwrap();
    log.append_handoff_reset("HANDOFF SUMMARY".into(), 1_000, false)
        .unwrap();
    let (messages, _) = log.derive_with_plan_and_directive(&WorkingSetPlan::default());
    let seen = text(&messages);
    assert!(seen.contains("HANDOFF SUMMARY"));
    assert!(!seen.contains("a directive too large") && !seen.contains("EARLY-RESULT"));
}
