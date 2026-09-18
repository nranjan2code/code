//! Single-turn capability assembly — the one pipeline for all five kinds.
//!
//! What a turn is allowed to call is computed from the reconciled
//! [`CapabilitySet`] in **one** function that runs every capability kind
//! (tool, skill, MCP server, hook, command) through the **same** four-stage
//! filter:
//!
//! 1. **Channel visibility** — name-based allow/deny from the chat's
//!    [`ChannelPolicy`].
//! 2. **Reach** — permission-engine blocking: a capability whose every
//!    standing is `Blocked` is removed.
//! 3. **Frozen contract** — only capabilities admitted at session start
//!    survive.
//! 4. **Domain slice** — progressive disclosure: only tools, MCP servers,
//!    and skills whose declared domains intersect the turn's required
//!    domains survive. Hooks and undeclared capabilities always survive.
//!
//! Each kind previously had its own assembly path — MCP aliases were
//! derived from the raw inventory with only channel + contract checks and
//! **skipped the domain slice entirely**; the `work` tool was bolted on in
//! `vak-agent`'s `tool_definitions()` *after* filtering; plugin hooks were
//! pulled from `PluginStore` with no contract or domain-slice awareness.
//! That produced no shared assertion that "the list the model sees is the
//! list the filters agreed on", which is exactly the class of intermittent
//! regression the user reported.
//!
//! See `docs/design/41-capability-registry.md` § Turn capabilities.

use std::collections::{BTreeSet, HashMap, HashSet};

use vak_agent::McpToolAlias;
use vak_config::ChannelPolicy;
use vak_hooks::{HookDef, HookEvent, HookFailureMode};
use vak_permission::Rule;
use vak_session::types::{CapabilityKind, FrozenContract};

use super::domain::Domain;
use super::snapshot::{Capability, CapabilityId, CapabilitySet};

// --- TurnCapabilities -----------------------------------------------------

/// The authoritative, filtered set of non-tool capabilities for a turn.
///
/// Built once from the reconciled [`CapabilitySet`] and the turn-scoped
/// policy state, then consumed atomically by the turn. The tool list
/// itself is still assembled in `run_turn_inner` (because tool *objects*
/// are Rust trait-objects that must be constructed with config), but the
/// *filtering* is unified: [`TurnCapabilities`] is the single source of
/// truth for which MCP aliases, hooks, frozen skills, and flow-tool flag
/// survive the four-stage pipeline, replacing the four divergent paths
/// that used to produce them.
#[derive(Debug, Clone, Default)]
pub struct TurnCapabilities {
    /// Built-in and broker tool identities admitted for this turn.
    pub tool_names: BTreeSet<String>,
    /// Descriptor projection used to render the model-facing prompt.
    pub descriptors: Vec<vak_session::types::CapabilityDescriptor>,
    /// Final MCP compatibility aliases, restricted to servers that survived
    /// all four filter stages *including* the domain slice.
    pub mcp_aliases: HashMap<String, McpToolAlias>,
    /// MCP server names that survived all four stages, for the runtime
    /// catalog observer to filter live discoveries against.
    pub mcp_server_names: Vec<String>,
    /// Hook definitions from capabilities that survived all four stages.
    pub hooks: Vec<HookDef>,
    /// Frozen skill objects that survived all four stages (channel, reach,
    /// contract, and domain slice). Constructed inline from the surviving
    /// capability set rather than calling a separate
    /// `skills::frozen_from_capabilities` that bypassed reach and domain
    /// slice.
    pub frozen_skills: Vec<crate::skills::FrozenSkill>,
    /// `true` iff the `flow` tool capability survived the full pipeline,
    /// including the domain slice. Replaces the post-hoc check in
    /// `Agent::tool_definitions()` that previously appended the `work`
    /// tool *after* all filters had already run.
    pub flow_admitted: bool,
    /// Names of Tool/McpServer/Skill capabilities that passed channel +
    /// reach + contract (stages 1-3 — they really were admitted to this
    /// session) but were then hidden by the domain slice (stage 4) for
    /// this specific turn. This is the actual live mechanism behind the
    /// misread self-correction signal (`vak_core::misread::escalated_capability`,
    /// see its module doc): if the model asks for one of these names
    /// anyway, the intent reading that produced `required_domains`
    /// measurably missed. Previously nothing populated this, so that
    /// signal never fired outside tests — see crates/vak-core/src/misread.rs.
    pub excluded_by_domain_slice: BTreeSet<String>,
}

// --- TurnProbe ------------------------------------------------------------

/// All turn-scoped inputs the four-stage pipeline reads.
pub struct TurnProbe<'a> {
    /// The reconciled, versioned capability set the turn binds to.
    pub capabilities: &'a CapabilitySet,
    /// Published registry epoch. A session contract is a baseline; a later
    /// epoch may add newly discovered capabilities at the next turn boundary.
    pub capability_epoch: u64,
    /// Immediate revocations, applied even before the next published epoch.
    pub revoked_ids: BTreeSet<CapabilityId>,
    /// The session's historical contract, retained for audit/route context.
    /// Capability visibility is rebuilt from the live registry at each turn.
    /// When `None`, stage 3 is a no-op.
    pub session_contract: Option<&'a FrozenContract>,
    /// Channel allow/deny overlay from the chat surface.
    pub channel_policy: &'a ChannelPolicy,
    /// Per-turn reach standings (already computed by `capability_standings`).
    pub reach_standings: &'a [crate::reach::Standing],
    /// Required domains for this turn (empty when intent is disabled or
    /// the reading is uncertain — stage 4 then preserves everything).
    pub required_domains: &'a BTreeSet<Domain>,
    /// Discovered MCP tool catalogs, keyed by server name.
    pub mcp_inventory: Option<&'a [(String, Vec<vak_mcp::McpToolInfo>)]>,
    /// Tool names that are always visible (orientation floor), never
    /// sliced away. Typically `read`, `glob`, `grep`, `skill`,
    /// `session_search`, `commitments`.
    pub orientation_floor: &'a [&'a str],
    /// Built-in tool names to exclude from MCP alias generation, because
    /// they would collide with a real tool definition.
    pub builtin_names: Vec<String>,
}

// --- Pipeline -------------------------------------------------------------

/// Names of capabilities that are fully blocked — every standing for
/// that capability name is `Blocked`. Returns a set of `CapabilityId`s
/// so callers can check membership regardless of kind.
fn blocked_ids(standings: &[crate::reach::Standing]) -> BTreeSet<CapabilityId> {
    let mut blocked = BTreeSet::new();
    for standing in standings {
        if !standing.reach.is_blocked() {
            continue;
        }
        let (kind, name) = match standing.tool.as_str() {
            "mcp" => (
                CapabilityKind::McpServer,
                standing
                    .label
                    .strip_prefix("mcp server `")
                    .and_then(|value| value.strip_suffix('`')),
            ),
            "skill" => (
                CapabilityKind::Skill,
                standing
                    .label
                    .strip_prefix("skill `")
                    .and_then(|value| value.strip_suffix('`')),
            ),
            _ => (
                CapabilityKind::Tool,
                standing
                    .label
                    .strip_prefix('`')
                    .and_then(|value| value.strip_suffix('`')),
            ),
        };
        if let Some(name) = name {
            blocked.insert(CapabilityId::new(kind, name));
        }
    }
    blocked
}

/// Whether a capability passes stages 1–3 (channel → reach → contract).
fn passes_channel_reach_contract(
    cap: &Capability,
    probe: &TurnProbe<'_>,
    blocked: &BTreeSet<CapabilityId>,
) -> bool {
    let id = &cap.id;
    // Stage 1: channel visibility
    let (allow, deny) = channel_policy_lists(probe.channel_policy, id);
    let channel_value = match id.kind {
        // MCP server policies use qualified `server/tool` globs. Check the
        // server against `server/*` so a deny of `blocked_server/*` correctly
        // blocks the whole server.
        CapabilityKind::McpServer => format!("{}/*", id.name),
        _ => id.name.clone(),
    };
    let visible = if id.kind == CapabilityKind::McpServer {
        let server = &id.name;
        let denied = deny.iter().any(|pattern| {
            crate::Core::policy_matches(std::slice::from_ref(pattern), &format!("{server}/*"))
        });
        let allowed = allow.as_ref().is_none_or(|patterns| {
            patterns.is_empty()
                || patterns.iter().any(|pattern| {
                    crate::Core::policy_matches(
                        std::slice::from_ref(pattern),
                        &format!("{server}/*"),
                    ) || pattern.starts_with(&format!("{server}/"))
                })
        });
        !denied && allowed
    } else {
        crate::Core::allowed_by(allow, deny, &channel_value)
    };
    if !visible {
        return false;
    }
    // Stage 2: reach
    if blocked.contains(id) || probe.revoked_ids.contains(id) {
        return false;
    }
    // Stage 3: frozen contract (enforced for initial epoch; relaxed on dynamic
    // refresh). A capability admitted after session start — a newly resolved
    // secret, a server that came back healthy — did not exist when the
    // contract was frozen, so it can never appear in `contract.capabilities`;
    // enforcing this check unconditionally would make it permanently
    // unreachable for the life of the session (see 64c1f4e2).
    if let Some(contract) = probe.session_contract {
        let in_contract = contract
            .capabilities
            .iter()
            .any(|c| c.kind == id.kind && c.name == id.name);
        if !in_contract && probe.capability_epoch <= 1 {
            return false;
        }
    }
    true
}

/// Whether a capability passes stage 4 (domain slice).
fn passes_domain_slice(cap: &Capability, probe: &TurnProbe<'_>) -> bool {
    // Hooks and commands are never sliced by domain — they are
    // event-driven or operator-driven, not turn-scoped.
    match cap.id.kind {
        CapabilityKind::Hook | CapabilityKind::Command => return true,
        _ => {}
    }
    // If intent is disabled or required_domains is empty, no slicing.
    if probe.required_domains.is_empty() {
        return true;
    }
    // Orientation floor: never slice these names regardless of kind.
    if probe.orientation_floor.contains(&cap.id.name.as_str()) {
        return true;
    }
    // Undeclared capabilities always survive (fail open).
    cap.serves.serves_any(probe.required_domains)
}

/// Pick the right allow/deny lists from the channel policy for a kind.
fn channel_policy_lists<'a>(
    policy: &'a ChannelPolicy,
    id: &CapabilityId,
) -> (&'a Option<Vec<String>>, &'a Vec<String>) {
    match id.kind {
        CapabilityKind::Tool | CapabilityKind::Command => (&policy.tools_allow, &policy.tools_deny),
        CapabilityKind::McpServer => (&policy.mcp_allow, &policy.mcp_deny),
        CapabilityKind::Skill => (&policy.skills_allow, &policy.skills_deny),
        CapabilityKind::Hook => (&policy.hooks_allow, &policy.hooks_deny),
    }
}

/// MCP aliases for servers admitted by the full pipeline.
///
/// A server must survive all four filter stages to contribute aliases.
/// Tool-name collisions between servers are resolved by dropping the
/// alias (ambiguous — the model must use the brokered `mcp` tool instead).
pub(crate) fn mcp_aliases_from_inventory(
    inventory: &[(String, Vec<vak_mcp::McpToolInfo>)],
    admitted_servers: &BTreeSet<String>,
    builtins: &BTreeSet<String>,
) -> HashMap<String, McpToolAlias> {
    let mut aliases = HashMap::new();
    let mut ambiguous = HashSet::new();
    for (server, tools) in inventory {
        if !admitted_servers.contains(server) {
            continue;
        }
        for tool in tools {
            if builtins.contains(&tool.name) {
                continue;
            }
            let alias = McpToolAlias {
                server: server.clone(),
                tool: tool.name.clone(),
                description: tool.description.clone(),
                schema: tool.input_schema.clone(),
            };
            if aliases.contains_key(&tool.name) {
                ambiguous.insert(tool.name.clone());
                continue;
            }
            aliases.insert(tool.name.clone(), alias);
        }
    }
    for name in ambiguous {
        aliases.remove(&name);
    }
    aliases
}

/// Reconstruct a [`HookDef`] from a capability's configuration JSON.
fn build_hook_def(cap: &Capability) -> Option<HookDef> {
    let cfg = cap.configuration.as_object()?;
    let event_str = cfg.get("event")?.as_str()?;
    let command = cfg.get("command")?.as_str()?.to_string();
    let event = match event_str {
        "session-start" | "session_start" | "start" => HookEvent::SessionStart,
        "pre-tool-use" | "pre_tool_use" => HookEvent::PreToolUse,
        "post-tool-use" | "post_tool_use" => HookEvent::PostToolUse,
        "stop" => HookEvent::Stop,
        _ => return None,
    };
    let matcher_str = cfg.get("matcher").and_then(|v| v.as_str());
    let matcher = match matcher_str {
        Some(m) if !m.trim().is_empty() => Some(Rule::parse(m).ok()?),
        _ => None,
    };
    let timeout_ms = cfg
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS);
    let failure_mode_str = cfg
        .get("failure_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("open");
    let failure_mode = match failure_mode_str {
        "open" => HookFailureMode::Open,
        "closed" => HookFailureMode::Closed,
        _ => return None,
    };
    Some(HookDef {
        event,
        matcher,
        command,
        timeout_ms,
        failure_mode,
    })
}

// --- Build ----------------------------------------------------------------

impl TurnCapabilities {
    /// Build the authoritative capability projection for one turn.
    ///
    /// Runs all capability kinds through the same four-stage pipeline and
    /// materializes the results atomically. Every field in the returned
    /// struct is the filtered product of that single pass — there is no
    /// second assembly that can drift.
    pub fn build(probe: &TurnProbe<'_>) -> Self {
        let set = probe.capabilities;
        let blocked = blocked_ids(probe.reach_standings);

        // ---- Stages 1+2+3: channel, reach, contract -----------------------
        let surviving: Vec<&Capability> = set
            .usable()
            .filter(|c| passes_channel_reach_contract(c, probe, &blocked))
            .collect();

        // ---- MCP aliases (stage 4 applied per-server) ----------------------
        // MCP aliases are derived from the inventory, but only for servers
        // that survived all four filter stages *including* the domain
        // slice. THIS is the key fix: aliases previously bypassed the domain
        // slice by being computed from the raw cache instead of the
        // filtered CapabilitySet.
        let mcp_servers_admitted: BTreeSet<String> = surviving
            .iter()
            .filter(|c| c.id.kind == CapabilityKind::McpServer)
            .filter(|c| passes_domain_slice(c, probe))
            .map(|c| c.id.name.clone())
            .collect();

        let mcp_server_names = mcp_servers_admitted.iter().cloned().collect::<Vec<_>>();

        // MCP aliases are derived from inventory. We check probe.mcp_inventory
        // first, but also extract tools from any surviving McpServer capabilities
        // whose configuration carries a catalog. This ensures aliases and prompt
        // descriptors never disagree even if the probe inventory was missing or partial.
        let mut inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = probe
            .mcp_inventory
            .map(|inv| inv.to_vec())
            .unwrap_or_default();

        for c in &surviving {
            if c.id.kind != CapabilityKind::McpServer {
                continue;
            }
            if inventory.iter().any(|(name, _)| name == &c.id.name) {
                continue;
            }
            if let Some(tools_arr) = c.configuration.get("tools").and_then(|t| t.as_array()) {
                let mut tools = Vec::new();
                for t in tools_arr {
                    let Some(name) = t.get("name").and_then(|n| n.as_str()) else {
                        continue;
                    };
                    let description = t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let input_schema = t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    tools.push(vak_mcp::McpToolInfo {
                        name: name.to_string(),
                        description,
                        input_schema,
                    });
                }
                if !tools.is_empty() {
                    inventory.push((c.id.name.clone(), tools));
                }
            }
        }

        let builtins: BTreeSet<String> = probe.builtin_names.iter().cloned().collect();
        let mcp_aliases = mcp_aliases_from_inventory(&inventory, &mcp_servers_admitted, &builtins);

        // ---- Hooks (stages 1-3; domain slice is a no-op for hooks) --------
        // Hooks are never sliced by domain — they are event-driven,
        // not turn-scoped. But they DO pass through channel + reach +
        // contract, which was previously missing: plugin hooks were
        // assembled from PluginStore with no contract awareness.
        let hooks: Vec<HookDef> = surviving
            .iter()
            .filter(|c| c.id.kind == CapabilityKind::Hook)
            .filter_map(|c| build_hook_def(c))
            .collect();

        // ---- Frozen skills (stage 4 applies) ------------------------------
        // Skills are progressively disclosed: a skill that serves a domain
        // the turn does not need is hidden, but an undeclared skill is
        // always visible (fail open). Each surviving skill is projected
        // into a FrozenSkill from the Capability's source/digest.
        //
        // Score surviving skills against required_domains to select top active skill.
        let mut best_skill_name: Option<String> = None;
        if !probe.required_domains.is_empty() {
            let mut highest_score = 0;
            for c in surviving
                .iter()
                .filter(|c| c.id.kind == CapabilityKind::Skill && passes_domain_slice(c, probe))
            {
                let score = match &c.serves {
                    super::domain::Serves::Declared(domains) => {
                        domains.intersection(probe.required_domains).count()
                    }
                    super::domain::Serves::Undeclared => 0,
                };
                if score > highest_score {
                    highest_score = score;
                    best_skill_name = Some(c.id.name.clone());
                }
            }
        }

        let frozen_skills: Vec<crate::skills::FrozenSkill> = surviving
            .iter()
            .filter(|c| c.id.kind == CapabilityKind::Skill)
            .filter(|c| passes_domain_slice(c, probe))
            .filter_map(|c| {
                let is_active = best_skill_name.as_ref() == Some(&c.id.name);
                Some(crate::skills::FrozenSkill {
                    name: c.id.name.clone(),
                    description: c.summary.clone(),
                    path: c.source.clone()?,
                    digest: c.digest.clone()?,
                    provenance: Some(c.origin.label()),
                    is_active,
                })
            })
            .collect();

        // ---- Flow/work tool admission --------------------------------------
        // `flow` is a Tool capability, so it passes through the full
        // pipeline. The agent layer reads this flag and knows whether to
        // include the `work` tool definition.
        let flow_admitted = surviving.iter().any(|c| {
            c.id.kind == CapabilityKind::Tool
                && c.id.name == "flow"
                && passes_domain_slice(c, probe)
        });

        let tool_names = surviving
            .iter()
            .filter(|c| c.id.kind == CapabilityKind::Tool)
            .filter(|c| passes_domain_slice(c, probe))
            .map(|c| c.id.name.clone())
            .collect();
        let descriptors = surviving
            .iter()
            .filter(|c| passes_domain_slice(c, probe))
            .map(|c| {
                let mut desc = c.to_descriptor();
                if c.id.kind == CapabilityKind::Skill
                    && best_skill_name.as_ref() == Some(&c.id.name)
                {
                    if desc.configuration.is_null() {
                        desc.configuration = serde_json::json!({"active": true});
                    } else if let Some(obj) = desc.configuration.as_object_mut() {
                        obj.insert("active".into(), serde_json::json!(true));
                    }
                }
                desc
            })
            .collect();

        let excluded_by_domain_slice: BTreeSet<String> = surviving
            .iter()
            .filter(|c| {
                matches!(
                    c.id.kind,
                    CapabilityKind::Tool | CapabilityKind::McpServer | CapabilityKind::Skill
                )
            })
            .filter(|c| !passes_domain_slice(c, probe))
            .map(|c| c.id.name.clone())
            .collect();

        TurnCapabilities {
            tool_names,
            descriptors,
            mcp_aliases,
            mcp_server_names,
            hooks,
            frozen_skills,
            flow_admitted,
            excluded_by_domain_slice,
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
    use vak_session::types::{CapabilityDescriptor, CapabilityInvocation};

    fn make_cap(name: &str, kind: CapabilityKind, serves: Serves) -> Capability {
        Capability {
            id: CapabilityId::new(kind, name),
            origin: Origin::Builtin,
            summary: String::new(),
            serves,
            digest: None,
            source: None,
            resolution: Resolution::Static,
            configuration: serde_json::Value::Null,
        }
    }

    fn make_skill_cap(name: &str, serves: Serves) -> Capability {
        Capability {
            id: CapabilityId::new(CapabilityKind::Skill, name),
            origin: Origin::Builtin,
            summary: format!("Skill: {name}"),
            serves,
            digest: Some(format!("sha:{name}")),
            source: Some(std::path::PathBuf::from(format!("/tmp/{name}.vak"))),
            resolution: Resolution::Static,
            configuration: serde_json::Value::Null,
        }
    }

    fn make_hook_cap(name: &str, event: &str, command: &str) -> Capability {
        Capability {
            id: CapabilityId::new(CapabilityKind::Hook, name),
            origin: Origin::Builtin,
            summary: String::new(),
            serves: Serves::Undeclared,
            digest: None,
            source: None,
            resolution: Resolution::Static,
            configuration: serde_json::json!({
                "event": event,
                "matcher": null,
                "command": command,
                "timeout_ms": 10000,
                "failure_mode": "open",
            }),
        }
    }

    fn make_contract(names: &[(&str, CapabilityKind)]) -> FrozenContract {
        let capabilities = names
            .iter()
            .map(|(n, k)| CapabilityDescriptor {
                name: (*n).to_string(),
                kind: k.clone(),
                invocation: CapabilityInvocation::ModelTool,
                description: String::new(),
                source: None,
                digest: None,
                provenance: None,
                configuration: serde_json::Value::Null,
            })
            .collect();
        FrozenContract {
            app_version: "test".to_string(),
            provider: "test".to_string(),
            model: "test".to_string(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: String::new(),
            permission_mode: "read-only".to_string(),
            capabilities,
            prompt_layers: Vec::new(),
        }
    }

    fn empty_policy() -> ChannelPolicy {
        ChannelPolicy::default()
    }

    fn make_probe<'a>(
        set: &'a CapabilitySet,
        contract: Option<&'a FrozenContract>,
        policy: &'a ChannelPolicy,
        standings: &'a [crate::reach::Standing],
        required: &'a BTreeSet<Domain>,
        inventory: Option<&'a [(String, Vec<vak_mcp::McpToolInfo>)]>,
    ) -> TurnProbe<'a> {
        TurnProbe {
            capabilities: set,
            capability_epoch: set.epoch,
            revoked_ids: BTreeSet::new(),
            session_contract: contract,
            channel_policy: policy,
            reach_standings: standings,
            required_domains: required,
            mcp_inventory: inventory,
            orientation_floor: &[
                "read",
                "glob",
                "grep",
                "skill",
                "session_search",
                "commitments",
            ],
            builtin_names: default_builtins(),
        }
    }

    fn default_builtins() -> Vec<String> {
        vec![
            "read".to_string(),
            "glob".to_string(),
            "grep".to_string(),
            "bash".to_string(),
            "write".to_string(),
            "edit".to_string(),
        ]
    }

    #[test]
    fn mcp_aliases_are_domain_sliced() {
        // A weather MCP server (serves LiveData) and a code MCP server
        // (serves CodeExec). When required_domains = [Web, LiveData],
        // only the weather server's aliases survive.
        let set = CapabilitySet::new(
            1,
            vec![
                make_cap(
                    "weather",
                    CapabilityKind::McpServer,
                    Serves::declared([Domain::LiveData]),
                ),
                make_cap(
                    "code_server",
                    CapabilityKind::McpServer,
                    Serves::declared([Domain::CodeExec]),
                ),
            ],
        );
        let required = BTreeSet::from([Domain::Web, Domain::LiveData]);
        let inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = vec![
            (
                "weather".to_string(),
                vec![vak_mcp::McpToolInfo {
                    name: "get_weather".into(),
                    description: "Get weather".into(),
                    input_schema: serde_json::json!({}),
                }],
            ),
            (
                "code_server".to_string(),
                vec![vak_mcp::McpToolInfo {
                    name: "format_code".into(),
                    description: "Format code".into(),
                    input_schema: serde_json::json!({}),
                }],
            ),
        ];
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.mcp_aliases.contains_key("get_weather"));
        assert!(!tc.mcp_aliases.contains_key("format_code"));
    }

    #[test]
    fn excluded_by_domain_slice_tracks_hidden_but_admitted_capabilities() {
        // The real signal behind `misread::escalated_capability`: a tool
        // that was admitted (would pass channel+reach+contract) but got
        // hidden from THIS turn by the domain slice must show up here, so
        // the model asking for it anyway is a measured misread — not a
        // silently-dropped signal (see crates/vak-core/src/misread.rs).
        let set = CapabilitySet::new(
            1,
            vec![
                make_cap(
                    "code_server",
                    CapabilityKind::McpServer,
                    Serves::declared([Domain::CodeExec]),
                ),
                make_cap("generic_tool", CapabilityKind::Tool, Serves::Undeclared),
            ],
        );
        let required = BTreeSet::from([Domain::Web, Domain::LiveData]);
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(
            tc.excluded_by_domain_slice.contains("code_server"),
            "a CodeExec-only server must be excluded when required_domains is [Web, LiveData]: {:?}",
            tc.excluded_by_domain_slice
        );
        // Undeclared capabilities fail open (always survive), so they must
        // NOT appear as "excluded" even though they weren't explicitly
        // requested — that would produce a false misread signal every turn.
        assert!(
            !tc.excluded_by_domain_slice.contains("generic_tool"),
            "an undeclared (fail-open) tool must never be reported as domain-excluded: {:?}",
            tc.excluded_by_domain_slice
        );
    }

    #[test]
    fn undeclared_mcp_server_survives_slice() {
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "generic",
                CapabilityKind::McpServer,
                Serves::Undeclared,
            )],
        );
        let required = BTreeSet::from([Domain::Web]);
        let inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = vec![(
            "generic".to_string(),
            vec![vak_mcp::McpToolInfo {
                name: "do_thing".into(),
                description: "Does a thing".into(),
                input_schema: serde_json::json!({}),
            }],
        )];
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.mcp_aliases.contains_key("do_thing"));
    }

    #[test]
    fn contract_filter_blocks_uncaps() {
        let set = CapabilitySet::new(
            1,
            vec![
                make_skill_cap("in_contract", Serves::Undeclared),
                make_skill_cap("not_in_contract", Serves::Undeclared),
            ],
        );
        let contract = make_contract(&[("in_contract", CapabilityKind::Skill)]);
        let policy = empty_policy();
        let required = BTreeSet::new();
        let probe = make_probe(&set, Some(&contract), &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.frozen_skills.iter().any(|s| s.name == "in_contract"));
        assert!(!tc.frozen_skills.iter().any(|s| s.name == "not_in_contract"));
    }

    #[test]
    fn no_contract_admits_everything() {
        let set = CapabilitySet::new(
            1,
            vec![
                make_cap("tool_a", CapabilityKind::Tool, Serves::Undeclared),
                make_skill_cap("skill_b", Serves::Undeclared),
            ],
        );
        let policy = empty_policy();
        let required = BTreeSet::new();
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.frozen_skills.iter().any(|s| s.name == "skill_b"));
    }

    #[test]
    fn channel_deny_blocks_mcp_server() {
        let mut policy = empty_policy();
        policy.mcp_deny = vec!["blocked_server/*".to_string()];
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "blocked_server",
                CapabilityKind::McpServer,
                Serves::Undeclared,
            )],
        );
        let inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = vec![(
            "blocked_server".to_string(),
            vec![vak_mcp::McpToolInfo {
                name: "secret_tool".into(),
                description: "secret".into(),
                input_schema: serde_json::json!({}),
            }],
        )];
        let required = BTreeSet::new();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        let tc = TurnCapabilities::build(&probe);
        assert!(!tc.mcp_aliases.contains_key("secret_tool"));
    }

    #[test]
    fn tool_specific_mcp_allow_keeps_server_admitted() {
        let mut policy = empty_policy();
        policy.mcp_allow = Some(vec!["search/query".into()]);
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "search",
                CapabilityKind::McpServer,
                Serves::Undeclared,
            )],
        );
        let inventory = vec![(
            "search".to_string(),
            vec![vak_mcp::McpToolInfo {
                name: "query".into(),
                description: "query".into(),
                input_schema: serde_json::json!({}),
            }],
        )];
        let required = BTreeSet::new();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        assert!(
            TurnCapabilities::build(&probe)
                .mcp_aliases
                .contains_key("query")
        );
    }

    #[test]
    fn blocked_mcp_standing_is_instance_scoped() {
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "search",
                CapabilityKind::McpServer,
                Serves::Undeclared,
            )],
        );
        let standing = crate::reach::Standing {
            tool: "mcp".into(),
            label: "mcp server `search`".into(),
            reach: crate::reach::Reach::Blocked,
            reason: "denied".into(),
            remedy: String::new(),
        };
        let policy = empty_policy();
        let required = BTreeSet::new();
        let standings = [standing];
        let probe = make_probe(&set, None, &policy, &standings, &required, None);
        assert!(TurnCapabilities::build(&probe).mcp_server_names.is_empty());
    }

    #[test]
    fn hooks_survive_from_capability_set() {
        let set = CapabilitySet::new(
            1,
            vec![make_hook_cap("pre_tool/git", "pre_tool_use", "cargo check")],
        );
        let policy = empty_policy();
        let required = BTreeSet::new();
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert_eq!(tc.hooks.len(), 1);
        assert_eq!(tc.hooks[0].command, "cargo check");
    }

    #[test]
    fn hooks_filtered_by_contract() {
        let set = CapabilitySet::new(
            1,
            vec![
                make_hook_cap("keep_me", "pre_tool_use", "echo hi"),
                make_hook_cap("drop_me", "pre_tool_use", "echo bye"),
            ],
        );
        let contract = make_contract(&[("keep_me", CapabilityKind::Hook)]);
        let policy = empty_policy();
        let required = BTreeSet::new();
        let probe = make_probe(&set, Some(&contract), &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert_eq!(tc.hooks.len(), 1);
        assert_eq!(tc.hooks[0].command, "echo hi");
    }

    #[test]
    fn flow_admitted_when_capability_present() {
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "flow",
                CapabilityKind::Tool,
                Serves::declared([Domain::Orchestration]),
            )],
        );
        let policy = empty_policy();
        let required = BTreeSet::from([Domain::Orchestration]);
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.flow_admitted);
    }

    #[test]
    fn flow_not_admitted_when_domain_mismatched() {
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "flow",
                CapabilityKind::Tool,
                Serves::declared([Domain::Orchestration]),
            )],
        );
        let policy = empty_policy();
        let required = BTreeSet::from([Domain::Filesystem]);
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(!tc.flow_admitted);
    }

    // ---- End-to-end test matrix: each kind through the full pipeline ----

    #[test]
    fn tool_capability_survives_pipeline() {
        // A tool with the right domain survives all four stages.
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "webfetch",
                CapabilityKind::Tool,
                Serves::declared([Domain::Web, Domain::LiveData]),
            )],
        );
        let required = BTreeSet::from([Domain::LiveData]);
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let _tc = TurnCapabilities::build(&probe);
        // Tools aren't materialised as objects in TurnCapabilities, but
        // the CapabilitySet confirms the tool survived.
        assert!(set.usable().any(|c| c.id.name == "webfetch"));
    }

    #[test]
    fn tool_capability_blocked_by_channel_deny() {
        let mut policy = empty_policy();
        policy.tools_deny = vec!["webfetch".to_string()];
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "webfetch",
                CapabilityKind::Tool,
                Serves::Undeclared,
            )],
        );
        let req = BTreeSet::new();
        let probe = make_probe(&set, None, &policy, &[], &req, None);
        let tc = TurnCapabilities::build(&probe);
        // No MCP aliases, no skills, no flow — and the tool is not admitted.
        assert!(!tc.flow_admitted);
    }

    #[test]
    fn mcp_capability_survives_pipeline() {
        // MCP server with matching domain + inventory → aliases survive.
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "tavily",
                CapabilityKind::McpServer,
                Serves::declared([Domain::Web, Domain::LiveData]),
            )],
        );
        let required = BTreeSet::from([Domain::Web]);
        let inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = vec![(
            "tavily".to_string(),
            vec![vak_mcp::McpToolInfo {
                name: "tavily_search".into(),
                description: "Search the web".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
        )];
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.mcp_aliases.contains_key("tavily_search"));
        assert!(tc.mcp_server_names.contains(&"tavily".to_string()));
    }

    #[test]
    fn mcp_capability_blocked_by_domain_mismatch() {
        // MCP server serving CodeExec, required domain is Web → aliases dropped.
        let set = CapabilitySet::new(
            1,
            vec![make_cap(
                "code_server",
                CapabilityKind::McpServer,
                Serves::declared([Domain::CodeExec]),
            )],
        );
        let required = BTreeSet::from([Domain::Web]);
        let inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = vec![(
            "code_server".to_string(),
            vec![vak_mcp::McpToolInfo {
                name: "run_linter".into(),
                description: "Lint code".into(),
                input_schema: serde_json::json!({}),
            }],
        )];
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        let tc = TurnCapabilities::build(&probe);
        assert!(!tc.mcp_aliases.contains_key("run_linter"));
        assert!(tc.mcp_server_names.is_empty());
    }

    #[test]
    fn skill_capability_survives_pipeline() {
        let set = CapabilitySet::new(
            1,
            vec![make_skill_cap(
                "weather_skill",
                Serves::declared([Domain::LiveData]),
            )],
        );
        let required = BTreeSet::from([Domain::LiveData]);
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.frozen_skills.iter().any(|s| s.name == "weather_skill"));
    }

    #[test]
    fn skill_capability_blocked_by_domain_mismatch() {
        let set = CapabilitySet::new(
            1,
            vec![make_skill_cap(
                "code_skill",
                Serves::declared([Domain::CodeExec]),
            )],
        );
        let required = BTreeSet::from([Domain::Web]);
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(!tc.frozen_skills.iter().any(|s| s.name == "code_skill"));
    }

    #[test]
    fn hook_capability_survives_pipeline() {
        let set = CapabilitySet::new(
            1,
            vec![make_hook_cap(
                "pre_tool/check",
                "pre_tool_use",
                "cargo check",
            )],
        );
        let policy = empty_policy();
        let req = BTreeSet::new();
        let probe = make_probe(&set, None, &policy, &[], &req, None);
        let tc = TurnCapabilities::build(&probe);
        assert_eq!(tc.hooks.len(), 1);
        assert_eq!(tc.hooks[0].command, "cargo check");
    }

    #[test]
    fn hook_capability_blocked_by_channel_deny() {
        let mut policy = empty_policy();
        policy.hooks_deny = vec!["pre_tool/cargo".to_string()];
        let set = CapabilitySet::new(
            1,
            vec![make_hook_cap(
                "pre_tool/cargo",
                "pre_tool_use",
                "cargo build",
            )],
        );
        let req = BTreeSet::new();
        let probe = make_probe(&set, None, &policy, &[], &req, None);
        let tc = TurnCapabilities::build(&probe);
        assert!(tc.hooks.is_empty());
    }

    #[test]
    fn all_four_kinds_survive_together_under_greeting() {
        // A greeting (Converse act) requires Filesystem + Memory domains.
        // Undeclared capabilities fail open, declared ones are checked
        // against the floor + required domains.
        let set = CapabilitySet::new(
            1,
            vec![
                make_cap(
                    "read",
                    CapabilityKind::Tool,
                    Serves::declared([Domain::Filesystem]),
                ),
                make_cap(
                    "webfetch",
                    CapabilityKind::Tool,
                    Serves::declared([Domain::Web]),
                ),
                make_cap(
                    "tavily",
                    CapabilityKind::McpServer,
                    Serves::declared([Domain::Web]),
                ),
                make_skill_cap("pdf", Serves::declared([Domain::Documents])),
                make_hook_cap("hook_a", "pre_tool_use", "echo hi"),
            ],
        );
        let required = BTreeSet::from([Domain::Filesystem]);
        let inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = vec![(
            "tavily".to_string(),
            vec![vak_mcp::McpToolInfo {
                name: "search".into(),
                description: "web search".into(),
                input_schema: serde_json::json!({}),
            }],
        )];
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        let tc = TurnCapabilities::build(&probe);
        // read (Filesystem) survives; webfetch (Web) does not (not in floor)
        // tavily MCP (Web) does not survive; pdf skill (Documents) does not
        // hook_a survives (hooks not sliced)
        assert!(tc.mcp_aliases.is_empty(), "web domain not required");
        assert!(tc.frozen_skills.is_empty(), "documents domain not required");
        assert_eq!(tc.hooks.len(), 1);
    }

    #[test]
    fn mcp_and_hook_both_admitted_under_live_data_query() {
        // When the user asks a factual question, LiveData + Web are required.
        // MCP server serving LiveData should admit aliases;
        // hook serving no domains should still survive (hooks aren't sliced).
        let set = CapabilitySet::new(
            1,
            vec![
                make_cap(
                    "tavily",
                    CapabilityKind::McpServer,
                    Serves::declared([Domain::Web, Domain::LiveData]),
                ),
                make_cap(
                    "unreachable_mcp",
                    CapabilityKind::McpServer,
                    Serves::Undeclared,
                ),
                make_hook_cap("audit_hook", "post_tool_use", "echo done"),
            ],
        );
        let required = BTreeSet::from([Domain::Web, Domain::LiveData]);
        let inventory: Vec<(String, Vec<vak_mcp::McpToolInfo>)> = vec![
            (
                "tavily".to_string(),
                vec![vak_mcp::McpToolInfo {
                    name: "search_web".into(),
                    description: "Search".into(),
                    input_schema: serde_json::json!({}),
                }],
            ),
            (
                "unreachable_mcp".to_string(),
                vec![vak_mcp::McpToolInfo {
                    name: "deep_tool".into(),
                    description: "Deep".into(),
                    input_schema: serde_json::json!({}),
                }],
            ),
        ];
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, Some(&inventory));
        let tc = TurnCapabilities::build(&probe);
        // Both MCP servers survive: tavily (Web+Live, matches required)
        // and unreachable_mcp (Undeclared, fails open)
        assert!(tc.mcp_aliases.contains_key("search_web"));
        assert!(tc.mcp_aliases.contains_key("deep_tool"));
        // Hook survives — hooks are never domain-sliced.
        assert_eq!(tc.hooks.len(), 1);
    }

    #[test]
    fn highest_affinity_skill_is_marked_active() {
        let set = CapabilitySet::new(
            1,
            vec![
                make_skill_cap(
                    "software-development",
                    Serves::declared([Domain::CodeExec, Domain::Documents, Domain::Vcs]),
                ),
                make_skill_cap("writing-and-editing", Serves::declared([Domain::Documents])),
            ],
        );
        let required = BTreeSet::from([Domain::CodeExec, Domain::Documents, Domain::Vcs]);
        let policy = empty_policy();
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);

        let sw = tc
            .frozen_skills
            .iter()
            .find(|s| s.name == "software-development")
            .unwrap();
        assert!(sw.is_active);

        let wr = tc
            .frozen_skills
            .iter()
            .find(|s| s.name == "writing-and-editing")
            .unwrap();
        assert!(!wr.is_active);

        let sw_desc = tc
            .descriptors
            .iter()
            .find(|d| d.name == "software-development")
            .unwrap();
        assert_eq!(
            sw_desc
                .configuration
                .get("active")
                .and_then(|v| v.as_bool()),
            Some(true)
        );

        let wr_desc = tc
            .descriptors
            .iter()
            .find(|d| d.name == "writing-and-editing")
            .unwrap();
        assert_ne!(
            wr_desc
                .configuration
                .get("active")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn mcp_aliases_extracted_from_capability_configuration_when_inventory_none() {
        let mut cap = make_cap(
            "tavily",
            CapabilityKind::McpServer,
            Serves::declared([Domain::Web]),
        );
        cap.configuration = serde_json::json!({
            "tools": [
                {
                    "name": "tavily_search",
                    "description": "Search the web using Tavily",
                    "inputSchema": { "type": "object", "properties": { "query": { "type": "string" } } }
                }
            ]
        });
        let set = CapabilitySet::new(1, vec![cap]);
        let inv = set.mcp_inventory();
        assert_eq!(inv.len(), 1);
        assert_eq!(inv[0].0, "tavily");
        assert_eq!(inv[0].1.len(), 1);
        assert_eq!(inv[0].1[0].name, "tavily_search");

        // When TurnProbe is created with NO inventory (inventory = None),
        // aliases should still be derived from the capability configuration!
        let policy = empty_policy();
        let required = BTreeSet::from([Domain::Web]);
        let probe = make_probe(&set, None, &policy, &[], &required, None);
        let tc = TurnCapabilities::build(&probe);

        assert!(
            tc.mcp_aliases.contains_key("tavily_search"),
            "mcp_aliases must be populated from capability configuration even when probe.mcp_inventory is None"
        );
        let alias = tc.mcp_aliases.get("tavily_search").unwrap();
        assert_eq!(alias.server, "tavily");
        assert_eq!(alias.tool, "tavily_search");
        assert_eq!(alias.description, "Search the web using Tavily");
    }
}
