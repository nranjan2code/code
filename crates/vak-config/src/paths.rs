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
//! `VAK_HOME` overrides the DATA home everywhere — an explicit override
//! is the user's layout choice and yields a self-contained tree.

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

/// The canonical **project workspace** a fresh install brings up its
/// durable services against — `~/vak-home`, a plain directory a person
/// can `cd` into, distinct from `data_home()` (which holds sessions,
/// config, and other application-managed state, not something a user
/// browses or edits directly).
///
/// This exists because nothing previously named a workspace at install
/// time: `self install` placed binaries, and `self services-sync`
/// captured whatever directory it happened to be run from as the
/// gateway's workspace (docs/design/32 invariant 3) — correct as a
/// mechanism, but with no answer to "which directory" until an operator
/// picked one. A real incident: services-sync was run from inside the
/// vak *source checkout* while developing it, silently binding an
/// always-on Telegram bridge to the tool's own dev repo. `self install`
/// now creates this directory and syncs services against it on a truly
/// fresh install (no prior units), so there is always a sane, isolated
/// default — never the workspace a person happened to be standing in.
pub fn default_workspace() -> PathBuf {
    resolve(get_var("VAK_HOME").as_deref()).workspace
}

/// The gateway's persisted workspace selection. An absent or malformed
/// selection intentionally falls back to the canonical default workspace.
pub fn gateway_workspace() -> PathBuf {
    let homes = resolve(get_var("VAK_HOME").as_deref());
    gateway_workspace_at(&homes.data, &homes.workspace)
}

/// Per-agent data home: sessions, memory, and agent-specific config.
/// Under data_home()/agents/<agent_id>/
pub fn agent_home(agent_id: &str) -> PathBuf {
    agent_home_at(&data_home(), agent_id)
}

/// Per-agent data home resolved from an explicit data home root.
pub fn agent_home_at(data: &std::path::Path, agent_id: &str) -> PathBuf {
    data.join("agents").join(agent_id)
}

/// Per-agent project workspace (the directory file/shell tools operate in),
/// distinct from `agent_home` (which holds sessions/memory). The built-in
/// "vak" agent keeps using the base workspace so existing single-agent
/// installs see no path change; every other agent gets an isolated
/// subdirectory nested *under that same base workspace*, so agents never see
/// each other's files, while the same agent id opened against two different
/// base workspaces (e.g. two server instances, or a gateway channel pointed
/// at a different project) still resolves to two independently isolated
/// workspaces rather than one shared global directory keyed on agent id
/// alone.
pub fn agent_workspace(base: &std::path::Path, agent_id: &str) -> PathBuf {
    if agent_id == "vak" {
        base.to_path_buf()
    } else {
        base.join(".vak/agents").join(agent_id).join("workspace")
    }
}

/// Resolve the gateway workspace from an explicit data home. This variant
/// keeps server tests isolated when a `Core` uses a temporary sessions home.
pub fn gateway_workspace_at(data: &std::path::Path, default: &std::path::Path) -> PathBuf {
    let path = data.join("gateway/default-workspace");
    std::fs::read_to_string(path)
        .ok()
        .map(|raw| PathBuf::from(raw.trim()))
        .filter(|workspace| workspace.is_absolute() && workspace.is_dir())
        .unwrap_or_else(|| default.to_path_buf())
}

/// Persist or clear the user-selected gateway workspace atomically.
pub fn persist_gateway_workspace_at(
    data: &std::path::Path,
    workspace: Option<&std::path::Path>,
) -> Result<(), std::io::Error> {
    let dir = data.join("gateway");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("default-workspace");
    match workspace {
        Some(workspace) => {
            let temp = dir.join(format!(".default-workspace.{}.tmp", std::process::id()));
            std::fs::write(&temp, format!("{}\n", workspace.display()))?;
            std::fs::rename(temp, path)?;
        }
        None => match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        },
    }
    Ok(())
}

/// Canonical toolchain search directories across standard locations (Homebrew, Cargo, Local, System, version managers).
/// Essential for macOS GUI app bundles and daemon/service processes where shell profile is not evaluated.
pub fn canonical_toolchain_paths() -> Vec<PathBuf> {
    let home = base_home();
    let mut dirs = Vec::new();

    // Homebrew / standard system paths
    for dir in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        let p = PathBuf::from(dir);
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    }

    // User toolchains
    for sub in [
        ".cargo/bin",
        ".local/bin",
        "bin",
        ".local/share/pnpm",
        "Library/pnpm",
        ".local/share/fnm/current/bin",
        ".fnm/current/bin",
        ".asdf/shims",
        ".local/share/mise/shims",
        ".pyenv/shims",
    ] {
        let p = home.join(sub);
        if p.is_dir() && !dirs.contains(&p) {
            dirs.push(p);
        }
    }

    // Scan active node versions in nvm
    let nvm_node = home.join(".nvm/versions/node");
    if nvm_node.is_dir()
        && let Ok(entries) = std::fs::read_dir(&nvm_node)
    {
        for entry in entries.flatten() {
            let bin = entry.path().join("bin");
            if bin.is_dir() && !dirs.contains(&bin) {
                dirs.push(bin);
            }
        }
    }

    dirs
}

/// Computes an augmented PATH value merging the current process PATH with canonical toolchain paths.
pub fn augmented_process_path() -> std::ffi::OsString {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(existing) = std::env::var_os("PATH") {
        parts.extend(std::env::split_paths(&existing).filter(|p| !p.as_os_str().is_empty()));
    }
    for toolchain_path in canonical_toolchain_paths() {
        if !parts.contains(&toolchain_path) {
            parts.push(toolchain_path);
        }
    }
    std::env::join_paths(parts).unwrap_or_default()
}

/// Every home derived from one override decision. Pure so tests can
/// exercise both branches without touching process-global environment,
/// and so a caller needing two of them reads `VAK_HOME` once rather than
/// pairing values from before and after another thread changed it.
struct Homes {
    data: PathBuf,
    cache: PathBuf,
    logs: PathBuf,
    workspace: PathBuf,
}

fn resolve(override_home: Option<&str>) -> Homes {
    let base = base_home();
    match override_home.filter(|s| !s.is_empty()) {
        // An explicit override is a self-contained sandbox: everything
        // nests under it so tests and portable installs stay one tree,
        // including the Shared config's workspace, which would otherwise
        // inherit the operator's real ~/vak-home state.
        Some(h) => {
            let data = PathBuf::from(h);
            Homes {
                cache: data.join("cache"),
                logs: data.join("logs"),
                workspace: data.join("vak-home"),
                data,
            }
        }
        None => Homes {
            workspace: base.join("vak-home"),
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

/// Pin this process's installation home, at the highest precedence.
///
/// `VAK_HOME` is normally read from the environment, which makes it
/// awkward to set from inside a test: `std::env::set_var` is `unsafe`, and
/// `unsafe_code` is denied workspace-wide (AGENTS.md invariant 6). The
/// override map [`crate::set_override`] already sits above the real
/// environment in [`crate::get_var`]'s precedence, so pinning the home is
/// safe and needs no `unsafe` at all.
pub fn set_home_override(path: &std::path::Path) {
    crate::set_override("VAK_HOME", path.to_string_lossy().into_owned());
}

/// Point this process at a private, empty home, and return it.
///
/// **Every test that builds a `Core` must call this.** Without it,
/// `load_with_trust` reads the operator's real Shared layer
/// (`~/vak-home/.vak/config.toml` and the real Shared secret scope), so a
/// personal setting silently changes what the test exercises — a real MCP server
/// gets advertised, `[memory] reflection = true` consumes a scripted
/// provider response, a real provider key makes an "unconfigured" case
/// pass. Tests were reading the developer's machine.
///
/// Idempotent per process: the first call wins and later ones return the
/// same directory, so tests sharing a binary share one empty home. That is
/// the isolation that matters — each test already scopes its own cwd and
/// sessions home.
pub fn isolate_home_for_tests() -> PathBuf {
    static ISOLATED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ISOLATED
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("vak-test-home-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            set_home_override(&dir);
            dir
        })
        .clone()
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
    fn default_workspace_is_a_plain_dir_under_the_account_home_not_data_home() {
        let canonical = resolve(None);
        assert_eq!(canonical.workspace, base_home().join("vak-home"));
        assert_ne!(
            canonical.workspace, canonical.data,
            "the default workspace must never collide with the app's own data home"
        );

        let sandbox = tempfile::tempdir().unwrap();
        let overridden = resolve(Some(sandbox.path().to_str().unwrap()));
        assert_eq!(overridden.workspace, overridden.data.join("vak-home"));
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
        assert_eq!(h.workspace, base_home().join("vak-home"));
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

    #[test]
    fn gateway_workspace_sidecar_defaults_and_round_trips() {
        let data = tempfile::tempdir().unwrap();
        let default = PathBuf::from("/Users/example/vak-home");
        assert_eq!(gateway_workspace_at(data.path(), &default), default);
        let selected = tempfile::tempdir().unwrap();
        persist_gateway_workspace_at(data.path(), Some(selected.path())).unwrap();
        assert_eq!(gateway_workspace_at(data.path(), &default), selected.path());
        persist_gateway_workspace_at(data.path(), None).unwrap();
        assert_eq!(gateway_workspace_at(data.path(), &default), default);
    }

    #[test]
    fn gateway_workspace_ignores_relative_or_missing_sidecars() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("gateway/default-workspace");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "relative\n").unwrap();
        let default = PathBuf::from("/Users/example/vak-home");
        assert_eq!(gateway_workspace_at(data.path(), &default), default);
    }

    #[test]
    fn agent_home_at_resolves_scoped_subdirectory() {
        let data = PathBuf::from("/tmp/vak-test-home");
        assert_eq!(agent_home_at(&data, "vak"), data.join("agents/vak"));
        assert_eq!(
            agent_home_at(&data, "agent-123"),
            data.join("agents/agent-123")
        );
    }

    #[test]
    fn agent_workspace_isolates_user_created_agents_from_each_other() {
        let base = PathBuf::from("/Users/example/vak-home");
        // The built-in agent keeps the process's own base workspace, so
        // existing single-agent installs see no path change.
        assert_eq!(agent_workspace(&base, "vak"), base);
        // Every other agent gets its own isolated directory nested under
        // that same base, distinct from the base itself and each other.
        let a = agent_workspace(&base, "agent-a");
        let b = agent_workspace(&base, "agent-b");
        assert_ne!(a, base);
        assert_ne!(b, base);
        assert_ne!(a, b);
        assert_eq!(a, base.join(".vak/agents/agent-a/workspace"));
    }

    #[test]
    fn agent_workspace_isolates_the_same_agent_id_across_different_base_workspaces() {
        let base_a = PathBuf::from("/Users/example/project-a");
        let base_b = PathBuf::from("/Users/example/project-b");
        assert_ne!(
            agent_workspace(&base_a, "newsy"),
            agent_workspace(&base_b, "newsy"),
            "the same agent id under two different base workspaces must not collide"
        );
    }
}
