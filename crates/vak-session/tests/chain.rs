#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The ledger is the product's evidence. These assert it is durable on write
//! and tamper-evident on read.

use std::path::PathBuf;

use vak_session::types::{Entry, EntryPayload, FrozenContract, SessionHeader};
use vak_session::{ActivityKind, ActivityRecord, ActivityStatus, SessionLog};

fn header() -> SessionHeader {
    SessionHeader {
        session_id: "s-chain".into(),
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

fn activity(label: &str) -> ActivityRecord {
    ActivityRecord {
        activity_id: format!("a-{label}"),
        turn: Some(1),
        kind: ActivityKind::Diagnostic,
        status: ActivityStatus::Succeeded,
        label: label.into(),
        detail: None,
        data: Default::default(),
    }
}

fn seeded(path: &std::path::Path) -> SessionLog {
    let mut log = SessionLog::create(path.to_path_buf(), header()).unwrap();
    for label in ["first", "second", "third"] {
        log.append_activity(activity(label)).unwrap();
    }
    log
}

#[test]
fn every_appended_entry_links_to_its_predecessor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let log = seeded(&path);
    let entries = log.chain_to_root();

    assert!(
        entries[0].prev_hash.is_none(),
        "the first entry has no predecessor"
    );
    for entry in &entries[1..] {
        assert!(
            entry.prev_hash.is_some(),
            "every later entry must carry a chain link"
        );
    }
    drop(log);

    let reopened = SessionLog::open(path).unwrap();
    assert!(
        reopened.warnings().is_empty(),
        "an untouched ledger must verify clean: {:?}",
        reopened.warnings()
    );
}

/// The point of the chain: an interior edit that re-links `parent_id`
/// correctly is still caught.
#[test]
fn an_interior_edit_is_detected_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    drop(seeded(&path));

    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    assert!(lines.len() >= 3);
    lines[1] = lines[1].replace("first", "tampered");
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();

    let reopened = SessionLog::open(path).unwrap();
    assert!(
        reopened
            .warnings()
            .iter()
            .any(|w| w.contains("broken hash chain")),
        "a rewritten entry must be reported: {:?}",
        reopened.warnings()
    );
}

/// Ledgers are a frozen, append-only contract. A file written before chaining
/// existed must still open, with the gap reported rather than raised.
#[test]
fn pre_chain_ledgers_still_open_and_say_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    drop(seeded(&path));

    let text = std::fs::read_to_string(&path).unwrap();
    let stripped: Vec<String> = text
        .lines()
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let mut map = value.as_object().unwrap().clone();
            map.remove("prev_hash");
            serde_json::to_string(&map).unwrap()
        })
        .collect();
    std::fs::write(&path, stripped.join("\n") + "\n").unwrap();

    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(
        reopened.len(),
        stripped.len(),
        "an unchained ledger must still be fully readable"
    );
    assert!(
        reopened
            .warnings()
            .iter()
            .any(|w| w.contains("before hash chaining")),
        "the unverifiable gap must be reported: {:?}",
        reopened.warnings()
    );
    assert!(
        !reopened
            .warnings()
            .iter()
            .any(|w| w.contains("broken hash chain")),
        "absent links are not evidence of tampering"
    );
}

/// Appending onto a legacy ledger must start chaining from that point rather
/// than leaving the rest unverifiable forever.
#[test]
fn appending_to_a_legacy_ledger_starts_the_chain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    drop(seeded(&path));
    let text = std::fs::read_to_string(&path).unwrap();
    let stripped: Vec<String> = text
        .lines()
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let mut map = value.as_object().unwrap().clone();
            map.remove("prev_hash");
            serde_json::to_string(&map).unwrap()
        })
        .collect();
    std::fs::write(&path, stripped.join("\n") + "\n").unwrap();

    let mut log = SessionLog::open(path.clone()).unwrap();
    let appended = log.append_activity(activity("after")).unwrap();
    assert!(
        appended.prev_hash.is_some(),
        "a new append links to the last line it actually saw"
    );
    drop(log);

    let reopened = SessionLog::open(path).unwrap();
    assert!(
        !reopened
            .warnings()
            .iter()
            .any(|w| w.contains("broken hash chain")),
        "chaining onto a legacy tail is not a break: {:?}",
        reopened.warnings()
    );
}

/// A torn final line (crash mid-append) must stay recoverable.
#[test]
fn a_torn_tail_does_not_make_the_session_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    drop(seeded(&path));

    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("{\"id\":\"partial\",\"ts\":\"2026");
    std::fs::write(&path, text).unwrap();

    let reopened = SessionLog::open(path).unwrap();
    assert!(reopened.len() >= 3);
    assert!(
        reopened
            .warnings()
            .iter()
            .any(|w| w.contains("unparseable")),
        "the torn line must be reported: {:?}",
        reopened.warnings()
    );
}

#[test]
fn an_absent_link_is_omitted_rather_than_serialized_as_null() {
    let entry = Entry::new(None, EntryPayload::Header(header()));
    let json = serde_json::to_string(&entry).unwrap();
    assert!(
        !json.contains("prev_hash"),
        "an absent link must not be serialized as null: {json}"
    );
}
