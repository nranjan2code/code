// A test's output is for the person running it.
#![allow(clippy::disallowed_macros)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `--purge` leaves nothing of ours, and nothing of yours
//! (`docs/design/46-stabilization-install-and-onboarding.md` D3, S9).
//!
//! The assertion that matters is **what survived**, not what was removed.
//! A delete-list can be tested by checking the list ran; a preserve rule
//! can only be tested by looking at the filesystem afterwards and finding
//! the user's own files still there.

use std::path::{Path, PathBuf};
use std::process::Command;

fn vak_binary() -> PathBuf {
    // The integration binary sits beside the test binary's target dir.
    let mut path = std::env::current_exe().expect("test exe");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("vak")
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

/// A purge removes every declared root and leaves a project's own `.vak`
/// alone, because that is the user's file in the user's repository.
#[test]
fn purge_removes_declared_state_and_preserves_the_users_own_projects() {
    let binary = vak_binary();
    if !binary.exists() {
        // `cargo test` without a prior build of the binary target. Skip
        // rather than fail on something that is not this test's subject.
        eprintln!("skipping: no vak binary at {}", binary.display());
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    // State the registry declares, across both roots.
    let data = home.path();
    write(&data.join("sessions/w/a.jsonl"), "{\"kind\":\"header\"}\n");
    write(
        &data.join("agents/writer/sessions/w/b.jsonl"),
        "{\"kind\":\"header\"}\n",
    );
    write(&data.join("agents/writer/memory/user/USER.md"), "note\n");
    write(&data.join("security-events.jsonl"), "{}\n");
    write(
        &data.join("gateway/bots.json"),
        "{\"schema\":1,\"bots\":[]}",
    );
    let shared = data.join("vak-home");
    write(&shared.join(".env"), "SECRET=value\n");
    write(&shared.join(".vak/config.toml"), "provider = \"ollama\"\n");
    write(
        &shared.join(".vak/skills/demo/SKILL.md"),
        "---\nname: demo\n---\n",
    );

    // The user's own project, which a purge must never touch.
    let project_config = project.path().join(".vak/config.toml");
    write(&project_config, "model = \"theirs\"\n");
    let project_source = project.path().join("src/main.rs");
    write(&project_source, "fn main() {}\n");

    let output = Command::new(&binary)
        .args(["self", "uninstall", "--yes", "--purge", "--prefix"])
        .arg(home.path().join("prefix"))
        .env("VAK_HOME", home.path())
        .output()
        .expect("run uninstall");
    assert!(
        output.status.success(),
        "purge failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Ours is gone.
    for gone in [
        data.join("sessions"),
        // Every declared subpath of an Agent home, and then the emptied
        // homes themselves.
        data.join("agents"),
        data.join("security-events.jsonl"),
        data.join("gateway"),
        shared.join(".env"),
        shared.join(".vak/config.toml"),
        shared.join(".vak/skills"),
    ] {
        assert!(
            !gone.exists(),
            "{} survived a purge; the next install would not be a first run",
            gone.display()
        );
    }

    // Theirs is not.
    assert!(
        project_config.exists(),
        "a purge deleted a project's own .vak — that is the user's file in the user's repository"
    );
    assert!(project_source.exists(), "a purge touched project source");
}

/// Logs are Vak's state too: a purge that left them made the next install
/// read a previous version's service logs as its own.
#[test]
fn purge_includes_logs() {
    let binary = vak_binary();
    if !binary.exists() {
        eprintln!("skipping: no vak binary at {}", binary.display());
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let log = home.path().join("logs/gateway.log");
    write(&log, "old service output\n");
    let output = Command::new(&binary)
        .args(["self", "uninstall", "--yes", "--purge", "--prefix"])
        .arg(home.path().join("prefix"))
        .env("VAK_HOME", home.path())
        .output()
        .expect("run uninstall");
    assert!(
        output.status.success(),
        "purge failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!log.exists(), "a purge left the logs behind");
}

/// A symlinked root is refused rather than followed.
///
/// `remove_dir_all` through a symlink deletes whatever it points at, which
/// on a machine where someone has redirected their vak home is somebody
/// else's directory entirely.
#[test]
fn a_symlinked_root_is_refused_not_followed() {
    let binary = vak_binary();
    if !binary.exists() {
        eprintln!("skipping: no vak binary");
        return;
    }
    #[cfg(unix)]
    {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let precious = elsewhere.path().join("precious.txt");
        write(&precious, "do not delete me\n");

        // `sessions` is a symlink into a directory we do not own.
        std::fs::create_dir_all(home.path()).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), home.path().join("sessions")).unwrap();

        let output = Command::new(&binary)
            .args(["self", "uninstall", "--yes", "--purge", "--prefix"])
            .arg(home.path().join("prefix"))
            .env("VAK_HOME", home.path())
            .output()
            .expect("run uninstall");

        assert!(
            precious.exists(),
            "a purge followed a symlink and deleted a directory it does not own"
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            combined.contains("symlink"),
            "the refusal must say why: {combined}"
        );
    }
}
