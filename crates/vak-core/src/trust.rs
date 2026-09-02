//! Workspace trust: one marker store, read the same way everywhere.
//!
//! A project's `.vak/config.toml` and `.env` can grant execution power
//! (permission mode, allow rules, hooks, MCP servers, base-URL
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
pub fn marker_path(cwd: &Path) -> PathBuf {
    vak_config::paths::data_home()
        .join("trusted")
        .join(format!("{:016x}", fnv1a(cwd.to_string_lossy().as_bytes())))
}

/// True when this workspace has a recorded trust decision.
pub fn is_trusted(cwd: &Path) -> bool {
    marker_path(cwd).is_file()
}

/// True when the workspace asks for nothing privileged, so opening it
/// needs no decision at all. Keeping this distinct from `is_trusted`
/// is what lets onboarding stay silent for an ordinary directory and
/// speak up only for one that actually requests power.
pub fn requests_privilege(cwd: &Path) -> bool {
    vak_config::project_path(cwd).is_file() || cwd.join(".env").is_file()
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
        std::fs::write(other.path().join(".env"), "K=v").unwrap();
        assert!(requests_privilege(other.path()));
    }

    #[test]
    fn distinct_paths_get_distinct_markers() {
        assert_ne!(
            marker_path(Path::new("/a/one")),
            marker_path(Path::new("/a/two"))
        );
    }
}
