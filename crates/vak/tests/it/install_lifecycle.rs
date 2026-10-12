//! The managed install lifecycle, driven as a real user drives it: by
//! running the `vak` binary, not by calling into its modules.
//!
//! Everything here is about a property the unit tests structurally cannot
//! reach — what happens when the binary performing an operation is *itself*
//! inside the prefix that operation rewrites. A module-level test always
//! runs from `target/debug/deps/`, which is never the install root, so the
//! interesting case never occurred.
//!
//! It is not a hypothetical. `self reinstall`, run from the installed CLI —
//! which is the `vak` on a person's PATH, and therefore the overwhelmingly
//! common invocation — used to clear the prefix, fail to find the source it
//! had just deleted, and leave the install EMPTY. The repair command
//! destroyed the thing it repairs.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn build_cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vak"))
}

/// Run a `vak self …` subcommand and return (exit code, stdout+stderr).
fn run(cli: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(cli)
        .args(args)
        .output()
        .expect("run the vak binary");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap_or(-1), text)
}

fn install_to(prefix: &Path) {
    let (code, text) = run(
        &build_cli(),
        &["self", "install", "--prefix", &prefix.to_string_lossy()],
    );
    assert_eq!(code, 0, "install failed: {text}");
}

/// The regression. Reinstalling from the installed binary must leave a
/// working install, not a hole where one used to be.
#[test]
fn reinstall_from_the_installed_binary_leaves_a_working_install() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("prefix");
    install_to(&prefix);

    let installed = prefix.join("bin").join("vak");
    assert!(installed.exists(), "install placed no CLI");

    let (code, text) = run(
        &installed,
        &[
            "self",
            "reinstall",
            "--prefix",
            &prefix.to_string_lossy(),
            "--yes",
        ],
    );
    assert_eq!(
        code, 0,
        "reinstall from the installed binary failed: {text}"
    );

    assert!(
        installed.exists(),
        "reinstall deleted the CLI and did not put it back — the prefix was \
         cleared while the binary being run was inside it"
    );
    let (code, text) = run(&installed, &["--version"]);
    assert_eq!(code, 0, "the reinstalled CLI does not run: {text}");

    let (code, text) = run(
        &installed,
        &["self", "verify", "--prefix", &prefix.to_string_lossy()],
    );
    assert_eq!(code, 0, "the reinstalled install does not verify: {text}");
}

/// Verify has to see through to the filesystem, not just re-read what the
/// manifest says about itself.
#[test]
fn verify_catches_a_component_that_was_tampered_with() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("prefix");
    install_to(&prefix);

    let victim = prefix.join("bin").join("vak-delivery-worker");
    if !victim.exists() {
        // Optional component; nothing to prove on a build without it.
        return;
    }
    let mut bytes = std::fs::read(&victim).unwrap();
    bytes.push(0);
    std::fs::write(&victim, bytes).unwrap();

    let (code, text) = run(
        &build_cli(),
        &["self", "verify", "--prefix", &prefix.to_string_lossy()],
    );
    assert_ne!(code, 0, "verify passed on a tampered component: {text}");
    assert!(
        text.contains("vak-delivery-worker"),
        "verify must name the component it found wrong, got: {text}"
    );
}

/// Uninstall is the reverse of install: the prefix goes, the data home
/// stays. `--purge` is the only thing that touches data, and is not
/// exercised here for the obvious reason.
#[test]
fn uninstall_removes_the_prefix_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("prefix");
    install_to(&prefix);
    assert!(prefix.join("bin").join("vak").exists());

    let (code, text) = run(
        &build_cli(),
        &[
            "self",
            "uninstall",
            "--prefix",
            &prefix.to_string_lossy(),
            "--yes",
        ],
    );
    assert_eq!(code, 0, "uninstall failed: {text}");
    assert!(
        !prefix.exists() || std::fs::read_dir(&prefix).unwrap().next().is_none(),
        "uninstall left files under {}",
        prefix.display()
    );
    assert!(
        text.contains("kept"),
        "uninstall must say that configuration and data were kept, got: {text}"
    );
}

/// A fresh install verifies clean. If this ever fails, every other
/// assertion in this file is measuring the wrong baseline.
#[test]
fn a_fresh_install_verifies_clean() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("prefix");
    install_to(&prefix);

    let (code, text) = run(
        &build_cli(),
        &["self", "verify", "--prefix", &prefix.to_string_lossy()],
    );
    assert_eq!(code, 0, "a fresh install does not verify: {text}");
    assert!(text.contains("verifies clean"), "unexpected output: {text}");
}
