//! Content in a shared chain dies with its conversation (plan M7a-b).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vak_session::chain::RecordChain;
use vak_session::content::{Restored, restore, seal_except, seal_fields};
use vak_session::objects::{TenantObjects, conversation_scope};

fn tenant() -> std::sync::Arc<TenantObjects> {
    TenantObjects::for_tenant(&vak_config::paths::local_tenant_home()).unwrap()
}

#[test]
fn a_shared_row_keeps_its_ids_and_loses_its_content_with_the_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let chain = RecordChain::at(dir.path().join("inbox"));
    for session in ["ses-content-kept", "ses-content-gone"] {
        let mut row = json!({
            "id": session,
            "kind": "task_summary",
            "title": "quarterly numbers",
            "body": "revenue rose in the north",
        });
        seal_fields(&mut row, session, &["title", "body"]).unwrap();
        assert!(row.get("title").is_none() && row.get("body").is_none());
        assert_eq!(row["id"], session);
        chain.append(&row).unwrap();
    }
    let stored = chain.text();
    assert!(!stored.contains("quarterly") && !stored.contains("revenue"));

    tenant()
        .destroy_scope_key(&conversation_scope("ses-content-gone"))
        .unwrap();

    let rows: Vec<serde_json::Value> = chain.read();
    assert_eq!(rows.len(), 2, "erasing removes no row");
    let mut kept = rows[0].clone();
    assert_eq!(restore(&mut kept).unwrap(), Restored::Whole);
    assert_eq!(kept["body"], "revenue rose in the north");
    assert!(kept.get("sealed").is_none());
    let mut gone = rows[1].clone();
    assert_eq!(restore(&mut gone).unwrap(), Restored::Erased);
    assert_eq!(gone["id"], "ses-content-gone");
    assert!(gone.get("body").is_none());
}

#[test]
fn everything_but_a_rows_identity_can_be_sealed() {
    let mut row = json!({
        "event_id": "e1",
        "kind": "opened",
        "spec": {"objective": "ship the report"},
        "note": "by friday",
    });
    seal_except(&mut row, "ses-content-except", &["event_id", "kind"]).unwrap();
    assert_eq!(
        row.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["event_id", "kind", "sealed"]
    );
    assert_eq!(restore(&mut row).unwrap(), Restored::Whole);
    assert_eq!(row["spec"]["objective"], "ship the report");
    assert_eq!(row["note"], "by friday");
}

#[test]
fn a_row_with_no_sealed_content_is_whole() {
    let mut row = json!({"id": "plain", "body": "about no conversation"});
    assert_eq!(restore(&mut row).unwrap(), Restored::Whole);
    assert_eq!(row["body"], "about no conversation");
}

#[test]
fn a_conversations_own_chain_is_sealed_and_goes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("executions");
    let chain = || RecordChain::at(&path).of_conversation("ses-content-chain");
    chain()
        .append(&json!({"line": "pip install pandas"}))
        .unwrap();
    chain().append(&json!({"line": "42 rows written"})).unwrap();

    for segment in vak_session::SessionLog::segment_files(&path) {
        let report = vak_storage::records::verify_chain(&segment).unwrap();
        assert_eq!(report.encrypted_frames, 2, "every frame is sealed");
        let bytes = std::fs::read(&segment).unwrap();
        assert!(!bytes.windows(6).any(|window| window == b"pandas"));
    }
    // A reader names no conversation: the chain's directory does.
    assert_eq!(RecordChain::at(&path).read::<serde_json::Value>().len(), 2);

    tenant()
        .destroy_scope_key(&conversation_scope("ses-content-chain"))
        .unwrap();
    assert!(
        RecordChain::at(&path)
            .read::<serde_json::Value>()
            .is_empty()
    );
    assert!(chain().append(&json!({"line": "more"})).is_err());
}
