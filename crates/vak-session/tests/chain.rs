#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The ledger is the product's evidence. These assert it is durable on write
//! and tamper-evident on read.

use std::path::PathBuf;

use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{ActivityKind, ActivityRecord, ActivityStatus, SessionLog};

fn header() -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-chain".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
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
        kind: ActivityKind::Diagnostic,
        status: ActivityStatus::Succeeded,
        label: label.into(),
        detail: None,
        data: Default::default(),
    }
}

fn ledger(dir: &std::path::Path, labels: &[&str]) -> PathBuf {
    let path = dir.join("s");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    for label in labels {
        log.append_activity(activity(label)).unwrap();
    }
    path
}

fn open_segment(path: &std::path::Path) -> PathBuf {
    path.join("seg-00000001.log")
}

/// The frame chain over the stored bytes is the ledger's integrity
/// (docs/design/73 §6): an untouched segment verifies with no key.
#[test]
fn every_appended_entry_links_to_its_predecessor() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger(dir.path(), &["a", "b", "c"]);
    let report = vak_storage::records::verify_chain(&open_segment(&path)).unwrap();
    assert_eq!(report.entries, 4, "header and three activities");
    assert!(report.torn_tail.is_none());
    let log = SessionLog::open_read_only(path).unwrap();
    assert!(log.warnings().is_empty(), "{:?}", log.warnings());
}

/// An edited byte breaks the chain; the ledger still opens, and says so.
#[test]
fn an_interior_edit_is_detected_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger(dir.path(), &["a", "b", "c"]);
    let segment = open_segment(&path);
    let mut bytes = std::fs::read(&segment).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&segment, bytes).unwrap();
    let log = SessionLog::open_read_only(path).unwrap();
    assert!(
        log.warnings()
            .iter()
            .any(|w| w.contains("failed verification")),
        "a rewritten entry must be reported: {:?}",
        log.warnings()
    );
}

/// A crash mid-append leaves a torn frame: readers keep what came before,
/// and the next writer cuts it off and appends cleanly after it.
#[test]
fn a_torn_tail_does_not_make_the_session_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let path = ledger(dir.path(), &["a", "b"]);
    let segment = open_segment(&path);
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&segment)
        .unwrap();
    std::io::Write::write_all(&mut file, &[9, 0, 0, 0, 1, 2]).unwrap();
    drop(file);
    assert_eq!(
        SessionLog::open_read_only(path.clone())
            .unwrap()
            .chain_to_root()
            .len(),
        3
    );
    let mut log = SessionLog::open(path.clone()).unwrap();
    log.append_activity(activity("after")).unwrap();
    drop(log);
    let reopened = SessionLog::open_read_only(path).unwrap();
    assert_eq!(reopened.chain_to_root().len(), 4);
    assert!(reopened.warnings().is_empty(), "{:?}", reopened.warnings());
}
