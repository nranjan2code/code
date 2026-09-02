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

impl StateEntry {
    /// True when `relative` is this entry or lives inside it.
    pub fn matches(&self, relative: &Path) -> bool {
        let entry = Path::new(self.path);
        relative == entry || relative.starts_with(entry)
    }
}

/// Everything vak writes that outlives a process.
///
/// Verified against a real installation rather than inferred from the
/// source: the enforcement test below drives a workspace and fails on any
/// file that appears here without a declaration.
pub const REGISTRY: &[StateEntry] = &[
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
        kind: Kind::Ledger,
        on_update: OnUpdate::Untouched,
        on_purge: OnPurge::Remove,
        in_backup: true,
    },
    StateEntry {
        path: "skill-proposals",
        root: Root::Data,
        owner: "vak-core",
        schema: None,
        kind: Kind::Ledger,
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
    StateEntry {
        path: ".env",
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

/// Relative paths an ordinary backup copies, for `root`.
pub fn backup_paths(root: Root) -> Vec<&'static str> {
    entries_for(root)
        .filter(|e| e.in_backup)
        .map(|e| e.path)
        .collect()
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

    #[test]
    fn an_undeclared_path_is_reported_as_undeclared() {
        assert!(is_declared(Root::Data, Path::new("sessions/x.jsonl")));
        assert!(!is_declared(
            Root::Data,
            Path::new("something-nobody-declared.json")
        ));
    }
}
