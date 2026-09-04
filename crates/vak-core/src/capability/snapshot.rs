//! Immutable, versioned capability sets, and what binds to one.
//!
//! The unit of atomicity is the **turn**, not the session. A turn takes one
//! `Arc<CapabilitySet>` at its start and holds it to the end, so a plan
//! formed in step one cannot have its tools change by step four. Between
//! turns the session re-binds to whatever the registry has published since.
//!
//! That is what makes session rotation unnecessary rather than merely
//! forbidden. The previous design froze a *copy* of the capability list into
//! the session header, which made a session born during a slow discovery
//! pass permanently degraded — its only escape hatches were restarting the
//! process or rotating the session, and both are ruled out. Binding by
//! epoch instead means a three-week-old conversation picks up a skill you
//! add today, at its next turn, with no restart and no rotation.
//!
//! Audit gets stronger, not weaker: previously you could reconstruct what a
//! session was *born* with; now every turn records the epoch it ran at and
//! every transition is a ledger entry, so you can reconstruct what each turn
//! actually saw.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use vak_session::types::{CapabilityDescriptor, CapabilityKind};

use super::domain::{Domain, Serves};
use super::resolution::Resolution;

/// A monotonic version of the whole capability set. Never reused, never
/// decreases, and bumped only when a reconcile pass produces a set that
/// differs from the last published one.
pub type Epoch = u64;

/// Where a capability came from. Kept structured rather than as a display
/// string, because `reach` used to recover a server name by string-parsing a
/// human-readable label — a rendering decision that had become load-bearing
/// for a policy decision.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "kebab-case")]
pub enum Origin {
    /// Compiled in.
    Builtin,
    /// The workspace's own `.vak` directory.
    Workspace,
    /// The shared/user-level capability root.
    Shared,
    /// Contributed by an installed plugin.
    Plugin { plugin: String, scope: String },
    /// Injected at runtime by a host surface.
    Runtime,
}

impl Origin {
    pub fn label(&self) -> String {
        match self {
            Origin::Builtin => "builtin".into(),
            Origin::Workspace => "workspace".into(),
            Origin::Shared => "shared".into(),
            Origin::Plugin { plugin, scope } => format!("plugin:{scope}:{plugin}"),
            Origin::Runtime => "runtime".into(),
        }
    }
}

/// A capability's stable identity: kind plus name.
///
/// Kind-qualified because a skill named `pdf` and an MCP server named `pdf`
/// are different things, and a registry keyed on the bare name would let one
/// shadow the other silently.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CapabilityId {
    pub kind: CapabilityKind,
    pub name: String,
}

impl CapabilityId {
    pub fn new(kind: CapabilityKind, name: impl Into<String>) -> Self {
        CapabilityId {
            kind,
            name: name.into(),
        }
    }
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.kind {
            CapabilityKind::Tool => "tool",
            CapabilityKind::Skill => "skill",
            CapabilityKind::McpServer => "mcp",
            CapabilityKind::Hook => "hook",
            CapabilityKind::Command => "command",
        };
        write!(f, "{kind}:{}", self.name)
    }
}

/// One capability, as the registry knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    pub origin: Origin,
    pub summary: String,
    /// What the capability says it is for. See `domain.rs` — undeclared is
    /// a real answer and is never sliced away.
    pub serves: Serves,
    /// Content digest where the capability has content (a skill body).
    pub digest: Option<String>,
    pub source: Option<std::path::PathBuf>,
    pub resolution: Resolution,
    /// Kind-specific frozen configuration: tool schemas, command templates,
    /// hook lifecycle options, a discovered MCP catalog.
    #[serde(default)]
    pub configuration: serde_json::Value,
}

impl Capability {
    /// Whether a turn bound to this set may use it right now.
    pub fn is_usable(&self) -> bool {
        self.resolution.is_usable()
    }

    /// Project back into the wire type the session header and prompt already
    /// speak, so nothing downstream has to learn a second representation.
    pub fn to_descriptor(&self) -> CapabilityDescriptor {
        use vak_session::types::CapabilityInvocation;
        let invocation = match self.id.kind {
            CapabilityKind::Tool | CapabilityKind::McpServer => CapabilityInvocation::ModelTool,
            CapabilityKind::Skill => CapabilityInvocation::SkillLoader,
            CapabilityKind::Hook => CapabilityInvocation::Automatic,
            CapabilityKind::Command => CapabilityInvocation::UserCommand,
        };
        CapabilityDescriptor {
            name: self.id.name.clone(),
            kind: self.id.kind.clone(),
            invocation,
            description: self.summary.clone(),
            source: self.source.clone(),
            digest: self.digest.clone(),
            provenance: Some(self.origin.label()),
            configuration: self.configuration.clone(),
        }
    }
}

/// An immutable published capability set.
///
/// Shared as `Arc<CapabilitySet>` and **refcounted, not retained**: the
/// registry keeps only the current one, so an old epoch lives exactly as
/// long as the last turn holding it. A design that kept every version would
/// climb steadily over weeks of edits and end in an OOM, which is the one
/// failure a no-restart system cannot absorb.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub epoch: Epoch,
    pub published_at: SystemTime,
    /// Ordered by id, so the digest is stable across reconcile passes that
    /// discovered the same things in a different order.
    capabilities: BTreeMap<CapabilityId, Capability>,
    /// Content digest of the usable set. Two epochs with the same digest
    /// describe the same world.
    pub digest: String,
}

impl CapabilitySet {
    pub fn new(epoch: Epoch, capabilities: Vec<Capability>) -> Self {
        let map: BTreeMap<CapabilityId, Capability> = capabilities
            .into_iter()
            .map(|c| (c.id.clone(), c))
            .collect();
        let digest = Self::digest_of(&map);
        CapabilitySet {
            epoch,
            published_at: SystemTime::now(),
            capabilities: map,
            digest,
        }
    }

    pub fn empty() -> Self {
        CapabilitySet::new(0, Vec::new())
    }

    /// Digest over identity, usability and content — the things that change
    /// what a turn can do. Deliberately *excludes* volatile resolution
    /// detail like `retry_at` and attempt counts, so a server failing on a
    /// backoff rhythm does not publish a new epoch every few seconds and
    /// churn every live session's prompt.
    fn digest_of(map: &BTreeMap<CapabilityId, Capability>) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for (id, capability) in map {
            hasher.update(id.to_string().as_bytes());
            hasher.update([u8::from(capability.is_usable())]);
            hasher.update(capability.digest.clone().unwrap_or_default().as_bytes());
            hasher.update(capability.summary.as_bytes());
            hasher.update(capability.origin.label().as_bytes());
            for label in capability.serves.labels() {
                hasher.update(label.as_bytes());
            }
            hasher.update(capability.configuration.to_string().as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }

    pub fn get(&self, id: &CapabilityId) -> Option<&Capability> {
        self.capabilities.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities.values()
    }

    /// Everything a turn may actually use.
    pub fn usable(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities.values().filter(|c| c.is_usable())
    }

    pub fn of_kind(&self, kind: CapabilityKind) -> impl Iterator<Item = &Capability> {
        self.capabilities
            .values()
            .filter(move |c| c.id.kind == kind && c.is_usable())
    }

    /// Configured but not usable, with the reason. The counterpart the
    /// operator report and the model's standing section both render.
    pub fn unusable(&self) -> impl Iterator<Item = &Capability> {
        self.capabilities.values().filter(|c| !c.is_usable())
    }

    /// The descriptor vector the session header and prompt speak.
    pub fn descriptors(&self) -> Vec<CapabilityDescriptor> {
        self.usable().map(|c| c.to_descriptor()).collect()
    }

    /// Narrow to the capabilities serving at least one required domain.
    ///
    /// Strictly subtractive, and only `Tool` entries are sliced: a skill is
    /// already progressively disclosed by its loader and an MCP server is
    /// already lazy, so slicing those spends risk for no context saving.
    /// Undeclared capabilities always survive.
    pub fn sliced_to(&self, required: &std::collections::BTreeSet<Domain>) -> Vec<&Capability> {
        self.usable()
            .filter(|c| {
                if c.id.kind != CapabilityKind::Tool {
                    return true;
                }
                c.serves.serves_any(required)
            })
            .collect()
    }

    /// What changed between two published sets.
    pub fn delta_from(&self, previous: &CapabilitySet) -> CapabilityDelta {
        let mut delta = CapabilityDelta::default();
        for (id, capability) in &self.capabilities {
            match previous.capabilities.get(id) {
                None => delta.added.push(id.clone()),
                Some(before) => {
                    if before.is_usable() != capability.is_usable()
                        || before.digest != capability.digest
                        || before.summary != capability.summary
                    {
                        delta.updated.push(id.clone());
                    }
                }
            }
        }
        for id in previous.capabilities.keys() {
            if !self.capabilities.contains_key(id) {
                delta.removed.push(id.clone());
            }
        }
        delta
    }
}

/// What one epoch transition changed, for the ledger entry and the notice
/// the model gets on its next turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDelta {
    pub added: Vec<CapabilityId>,
    pub updated: Vec<CapabilityId>,
    pub removed: Vec<CapabilityId>,
}

impl CapabilityDelta {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.updated.is_empty() && self.removed.is_empty()
    }

    /// A sentence for the model's standing section, so a capability that
    /// appeared or vanished under a live session is announced rather than
    /// silently changing what works.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        let names = |ids: &[CapabilityId]| {
            ids.iter()
                .map(|id| format!("`{id}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !self.added.is_empty() {
            parts.push(format!("now available: {}", names(&self.added)));
        }
        if !self.updated.is_empty() {
            parts.push(format!("changed: {}", names(&self.updated)));
        }
        if !self.removed.is_empty() {
            parts.push(format!("no longer available: {}", names(&self.removed)));
        }
        parts.join("; ")
    }
}

/// What a turn holds: one snapshot, for its whole duration.
#[derive(Debug, Clone)]
pub struct Binding {
    pub set: Arc<CapabilitySet>,
    /// The delta from the epoch the session was previously bound to, if it
    /// moved. Rendered into the standing section exactly once.
    pub delta: Option<CapabilityDelta>,
}

impl Binding {
    pub fn epoch(&self) -> Epoch {
        self.set.epoch
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn cap(name: &str, kind: CapabilityKind, usable: bool) -> Capability {
        Capability {
            id: CapabilityId::new(kind, name),
            origin: Origin::Builtin,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            resolution: if usable {
                Resolution::Static
            } else {
                Resolution::Degraded {
                    failure: super::super::resolution::Failure::new("down", ""),
                    retry_at: SystemTime::now(),
                }
            },
            configuration: serde_json::Value::Null,
        }
    }

    #[test]
    fn unusable_capabilities_are_excluded_from_descriptors() {
        let set = CapabilitySet::new(
            1,
            vec![
                cap("read", CapabilityKind::Tool, true),
                cap("tavily", CapabilityKind::McpServer, false),
            ],
        );
        let names: Vec<_> = set.descriptors().into_iter().map(|d| d.name).collect();
        assert_eq!(names, vec!["read"]);
        assert_eq!(set.unusable().count(), 1);
    }

    #[test]
    fn the_digest_ignores_volatile_retry_detail() {
        // Two sets identical except for backoff bookkeeping must share a
        // digest, or a failing server republishes an epoch every few seconds.
        let mut a = cap("tavily", CapabilityKind::McpServer, false);
        let mut b = a.clone();
        a.resolution = Resolution::Static.failed(SystemTime::now(), "down", "");
        b.resolution = Resolution::Static
            .failed(SystemTime::now(), "down", "")
            .failed(SystemTime::now(), "down", "");
        let set_a = CapabilitySet::new(1, vec![a]);
        let set_b = CapabilitySet::new(2, vec![b]);
        assert_eq!(set_a.digest, set_b.digest);
    }

    #[test]
    fn a_kind_change_is_a_different_capability() {
        let set = CapabilitySet::new(
            1,
            vec![
                cap("pdf", CapabilityKind::Skill, true),
                cap("pdf", CapabilityKind::McpServer, true),
            ],
        );
        assert_eq!(
            set.usable().count(),
            2,
            "kind-qualified ids must not shadow"
        );
    }

    #[test]
    fn delta_names_what_moved() {
        let before = CapabilitySet::new(1, vec![cap("read", CapabilityKind::Tool, true)]);
        let after = CapabilitySet::new(
            2,
            vec![
                cap("read", CapabilityKind::Tool, true),
                cap("weather", CapabilityKind::McpServer, true),
            ],
        );
        let delta = after.delta_from(&before);
        assert_eq!(
            delta.added,
            vec![CapabilityId::new(CapabilityKind::McpServer, "weather")]
        );
        assert!(delta.removed.is_empty());
        assert!(delta.describe().contains("now available"));
    }

    #[test]
    fn losing_usability_reads_as_an_update_not_a_removal() {
        let before = CapabilitySet::new(1, vec![cap("tavily", CapabilityKind::McpServer, true)]);
        let after = CapabilitySet::new(2, vec![cap("tavily", CapabilityKind::McpServer, false)]);
        let delta = after.delta_from(&before);
        assert!(delta.removed.is_empty(), "still configured, just unusable");
        assert_eq!(delta.updated.len(), 1);
    }

    #[test]
    fn slicing_never_removes_a_non_tool_or_an_undeclared_tool() {
        let required = std::collections::BTreeSet::from([Domain::LiveData]);
        let mut filesystem_tool = cap("read", CapabilityKind::Tool, true);
        filesystem_tool.serves = Serves::declared([Domain::Filesystem]);
        let set = CapabilitySet::new(
            1,
            vec![
                filesystem_tool,
                cap("bash", CapabilityKind::Tool, true), // undeclared
                cap("pdf", CapabilityKind::Skill, true),
            ],
        );
        let kept: Vec<_> = set
            .sliced_to(&required)
            .into_iter()
            .map(|c| c.id.name.clone())
            .collect();
        assert!(kept.contains(&"bash".to_string()), "undeclared fails open");
        assert!(kept.contains(&"pdf".to_string()), "skills are not sliced");
        assert!(!kept.contains(&"read".to_string()));
    }
}
