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
use vak_session::types::{FrozenContract, IntentRecord, MessageRecord, SessionHeader};
use vak_session::{SessionLog, SessionPath};

fn header() -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: "s-intent".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
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
        outcome: None,
        model_visible: note.map(str::to_string),
        commitment_id: None,
        strands: Vec::new(),
        strand_commitments: Default::default(),
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

/// The intent note reaches the model through the request tail
/// (docs/design/68-context-engine.md §6/§10), not spliced into the
/// projection: `derive_messages()` carries only the conversation, and
/// `tail_sections()` carries the note the assembler renders alongside it.
#[test]
fn an_intent_note_reaches_the_model_through_the_tail_not_the_projection() {
    let dir = tempdir().unwrap();
    let path = SessionPath::new_session_file(dir.path(), &PathBuf::from("/tmp/proj"), "s-intent");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    log.append_intent(record(Some("Cite your sources.")))
        .unwrap();
    log.append_message(user("what changed in the parser?"))
        .unwrap();

    let messages = log.derive_messages();
    let texts: Vec<String> = messages.iter().map(|m| m.text_content()).collect();
    assert!(
        !texts.iter().any(|t| t.contains("Cite your sources.")),
        "the intent note must not be spliced into the projection: {texts:?}"
    );
    assert!(
        texts
            .iter()
            .any(|t| t.contains("what changed in the parser?")),
        "user turn present"
    );

    let tail = log.tail_sections(None);
    let intent = tail.intent.expect("intent tail section present");
    assert!(intent.contains("Cite your sources."));
    assert!(intent.starts_with("<intent>"));
}

/// A long note in a non-Latin script reaches the tail whole. The tail used
/// to be cut at a byte offset, which panics inside a multi-byte character
/// and silently drops the rest (AGENTS.md invariants 3 and 36).
#[test]
fn a_long_non_ascii_intent_note_reaches_the_tail_whole() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let note = "कृपया हर भाग का उत्तर दें। ".repeat(300);
    assert!(note.len() > 8_000, "longer than any old cut");
    log.append_intent(record(Some(&note))).unwrap();
    let intent = log.tail_sections(None).intent.expect("intent tail section");
    assert!(intent.contains(note.trim_end()));
}

/// `ContextProfile::Working` / `Full`: the workspace delta the host recorded
/// for this turn reaches the model through the tail, from the ledger bytes,
/// and only when the turn's reading asked for it.
#[test]
fn a_workspace_delta_reaches_the_tail_only_for_a_working_reading() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let delta = |id: &str| vak_session::ActivityRecord {
        activity_id: id.into(),
        turn: None,
        kind: vak_session::ActivityKind::Diagnostic,
        status: vak_session::ActivityStatus::Succeeded,
        label: "Workspace changes since the session began".into(),
        detail: Some("M src/parser.rs (+12 -3)".into()),
        data: std::collections::BTreeMap::from([(
            "section".to_string(),
            SessionLog::WORKSPACE_DELTA_SECTION.to_string(),
        )]),
    };

    // A recall reading: the delta is in the ledger but not in the tail.
    log.append_intent(record(None)).unwrap();
    log.append_activity(delta("wd-1")).unwrap();
    log.append_message(user("explain the parser")).unwrap();
    assert!(log.tail_sections(None).workspace.is_none());

    // A working reading on the next turn, with its own delta activity.
    let mut working = record(None);
    working.engagement.posture.context = vak_intent::ContextProfile::Working;
    log.append_intent(working).unwrap();
    log.append_activity(delta("wd-2")).unwrap();
    log.append_message(user("now fix the failing test"))
        .unwrap();
    let tail = log.tail_sections(None);
    let workspace = tail.workspace.expect("workspace section present");
    assert!(workspace.starts_with("<workspace_delta>"), "{workspace}");
    assert!(workspace.contains("src/parser.rs"));

    // A working reading whose turn recorded no delta shows nothing — a
    // previous turn's delta is never presented as current.
    let mut working = record(None);
    working.engagement.posture.context = vak_intent::ContextProfile::Working;
    log.append_intent(working).unwrap();
    log.append_message(user("and run it")).unwrap();
    assert!(log.tail_sections(None).workspace.is_none());
}

#[test]
fn goal_projection_ignores_control_updates_when_finding_active_work() {
    let dir = tempdir().unwrap();
    let path = SessionPath::new_session_file(dir.path(), &PathBuf::from("/tmp/proj"), "s-intent");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 1,
        relation: vak_intent::GoalRelation::New,
        request: "build the report".into(),
        supersedes_revision: None,
        explicit: false,
    })
    .unwrap();
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 2,
        relation: vak_intent::GoalRelation::Status,
        request: "what is the status?".into(),
        supersedes_revision: None,
        explicit: false,
    })
    .unwrap();
    let state = log.goal_state().expect("goal state exists");
    assert_eq!(state.objective, "build the report");
    assert_eq!(state.revision, 2);
    // A status check is not work: the next message adds to the goal, and its
    // revision follows the status update rather than reusing it.
    let next = log.next_goal_update("include the regional split");
    assert_eq!(next.relation, vak_intent::GoalRelation::AddsTo);
    assert_eq!(next.revision, 3);
    drop(log);
    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(reopened.goal_state(), Some(state));
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
    // None of the notes are spliced into the projection any more — they
    // reach the model through the tail instead.
    assert!(!joined.contains("FIRST directive"));
    assert!(!joined.contains("SECOND directive"));
    assert!(!joined.contains("THIRD directive"));
    // The turns themselves all survive; only the guidance moved.
    for turn in ["one", "two", "three"] {
        assert!(joined.contains(turn), "lost turn `{turn}`");
    }

    let intent = log.tail_sections(None).intent.expect("newest note present");
    assert!(intent.contains("THIRD directive"));
    assert!(!intent.contains("FIRST directive"));
    assert!(!intent.contains("SECOND directive"));
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
    let intent = reopened
        .tail_sections(None)
        .intent
        .expect("intent tail section present after reopening");
    assert!(intent.contains("a rule this build would never generate"));
}

/// An intent note is runtime guidance for the turn in flight, not
/// conversation. It must not be swept into a compaction packet's
/// summariser input as though the user had said it.
#[test]
fn intent_notes_are_control_and_stay_out_of_the_compaction_packet() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let mut ids = Vec::new();
    for index in 0..6 {
        log.append_intent(record(Some(&format!("directive {index}"))))
            .unwrap();
        ids.push(
            log.append_message(user(&format!("turn {index}")))
                .unwrap()
                .id,
        );
        log.append_message(vak_session::types::MessageRecord {
            message: Message::assistant(vec![vak_llm::ContentBlock::text(format!(
                "answer {index}"
            ))]),
            meta: None,
        })
        .unwrap();
    }
    let (older, _) = log.packet_transcript(&ids[0], &ids[3]);
    assert!(
        !older.contains("<intent>"),
        "an intent note was queued for summarization: {older}"
    );
    assert!(older.contains("turn 0"), "{older}");
    assert!(!older.contains("turn 4"), "{older}");
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
