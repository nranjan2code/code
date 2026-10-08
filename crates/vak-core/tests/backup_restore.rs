//! Restoring a backup taken before an erasure (plan M7a-h, docs/design/74
//! §4.8): the backup still holds what was erased and the key that read
//! it; the restore erases it again before it ends, and fences every
//! process that had the store open.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::backup::Conflict;
use vak_core::erasure::Cause;
use vak_session::SessionLog;
use vak_session::objects::{TenantObjects, conversation_scope};

async fn conversation(core: &Core, said: &str) -> (String, std::path::PathBuf) {
    let mut log = core.start_session().await.unwrap();
    let id = log.header().unwrap().session_id.clone();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text(said),
        meta: None,
    })
    .unwrap();
    (id, log.path().to_path_buf())
}

fn indexed(core: &Core, needle: &str) -> bool {
    let catalog = core.catalog().unwrap();
    catalog.catch_up().unwrap();
    catalog
        .dump()
        .unwrap()
        .iter()
        .any(|row| row.contains(needle))
}

#[tokio::test]
async fn restore_reapplies_erasures() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let shared = core.shared_scope();
    let (gone, gone_ledger) = conversation(&core, "the zephyrine launch date").await;
    let (_, kept_ledger) = conversation(&core, "the marigold budget").await;
    assert!(indexed(&core, "zephyrine"));
    let tenant = TenantObjects::for_tenant(&vak_config::paths::local_tenant_home()).unwrap();
    let has_key = |tenant: &TenantObjects| {
        !tenant
            .scopes_with_prefix(&conversation_scope(&gone))
            .unwrap()
            .is_empty()
    };

    // The backup: taken while the conversation is whole, key included.
    let backup = tempfile::tempdir().unwrap();
    let manifest = core.backup_create(backup.path(), false).unwrap();
    assert_eq!(manifest.version, 2);
    assert!(manifest.erasures.is_empty() && manifest.scope_keys >= 2);
    let epoch = manifest.store_epoch.expect("the manifest says the epoch");
    assert!(manifest.ref_generation.is_some());
    assert!(has_key(&tenant));

    // Then it is erased.
    vak_core::trash::set(&shared, std::slice::from_ref(&gone), true).unwrap();
    let receipt = core
        .erase_conversation(&gone, None, Cause::Person, None)
        .unwrap();
    assert!(!has_key(&tenant) && !indexed(&core, "zephyrine"));

    // The preview names the erasure the backup predates.
    let preview = core.restore_preview(backup.path()).unwrap();
    assert_eq!(
        preview.erasures_to_reapply,
        std::slice::from_ref(&receipt.id)
    );
    assert!(
        core.restore_preview(work.path()).is_err(),
        "a folder with no manifest is not a backup"
    );

    let report = core.backup_restore(backup.path(), Conflict::Skip).unwrap();
    assert_eq!(report.erasures_reapplied, 1);
    assert!(
        report.keys_removed >= 1,
        "the backup brought the key back, and it was destroyed again: {report:?}"
    );
    assert!(report.writer_epoch > epoch, "the writer epoch moved");

    // The erased conversation is still erased; the other is whole.
    assert!(!has_key(&tenant), "no key for it is on disk");
    assert!(tenant.scope_destroyed(&conversation_scope(&gone)));
    assert!(SessionLog::open_read_only(gone_ledger).is_err());
    assert!(vak_core::trash::is_trashed(&shared, &gone));
    assert!(
        vak_core::trash::state(&shared, &gone)
            .unwrap()
            .erased_at
            .is_some()
    );
    assert!(!indexed(&core, "zephyrine") && indexed(&core, "marigold"));
    let kept = SessionLog::open_read_only(kept_ledger).unwrap();
    assert!(
        kept.derive_transcript()
            .iter()
            .any(|message| message.message.text_content().contains("marigold"))
    );
    assert_eq!(core.erasure_receipts(), [receipt]);

    // Every process that had the store open is fenced, this one included.
    assert!(vak_session::fence::is_fenced());
    assert!(core.backup_restore(backup.path(), Conflict::Skip).is_err());
}
