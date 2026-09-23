//! The capability registry: one owner, one reconcile loop, one projection.
//!
//! # Why a loop and not a cache
//!
//! Everything this replaces was **edge-triggered**: discovery warmed once, a
//! prompt froze once, a connection opened once, a catalog cached once — with
//! a retry guard written so that caching a *failure* counted as having
//! succeeded. A single missed edge was therefore permanent, and uptime is
//! what converts a small race into a dead integration.
//!
//! This is **level-triggered**, the way long-running systems are built:
//! `reconcile()` compares desired state to observed state and moves toward
//! it, idempotently. Hints (a filesystem event, an MCP `list_changed`, a
//! plugin toggle) only make it run *sooner*; the ticker guarantees it runs
//! anyway. A dropped hint costs one tick of latency. It never costs
//! correctness.
//!
//! # Two channels, deliberately asymmetric
//!
//! Additions and catalog changes take effect at the next **turn boundary**,
//! via a published epoch. Revocations take effect **immediately**, mid-turn,
//! fail-closed. That split follows a rule the codebase already believes:
//! narrowing is always safe, widening needs admission. An operator disabling
//! a compromised plugin must not wait for a long turn to finish, while a
//! newly added skill appearing halfway through a plan would be a torn read.
//!
//! # Offline by construction
//!
//! A pass never talks to anything. Declarations are a filesystem walk at
//! worst, and an MCP server's catalog and failure arrive as declared data
//! from the on-demand pool (`vak_mcp::McpManager`), which is the only thing
//! allowed to start a server. So reconciling is cheap enough to run at every
//! turn admission, and it can never spawn an integration nobody asked for.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};

use super::domain::Serves;
use super::resolution::Resolution;
use super::snapshot::{Capability, CapabilityDelta, CapabilityId, CapabilitySet, Epoch, Origin};

/// How often the loop reconciles with no hint at all. The safety net that
/// makes a missed event survivable.
pub const RECONCILE_INTERVAL: Duration = Duration::from_secs(10);

/// Hints are coalesced over this window, so an editor writing six files does
/// one reconcile rather than six.
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// One capability as its source declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub id: CapabilityId,
    pub origin: Origin,
    pub summary: String,
    pub serves: Serves,
    pub digest: Option<String>,
    pub source: Option<std::path::PathBuf>,
    /// Kind-specific detail, part of the published digest — for an MCP
    /// server, what the pool has observed (catalog, last failure).
    pub configuration: serde_json::Value,
}

/// Where declarations come from.
///
/// Implemented by `Core`, which already owns skill/hook/command/MCP
/// discovery. Keeping it behind a trait means the loop is testable without a
/// workspace, a filesystem, or a live MCP server.
#[async_trait]
pub trait CapabilityProvider: Send + Sync {
    /// Everything currently declared. Cheap and offline: a filesystem walk
    /// at worst, never a network call or a spawned process.
    fn declare(&self) -> Vec<Declaration>;

    /// Housekeeping on each pass, such as evicting idle pooled connections.
    /// It may only ever release resources, never acquire them.
    async fn upkeep(&self) {}
}

/// Why the loop woke up. Purely an optimisation — the ticker would have
/// caught all of these eventually.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// A watched capability directory changed.
    SourceChanged(String),
    /// The MCP pool learned something about a server (a catalog, a
    /// failure), or the server announced a catalog change.
    ServerObserved(String),
    /// Configuration or plugin enablement moved.
    ConfigChanged,
    /// Someone asked for an immediate pass (boot, or a CLI about to run one
    /// turn and exit).
    Immediate,
}

/// The health of the loop itself.
///
/// Exposed so "capabilities look thin because reconciliation has been
/// failing for two hours" is a visible fact rather than an unexplained
/// shortage of tools.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileStatus {
    pub last_run: Option<SystemTime>,
    pub last_change: Option<SystemTime>,
    pub epoch: Epoch,
    pub passes: u64,
}

/// The registry.
pub struct CapabilityRegistry {
    provider: Arc<dyn CapabilityProvider>,
    current: RwLock<Arc<CapabilitySet>>,
    next_epoch: AtomicU64,
    /// The immediate channel. Checked at dispatch, independent of any epoch.
    revoked: RwLock<BTreeMap<CapabilityId, String>>,
    revoked_fast: std::sync::RwLock<BTreeMap<CapabilityId, String>>,
    status: RwLock<ReconcileStatus>,
    hints: mpsc::UnboundedSender<Hint>,
}

impl CapabilityRegistry {
    pub fn new(
        provider: Arc<dyn CapabilityProvider>,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<Hint>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let registry = Arc::new(CapabilityRegistry {
            provider,
            current: RwLock::new(Arc::new(CapabilitySet::empty())),
            next_epoch: AtomicU64::new(1),
            revoked: RwLock::new(BTreeMap::new()),
            revoked_fast: std::sync::RwLock::new(BTreeMap::new()),
            status: RwLock::new(ReconcileStatus::default()),
            hints: tx,
        });
        (registry, rx)
    }

    /// Ask for a reconcile sooner than the ticker would. Never blocks and
    /// never fails meaningfully: if the loop is gone, the hint is moot.
    pub fn hint(&self, hint: Hint) {
        let _ = self.hints.send(hint);
    }

    /// The current published set. What a turn binds at its start.
    pub async fn current(&self) -> Arc<CapabilitySet> {
        self.current.read().await.clone()
    }

    /// Best-effort synchronous read of the current published set, for
    /// diagnostics/reporting call sites (`capability_diagnostics`, `/doctor`,
    /// the system prompt) that are not themselves async. Reconciliation is
    /// an eventually-consistent background loop by design — a rare
    /// contended `try_read` just means this diagnostic reflects the
    /// previous epoch for one more instant, not a correctness problem.
    /// Falls back to an empty set only if the lock is actually contended
    /// (never blocks), which a caller reporting "nothing degraded" during
    /// that instant is a harmless, self-correcting understatement.
    pub fn current_blocking(&self) -> Arc<CapabilitySet> {
        self.current
            .try_read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| Arc::new(CapabilitySet::empty()))
    }

    pub async fn status(&self) -> ReconcileStatus {
        self.status.read().await.clone()
    }

    /// Revoke immediately, mid-turn, fail-closed.
    ///
    /// Does not wait for an epoch: a compromised plugin must stop being
    /// callable now, not when the current turn happens to finish. This only
    /// ever narrows, which is why it is safe to apply without admission.
    pub async fn revoke(&self, id: CapabilityId, reason: impl Into<String>) {
        let reason = reason.into();
        self.revoked
            .write()
            .await
            .insert(id.clone(), reason.clone());
        if let Ok(mut revoked) = self.revoked_fast.write() {
            revoked.insert(id, reason);
        }
        self.hint(Hint::ConfigChanged);
    }

    pub async fn restore(&self, id: &CapabilityId) {
        self.revoked.write().await.remove(id);
        if let Ok(mut revoked) = self.revoked_fast.write() {
            revoked.remove(id);
        }
        self.hint(Hint::ConfigChanged);
    }

    pub fn revoked_now(&self, id: &CapabilityId) -> bool {
        self.revoked_fast
            .read()
            .ok()
            .is_some_and(|revoked| revoked.contains_key(id))
    }

    /// Whether the declared world differs from the published set, so turn
    /// admission knows a pass is worth running before it binds an epoch.
    pub async fn has_pending_changes(&self) -> bool {
        let declarations = self.provider.declare();
        let current = self.current.read().await;
        declarations.len() != current.all().count()
            || declarations.iter().any(|d| match current.get(&d.id) {
                None => true,
                Some(existing) => {
                    existing.digest != d.digest || existing.configuration != d.configuration
                }
            })
    }

    /// Whether `id` is revoked right now, regardless of the caller's epoch.
    /// Dispatch checks this; availability comes from the bound epoch, but
    /// authorization always comes from the present.
    pub async fn revocation(&self, id: &CapabilityId) -> Option<String> {
        self.revoked.read().await.get(id).cloned()
    }

    pub async fn revoked_ids(&self) -> BTreeSet<CapabilityId> {
        self.revoked.read().await.keys().cloned().collect()
    }

    /// One idempotent pass. Safe to call at any time, from anywhere, as
    /// often as you like — that is the whole point of a level-triggered
    /// design. Returns the delta if a new epoch was published.
    pub async fn reconcile(&self) -> Option<CapabilityDelta> {
        let now = SystemTime::now();
        let previous = self.current.read().await.clone();
        let declarations = self.provider.declare();

        // A revoked capability is retired in the published set as well as
        // blocked at dispatch, so the prompt stops advertising it at the next
        // turn instead of describing a tool that will always refuse.
        let revoked = self.revoked.read().await.clone();
        let capabilities: Vec<Capability> = declarations
            .into_iter()
            .map(|declaration| Capability {
                resolution: match revoked.get(&declaration.id) {
                    Some(reason) => Resolution::Retired {
                        reason: reason.clone(),
                    },
                    None => Resolution::Available,
                },
                id: declaration.id,
                origin: declaration.origin,
                summary: declaration.summary,
                serves: declaration.serves,
                digest: declaration.digest,
                source: declaration.source,
                configuration: declaration.configuration,
            })
            .collect();

        let candidate = CapabilitySet::new(previous.epoch, capabilities);

        let mut status = self.status.write().await;
        status.passes += 1;
        status.last_run = Some(now);

        if candidate.digest == previous.digest {
            status.epoch = previous.epoch;
            return None;
        }

        // Publish. Only here does the epoch move, and only because the world
        // actually differs.
        let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed);
        let published = Arc::new(CapabilitySet::new(
            epoch,
            candidate.all().cloned().collect(),
        ));
        let delta = published.delta_from(&previous);
        *self.current.write().await = published;
        status.epoch = epoch;
        status.last_change = Some(now);
        Some(delta)
    }

    /// Run the loop until `shutdown` fires.
    ///
    /// Selects over the ticker and the hint channel; a burst of hints is
    /// coalesced into one pass. The ticker is what makes a dropped hint a
    /// latency problem rather than a correctness one.
    pub async fn run(
        self: Arc<Self>,
        mut hints: mpsc::UnboundedReceiver<Hint>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut ticker = tokio::time::interval(RECONCILE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => {}
                hint = hints.recv() => {
                    if hint.is_none() {
                        return;
                    }
                    // Coalesce a burst: an editor saving six files should
                    // produce one pass, not six.
                    tokio::time::sleep(DEBOUNCE).await;
                    while hints.try_recv().is_ok() {}
                }
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        return;
                    }
                    continue;
                }
            }
            self.reconcile().await;
            self.provider.upkeep().await;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use vak_session::types::CapabilityKind;

    /// A provider whose declarations the test drives.
    struct Fake {
        declarations: Mutex<Vec<Declaration>>,
    }

    impl Fake {
        fn new(declarations: Vec<Declaration>) -> Arc<Self> {
            Arc::new(Fake {
                declarations: Mutex::new(declarations),
            })
        }
    }

    #[async_trait]
    impl CapabilityProvider for Fake {
        fn declare(&self) -> Vec<Declaration> {
            self.declarations.lock().unwrap().clone()
        }
    }

    fn decl(name: &str, kind: CapabilityKind) -> Declaration {
        Declaration {
            id: CapabilityId::new(kind, name),
            origin: Origin::Workspace,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            configuration: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn a_first_pass_publishes_an_epoch() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool)]);
        let (registry, _rx) = CapabilityRegistry::new(provider);
        assert!(registry.reconcile().await.is_some());
        let set = registry.current().await;
        assert_eq!(set.epoch, 1);
        assert_eq!(set.usable().count(), 1);
    }

    #[tokio::test]
    async fn reconciling_an_unchanged_world_publishes_nothing() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool)]);
        let (registry, _rx) = CapabilityRegistry::new(provider);
        registry.reconcile().await;
        let first = registry.current().await.epoch;
        assert!(registry.reconcile().await.is_none(), "idempotent");
        assert!(!registry.has_pending_changes().await);
        assert_eq!(registry.current().await.epoch, first, "no epoch churn");
    }

    /// What the MCP pool observes arrives as declared configuration, and a
    /// change to it is a change to the world: a new epoch, picked up at the
    /// next turn with no restart.
    #[tokio::test]
    async fn an_observed_catalog_publishes_a_new_epoch() {
        let provider = Fake::new(vec![decl("search", CapabilityKind::McpServer)]);
        let (registry, _rx) = CapabilityRegistry::new(provider.clone());
        registry.reconcile().await;
        let before = registry.current().await.epoch;

        provider.declarations.lock().unwrap()[0].configuration =
            serde_json::json!({"tools": [{"name": "query"}]});
        assert!(registry.has_pending_changes().await);
        assert!(registry.reconcile().await.is_some());
        let set = registry.current().await;
        assert!(set.epoch > before);
        assert_eq!(
            set.get(&CapabilityId::new(CapabilityKind::McpServer, "search"))
                .unwrap()
                .configuration,
            serde_json::json!({"tools": [{"name": "query"}]})
        );
    }

    #[tokio::test]
    async fn a_capability_added_later_appears_without_a_restart() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool)]);
        let (registry, _rx) = CapabilityRegistry::new(provider.clone());
        registry.reconcile().await;
        let before = registry.current().await.epoch;

        provider
            .declarations
            .lock()
            .unwrap()
            .push(decl("pdf", CapabilityKind::Skill));

        let delta = registry.reconcile().await.expect("a change was published");
        assert_eq!(delta.added.len(), 1);
        assert!(registry.current().await.epoch > before);
        assert_eq!(registry.current().await.usable().count(), 2);
    }

    #[tokio::test]
    async fn removing_a_capability_at_source_removes_it_from_the_set() {
        let provider = Fake::new(vec![
            decl("read", CapabilityKind::Tool),
            decl("pdf", CapabilityKind::Skill),
        ]);
        let (registry, _rx) = CapabilityRegistry::new(provider.clone());
        registry.reconcile().await;
        provider
            .declarations
            .lock()
            .unwrap()
            .retain(|d| d.id.name != "pdf");
        let delta = registry.reconcile().await.expect("removal is a change");
        assert_eq!(delta.removed.len(), 1);
        assert!(delta.describe().contains("no longer available"));
    }

    #[tokio::test]
    async fn revocation_is_immediate_and_independent_of_any_epoch() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool)]);
        let (registry, _rx) = CapabilityRegistry::new(provider);
        registry.reconcile().await;
        let id = CapabilityId::new(CapabilityKind::Tool, "read");
        assert!(registry.revocation(&id).await.is_none());

        registry.revoke(id.clone(), "operator disabled").await;
        // Visible at dispatch immediately, with no reconcile in between.
        assert_eq!(
            registry.revocation(&id).await.as_deref(),
            Some("operator disabled")
        );
        // And retired from the published set on the next pass.
        registry.reconcile().await;
        assert_eq!(registry.current().await.usable().count(), 0);
        assert_eq!(
            registry.current_blocking().unusable().count(),
            1,
            "the synchronous accessor reads the same published set"
        );
    }

    #[tokio::test]
    async fn restoring_a_revoked_capability_requires_a_fresh_published_epoch() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool)]);
        let (registry, _rx) = CapabilityRegistry::new(provider);
        registry.reconcile().await;
        let id = CapabilityId::new(CapabilityKind::Tool, "read");
        let before = registry.current().await.epoch;

        registry.revoke(id.clone(), "operator disabled").await;
        registry.reconcile().await;
        assert!(registry.revoked_now(&id));
        assert_eq!(registry.current().await.usable().count(), 0);

        registry.restore(&id).await;
        // Restore only removes the immediate deny. It cannot resurrect a
        // capability in a bound turn before reconciliation publishes it.
        assert!(!registry.revoked_now(&id));
        assert_eq!(registry.current().await.usable().count(), 0);
        registry.reconcile().await;
        assert!(registry.current().await.epoch > before);
        assert_eq!(registry.current().await.usable().count(), 1);
    }
}
