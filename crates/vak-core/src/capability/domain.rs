//! What a capability is *for*, in the capability's own words.
//!
//! The old design asked the harness to hold the opinion: a static table
//! mapped each `Act` to a list of built-in tool names, so `Act::Answer` meant
//! `["webfetch", "mcp"]`. That structure cannot answer the only question that
//! matters — *which of the things this user installed could serve this
//! request?* — because installed things are not in the table and never can
//! be. Every new integration needed a harness edit, and the edit only ever
//! happened after someone reported a confidently wrong answer.
//!
//! Here the capability classifies itself against a shared vocabulary and the
//! harness only matches. This is how MCP and LSP both work, and it has the
//! property that matters: **adding an integration never requires touching
//! this file.** That is the test for whether something is hardcoded, and a
//! vocabulary passes it where a table of instance names does not.
//!
//! Two escape valves keep it honest:
//!
//! * [`Domain::Custom`] carries a name this build has never heard of, so a
//!   plugin's own domain survives round-tripping instead of being dropped.
//! * A capability that declares *nothing* is [`Serves::Undeclared`] and is
//!   never sliced away. Slicing exists to save context, not to enforce
//!   policy — `reach` and the permission engine do that — so the failure
//!   modes are asymmetric: an extra capability costs a little context, a
//!   missing one costs the task. Undeclared therefore fails open.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// A kind of work a capability can do.
///
/// Ordering is derived and only used to make `BTreeSet` iteration stable for
/// digests; it carries no priority meaning.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Domain {
    /// Facts that change faster than a model's training data: prices,
    /// markets, tickets, inventory, "what is assigned to me today".
    LiveData,
    /// Fetching and reading from the open web.
    Web,
    /// Reading or writing files in the workspace.
    Filesystem,
    /// Running commands or code.
    CodeExec,
    /// Durable recall across sessions.
    Memory,
    /// Reaching a human or another system through a channel.
    Messaging,
    /// Producing or transforming documents and artifacts.
    Documents,
    /// Coordinating other agents, flows, or units of work.
    Orchestration,
    /// Version control and change history.
    Vcs,
    /// The system's own health, spend, and audit surfaces.
    Observability,
    /// A domain this build does not know. Preserved verbatim so a plugin
    /// that speaks its own vocabulary is not silently flattened.
    #[serde(untagged)]
    Custom(String),
}

impl Domain {
    pub fn as_str(&self) -> &str {
        match self {
            Domain::LiveData => "live-data",
            Domain::Web => "web",
            Domain::Filesystem => "filesystem",
            Domain::CodeExec => "code-exec",
            Domain::Memory => "memory",
            Domain::Messaging => "messaging",
            Domain::Documents => "documents",
            Domain::Orchestration => "orchestration",
            Domain::Vcs => "vcs",
            Domain::Observability => "observability",
            Domain::Custom(name) => name,
        }
    }

    /// Parse one declared domain. Unknown names become [`Domain::Custom`]
    /// rather than an error: a capability declaring a domain this build does
    /// not know is a forward-compatibility event, not a misconfiguration.
    pub fn parse(value: &str) -> Domain {
        match value.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "live-data" | "livedata" => Domain::LiveData,
            "web" => Domain::Web,
            "filesystem" | "fs" => Domain::Filesystem,
            "code-exec" | "exec" => Domain::CodeExec,
            "memory" => Domain::Memory,
            "messaging" => Domain::Messaging,
            "documents" | "docs" => Domain::Documents,
            "orchestration" => Domain::Orchestration,
            "vcs" | "git" => Domain::Vcs,
            "observability" => Domain::Observability,
            other => Domain::Custom(other.to_string()),
        }
    }

    pub fn parse_list(values: &[String]) -> BTreeSet<Domain> {
        values
            .iter()
            .map(|value| Domain::parse(value))
            .collect::<BTreeSet<_>>()
    }
}

impl std::fmt::Display for Domain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a capability claims to serve.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Serves {
    /// The capability said nothing. It is never sliced away — see the module
    /// note on asymmetric failure modes. This is the default for an MCP
    /// server whose config carries no `serves`, which is the common case and
    /// deliberately *not* guessed at from tool names: inferring a domain
    /// from a keyword list would reintroduce exactly the harness-side table
    /// this module exists to delete.
    #[default]
    Undeclared,
    /// The capability classified itself.
    Declared(BTreeSet<Domain>),
}

impl Serves {
    pub fn declared<I: IntoIterator<Item = Domain>>(domains: I) -> Serves {
        Serves::Declared(domains.into_iter().collect())
    }

    /// Whether this capability should survive a slice that requires
    /// `required`. An undeclared capability always survives; a declared one
    /// survives when it serves at least one required domain.
    pub fn serves_any(&self, required: &BTreeSet<Domain>) -> bool {
        match self {
            Serves::Undeclared => true,
            Serves::Declared(mine) => mine.intersection(required).next().is_some(),
        }
    }

    pub fn is_undeclared(&self) -> bool {
        matches!(self, Serves::Undeclared)
    }

    /// Stable rendering for reports and digests.
    pub fn labels(&self) -> Vec<String> {
        match self {
            Serves::Undeclared => Vec::new(),
            Serves::Declared(domains) => domains.iter().map(|d| d.to_string()).collect(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn unknown_domains_survive_instead_of_being_dropped() {
        let parsed = Domain::parse("procurement");
        assert_eq!(parsed, Domain::Custom("procurement".into()));
        assert_eq!(parsed.to_string(), "procurement");
    }

    #[test]
    fn parsing_is_forgiving_about_shape() {
        assert_eq!(Domain::parse(" Live_Data "), Domain::LiveData);
        assert_eq!(Domain::parse("GIT"), Domain::Vcs);
    }

    #[test]
    fn undeclared_always_survives_a_slice() {
        let required = BTreeSet::from([Domain::LiveData]);
        assert!(Serves::Undeclared.serves_any(&required));
    }

    #[test]
    fn declared_survives_only_on_intersection() {
        let required = BTreeSet::from([Domain::LiveData]);
        assert!(Serves::declared([Domain::LiveData, Domain::Web]).serves_any(&required));
        assert!(!Serves::declared([Domain::Filesystem]).serves_any(&required));
    }

    #[test]
    fn a_custom_domain_matches_itself() {
        let required = BTreeSet::from([Domain::Custom("procurement".into())]);
        assert!(Serves::declared([Domain::Custom("procurement".into())]).serves_any(&required));
        assert!(!Serves::declared([Domain::Web]).serves_any(&required));
    }
}
