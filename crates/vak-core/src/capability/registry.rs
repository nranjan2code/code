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

/// One capability as its source declares it, before any probing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub id: CapabilityId,
    pub origin: Origin,
    pub summary: String,
    pub serves: Serves,
    pub digest: Option<String>,
    pub source: Option<std::path::PathBuf>,
    pub configuration: serde_json::Value,
    /// Whether this capability talks to something outside the process and
    /// therefore has to be probed before it can be called usable. True for
    /// MCP servers; false for everything knowable offline.
    pub needs_probe: bool,
}

/// What a successful probe learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// Discovered detail (an MCP tool catalog) folded into `configuration`.
    pub configuration: serde_json::Value,
    /// Whether the source announces its own changes. When false the loop
    /// re-probes on a rhythm, because silence carries no information.
    pub announces_changes: bool,
}

/// Where declarations come from and how probing happens.
///
/// Implemented by `Core`, which already owns skill/hook/command/MCP
/// discovery. Keeping it behind a trait means the loop is testable without a
/// workspace, a filesystem, or a live MCP server.
#[async_trait]
pub trait CapabilityProvider: Send + Sync {
    /// Everything currently declared. Cheap and offline: a filesystem walk
    /// at worst, never a network call.
    fn declare(&self) -> Vec<Declaration>;

    /// Probe one capability that declared `needs_probe`.
    async fn probe(&self, id: &CapabilityId) -> Result<ProbeReport, ProbeFailure>;

    /// Housekeeping on each pass: evicting idle connections, etc. Default is
    /// nothing.
    async fn upkeep(&self) {}
}

/// A probe that did not succeed, in terms an operator can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeFailure {
    pub reason: String,
    pub remedy: String,
}

impl ProbeFailure {
    pub fn new(reason: impl Into<String>, remedy: impl Into<String>) -> Self {
        ProbeFailure {
            reason: reason.into(),
            remedy: remedy.into(),
        }
    }
}

/// Why the loop woke up. Purely an optimisation — the ticker would have
/// caught all of these eventually.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hint {
    /// A watched capability directory changed.
    SourceChanged(String),
    /// An MCP server announced a catalog change.
    ServerAnnounced(String),
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
    pub probes_run: u64,
    pub probes_failed: u64,
}

/// The registry.
pub struct CapabilityRegistry {
    provider: Arc<dyn CapabilityProvider>,
    current: RwLock<Arc<CapabilitySet>>,
    next_epoch: AtomicU64,
    /// Resolutions carried across passes so backoff accumulates rather than
    /// resetting every time the loop runs.
    resolutions: RwLock<BTreeMap<CapabilityId, Resolution>>,
    /// The immediate channel. Checked at dispatch, independent of any epoch.
    revoked: RwLock<BTreeMap<CapabilityId, String>>,
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
            resolutions: RwLock::new(BTreeMap::new()),
            revoked: RwLock::new(BTreeMap::new()),
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

    pub async fn status(&self) -> ReconcileStatus {
        self.status.read().await.clone()
    }

    /// Revoke immediately, mid-turn, fail-closed.
    ///
    /// Does not wait for an epoch: a compromised plugin must stop being
    /// callable now, not when the current turn happens to finish. This only
    /// ever narrows, which is why it is safe to apply without admission.
    pub async fn revoke(&self, id: CapabilityId, reason: impl Into<String>) {
        self.revoked.write().await.insert(id, reason.into());
        self.hint(Hint::ConfigChanged);
    }

    pub async fn restore(&self, id: &CapabilityId) {
        self.revoked.write().await.remove(id);
        self.hint(Hint::ConfigChanged);
    }

    /// Force `id` to be probed on the next pass, discarding any backoff.
    ///
    /// Used when something external tells us the world changed — an MCP
    /// server announcing `tools/list_changed`, or an operator saying they
    /// have fixed a server and want it retried now rather than at the end of
    /// a ten-minute backoff. Only ever schedules work; it cannot mark
    /// anything usable on its own.
    pub async fn mark_due(&self, id: &CapabilityId) {
        let mut resolutions = self.resolutions.write().await;
        if let Some(resolution) = resolutions.get_mut(id)
            && !matches!(resolution, Resolution::Static)
        {
            *resolution = Resolution::Probing;
        }
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
        let declarations = self.provider.declare();

        // 1. Carry resolutions forward for ids that survived, so backoff and
        //    catalog TTLs accumulate across passes.
        let mut resolutions = self.resolutions.write().await;
        let declared_ids: BTreeSet<CapabilityId> =
            declarations.iter().map(|d| d.id.clone()).collect();
        resolutions.retain(|id, _| declared_ids.contains(id));
        for declaration in &declarations {
            resolutions.entry(declaration.id.clone()).or_insert({
                if declaration.needs_probe {
                    Resolution::Probing
                } else {
                    Resolution::Static
                }
            });
        }

        // 2. Anything that needs probing and is due, probed concurrently.
        //    Concurrency matters: serial probing made the cost of N dead
        //    servers the sum of their timeouts rather than the max.
        let due: Vec<CapabilityId> = declarations
            .iter()
            .filter(|d| d.needs_probe)
            .filter(|d| {
                resolutions
                    .get(&d.id)
                    .map(|r| r.is_due(now))
                    .unwrap_or(true)
            })
            .map(|d| d.id.clone())
            .collect();
        drop(resolutions);

        let mut probed: BTreeMap<CapabilityId, (Resolution, serde_json::Value)> = BTreeMap::new();
        if !due.is_empty() {
            let results = futures::future::join_all(
                due.iter()
                    .map(|id| async move { (id.clone(), self.provider.probe(id).await) }),
            )
            .await;
            let mut resolutions = self.resolutions.write().await;
            let mut status = self.status.write().await;
            for (id, result) in results {
                status.probes_run += 1;
                match result {
                    Ok(report) => {
                        resolutions
                            .insert(id.clone(), Resolution::ready(now, report.announces_changes));
                        probed.insert(
                            id,
                            (
                                Resolution::ready(now, report.announces_changes),
                                report.configuration,
                            ),
                        );
                    }
                    Err(failure) => {
                        status.probes_failed += 1;
                        let previous = resolutions.get(&id).cloned().unwrap_or(Resolution::Probing);
                        let next = previous.failed(now, failure.reason, failure.remedy);
                        resolutions.insert(id.clone(), next.clone());
                        probed.insert(id, (next, serde_json::Value::Null));
                    }
                }
            }
        }

        // 3. Assemble. A revoked capability is retired in the published set
        //    as well as blocked at dispatch, so the prompt stops advertising
        //    it at the next turn instead of describing a tool that will
        //    always refuse.
        let revoked = self.revoked.read().await.clone();
        let resolutions = self.resolutions.read().await;
        let capabilities: Vec<Capability> = declarations
            .into_iter()
            .map(|declaration| {
                let resolution = if let Some(reason) = revoked.get(&declaration.id) {
                    Resolution::Retired {
                        reason: reason.clone(),
                    }
                } else if let Some((resolution, _)) = probed.get(&declaration.id) {
                    resolution.clone()
                } else {
                    resolutions
                        .get(&declaration.id)
                        .cloned()
                        .unwrap_or(Resolution::Static)
                };
                let configuration = match probed.get(&declaration.id) {
                    Some((_, config)) if !config.is_null() => config.clone(),
                    _ => declaration.configuration,
                };
                Capability {
                    id: declaration.id,
                    origin: declaration.origin,
                    summary: declaration.summary,
                    serves: declaration.serves,
                    digest: declaration.digest,
                    source: declaration.source,
                    resolution,
                    configuration,
                }
            })
            .collect();
        drop(resolutions);

        let previous = self.current.read().await.clone();
        let candidate = CapabilitySet::new(previous.epoch, capabilities);

        let mut status = self.status.write().await;
        status.passes += 1;
        status.last_run = Some(now);

        if candidate.digest == previous.digest {
            status.epoch = previous.epoch;
            return None;
        }

        // 4. Publish. Only here does the epoch move, and only because the
        //    world actually differs — a failing server on a backoff rhythm
        //    does not churn every live session's prompt, because the digest
        //    excludes volatile retry detail.
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
mod tests {
    use super::*;
    use std::sync::Mutex;
    use vak_session::types::CapabilityKind;

    /// A provider whose declarations and probe outcomes the test drives.
    struct Fake {
        declarations: Mutex<Vec<Declaration>>,
        probe_ok: Mutex<bool>,
        probes: Mutex<u32>,
    }

    impl Fake {
        fn new(declarations: Vec<Declaration>) -> Arc<Self> {
            Arc::new(Fake {
                declarations: Mutex::new(declarations),
                probe_ok: Mutex::new(true),
                probes: Mutex::new(0),
            })
        }
    }

    #[async_trait]
    impl CapabilityProvider for Fake {
        fn declare(&self) -> Vec<Declaration> {
            self.declarations.lock().unwrap().clone()
        }
        async fn probe(&self, _id: &CapabilityId) -> Result<ProbeReport, ProbeFailure> {
            *self.probes.lock().unwrap() += 1;
            if *self.probe_ok.lock().unwrap() {
                Ok(ProbeReport {
                    configuration: serde_json::json!({"tools": ["search"]}),
                    announces_changes: true,
                })
            } else {
                Err(ProbeFailure::new("connection refused", "start the server"))
            }
        }
    }

    fn decl(name: &str, kind: CapabilityKind, needs_probe: bool) -> Declaration {
        Declaration {
            id: CapabilityId::new(kind, name),
            origin: Origin::Workspace,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            configuration: serde_json::Value::Null,
            needs_probe,
        }
    }

    #[tokio::test]
    async fn a_first_pass_publishes_an_epoch() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool, false)]);
        let (registry, _rx) = CapabilityRegistry::new(provider);
        assert!(registry.reconcile().await.is_some());
        let set = registry.current().await;
        assert_eq!(set.epoch, 1);
        assert_eq!(set.usable().count(), 1);
    }

    #[tokio::test]
    async fn reconciling_an_unchanged_world_publishes_nothing() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool, false)]);
        let (registry, _rx) = CapabilityRegistry::new(provider);
        registry.reconcile().await;
        let first = registry.current().await.epoch;
        assert!(registry.reconcile().await.is_none(), "idempotent");
        assert!(registry.reconcile().await.is_none());
        assert_eq!(registry.current().await.epoch, first, "no epoch churn");
    }

    #[tokio::test]
    async fn a_failed_probe_is_unusable_and_never_a_fake_tool() {
        let provider = Fake::new(vec![decl("tavily", CapabilityKind::McpServer, true)]);
        *provider.probe_ok.lock().unwrap() = false;
        let (registry, _rx) = CapabilityRegistry::new(provider);
        registry.reconcile().await;
        let set = registry.current().await;
        assert_eq!(set.usable().count(), 0);
        assert_eq!(set.unusable().count(), 1);
        let failed = set.unusable().next().unwrap();
        assert!(matches!(failed.resolution, Resolution::Degraded { .. }));
        // The old bug: a tool literally named `error` in the catalog.
        assert!(!set.descriptors().iter().any(|d| d.name == "error"));
    }

    #[tokio::test]
    async fn a_server_that_recovers_becomes_usable_with_no_restart() {
        let provider = Fake::new(vec![decl("tavily", CapabilityKind::McpServer, true)]);
        *provider.probe_ok.lock().unwrap() = false;
        let (registry, _rx) = CapabilityRegistry::new(provider.clone());
        registry.reconcile().await;
        assert_eq!(registry.current().await.usable().count(), 0);

        // The operator starts the server. Clear the backoff the way the
        // passage of time would, then reconcile again.
        *provider.probe_ok.lock().unwrap() = true;
        registry.resolutions.write().await.insert(
            CapabilityId::new(CapabilityKind::McpServer, "tavily"),
            Resolution::Probing,
        );

        let delta = registry.reconcile().await;
        assert!(delta.is_some(), "recovery must publish a new epoch");
        assert_eq!(
            registry.current().await.usable().count(),
            1,
            "this is the case the old retry guard made impossible"
        );
    }

    #[tokio::test]
    async fn a_capability_added_later_appears_without_a_restart() {
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool, false)]);
        let (registry, _rx) = CapabilityRegistry::new(provider.clone());
        registry.reconcile().await;
        let before = registry.current().await.epoch;

        provider
            .declarations
            .lock()
            .unwrap()
            .push(decl("pdf", CapabilityKind::Skill, false));

        let delta = registry.reconcile().await.expect("a change was published");
        assert_eq!(delta.added.len(), 1);
        assert!(registry.current().await.epoch > before);
        assert_eq!(registry.current().await.usable().count(), 2);
    }

    #[tokio::test]
    async fn removing_a_capability_at_source_removes_it_from_the_set() {
        let provider = Fake::new(vec![
            decl("read", CapabilityKind::Tool, false),
            decl("pdf", CapabilityKind::Skill, false),
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
        let provider = Fake::new(vec![decl("read", CapabilityKind::Tool, false)]);
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
    }

    #[tokio::test]
    async fn backoff_accumulates_across_passes_rather_than_resetting() {
        let provider = Fake::new(vec![decl("tavily", CapabilityKind::McpServer, true)]);
        *provider.probe_ok.lock().unwrap() = false;
        let (registry, _rx) = CapabilityRegistry::new(provider);
        registry.reconcile().await;
        // Not due yet, so a second pass must not probe again.
        registry.reconcile().await;
        let status = registry.status().await;
        assert_eq!(
            status.probes_run, 1,
            "backoff must be honoured across passes"
        );
        assert_eq!(status.probes_failed, 1);
    }
}
