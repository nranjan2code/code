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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use vak_config::PermissionMode;
use vak_core::Core;
use vak_core::discovery::ModelCatalogues;

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
    /// The default Core's model catalogues, shared by every Core the pool
    /// builds (docs/design/15-reliability.md, "How discovery stays warm").
    /// A catalogue is keyed by credential fingerprint, so a channel Core
    /// built for its first message in an hour plans from what the pool
    /// already knows instead of from nothing.
    catalogues: ModelCatalogues,
    /// Set once the host starts discovery refresh; every Core built after
    /// that starts its own loop.
    refreshing: AtomicBool,
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
        let catalogues = default_core.model_catalogues();
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
            catalogues,
            refreshing: AtomicBool::new(false),
            entries: Mutex::new(entries),
            max: max.max(1),
            idle,
        }
    }

    /// Start model-discovery refresh for every Core in the pool now, and for
    /// each one it builds from here on. Idempotent. The host calls this once
    /// it is serving; a Core's loop stops asking providers when that Core
    /// stops planning turns, so an evicted entry goes quiet by itself.
    pub fn start_model_discovery(&self) {
        self.refreshing.store(true, Ordering::Release);
        let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        for entry in entries.values() {
            entry.core.start_model_discovery();
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
                // A warm entry may outlive a persisted workspace downgrade.
                // Recheck only the security ceiling on cache hits: full
                // preference refresh would take unrelated capability locks
                // while an active turn is running.
                if let Err(error) = entry.core.enforce_persisted_permission_ceiling() {
                    return Err(format!(
                        "could not recheck workspace permission ceiling: {error}"
                    ));
                }
                entry.last_active = now;
                return Ok(entry.core.clone());
            }
        }
        // Trust is read from the one marker store, not assumed. This used to
        // pass `true` unconditionally, so a workspace whose trust prompt an
        // operator had declined in a terminal still had its hooks, MCP
        // servers, `permission_mode` and secret scope applied the moment a chat
        // routed a turn into it — two answers to "is this workspace
        // trusted?", which is exactly what `vak_core::trust` exists to end.
        //
        // Approving a channel's workspace in the admin console records
        // trust (see `admin::note_workspace_trust`), so the operator-driven
        // path this replaced still resolves to `true` — it now does so
        // because someone decided, not because the code assumed.
        let trusted = vak_core::trust::is_trusted(&key.0);
        // Start outside the lock: `Core::new_with_trust` does filesystem IO
        // (config load) and must not hold up every other pool lookup.
        let core = Core::new_with_trust(key.0.clone(), trusted)
            .map(|c| c.with_surface(vak_core::Surface::Server))
            .map_err(|e| e.to_string())?;
        core.set_agent_network_broker(self.agent_network.clone());
        core.share_model_catalogues(self.catalogues.clone());
        if policy != vak_config::ChannelPolicy::default() {
            core.apply_channel_policy(policy);
        }
        // Cap and pin before the instance is ever published to the map, so
        // no other request can observe it at the un-capped default.
        if let Some(requested) = permission_override
            && let Some((requested, ceiling)) = apply_permission_override(&core, requested)
        {
            vak_core::security_events::record(
                &core.scope(),
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
        self.evict_to_cap_locked(&mut entries);
        // Only the instance that is kept gets a loop; a loser of the race
        // above is dropped without ever having asked a provider anything.
        if self.refreshing.load(Ordering::Acquire) {
            core.start_model_discovery();
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

    /// Neither sweep drops an entry something outside the pool still holds.
    /// Dropping it frees nothing, since the holder keeps the Core alive, and
    /// the next lookup would build a second Core beside the live one. The
    /// cap is therefore a bound on Cores nobody holds, and the pool may
    /// briefly exceed it while more workspaces than that are busy.
    fn evict_idle_locked(&self, entries: &mut HashMap<PoolKey, PooledEntry>, now: Instant) {
        let default = self.default_key();
        let idle = self.idle;
        entries.retain(|key, entry| {
            *key == default
                || entry.core.has_other_handles()
                || now.saturating_duration_since(entry.last_active) < idle
        });
    }

    /// The permanent entry's key: the gateway's own workspace with no
    /// permission override. A same-workspace *override* entry is an
    /// ordinary evictable entry — it is not the gateway's own Core.
    fn default_key(&self) -> PoolKey {
        (self.default_workspace.clone(), None, String::new())
    }

    /// Drop every pooled instance except the gateway's own, so the next
    /// inbound message on each channel rebuilds against current config.
    ///
    /// `apply_permission_override` reads a workspace's ceiling exactly once,
    /// when the entry is constructed. Persisting a narrower mode therefore
    /// took effect for the console immediately and for every warm channel
    /// only after its idle window expired — up to half an hour of chats
    /// still running under a ceiling the operator had already revoked. The
    /// answer is not to re-derive each entry's cap in place (that would be a
    /// second copy of the capping rule) but to discard the entries, so the
    /// one place that computes a ceiling runs again.
    ///
    /// Returns how many were dropped, for the audit line.
    pub fn invalidate_pooled(&self) -> usize {
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let default = self.default_key();
        let before = entries.len();
        entries.retain(|key, _| *key == default);
        before - entries.len()
    }

    /// Cap enforcement: make room for one new entry by dropping the
    /// least-recently-active non-default entries nothing else holds (see
    /// `evict_idle_locked`), as many as it takes, so a pool that grew past
    /// the cap while busy shrinks back once those Cores are released. The
    /// default workspace is never evicted, matching `GatewayState`'s old
    /// single-`Core` behavior for the gateway's own cwd.
    fn evict_to_cap_locked(&self, entries: &mut HashMap<PoolKey, PooledEntry>) {
        let default = self.default_key();
        while entries.len() >= self.max {
            let Some(oldest) = entries
                .iter()
                .filter(|(key, entry)| **key != default && !entry.core.has_other_handles())
                .min_by_key(|(_, entry)| entry.last_active)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
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
        vak_config::paths::isolate_home_for_tests();
        Core::new_with_trust(dir.to_path_buf(), true).expect("core")
    }

    /// Write a workspace config that fixes the workspace's own permission
    /// mode — the ceiling every channel override is capped against — and
    /// record the operator's trust decision for it.
    ///
    /// The trust marker is not incidental setup. `permission_mode` is a
    /// privileged key: `load_with_trust` strips it from an untrusted
    /// project, and the pool now reads the real marker store rather than
    /// assuming trust. A test that skipped this would be asserting the cap
    /// against a ceiling no run would ever see.
    fn workspace_with_mode(dir: &std::path::Path, mode: &str) {
        let vak = dir.join(".vak");
        std::fs::create_dir_all(&vak).expect("mkdir .vak");
        std::fs::write(
            vak.join("config.toml"),
            format!("permission_mode = \"{mode}\"\n"),
        )
        .expect("write config");
        vak_config::paths::isolate_home_for_tests();
        vak_core::trust::record(dir).expect("record trust");
    }

    /// A narrowed mode used to reach warm channels only when their idle
    /// window expired — up to half an hour of chats running under a ceiling
    /// the operator had already revoked.
    #[test]
    fn invalidating_the_pool_drops_channel_instances_and_keeps_the_default() {
        let default_dir = tempfile::tempdir().unwrap();
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));
        pool.resolve_at(a.path(), None, Instant::now()).unwrap();
        pool.resolve_at(b.path(), None, Instant::now()).unwrap();
        assert_eq!(pool.len(), 3);

        assert_eq!(pool.invalidate_pooled(), 2);
        assert_eq!(pool.len(), 1, "the gateway's own Core is never dropped");
        assert!(
            pool.resolve_at(a.path(), None, Instant::now()).is_ok(),
            "and the next message rebuilds against current config"
        );
    }

    /// The pool reads the one trust marker store rather than assuming trust.
    /// A workspace nobody has vouched for gets its privileged keys stripped
    /// here exactly as it would in a terminal.
    #[test]
    fn an_untrusted_workspace_does_not_get_its_privileged_keys() {
        let default_dir = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        vak_config::paths::isolate_home_for_tests();
        std::fs::create_dir_all(ws.path().join(".vak")).unwrap();
        std::fs::write(
            ws.path().join(".vak/config.toml"),
            "permission_mode = \"full-access\"\n",
        )
        .unwrap();
        // Deliberately NOT recording trust.
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(1800));

        // Compared against what the SAME workspace resolves to with its
        // project file ignored, rather than against a hardcoded default: the
        // global layer is shared by this whole test binary, so the baseline
        // is whatever it happens to be. The claim under test is only that
        // the untrusted project's own `full-access` is not applied.
        let baseline = Core::new_with_trust(ws.path().to_path_buf(), false)
            .unwrap()
            .effective_permission_mode();
        let core = pool.resolve_at(ws.path(), None, Instant::now()).unwrap();
        assert_eq!(
            core.effective_permission_mode(),
            baseline,
            "an unvouched project must not configure itself into full access"
        );

        // And the marker is what makes the difference: vouch for it and the
        // project's own mode applies.
        vak_core::trust::record(ws.path()).unwrap();
        let trusted = Core::new_with_trust(ws.path().to_path_buf(), true).unwrap();
        assert_eq!(
            trusted.effective_permission_mode(),
            PermissionMode::FullAccess
        );
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
        let events = vak_core::security_events::list(&core.scope(), 500);
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

    /// Cap eviction must not orphan a Core that is still in use. A running
    /// turn holds a clone of its Core; if the cap drops that entry anyway,
    /// the next message for the workspace builds a second, independent Core
    /// beside the live one, and whatever runtime state the live one carries
    /// (here a narrowed permission pin) is silently absent from the new one.
    /// The held clone stands in for the in-flight turn; the runtime pin is
    /// the marker that tells the two instances apart.
    #[test]
    fn cap_eviction_does_not_replace_a_core_still_in_use() {
        let default_dir = tempfile::tempdir().unwrap();
        let busy = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        workspace_with_mode(busy.path(), "full-access");
        let pool = CorePool::new(test_core(default_dir.path()), 2, Duration::from_secs(1800));
        let t0 = Instant::now();

        let in_flight = pool.resolve_at(busy.path(), None, t0).unwrap();
        in_flight.set_permission_mode(PermissionMode::ReadOnly);

        // Second workspace reaches the cap while `in_flight` is still held.
        pool.resolve_at(other.path(), None, t0 + Duration::from_secs(1))
            .unwrap();

        let next_message = pool
            .resolve_at(busy.path(), None, t0 + Duration::from_secs(2))
            .unwrap();
        assert_eq!(
            next_message.effective_permission_mode(),
            PermissionMode::ReadOnly,
            "the workspace's next message must reach the live Core, not a \
             fresh one rebuilt at the workspace's full-access default"
        );

        // Once nothing holds it, the cap applies to it again.
        drop(in_flight);
        drop(next_message);
        let third = tempfile::tempdir().unwrap();
        pool.resolve_at(third.path(), None, t0 + Duration::from_secs(3))
            .unwrap();
        let paths: Vec<_> = pool
            .snapshot_at(t0 + Duration::from_secs(3))
            .into_iter()
            .map(|e| e.workspace)
            .collect();
        assert!(!paths.contains(&canonical(busy.path())));
    }

    /// The same holds for the idle sweep: a turn that runs longer than the
    /// idle window must not lose its pool entry partway through.
    #[test]
    fn idle_sweep_keeps_a_core_still_in_use() {
        let default_dir = tempfile::tempdir().unwrap();
        let busy = tempfile::tempdir().unwrap();
        let pool = CorePool::new(test_core(default_dir.path()), 8, Duration::from_secs(5));
        let t0 = Instant::now();

        let in_flight = pool.resolve_at(busy.path(), None, t0).unwrap();
        let after_idle = t0 + Duration::from_secs(60);
        pool.resolve_at(default_dir.path(), None, after_idle)
            .unwrap();
        assert!(
            pool.snapshot_at(after_idle)
                .iter()
                .any(|e| e.workspace == canonical(busy.path()))
        );

        drop(in_flight);
        pool.resolve_at(default_dir.path(), None, after_idle)
            .unwrap();
        assert!(
            !pool
                .snapshot_at(after_idle)
                .iter()
                .any(|e| e.workspace == canonical(busy.path()))
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

    /// Every Core the pool builds plans from the default Core's catalogues,
    /// so a channel Core built for its first message in an hour is not cold;
    /// and once the host starts refresh, every Core in the pool, resident or
    /// built later, runs its own loop. The test never yields, so no loop
    /// gets to ask a provider anything.
    #[tokio::test]
    async fn pooled_cores_share_catalogues_and_refresh_once_started() {
        let default_dir = tempfile::tempdir().unwrap();
        let resident_dir = tempfile::tempdir().unwrap();
        let later_dir = tempfile::tempdir().unwrap();
        let default = test_core(default_dir.path());
        let pool = CorePool::new(default.clone(), 8, Duration::from_secs(1800));

        let resident = pool
            .resolve_at(resident_dir.path(), None, Instant::now())
            .expect("start");
        assert!(
            resident
                .model_catalogues()
                .same_store(&default.model_catalogues())
        );
        assert!(
            !resident.model_discovery_started(),
            "the host decides when refresh starts"
        );

        pool.start_model_discovery();
        assert!(default.model_discovery_started());
        assert!(resident.model_discovery_started());

        let later = pool
            .resolve_at(later_dir.path(), None, Instant::now())
            .expect("start");
        assert!(
            later
                .model_catalogues()
                .same_store(&default.model_catalogues())
        );
        assert!(later.model_discovery_started());
    }

    /// A warm pool entry is reused, but its persisted permission ceiling is
    /// rechecked on every cache hit. This security-only check avoids taking
    /// unrelated capability locks while an active turn is running and cancels
    /// the old Core permission lease before narrowing the resident mode.
    #[test]
    fn warm_pool_entry_rechecks_a_permission_mode_change_written_after_it_started() {
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
            PermissionMode::ReadOnly,
            "a warm channel must observe the persisted security ceiling"
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
