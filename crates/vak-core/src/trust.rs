//! Workspace trust: one marker store, read the same way everywhere.
//!
//! A project's `.vak/config.toml` and secret scope can grant execution
//! power (permission mode, allow rules, hooks, MCP servers, base-URL
//! redirection), so they stay demoted until the operator says otherwise
//! (`docs/design/05-config.md`). The decision is per canonical directory
//! and remembered under `<data_home>/trusted/` by space id.
//!
//! This lived in the CLI binary, where the server and the onboarding
//! projection could not reach it — so "is this workspace trusted?" had
//! one implementation and several guesses. One authority now.

use std::path::{Path, PathBuf};

fn trusted_dir() -> PathBuf {
    vak_config::scope::SharedScope::new(vak_config::paths::data_home()).trusted()
}

/// Where the trust decision for `cwd` is recorded: a marker named by the
/// space its folder is bound to (`vak_config::spaces`), so every spelling of
/// one directory (`/tmp/x` and `/private/tmp/x`, relative and absolute) has
/// one answer, and a decision is about the space, not a string.
pub fn marker_path(cwd: &Path) -> PathBuf {
    trusted_dir().join(vak_config::spaces::key(cwd))
}

/// True when this workspace has a recorded trust decision. Asking never
/// binds a folder that has not been opened.
pub fn is_trusted(cwd: &Path) -> bool {
    vak_config::spaces::bound_space(cwd).is_some_and(|space| {
        trusted_dir().join(&space).is_file()
            || process_trust()
                .lock()
                .is_ok_and(|trust| trust.contains(&space))
    })
}

fn process_trust() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static TRUST: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    TRUST.get_or_init(Default::default)
}

/// Trust `cwd`'s space for the life of this process without recording it,
/// as a `Core` opened trusted does (`serve --trust`). The space's Agent
/// workspaces belong to it (`vak_config::spaces`), so they are trusted with
/// it, and nothing outlives the process.
pub fn trust_for_process(cwd: &Path) -> Result<(), String> {
    let space = vak_config::spaces::bind(cwd)?;
    process_trust()
        .lock()
        .map_err(|_| "trust lock poisoned".to_string())?
        .insert(space);
    Ok(())
}

/// True when the workspace asks for nothing privileged, so opening it
/// needs no decision at all. Keeping this distinct from `is_trusted`
/// is what lets onboarding stay silent for an ordinary directory and
/// speak up only for one that actually requests power.
pub fn requests_privilege(cwd: &Path) -> bool {
    vak_config::project_path(cwd).is_file()
        || vak_config::credentials::scope_has_any(&cwd.join(".env"))
}

/// Which privileged sections a workspace's project layer actually asks
/// for, read as **text**, never loaded.
///
/// The trust review has to tell an operator what they would be agreeing
/// to, and it must do that without activating any of it — parsing the
/// config through the normal loader to describe it would be granting the
/// thing being asked about (doc 46, Step 2).
pub fn requested_privileges(cwd: &Path) -> Vec<&'static str> {
    let mut found = Vec::new();
    if vak_config::credentials::scope_has_any(&cwd.join(".env")) {
        found.push("secrets in this project's secret scope");
    }
    let Ok(text) = std::fs::read_to_string(vak_config::project_path(cwd)) else {
        return found;
    };
    // Section headers and top-level keys only. A substring scan is enough
    // to answer "does this file ask for X?", and cannot execute anything.
    for (needle, label) in [
        ("[[hooks]]", "hooks that run commands"),
        ("[hooks]", "hooks that run commands"),
        // No closing bracket: a server is declared as
        // `[mcp.servers.<name>]`, so matching the full header would miss
        // every real one.
        ("[mcp.servers", "external tool servers"),
        ("permission_mode", "a permission mode"),
        ("anthropic_base_url", "a redirected provider endpoint"),
        ("[sandbox]", "sandbox settings"),
        ("[gateway]", "gateway settings"),
        // Network exposure: which interface answers, which Host headers are
        // accepted, and whether a remote caller reaches a shell
        // (docs/design/48-web-client.md §4.2).
        ("[server]", "network exposure settings"),
        ("allow", "pre-granted allow rules"),
        ("[prompt", "prompt layers"),
    ] {
        if text.contains(needle) && !found.contains(&label) {
            found.push(label);
        }
    }
    found
}

/// Record the decision, the one writer of a trust marker. Best-effort: an
/// unwritable data home means the workspace is simply asked about again
/// next time, never silently trusted.
pub fn record(cwd: &Path) -> std::io::Result<()> {
    let space = vak_config::spaces::bind(cwd).map_err(std::io::Error::other)?;
    let marker = trusted_dir().join(space);
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(marker, cwd.to_string_lossy().as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// One directory must have one trust record, however it is spelled.
    /// `CorePool` canonicalizes its keys and the CLI passes the cwd as
    /// given, so before this the same workspace could be trusted through one
    /// path and untrusted through the other.
    #[test]
    fn a_decision_is_visible_through_every_spelling_of_the_same_directory() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        assert!(!is_trusted(dir.path()));

        record(dir.path()).unwrap();
        assert!(is_trusted(dir.path()));
        assert!(is_trusted(&canonical), "canonical form sees it too");

        // And a relative spelling of the same place.
        let relative = dir.path().join("./");
        assert!(is_trusted(&relative));
    }

    #[test]
    fn a_directory_with_no_privileged_files_asks_for_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!requests_privilege(dir.path()));
    }

    #[test]
    fn a_project_config_or_env_makes_the_workspace_privileged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
        std::fs::write(dir.path().join(".vak/config.toml"), "").unwrap();
        assert!(requests_privilege(dir.path()));

        let other = tempfile::tempdir().unwrap();
        vak_config::upsert_env_file(&other.path().join(".env"), "K", "v").unwrap();
        assert!(requests_privilege(other.path()));
    }

    #[test]
    fn a_review_names_what_a_project_asks_for_without_loading_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
        std::fs::write(
            dir.path().join(".vak/config.toml"),
            "permission_mode = \"full-access\"\n[mcp.servers.thing]\ncommand = \"sh\"\n",
        )
        .unwrap();
        let asked = requested_privileges(dir.path());
        assert!(asked.contains(&"a permission mode"), "{asked:?}");
        assert!(asked.contains(&"external tool servers"), "{asked:?}");
    }

    #[test]
    fn a_plain_directory_asks_for_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(requested_privileges(dir.path()).is_empty());
    }

    #[test]
    fn distinct_paths_get_distinct_markers() {
        assert_ne!(
            marker_path(Path::new("/a/one")),
            marker_path(Path::new("/a/two"))
        );
    }
}
