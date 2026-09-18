//! Workspace trust: one marker store, read the same way everywhere.
//!
//! A project's `.vak/config.toml` and secret scope can grant execution
//! power (permission mode, allow rules, hooks, MCP servers, base-URL
//! redirection), so they stay demoted until the operator says otherwise
//! (`docs/design/05-config.md`). The decision is per canonical directory
//! and remembered under `<data_home>/trusted/`.
//!
//! This lived in the CLI binary, where the server and the onboarding
//! projection could not reach it — so "is this workspace trusted?" had
//! one implementation and several guesses. One authority now.

use std::path::{Path, PathBuf};

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Where the trust decision for `cwd` is recorded.
///
/// The path is canonicalized first. Without that, one directory has as many
/// trust records as it has spellings — `/tmp/x` and `/private/tmp/x` are the
/// same directory on macOS, a relative path and its absolute form are the
/// same directory everywhere, and a decision recorded through one is
/// invisible through the other. Callers do not agree on a spelling
/// (`CorePool` canonicalizes its keys; the CLI passes the cwd as given), and
/// "is this workspace trusted?" having two answers is the exact failure this
/// module exists to end.
///
/// A path that cannot be canonicalized (it does not exist yet) falls back to
/// its literal form rather than failing: recording a decision about a
/// directory that is about to be created is legitimate.
pub fn marker_path(cwd: &Path) -> PathBuf {
    marker_for(&cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()))
}

fn marker_for(path: &Path) -> PathBuf {
    vak_config::paths::data_home()
        .join("trusted")
        .join(format!("{:016x}", fnv1a(path.to_string_lossy().as_bytes())))
}

/// True when this workspace has a recorded trust decision.
///
/// Checks the literal spelling too, so a marker written before paths were
/// canonicalized still counts. Decisions an operator has already made must
/// not be silently forgotten by a change to how they are addressed.
pub fn is_trusted(cwd: &Path) -> bool {
    marker_path(cwd).is_file() || marker_for(cwd).is_file()
}

/// Record a trust decision for `cwd`, the same way the CLI's own interactive
/// "trust this workspace?" prompt does — a file at [`marker_path`], so every
/// later [`is_trusted`] call (including `CorePool`'s) sees it immediately.
///
/// A user-created Agent's isolated workspace (`.vak/agents/<id>/workspace`)
/// is never visited or prompted about directly, so without this it can never
/// pass `is_trusted` and its own `permission_mode`, `hooks`, `mcp.servers`,
/// and other privileged config are silently stripped forever (see
/// `vak_config`'s `PRIVILEGED_KEYS_NOTICE`) — an Agent whose settings a user
/// configures through a trusted admin session but that silently never apply.
/// Callers must only invoke this when the *creating* context is itself
/// already trusted; it is not a substitute for that decision, only a way to
/// carry it forward onto a directory the decision already covers in spirit.
pub fn mark_trusted(cwd: &Path) -> std::io::Result<()> {
    let marker = marker_path(cwd);
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&marker, cwd.to_string_lossy().as_bytes())
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

/// Record the decision. Best-effort: an unwritable data home means the
/// workspace is simply asked about again next time, never silently
/// trusted.
pub fn record(cwd: &Path) -> std::io::Result<()> {
    let marker = marker_path(cwd);
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
        crate::pin_test_data_home();
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

    /// A marker written before paths were canonicalized still counts —
    /// decisions an operator already made must not be forgotten by a change
    /// to how they are addressed.
    #[test]
    fn a_legacy_uncanonicalized_marker_is_still_honoured() {
        crate::pin_test_data_home();
        let dir = tempfile::tempdir().unwrap();
        // `/var/...` on macOS canonicalizes to `/private/var/...`, so this
        // literal-form marker is at a different hash than the current one.
        let legacy = marker_for(dir.path());
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, dir.path().to_string_lossy().as_bytes()).unwrap();
        assert!(is_trusted(dir.path()));
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
