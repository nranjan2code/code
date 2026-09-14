//! `Core` as a [`CapabilityProvider`]: five kinds, one declaration set.
//!
//! Each kind used to have its own lifecycle — a `read_dir` walk per turn for
//! skills and commands, a fingerprint-keyed cache for MCP, a config re-read
//! for hooks, a static list for tools — and none of them could notice a
//! change once a session's contract had frozen. Here they all produce the
//! same [`Declaration`] and travel the same loop, so "added, updated,
//! edited, removed" means one thing for all five.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use vak_session::types::CapabilityKind;

use super::domain::{Domain, Serves};
use super::registry::{
    CapabilityProvider, CapabilityRegistry, Declaration, Hint, ProbeFailure, ProbeReport,
};
use super::snapshot::{CapabilityId, Origin};
use crate::Core;

/// Built-in tools classify themselves here rather than in an act→tool table.
///
/// This is a property *of each tool*, kept beside the tool list rather than
/// in the intent kernel, and it is the reason adding an MCP server never
/// requires editing the harness: the slice matches on declared domains, and
/// anything undeclared is never sliced away.
fn builtin_domains(name: &str) -> Serves {
    use Domain::*;
    let domains: &[Domain] = match name {
        "read" | "glob" | "grep" => &[Filesystem],
        "write" | "edit" | "data_query" => &[Documents],
        "bash" => &[CodeExec],
        "webfetch" => &[Web, LiveData],
        "browse" => &[Web, LiveData],
        // The broker to every external integration. Claiming `live-data`
        // here is what let "how is the weather" reach a configured search
        // server; the individual servers refine it by declaring their own.
        "mcp" => &[LiveData, Web, Documents, Messaging],
        "skill" => &[Documents, Orchestration],
        "task" | "flow" | "tasks" => &[Orchestration],
        "session_search" => &[Memory],
        "remember" | "propose_skill" | "entity_record" | "entity_query" => &[Memory],
        "commitments" => &[Memory, Orchestration],
        // A tool this build does not classify stays undeclared, which means
        // it is never sliced away. Failing open is correct here: slicing
        // saves context, it does not enforce policy.
        _ => return Serves::Undeclared,
    };
    Serves::declared(domains.iter().cloned())
}

/// Standard starter skills classify themselves here if not declared in frontmatter.
fn starter_skill_domains(name: &str) -> Serves {
    use Domain::*;
    let domains: &[Domain] = match name {
        "software-development" => &[CodeExec, Documents, Vcs, Filesystem],
        "debugging" => &[CodeExec, Observability],
        "code-review" => &[Vcs, Documents],
        "data-and-spreadsheets" => &[Documents, CodeExec],
        "research-and-sources" => &[Web, LiveData],
        "writing-and-editing" => &[Documents],
        "planning-and-organizing" => &[Orchestration],
        "getting-started" => &[Documents],
        _ => return Serves::Undeclared,
    };
    Serves::declared(domains.iter().cloned())
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

    /// What each built-in tool declares it serves, for the per-turn slice.
    ///
    /// A tool absent from this map is undeclared and is never sliced away.
    pub fn declared_tool_domains(&self) -> std::collections::BTreeMap<String, BTreeSet<Domain>> {
        let mut out = std::collections::BTreeMap::new();
        for name in self.tool_names() {
            if let Serves::Declared(domains) = builtin_domains(&name) {
                out.insert(name, domains);
            }
        }
        if let Serves::Declared(domains) = builtin_domains("flow") {
            out.insert("flow".into(), domains);
        }
        out
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
        // `tool_names()` already applies the runtime toggles and channel
        // policy, so a `[tools]` flag flipped through `PUT /config` shows up
        // on the next reconcile rather than at the next process start.
        for name in self.tool_names() {
            let serves = builtin_domains(&name);
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Tool, &name),
                origin: Origin::Builtin,
                summary: String::new(),
                serves,
                digest: None,
                source: None,
                configuration: serde_json::Value::Null,
                needs_probe: false,
            });
        }
        if self.channel_tool_allowed("flow") {
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Tool, "flow"),
                origin: Origin::Builtin,
                summary: "Managed static-flow dispatcher".into(),
                serves: builtin_domains("flow"),
                digest: None,
                source: None,
                configuration: serde_json::Value::Null,
                needs_probe: false,
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
            let serves = skill
                .serves
                .as_ref()
                .map(|values| Serves::Declared(Domain::parse_list(values)))
                .unwrap_or_else(|| starter_skill_domains(&skill.name));
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Skill, &skill.name),
                origin: origin_from_provenance(skill.provenance.as_deref()),
                summary: skill.description.clone(),
                serves,
                digest: Some(digest),
                source: Some(skill.path.clone()),
                configuration: serde_json::Value::Null,
                needs_probe: false,
            });
        }

        // --- mcp servers -------------------------------------------------
        let mcp = self.effective_mcp();
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
                digest: None,
                source: None,
                configuration: serde_json::Value::Null,
                // The one kind that talks to something outside the process,
                // and therefore the one kind that must be probed before it
                // can be called usable.
                needs_probe: true,
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
                configuration: serde_json::json!({
                    "event": hook.event,
                    "matcher": hook.matcher,
                    "command": hook.command,
                    "timeout_ms": hook.timeout_ms,
                    "failure_mode": hook.failure_mode,
                }),
                needs_probe: false,
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
                needs_probe: false,
            });
        }

        out
    }

    async fn probe(&self, id: &CapabilityId) -> Result<ProbeReport, ProbeFailure> {
        if id.kind != CapabilityKind::McpServer {
            return Ok(ProbeReport {
                configuration: serde_json::Value::Null,
                announces_changes: true,
            });
        }
        let Some(manager) = self.mcp_manager() else {
            return Err(ProbeFailure::new(
                "no MCP manager for this server set",
                "check that the server is still configured",
            ));
        };
        match manager.probe(&id.name).await {
            Ok(tools) => {
                let announces_changes = manager.is_connected(&id.name).await;
                Ok(ProbeReport {
                    configuration: serde_json::json!({
                        "tools": tools
                            .iter()
                            .map(|t| serde_json::json!({
                                "name": t.name,
                                "description": t.description,
                                "inputSchema": t.input_schema,
                            }))
                            .collect::<Vec<_>>(),
                    }),
                    announces_changes,
                })
            }
            Err(reason) => Err(ProbeFailure::new(
                reason,
                format!(
                    "check the `{}` entry under [mcp.servers] — command, args, and any required env",
                    id.name
                ),
            )),
        }
    }

    async fn upkeep(&self) {
        // Idle eviction. A process that stays up for weeks must not hold a
        // subprocess for every server it has ever touched; the next call
        // respawns on demand.
        if let Some(manager) = self.mcp_manager() {
            manager.evict_idle(vak_mcp::IDLE_TTL).await;
        }
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

/// Domains this reading requires, derived from the reading's own axes.
///
/// This is the replacement for `act_capabilities`'s table of built-in tool
/// names. The distinction that matters: this maps vak's *model of the
/// request* to a vocabulary, and adding an integration never touches it.
/// The old table enumerated instances, so every integration needed an edit —
/// and the edit only ever happened after a user reported a confidently wrong
/// answer.
pub fn required_domains(act: vak_intent::Act, evidence_is_external: bool) -> BTreeSet<Domain> {
    use Domain::*;
    use vak_intent::Act;
    let mut required = BTreeSet::from([Filesystem, Memory]);
    match act {
        Act::Converse => {}
        Act::Answer => {
            required.insert(LiveData);
            required.insert(Web);
        }
        Act::Locate => {
            required.extend([LiveData, Web, Vcs]);
        }
        Act::Analyze => {
            required.extend([LiveData, Web, CodeExec, Vcs]);
        }
        Act::Author => {
            required.extend([Documents, Web]);
        }
        Act::Modify => {
            required.extend([Documents, CodeExec, Vcs]);
        }
        Act::Operate => {
            required.extend([CodeExec, Web, LiveData, Messaging, Documents, Orchestration]);
        }
        Act::Verify => {
            required.extend([CodeExec, Observability]);
        }
        Act::Orchestrate => {
            required.extend([Orchestration, CodeExec]);
        }
        Act::Govern => {
            required.extend([Memory, Orchestration, Documents, Observability]);
        }
    }
    // A question whose answer is not already in front of us needs a way to
    // go and get one, whatever the act was read as. This is the general form
    // of the weather bug: the failure was never specific to `Answer`.
    if evidence_is_external {
        required.insert(LiveData);
        required.insert(Web);
    }
    required
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_intent::Act;

    #[test]
    fn every_act_that_produces_a_fact_can_reach_a_live_source() {
        // The regression the old table encoded: `Answer` and `Locate` were
        // the two acts that exist to produce a fact, and the two that could
        // not go and get one.
        for act in [Act::Answer, Act::Locate, Act::Analyze, Act::Operate] {
            let required = required_domains(act, false);
            assert!(
                required.contains(&Domain::LiveData),
                "{act:?} must be able to reach a live source"
            );
        }
    }

    #[test]
    fn an_external_question_reaches_live_data_whatever_the_act() {
        // Even for the act that needs least, an answer that is not in
        // context must be able to go and get one.
        let required = required_domains(Act::Verify, true);
        assert!(required.contains(&Domain::LiveData));
    }

    #[test]
    fn a_greeting_stays_narrow() {
        let required = required_domains(Act::Converse, false);
        assert!(!required.contains(&Domain::CodeExec));
        assert!(!required.contains(&Domain::LiveData));
    }

    #[test]
    fn the_mcp_broker_serves_live_data_so_a_search_server_is_reachable() {
        // This is the specific link that made "how is the weather" work:
        // `mcp` must survive a slice that requires live data.
        let required = required_domains(Act::Answer, false);
        assert!(builtin_domains("mcp").serves_any(&required));
    }

    #[test]
    fn an_unclassified_tool_is_undeclared_and_never_sliced_away() {
        assert!(builtin_domains("some_future_tool").is_undeclared());
        let required = BTreeSet::from([Domain::Vcs]);
        assert!(builtin_domains("some_future_tool").serves_any(&required));
    }
}
