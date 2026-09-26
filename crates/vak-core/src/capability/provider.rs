//! `Core` as a [`CapabilityProvider`]: five kinds, one declaration set.
//!
//! Each kind used to have its own lifecycle — a `read_dir` walk per turn for
//! skills and commands, a fingerprint-keyed cache for MCP, a config re-read
//! for hooks, a static list for tools — and none of them could notice a
//! change once a session's contract had frozen. Here they all produce the
//! same [`Declaration`] and travel the same loop, so "added, updated,
//! edited, removed" means one thing for all five.

use std::sync::Arc;

use async_trait::async_trait;
use vak_session::types::CapabilityKind;

use super::domain::{Domain, Serves};
use super::registry::{CapabilityProvider, CapabilityRegistry, Declaration, Hint};
use super::snapshot::{CapabilityId, Origin};
use crate::Core;

/// Whether a call reaches information from outside the machine and the
/// conversation — the kind an answer should cite — decided from what the
/// capability *declares it serves*, never from its name or its output.
///
/// * A built-in resolves through its own `Tool::serves` (`tool_serves`).
/// * An MCP call resolves to its server's declared `serves`. A server that
///   declares nothing falls back to the `mcp` broker's own declaration, which
///   claims the web and live data; declaring `serves = ["documents"]` opts a
///   server out. Listing a server's tools is not retrieval, only calling one.
/// * A tool that declares nothing is not retrieval.
///
/// `mcp_server` is the server a bare MCP tool name resolves to, if `name` is
/// one; `server_serves` returns a server's configured `serves` list.
pub(crate) fn call_retrieves_external(
    name: &str,
    input: &serde_json::Value,
    mcp_server: Option<&str>,
    tool_serves: &dyn Fn(&str) -> Vec<String>,
    server_serves: &dyn Fn(&str) -> Vec<String>,
) -> bool {
    let reaches_outside = |serves: &[String]| {
        Domain::parse_list(serves)
            .iter()
            .any(|domain| matches!(domain, Domain::Web | Domain::LiveData))
    };
    let server = if name == "mcp" {
        if input.get("action").and_then(|a| a.as_str()) != Some("call") {
            return false;
        }
        input.get("server").and_then(|s| s.as_str())
    } else {
        mcp_server
    };
    match server {
        Some(server) => {
            let declared = server_serves(server);
            if declared.is_empty() {
                let broker: Vec<String> = vak_mcp::McpTool::SERVES
                    .iter()
                    .map(|d| d.to_string())
                    .collect();
                reaches_outside(&broker)
            } else {
                reaches_outside(&declared)
            }
        }
        None => reaches_outside(&tool_serves(name)),
    }
}

/// Whether a built-in tool observes the current state of something — a file,
/// the repository, a command's output, a page, a live value — decided from
/// what it declares it serves. Memory, messaging, orchestration and document
/// production recall or change things; they observe nothing, so a turn that
/// asked for a current value is not answered by them. An MCP call counts
/// through [`call_retrieves_external`].
pub(crate) fn serves_observation(serves: &[String]) -> bool {
    Domain::parse_list(serves).iter().any(|domain| {
        matches!(
            domain,
            Domain::LiveData
                | Domain::Web
                | Domain::Filesystem
                | Domain::CodeExec
                | Domain::Vcs
                | Domain::Observability
        )
    })
}

impl Core {
    /// The registry, created and started on first use.
    ///
    /// Idempotent: many surfaces call this, and they all get the same
    /// registry and the same loop.
    pub fn capability_registry(&self) -> Arc<CapabilityRegistry> {
        if let Some(registry) = self.inner.capability_registry.get() {
            return registry.clone();
        }
        let provider: Arc<dyn CapabilityProvider> = Arc::new(self.clone());
        let (registry, hints) = CapabilityRegistry::new(provider);
        // Losing the race is fine: the winner's registry is the one everyone
        // uses, and the loser's is dropped without ever having been started.
        if let Err(losing) = self.inner.capability_registry.set(registry.clone()) {
            // Another thread won. Use theirs and drop ours unstarted, rather
            // than running two loops against one provider.
            drop(losing);
            return match self.inner.capability_registry.get() {
                Some(winner) => winner.clone(),
                // Unreachable in practice (`set` only fails when occupied),
                // but a registry is not worth a panic: an unstarted one still
                // reconciles on demand.
                None => registry,
            };
        }
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        if let Ok(mut slot) = self.inner.capability_shutdown.lock() {
            *slot = Some(shutdown_tx);
        }
        // Only spawn when a runtime is present. A synchronous caller (tests,
        // one-shot tooling) still gets a working registry — it just
        // reconciles on demand rather than on a rhythm.
        if tokio::runtime::Handle::try_current().is_ok() {
            let loop_registry = registry.clone();
            tokio::spawn(async move {
                loop_registry.run(hints, shutdown_rx).await;
            });
            registry.hint(Hint::Immediate);
        }
        registry
    }

    /// Reconcile now and return the published set, for callers that cannot
    /// wait for the loop: a one-shot CLI turn, or a surface that just
    /// changed configuration and wants the result to be visible immediately.
    pub async fn reconcile_capabilities(&self) -> Arc<super::snapshot::CapabilitySet> {
        let registry = self.capability_registry();
        registry.reconcile().await;
        registry.current().await
    }
}

#[async_trait]
impl CapabilityProvider for Core {
    fn declare(&self) -> Vec<Declaration> {
        let mut out = Vec::new();

        // --- tools -------------------------------------------------------
        // `tool_declarations()` already applies the runtime toggles and
        // channel policy, so a `[tools]` flag flipped through `PUT /config`
        // shows up on the next reconcile rather than at the next process
        // start. Each tool states its own domains.
        for (name, serves) in self.tool_declarations() {
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Tool, &name),
                origin: Origin::Builtin,
                summary: String::new(),
                serves: Serves::from_labels(serves),
                digest: None,
                source: None,
                configuration: serde_json::Value::Null,
            });
        }
        if self.channel_tool_allowed("flow") {
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Tool, "flow"),
                origin: Origin::Builtin,
                summary: "Managed static-flow dispatcher".into(),
                serves: Serves::from_labels(vak_agent::FLOW_SERVES),
                digest: None,
                source: None,
                configuration: serde_json::Value::Null,
            });
        }

        // --- skills ------------------------------------------------------
        for skill in self.skills() {
            let Ok(digest) = skill.digest() else {
                // A skill whose body cannot be read is not silently dropped:
                // it simply does not declare, and the parse diagnostic
                // surfaces it. Admitting it would advertise instructions the
                // loader could not then produce.
                continue;
            };
            // A skill classifies itself through its own `serves:`
            // frontmatter (`crate::skills::validate`). One that declares
            // nothing — including every seeded skill that predates this
            // field — is undeclared and therefore never sliced away; there
            // is no name-keyed table here to fall back to.
            let serves = skill
                .serves
                .as_ref()
                .map(|values| Serves::Declared(Domain::parse_list(values)))
                .unwrap_or(Serves::Undeclared);
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Skill, &skill.name),
                origin: origin_from_provenance(skill.provenance.as_deref()),
                summary: skill.description.clone(),
                serves,
                digest: Some(digest),
                source: Some(skill.path.clone()),
                configuration: serde_json::Value::Null,
            });
        }

        // --- mcp servers -------------------------------------------------
        // Declared from config alone; what the on-demand pool has observed
        // (catalog, last failure) rides along as data. Nothing here starts a
        // server — only a model's `mcp` call does.
        let mcp = self.effective_mcp();
        let observed = self
            .mcp_manager()
            .map(|manager| manager.observations())
            .unwrap_or_default();
        for (name, server) in mcp.servers {
            let serves = if server.serves.is_empty() {
                // Deliberately not guessed from tool names: a keyword table
                // would reintroduce exactly the harness-side opinion this
                // design deletes. Undeclared is never sliced away, so the
                // common case of a server with no `serves` stays reachable.
                Serves::Undeclared
            } else {
                Serves::Declared(Domain::parse_list(&server.serves))
            };
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(name.as_bytes());
            hasher.update(server.command.as_bytes());
            for arg in &server.args {
                hasher.update(arg.as_bytes());
            }
            for (k, v) in &server.env {
                hasher.update(k.as_bytes());
                if let Some(resolved) =
                    crate::interpolate_env_var_with(v, |key| self.mcp_secret(key))
                {
                    hasher.update(b"resolved:");
                    hasher.update(resolved.as_bytes());
                } else {
                    hasher.update(b"unresolved:");
                    hasher.update(v.as_bytes());
                }
            }
            let digest = Some(format!("{:x}", hasher.finalize()));

            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::McpServer, &name),
                origin: if name.starts_with("plugin.") {
                    Origin::Plugin {
                        plugin: name.split('.').nth(1).unwrap_or_default().to_string(),
                        scope: "workspace".into(),
                    }
                } else {
                    Origin::Workspace
                },
                summary: "MCP server reached through the brokered mcp tool".into(),
                serves,
                digest,
                source: None,
                configuration: observed
                    .get(&name)
                    .map(mcp_observation_json)
                    .unwrap_or(serde_json::Value::Null),
            });
        }

        // --- hooks -------------------------------------------------------
        for hook in self.effective_hooks().into_iter().filter(|h| h.enabled) {
            out.push(Declaration {
                id: CapabilityId::new(
                    CapabilityKind::Hook,
                    format!("{}/{}", hook.event, hook.command),
                ),
                origin: Origin::Workspace,
                summary: hook.matcher.clone().unwrap_or_default(),
                serves: Serves::Undeclared,
                digest: None,
                source: None,
                // The hook's own config, read back by `crate::hook_def` — the
                // same reader config validation uses.
                configuration: serde_json::to_value(&hook).unwrap_or_default(),
            });
        }

        // --- commands ----------------------------------------------------
        for command in self.custom_commands() {
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Command, &command.name),
                origin: origin_from_provenance(Some(&command.source)),
                summary: command.description.clone(),
                serves: Serves::Undeclared,
                digest: None,
                source: None,
                configuration: serde_json::json!({ "template": command.template }),
            });
        }

        out
    }

    async fn upkeep(&self) {
        // Idle eviction: the pool's only background work, and it only ever
        // releases. A process that stays up for weeks must not hold a
        // subprocess for every server it has ever touched; the next call
        // respawns on demand.
        if let Some(manager) = self.mcp_manager() {
            manager.evict_idle(vak_mcp::IDLE_TTL).await;
        }
    }
}

/// A server's observation as declared configuration: its tools (name,
/// description, schema) once used, and the last failure's reason. Only the
/// reason — never attempt counts or times — so repeated failures for one
/// cause do not republish an epoch.
fn mcp_observation_json(observation: &vak_mcp::ServerObservation) -> serde_json::Value {
    let mut config = serde_json::Map::new();
    if let Some(tools) = &observation.tools {
        config.insert(
            "tools".into(),
            tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                    })
                })
                .collect(),
        );
    }
    if let Some(failure) = &observation.failure {
        config.insert("last_failure".into(), failure.clone().into());
    }
    if config.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(config)
    }
}

fn origin_from_provenance(provenance: Option<&str>) -> Origin {
    match provenance {
        Some(p) if p.starts_with("plugin:") => {
            let parts: Vec<&str> = p.split(':').collect();
            Origin::Plugin {
                scope: parts.get(1).unwrap_or(&"workspace").to_string(),
                plugin: parts.get(2).unwrap_or(&"unknown").to_string(),
            }
        }
        Some("user") => Origin::Shared,
        Some("project") => Origin::Workspace,
        _ => Origin::Workspace,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use vak_intent::Act;

    /// The act → domain table lives in the kernel (`vak_intent::engage`);
    /// this crate only parses the names. A second copy of the table here
    /// drifted once already.
    fn required_for(act: Act) -> BTreeSet<Domain> {
        let reading = vak_intent::Reading {
            act,
            confidence: 0.9,
            ..vak_intent::Reading::general()
        };
        let engagement = vak_intent::derive(&reading, &vak_intent::Authority::default(), true);
        engagement
            .limits
            .required_domains
            .iter()
            .map(|name| Domain::parse(name))
            .collect()
    }

    #[test]
    fn every_act_that_produces_a_fact_can_reach_a_live_source() {
        for act in [Act::Answer, Act::Locate, Act::Analyze, Act::Operate] {
            assert!(
                required_for(act).contains(&Domain::LiveData),
                "{act:?} must be able to reach a live source"
            );
        }
    }

    #[test]
    fn a_greeting_stays_narrow() {
        let required = required_for(Act::Converse);
        assert!(!required.contains(&Domain::CodeExec));
        assert!(!required.contains(&Domain::LiveData));
    }

    #[test]
    fn the_mcp_broker_serves_live_data_so_a_search_server_is_reachable() {
        // This is the specific link that made "how is the weather" work:
        // `mcp` must survive a slice that requires live data.
        let required = required_for(Act::Answer);
        assert!(Serves::from_labels(vak_mcp::McpTool::SERVES).serves_any(&required));
    }

    #[test]
    fn a_tool_that_declares_nothing_is_undeclared_and_never_sliced_away() {
        assert!(Serves::from_labels(&[]).is_undeclared());
        let required = BTreeSet::from([Domain::Vcs]);
        assert!(Serves::from_labels(&[]).serves_any(&required));
    }
}

#[cfg(test)]
mod retrieval_tests {
    use super::call_retrieves_external;
    use serde_json::json;

    fn serves(server: &str) -> Vec<String> {
        match server {
            "docs-only" => vec!["documents".into()],
            "search" => vec!["web".into()],
            "local-fs" => vec!["filesystem".into()],
            _ => Vec::new(), // declares nothing, like a stock Tavily config
        }
    }

    /// Built-ins answer from their own `Tool::serves`, as the turn does.
    fn tool_serves(name: &str) -> Vec<String> {
        let mut tools = vak_tools::default_tools();
        tools.push(std::sync::Arc::new(vak_tools::WebFetchTool));
        tools.push(std::sync::Arc::new(vak_tools::WebBrowseTool));
        tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(|tool| tool.serves().iter().map(|d| d.to_string()).collect())
            .unwrap_or_default()
    }

    fn retrieves(name: &str, input: serde_json::Value, server: Option<&str>) -> bool {
        call_retrieves_external(name, &input, server, &tool_serves, &serves)
    }

    #[test]
    fn built_ins_that_reach_the_web_do_and_others_do_not() {
        for tool in ["webfetch", "browse"] {
            assert!(retrieves(tool, json!({}), None), "{tool}");
        }
        // Names the old keyword rule got wrong in either direction.
        for tool in [
            "read",
            "grep",
            "glob",
            "bash",
            "session_search",
            "entity_query",
            "emit_research_card",
            "some_future_tool",
        ] {
            assert!(!retrieves(tool, json!({}), None), "{tool} is not retrieval");
        }
    }

    #[test]
    fn an_mcp_call_is_retrieval_unless_its_server_declares_otherwise() {
        let call = |server: &str| json!({"action": "call", "server": server, "tool": "anything"});
        assert!(
            retrieves("mcp", call("tavily"), None),
            "a server declaring nothing inherits the broker's web/live-data claim"
        );
        assert!(retrieves("mcp", call("search"), None));
        assert!(
            !retrieves("mcp", call("docs-only"), None),
            "declaring documents opts out"
        );
        assert!(!retrieves("mcp", call("local-fs"), None));
    }

    #[test]
    fn listing_mcp_tools_is_not_retrieval() {
        assert!(!retrieves("mcp", json!({"action": "list"}), None));
    }

    #[test]
    fn a_bare_mcp_tool_name_resolves_through_its_server() {
        assert!(retrieves("tavily_search", json!({}), Some("tavily")));
        assert!(!retrieves("notes_lookup", json!({}), Some("docs-only")));
    }
}
