//! `Core` as a [`CapabilityProvider`]: five kinds, one declaration set.
//!
//! Each kind used to have its own lifecycle — a `read_dir` walk per turn for
//! skills and commands, a fingerprint-keyed cache for MCP, a config re-read
//! for hooks, a static list for tools — and none of them could notice a
//! change once a session's contract had frozen. Here they all produce the
//! same [`Declaration`] and travel the same loop, so "added, updated,
//! edited, removed" means one thing for all five.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use vak_session::types::CapabilityKind;

use super::domain::{Domain, Serves};
use super::registry::{CapabilityProvider, CapabilityRegistry, Declaration, Hint};
use super::snapshot::{CapabilityId, Origin};
use crate::Core;

/// Capability reconciliation outlives individual turns, but it must not
/// outlive the Core that supplies its declarations. Keep the turn-specific
/// context from the Core that first created the registry and upgrade the
/// shared state only for each pass.
pub(crate) struct WeakCapabilityProvider {
    inner: Weak<crate::CoreInner>,
    context: CapabilityCoreContext,
}

#[derive(Clone)]
struct CapabilityCoreContext {
    task_copy_boundary: bool,
    new_documents: Arc<Vec<String>>,
    default_deliver_to: Option<String>,
    surface: crate::Surface,
    prompt_role: Option<String>,
    agent_identity: Option<vak_session::types::AgentIdentity>,
    conversation_context: Option<vak_session::types::ConversationContext>,
    run_admission: crate::admission::RunAdmission,
    prompt_overlays: Arc<Vec<crate::prompts::LayerInput>>,
    approver_answerable: bool,
}

impl WeakCapabilityProvider {
    fn new(core: &Core) -> Self {
        Self {
            inner: Arc::downgrade(&core.inner),
            context: CapabilityCoreContext {
                task_copy_boundary: core.task_copy_boundary,
                new_documents: core.new_documents.clone(),
                default_deliver_to: core.default_deliver_to.clone(),
                surface: core.surface.clone(),
                prompt_role: core.prompt_role.clone(),
                agent_identity: core.agent_identity.clone(),
                conversation_context: core.conversation_context.clone(),
                run_admission: core.run_admission.clone(),
                prompt_overlays: core.prompt_overlays.clone(),
                approver_answerable: core.approver_answerable,
            },
        }
    }

    fn upgrade(&self) -> Option<Core> {
        Some(Core {
            inner: self.inner.upgrade()?,
            task_copy_boundary: self.context.task_copy_boundary,
            new_documents: self.context.new_documents.clone(),
            default_deliver_to: self.context.default_deliver_to.clone(),
            surface: self.context.surface.clone(),
            prompt_role: self.context.prompt_role.clone(),
            agent_identity: self.context.agent_identity.clone(),
            conversation_context: self.context.conversation_context.clone(),
            run_admission: self.context.run_admission.clone(),
            prompt_overlays: self.context.prompt_overlays.clone(),
            approver_answerable: self.context.approver_answerable,
        })
    }
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
        let provider: Arc<dyn CapabilityProvider> = Arc::new(WeakCapabilityProvider::new(self));
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
impl CapabilityProvider for WeakCapabilityProvider {
    fn declare(&self) -> Vec<Declaration> {
        let Some(core) = self.upgrade() else {
            return Vec::new();
        };
        core.capability_declarations()
    }

    async fn upkeep(&self) {
        if let Some(core) = self.upgrade() {
            core.capability_upkeep().await;
        }
    }
}

impl Core {
    pub(crate) fn capability_declarations(&self) -> Vec<Declaration> {
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
                serves: Serves::from_labels(&serves.iter().map(String::as_str).collect::<Vec<_>>()),
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

        // --- presentations -----------------------------------------------
        // An active pack that adds kinds to a card block
        // (docs/design/91-presentation-capabilities.md). It changes what a
        // card looks like, never what is authorized.
        for pack in self.pack_kinds().packs() {
            out.push(Declaration {
                id: CapabilityId::new(CapabilityKind::Presentation, &pack.id),
                origin: match &pack.plugin {
                    Some(plugin) => Origin::Plugin {
                        plugin: plugin.clone(),
                        scope: "workspace".into(),
                    },
                    None => Origin::Workspace,
                },
                summary: format!("Draws {} cards", pack.kinds.join(", ")),
                serves: Serves::Undeclared,
                digest: Some(pack.digest.clone()),
                source: None,
                configuration: serde_json::json!({ "block": pack.block, "kinds": pack.kinds }),
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

    async fn capability_upkeep(&self) {
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
        Some("agent") => Origin::Agent,
        Some("project") => Origin::Workspace,
        _ => Origin::Workspace,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::time::Duration;

    #[tokio::test]
    async fn starting_the_registry_does_not_keep_its_core_alive() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        let weak = Arc::downgrade(&core.inner);
        let registry = core.capability_registry();

        drop(registry);
        drop(core);
        tokio::time::timeout(Duration::from_secs(1), async {
            while weak.upgrade().is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the capability loop must release CoreInner after its owner drops");
    }

    #[test]
    fn a_tool_that_declares_nothing_is_undeclared_and_never_sliced_away() {
        assert!(Serves::from_labels(&[]).is_undeclared());
        let required = BTreeSet::from([Domain::Vcs]);
        assert!(Serves::from_labels(&[]).serves_any(&required));
    }
}
