//! Erasing the whole install (plan M7b-f). It empties the data home, so
//! this test has a binary of its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::erasure::{Cause, ErasureError, Receipt};

fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found
}

#[tokio::test]
async fn install_erasure_leaves_a_receipt_and_nothing_else() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("orchard");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.md"), "the owner's notes").unwrap();
    let shared = vak_config::paths::default_workspace();
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("kept.md"), "the owner's own file").unwrap();

    let core = Core::new(folder.clone()).unwrap();
    let mut held = String::new();
    for said in ["plan the orchard walk", "prune the pear trees"] {
        let mut log = core.start_session().await.unwrap();
        held = log.header().unwrap().session_id.clone();
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(said),
            meta: None,
        })
        .unwrap();
    }
    let secrets = vak_config::user_env_path().unwrap();
    vak_config::credentials::set(&secrets, "ORCHARD_SERVICE_KEY", "s3cret").unwrap();
    let data = vak_config::paths::data_home();
    assert!(files_under(&data).len() > 3, "the home holds real state");

    let preview = core.install_erasure_preview().unwrap();
    assert_eq!(preview.conversations, 2);
    assert!(preview.keys >= 2);

    // Anything on hold refuses it, and nothing is removed.
    core.hold_conversation(&held, true).unwrap();
    assert!(matches!(
        core.erase_install(None, Cause::Person, None),
        Err(ErasureError::Held)
    ));
    core.hold_conversation(&held, false).unwrap();
    assert!(matches!(
        core.erase_install(Some("not the preview"), Cause::Person, None),
        Err(ErasureError::StalePreview)
    ));
    assert_eq!(core.install_erasure_preview().unwrap(), preview);

    let receipt = core
        .erase_install(Some(&preview.digest), Cause::Person, None)
        .unwrap();
    assert_eq!(receipt.scope, "install");
    assert_eq!(receipt.conversations, 2);
    assert_eq!(receipt.keys_destroyed, preview.keys);
    assert!(receipt.verifies());

    // The receipt is all that is left of what Vakyartha stored.
    // (A test's home holds the owner's own folder too; it is checked below.)
    let left: Vec<_> = files_under(&data)
        .into_iter()
        .filter(|file| !file.starts_with(&shared))
        .collect();
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(left[0].ends_with(format!("erased/{}.json", receipt.id)));
    let kept: Receipt = serde_json::from_slice(&std::fs::read(&left[0]).unwrap()).unwrap();
    assert_eq!(kept, receipt);
    assert!(kept.verifies(), "it checks with the key it carries");
    for root in [
        vak_config::paths::cache_home(),
        vak_config::paths::logs_dir(),
        vak_config::paths::runtime_dir(),
    ] {
        if !data.starts_with(&root) {
            assert!(
                files_under(&root)
                    .iter()
                    .all(|file| file.starts_with(&data))
            );
        }
    }
    assert_eq!(
        vak_config::credentials::get(&secrets, "ORCHARD_SERVICE_KEY"),
        None,
        "a stored secret is gone"
    );

    // What a person owns is untouched.
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.md")).unwrap(),
        "the owner's notes"
    );
    assert_eq!(
        std::fs::read_to_string(shared.join("kept.md")).unwrap(),
        "the owner's own file"
    );

    // A fresh start reads the home as new, and still shows the receipt.
    assert!(vak_core::baseline::check_data_home(&data).is_ok());
    assert_eq!(core.erasure_receipts(), vec![receipt]);
}
