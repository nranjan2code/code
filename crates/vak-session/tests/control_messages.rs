#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Control traffic is tagged structurally and carries ledger identity, so no
//! consumer has to guess from text or count messages to line things up.

use tempfile::tempdir;
use vak_intent::control::ControlKind;
use vak_llm::{ContentBlock, Message, Role};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{MessageRecord, SessionLog};

fn log(dir: &tempfile::TempDir) -> SessionLog {
    SessionLog::create(
        dir.path().join("s.jsonl"),
        SessionHeader {
            space: None,
            run: None,
            cause: None,
            agent: None,
            session_id: "s".into(),
            created_at: chrono::Utc::now(),
            cwd: dir.path().to_path_buf(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
            conversation: None,
            contract: FrozenContract {
                app_version: "0".into(),
                provider: "t".into(),
                model: "t".into(),
                route_ladder: Vec::new(),
                route_objective: String::new(),
                route_annotations: Vec::new(),
                system_prompt: String::new(),
                permission_mode: "read-only".into(),
                capabilities: Vec::new(),
                prompt_layers: Vec::new(),
            },
        },
    )
    .unwrap()
}

#[test]
fn the_structural_tag_wins_even_when_the_body_has_no_marker() {
    let record = MessageRecord::control(ControlKind::FenceCheck, "please resend");
    assert_eq!(record.control_kind(), Some(ControlKind::FenceCheck));
}

#[test]
fn text_alone_never_makes_a_message_control() {
    // A message is control only if the runtime tagged it. One whose text
    // merely begins with a marker (a user pasting a log line, say) is the
    // user's own.
    let untagged = MessageRecord {
        message: Message::user_text("[grounding-check]: cite your results"),
        meta: None,
    };
    assert_eq!(untagged.control_kind(), None);
}

#[test]
fn what_the_user_wrote_is_never_control() {
    for text in [
        "how did the market do",
        "[ERROR]: it broke, help",
        "see [stop-hook] docs",
    ] {
        let record = MessageRecord {
            message: Message::user_text(text),
            meta: None,
        };
        assert_eq!(record.control_kind(), None, "{text}");
    }
    let assistant = MessageRecord {
        message: Message::assistant(vec![ContentBlock::text("[fence-check]: quoting a nudge")]),
        meta: None,
    };
    assert_eq!(
        assistant.control_kind(),
        None,
        "only user-role text can be a control message"
    );
}

#[test]
fn the_transcript_carries_identity_and_class_for_every_message() {
    let dir = tempdir().unwrap();
    let mut log = log(&dir);
    log.append_message(MessageRecord {
        message: Message::user_text("what is the weather"),
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text("sunny")]),
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord::control(
        ControlKind::PresentationCheck,
        "[presentation-check]: use a card",
    ))
    .unwrap();
    let transcript = log.derive_transcript();
    assert_eq!(transcript.len(), 3);
    assert_eq!(transcript[0].control, None);
    assert_eq!(transcript[2].control, Some(ControlKind::PresentationCheck));
    assert_eq!(transcript[2].message.role, Role::User);
    let mut ids: Vec<_> = transcript.iter().map(|m| m.entry_id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3, "every message has its own ledger identity");
}
