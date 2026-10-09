//! A take-over that succeeds clears the failure the machine last saw:
//! found in the browser on 2026-10-09, the admin's Second copy still said
//! "It did not go through: another machine took the work over" beside
//! "This machine holds the work now". A take-over that pulls fences this
//! process, so it has a test binary of its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::sync::{Role, SyncError};

#[tokio::test]
async fn a_successful_takeover_forgets_the_last_failure() {
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
    core.sync_setup(&remote).unwrap();
    let mut log = core.start_session().await.unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("the marigold budget"),
        meta: None,
    })
    .unwrap();
    core.sync_push().unwrap();

    // A push fails and is remembered.
    std::fs::rename(&remote, &away).unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("the zephyrine plan"),
        meta: None,
    })
    .unwrap();
    assert!(matches!(
        core.sync_auto(),
        Some(Err(SyncError::Unreachable))
    ));
    assert!(core.sync_status().unwrap().last_error.is_some());
    std::fs::rename(&away, &remote).unwrap();

    // The machine learns the work was taken from it, then takes it back.
    let local = core.shared_scope().sync().join("local.json");
    let mut note: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&local).unwrap()).unwrap();
    note["role"] = serde_json::to_value(Role::Lost).unwrap();
    std::fs::write(&local, serde_json::to_vec(&note).unwrap()).unwrap();
    core.sync_takeover(true, true).unwrap();

    let status = core.sync_status().unwrap();
    assert_eq!(status.role, Role::Holder);
    assert_eq!(status.last_error, None);
}
