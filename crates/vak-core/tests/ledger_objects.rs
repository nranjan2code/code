//! A ledger keeps a large tool result as a tenant object and a reference:
//! the body is not in the ledger's bytes, reads back after reopening, and
//! lives under the tenant's declared objects root (plan M3b slice 2).
#![allow(clippy::unwrap_used, clippy::panic)]

use vak_core::Core;

#[tokio::test]
async fn evidence_body_is_a_tenant_object_not_ledger_bytes() {
    let home = vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    let body = "evidence line that should live outside the ledger\n".repeat(2_000);

    let mut session = core.start_session().await.unwrap();
    let session_id = session.header().unwrap().session_id.clone();
    session
        .append_evidence_body("call-1", body.clone())
        .unwrap();
    let ledger = session.path().to_path_buf();
    drop(session);

    assert!(!vak_session::SessionLog::text(&ledger).contains("should live outside"));
    let reopened = core.open_session(&session_id).await.unwrap();
    let vak_session::EntryPayload::EvidenceBody(record) =
        &reopened.chain_to_root().last().unwrap().payload
    else {
        panic!("the last entry is the evidence body");
    };
    assert_eq!(
        reopened.object_text(&record.body).as_deref(),
        Some(body.as_str())
    );

    let objects =
        vak_config::paths::tenant_home_at(&home, &vak_session::trace::local::tenant().to_string())
            .join("objects");
    assert!(
        walk_files(&objects) > 0,
        "the body is stored under {objects:?}"
    );
}

fn walk_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            if entry.path().is_dir() {
                walk_files(&entry.path())
            } else {
                1
            }
        })
        .sum()
}
