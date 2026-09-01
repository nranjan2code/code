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
//!
//! Per-channel permission overrides (docs/design/34 "Per-channel
//! permission mode") make the pool key a *pair*: `(workspace, override)`.
//! Two channels sharing a workspace but wanting different trust levels get
//! two distinct `Core` instances, because a `Core`'s permission mode is a
//! single piece of shared mutable state — handing one `Core` to two
//! channels with different intended modes would let whichever channel
//! resolved first dictate the other's permissions. The override itself is
//! capped in `apply_permission_override` and can only ever reduce, never
//! raise, what the workspace's own config already grants.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use vak_config::PermissionMode;
use vak_core::Core;

/// Pool identity. `None` in the second slot is "inherit this workspace's
/// own configured mode" — today's behavior and the key the gateway's own
/// default entry always uses, so an un-overridden channel keeps sharing
/// the exact `Core` it shares now.
type PoolKey = (PathBuf, Option<PermissionMode>, String);

struct PooledEntry {
    core: Core,
    last_active: Instant,
}

/// One workspace's warm/cold state, for the admin UI's live indicator.
pub struct PoolStatusEntry {
    pub workspace: PathBuf,
    pub is_default: bool,
    pub idle_secs: u64,
    /// The permission override this pooled instance was keyed by, if any,
    /// so the panel can tell two same-workspace instances apart.
    pub permission_override: Option<PermissionMode>,
    /// What this instance actually resolved to after capping.
    pub effective_permission_mode: PermissionMode,
}

/// Clamp a requested per-channel override to what the workspace's own
/// resolved configuration already grants, and pin the result onto `core`.
///
/// `core` must be freshly constructed by `Core::new_with_trust`, so its
/// `effective_permission_mode()` is precisely the workspace's own
/// configured mode — the same value a local `vak` run in that workspace
/// would get. That value is the ceiling. `capped_by` is a `min`, so the
/// result is provably never more permissive than the ceiling; there is no
/// code path here that pins a mode above it.
///
/// Returns `Some((requested, capped))` when the request had to be reduced,
/// so the caller can record it in the audit log.
fn apply_permission_override(
    core: &Core,
    requested: PermissionMode,
) -> Option<(PermissionMode, PermissionMode)> {
    let workspace_ceiling = core.effective_permission_mode();
    let capped = requested.capped_by(workspace_ceiling);
    core.set_permission_mode(capped);
    (capped != requested).then_some((requested, workspace_ceiling))
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
    agent_network: vak_core::agent_network::AgentNetworkBroker,
    entries: Mutex<HashMap<PoolKey, PooledEntry>>,
    max: usize,
    idle: Duration,
}

impl CorePool {
    /// `default_core` seeds the pool's permanent entry for the gateway's
    /// own workspace — this replaces the old bare `state.core` field.
    pub fn new(default_core: Core, max: usize, idle: Duration) -> Self {
        let default_workspace = canonical(default_core.cwd());
        let agent_network = default_core.agent_network_broker();
        let mut entries = HashMap::new();
        entries.insert(
            (default_workspace.clone(), None, String::new()),
            PooledEntry {
                core: default_core,
                last_active: Instant::now(),
            },
        );
        CorePool {
            default_workspace,
            agent_network,
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
    ///
    /// `permission_override` is the channel's requested permission mode
    /// (`None` = inherit the workspace's own). It is part of the pool key,
    /// so a channel asking for a different mode never shares an instance
    /// with one asking for another, and it is capped to the workspace's
    /// own resolved mode before being pinned onto the fresh `Core`.
    #[allow(dead_code)]
    pub fn resolve_at(
        &self,
        workspace: &Path,
        permission_override: Option<PermissionMode>,
        now: Instant,
    ) -> Result<Core, String> {
        self.resolve_at_with_policy(
            workspace,
            permission_override,
            vak_config::ChannelPolicy::default(),
            now,
        )
    }

    pub fn resolve_at_with_policy(
        &self,
        workspace: &Path,
        permission_override: Option<PermissionMode>,
        policy: vak_config::ChannelPolicy,
        now: Instant,
    ) -> Result<Core, String> {
        let policy_key = if policy == vak_config::ChannelPolicy::default() {
            String::new()
        } else {
            serde_json::to_string(&policy).map_err(|e| e.to_string())?
        };
        let key: PoolKey = (canonical(workspace), permission_override, policy_key);
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
        let core = Core::new_with_trust(key.0.clone(), true).map_err(|e| e.to_string())?;
        core.set_agent_network_broker(self.agent_network.clone());
        if policy != vak_config::ChannelPolicy::default() {
            core.apply_channel_policy(policy);
        }
        // Cap and pin before the instance is ever published to the map, so
        // no other request can observe it at the un-capped default.
        if let Some(requested) = permission_override
            && let Some((requested, ceiling)) = apply_permission_override(&core, requested)
        {
            vak_core::security_events::record(
                &core.sessions_home(),
                vak_core::security_events::EventKind::PermissionCapped,
                "permission_capped",
                &format!(
                    "workspace={} requested={} capped_to={}",
                    key.0.display(),
                    requested.as_str(),
                    ceiling.as_str()
                ),
                None,
            );
        }
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

    fn evict_idle_locked(&self, entries: &mut HashMap<PoolKey, PooledEntry>, now: Instant) {
        let default = self.default_key();
        let idle = self.idle;
        entries.retain(|key, entry| {
            *key == default || now.saturating_duration_since(entry.last_active) < idle
        });
    }

    /// The permanent entry's key: the gateway's own workspace with no
    /// permission override. A same-workspace *override* entry is an
    /// ordinary evictable entry — it is not the gateway's own Core.
    fn default_key(&self) -> PoolKey {
        (self.default_workspace.clone(), None, String::new())
    }

    /// Cap enforcement: drop the least-recently-active non-default entry.
    /// The default workspace is never evicted, matching `GatewayState`'s
    /// old single-`Core` behavior for the gateway's own cwd.
    fn evict_oldest_idle_locked(&self, entries: &mut HashMap<PoolKey, PooledEntry>) {
        let default = self.default_key();
        if let Some(oldest) = entries
            .iter()
            .filter(|(key, _)| **key != default)
            .min_by_key(|(_, entry)| entry.last_active)
            .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
    }

    /// Snapshot of every currently-warm workspace, for the admin UI's
    /// pool-status panel. `now` is explicit for the same testability reason
    /// as `resolve_at`.
    pub fn snapshot_at(&self, now: Instant) -> Vec<PoolStatusEntry> {
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let default = self.default_key();
        let mut out: Vec<PoolStatusEntry> = entries
            .iter()
            .map(|(key, entry)| PoolStatusEntry {
                workspace: key.0.clone(),
                is_default: *key == default,
                idle_secs: now.saturating_duration_since(entry.last_active).as_secs(),
                permission_override: key.1,
                effective_permission_mode: entry.core.effective_permission_mode(),
            })
            .collect();
        out.sort_by(|a, b| {
            a.workspace.cmp(&b.workspace).then_with(|| {
                a.permission_override
                    .map(|m| m.rank())
                    .cmp(&b.permission_override.map(|m| m.rank()))
            })
        });
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

    /// Write a workspace config that fixes the workspace's own permission
    /// mode — the ceiling every channel override is capped against.
    fn workspace_with_mode(dir: &std::path::Path, mode: &str) {
        let vak = dir.join(".vak");
        std::fs::create_dir_all(&vak).expect("mkdir .vak");
        std::fs::write(
            vak.join("config.toml"),
            format!("permission_mode = \"{mode}\"\n"),
        )
        .expect("write config");
    }

    #[test]
    fn no_override_inherits_the_workspace_mode_unchanged() {
        let default_dir = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        workspace_with_mode(ws.path(), "full-access");
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));

        let core = pool.resolve_at(ws.path(), None, Instant::now()).unwrap();
        // Today's behavior, untouched: the workspace's own config wins.
        assert_eq!(core.effective_permission_mode(), PermissionMode::FullAccess);
        assert!(!core.permission_mode_runtime_pinned());
    }

    #[test]
    fn override_at_or_below_the_workspace_mode_is_applied_exactly() {
        let default_dir = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        workspace_with_mode(ws.path(), "full-access");
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));

        for requested in [PermissionMode::ReadOnly, PermissionMode::WorkspaceWrite] {
            let core = pool
                .resolve_at(ws.path(), Some(requested), Instant::now())
                .unwrap();
            assert_eq!(
                core.effective_permission_mode(),
                requested,
                "a reduction to {requested:?} under full-access must apply verbatim"
            );
        }
        // And an override that exactly equals the ceiling is a no-op match.
        let core = pool
            .resolve_at(ws.path(), Some(PermissionMode::FullAccess), Instant::now())
            .unwrap();
        assert_eq!(core.effective_permission_mode(), PermissionMode::FullAccess);
    }

    /// The security property: an override asking for MORE than the
    /// workspace's own config grants is clamped to the workspace's mode,
    /// and the reduction is written to the audit log.
    #[test]
    fn override_above_the_workspace_mode_is_capped_and_audited() {
        let default_dir = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        workspace_with_mode(ws.path(), "read-only");
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));

        let core = pool
            .resolve_at(ws.path(), Some(PermissionMode::FullAccess), Instant::now())
            .unwrap();
        assert_eq!(
            core.effective_permission_mode(),
            PermissionMode::ReadOnly,
            "a full-access override on a read-only workspace must NOT escalate"
        );

        // `sessions_home` is process-wide here (these pool tests build
        // real `Core`s), so scope the assertion to this test's own unique
        // tempdir workspace rather than to the whole event log.
        let marker = format!("workspace={}", canonical(ws.path()).display());
        let events = vak_core::security_events::list(&core.sessions_home(), 500);
        let capped: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == vak_core::security_events::EventKind::PermissionCapped
                    && e.detail.contains(&marker)
            })
            .collect();
        assert_eq!(capped.len(), 1, "the silent reduction must be auditable");
        assert!(capped[0].detail.contains("requested=full-access"));
        assert!(capped[0].detail.contains("capped_to=read-only"));

        // A workspace-write request on the same read-only workspace is
        // capped too — the ceiling is the config, not merely "not full".
        let core2 = pool
            .resolve_at(
                ws.path(),
                Some(PermissionMode::WorkspaceWrite),
                Instant::now(),
            )
            .unwrap();
        assert_eq!(core2.effective_permission_mode(), PermissionMode::ReadOnly);
    }

    /// Two channels, one workspace, different overrides: they must never
    /// share a `Core`, because a `Core` carries exactly one permission
    /// mode and sharing would let the looser channel's mode leak into the
    /// tighter one (or vice versa, depending on who resolved first).
    #[test]
    fn same_workspace_different_overrides_get_distinct_cores() {
        let default_dir = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        workspace_with_mode(ws.path(), "full-access");
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));
        let t0 = Instant::now();

        let tight = pool
            .resolve_at(ws.path(), Some(PermissionMode::ReadOnly), t0)
            .unwrap();
        let loose = pool
            .resolve_at(ws.path(), Some(PermissionMode::FullAccess), t0)
            .unwrap();
        let inherited = pool.resolve_at(ws.path(), None, t0).unwrap();

        // Three distinct pooled instances for one workspace path.
        assert_eq!(pool.len(), 4); // default + the three above
        assert_eq!(tight.cwd(), loose.cwd());

        // Neither leaks into the other, in either direction.
        assert_eq!(tight.effective_permission_mode(), PermissionMode::ReadOnly);
        assert_eq!(
            loose.effective_permission_mode(),
            PermissionMode::FullAccess
        );
        assert_eq!(
            inherited.effective_permission_mode(),
            PermissionMode::FullAccess
        );

        // Re-resolving the tight channel still yields the tight instance —
        // the looser resolution did not overwrite the cached entry.
        let tight_again = pool
            .resolve_at(ws.path(), Some(PermissionMode::ReadOnly), t0)
            .unwrap();
        assert_eq!(
            tight_again.effective_permission_mode(),
            PermissionMode::ReadOnly
        );

        // The pool status panel can tell the instances apart.
        let snapshot = pool.snapshot_at(t0);
        let mut overrides: Vec<_> = snapshot
            .iter()
            .filter(|e| e.workspace == canonical(ws.path()))
            .map(|e| (e.permission_override, e.effective_permission_mode))
            .collect();
        overrides.sort_by_key(|(o, _)| o.map(|m| m.rank()));
        assert_eq!(overrides.len(), 3);
    }

    #[test]
    fn override_entries_are_evictable_but_the_default_key_is_not() {
        let default_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(60));
        let t0 = Instant::now();
        // An override entry on the *default* workspace is an ordinary
        // evictable entry, not the permanent gateway Core.
        pool.resolve_at(default_dir.path(), Some(PermissionMode::ReadOnly), t0)
            .unwrap();
        assert_eq!(pool.len(), 2);

        let other = tempfile::tempdir().unwrap();
        pool.resolve_at(other.path(), None, t0 + Duration::from_secs(120))
            .unwrap();
        let snapshot = pool.snapshot_at(t0 + Duration::from_secs(120));
        assert!(snapshot.iter().any(|e| e.is_default));
        assert!(
            !snapshot
                .iter()
                .any(|e| e.permission_override == Some(PermissionMode::ReadOnly))
        );
    }

    #[test]
    fn lazy_start_and_cache_hit() {
        let default_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));
        let t0 = Instant::now();
        assert_eq!(pool.len(), 1); // default only

        let first = pool.resolve_at(other_dir.path(), None, t0).expect("start");
        assert_eq!(pool.len(), 2);

        let second = pool
            .resolve_at(other_dir.path(), None, t0 + Duration::from_secs(1))
            .expect("cache hit");
        // Same canonical workspace resolves to the same pooled Core
        // instance (Arc-backed clone), not a freshly started one.
        assert_eq!(first.cwd(), second.cwd());
        assert_eq!(pool.len(), 2);
    }

    /// A warm pool entry is an `Arc`-backed `Core` resolved once from the
    /// workspace's config and then reused verbatim on every cache hit — a
    /// permission-mode change persisted to that workspace's
    /// `.vak/config.toml` after the entry warmed (by a separate `vak`
    /// process, or a different workspace's own config changing while this
    /// gateway serves it too, docs/design/34 Phase 2) is invisible to it
    /// until eviction. AGENTS.md rule 17 promises "permission changes
    /// revoke active capabilities before apply", and `apply_permission_mode`
    /// (vak-server/src/lib.rs) keeps that promise for `state.sessions` —
    /// but a gateway-routed channel dispatches through `CorePool` instead,
    /// which that sweep never touches.
    ///
    /// A first attempt at closing this (re-deriving the mode from disk on
    /// every cache hit, before returning the entry) reproducibly hung
    /// `busy_message_is_steered_not_dropped` (tests/gateway.rs) — a
    /// steering message resolved mid-turn on the same warm `Core` a live
    /// tool call was still running on. Wrapping the refresh in
    /// `tokio::task::block_in_place` did not fix it, so the hang is not
    /// simple executor-thread starvation from synchronous file IO; it did
    /// not surface in this module's own tests (no in-flight turn to race
    /// against), only against a real in-progress dispatch. That fix was
    /// reverted rather than shipped un-understood. This test intentionally
    /// documents the *current* (unfixed) behavior, so the gap stays
    /// visible and any future fix attempt has this exact test — plus
    /// `busy_message_is_steered_not_dropped` — as its two required checks.
    #[test]
    fn warm_pool_entry_does_not_see_a_permission_mode_change_written_after_it_started() {
        let default_dir = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        workspace_with_mode(ws.path(), "full-access");
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));
        let t0 = Instant::now();

        let warm = pool.resolve_at(ws.path(), None, t0).unwrap();
        assert_eq!(warm.effective_permission_mode(), PermissionMode::FullAccess);

        // An operator downgrades the workspace to read-only — e.g. from the
        // admin console's Settings page — well within the 30-minute idle
        // window, on an otherwise-active channel that never goes idle.
        workspace_with_mode(ws.path(), "read-only");

        let still_warm = pool
            .resolve_at(ws.path(), None, t0 + Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            still_warm.effective_permission_mode(),
            PermissionMode::FullAccess,
            "documents the gap: an active channel keeps the pre-downgrade \
             mode until its pool entry is idle-evicted or the process restarts"
        );
    }

    #[test]
    fn idle_eviction_after_configured_duration() {
        let default_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(60));
        let t0 = Instant::now();
        pool.resolve_at(other_dir.path(), None, t0).unwrap();
        assert_eq!(pool.len(), 2);

        // Still within the idle window: a lookup elsewhere must not evict it.
        let unrelated_dir = tempfile::tempdir().unwrap();
        pool.resolve_at(unrelated_dir.path(), None, t0 + Duration::from_secs(30))
            .unwrap();
        assert_eq!(pool.len(), 3);

        // Past the idle window: the next lookup sweeps stale entries first.
        pool.resolve_at(unrelated_dir.path(), None, t0 + Duration::from_secs(120))
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
        pool.resolve_at(other_dir.path(), None, t0).unwrap();
        // Way past idle for everything, including the default.
        let far_future = t0 + Duration::from_secs(10_000);
        pool.resolve_at(other_dir.path(), None, far_future)
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
        pool.resolve_at(dir_a.path(), None, t0).unwrap();
        assert_eq!(pool.len(), 2);
        pool.resolve_at(dir_b.path(), None, t0 + Duration::from_secs(10))
            .unwrap();
        assert_eq!(pool.len(), 2);

        let snapshot = pool.snapshot_at(t0 + Duration::from_secs(10));
        let paths: Vec<_> = snapshot.iter().map(|e| e.workspace.clone()).collect();
        assert!(paths.contains(&pool.default_workspace().to_path_buf()));
        assert!(paths.contains(&canonical(dir_b.path())));
        assert!(!paths.contains(&canonical(dir_a.path())));
    }

    #[test]
    fn distinct_channel_policies_get_distinct_pool_entries() {
        let default_dir = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));
        let allow_tavily = vak_config::ChannelPolicy {
            mcp_allow: Some(vec!["tavily/*".into()]),
            ..Default::default()
        };
        let deny_mcp = vak_config::ChannelPolicy {
            mcp_allow: Some(Vec::new()),
            ..Default::default()
        };
        pool.resolve_at_with_policy(default_dir.path(), None, allow_tavily, Instant::now())
            .unwrap();
        pool.resolve_at_with_policy(
            default_dir.path(),
            None,
            deny_mcp,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(pool.len(), 3);
    }
}
