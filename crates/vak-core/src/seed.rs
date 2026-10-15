//! Shared capability seeds, applied by **setup** and reconciled by install/update.
//!
//! Lives in `vak-core` because every surface that can run setup needs it:
//! the CLI (`vak setup seed`) and the web wizard (`POST /onboarding/seed`)
//! must install the same seeds the same way, and a copy in the binary
//! crate could only ever serve one of them.
//!
//! Placing binaries used to seed skills, plugins, and a disabled hook as a
//! side effect (`docs/design/46-stabilization-install-and-onboarding.md`
//! D6). That had two defects beyond the contract violation: it only ran
//! when the prefix happened to equal the platform default, so any
//! `--prefix` install silently got nothing; and it never ran on update, so
//! a seed shipped in a release reached nobody who upgraded. Setup owns it
//! now, against the workspace the operator actually chose. Updates run the
//! same reconciliation so newly shipped standard capabilities reach existing
//! workspaces without overwriting user edits.
//!
//! Every skill Vakyartha ships lives in a shipped package (`packages/`,
//! `crate::packages::SHIPPED`): there is no second way to ship one.

use std::collections::BTreeMap;
use std::path::Path;

use vak_config::HookConfig;
use vak_plugin::{InstallOptions, InstallScope, PluginStore};

const SEED_MANIFEST: &str = ".seed-manifest.json";

pub fn seed_shared_capabilities() -> Result<(), String> {
    let workspace = vak_config::scope::WorkspaceScope::new(vak_config::paths::default_workspace());
    let root = workspace.project_dir();
    seed_plugins(&root).map_err(|error| format!("Shared plugin seed failed: {error}"))?;
    cleanup_retired_plugins(&root)
        .map_err(|error| format!("Retired plugin cleanup failed: {error}"))?;
    let hooks = [HookConfig {
        event: "session_start".into(),
        matcher: None,
        command: "/usr/bin/true".into(),
        timeout_ms: Some(1_000),
        enabled: false,
        failure_mode: Some("open".into()),
    }];
    vak_config::seed_global_hooks_if_empty(&hooks)
        .map_err(|error| format!("Shared automation seed failed: {error}"))?;
    vak_config::seed_global_plugins_network_allow_if_empty()
        .map_err(|error| format!("Shared plugin network seed failed: {error}"))?;
    Ok(())
}

/// Remove plugin packages whose skill descriptions reference retired tool
/// names (e.g. `python_eval`, `react_preview`). These plugins were shipped
/// before the tool interface was unified on `bash` and their SKILL.md files
/// still instruct the model to call tools that no longer exist — which
/// causes `unknown_capability` errors and model hallucinations of tool
/// output (docs/design/53, AGENTS.md invariants 9 and 29).
///
/// This runs during setup and update so stale plugins never survive a
/// version bump. It also prunes stale `network_allow` entries so the
/// retained allowlist stays consistent with the on-disk plugin store.
fn cleanup_retired_plugins(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let store = vak_plugin::PluginStore::new(root);
    let flagged = store.retired_plugins()?;
    for (name, retired_tools) in &flagged {
        tracing::info!(
            count = retired_tools.len(),
            "removing a plugin that references retired tools"
        );
        store
            .remove(name)
            .map_err(|error| format!("could not remove retired plugin '{name}': {error}"))?;
        // Prune stale network_allow entries for the removed plugin. Leaving
        // this behind would advertise a capability that no longer exists.
        let config = root.join("config.toml");
        if config.is_file() {
            vak_config::prune_plugins_network_allow(&config, name)
                .map_err(|error| format!("could not prune network_allow for '{name}': {error}"))?;
        }
    }
    Ok(())
}

fn load_seed_manifest(root: &Path) -> BTreeMap<String, String> {
    std::fs::read(root.join(SEED_MANIFEST))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_seed_manifest(root: &Path, shipped: &BTreeMap<String, String>) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(shipped).map_err(std::io::Error::other)?;
    let manifest_path = root.join(SEED_MANIFEST);
    let temporary = manifest_path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, manifest_path)?;
    Ok(())
}

/// Installs every package Vakyartha ships (`crate::packages::SHIPPED`) into
/// the Shared layer. A new one is installed in its shipped state. One
/// already installed is replaced by the shipped bytes only while it is still
/// exactly what was shipped last, keeping the owner's enabled state; a copy
/// the owner changed is left alone.
fn seed_plugins(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let store = PluginStore::new(root);
    let mut shipped = load_seed_manifest(root);
    // Stage inside the plugin root rather than the system temp dir: same
    // filesystem as the destination, so installing is a rename and never a
    // cross-device copy.
    let staging = root.join(".seed-staging");
    let _ = std::fs::remove_dir_all(&staging);
    let options = InstallOptions {
        scope: InstallScope::User,
        allow_unlicensed: false,
    };
    for package in crate::packages::SHIPPED {
        let name = package.name;
        let staged = staging.join(name);
        crate::packages::stage(name, &staged)?;
        let key = format!("plugin:{name}");
        let staged_digest = digest_of_directory(&staged)?;
        match store.list()?.into_iter().find(|plugin| plugin.name == name) {
            Some(existing) if existing.digest == staged_digest => {
                shipped.insert(key, existing.digest);
            }
            Some(existing) => {
                let untouched = digest_of_directory(&existing.package_path).ok().as_deref()
                    == Some(existing.digest.as_str())
                    && shipped.get(&key) == Some(&existing.digest);
                if untouched {
                    let installed = store.update_local(&staged, options)?;
                    if existing.enabled && !installed.enabled {
                        let _ = store.enable(name)?;
                    }
                    shipped.insert(key, installed.digest);
                }
            }
            None => {
                let installed = store.install_local(&staged, options)?;
                if package.enabled {
                    let _ = store.enable(name)?;
                }
                shipped.insert(key, installed.digest);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&staging);
    write_seed_manifest(root, &shipped)?;
    Ok(())
}

fn digest_of_directory(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let inspection = vak_plugin::inspect_package(root)?;
    Ok(inspection.digest)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    #[test]
    fn seed_manifest_tracks_shipped_packages_without_clobbering_edits() {
        let _home = vak_config::paths::isolate_home_for_tests();
        let _ = seed_shared_capabilities();
        let root = vak_config::paths::default_workspace().join(".vak");
        let manifest: BTreeMap<String, String> = serde_json::from_slice(
            &std::fs::read(root.join(SEED_MANIFEST)).expect("seed manifest"),
        )
        .expect("valid seed manifest");
        assert_eq!(manifest.len(), crate::packages::SHIPPED.len());
        let plugins = PluginStore::new(&root).list().expect("seed plugins");
        let package = plugins
            .iter()
            .find(|plugin| plugin.name == "developer-starter")
            .expect("developer starter")
            .package_path
            .join("operator-note.txt");
        std::fs::write(&package, "operator edit\n").expect("edit plugin package");
        let _ = seed_shared_capabilities();
        assert!(package.is_file(), "edited plugin package was overwritten");
    }

    /// Every shipped package is installed in its shipped state, byte for
    /// byte what `packages/<name>` holds.
    #[test]
    fn every_shipped_package_is_installed_as_shipped() {
        let dir = tempfile::tempdir().expect("temporary plugin root");
        seed_plugins(dir.path()).expect("seed packages");
        let installed = PluginStore::new(dir.path()).list().expect("list packages");
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
        for shipped in crate::packages::SHIPPED {
            let plugin = installed
                .iter()
                .find(|plugin| plugin.name == shipped.name)
                .unwrap_or_else(|| panic!("{} was not installed", shipped.name));
            assert_eq!(plugin.enabled, shipped.enabled, "{}", shipped.name);
            assert_eq!(plugin.scope, InstallScope::User);
            assert_eq!(
                Some(plugin.digest.clone()),
                crate::packages::digest(shipped.name),
                "{} drifted from packages/{}",
                shipped.name,
                shipped.name
            );
            assert_eq!(
                vak_plugin::inspect_package(&source.join(shipped.name))
                    .expect("source package")
                    .digest,
                plugin.digest,
                "{}",
                shipped.name
            );
        }
    }
}
