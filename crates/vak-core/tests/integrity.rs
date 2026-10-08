//! Verifying the data home (plan M7a-i, docs/design/74 §6.2 A9): a whole
//! home is sound, an erased conversation still verifies, and one changed
//! byte in a stored record is found and named.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::erasure::Cause;
use vak_session::SessionLog;

#[tokio::test]
async fn a_changed_byte_is_found_and_an_erased_conversation_still_verifies() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let mut ledgers = Vec::new();
    let mut ids = Vec::new();
    for said in [
        "the marigold budget",
        "the zephyrine launch",
        "the walnut table",
    ] {
        let mut log = core.start_session().await.unwrap();
        ids.push(log.header().unwrap().session_id.clone());
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(said),
            meta: None,
        })
        .unwrap();
        ledgers.push(log.path().to_path_buf());
    }
    // One is erased: its key is gone and its bytes are not.
    let shared = core.shared_scope();
    vak_core::trash::set(&shared, std::slice::from_ref(&ids[1]), true).unwrap();
    core.erase_conversation(&ids[1], None, Cause::Person, None)
        .unwrap();
    core.catalog().unwrap().catch_up().unwrap();

    let whole = core.data_integrity();
    assert!(whole.sound(), "{whole:?}");
    assert_eq!(whole.conversations, 3, "the erased one is verified too");
    assert!(whole.chains >= 1 && whole.records >= 6, "{whole:?}");
    assert_eq!(whole.keys_destroyed, 1);
    assert_eq!((whole.receipts, whole.receipts_unverified), (1, 0));
    assert_eq!(whole.search, "current");
    assert!(!whole.fenced);

    // Flip one byte in the middle of a stored record.
    let segment = SessionLog::segment_files(&ledgers[2])[0].clone();
    let mut bytes = std::fs::read(&segment).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x01;
    std::fs::write(&segment, bytes).unwrap();

    let damaged = core.data_integrity();
    assert!(!damaged.sound());
    assert_eq!(damaged.broken.len(), 1, "{damaged:?}");
    assert!(damaged.broken[0].at.ends_with(&ids[2]), "{damaged:?}");
    assert_eq!(damaged.broken[0].segment, 1);
}
