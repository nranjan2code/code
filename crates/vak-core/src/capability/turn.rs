//! Single-turn capability admission — the one pipeline for all five kinds.
//!
//! What a turn may call is computed from the reconciled [`CapabilitySet`] in
//! **one** function that runs every kind (tool, skill, MCP server, hook,
//! command) through the same two policy stages:
//!
//! 1. **Channel visibility** — allow/deny from the chat's [`ChannelPolicy`].
//! 2. **Reach** — a capability the composed permission policy fully blocks,
//!    or one revoked since the last published epoch, is removed.
//!
//! That is the whole of admission, and it is policy. What the turn's reading
//! *predicts* it will need is a separate, later question — which admitted
//! tools are loaded with full schemas and which wait behind `find_tools` —
//! answered by [`super::surface`]. Keeping the two apart is what makes
//! presentation safe to get wrong: a misread can cost a `find_tools` call,
//! never a capability (docs/design/41-capability-registry.md, "Turn
//! capabilities"; docs/design/68-context-engine.md §5).

use std::collections::{BTreeSet, HashMap};

use vak_config::ChannelPolicy;
use vak_hooks::{HookDef, HookEvent, HookFailureMode};
use vak_session::types::CapabilityKind;

use super::snapshot::{Capability, CapabilityId, CapabilitySet};

/// The authoritative admitted set for one turn, built atomically so every
/// consumer — schemas, prompt, broker, hooks — reads the same answer.
#[derive(Debug, Clone, Default)]
pub struct TurnCapabilities {
    /// Built-in tool identities admitted for this turn.
    pub tool_names: BTreeSet<String>,
    /// Descriptor projection of everything admitted, for the prompt and the
    /// ledger.
    pub descriptors: Vec<vak_session::types::CapabilityDescriptor>,
    /// Bare MCP tool name → the admitted server that owns it. The `mcp`
    /// broker is the only way an MCP tool is called; this lets the loop
    /// repair a call a model addresses by the bare name. Names that collide
    /// across servers, or with a built-in, are left out as ambiguous.
    pub mcp_tool_index: HashMap<String, String>,
    /// Admitted MCP server names.
    pub mcp_server_names: Vec<String>,
    /// Hooks to run this turn.
    pub hooks: Vec<HookDef>,
    /// Admitted skills, digest-pinned for loading.
    pub frozen_skills: Vec<crate::skills::FrozenSkill>,
    /// Whether the managed `flow` capability was admitted.
    pub flow_admitted: bool,
}

/// All turn-scoped inputs the pipeline reads.
pub struct TurnProbe<'a> {
    /// The reconciled, versioned capability set the turn binds to.
    pub capabilities: &'a CapabilitySet,
    /// Immediate revocations, applied even before the next published epoch.
    pub revoked_ids: BTreeSet<CapabilityId>,
    /// Channel allow/deny overlay from the chat surface.
    pub channel_policy: &'a ChannelPolicy,
    /// Per-turn reach standings (`Core::capability_standings`).
    pub reach_standings: &'a [crate::reach::Standing],
    /// Discovered MCP tool catalogues, keyed by server name.
    pub mcp_inventory: &'a [(String, Vec<vak_mcp::McpToolInfo>)],
    /// Built-in tool names, which a bare MCP tool name may never shadow.
    pub builtin_names: &'a BTreeSet<String>,
}

fn blocked_ids(standings: &[crate::reach::Standing]) -> BTreeSet<CapabilityId> {
    standings
        .iter()
        .filter(|standing| standing.reach.is_blocked())
        .map(|standing| standing.id.clone())
        .collect()
}

fn admitted(cap: &Capability, probe: &TurnProbe<'_>, blocked: &BTreeSet<CapabilityId>) -> bool {
    visible_on_channel(&cap.id, probe.channel_policy)
        && !blocked.contains(&cap.id)
        && !probe.revoked_ids.contains(&cap.id)
}

fn visible_on_channel(id: &CapabilityId, policy: &ChannelPolicy) -> bool {
    match id.kind {
        // MCP policies use qualified `server/tool` globs: a server is visible
        // unless `server/*` is denied, and when an allow list exists it must
        // name the server or one of its tools.
        CapabilityKind::McpServer => {
            let whole = format!("{}/*", id.name);
            let denied = policy
                .mcp_deny
                .iter()
                .any(|pattern| crate::Core::policy_matches(std::slice::from_ref(pattern), &whole));
            let allowed = policy.mcp_allow.as_ref().is_none_or(|patterns| {
                patterns.is_empty()
                    || patterns.iter().any(|pattern| {
                        crate::Core::policy_matches(std::slice::from_ref(pattern), &whole)
                            || pattern.starts_with(&format!("{}/", id.name))
                    })
            });
            !denied && allowed
        }
        CapabilityKind::Tool | CapabilityKind::Command => {
            crate::Core::allowed_by(&policy.tools_allow, &policy.tools_deny, &id.name)
        }
        CapabilityKind::Skill => {
            crate::Core::allowed_by(&policy.skills_allow, &policy.skills_deny, &id.name)
        }
        CapabilityKind::Hook => {
            crate::Core::allowed_by(&policy.hooks_allow, &policy.hooks_deny, &id.name)
        }
    }
}

/// Bare MCP tool name → owning server, for admitted servers only. Used both
/// at turn admission and by the broker's live catalogue observer, so a tool
/// discovered mid-turn is indexed by the same rule.
pub(crate) fn mcp_tool_index(
    inventory: &[(String, Vec<vak_mcp::McpToolInfo>)],
    admitted_servers: &BTreeSet<String>,
    builtins: &BTreeSet<String>,
) -> HashMap<String, String> {
    let mut index = HashMap::new();
    let mut ambiguous = BTreeSet::new();
    for (server, tools) in inventory {
        if !admitted_servers.contains(server) {
            continue;
        }
        for tool in tools {
            if builtins.contains(&tool.name) {
                continue;
            }
            if index.insert(tool.name.clone(), server.clone()).is_some() {
                ambiguous.insert(tool.name.clone());
            }
        }
    }
    for name in ambiguous {
        index.remove(&name);
    }
    index
}

/// A hook read from its declaration. One the reader rejects is dropped when
/// it was advisory (fail-open) and, when it was declared fail-closed —
/// or its failure mode is itself unreadable, so it may have been — becomes a
/// refusal of every tool call: a guard whose definition cannot be read must
/// not quietly stop guarding. `Core::capability_diagnostics` names it.
fn hook_from_declaration(cap: &Capability) -> Option<HookDef> {
    let config: vak_config::HookConfig = serde_json::from_value(cap.configuration.clone()).ok()?;
    match crate::hook_def(&config) {
        Ok(def) => Some(def),
        Err(reason) => {
            let declared_open = config.failure_mode.as_deref().unwrap_or("open") == "open";
            (!declared_open).then(|| HookDef {
                event: HookEvent::PreToolUse,
                matcher: None,
                command: config.command.clone(),
                timeout_ms: 0,
                failure_mode: HookFailureMode::Closed,
                refusal: Some(format!(
                    "fail-closed hook `{}` cannot be read ({reason}); fix it to resume tool use",
                    cap.id.name
                )),
            })
        }
    }
}

impl TurnCapabilities {
    /// Build the admitted set for one turn in a single pass.
    pub fn build(probe: &TurnProbe<'_>) -> Self {
        let blocked = blocked_ids(probe.reach_standings);
        let surviving: Vec<&Capability> = probe
            .capabilities
            .usable()
            .filter(|cap| admitted(cap, probe, &blocked))
            .collect();
        let of_kind = |kind: CapabilityKind| {
            surviving
                .iter()
                .copied()
                .filter(move |cap| cap.id.kind == kind)
        };

        let mcp_servers: BTreeSet<String> = of_kind(CapabilityKind::McpServer)
            .map(|cap| cap.id.name.clone())
            .collect();
        let mcp_tool_index = mcp_tool_index(probe.mcp_inventory, &mcp_servers, probe.builtin_names);

        let hooks = of_kind(CapabilityKind::Hook)
            .filter_map(hook_from_declaration)
            .collect();

        let frozen_skills = of_kind(CapabilityKind::Skill)
            .filter_map(|cap| {
                Some(crate::skills::FrozenSkill {
                    name: cap.id.name.clone(),
                    description: cap.summary.clone(),
                    path: cap.source.clone()?,
                    digest: cap.digest.clone()?,
                    provenance: Some(cap.origin.label()),
                })
            })
            .collect();

        let tool_names: BTreeSet<String> = of_kind(CapabilityKind::Tool)
            .map(|cap| cap.id.name.clone())
            .collect();

        TurnCapabilities {
            flow_admitted: tool_names.contains("flow"),
            tool_names,
            descriptors: surviving.iter().map(|cap| cap.to_descriptor()).collect(),
            mcp_tool_index,
            mcp_server_names: mcp_servers.into_iter().collect(),
            hooks,
            frozen_skills,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::capability::domain::Serves;
    use crate::capability::resolution::Resolution;
    use crate::capability::snapshot::Origin;

    fn cap(name: &str, kind: CapabilityKind) -> Capability {
        Capability {
            id: CapabilityId::new(kind, name),
            origin: Origin::Builtin,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            resolution: Resolution::Available,
            configuration: serde_json::Value::Null,
        }
    }

    fn skill(name: &str) -> Capability {
        Capability {
            summary: format!("Skill {name}"),
            digest: Some(format!("sha:{name}")),
            source: Some(std::path::PathBuf::from(format!("/tmp/{name}/SKILL.md"))),
            ..cap(name, CapabilityKind::Skill)
        }
    }

    fn hook(name: &str, config: serde_json::Value) -> Capability {
        Capability {
            configuration: config,
            ..cap(name, CapabilityKind::Hook)
        }
    }

    fn server_tools(server: &str, tools: &[&str]) -> (String, Vec<vak_mcp::McpToolInfo>) {
        (
            server.to_string(),
            tools
                .iter()
                .map(|name| vak_mcp::McpToolInfo {
                    name: (*name).into(),
                    description: String::new(),
                    input_schema: serde_json::json!({}),
                })
                .collect(),
        )
    }

    fn blocked(kind: CapabilityKind, name: &str) -> crate::reach::Standing {
        crate::reach::Standing {
            id: CapabilityId::new(kind, name),
            tool: name.into(),
            label: name.into(),
            reach: crate::reach::Reach::Blocked,
            reason: "denied".into(),
            remedy: String::new(),
        }
    }

    struct Fixture {
        set: CapabilitySet,
        policy: ChannelPolicy,
        standings: Vec<crate::reach::Standing>,
        inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)>,
        revoked: BTreeSet<CapabilityId>,
        builtins: BTreeSet<String>,
    }

    impl Fixture {
        fn new(caps: Vec<Capability>) -> Self {
            Fixture {
                set: CapabilitySet::new(1, caps),
                policy: ChannelPolicy::default(),
                standings: Vec::new(),
                inventory: Vec::new(),
                revoked: BTreeSet::new(),
                builtins: ["read", "bash"].iter().map(|s| s.to_string()).collect(),
            }
        }

        fn build(&self) -> TurnCapabilities {
            TurnCapabilities::build(&TurnProbe {
                capabilities: &self.set,
                revoked_ids: self.revoked.clone(),
                channel_policy: &self.policy,
                reach_standings: &self.standings,
                mcp_inventory: &self.inventory,
                builtin_names: &self.builtins,
            })
        }
    }

    /// Admission is policy only: nothing is removed for serving a domain the
    /// turn's reading did not predict — that is the surface's decision.
    #[test]
    fn every_admitted_kind_survives_regardless_of_domain() {
        let mut bash = cap("bash", CapabilityKind::Tool);
        bash.serves = Serves::from_labels(&["code-exec"]);
        let fixture = Fixture::new(vec![
            bash,
            cap("flow", CapabilityKind::Tool),
            cap("search", CapabilityKind::McpServer),
            skill("travel"),
            hook(
                "stop/notify",
                serde_json::json!({"event": "stop", "command": "notify"}),
            ),
        ]);
        let tc = fixture.build();
        assert!(tc.tool_names.contains("bash"));
        assert!(tc.flow_admitted);
        assert_eq!(tc.mcp_server_names, vec!["search"]);
        assert_eq!(tc.frozen_skills.len(), 1);
        assert_eq!(tc.hooks.len(), 1);
        assert_eq!(tc.descriptors.len(), 5);
    }

    #[test]
    fn channel_policy_removes_each_kind() {
        let mut fixture = Fixture::new(vec![
            cap("bash", CapabilityKind::Tool),
            cap("search", CapabilityKind::McpServer),
            skill("travel"),
        ]);
        fixture.policy.tools_deny = vec!["bash".into()];
        fixture.policy.mcp_deny = vec!["search/*".into()];
        fixture.policy.skills_deny = vec!["travel".into()];
        let tc = fixture.build();
        assert!(tc.tool_names.is_empty());
        assert!(tc.mcp_server_names.is_empty());
        assert!(tc.frozen_skills.is_empty());
        assert!(tc.descriptors.is_empty());
    }

    #[test]
    fn a_tool_scoped_mcp_allow_keeps_its_server() {
        let mut fixture = Fixture::new(vec![cap("search", CapabilityKind::McpServer)]);
        fixture.policy.mcp_allow = Some(vec!["search/query".into()]);
        fixture.inventory = vec![server_tools("search", &["query"])];
        let tc = fixture.build();
        assert_eq!(
            tc.mcp_tool_index.get("query").map(String::as_str),
            Some("search")
        );
    }

    #[test]
    fn a_blocked_standing_removes_exactly_that_capability() {
        let mut fixture = Fixture::new(vec![
            cap("search", CapabilityKind::McpServer),
            cap("notes", CapabilityKind::McpServer),
            cap("webfetch", CapabilityKind::Tool),
        ]);
        fixture.standings = vec![
            blocked(CapabilityKind::McpServer, "search"),
            blocked(CapabilityKind::Tool, "webfetch"),
        ];
        let tc = fixture.build();
        assert_eq!(tc.mcp_server_names, vec!["notes"]);
        assert!(tc.tool_names.is_empty());
    }

    #[test]
    fn a_revoked_capability_is_gone_before_the_next_epoch() {
        let mut fixture = Fixture::new(vec![cap("webfetch", CapabilityKind::Tool)]);
        fixture
            .revoked
            .insert(CapabilityId::new(CapabilityKind::Tool, "webfetch"));
        assert!(fixture.build().tool_names.is_empty());
    }

    #[test]
    fn the_mcp_index_drops_collisions_and_never_shadows_a_builtin() {
        let mut fixture = Fixture::new(vec![
            cap("a", CapabilityKind::McpServer),
            cap("b", CapabilityKind::McpServer),
        ]);
        fixture.inventory = vec![
            server_tools("a", &["search", "only_a", "read"]),
            server_tools("b", &["search"]),
            server_tools("not_admitted", &["hidden"]),
        ];
        let index = fixture.build().mcp_tool_index;
        assert_eq!(index.get("only_a").map(String::as_str), Some("a"));
        assert!(!index.contains_key("search"), "ambiguous across servers");
        assert!(
            !index.contains_key("read"),
            "a built-in is never an MCP alias"
        );
        assert!(!index.contains_key("hidden"));
    }

    #[test]
    fn a_broken_advisory_hook_is_dropped() {
        let fixture = Fixture::new(vec![hook(
            "pre/bad",
            serde_json::json!({"event": "pre-tool-use", "match": "((", "command": "x"}),
        )]);
        assert!(fixture.build().hooks.is_empty());
    }

    #[test]
    fn a_broken_fail_closed_hook_refuses_instead_of_vanishing() {
        let fixture = Fixture::new(vec![hook(
            "pre/guard",
            serde_json::json!({
                "event": "pre-tool-use",
                "match": "((",
                "command": "guard",
                "failure_mode": "closed",
            }),
        )]);
        let hooks = fixture.build().hooks;
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].event, HookEvent::PreToolUse);
        assert!(
            hooks[0].matcher.is_none(),
            "refuses every tool it might guard"
        );
        assert!(hooks[0].refusal.as_deref().unwrap().contains("pre/guard"));
    }

    #[test]
    fn a_valid_hook_reads_through_the_shared_reader() {
        let fixture = Fixture::new(vec![hook(
            "pre/lint",
            serde_json::json!({
                "event": "pre-tool-use",
                "match": "bash",
                "command": "lint",
                "timeout_ms": 500,
                "failure_mode": "closed",
            }),
        )]);
        let hooks = fixture.build().hooks;
        assert_eq!(hooks.len(), 1);
        assert!(hooks[0].matcher.is_some());
        assert_eq!(hooks[0].timeout_ms, 500);
        assert_eq!(hooks[0].failure_mode, HookFailureMode::Closed);
        assert!(hooks[0].refusal.is_none());
    }
}
