//! The automatic push (data-architecture plan M9-e): a remote that
//! cannot be reached loses nothing, and the push goes through when it is
//! back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::sync::{Role, SyncError};

async fn say(core: &Core, said: &str) {
    let mut log = core.start_session().await.unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text(said),
        meta: None,
    })
    .unwrap();
}

#[tokio::test]
async fn sync_survives_network_loss() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let (work, remote, away) = (
        dir.path().join("work"),
        dir.path().join("remote"),
        dir.path().join("unplugged"),
    );
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    let core = Core::new(work).unwrap();

    // Nothing is pushed by itself before a remote is set up and first
    // pushed by hand.
    assert!(core.sync_auto().is_none());
    core.sync_setup(&remote).unwrap();
    assert!(core.sync_auto().is_none());
    say(&core, "the marigold budget").await;
    let first = core.sync_push().unwrap();
    assert_eq!(core.sync_status().unwrap().role, Role::Holder);
    assert!(core.sync_auto().is_none(), "nothing changed");

    // Work goes on while the folder is gone.
    std::fs::rename(&remote, &away).unwrap();
    say(&core, "the zephyrine plan").await;
    assert!(matches!(
        core.sync_auto(),
        Some(Err(SyncError::Unreachable))
    ));
    let status = core.sync_status().unwrap();
    assert!(!status.reachable && status.unpushed > 0);
    assert!(status.last_error.is_some());
    // The next try waits; it is not hammered every tick.
    assert!(core.sync_auto().is_none());
    say(&core, "the heliotrope notes").await;
    assert!(matches!(core.sync_push(), Err(SyncError::Unreachable)));

    // The folder is back: the push that was owed goes through, with
    // everything written meanwhile.
    std::fs::rename(&away, &remote).unwrap();
    let local = core.shared_scope().sync().join("local.json");
    let mut note: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&local).unwrap()).unwrap();
    note["tried_at"] = serde_json::json!("2020-01-01T00:00:00Z");
    std::fs::write(&local, serde_json::to_vec(&note).unwrap()).unwrap();
    let pushed = core.sync_auto().unwrap().unwrap();
    assert_eq!(pushed.generation, first.generation + 1);
    assert!(pushed.copied > 0);
    let status = core.sync_status().unwrap();
    assert_eq!((status.unpushed, status.last_error), (0, None));
    assert_eq!(status.remote_generation, Some(pushed.generation));
    assert!(core.sync_auto().is_none());
    assert!(core.data_integrity().broken.is_empty());
}
