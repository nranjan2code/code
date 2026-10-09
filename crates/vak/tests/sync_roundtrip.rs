//! The folder remote between two machines (data-architecture plan M9-a,
//! M9-b). Machine A is this test process's own home; machine B is a second
//! home that only the `vak` binary touches, as a second machine would be.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Output};

/// Runs `vak` as the machine whose data home is `home`. `HOME` is a scratch
/// folder, so no OS keychain is reached and secrets stay in the home.
fn vak(home: &Path, cwd: &Path, args: &[&str], passphrase: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vak"));
    command
        .current_dir(cwd)
        .args(args)
        .env("VAK_HOME", home)
        .env("HOME", home.join("user-home"))
        .env_remove("VAK_KEY_PASSPHRASE")
        .stdin(std::process::Stdio::null());
    if let Some(passphrase) = passphrase {
        command.env("VAK_KEY_PASSPHRASE", passphrase);
    }
    command.output().unwrap()
}

fn ok(output: &Output) -> String {
    let (out, err) = (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    );
    assert!(output.status.success(), "stdout: {out}\nstderr: {err}");
    out
}

fn refused(output: &Output) -> String {
    assert!(!output.status.success(), "it should have been refused");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

const PASSPHRASE: &str = "four quiet lanterns";

#[tokio::test]
async fn push_pull_roundtrip_identical_derive_messages() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let (work, remote, home_b) = (
        dir.path().join("work"),
        dir.path().join("remote"),
        dir.path().join("machine-b"),
    );
    for folder in [&work, &remote, &home_b] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let home_a = vak_config::paths::data_home();

    // Machine A holds two conversations.
    let core = vak_core::Core::new(work.clone()).unwrap();
    let mut ids = Vec::new();
    for said in ["the marigold budget is due Monday", "the zephyrine picnic"] {
        let mut log = core.start_session().await.unwrap();
        ids.push(log.header().unwrap().session_id.clone());
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(said),
            meta: None,
        })
        .unwrap();
    }
    vak_config::credentials::set(
        &vak_config::user_env_path().unwrap(),
        "A_SERVICE_KEY",
        "never-leaves-machine-a",
    )
    .unwrap();
    drop(core);

    let folder = remote.to_string_lossy().into_owned();
    let a = |args: &[&str], passphrase: Option<&str>| vak(&home_a, &work, args, passphrase);
    let b = |args: &[&str], passphrase: Option<&str>| vak(&home_b, &work, args, passphrase);

    // Nothing is set up yet; then A pushes.
    assert!(refused(&a(&["sync", "now"], None)).contains("vak sync setup"));
    ok(&a(&["sync", "setup", &folder], None));
    let pushed = ok(&a(&["sync", "now"], None));
    assert!(pushed.contains("Pushed"), "{pushed}");
    let said_on_a = ok(&a(&["data", "cat", &ids[0]], None));
    assert!(said_on_a.contains("marigold"), "{said_on_a}");

    // Nothing readable and no secret is in the remote folder.
    let mut in_remote = Vec::new();
    let mut stack = vec![remote.clone()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(next).unwrap().flatten() {
            if entry.path().is_dir() {
                stack.push(entry.path());
            } else {
                in_remote.push(entry.path());
            }
        }
    }
    assert!(in_remote.len() > 5);
    for file in &in_remote {
        let bytes = std::fs::read(file).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("marigold") && !text.contains("never-leaves"),
            "{}",
            file.display()
        );
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            !name.contains("credential") && !name.starts_with("refs.db"),
            "{name}"
        );
    }

    // A second push with nothing changed copies nothing.
    assert!(ok(&a(&["sync", "now"], None)).contains("0 files copied"));

    // Machine B without the keys: it pulls the bytes and reads nothing.
    ok(&b(&["sync", "setup", &folder], None));
    ok(&b(&["sync", "pull"], None));
    let unread = b(&["data", "cat", &ids[0]], None);
    assert!(
        !String::from_utf8_lossy(&unread.stdout).contains("marigold"),
        "a machine without the key file reads nothing"
    );

    // A fresh machine B takes the key file first, then pulls.
    std::fs::remove_dir_all(&home_b).unwrap();
    std::fs::create_dir_all(&home_b).unwrap();
    let key_file = dir.path().join("vakyartha.key");
    let key_path = key_file.to_string_lossy().into_owned();
    assert!(
        refused(&a(&["sync", "key", "export", &key_path], Some("short"))).contains("at least 12")
    );
    ok(&a(&["sync", "key", "export", &key_path], Some(PASSPHRASE)));
    let sealed = std::fs::read_to_string(&key_file).unwrap();
    assert!(sealed.starts_with("vakyartha-key-file-1"));
    assert!(
        refused(&b(
            &["sync", "key", "import", &key_path],
            Some("not the passphrase")
        ))
        .contains("passphrase is wrong")
    );
    ok(&b(&["sync", "key", "import", &key_path], Some(PASSPHRASE)));
    ok(&b(&["sync", "setup", &folder], None));
    let pulled = ok(&b(&["sync", "pull"], None));
    assert!(pulled.contains("Start Vakyartha again"), "{pulled}");

    // Both conversations read on B exactly as on A.
    for id in &ids {
        let on_a = ok(&a(&["data", "cat", id], None));
        let on_b = ok(&b(&["data", "cat", id], None));
        assert_eq!(on_a, on_b);
        assert!(!on_b.trim().is_empty());
    }
    // B is intact, and it does not hold A's secret.
    let verified = ok(&b(&["data", "verify"], None));
    assert!(!verified.to_lowercase().contains("damaged:"), "{verified}");
    assert!(
        !home_b.join("credential_index.json").exists()
            || !std::fs::read_to_string(home_b.join("credential_index.json"))
                .unwrap()
                .contains("A_SERVICE_KEY")
    );

    // A machine that already holds data under its own keys takes no key file.
    assert!(
        refused(&a(&["sync", "key", "import", &key_path], Some(PASSPHRASE)))
            .contains("new install")
    );

    // A push that stopped before its index was replaced changes nothing
    // for a puller: blobs it added are extra, and the old index is whole.
    std::fs::create_dir_all(remote.join("blobs/zz")).unwrap();
    std::fs::write(remote.join("blobs/zz/unfinished.part"), "half a file").unwrap();
    let home_d = dir.path().join("machine-d");
    std::fs::create_dir_all(&home_d).unwrap();
    let d = |args: &[&str], passphrase: Option<&str>| vak(&home_d, &work, args, passphrase);
    ok(&d(&["sync", "key", "import", &key_path], Some(PASSPHRASE)));
    ok(&d(&["sync", "setup", &folder], None));
    ok(&d(&["sync", "pull"], None));
    assert_eq!(
        ok(&d(&["data", "cat", &ids[1]], None)),
        ok(&a(&["data", "cat", &ids[1]], None))
    );
    // The next push clears what the unfinished one left.
    ok(&a(&["sync", "now"], None));
    assert!(!remote.join("blobs/zz/unfinished.part").exists());

    // A damaged remote is refused whole: nothing on the puller changes.
    let victim = in_remote
        .iter()
        .find(|file| file.to_string_lossy().contains("/blobs/"))
        .unwrap();
    let mut bytes = std::fs::read(victim).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(victim, bytes).unwrap();
    let home_c = dir.path().join("machine-c");
    std::fs::create_dir_all(&home_c).unwrap();
    let c = |args: &[&str], passphrase: Option<&str>| vak(&home_c, &work, args, passphrase);
    ok(&c(&["sync", "key", "import", &key_path], Some(PASSPHRASE)));
    ok(&c(&["sync", "setup", &folder], None));
    assert!(refused(&c(&["sync", "pull"], None)).contains("do not match its index"));
    assert!(!home_c.join("agents").exists(), "nothing was written");
}
