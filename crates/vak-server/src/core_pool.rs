//! Multi-tenant `Core` pool (docs/design/34-channel-onboarding.md Phase 2).
//!
//! `GatewayState` used to run one `Core`, fixed at process start, for every
//! channel regardless of which workspace an allowlist entry pointed at — a
//! channel's `workspace` field only picked provider/model, never the
//! sandbox, permission mode, or session ledger that workspace's own `Core`
//! would resolve. `CorePool` makes that real: a canonical-workspace-path ->
//! lazily-started `Core` map, with the gateway's own default workspace
//! pinned permanently and every other entry idle-evicted / capacity-bounded.
//!
//! Security note: `resolve_at` always goes through `Core::new_with_trust`,
//! the exact same trust/permission/sandbox resolution a local `vak` run in
//! that workspace gets — pooling must never grant a channel more access
//! than a local session already has. The allowlist approval step (Phase 1)
//! is what gates a channel reaching a workspace at all; this module does
//! not weaken that boundary.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use vak_core::Core;

struct PooledEntry {
    core: Core,
    last_active: Instant,
}

/// One workspace's warm/cold state, for the admin UI's live indicator.
pub struct PoolStatusEntry {
    pub workspace: PathBuf,
    pub is_default: bool,
    pub idle_secs: u64,
}

/// Canonicalize the same way `checkpoints.rs` already does: best effort,
/// falling back to the given path unchanged when the filesystem can't
/// resolve it (a workspace that doesn't exist yet, or a test tempdir that
/// races cleanup) rather than failing pool lookups outright.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub struct CorePool {
    default_workspace: PathBuf,
    entries: Mutex<HashMap<PathBuf, PooledEntry>>,
    max: usize,
    idle: Duration,
}

impl CorePool {
    /// `default_core` seeds the pool's permanent entry for the gateway's
    /// own workspace — this replaces the old bare `state.core` field.
    pub fn new(default_core: Core, max: usize, idle: Duration) -> Self {
        let default_workspace = canonical(default_core.cwd());
        let mut entries = HashMap::new();
        entries.insert(
            default_workspace.clone(),
            PooledEntry {
                core: default_core,
                last_active: Instant::now(),
            },
        );
        CorePool {
            default_workspace,
            entries: Mutex::new(entries),
            max: max.max(1),
            idle,
        }
    }

    /// The gateway's own workspace — the pool's permanent, never-evicted
    /// entry. Exposed for tests and any future caller that needs to tell
    /// "the default" apart from "an approved channel's own workspace"
    /// without re-deriving it from a `Core`.
    #[allow(dead_code)]
    pub fn default_workspace(&self) -> &Path {
        &self.default_workspace
    }

    /// Resolve (lazily starting) the `Core` for `workspace`, evicting idle
    /// entries and enforcing the cap along the way. `now` is threaded
    /// explicitly rather than read from `Instant::now()` internally so
    /// tests can drive eviction deterministically without sleeping.
    pub fn resolve_at(&self, workspace: &Path, now: Instant) -> Result<Core, String> {
        let key = canonical(workspace);
        {
            let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            self.evict_idle_locked(&mut entries, now);
            if let Some(entry) = entries.get_mut(&key) {
                entry.last_active = now;
                return Ok(entry.core.clone());
            }
        }
        // Start outside the lock: `Core::new_with_trust` does filesystem IO
        // (config load) and must not hold up every other pool lookup.
        let core = Core::new_with_trust(key.clone(), true).map_err(|e| e.to_string())?;
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        // Someone else may have started the same workspace while we didn't
        // hold the lock; keep whichever is already resident to avoid a
        // second live Core silently replacing the one other requests hold.
        if let Some(entry) = entries.get_mut(&key) {
            entry.last_active = now;
            return Ok(entry.core.clone());
        }
        if entries.len() >= self.max {
            self.evict_oldest_idle_locked(&mut entries);
        }
        entries.insert(
            key,
            PooledEntry {
                core: core.clone(),
                last_active: now,
            },
        );
        Ok(core)
    }

    fn evict_idle_locked(&self, entries: &mut HashMap<PathBuf, PooledEntry>, now: Instant) {
        let default = self.default_workspace.clone();
        let idle = self.idle;
        entries.retain(|path, entry| {
            *path == default || now.saturating_duration_since(entry.last_active) < idle
        });
    }

    /// Cap enforcement: drop the least-recently-active non-default entry.
    /// The default workspace is never evicted, matching `GatewayState`'s
    /// old single-`Core` behavior for the gateway's own cwd.
    fn evict_oldest_idle_locked(&self, entries: &mut HashMap<PathBuf, PooledEntry>) {
        let default = self.default_workspace.clone();
        if let Some(oldest) = entries
            .iter()
            .filter(|(path, _)| **path != default)
            .min_by_key(|(_, entry)| entry.last_active)
            .map(|(path, _)| path.clone())
        {
            entries.remove(&oldest);
        }
    }

    /// Snapshot of every currently-warm workspace, for the admin UI's
    /// pool-status panel. `now` is explicit for the same testability reason
    /// as `resolve_at`.
    pub fn snapshot_at(&self, now: Instant) -> Vec<PoolStatusEntry> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let mut out: Vec<PoolStatusEntry> = entries
            .iter()
            .map(|(path, entry)| PoolStatusEntry {
                workspace: path.clone(),
                is_default: *path == self.default_workspace,
                idle_secs: now.saturating_duration_since(entry.last_active).as_secs(),
            })
            .collect();
        out.sort_by(|a, b| a.workspace.cmp(&b.workspace));
        out
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_core(dir: &std::path::Path) -> Core {
        Core::new_with_trust(dir.to_path_buf(), true).expect("core")
    }

    #[test]
    fn lazy_start_and_cache_hit() {
        let default_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));
        let t0 = Instant::now();
        assert_eq!(pool.len(), 1); // default only

        let first = pool.resolve_at(other_dir.path(), t0).expect("start");
        assert_eq!(pool.len(), 2);

        let second = pool
            .resolve_at(other_dir.path(), t0 + Duration::from_secs(1))
            .expect("cache hit");
        // Same canonical workspace resolves to the same pooled Core
        // instance (Arc-backed clone), not a freshly started one.
        assert_eq!(first.cwd(), second.cwd());
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn idle_eviction_after_configured_duration() {
        let default_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(60));
        let t0 = Instant::now();
        pool.resolve_at(other_dir.path(), t0).unwrap();
        assert_eq!(pool.len(), 2);

        // Still within the idle window: a lookup elsewhere must not evict it.
        let unrelated_dir = tempfile::tempdir().unwrap();
        pool.resolve_at(unrelated_dir.path(), t0 + Duration::from_secs(30))
            .unwrap();
        assert_eq!(pool.len(), 3);

        // Past the idle window: the next lookup sweeps stale entries first.
        pool.resolve_at(unrelated_dir.path(), t0 + Duration::from_secs(120))
            .unwrap();
        let snapshot = pool.snapshot_at(t0 + Duration::from_secs(120));
        let paths: Vec<_> = snapshot.iter().map(|e| e.workspace.clone()).collect();
        assert!(paths.contains(&pool.default_workspace().to_path_buf()));
        assert!(!paths.contains(&canonical(other_dir.path())));
    }

    #[test]
    fn default_workspace_never_evicted_by_idle_sweep() {
        let default_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(5));
        let t0 = Instant::now();
        pool.resolve_at(other_dir.path(), t0).unwrap();
        // Way past idle for everything, including the default.
        let far_future = t0 + Duration::from_secs(10_000);
        pool.resolve_at(other_dir.path(), far_future)
            .unwrap_or_else(|_| test_core(other_dir.path()));
        let snapshot = pool.snapshot_at(far_future);
        assert!(
            snapshot
                .iter()
                .any(|e| e.is_default && e.workspace == canonical(default_dir.path()))
        );
    }

    #[test]
    fn cap_enforcement_evicts_oldest_idle_not_default() {
        let default_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 2, Duration::from_secs(1800));
        let t0 = Instant::now();

        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        // Cap is 2: default + one more fits; a second non-default entry
        // must evict the oldest-idle non-default one (dir_a), not default.
        pool.resolve_at(dir_a.path(), t0).unwrap();
        assert_eq!(pool.len(), 2);
        pool.resolve_at(dir_b.path(), t0 + Duration::from_secs(10))
            .unwrap();
        assert_eq!(pool.len(), 2);

        let snapshot = pool.snapshot_at(t0 + Duration::from_secs(10));
        let paths: Vec<_> = snapshot.iter().map(|e| e.workspace.clone()).collect();
        assert!(paths.contains(&pool.default_workspace().to_path_buf()));
        assert!(paths.contains(&canonical(dir_b.path())));
        assert!(!paths.contains(&canonical(dir_a.path())));
    }
}
