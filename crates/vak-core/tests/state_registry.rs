#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Enforcement for the durable state registry
//! (`docs/design/46-stabilization-install-and-onboarding.md` Part VII.2).
//!
//! The registry's whole value is that it is complete. A declaration that
//! drifts from what the code actually writes is worse than none, because
//! `--purge`, backup coverage, and the upgrade gate all trust it.
//!
//! So this drives real work against a private home and fails on any
//! durable file that appeared without a declaration.

use std::path::{Path, PathBuf};

use vak_core::state::{self, Root};

/// Every file under `root`, as paths relative to it.
fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                found.push(rel.to_path_buf());
            }
        }
    }
    found
}

/// Files the operating system or a build leaves behind, which are nobody's
/// durable state and would otherwise make this test fail for reasons that
/// have nothing to do with vak.
fn is_incidental(relative: &Path) -> bool {
    relative
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some(".DS_Store") | Some(".git")))
}

/// Starting a session and recording a security event is enough to touch
/// the ledgers, the sessions tree, and the trust store — the paths a
/// first run actually creates.
#[tokio::test]
async fn a_real_run_writes_only_declared_durable_state() {
    let home = vak_config::paths::isolate_home_for_tests();
    let workspace = tempfile::tempdir().expect("workspace");

    let core = vak_core::Core::new_with_trust(workspace.path().to_path_buf(), true).expect("core");
    let sessions_home = home.join("state-registry-run");
    core.set_sessions_home(sessions_home.clone());

    // Real durable writes: a session ledger, and an audit entry.
    let _session = core.start_session().await.expect("session");
    vak_core::security_events::record(
        &sessions_home,
        vak_core::security_events::EventKind::ConfigChange,
        "registry_probe",
        "state-registry test",
        None,
    );

    let undeclared: Vec<PathBuf> = files_under(&sessions_home)
        .into_iter()
        .filter(|rel| !is_incidental(rel))
        .filter(|rel| !state::is_declared(Root::Data, rel))
        .collect();

    assert!(
        undeclared.is_empty(),
        "durable files were written that the registry does not declare: {undeclared:?}\n\
         Add them to `vak_core::state::REGISTRY` — `--purge`, backup coverage, and the \
         upgrade gate all read it, so an undeclared file is one that silently survives a \
         purge, never gets backed up, and is never checked across an update."
    );
}

/// Seeding writes the Shared layer, which is the other root.
#[test]
fn seeding_writes_only_declared_shared_state() {
    let home = vak_config::paths::isolate_home_for_tests();
    vak_core::seed::seed_shared_capabilities();

    let shared = vak_config::paths::default_workspace();
    if !shared.exists() {
        // `isolate_home_for_tests` points the Shared root inside `home`;
        // if nothing was written there, there is nothing to check.
        assert!(home.exists());
        return;
    }

    let undeclared: Vec<PathBuf> = files_under(&shared)
        .into_iter()
        .filter(|rel| !is_incidental(rel))
        .filter(|rel| !state::is_declared(Root::Shared, rel))
        .collect();

    assert!(
        undeclared.is_empty(),
        "seeding wrote Shared files the registry does not declare: {undeclared:?}"
    );
}

/// A purge must be able to name what it removes, for every root.
///
/// `--purge` derives its targets from this, so an entry with no disposition
/// would be a file the purge simply never considered.
#[test]
fn every_entry_states_what_a_purge_does_with_it() {
    for entry in state::REGISTRY {
        // The type makes this total; the assertion is that the registry is
        // non-empty and every root is represented, so a whole root cannot
        // be forgotten.
        let _ = entry.on_purge;
    }
    for root in [Root::Data, Root::Shared, Root::Cache] {
        assert!(
            state::entries_for(root).next().is_some(),
            "{root:?} has no declared entries — a whole root would escape a purge"
        );
    }
}
