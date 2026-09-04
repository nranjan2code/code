//! End-to-end guards for the capability lifecycle
//! (docs/design/41-capability-registry.md).
//!
//! Every test here pins a behaviour that was broken in a way no error
//! surfaced: the agent did not fail, it answered from memory and sounded
//! certain. That invisibility is why these are integration tests rather than
//! notes in a changelog.

use std::collections::BTreeSet;

use vak_core::capability::registry::{CapabilityProvider, Declaration, ProbeFailure, ProbeReport};
use vak_core::capability::{
    Capability, CapabilityId, CapabilityRegistry, CapabilitySet, Domain, Origin, Resolution,
    Serves, standing_section,
};
use vak_session::types::CapabilityKind;

// ---------------------------------------------------------------- fakes ---

struct Fake {
    declarations: std::sync::Mutex<Vec<Declaration>>,
    healthy: std::sync::Mutex<bool>,
}

impl Fake {
    fn new(declarations: Vec<Declaration>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Fake {
            declarations: std::sync::Mutex::new(declarations),
            healthy: std::sync::Mutex::new(true),
        })
    }
}

#[async_trait::async_trait]
impl CapabilityProvider for Fake {
    fn declare(&self) -> Vec<Declaration> {
        self.declarations.lock().unwrap().clone()
    }
    async fn probe(&self, _id: &CapabilityId) -> Result<ProbeReport, ProbeFailure> {
        if *self.healthy.lock().unwrap() {
            Ok(ProbeReport {
                configuration: serde_json::json!({"tools": [{"name": "search"}]}),
                announces_changes: false,
            })
        } else {
            Err(ProbeFailure::new(
                "connection refused",
                "check the entry under [mcp.servers]",
            ))
        }
    }
}

fn declaration(name: &str, kind: CapabilityKind, needs_probe: bool) -> Declaration {
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

// ------------------------------------------------------------- the case ---

/// The reported defect, at the layer that caused it.
///
/// A configured search server must be *usable* — present in the descriptors
/// the prompt and tool vector are built from — as soon as it answers a probe.
/// It used to depend on winning a race against session admission, and losing
/// that race was permanent for the life of the session.
#[tokio::test]
async fn a_configured_server_that_answers_is_usable() {
    let provider = Fake::new(vec![declaration("tavily", CapabilityKind::McpServer, true)]);
    let (registry, _hints) = CapabilityRegistry::new(provider);
    registry.reconcile().await;

    let set = registry.current().await;
    assert_eq!(set.usable().count(), 1);
    assert!(set.descriptors().iter().any(|d| d.name == "tavily"));
}

/// A server that is down must never be advertised as usable, and must never
/// appear as a tool named `error`.
#[tokio::test]
async fn a_server_that_is_down_is_named_and_never_faked() {
    let provider = Fake::new(vec![declaration("tavily", CapabilityKind::McpServer, true)]);
    *provider.healthy.lock().unwrap() = false;
    let (registry, _hints) = CapabilityRegistry::new(provider);
    registry.reconcile().await;

    let set = registry.current().await;
    assert_eq!(set.usable().count(), 0, "a dead server is not usable");
    assert!(
        !set.descriptors().iter().any(|d| d.name == "error"),
        "a failure must never become a callable tool"
    );

    // And the model is told, with the fix, rather than left to guess.
    let section = standing_section(&set, None);
    assert!(section.contains("tavily"));
    assert!(section.contains("connection refused"));
    assert!(section.contains("[mcp.servers]"));
    assert!(
        section.contains("do not answer as though you had the data"),
        "the instruction that stops a confident wrong answer"
    );
}

/// Recovery with no restart and no session rotation. This is the property
/// the old retry guard made impossible: a failure was cached as a success,
/// so `inventory.is_none()` never fired again.
#[tokio::test]
async fn a_server_recovers_without_a_restart() {
    let provider = Fake::new(vec![declaration("tavily", CapabilityKind::McpServer, true)]);
    *provider.healthy.lock().unwrap() = false;
    let (registry, _hints) = CapabilityRegistry::new(provider.clone());
    registry.reconcile().await;
    assert_eq!(registry.current().await.usable().count(), 0);

    // The operator fixes it. No process restart, no new session.
    *provider.healthy.lock().unwrap() = true;
    registry
        .mark_due(&CapabilityId::new(CapabilityKind::McpServer, "tavily"))
        .await;
    registry.reconcile().await;

    assert_eq!(
        registry.current().await.usable().count(),
        1,
        "a recovered server must become usable again on its own"
    );
}

// ------------------------------------------------------------ lifecycle ---

/// Add, edit and remove, on a registry that is never restarted.
#[tokio::test]
async fn capabilities_can_be_added_edited_and_removed_while_running() {
    let provider = Fake::new(vec![declaration("read", CapabilityKind::Tool, false)]);
    let (registry, _hints) = CapabilityRegistry::new(provider.clone());
    registry.reconcile().await;
    let first = registry.current().await.epoch;

    // --- added -----------------------------------------------------------
    provider
        .declarations
        .lock()
        .unwrap()
        .push(declaration("pdf", CapabilityKind::Skill, false));
    let delta = registry.reconcile().await.expect("adding is a change");
    assert_eq!(delta.added.len(), 1);
    assert!(delta.describe().contains("now available"));
    let second = registry.current().await.epoch;
    assert!(second > first, "a change publishes a new epoch");

    // --- edited ----------------------------------------------------------
    provider
        .declarations
        .lock()
        .unwrap()
        .iter_mut()
        .filter(|d| d.id.name == "pdf")
        .for_each(|d| d.digest = Some("rewritten".into()));
    let delta = registry.reconcile().await.expect("editing is a change");
    assert_eq!(delta.updated.len(), 1);
    assert!(registry.current().await.epoch > second);

    // --- removed ---------------------------------------------------------
    provider
        .declarations
        .lock()
        .unwrap()
        .retain(|d| d.id.name != "pdf");
    let delta = registry.reconcile().await.expect("removing is a change");
    assert_eq!(delta.removed.len(), 1);
    assert!(
        delta.describe().contains("no longer available"),
        "removal is announced, not silent"
    );
}

/// Reconciling an unchanged world must publish nothing.
///
/// A loop that ran every ten seconds for three weeks and republished each
/// time would churn every live session's prompt and defeat the point.
#[tokio::test]
async fn a_quiet_world_never_churns_the_epoch() {
    let provider = Fake::new(vec![declaration("read", CapabilityKind::Tool, false)]);
    let (registry, _hints) = CapabilityRegistry::new(provider);
    registry.reconcile().await;
    let epoch = registry.current().await.epoch;
    for _ in 0..25 {
        assert!(registry.reconcile().await.is_none());
    }
    assert_eq!(registry.current().await.epoch, epoch);
}

/// A failing server on a backoff rhythm must not republish an epoch on every
/// pass — the digest deliberately excludes volatile retry bookkeeping.
#[tokio::test]
async fn a_persistently_failing_server_does_not_churn_the_epoch() {
    let provider = Fake::new(vec![declaration("tavily", CapabilityKind::McpServer, true)]);
    *provider.healthy.lock().unwrap() = false;
    let (registry, _hints) = CapabilityRegistry::new(provider.clone());
    registry.reconcile().await;
    let epoch = registry.current().await.epoch;

    let id = CapabilityId::new(CapabilityKind::McpServer, "tavily");
    for _ in 0..10 {
        registry.mark_due(&id).await;
        registry.reconcile().await;
    }
    assert_eq!(
        registry.current().await.epoch,
        epoch,
        "repeated failure is not repeated news"
    );
}

// ----------------------------------------------------------- revocation ---

/// Revocation is immediate and independent of any epoch. An operator
/// disabling a compromised plugin must not wait for a long turn to finish.
#[tokio::test]
async fn revocation_applies_immediately_and_restoration_republishes() {
    let provider = Fake::new(vec![declaration("bash", CapabilityKind::Tool, false)]);
    let (registry, _hints) = CapabilityRegistry::new(provider);
    registry.reconcile().await;
    let id = CapabilityId::new(CapabilityKind::Tool, "bash");

    registry.revoke(id.clone(), "operator disabled").await;
    assert_eq!(
        registry.revocation(&id).await.as_deref(),
        Some("operator disabled"),
        "visible at dispatch with no reconcile in between"
    );

    registry.reconcile().await;
    assert_eq!(registry.current().await.usable().count(), 0);

    registry.restore(&id).await;
    registry.reconcile().await;
    assert_eq!(registry.current().await.usable().count(), 1);
}

// --------------------------------------------------------------- slices ---

/// The slice must never remove a capability that has not classified itself.
/// This is what keeps a freshly installed integration reachable with no
/// configuration and no harness edit.
#[test]
fn an_undeclared_capability_survives_every_slice() {
    let mut declared = Capability {
        id: CapabilityId::new(CapabilityKind::Tool, "bash"),
        origin: Origin::Builtin,
        summary: String::new(),
        serves: Serves::declared([Domain::CodeExec]),
        digest: None,
        source: None,
        resolution: Resolution::Static,
        configuration: serde_json::Value::Null,
    };
    let mut undeclared = declared.clone();
    undeclared.id = CapabilityId::new(CapabilityKind::Tool, "installed_thing");
    undeclared.serves = Serves::Undeclared;
    declared.serves = Serves::declared([Domain::CodeExec]);

    let set = CapabilitySet::new(1, vec![declared, undeclared]);
    let required = BTreeSet::from([Domain::LiveData]);
    let kept: Vec<String> = set
        .sliced_to(&required)
        .into_iter()
        .map(|c| c.id.name.clone())
        .collect();

    assert!(kept.contains(&"installed_thing".to_string()));
    assert!(!kept.contains(&"bash".to_string()));
}

/// A domain this build has never heard of must round-trip and match itself,
/// so a plugin with its own vocabulary is not silently flattened.
#[test]
fn an_unknown_domain_survives_and_matches() {
    let parsed = Domain::parse("procurement");
    assert_eq!(parsed, Domain::Custom("procurement".into()));
    let serves = Serves::declared([parsed.clone()]);
    assert!(serves.serves_any(&BTreeSet::from([parsed])));
    assert!(!serves.serves_any(&BTreeSet::from([Domain::Web])));
}
