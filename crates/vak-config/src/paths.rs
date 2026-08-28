//! Canonical install/data layout (docs/design/32-release-engineering.md,
//! "Canonical layout" section). THE single source of truth for where
//! vak puts binaries, data, caches, and logs on each platform.
//!
//! Every crate resolves paths through these functions — never by hand-
//! rolling `HOME/.vak` again. The 0.7 drift incident (services kept
//! executing an old image while five call sites disagreed about home)
//! is the standing reason this module exists.
//!
//! macOS (Apple File System Programming Guide):
//!   data   ~/Library/Application Support/vak
//!   cache  ~/Library/Caches/vak
//!   logs   ~/Library/Logs/vak
//! Linux (XDG Base Directory Specification):
//!   data   $XDG_DATA_HOME/vak        (~/.local/share/vak)
//!   cache  $XDG_CACHE_HOME/vak       (~/.cache/vak)
//!   logs   $XDG_STATE_HOME/vak/logs  (~/.local/state/vak/logs)
//!
//! `VAK_HOME` overrides the DATA home everywhere and disables the
//! legacy migration — an explicit override is the user's layout choice.

use std::path::PathBuf;

use crate::get_var;

/// The user data home: sessions, memory, tasks, config state, audit logs.
pub fn data_home() -> PathBuf {
    resolve(get_var("VAK_HOME").as_deref()).data
}

/// Rebuildable artifacts only (the SQLite FTS index and WAL sidecars).
/// Deleting this directory must always be safe; it is rebuilt from JSONL.
pub fn cache_home() -> PathBuf {
    resolve(get_var("VAK_HOME").as_deref()).cache
}

/// Service + CLI log files (Console.app-visible on macOS).
pub fn logs_dir() -> PathBuf {
    resolve(get_var("VAK_HOME").as_deref()).logs
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
    "vak"
}

fn base_home() -> PathBuf {
    let environment_home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    #[allow(deprecated)]
    let account_home = std::env::home_dir();
    resolve_base_home(environment_home.map(PathBuf::from), account_home)
}

fn resolve_base_home(environment_home: Option<PathBuf>, account_home: Option<PathBuf>) -> PathBuf {
    environment_home
        .filter(|path| path.is_absolute())
        .or_else(|| account_home.filter(|path| path.is_absolute()))
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(target_os = "macos")]
const LEGACY_HOME_SUFFIX: &str = ".vak";

/// One-time migration of the pre-0.8 dotdir layout into the canonical
/// Library/XDG locations. Renames `~/.vak` → data home (atomic when
/// both sit under $HOME), then relocates rebuildable store artifacts to
/// the cache home. Skipped entirely when VAK_HOME overrides the
/// location, when the legacy dir is absent, or when the target already
/// exists. Never deletes data: if any step fails the legacy tree stays
/// put and the caller surfaces a warning.
pub fn migrate_legacy_home() -> Result<(), String> {
    if get_var("VAK_HOME").is_some() {
        return Ok(());
    }
    let legacy = base_home().join(LEGACY_HOME_SUFFIX);
    if !legacy.exists() {
        return Ok(());
    }
    let homes = resolve(None);
    let target = &homes.data;
    if target.exists() {
        // Both exist: leave everything alone rather than guess a merge
        // order. Surface loudly instead.
        return Err(format!(
            "both {} and {} exist; resolve manually (move or delete the legacy dir)",
            legacy.display(),
            target.display()
        ));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&legacy, target).map_err(|e| format!("rename {}: {e}", legacy.display()))?;

    // Relocate rebuildable index files to the cache home.
    let cache = &homes.cache;
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    for name in ["store.db", "store.db-wal", "store.db-shm"] {
        let from = target.join(name);
        if from.exists()
            && let Err(e) = std::fs::rename(&from, cache.join(name))
        {
            return Err(format!("relocate {name}: {e}"));
        }
    }

    // Relocate service logs so Console.app keeps seeing them.
    let logs = &homes.logs;
    let old_logs = target.join("logs");
    if old_logs.is_dir() {
        std::fs::create_dir_all(logs).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(&old_logs)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let dest = logs.join(entry.file_name());
            if !dest.exists()
                && let Err(e) = std::fs::rename(entry.path(), &dest)
            {
                return Err(format!("relocate log {}: {e}", entry.path().display()));
            }
        }
        let _ = std::fs::remove_dir(&old_logs);
    }
    Ok(())
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
            assert_eq!(h.data, base.join("Library/Application Support/vak"));
            assert_eq!(h.cache, base.join("Library/Caches/vak"));
            assert_eq!(h.logs, base.join("Library/Logs/vak"));
        }
        // Invariants that hold on every platform:
        assert!(
            !h.cache.starts_with(&h.data),
            "cache must not nest inside data"
        );
        for p in [&h.data, &h.cache, &h.logs] {
            assert!(
                !p.to_string_lossy().contains("/.vak"),
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

    #[test]
    fn missing_environment_home_uses_absolute_account_home() {
        assert_eq!(
            resolve_base_home(None, Some(PathBuf::from("/Users/example"))),
            PathBuf::from("/Users/example")
        );
        assert_eq!(
            resolve_base_home(Some(PathBuf::from("relative")), None),
            PathBuf::from("/")
        );
    }
}
