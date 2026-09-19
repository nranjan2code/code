//! One projection, three audiences.
//!
//! Previously the model and the operator read different sources and could
//! disagree. `doctor` counted hooks and MCP servers from raw config while
//! counting skills from the effective set, so a plugin-contributed server was
//! invisible to the operator and present to the model. `reach` recovered a
//! server name by string-parsing a human-readable label, making a rendering
//! decision load-bearing for a policy decision. And `capability_diagnostics`
//! — the one structure that explained *why* something was dropped — was
//! rendered only into the system prompt, so the model was told and the person
//! who could fix the configuration was not.
//!
//! Everything here derives from one published [`CapabilitySet`]. The model's
//! standing section, the `doctor` report, and the admin console render this
//! same value, so they cannot drift apart.

use serde::{Deserialize, Serialize};

use super::registry::ReconcileStatus;
use super::snapshot::{CapabilityDelta, CapabilitySet};

/// One capability, flattened for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRow {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub origin: String,
    pub summary: String,
    /// Declared domains; empty means undeclared, which is never sliced away.
    pub serves: Vec<String>,
    pub usable: bool,
    /// "ready" | "checking" | "unavailable: … " | "removed: …"
    pub status: String,
    /// What the operator can do, when there is something to do.
    pub remedy: String,
    pub source: Option<String>,
}

/// The whole subsystem, in one value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub epoch: u64,
    pub digest: String,
    pub reconcile: ReconcileStatus,
    pub capabilities: Vec<CapabilityRow>,
}

impl CapabilityReport {
    pub fn build(set: &CapabilitySet, reconcile: ReconcileStatus) -> Self {
        let capabilities = set
            .all()
            .map(|capability| {
                let remedy = match &capability.resolution {
                    super::resolution::Resolution::Degraded { failure, .. } => {
                        failure.remedy.clone()
                    }
                    _ => String::new(),
                };
                CapabilityRow {
                    id: capability.id.to_string(),
                    kind: format!("{:?}", capability.id.kind).to_lowercase(),
                    name: capability.id.name.clone(),
                    origin: capability.origin.label(),
                    summary: capability.summary.clone(),
                    serves: capability.serves.labels(),
                    usable: capability.is_usable(),
                    status: capability.resolution.summary(),
                    remedy,
                    source: capability.source.as_ref().map(|p| p.display().to_string()),
                }
            })
            .collect();
        CapabilityReport {
            epoch: set.epoch,
            digest: set.digest.clone(),
            reconcile,
            capabilities,
        }
    }

    pub fn usable(&self) -> impl Iterator<Item = &CapabilityRow> {
        self.capabilities.iter().filter(|row| row.usable)
    }

    pub fn unusable(&self) -> impl Iterator<Item = &CapabilityRow> {
        self.capabilities.iter().filter(|row| !row.usable)
    }

    /// Counts by kind, over the *effective* set — the number `doctor` should
    /// have been showing all along.
    pub fn counts(&self) -> Vec<(String, usize, usize)> {
        let mut kinds: Vec<String> = self
            .capabilities
            .iter()
            .map(|row| row.kind.clone())
            .collect();
        kinds.sort();
        kinds.dedup();
        kinds
            .into_iter()
            .map(|kind| {
                let total = self
                    .capabilities
                    .iter()
                    .filter(|row| row.kind == kind)
                    .count();
                let usable = self
                    .capabilities
                    .iter()
                    .filter(|row| row.kind == kind && row.usable)
                    .count();
                (kind, usable, total)
            })
            .collect()
    }

    /// One line for `doctor`'s facts block.
    pub fn summary_line(&self) -> String {
        let parts = self
            .counts()
            .into_iter()
            .map(|(kind, usable, total)| {
                if usable == total {
                    format!("{usable} {kind}")
                } else {
                    format!("{usable}/{total} {kind}")
                }
            })
            .collect::<Vec<_>>();
        format!("capabilities (epoch {}): {}", self.epoch, parts.join(" · "))
    }
}

/// The model-facing standing section: what is configured but not usable on
/// this turn, and what changed since this session's last turn.
///
/// The wording matters. A model told only "you have no live-data tool" answers
/// from memory and sounds certain; told "the `search` server is configured
/// and currently unreachable", it says so and names the fix.
pub fn standing_section(set: &CapabilitySet, delta: Option<&CapabilityDelta>) -> String {
    let mut out = String::new();

    if let Some(delta) = delta.filter(|d| !d.is_empty()) {
        out.push_str(&format!(
            "\nYour capabilities changed since your last turn — {}.\n",
            delta.describe()
        ));
    }

    let unusable: Vec<_> = set
        .unusable()
        .filter(|c| {
            c.id.kind != vak_session::types::CapabilityKind::Hook
                && c.id.kind != vak_session::types::CapabilityKind::Command
        })
        .collect();
    if !unusable.is_empty() {
        out.push_str(
            "\nConfigured but NOT usable on this turn. These are not in your tool schemas \
             and calling them will fail. If the request needs one, say so plainly, name the \
             capability, and give the operator the fix — do not substitute a different tool \
             and do not answer as though you had the data:\n",
        );
        for capability in unusable {
            out.push_str(&format!(
                "- {}: {}",
                capability.id,
                capability.resolution.summary()
            ));
            if let super::resolution::Resolution::Degraded { failure, .. } = &capability.resolution
                && !failure.remedy.is_empty()
            {
                out.push_str(&format!(" Fix: {}.", failure.remedy));
            }
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::capability::domain::Serves;
    use crate::capability::resolution::{Failure, Resolution};
    use crate::capability::snapshot::{Capability, CapabilityId, Origin};
    use std::time::SystemTime;
    use vak_session::types::CapabilityKind;

    fn broken(name: &str) -> Capability {
        Capability {
            id: CapabilityId::new(CapabilityKind::McpServer, name),
            origin: Origin::Workspace,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            resolution: Resolution::Degraded {
                failure: Failure::new("connection refused", "run `vak mcp start tavily`"),
                retry_at: SystemTime::now() + std::time::Duration::from_secs(30),
            },
            configuration: serde_json::Value::Null,
        }
    }

    fn ok(name: &str, kind: CapabilityKind) -> Capability {
        Capability {
            id: CapabilityId::new(kind, name),
            origin: Origin::Builtin,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            resolution: Resolution::Static,
            configuration: serde_json::Value::Null,
        }
    }

    #[test]
    fn the_operator_sees_the_same_failure_the_model_does() {
        let set = CapabilitySet::new(7, vec![ok("read", CapabilityKind::Tool), broken("tavily")]);
        let report = CapabilityReport::build(&set, ReconcileStatus::default());
        let row = report.unusable().next().expect("one unusable");
        assert!(row.status.contains("connection refused"));
        assert!(row.remedy.contains("vak mcp start"));

        let prompt = standing_section(&set, None);
        assert!(prompt.contains("connection refused"));
        assert!(prompt.contains("vak mcp start"));
    }

    #[test]
    fn counts_come_from_the_effective_set_not_raw_config() {
        let set = CapabilitySet::new(
            1,
            vec![
                ok("read", CapabilityKind::Tool),
                ok("pdf", CapabilityKind::Skill),
                broken("tavily"),
            ],
        );
        let report = CapabilityReport::build(&set, ReconcileStatus::default());
        let line = report.summary_line();
        assert!(
            line.contains("0/1 mcpserver"),
            "unusable must be visible: {line}"
        );
        assert!(line.contains("1 tool"));
    }

    #[test]
    fn a_delta_is_announced_once_to_the_model() {
        let before = CapabilitySet::new(1, vec![ok("read", CapabilityKind::Tool)]);
        let after = CapabilitySet::new(
            2,
            vec![
                ok("read", CapabilityKind::Tool),
                ok("pdf", CapabilityKind::Skill),
            ],
        );
        let delta = after.delta_from(&before);
        let section = standing_section(&after, Some(&delta));
        assert!(section.contains("changed since your last turn"));
        assert!(section.contains("skill:pdf"));
    }

    #[test]
    fn a_clean_set_says_nothing() {
        let set = CapabilitySet::new(1, vec![ok("read", CapabilityKind::Tool)]);
        assert!(standing_section(&set, None).is_empty());
    }
}
