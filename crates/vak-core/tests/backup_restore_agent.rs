//! Restoring a backup taken before an Agent's data was erased: the
//! Agent's conversations come back unreadable, because their keys are
//! destroyed again, and the lists say they are erased, not present. A
//! restore fences the process, so this has a test binary of its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::backup::Conflict;
use vak_core::erasure::Cause;
use vak_session::SessionLog;

fn definitions(work: &std::path::Path, scout: &str) {
    let file = vak_core::agent_definitions::path(work);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        file,
        serde_json::json!([{
            "id": "scout", "revision": 1, "lifecycle": scout, "name": "scout",
            "character": "vak", "personality": "Calm.", "behaviour": "Brief.",
            "animation": "off", "voice": "default"
        }])
        .to_string(),
    )
    .unwrap();
}

#[tokio::test]
async fn a_restore_lists_an_erased_agents_conversations_as_erased() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    definitions(work.path(), "active");
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let shared = core.shared_scope();

    let mut log = core.start_session().await.unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("the marigold budget"),
        meta: None,
    })
    .unwrap();
    let own_id = log.header().unwrap().session_id.clone();
    let mut header = SessionLog::read_header(log.path()).unwrap();
    drop(log);
    header.session_id = format!("{own_id}-scout");
    let scout_id = header.session_id.clone();
    let ledger = vak_config::scope::AgentScope::new(shared.agents_dir().join("scout"))
        .session_file(core.cwd(), &scout_id);
    std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
    let mut scout = SessionLog::create(ledger, header).unwrap();
    scout
        .append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text("the zephyrine plan"),
            meta: None,
        })
        .unwrap();
    drop(scout);

    let backup = tempfile::tempdir().unwrap();
    core.backup_create(backup.path(), false).unwrap();

    definitions(work.path(), "archived");
    let preview = core.agent_erasure_preview("scout").unwrap();
    assert_eq!(preview.conversations, 1);
    core.erase_agent("scout", Some(&preview.digest), Cause::Person, None)
        .unwrap();
    assert!(
        vak_core::trash::state(&shared, &scout_id)
            .unwrap()
            .erased_at
            .is_some()
    );
    core.backup_restore(backup.path(), Conflict::Skip).unwrap();
    let state = vak_core::trash::state(&shared, &scout_id).unwrap();
    assert!(state.erased_at.is_some(), "{state:?}");
    assert!(vak_core::trash::is_trashed(&shared, &scout_id));
    assert!(!vak_core::trash::is_trashed(&shared, &own_id));
}
