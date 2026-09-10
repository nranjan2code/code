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

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

use vak_config::HookConfig;
use vak_plugin::{InstallOptions, InstallScope, PluginStore};

const SKILLS: &[(&str, &str, &str)] = &[
    (
        "getting-started",
        "Explain tasks clearly and help a new user choose the simplest next step.",
        "Translate jargon into plain language. Ask only for information that is genuinely needed, then present a short, actionable next step before optional detail.",
    ),
    (
        "research-and-sources",
        "Research a question with traceable sources and clearly separated evidence and inference.",
        "Define the question and freshness requirement, prefer primary sources, record publication dates, and distinguish sourced facts from your own synthesis. Never present an unverified assumption as a citation.",
    ),
    (
        "planning-and-organizing",
        "Turn goals into practical plans, checklists, and prioritised next actions.",
        "Clarify the desired outcome, identify dependencies and decisions, then produce a plan sized to the work. Keep ownership, deadlines, and open questions explicit.",
    ),
    (
        "debugging",
        "Diagnose failures from evidence before proposing or applying a fix.",
        "Reproduce or isolate the failure, capture the first meaningful error, trace inputs to the failing boundary, and test the smallest fix. Separate confirmed cause from hypotheses.",
    ),
    (
        "code-review",
        "Review code for correctness, security, regressions, and maintainability with actionable findings.",
        "Read the diff in context, prioritise concrete defects over style preferences, include impact and a precise location, and say when a concern is unverified rather than overstating it.",
    ),
    (
        "data-and-spreadsheets",
        "Clean, analyse, and explain tabular data without silently changing its meaning.",
        "Inspect headers, types, missing values, and units before transforming data. Keep source data intact, make calculations reproducible, and label estimates, exclusions, and assumptions.",
    ),
];

const PLUGINS: &[(&str, &str, &str, &str)] = &[
    (
        "developer-starter",
        "1.0.0",
        "Core developer workflows for implementation, debugging, and code review.",
        "software-development",
    ),
    (
        "everyday-starter",
        "1.0.0",
        "Plain-language writing, planning, research, and data help for everyday work.",
        "writing-and-editing",
    ),
];

const PLUGIN_SKILLS: &[(&str, &str, &str)] = &[
    (
        "software-development",
        "Implement and explain software changes with focused verification and clear tradeoffs.",
        "Inspect the existing conventions first. Make the smallest coherent change, preserve public contracts, add targeted tests for changed behavior, and report exactly what was verified.",
    ),
    (
        "writing-and-editing",
        "Draft, rewrite, summarize, and polish documents while preserving the requested voice.",
        "First identify audience, purpose, and format. Preserve facts and explicit constraints, make the smallest useful edit, and call out material ambiguities instead of inventing details.",
    ),
];

const SEED_MANIFEST: &str = ".seed-manifest.json";

pub fn seed_shared_capabilities() -> Result<(), String> {
    let root = vak_config::paths::default_workspace().join(".vak");
    seed_skills(&root.join("skills"))
        .map_err(|error| format!("Shared skill seed failed: {error}"))?;
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
        eprintln!(
            "removing retired plugin '{name}' (references retired tools: {})",
            retired_tools.join(", ")
        );
        store
            .remove(name)
            .map_err(|error| format!("could not remove retired plugin '{name}': {error}"))?;
        // Prune stale network_allow entries for the removed plugin. Leaving
        // this behind would advertise a capability that no longer exists.
        vak_config::prune_plugins_network_allow(root, name)
            .map_err(|error| format!("could not prune network_allow for '{name}': {error}"))?;
    }
    Ok(())
}

fn seed_skills(root: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let manifest_root = root.parent().unwrap_or(root);
    let mut shipped = load_seed_manifest(manifest_root);
    let previous = shipped.clone();
    for (name, description, body) in PLUGIN_SKILLS {
        let path = root.join(name).join("SKILL.md");
        let expected = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
        if path.is_file()
            && matches!(std::fs::read_to_string(&path), Ok(content) if content == expected)
        {
            std::fs::remove_file(&path)?;
            let _ = std::fs::remove_dir(path.parent().unwrap_or(root));
        }
    }
    for (name, description, body) in SKILLS {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("SKILL.md");
        let content = format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n");
        let expected = content.as_bytes();
        let expected_digest = digest(expected);
        match std::fs::read(&path) {
            Ok(current) if digest(&current) == expected_digest => {
                shipped.insert(name.to_string(), expected_digest);
            }
            Ok(current) => {
                // Only advance a seed when the file still equals the last
                // bytes we shipped. An untracked pre-existing file is treated
                // as user-owned and is never overwritten.
                if previous
                    .get(*name)
                    .is_some_and(|old| *old == digest(&current))
                {
                    std::fs::write(&path, expected)?;
                    shipped.insert(name.to_string(), expected_digest);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(&path, expected)?;
                shipped.insert(name.to_string(), expected_digest);
            }
            Err(error) => return Err(error),
        }
    }
    write_seed_manifest(manifest_root, &shipped)?;
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
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

fn seed_plugins(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let store = PluginStore::new(root);
    let mut shipped = load_seed_manifest(root);
    // Stage inside the plugin root rather than the system temp dir: same
    // filesystem as the destination, so installing is a rename and never a
    // cross-device copy — the same reason the installer stages inside its
    // own prefix.
    let staging = root.join(".seed-staging");
    let _ = std::fs::remove_dir_all(&staging);
    for (name, version, description, skill_name) in PLUGINS {
        let package = staging.join(name);
        let skill_path = package.join(format!("skills/{skill_name}"));
        std::fs::create_dir_all(&skill_path)?;
        std::fs::write(
            package.join("vak-plugin.json"),
            format!(
                r#"{{"schema":1,"name":"{name}","version":"{version}","description":"{description}","license":"MIT","components":{{"skills":["skills"]}}}}"#
            ),
        )?;
        let (skill_description, body) = PLUGIN_SKILLS
            .iter()
            .find(|(candidate, _, _)| candidate == skill_name)
            .map(|(_, description, body)| (*description, *body))
            .ok_or_else(|| format!("missing seed skill {skill_name}"))?;
        std::fs::write(
            skill_path.join("SKILL.md"),
            format!("---\nname: {skill_name}\ndescription: {skill_description}\n---\n\n{body}\n"),
        )?;
        if let Some(existing) = store
            .list()?
            .into_iter()
            .find(|plugin| plugin.name == *name)
        {
            let staged_digest = digest_of_directory(&package)?;
            let installed_digest = digest_of_directory(&existing.package_path).ok();
            if existing.digest == staged_digest {
                shipped.insert(format!("plugin:{name}"), existing.digest);
            } else if installed_digest.as_deref() == Some(existing.digest.as_str())
                && shipped.get(&format!("plugin:{name}")) == Some(&existing.digest)
            {
                let installed = store.update_local(
                    &package,
                    InstallOptions {
                        scope: InstallScope::User,
                        allow_unlicensed: false,
                    },
                )?;
                if !installed.enabled {
                    let _ = store.enable(name)?;
                }
                shipped.insert(format!("plugin:{name}"), installed.digest);
            }
            continue;
        }
        let installed = store.install_local(
            &package,
            InstallOptions {
                scope: InstallScope::User,
                allow_unlicensed: false,
            },
        )?;
        if !installed.enabled {
            let _ = store.enable(name)?;
        }
        shipped.insert(format!("plugin:{name}"), installed.digest);
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
    fn seed_manifest_tracks_standard_skills_without_clobbering_edits() {
        let _home = vak_config::paths::isolate_home_for_tests();
        let _ = seed_shared_capabilities();
        let root = vak_config::paths::default_workspace().join(".vak");
        let manifest: BTreeMap<String, String> = serde_json::from_slice(
            &std::fs::read(root.join(SEED_MANIFEST)).expect("seed manifest"),
        )
        .expect("valid seed manifest");
        assert_eq!(manifest.len(), SKILLS.len() + PLUGINS.len());

        let edited = root.join("skills/debugging/SKILL.md");
        let before = std::fs::read(&edited).expect("seed skill");
        std::fs::write(&edited, [before.as_slice(), b"\noperator edit\n"].concat())
            .expect("edit seed skill");
        let _ = seed_shared_capabilities();
        let after = std::fs::read(&edited).expect("edited seed skill");
        assert!(after.ends_with(b"\noperator edit\n"));

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
}
