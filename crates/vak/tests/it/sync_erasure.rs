//! An erasure made on one machine reaches the other and cannot come back
//! (data-architecture plan M9-d). Machine A is this test process's home;
//! B and C are homes only the `vak` binary touches.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Output};

const PASSPHRASE: &str = "four quiet lanterns";

fn vak(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vak"))
        .current_dir(cwd)
        .args(args)
        .env("VAK_HOME", home)
        .env("HOME", home.join("user-home"))
        .env("VAK_KEY_PASSPHRASE", PASSPHRASE)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

fn ok(output: Output) -> String {
    let (out, err) = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    );
    assert!(output.status.success(), "stdout: {out}\nstderr: {err}");
    out
}

fn all(output: Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[tokio::test]
async fn erasure_propagates_and_cannot_resurrect() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let (work, remote) = (dir.path().join("work"), dir.path().join("remote"));
    let (home_b, home_c) = (dir.path().join("b"), dir.path().join("c"));
    for folder in [&work, &remote, &home_b, &home_c] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let home_a = vak_config::paths::data_home();
    let folder = remote.to_string_lossy().into_owned();
    let key = dir.path().join("key.txt").to_string_lossy().into_owned();

    let core = vak_core::Core::new(work.clone()).unwrap();
    let mut ids = Vec::new();
    for said in ["the marigold budget", "the zephyrine plan"] {
        let mut log = core.start_session().await.unwrap();
        ids.push(log.header().unwrap().session_id.clone());
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(said),
            meta: None,
        })
        .unwrap();
    }
    let (kept, gone) = (ids[0].clone(), ids[1].clone());

    // A pushes; B and C each take the keys and pull. All three read it.
    ok(vak(&home_a, &work, &["sync", "setup", &folder]));
    ok(vak(&home_a, &work, &["sync", "now"]));
    ok(vak(&home_a, &work, &["sync", "key", "export", &key]));
    for home in [&home_b, &home_c] {
        ok(vak(home, &work, &["sync", "key", "import", &key]));
        ok(vak(home, &work, &["sync", "setup", &folder]));
        ok(vak(home, &work, &["sync", "pull"]));
        assert!(ok(vak(home, &work, &["data", "cat", &gone])).contains("zephyrine"));
    }

    // A erases one conversation, puts the other on hold, and pushes.
    vak_core::trash::set(&core.shared_scope(), std::slice::from_ref(&gone), true).unwrap();
    let receipt = core
        .erase_conversation(&gone, None, vak_core::erasure::Cause::Person, None)
        .unwrap();
    core.hold_conversation(&kept, true).unwrap();
    ok(vak(&home_a, &work, &["sync", "now"]));

    // The remote no longer holds the erased conversation's key.
    // B pulls: the erasure, the receipt and the hold are all there.
    ok(vak(&home_b, &work, &["sync", "pull"]));
    assert!(!all(vak(&home_b, &work, &["data", "cat", &gone])).contains("zephyrine"));
    assert!(ok(vak(&home_b, &work, &["data", "cat", &kept])).contains("marigold"));
    let receipts = ok(vak(&home_b, &work, &["data", "receipts"]));
    assert!(
        receipts.contains(&receipt.id) && receipts.contains("signature ok"),
        "{receipts}"
    );
    let verified = ok(vak(&home_b, &work, &["data", "verify"]));
    assert!(verified.contains("1 destroyed, 1 on hold"), "{verified}");

    // C still holds the copy from before the erasure, key and all. It
    // cannot push that copy back: it is standing by.
    assert!(ok(vak(&home_c, &work, &["data", "cat", &gone])).contains("zephyrine"));
    assert!(all(vak(&home_c, &work, &["sync", "now"])).contains("standing by"));
    // Taking over, even by force, brings the erasure first.
    ok(vak(&home_c, &work, &["sync", "takeover", "--force"]));
    assert!(!all(vak(&home_c, &work, &["data", "cat", &gone])).contains("zephyrine"));
    ok(vak(&home_c, &work, &["sync", "now"]));

    // Whoever pulls from now on gets it erased, the hold kept.
    let home_d = dir.path().join("d");
    std::fs::create_dir_all(&home_d).unwrap();
    ok(vak(&home_d, &work, &["sync", "key", "import", &key]));
    ok(vak(&home_d, &work, &["sync", "setup", &folder]));
    ok(vak(&home_d, &work, &["sync", "pull"]));
    assert!(!all(vak(&home_d, &work, &["data", "cat", &gone])).contains("zephyrine"));
    assert!(ok(vak(&home_d, &work, &["data", "cat", &kept])).contains("marigold"));
    assert!(ok(vak(&home_d, &work, &["data", "verify"])).contains("1 destroyed, 1 on hold"));

    // No file in the remote holds the erased conversation's key.
    let scope = format!("conversation:{gone}");
    let hex: String = scope.bytes().map(|b| format!("{b:02x}")).collect();
    let index = std::fs::read_to_string(remote.join("index.json")).unwrap();
    assert!(
        !index.contains(&format!("keys/{hex}")),
        "the key is still named"
    );
    assert!(index.contains(&format!("tombstones/{hex}")));
}
