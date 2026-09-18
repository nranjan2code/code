//! Which workspaces a person wants to *see*, as distinct from which ones
//! exist.
//!
//! Adding a workspace was possible from every surface; removing one was
//! possible from none. A folder opened once stayed in the list forever —
//! including a folder opened by mistake, a folder since deleted, and the
//! `/tmp` scratch directory somebody tried a prompt in.
//!
//! # Forgetting is not deleting
//!
//! This file records a *presentation* decision and nothing else. Forgetting
//! a workspace removes it from the lists a surface shows. It does not touch:
//!
//! - the session ledgers under `<data_home>/sessions/<hash>/`,
//! - memory, checkpoints, receipts, or commitments,
//! - the project's own `.vak/config.toml`, secret scope, or trust decision.
//!
//! So re-adding it later restores everything, which is the property that
//! makes forgetting safe to offer as a one-click action. Deleting a
//! workspace's history is a different, far more consequential operation, and
//! it deliberately does not live behind this door.
//!
//! # One store, every surface
//!
//! The desktop kept its own `recent_projects` in `desktop.json` while the
//! server derived recents from the ledger directories, so the same folder
//! could be listed in one surface and not the other, and "remove" in one
//! would have meant nothing to the other. This is the single source of
//! truth for all of them (AGENTS.md invariant 30).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The on-disk shape. Sets, so repeated adds are idempotent and the file
/// stays stable under diff.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspaces {
    /// Bumped only for a breaking shape change.
    pub schema: u32,
    /// Workspaces a surface may offer, most-recently-opened first. Ordered,
    /// so this is a `Vec` rather than a set.
    pub known: Vec<PathBuf>,
    /// Workspaces the operator asked not to be shown. Kept rather than
    /// simply dropped from `known`, because a workspace is also discoverable
    /// from the ledger on disk — without a durable "no", it would reappear
    /// the moment anything rescanned.
    pub forgotten: BTreeSet<PathBuf>,
}

const SCHEMA: u32 = 1;
const MAX_KNOWN: usize = 24;

fn store_path() -> PathBuf {
    vak_config::paths::data_home().join("workspaces.json")
}

pub fn load() -> Workspaces {
    std::fs::read_to_string(store_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(value: &Workspaces) -> std::io::Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    // Same-directory temp + rename: a torn write here would lose the list,
    // and the list is cheap to rebuild but annoying to lose.
    //
    // The temp name is UNIQUE PER WRITER. A fixed `.json.tmp` is a race
    // between any two writers — and there are always at least two here, the
    // desktop shell and the server, both live at once. Whichever renamed
    // first deleted the other's file out from under it, and the loser
    // failed with a bare ENOENT on a path it had just written. Found by
    // four tests failing in parallel and passing alone, which is what that
    // bug looks like from the outside.
    // Process id AND a per-call counter: two threads in one process share a
    // pid, so a pid-only name still raced (which is exactly how this was
    // found — four tests failing together and passing alone).
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{unique}", std::process::id()));
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Record `path` as opened: newest first, and no longer forgotten.
///
/// The write is atomic — no reader ever sees a torn file — but a
/// read-modify-write racing another writer can still lose an entry. That is
/// deliberate: this is a list of folders to show, the loser's cost is one
/// re-open or one re-forget, and a lock file guarding a recents list would
/// be more machinery than the failure is worth.
///
/// Re-opening is exactly how a forgotten workspace comes back — there is no
/// separate "unforget" verb to discover, because the action a person takes
/// is "open it again".
pub fn remember(path: &Path) -> std::io::Result<()> {
    let path = canonical(path);
    let mut store = load();
    store.schema = SCHEMA;
    store.forgotten.remove(&path);
    store.known.retain(|known| known != &path);
    store.known.insert(0, path);
    store.known.truncate(MAX_KNOWN);
    save(&store)
}

/// Stop showing `path`. Its sessions, memory, and settings are untouched.
pub fn forget(path: &Path) -> std::io::Result<()> {
    let path = canonical(path);
    let mut store = load();
    store.schema = SCHEMA;
    store.known.retain(|known| known != &path);
    store.forgotten.insert(path);
    save(&store)
}

/// Whether `path` has been explicitly forgotten.
pub fn is_forgotten(path: &Path) -> bool {
    load().forgotten.contains(&canonical(path))
}

/// The workspaces a surface should offer.
///
/// `discovered` are paths found by other means (ledger directories, a
/// shell's cwd); they are merged in, minus anything forgotten, so a surface
/// never has to reimplement the filter and accidentally show one back.
pub fn visible(discovered: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let store = load();
    let mut out: Vec<PathBuf> = Vec::new();
    for path in store.known.iter().cloned().chain(discovered) {
        let path = canonical(&path);
        if store.forgotten.contains(&path) || out.contains(&path) {
            continue;
        }
        // A folder that no longer exists is not worth offering; it is also
        // not worth a durable "forgotten" entry, since it may come back
        // (an unmounted volume, a worktree being rebuilt).
        if !path.is_dir() {
            continue;
        }
        out.push(path);
    }
    out
}

/// Resolve symlinks where possible so the same folder is one entry.
///
/// `/tmp` and `/var` are symlinks on macOS, so the same directory reached
/// two ways would otherwise be listed twice and "forget" would only hide
/// one of them.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// These tests share ONE store (the home override is process-global), so
    /// they are serialized. Running them concurrently is not a realistic
    /// scenario being covered — it is two tests fighting over one file.
    static STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        STORE.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One shared, empty home for the whole test binary — the convention
    /// `isolate_home_for_tests` establishes, and the only one that works:
    /// the home override is PROCESS-global, so a per-test home is clobbered
    /// by whichever test runs next. These tests therefore share one store
    /// and each asserts only about its OWN uniquely-named workspace.
    fn workspace(name: &str) -> PathBuf {
        let home = vak_config::paths::isolate_home_for_tests();
        let ws = home.join(name);
        std::fs::create_dir_all(&ws).unwrap();
        canonical(&ws)
    }

    #[test]
    fn forgetting_hides_it_and_reopening_brings_it_back() {
        let _guard = guard();
        let ws = workspace("forget-roundtrip");

        remember(&ws).unwrap();
        assert!(visible([]).contains(&ws));

        forget(&ws).unwrap();
        assert!(!visible([]).contains(&ws), "forgotten but still shown");
        assert!(is_forgotten(&ws));

        // Re-opening is the un-forget. No separate verb to discover.
        remember(&ws).unwrap();
        assert!(visible([]).contains(&ws));
        assert!(!is_forgotten(&ws));
    }

    /// The whole safety argument: forgetting is a view decision, so nothing
    /// a later re-add would need may be destroyed by it.
    #[test]
    fn forgetting_touches_no_workspace_content() {
        let _guard = guard();
        let ws = workspace("forget-keeps-content");
        std::fs::create_dir_all(ws.join(".vak")).unwrap();
        std::fs::write(ws.join(".vak/config.toml"), "model = \"kept\"\n").unwrap();

        remember(&ws).unwrap();
        forget(&ws).unwrap();

        assert!(ws.is_dir(), "the workspace directory was removed");
        assert_eq!(
            std::fs::read_to_string(ws.join(".vak/config.toml")).unwrap(),
            "model = \"kept\"\n",
            "forgetting a workspace must not touch its settings"
        );
    }

    /// A forgotten workspace that something else rediscovers (a ledger
    /// scan) must stay hidden — otherwise "remove" lasts until the next
    /// refresh, which is not removal.
    #[test]
    fn a_rediscovered_workspace_stays_forgotten() {
        let _guard = guard();
        let ws = workspace("forget-rediscovered");
        forget(&ws).unwrap();
        assert!(
            !visible([ws.clone()]).contains(&ws),
            "rediscovery resurrected a forgotten workspace"
        );
    }

    /// A folder that no longer exists is not worth offering.
    #[test]
    fn a_missing_directory_is_not_offered() {
        let _guard = guard();
        let ws = workspace("forget-missing");
        remember(&ws).unwrap();
        std::fs::remove_dir_all(&ws).unwrap();
        assert!(!visible([]).contains(&ws));
    }
}
