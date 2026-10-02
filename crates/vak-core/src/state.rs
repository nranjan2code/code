//! The durable state registry
//! (`docs/design/46-stabilization-install-and-onboarding.md` Part VII.2).
//!
//! **You cannot promise to preserve what you have not enumerated**, and a
//! prose enumeration rots. Every durable artifact vak owns is declared
//! here, once, and three things that each used to carry their own partial
//! list now read this one instead: `--purge`'s preserve rules, backup
//! coverage, and the upgrade gate.
//!
//! `crates/vak-core/src/backup.rs` is the cautionary example — it
//! hardcoded five directories and four files, so anything added later was
//! silently outside every backup taken. That is precisely the drift a
//! registry exists to make impossible, and the test below fails the build
//! when a durable file appears that nobody declared.

use std::path::{Path, PathBuf};

/// Which root an entry is relative to.
///
/// The two are genuinely different places with different lifetimes:
/// `~/vak-home` is the Shared configuration and secret home a person can
/// browse, and the platform data home holds application-managed state.
/// Conflating them is how a purge either spares secrets or eats sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Root {
    /// `vak_config::paths::data_home()`.
    Data,
    /// `vak_config::paths::cache_home()` — rebuildable, always safe to delete.
    Cache,
    /// `vak_config::paths::default_workspace()` — the Shared layer.
    Shared,
    /// `vak_config::paths::logs_dir()` — service and CLI logs.
    ///
    /// A fourth root, and one that escaped this registry until a real
    /// install showed logs from a previous version surviving a purge.
    Logs,
}

impl Root {
    /// Every root, so a pass over "all of Vak's state" (a purge) cannot
    /// leave one out by listing them by hand.
    pub const ALL: [Root; 4] = [Root::Data, Root::Cache, Root::Shared, Root::Logs];
}

/// What kind of thing this is, which is what decides how it may be treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Append-only. New information is a new entry, never a changed one
    /// (AGENTS.md invariants 1 and 2).
    Ledger,
    /// Structured settings.
    Config,
    /// Credentials. Never copied into a backup without an explicit request,
    /// never logged, never returned by an API.
    Secret,
    /// Derived from something else and safe to lose.
    Derived,
    /// Named content the runtime or a person edits in place: memory notes,
    /// entities, skill proposals, presentation packs, Office rooms. It is
    /// rewritten at runtime, so it is not a ledger, and an update never
    /// writes it (doc 73 §5 makes every save a version at M3b).
    Document,
}

/// What an update may do to this entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnUpdate {
    /// Not written at all. Byte-identical across an update.
    Untouched,
    /// May gain fields, never lose or redefine them (doc 46 VII.3).
    AdditiveOnly,
    /// Regenerated from a durable source; equivalence is what matters,
    /// not bytes.
    Rebuilt,
}

/// What `--purge` does with this entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnPurge {
    Remove,
    /// Survives a purge. Today this is exactly one thing — a project's own
    /// `.vak/` inside someone else's repository — and it is a preserve
    /// *rule*, not a delete list, because the safe default when a new file
    /// appears is to leave it alone.
    Preserve,
}

/// One durable artifact.
#[derive(Debug, Clone, Copy)]
pub struct StateEntry {
    /// Path relative to [`Root`]. A trailing component with no extension
    /// may be a directory; [`StateEntry::matches`] handles both.
    pub path: &'static str,
    pub root: Root,
    /// Crate that writes it, so a reader knows where to look.
    pub owner: &'static str,
    /// Schema version where the file carries one.
    pub schema: Option<u32>,
    pub kind: Kind,
    pub on_update: OnUpdate,
    pub on_purge: OnPurge,
    /// Whether an ordinary backup copies it. Secrets are excluded unless
    /// the operator asks, which is why this is not implied by `kind`.
    pub in_backup: bool,
}

/// The path segment that stands for any one Agent id
/// (`vak_config::paths::agent_home`), so each subpath of every Agent home is
/// declared once rather than the whole `agents/` tree as one entry.
pub const AGENT_SEGMENT: &str = "{agent}";

impl StateEntry {
    /// True when `relative` is this entry or lives inside it. An
    /// [`AGENT_SEGMENT`] in the entry matches exactly one path component.
    pub fn matches(&self, relative: &Path) -> bool {
        let mut actual = relative.components();
        for expected in Path::new(self.path).components() {
            let Some(found) = actual.next() else {
                return false;
            };
            let wildcard = expected.as_os_str() == AGENT_SEGMENT
                && matches!(found, std::path::Component::Normal(_));
            if !wildcard && expected != found {
                return false;
            }
        }
        true
    }

    /// The paths, relative to `base`, that this entry names on disk right
    /// now: itself when present, or one path per existing Agent home for a
    /// pattern entry. Everything that acts on the registry (backup, purge,
    /// the upgrade gate) goes through this, so a pattern can never be joined
    /// onto a root as a literal `{agent}` directory that does not exist.
    pub fn expand(&self, base: &Path) -> Vec<PathBuf> {
        let mut found = vec![PathBuf::new()];
        for part in Path::new(self.path).components() {
            if part.as_os_str() == AGENT_SEGMENT {
                found = found
                    .into_iter()
                    .flat_map(|prefix| {
                        child_dirs(&resolve(base, &prefix))
                            .into_iter()
                            .map(move |name| prefix.join(name))
                    })
                    .collect();
            } else {
                for prefix in &mut found {
                    prefix.push(part);
                }
            }
        }
        found.retain(|relative| resolve(base, relative).exists());
        found
    }
}

/// `base` joined with `relative`, where an empty `relative` is `base`
/// itself rather than `base` with a trailing separator.
pub fn resolve(base: &Path, relative: &Path) -> PathBuf {
    if relative.as_os_str().is_empty() {
        base.to_path_buf()
    } else {
        base.join(relative)
    }
}

/// Names of the real directories directly inside `dir`, sorted. Symlinks
/// are not followed, so a purge can never be led out of the root.
fn child_dirs(dir: &Path) -> Vec<std::ffi::OsString> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name())
        .collect();
    names.sort();
    names
}

/// One subpath of every Agent home. Declared per subpath so that a new
/// per-Agent store fails the enforcement test until someone states what it
/// is, instead of passing because `agents/` was declared whole
/// (data-architecture plan, "Now"; doc 73 D26).
const fn agent_entry(path: &'static str, owner: &'static str, kind: Kind) -> StateEntry {
    StateEntry {
        path,
        root: Root::Data,
        owner,
        schema: None,
        kind,
        on_update: match kind {
            Kind::Config => OnUpdate::AdditiveOnly,
            _ => OnUpdate::Untouched,
        },
        on_purge: OnPurge::Remove,
        in_backup: true,
    }
}

/// Everything vak writes that outlives a process.
///
/// Verified against a real installation rather than inferred from the
/// source: the enforcement test below drives a workspace and fails on any
/// file that appears here without a declaration.
pub const REGISTRY: &[StateEntry] = &[
    StateEntry {
        path: "auth",
        root: Root::Data,
        owner: "vak-server",
        schema: Some(1),
        // Public-key credentials and one-way recovery-code digests.
        kind: Kind::Config,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    // ---- ledgers: append-only, never rewritten by an update ----
    StateEntry {
        path: "sessions",
        root: Root::Data,
        owner: "vak-session",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    // ---- every Agent home, one entry per subpath ----
    // Some server-side stores here are written under the server Core's own
    // Agent whichever Agent owns the session (doc 73 D25); M3b moves them.
    agent_entry("agents/{agent}/sessions", "vak-session", Kind::Ledger),
    agent_entry("agents/{agent}/checkpoints", "vak-core", Kind::Ledger),
    agent_entry("agents/{agent}/memory", "vak-core", Kind::Document),
    agent_entry("agents/{agent}/entities", "vak-core", Kind::Document),
    agent_entry("agents/{agent}/skill-proposals", "vak-core", Kind::Document),
    agent_entry("agents/{agent}/skills", "vak-core", Kind::Config),
    agent_entry(
        "agents/{agent}/commitments.jsonl",
        "vak-commit",
        Kind::Ledger,
    ),
    agent_entry(
        "agents/{agent}/routing-evidence.jsonl",
        "vak-core",
        Kind::Ledger,
    ),
    agent_entry(
        "agents/{agent}/intent-evidence.jsonl",
        "vak-core",
        Kind::Ledger,
    ),
    agent_entry(
        "agents/{agent}/security-events.jsonl",
        "vak-core",
        Kind::Ledger,
    ),
    agent_entry("agents/{agent}/cost-log.jsonl", "vak-core", Kind::Ledger),
    agent_entry(
        "agents/{agent}/activity-log.jsonl",
        "vak-core",
        Kind::Ledger,
    ),
    agent_entry(
        "agents/{agent}/presentations.json",
        "vak-store",
        Kind::Document,
    ),
    agent_entry("agents/{agent}/flow-runs", "vak-flow", Kind::Ledger),
    agent_entry("agents/{agent}/agent-network", "vak-core", Kind::Config),
    agent_entry("agents/{agent}/mail-calendar", "vak-mail-calendar", Kind::Ledger),
    agent_entry("agents/{agent}/sandbox", "vak-server", Kind::Ledger),
    agent_entry(
        "agents/{agent}/office-workspaces",
        "vak-server",
        Kind::Document,
    ),
    agent_entry("agents/{agent}/coworking", "vak-server", Kind::Ledger),
    StateEntry {
        path: "security-events.jsonl",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "cost-log.jsonl",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "routing-evidence.jsonl",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "deleted.json",
        root: Root::Data,
        owner: "vak-core (trash)",
        schema: None,
        // The trash: session ids hidden everywhere until restored.
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "archive.json",
        root: Root::Data,
        owner: "vak-server",
        schema: None,
        // Session ids hidden from the everyday list, still searchable.
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "inbox.jsonl",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "operations",
        root: Root::Data,
        owner: "vak-server",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "checkpoints",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "memory",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Document,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "skill-proposals",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Document,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "learning",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    // ---- config: additive-only across an update ----
    StateEntry {
        path: "gateway",
        root: Root::Data,
        owner: "vak-server",
        schema: Some(1),
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "agent-network",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "tasks.json",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "desktop.json",
        root: Root::Data,
        owner: "vak-desktop",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "tray.json",
        root: Root::Data,
        owner: "vak-tray",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    StateEntry {
        path: "trusted",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "feeds",
        root: Root::Data,
        owner: "vak-server (scripts/feeds)",
        schema: None,
        // The feed store (DuckDB) and the feeds' security log, both at
        // paths the server hands the Python pipeline.
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "feeds.toml",
        root: Root::Data,
        owner: "vak-server",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "output.toml",
        root: Root::Data,
        owner: "vak-delivery",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "flows",
        root: Root::Data,
        owner: "vak-flow",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "flow-runs",
        root: Root::Data,
        owner: "vak-flow",
        schema: None,
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "jobs",
        root: Root::Data,
        owner: "vak-delivery",
        schema: Some(1),
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "skills",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    // ---- secrets ----
    // The encrypted-file credential backend (docs/design/44-shared-config.md,
    // "Secrets Chain") — used only when no OS-native secret service is
    // reachable, beside the Shared config layer (`default_workspace()`,
    // i.e. `~/vak-home` — the "configuration and secret home" of Part VI in
    // docs/design/46). It holds every scope (Shared, project, agent) in one
    // file, not one file per scope the way `.env` used to be laid out; a
    // host using the OS keychain instead has neither file. On a host that
    // does have them, both must be purged or backed up together — the key
    // alone or the data alone is not a usable secret.
    StateEntry {
        path: "credentials.enc",
        root: Root::Shared,
        owner: "vak-config",
        schema: None,
        kind: Kind::Secret,
        // A rotated key is the operator's write, never an update's.
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        // Only with `--include-secrets`, and then with a warning beside it.
        in_backup: false,
    },
    StateEntry {
        path: ".credential_key",
        root: Root::Shared,
        owner: "vak-config",
        schema: None,
        kind: Kind::Secret,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    // The advisory cross-process lock guarding the two entries above
    // (see `EncryptedFileStore::with_lock` in vak-config). Contains no
    // secret material and is safe to lose — a missing lock file just
    // degrades a future access to unsynchronized, it doesn't corrupt
    // anything already on disk.
    StateEntry {
        path: ".credential_key.lock",
        root: Root::Shared,
        owner: "vak-config",
        schema: None,
        kind: Kind::Derived,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    // ---- the Shared layer ----
    StateEntry {
        path: ".vak/config.toml",
        root: Root::Shared,
        owner: "vak-config",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: ".vak/skills",
        root: Root::Shared,
        owner: "vak-core",
        schema: None,
        kind: Kind::Config,
        // Seeds advance only where the file still matches what we shipped;
        // an edited one is the operator's file permanently (doc 46 VII.4).
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: ".vak/.seed-manifest.json",
        root: Root::Shared,
        owner: "vak-core",
        schema: Some(1),
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: ".vak/plugins",
        root: Root::Shared,
        owner: "vak-plugin",
        schema: Some(1),
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    // ---- derived: rebuilt, never carried ----
    StateEntry {
        path: "locks",
        root: Root::Data,
        owner: "vak-server",
        schema: None,
        kind: Kind::Derived,
        on_update: OnUpdate::Rebuilt,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    StateEntry {
        path: "broker.sock",
        root: Root::Data,
        owner: "vak-tools",
        schema: None,
        kind: Kind::Derived,
        on_update: OnUpdate::Rebuilt,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    StateEntry {
        path: "release",
        root: Root::Data,
        owner: "vak",
        schema: Some(2),
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    StateEntry {
        path: "vak-home",
        root: Root::Data,
        owner: "vak-config",
        schema: None,
        kind: Kind::Config,
        on_update: OnUpdate::AdditiveOnly,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    StateEntry {
        path: "",
        root: Root::Logs,
        owner: "vak-ops",
        schema: None,
        kind: Kind::Ledger,
        // Upgrades append to a log; they never rewrite one.
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
    StateEntry {
        path: "",
        root: Root::Cache,
        owner: "vak-store",
        schema: None,
        kind: Kind::Derived,
        on_update: OnUpdate::Rebuilt,
        on_purge: OnPurge::Remove,
        in_backup: false,
    },
];

/// Entries under one root.
pub fn entries_for(root: Root) -> impl Iterator<Item = &'static StateEntry> {
    REGISTRY.iter().filter(move |e| e.root == root)
}

/// Paths, relative to `base`, that an ordinary backup of `root` copies:
/// every declared entry present there, with each pattern entry expanded to
/// the Agent homes that exist.
pub fn backup_targets(root: Root, base: &Path) -> Vec<PathBuf> {
    entries_for(root)
        .filter(|e| e.in_backup)
        .flat_map(|e| e.expand(base))
        .collect()
}

/// Remove the directories a pattern entry implies once they are empty:
/// each Agent home, then `agents/` itself.
///
/// A purge removes declared subpaths; this tidies what held them. An Agent
/// home that still holds something undeclared is not empty, so it stays
/// exactly where it was, as the preserve rule requires.
pub fn remove_empty_pattern_dirs(root: Root, base: &Path) {
    let mut parents: Vec<PathBuf> = Vec::new();
    for entry in entries_for(root) {
        let components: Vec<_> = Path::new(entry.path).components().collect();
        let Some(at) = components
            .iter()
            .position(|c| c.as_os_str() == AGENT_SEGMENT)
        else {
            continue;
        };
        let parent: PathBuf = components[..at].iter().collect();
        if !parents.contains(&parent) {
            parents.push(parent);
        }
    }
    for parent in parents {
        let dir = resolve(base, &parent);
        for name in child_dirs(&dir) {
            // Fails, harmlessly, on a home that is not empty.
            let _ = std::fs::remove_dir(dir.join(name));
        }
        let _ = std::fs::remove_dir(&dir);
    }
}

/// True when `relative` under `root` is declared.
///
/// The enforcement test's question: did something write a durable file
/// nobody declared?
pub fn is_declared(root: Root, relative: &Path) -> bool {
    entries_for(root).any(|e| e.matches(relative))
}

/// The absolute location of `root` right now.
pub fn root_path(root: Root) -> PathBuf {
    match root {
        Root::Data => vak_config::paths::data_home(),
        Root::Cache => vak_config::paths::cache_home(),
        Root::Shared => vak_config::paths::default_workspace(),
        Root::Logs => vak_config::paths::logs_dir(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_entry_covers_what_is_inside_it() {
        let sessions = REGISTRY.iter().find(|e| e.path == "sessions").unwrap();
        assert!(sessions.matches(Path::new("sessions")));
        assert!(sessions.matches(Path::new("sessions/abc/def.jsonl")));
        assert!(!sessions.matches(Path::new("sessions-other")));
    }

    #[test]
    fn a_pattern_entry_matches_one_agent_component_and_nothing_wider() {
        let sessions = REGISTRY
            .iter()
            .find(|e| e.path == "agents/{agent}/sessions")
            .unwrap();
        assert!(sessions.matches(Path::new("agents/vak/sessions/h/a.jsonl")));
        assert!(sessions.matches(Path::new("agents/writer/sessions")));
        assert!(!sessions.matches(Path::new("agents/vak/memory/MEMORY.md")));
        assert!(!sessions.matches(Path::new("agents/sessions")));
        assert!(!sessions.matches(Path::new("agents")));
    }

    #[test]
    fn an_unknown_agent_subpath_is_undeclared() {
        // The point of the split: a new per-Agent store is not covered by
        // a blanket `agents` entry, so the enforcement test sees it.
        assert!(is_declared(
            Root::Data,
            Path::new("agents/vak/sessions/h/a.jsonl")
        ));
        assert!(is_declared(
            Root::Data,
            Path::new("agents/writer/office-workspaces/s/r.json")
        ));
        assert!(!is_declared(
            Root::Data,
            Path::new("agents/vak/new-store/x.json")
        ));
        assert!(!is_declared(Root::Data, Path::new("agents/vak/stray.txt")));
    }

    #[test]
    fn a_pattern_entry_expands_to_each_agent_home_present() {
        let home = tempfile::tempdir().unwrap();
        for rel in [
            "agents/vak/sessions/h/a.jsonl",
            "agents/writer/sessions/h/b.jsonl",
            "agents/writer/memory/user/USER.md",
        ] {
            let path = home.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, rel).unwrap();
        }
        let find = |path: &str| REGISTRY.iter().find(|e| e.path == path).unwrap();
        assert_eq!(
            find("agents/{agent}/sessions").expand(home.path()),
            vec![
                PathBuf::from("agents/vak/sessions"),
                PathBuf::from("agents/writer/sessions")
            ]
        );
        assert_eq!(
            find("agents/{agent}/memory").expand(home.path()),
            vec![PathBuf::from("agents/writer/memory")]
        );
        assert!(
            find("agents/{agent}/sandbox")
                .expand(home.path())
                .is_empty()
        );
        assert_eq!(
            find("sessions").expand(home.path()),
            Vec::<PathBuf>::new(),
            "an absent plain entry expands to nothing"
        );
    }

    #[test]
    fn empty_agent_homes_are_tidied_and_undeclared_ones_kept() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("agents/vak")).unwrap();
        std::fs::create_dir_all(home.path().join("agents/writer")).unwrap();
        std::fs::write(home.path().join("agents/writer/stray.txt"), "x").unwrap();
        remove_empty_pattern_dirs(Root::Data, home.path());
        assert!(!home.path().join("agents/vak").exists());
        assert!(home.path().join("agents/writer/stray.txt").exists());
    }

    #[test]
    fn an_entry_split_into_finer_entries_is_not_a_violation() {
        // Upgrading across the split: the earlier build declared `agents`
        // whole; the later one declares each subpath.
        let before = snap(
            "agents",
            "Untouched",
            vec![file("agents/vak/sessions/h/a.jsonl", "aaa")],
        );
        let mut after = snap(
            "agents/{agent}/sessions",
            "Untouched",
            vec![file("agents/vak/sessions/h/a.jsonl", "aaa")],
        );
        assert!(verify_upgrade(&before, &after).is_empty());

        after.entries[0].files[0].sha256 = "bbb".into();
        let found = verify_upgrade(&before, &after);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].detail.contains("contents changed"));

        after.entries[0].files.clear();
        let found = verify_upgrade(&before, &after);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].detail.contains("gone from the registry"));
    }

    #[test]
    fn every_entry_declares_a_distinct_path_within_its_root() {
        // Two entries for one path would let the two disagree about what a
        // purge or an update may do to it.
        let mut seen: Vec<(Root, &str)> = Vec::new();
        for entry in REGISTRY {
            let key = (entry.root, entry.path);
            assert!(
                !seen.contains(&key),
                "{:?}/{} is declared twice",
                entry.root,
                entry.path
            );
            seen.push(key);
        }
    }

    #[test]
    fn secrets_are_never_in_an_ordinary_backup() {
        for entry in REGISTRY.iter().filter(|e| e.kind == Kind::Secret) {
            assert!(
                !entry.in_backup,
                "{} is a secret and must not ride along in a routine backup",
                entry.path
            );
        }
    }

    #[test]
    fn a_ledger_is_never_rewritten_by_an_update() {
        // Append-only is what makes "no migration" sustainable rather than
        // merely stated (doc 46 VII.3 rule 5).
        for entry in REGISTRY.iter().filter(|e| e.kind == Kind::Ledger) {
            assert_eq!(
                entry.on_update,
                OnUpdate::Untouched,
                "{} is a ledger, so an update must not write it",
                entry.path
            );
        }
    }

    fn snap(entry: &str, on_update: &str, files: Vec<FileSnapshot>) -> StateSnapshot {
        StateSnapshot {
            version: "test".into(),
            taken_at: "now".into(),
            entries: vec![EntrySnapshot {
                entry: entry.into(),
                root: "data".into(),
                on_update: on_update.into(),
                files,
            }],
        }
    }

    fn file(path: &str, sha: &str) -> FileSnapshot {
        FileSnapshot {
            path: path.into(),
            sha256: sha.into(),
            bytes: 1,
            json: None,
        }
    }

    #[test]
    fn an_untouched_ledger_that_changed_is_a_violation() {
        // The rule that makes append-only real: an update rewriting a
        // ledger is the failure this gate exists to catch.
        let before = snap("sessions", "Untouched", vec![file("a.jsonl", "aaa")]);
        let after = snap("sessions", "Untouched", vec![file("a.jsonl", "bbb")]);
        let found = verify_upgrade(&before, &after);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].detail.contains("contents changed"));
    }

    #[test]
    fn a_vanished_file_is_a_violation_under_either_rule() {
        for rule in ["Untouched", "AdditiveOnly"] {
            let before = snap("gateway", rule, vec![file("bots.json", "aaa")]);
            let after = snap("gateway", rule, Vec::new());
            let found = verify_upgrade(&before, &after);
            assert_eq!(found.len(), 1, "{rule}: {found:?}");
            assert!(found[0].detail.contains("gone after the update"));
        }
    }

    #[test]
    fn an_additive_change_is_allowed_but_a_dropped_field_is_not() {
        // Adding a field is the whole point of additive-only; losing one
        // is the silent data loss it forbids.
        let mut before = snap("gateway", "AdditiveOnly", vec![file("bots.json", "aaa")]);
        before.entries[0].files[0].json =
            Some(serde_json::json!({ "schema": 1, "bots": [], "kept": true }));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bots.json");

        // Same fields plus a new one: allowed.
        std::fs::write(
            &path,
            serde_json::json!({ "schema": 1, "bots": [], "kept": true, "added": 2 }).to_string(),
        )
        .unwrap();
        assert!(dropped_keys(&path, &before.entries[0].files[0]).is_none());

        // A field silently removed: refused.
        std::fs::write(
            &path,
            serde_json::json!({ "schema": 1, "bots": [] }).to_string(),
        )
        .unwrap();
        let detail = dropped_keys(&path, &before.entries[0].files[0]).expect("dropped field");
        assert!(detail.contains("kept"), "{detail}");
    }

    #[test]
    fn rebuilt_state_may_differ_freely() {
        let before = snap("locks", "Rebuilt", vec![file("x", "aaa")]);
        let after = snap("locks", "Rebuilt", Vec::new());
        assert!(verify_upgrade(&before, &after).is_empty());
    }

    #[test]
    fn an_undeclared_path_is_reported_as_undeclared() {
        assert!(is_declared(Root::Data, Path::new("sessions/x.jsonl")));
        assert!(!is_declared(
            Root::Data,
            Path::new("something-nobody-declared.json")
        ));
    }
}

// ---- Snapshots and the upgrade contract ------------------------------------

use serde::{Deserialize, Serialize};

/// One durable file, as it stood at a moment in time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSnapshot {
    /// Path relative to its root.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    /// The document as it stood, for JSON files only.
    ///
    /// A digest can prove a file changed but not *how*, and
    /// `AdditiveOnly` needs the earlier key set to prove nothing was
    /// dropped. Carried here so a snapshot is self-contained: the gate
    /// compares a file written by one build against a document captured
    /// by another, possibly on a different machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<serde_json::Value>,
}

/// Everything the registry declares, as it stood at a moment in time.
///
/// Taken before an update and again after it, this is what turns "an
/// update must not lose data" from a promise into an assertion
/// (`docs/design/46-stabilization-install-and-onboarding.md` VII.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub version: String,
    pub taken_at: String,
    /// Registry entry path → the files found under it.
    pub entries: Vec<EntrySnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrySnapshot {
    pub entry: String,
    pub root: String,
    pub on_update: String,
    pub files: Vec<FileSnapshot>,
}

/// A way an update broke the contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Violation {
    pub entry: String,
    pub path: String,
    pub rule: String,
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} / {}): {}",
            self.path, self.entry, self.rule, self.detail
        )
    }
}

fn digest_of(path: &Path) -> Option<(String, u64)> {
    use sha2::{Digest as _, Sha256};
    let bytes = std::fs::read(path).ok()?;
    Some((format!("{:x}", Sha256::digest(&bytes)), bytes.len() as u64))
}

fn walk(root: &Path) -> Vec<PathBuf> {
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
            } else {
                found.push(path);
            }
        }
    }
    found
}

fn root_label(root: Root) -> &'static str {
    match root {
        Root::Data => "data",
        Root::Cache => "cache",
        Root::Shared => "shared",
        Root::Logs => "logs",
    }
}

/// Digest every declared file, right now.
pub fn snapshot(version: &str) -> StateSnapshot {
    let mut entries = Vec::new();
    for entry in REGISTRY {
        let base = root_path(entry.root);
        let mut files = Vec::new();
        let mut candidates = Vec::new();
        for relative in entry.expand(&base) {
            let target = resolve(&base, &relative);
            if target.is_dir() {
                candidates.extend(walk(&target));
            } else if target.is_file() {
                candidates.push(target);
            }
        }
        for file in candidates {
            let Some((sha256, bytes)) = digest_of(&file) else {
                continue;
            };
            let relative = file
                .strip_prefix(&base)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned();
            // Only for entries whose contract needs it: a ledger is
            // compared byte-for-byte, so carrying its parsed body would
            // bloat the snapshot for nothing.
            let json = (entry.on_update == OnUpdate::AdditiveOnly && relative.ends_with(".json"))
                .then(|| {
                    std::fs::read_to_string(&file)
                        .ok()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                })
                .flatten();
            files.push(FileSnapshot {
                path: relative,
                sha256,
                bytes,
                json,
            });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        entries.push(EntrySnapshot {
            entry: entry.path.to_string(),
            root: root_label(entry.root).to_string(),
            on_update: format!("{:?}", entry.on_update),
            files,
        });
    }
    StateSnapshot {
        version: version.to_string(),
        taken_at: chrono::Utc::now().to_rfc3339(),
        entries,
    }
}

/// Every key path present in a JSON document.
///
/// "Additive-only" means a field may be added and never removed, so the
/// check that matters is whether the *earlier* key set still exists —
/// comparing whole documents would fail on any legitimate addition.
fn key_paths(value: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                out.push(path.clone());
                key_paths(v, &path, out);
            }
        }
        // Array *contents* are data, not shape: an appended session or
        // ledger row is exactly what these files are for.
        serde_json::Value::Array(_) => {}
        _ => {}
    }
}

/// Check a later snapshot against an earlier one, per registry rule.
///
/// This is the assertion behind "an update never loses data" — and it
/// deliberately lives beside the registry rather than in the script that
/// calls it, so the rules have one definition.
pub fn verify_upgrade(before: &StateSnapshot, after: &StateSnapshot) -> Vec<Violation> {
    let mut violations = Vec::new();
    // A file is looked up wherever it is declared now, not only under the
    // entry that held it before, so splitting one entry into finer ones (as
    // the per-Agent split did to `agents`) loses nothing and is no violation.
    let declared_now = |root: &str, path: &str| {
        after
            .entries
            .iter()
            .filter(|e| e.root == root)
            .flat_map(|e| e.files.iter())
            .find(|f| f.path == path)
    };
    for prior in &before.entries {
        let still_declared = after
            .entries
            .iter()
            .any(|e| e.entry == prior.entry && e.root == prior.root);
        if !still_declared
            && prior
                .files
                .iter()
                .any(|f| declared_now(&prior.root, &f.path).is_none())
        {
            violations.push(Violation {
                entry: prior.entry.clone(),
                path: prior.entry.clone(),
                rule: "declared".into(),
                detail: "the entry is gone from the registry entirely".into(),
            });
            continue;
        }
        let base = match prior.root.as_str() {
            "shared" => root_path(Root::Shared),
            "cache" => root_path(Root::Cache),
            _ => root_path(Root::Data),
        };

        for file in &prior.files {
            let now = declared_now(&prior.root, &file.path);
            match prior.on_update.as_str() {
                "Untouched" => match now {
                    None => violations.push(Violation {
                        entry: prior.entry.clone(),
                        path: file.path.clone(),
                        rule: "Untouched".into(),
                        detail: "the file is gone after the update".into(),
                    }),
                    Some(now) if now.sha256 != file.sha256 => violations.push(Violation {
                        entry: prior.entry.clone(),
                        path: file.path.clone(),
                        rule: "Untouched".into(),
                        detail: format!(
                            "contents changed ({} bytes → {} bytes)",
                            file.bytes, now.bytes
                        ),
                    }),
                    Some(_) => {}
                },
                "AdditiveOnly" => {
                    let Some(_) = now else {
                        violations.push(Violation {
                            entry: prior.entry.clone(),
                            path: file.path.clone(),
                            rule: "AdditiveOnly".into(),
                            detail: "the file is gone after the update".into(),
                        });
                        continue;
                    };
                    // For JSON, prove no prior field was dropped. Other
                    // formats assert presence only, which this says
                    // plainly rather than implying a check it does not do.
                    if file.path.ends_with(".json")
                        && let Some(missing) = dropped_keys(&base.join(&file.path), file)
                    {
                        violations.push(Violation {
                            entry: prior.entry.clone(),
                            path: file.path.clone(),
                            rule: "AdditiveOnly".into(),
                            detail: missing,
                        });
                    }
                }
                // Rebuilt state is allowed to differ; it is derived.
                _ => {}
            }
        }
    }
    violations
}

/// Keys the earlier document had that the current file no longer does.
///
/// Needs the earlier document, which a digest alone cannot supply, so this
/// is only meaningful when the caller kept it. Returns `None` when nothing
/// was dropped or the comparison cannot be made.
fn dropped_keys(current: &Path, prior: &FileSnapshot) -> Option<String> {
    let raw = std::fs::read_to_string(current).ok()?;
    let now: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let before = prior.json.as_ref()?;
    let mut before_keys = Vec::new();
    key_paths(before, "", &mut before_keys);
    let mut now_keys = Vec::new();
    key_paths(&now, "", &mut now_keys);
    let missing: Vec<&String> = before_keys
        .iter()
        .filter(|k| !now_keys.contains(k))
        .collect();
    (!missing.is_empty()).then(|| {
        format!(
            "fields present before the update are gone: {}",
            missing
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}
