//! `vak sessions` lists the conversations of the folder it is run in. It
//! looked for `.jsonl` files after ledgers became segment directories and
//! listed nothing (found by the Mac-with-Linux sync lab, 2026-10-09).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

#[tokio::test]
async fn sessions_lists_this_folders_conversations_and_not_trashed_ones() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let core = vak_core::Core::new(work.clone()).unwrap();
    let mut ids = Vec::new();
    for said in ["the marigold budget", "the zephyrine picnic"] {
        let mut log = core.start_session().await.unwrap();
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(said),
            meta: None,
        })
        .unwrap();
        ids.push(log.header().unwrap().session_id.clone());
    }
    vak_core::trash::set(&core.shared_scope(), &ids[1..], true).unwrap();

    let home = vak_config::paths::data_home();
    let output = Command::new(env!("CARGO_BIN_EXE_vak"))
        .current_dir(&work)
        .arg("sessions")
        .env("VAK_HOME", &home)
        .env("HOME", home.join("user-home"))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{listed}");
    assert!(listed.contains(&ids[0]), "{listed}");
    assert!(
        !listed.contains(&ids[1]),
        "a trashed one is not listed: {listed}"
    );
    assert!(!listed.contains("no sessions yet"), "{listed}");
}
