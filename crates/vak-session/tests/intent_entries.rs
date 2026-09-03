//! Intent entries in the ledger and the projection.
//!
//! The property under test is `AGENTS.md` invariant 1 applied to intent:
//! whatever the engagement tells the model must be reconstructable from the
//! JSONL, and reconstructing it must produce the same prompt — not a fresh
//! derivation that today's rules happen to agree with.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::Message;
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, IntentRecord, MessageRecord, SessionHeader};

fn header() -> SessionHeader {
    SessionHeader {
        session_id: "s-intent".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        contract: FrozenContract {
            app_version: "2.0.1".into(),
            provider: "anthropic".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "system".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn record(note: Option<&str>) -> IntentRecord {
    IntentRecord {
        reading: vak_intent::Reading::general(),
        engagement: vak_intent::Engagement::general(),
        provenance: vak_intent::Provenance::new(
            vak_intent::Tier::Signals,
            vak_intent::RESOLVER_VERSION,
            Vec::new(),
        ),
        model_visible: note.map(str::to_string),
        commitment_id: None,
    }
}

fn open(dir: &std::path::Path) -> SessionLog {
    SessionLog::create(
        vak_session::SessionPath::new_session_file(dir, &PathBuf::from("/tmp/proj"), "s-intent"),
        header(),
    )
    .unwrap()
}

fn user(text: &str) -> MessageRecord {
    MessageRecord {
        message: Message::user_text(text),
        meta: None,
    }
}

#[test]
fn an_intent_note_reaches_the_model_immediately_before_its_turn() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    log.append_intent(record(Some("Cite your sources.")))
        .unwrap();
    log.append_message(user("what changed in the parser?"))
        .unwrap();

    let messages = log.derive_messages();
    let texts: Vec<String> = messages.iter().map(|m| m.text_content()).collect();
    let note_at = texts
        .iter()
        .position(|t| t.contains("Cite your sources."))
        .expect("intent note reached the model");
    let turn_at = texts
        .iter()
        .position(|t| t.contains("what changed in the parser?"))
        .expect("user turn present");
    assert_eq!(
        note_at + 1,
        turn_at,
        "the note must sit directly before the turn it governs, not at the top \
         of the conversation: {texts:?}"
    );
    assert!(texts[note_at].contains("<intent>"));
}

/// Per-turn guidance must not accumulate. Five stale notes waste context and
/// let an old instruction argue with the current one.
#[test]
fn only_the_newest_intent_note_is_projected() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    log.append_intent(record(Some("FIRST directive"))).unwrap();
    log.append_message(user("one")).unwrap();
    log.append_intent(record(Some("SECOND directive"))).unwrap();
    log.append_message(user("two")).unwrap();
    log.append_intent(record(Some("THIRD directive"))).unwrap();
    log.append_message(user("three")).unwrap();

    let joined = log
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("THIRD directive"));
    assert!(!joined.contains("FIRST directive"));
    assert!(!joined.contains("SECOND directive"));
    // The turns themselves all survive; only the guidance is superseded.
    for turn in ["one", "two", "three"] {
        assert!(joined.contains(turn), "lost turn `{turn}`");
    }
}

#[test]
fn an_engagement_with_nothing_to_say_contributes_no_tokens() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    log.append_intent(record(None)).unwrap();
    log.append_message(user("hi")).unwrap();

    let messages = log.derive_messages();
    assert_eq!(messages.len(), 1, "a silent engagement added a message");
    assert!(!messages[0].text_content().contains("<intent>"));
}

/// The prompt is reconstructed from the recorded bytes, not re-derived. A
/// ledger written by an older build must replay to the same prompt even if the
/// derivation rules have since changed.
#[test]
fn replaying_a_ledger_reproduces_the_recorded_note_verbatim() {
    let dir = tempdir().unwrap();
    let path = {
        let mut log = open(dir.path());
        log.append_intent(record(Some("a rule this build would never generate")))
            .unwrap();
        log.append_message(user("go")).unwrap();
        log.path().to_path_buf()
    };

    let reopened = SessionLog::open(path.clone()).unwrap();
    let joined = reopened
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("a rule this build would never generate"));
}

/// An intent note is runtime guidance for the turn in flight, not
/// conversation. It must not be swept into a compaction summary as though the
/// user had said it.
#[test]
fn intent_notes_are_control_and_stay_out_of_the_compaction_packet() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    for index in 0..6 {
        log.append_intent(record(Some(&format!("directive {index}"))))
            .unwrap();
        log.append_message(user(&format!("turn {index}"))).unwrap();
    }
    let plan = log.plan_compaction(2).expect("a plan over six turns");
    let older: Vec<String> = plan.older.iter().map(|m| m.text_content()).collect();
    assert!(
        !older.iter().any(|text| text.contains("<intent>")),
        "an intent note was queued for summarization: {older:?}"
    );
    assert!(older.iter().any(|text| text.contains("turn 0")));
}

#[test]
fn intent_entries_are_append_only_and_keep_the_hash_chain_intact() {
    let dir = tempdir().unwrap();
    let path = {
        let mut log = open(dir.path());
        log.append_intent(record(Some("note"))).unwrap();
        log.append_message(user("go")).unwrap();
        log.path().to_path_buf()
    };
    let reopened = SessionLog::open(path.clone()).unwrap();
    assert!(
        reopened.warnings().is_empty(),
        "intent entries broke the ledger hash chain: {:?}",
        reopened.warnings()
    );
}
