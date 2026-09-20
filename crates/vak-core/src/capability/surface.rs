//! The per-turn tool surface: core (in the stable prefix) vs. deferred
//! (schema withheld, reachable only through `find_tools` or, on Anthropic,
//! `defer_loading`). See docs/design/68-context-engine.md §5.
//!
//! This module decides a *presentation* split over tools the rest of the
//! pipeline already admitted for the turn — it never widens what was
//! admitted, and it never re-derives domain membership on its own: the
//! domain match comes from [`crate::intent::slice_capabilities`], the same
//! selector `crate::intent` exposes for any other caller.

use std::collections::{BTreeMap, BTreeSet};

use vak_llm::ToolDefinition;
use vak_session::types::{CapabilityDescriptor, CapabilityKind};

/// Tool names carried in the stable prefix on every turn, regardless of
/// domain: the orientation and discovery primitives a model needs to
/// operate at all. `recall` is included by name for forward-compatibility
/// with the evidence store (docs/design/68 §3) — today no capability named
/// `recall` exists, so it is silently absent from both `core` and
/// `deferred` rather than advertised as a broken tool.
const ALWAYS_CORE: &[&str] = &["find_tools", "skill", "mcp", "recall"];

fn is_card_tool(name: &str) -> bool {
    name.starts_with("emit_") && name.ends_with("_card")
}

/// The result of splitting one turn's admitted tools into what stays in the
/// stable prefix and what is only reachable on demand.
#[derive(Debug, Clone, Default)]
pub struct ToolSurface {
    /// Always sent with full schemas, in the stable prefix.
    pub core: Vec<ToolDefinition>,
    /// Schema withheld from the prefix; reachable via `find_tools`, or via
    /// Anthropic `defer_loading` on legs that support it.
    pub deferred: Vec<ToolDefinition>,
    /// One line per deferred tool — `- name — first sentence`. No schemas.
    pub index: String,
}

/// Build the tool surface for one turn.
///
/// `admitted` is the turn's already-filtered capability descriptors (channel
/// → reach → contract → domain slice all already applied); only
/// [`CapabilityKind::Tool`] entries are considered — an MCP server can never
/// reach `core` or `deferred` here, because it is reached only through the
/// `mcp` broker (docs/design/68 §5). `tool_defs` are the corresponding
/// schemas, already built for the turn (e.g. by `vak_tools::definitions`).
/// `required_domains` and `declared_serves` are passed straight through to
/// [`crate::intent::slice_capabilities`]: `All` (a disabled kernel) puts
/// every admitted tool in core, an explicit domain set keeps what serves it,
/// and the orientation floor — what a reading too weak to slice arrives as —
/// keeps only floor and undeclared tools in core. The `ALWAYS_CORE` set
/// survives independently of all of that.
pub fn build_tool_surface(
    admitted: &[CapabilityDescriptor],
    tool_defs: &[ToolDefinition],
    required_domains: &vak_intent::DomainSet,
    declared_serves: &BTreeMap<String, Vec<String>>,
    renders_cards: bool,
) -> ToolSurface {
    let tool_names: BTreeSet<&str> = admitted
        .iter()
        .filter(|c| c.kind == CapabilityKind::Tool)
        .map(|c| c.name.as_str())
        .collect();

    let admitted_tools: Vec<CapabilityDescriptor> = admitted
        .iter()
        .filter(|c| c.kind == CapabilityKind::Tool)
        .cloned()
        .collect();
    let domain_selected: BTreeSet<String> =
        crate::intent::slice_capabilities(&admitted_tools, required_domains, declared_serves)
            .into_iter()
            .map(|c| c.name)
            .collect();

    let mut core = Vec::new();
    let mut deferred = Vec::new();
    for def in tool_defs {
        // Anything not a Tool-kind admitted capability — an MCP alias, a
        // stale name — is never advertised directly at all.
        if !tool_names.contains(def.name.as_str()) {
            continue;
        }
        let always_core =
            ALWAYS_CORE.contains(&def.name.as_str()) || (renders_cards && is_card_tool(&def.name));
        if always_core || domain_selected.contains(&def.name) {
            core.push(def.clone());
        } else {
            deferred.push(def.clone());
        }
    }

    let index = deferred
        .iter()
        .map(|def| format!("- {} — {}", def.name, first_sentence(&def.description)))
        .collect::<Vec<_>>()
        .join("\n");

    ToolSurface {
        core,
        deferred,
        index,
    }
}

fn first_sentence(description: &str) -> &str {
    let trimmed = description.trim();
    if let Some(idx) = trimmed.find(". ") {
        return &trimmed[..=idx];
    }
    trimmed.lines().next().unwrap_or(trimmed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_session::types::CapabilityInvocation;

    fn tool_cap(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: name.into(),
            kind: CapabilityKind::Tool,
            invocation: CapabilityInvocation::ModelTool,
            description: String::new(),
            source: None,
            digest: None,
            provenance: None,
            configuration: serde_json::Value::Null,
        }
    }

    fn mcp_cap(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            kind: CapabilityKind::McpServer,
            ..tool_cap(name)
        }
    }

    fn def(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition::new(name, description, serde_json::json!({}))
    }

    fn domains(values: &[&str]) -> vak_intent::DomainSet {
        vak_intent::DomainSet::only(values.iter().copied())
    }

    fn serves(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(name, ds)| {
                (
                    name.to_string(),
                    ds.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
                )
            })
            .collect()
    }

    #[test]
    fn always_core_tools_stay_core_regardless_of_domain() {
        let admitted = vec![tool_cap("find_tools"), tool_cap("skill"), tool_cap("mcp")];
        let defs = vec![
            def("find_tools", "Search deferred tools."),
            def("skill", "Load a skill."),
            def("mcp", "Call an MCP server."),
        ];
        let surface = build_tool_surface(
            &admitted,
            &defs,
            &vak_intent::DomainSet::Empty,
            &BTreeMap::new(),
            false,
        );
        assert_eq!(surface.core.len(), 3);
        assert!(surface.deferred.is_empty());
    }

    #[test]
    fn card_tools_are_core_only_when_the_surface_renders_cards() {
        // Declared (not undeclared) and domain-mismatched, so the only way
        // into `core` is the card-tool rule this test exercises — an
        // undeclared tool would fail open through `slice_capabilities`
        // regardless of `renders_cards` and defeat the test.
        let admitted = vec![tool_cap("emit_metric_card")];
        let defs = vec![def("emit_metric_card", "Emit a metric card.")];
        let declared = serves(&[("emit_metric_card", &["documents"])]);
        let with_cards = build_tool_surface(
            &admitted,
            &defs,
            &vak_intent::DomainSet::Empty,
            &declared,
            true,
        );
        assert_eq!(with_cards.core.len(), 1);
        let without_cards = build_tool_surface(
            &admitted,
            &defs,
            &vak_intent::DomainSet::Empty,
            &declared,
            false,
        );
        assert!(without_cards.core.is_empty());
        assert_eq!(without_cards.deferred.len(), 1);
    }

    /// A disabled kernel (`All`) reproduces the pre-kernel surface: every
    /// admitted tool in the stable prefix.
    #[test]
    fn unconstrained_domains_put_every_tool_in_core() {
        let admitted = vec![tool_cap("bash"), tool_cap("webfetch")];
        let defs = vec![
            def("bash", "Run a command."),
            def("webfetch", "Fetch a URL."),
        ];
        let declared = serves(&[("bash", &["code-exec"]), ("webfetch", &["web"])]);
        let surface = build_tool_surface(
            &admitted,
            &defs,
            &vak_intent::DomainSet::All,
            &declared,
            false,
        );
        assert_eq!(surface.core.len(), 2, "{:?}", surface.core);
        assert!(surface.deferred.is_empty());
    }

    /// An uncertain reading arrives as the orientation floor and defers
    /// every declared tool outside it (design 68, Principle 6).
    #[test]
    fn the_orientation_floor_defers_declared_tools() {
        let admitted = vec![tool_cap("bash"), tool_cap("webfetch"), tool_cap("read")];
        let defs = vec![
            def("bash", "Run a command."),
            def("webfetch", "Fetch a URL."),
            def("read", "Read a file."),
        ];
        let declared = serves(&[
            ("bash", &["code-exec"]),
            ("webfetch", &["web"]),
            ("read", &["filesystem"]),
        ]);
        let floor = vak_intent::Engagement::orienting().limits.required_domains;
        let surface = build_tool_surface(&admitted, &defs, &floor, &declared, false);
        let core: Vec<&str> = surface.core.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(core, vec!["read"]);
        assert_eq!(surface.deferred.len(), 2);
    }

    #[test]
    fn matching_domain_tools_join_core() {
        let admitted = vec![tool_cap("webfetch"), tool_cap("bash")];
        let defs = vec![
            def("webfetch", "Fetch a URL."),
            def("bash", "Run a command."),
        ];
        let declared = serves(&[("webfetch", &["web"]), ("bash", &["code-exec"])]);
        let surface = build_tool_surface(&admitted, &defs, &domains(&["web"]), &declared, false);
        assert!(surface.core.iter().any(|d| d.name == "webfetch"));
        assert!(surface.deferred.iter().any(|d| d.name == "bash"));
    }

    #[test]
    fn mcp_servers_never_reach_the_surface_directly() {
        // Even if an MCP server's alias schema is handed in as a tool_def
        // (it should never be, post-refactor), it is dropped unless a
        // Tool-kind capability of the same name was actually admitted.
        let admitted = vec![mcp_cap("tavily")];
        let defs = vec![def("tavily", "Search the web.")];
        let surface = build_tool_surface(
            &admitted,
            &defs,
            &vak_intent::DomainSet::All,
            &BTreeMap::new(),
            false,
        );
        assert!(surface.core.is_empty());
        assert!(surface.deferred.is_empty());
    }

    #[test]
    fn the_index_lists_names_and_first_sentences_with_no_schema() {
        let admitted = vec![tool_cap("bash")];
        let defs = vec![def(
            "bash",
            "Run a shell command. Output is captured and truncated.",
        )];
        let declared = serves(&[("bash", &["code-exec"])]);
        let surface = build_tool_surface(
            &admitted,
            &defs,
            &vak_intent::DomainSet::Empty,
            &declared,
            false,
        );
        assert_eq!(surface.index, "- bash — Run a shell command.");
        assert!(!surface.index.contains("truncated"));
        assert!(!surface.index.contains('{'), "index must carry no schema");
    }
}
