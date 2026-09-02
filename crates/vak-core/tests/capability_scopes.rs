#![allow(clippy::unwrap_used, clippy::expect_used)]

//! End-to-end contract for the five extension points across both scopes.
//!
//! vak advertises skills, custom commands, plugins, hooks, and MCP servers as
//! working "global as well as specific to workspace" — and each is resolved by
//! its own code path, so "it works for skills" has never implied anything
//! about the other four. This asserts the whole matrix in one place:
//! discovery at each scope, workspace-over-shared precedence, and that every
//! `inherit_*` switch actually isolates the capability it names.
//!
//! `VAK_HOME` is process-global, so everything runs inside a single test.

use std::path::{Path, PathBuf};

use vak_core::Core;

struct Layout {
    _root: tempfile::TempDir,
    _shared: PathBuf,
    workspace: PathBuf,
}

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).unwrap();
    std::fs::write(path, body).unwrap();
}

fn skill(root: &Path, name: &str, description: &str) {
    write(
        &root.join("skills").join(name).join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: {description}\n---\nBody for {name}.\n"),
    );
}

fn command(root: &Path, name: &str, description: &str) {
    write(
        &root.join("commands").join(format!("{name}.md")),
        &format!("---\ndescription: {description}\n---\nTemplate for {name}.\n"),
    );
}

/// Builds a data home containing a shared workspace and a separate project
/// workspace, each carrying every capability kind, with one deliberate name
/// collision per kind.
fn layout() -> Layout {
    let root = tempfile::tempdir().unwrap();
    // Safe: `get_var` consults the override map above the real
    // environment, so pinning the home needs no `std::env::set_var` and
    // therefore no `unsafe` (AGENTS.md invariant 6).
    vak_config::paths::set_home_override(root.path());

    let shared = root.path().join("vak-home/.vak");
    let workspace = root.path().join("project/.vak");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();

    skill(&shared, "shared-only", "lives only in the shared workspace");
    skill(&shared, "collide", "the shared definition");
    skill(&workspace, "workspace-only", "lives only in this workspace");
    skill(&workspace, "collide", "the workspace definition");

    command(&shared, "shared-cmd", "shared command");
    command(&shared, "both-cmd", "shared version");
    command(&workspace, "workspace-cmd", "workspace command");
    command(&workspace, "both-cmd", "workspace version");

    write(
        &shared.join("config.toml"),
        "[mcp.servers.shared-server]\ncommand = \"true\"\nargs = []\n\n\
         [[hooks]]\nevent = \"pre-tool-use\"\ncommand = \"echo shared-hook\"\nenabled = true\n",
    );

    Layout {
        _root: root,
        _shared: shared,
        workspace,
    }
}

fn build(layout: &Layout, project_config: &str) -> Core {
    write(&layout.workspace.join("config.toml"), project_config);
    Core::new_with_trust(
        layout
            .workspace
            .parent()
            .expect("workspace parent")
            .to_path_buf(),
        true,
    )
    .unwrap()
}

fn skill_names(core: &Core) -> Vec<String> {
    let mut names: Vec<String> = core.skills().into_iter().map(|s| s.name).collect();
    names.sort();
    names
}

fn command_names(core: &Core) -> Vec<String> {
    let mut names: Vec<String> = core.custom_commands().into_iter().map(|c| c.name).collect();
    names.sort();
    names
}

#[test]
fn every_capability_resolves_at_both_scopes_and_every_switch_isolates() {
    let layout = layout();

    // ── default: both scopes contribute, workspace wins collisions ───────
    let core = build(&layout, "provider = \"anthropic\"\n");

    let skills = skill_names(&core);
    assert!(
        skills.contains(&"workspace-only".to_string()),
        "workspace skills must be discovered: {skills:?}"
    );
    assert!(
        skills.contains(&"shared-only".to_string()),
        "shared skills must be inherited by default: {skills:?}"
    );
    assert_eq!(
        skills.iter().filter(|n| *n == "collide").count(),
        1,
        "a shadowed skill must not appear twice: {skills:?}"
    );
    let winner = core
        .skills()
        .into_iter()
        .find(|s| s.name == "collide")
        .expect("collide");
    assert_eq!(
        winner.description, "the workspace definition",
        "workspace scope must shadow shared scope"
    );

    let commands = command_names(&core);
    assert!(
        commands.contains(&"workspace-cmd".to_string()),
        "workspace commands must be discovered: {commands:?}"
    );
    assert!(
        commands.contains(&"shared-cmd".to_string()),
        "shared commands must be inherited by default: {commands:?}"
    );

    assert!(
        core.effective_mcp().servers.contains_key("shared-server"),
        "an MCP server defined in the shared config must reach the workspace"
    );
    assert!(
        core.effective_hooks()
            .iter()
            .any(|h| h.command.contains("shared-hook")),
        "a hook defined in the shared config must reach the workspace"
    );

    // ── inherit_skills = false ───────────────────────────────────────────
    let isolated = build(
        &layout,
        "provider = \"anthropic\"\n[capabilities]\ninherit_skills = false\n",
    );
    let skills = skill_names(&isolated);
    assert!(
        skills.contains(&"workspace-only".to_string()),
        "isolation must not remove the workspace's own skills: {skills:?}"
    );
    assert!(
        !skills.contains(&"shared-only".to_string()),
        "inherit_skills = false must drop shared skills: {skills:?}"
    );
    assert!(
        command_names(&isolated).contains(&"shared-cmd".to_string()),
        "inherit_skills must not silently take commands with it"
    );

    // ── inherit_commands = false ─────────────────────────────────────────
    let isolated = build(
        &layout,
        "provider = \"anthropic\"\n[capabilities]\ninherit_commands = false\n",
    );
    let commands = command_names(&isolated);
    assert!(
        commands.contains(&"workspace-cmd".to_string()),
        "isolation must not remove the workspace's own commands: {commands:?}"
    );
    assert!(
        !commands.contains(&"shared-cmd".to_string()),
        "inherit_commands = false must drop shared commands: {commands:?}"
    );
    assert!(
        skill_names(&isolated).contains(&"shared-only".to_string()),
        "inherit_commands must not silently take skills with it"
    );

    // ── inherit_mcp = false ──────────────────────────────────────────────
    let isolated = build(
        &layout,
        "provider = \"anthropic\"\n[capabilities]\ninherit_mcp = false\n",
    );
    assert!(
        !isolated
            .effective_mcp()
            .servers
            .contains_key("shared-server"),
        "inherit_mcp = false must drop the shared MCP server"
    );

    // ── inherit_hooks = false ────────────────────────────────────────────
    let isolated = build(
        &layout,
        "provider = \"anthropic\"\n[capabilities]\ninherit_hooks = false\n",
    );
    assert!(
        !isolated
            .effective_hooks()
            .iter()
            .any(|h| h.command.contains("shared-hook")),
        "inherit_hooks = false must drop the shared hook"
    );

    // ── every switch at once: a fully isolated workspace ─────────────────
    let sealed = build(
        &layout,
        "provider = \"anthropic\"\n[capabilities]\ninherit_mcp = false\n\
         inherit_hooks = false\ninherit_skills = false\ninherit_commands = false\n\
         inherit_plugins = false\n",
    );
    let skills = skill_names(&sealed);
    let commands = command_names(&sealed);
    assert!(!skills.contains(&"shared-only".to_string()));
    assert!(!commands.contains(&"shared-cmd".to_string()));
    assert!(!sealed.effective_mcp().servers.contains_key("shared-server"));
    assert!(
        !sealed
            .effective_hooks()
            .iter()
            .any(|h| h.command.contains("shared-hook"))
    );
    assert!(
        skills.contains(&"workspace-only".to_string())
            && commands.contains(&"workspace-cmd".to_string()),
        "a sealed workspace keeps everything it defines itself: {skills:?} {commands:?}"
    );
}
