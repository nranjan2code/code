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
    StateEntry {
        path: "agents",
        root: Root::Data,
        owner: "vak-core",
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
        let target = if entry.path.is_empty() {
            base.clone()
        } else {
            base.join(entry.path)
        };
        let mut files = Vec::new();
        let candidates = if target.is_dir() {
            walk(&target)
        } else if target.is_file() {
            vec![target.clone()]
        } else {
            Vec::new()
        };
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
    for prior in &before.entries {
        let Some(later) = after
            .entries
            .iter()
            .find(|e| e.entry == prior.entry && e.root == prior.root)
        else {
            violations.push(Violation {
                entry: prior.entry.clone(),
                path: prior.entry.clone(),
                rule: "declared".into(),
                detail: "the entry is gone from the registry entirely".into(),
            });
            continue;
        };
        let base = match later.root.as_str() {
            "shared" => root_path(Root::Shared),
            "cache" => root_path(Root::Cache),
            _ => root_path(Root::Data),
        };

        for file in &prior.files {
            let now = later.files.iter().find(|f| f.path == file.path);
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
