//! Rotating the tenant's key (plan M7b-g): everything stays readable,
//! what was erased stays erased, and nothing is left under the old key.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::artifacts::{ArtifactKind, ArtifactStep, NewVersion, VersionSource};
use vak_core::erasure::Cause;
use vak_core::{Core, trash};
use vak_session::SessionLog;

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

fn says(ledger: &std::path::Path, needle: &str) -> bool {
    SessionLog::open_read_only(ledger.to_path_buf()).is_ok_and(|log| {
        log.derive_transcript()
            .iter()
            .any(|message| message.message.text_content().contains(needle))
    })
}

#[tokio::test]
async fn rotation_keeps_everything_readable() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(dir.path().to_path_buf()).unwrap();
    let space = vak_config::spaces::bind(dir.path()).unwrap();
    let (_, kept_ledger) = conversation(&core, "the marigold budget").await;
    let (gone, gone_ledger) = conversation(&core, "the zephyrine plan").await;

    let artifacts = core.artifacts();
    let file = artifacts
        .declare(
            &space,
            "vak",
            "report.md",
            ArtifactKind::File,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let version = artifacts
        .version(
            file,
            NewVersion {
                parent: None,
                bytes: b"the saffron report",
                source: VersionSource::Person,
            },
            None,
            None,
        )
        .unwrap();
    artifacts
        .record(file, ArtifactStep::Saved { version }, None, None)
        .unwrap();

    trash::set(&core.shared_scope(), std::slice::from_ref(&gone), true).unwrap();
    core.erase_conversation(&gone, None, Cause::Person, None)
        .unwrap();
    assert!(!says(&gone_ledger, "zephyrine"));

    let before = core.key_status().unwrap();
    assert_eq!((before.version, before.oldest_in_use), (0, 0));
    assert!(before.rotations.is_empty());
    assert_eq!(before.kept_in, "encrypted_file");

    let rotation = core.rotate_keys(None).unwrap();
    assert_eq!(rotation.version, 1);
    assert!(rotation.rewrapped >= 2, "{rotation:?}");

    // Nothing is left under the old key, and everything still reads.
    let after = core.key_status().unwrap();
    assert_eq!((after.version, after.oldest_in_use), (1, 1));
    assert_eq!(
        (after.keys, after.destroyed),
        (before.keys, before.destroyed)
    );
    assert_eq!(after.rotations, vec![rotation]);
    assert!(says(&kept_ledger, "marigold"));
    let read = artifacts.get(&file.to_string()).unwrap();
    assert_eq!(
        artifacts.bytes(&read, &version).unwrap(),
        b"the saffron report"
    );

    // What was erased stays erased.
    assert!(!says(&gone_ledger, "zephyrine"));

    // What is written afterwards reads, and a second rotation moves on.
    let (_, new_ledger) = conversation(&core, "the heliotrope notes").await;
    assert_eq!(core.rotate_keys(None).unwrap().version, 2);
    assert!(says(&new_ledger, "heliotrope") && says(&kept_ledger, "marigold"));
    let status = core.key_status().unwrap();
    assert_eq!((status.version, status.oldest_in_use), (2, 2));
    assert_eq!(status.rotations.len(), 2);

    // The earlier keys are retired: everything still reads, the key file
    // carries a marker in their place, and there is nothing more to retire.
    let retired = core.retire_keys(None).unwrap();
    assert_eq!((retired.version, retired.retired), (2, 2));
    assert!(says(&new_ledger, "heliotrope") && says(&kept_ledger, "marigold"));
    let read = artifacts.get(&file.to_string()).unwrap();
    assert_eq!(
        artifacts.bytes(&read, &version).unwrap(),
        b"the saffron report"
    );
    assert!(!says(&gone_ledger, "zephyrine"));
    let material =
        vak_session::objects::TenantObjects::key_material(&vak_config::paths::local_tenant_home())
            .unwrap();
    assert_eq!(material.keks.len(), 3);
    assert!(
        material.keks[..2]
            .iter()
            .all(|kek| kek == vak_storage::keys::RETIRED)
    );
    assert_ne!(material.keks[2], vak_storage::keys::RETIRED);
    assert!(core.retire_keys(None).is_err());
    let status = core.key_status().unwrap();
    assert_eq!(
        (status.version, status.oldest_in_use, status.retired),
        (2, 2, 2)
    );
    let (_, last_ledger) = conversation(&core, "the juniper list").await;
    assert!(says(&last_ledger, "juniper"));

    let integrity = core.data_integrity();
    assert!(integrity.broken.is_empty(), "{integrity:?}");
}
