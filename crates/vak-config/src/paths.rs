//! Canonical install/data layout (docs/design/32-release-engineering.md,
//! "Canonical layout" section). THE single source of truth for where
//! vakcoder puts binaries, data, caches, and logs on each platform.
//!
//! Every crate resolves paths through these functions — never by hand-
//! rolling `HOME/.vakcoder` again. The 0.7 drift incident (services kept
//! executing an old image while five call sites disagreed about home)
//! is the standing reason this module exists.
//!
//! macOS (Apple File System Programming Guide):
//!   data   ~/Library/Application Support/vakcoder
//!   cache  ~/Library/Caches/vakcoder
//!   logs   ~/Library/Logs/vakcoder
//! Linux (XDG Base Directory Specification):
//!   data   $XDG_DATA_HOME/vakcoder        (~/.local/share/vakcoder)
//!   cache  $XDG_CACHE_HOME/vakcoder       (~/.cache/vakcoder)
//!   logs   $XDG_STATE_HOME/vakcoder/logs  (~/.local/state/vakcoder/logs)
//!
//! `VAKCODER_HOME` overrides the DATA home everywhere and disables the
//! legacy migration — an explicit override is the user's layout choice.

use std::path::PathBuf;

use crate::get_var;

/// The user data home: sessions, memory, tasks, config state, audit logs.
pub fn data_home() -> PathBuf {
    resolve(get_var("VAKCODER_HOME").as_deref()).data
}

/// Rebuildable artifacts only (the SQLite FTS index and WAL sidecars).
/// Deleting this directory must always be safe; it is rebuilt from JSONL.
pub fn cache_home() -> PathBuf {
    resolve(get_var("VAKCODER_HOME").as_deref()).cache
}

/// Service + CLI log files (Console.app-visible on macOS).
pub fn logs_dir() -> PathBuf {
    resolve(get_var("VAKCODER_HOME").as_deref()).logs
}

/// All three homes derived from one override decision. Pure so tests can
/// exercise both branches without touching process-global environment.
struct Homes {
    data: PathBuf,
    cache: PathBuf,
    logs: PathBuf,
}

fn resolve(override_home: Option<&str>) -> Homes {
    let base = base_home();
    match override_home.filter(|s| !s.is_empty()) {
        // An explicit override is a self-contained sandbox: everything
        // nests under it so tests and portable installs stay one tree.
        Some(h) => {
            let data = PathBuf::from(h);
            Homes {
                cache: data.join("cache"),
                logs: data.join("logs"),
                data,
            }
        }
        None => Homes {
            #[cfg(target_os = "macos")]
            data: base
                .join("Library")
                .join("Application Support")
                .join(app_dir_name()),
            #[cfg(target_os = "macos")]
            cache: base.join("Library").join("Caches").join(app_dir_name()),
            #[cfg(target_os = "macos")]
            logs: base.join("Library").join("Logs").join(app_dir_name()),
            #[cfg(not(target_os = "macos"))]
            data: xdg(&base, "XDG_DATA_HOME", ".local/share"),
            #[cfg(not(target_os = "macos"))]
            cache: xdg(&base, "XDG_CACHE_HOME", ".cache"),
            #[cfg(not(target_os = "macos"))]
            logs: xdg(&base, "XDG_STATE_HOME", ".local/state").join("logs"),
        },
    }
}

#[cfg(not(target_os = "macos"))]
fn xdg(base: &std::path::Path, env_key: &str, default_suffix: &str) -> PathBuf {
    if let Some(v) = std::env::var_os(env_key)
        && !v.is_empty()
    {
        return PathBuf::from(v).join(app_dir_name());
    }
    base.join(default_suffix).join(app_dir_name())
}

fn app_dir_name() -> &'static str {
    "vakcoder"
}

fn base_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The override must win everywhere and produce a self-contained
    /// tree (tests + portable installs depend on one-directory depth).
    #[test]
    fn override_pins_every_directory_to_one_tree() {
        let sandbox = tempfile::tempdir().unwrap();
        let h = resolve(Some(sandbox.path().to_str().unwrap()));
        assert_eq!(h.data, sandbox.path());
        assert_eq!(
            h.logs,
            h.data.join("logs"),
            "overridden homes are self-contained"
        );
        assert_eq!(h.cache, h.data.join("cache"));
    }

    #[test]
    fn canonical_homes_follow_platform_convention() {
        let h = resolve(None);
        let base = base_home();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(h.data, base.join("Library/Application Support/vakcoder"));
            assert_eq!(h.cache, base.join("Library/Caches/vakcoder"));
            assert_eq!(h.logs, base.join("Library/Logs/vakcoder"));
        }
        // Invariants that hold on every platform:
        assert!(
            !h.cache.starts_with(&h.data),
            "cache must not nest inside data"
        );
        for p in [&h.data, &h.cache, &h.logs] {
            assert!(
                !p.to_string_lossy().contains("/.vakcoder"),
                "canonical layout must not use the legacy dotdir: {}",
                p.display()
            );
        }
    }

    #[test]
    fn empty_override_is_not_an_override() {
        let h = resolve(Some(""));
        assert!(
            !h.data.to_string_lossy().contains("/cache"),
            "empty string must fall through to the platform layout"
        );
    }
}
