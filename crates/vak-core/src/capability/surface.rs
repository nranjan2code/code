//! The per-turn tool surface: which admitted tools are *loaded* (full schema
//! in the request) and which are *deferred* (named in the prompt's catalogue,
//! schema one `find_tools` call away). See docs/design/68-context-engine.md §5.
//!
//! This is presentation, never policy. Every tool here was already admitted
//! by [`super::turn`]; the split only decides what the model reads up front,
//! so a deferred tool costs one `find_tools` round trip, never a capability.
//!
//! The split reads each tool's own declaration (`Tool::always_loaded`,
//! `Tool::serves`, `Tool::presents_cards`), never a reading of the request
//! (docs/design/88-control-plane.md §12.1); the harness keeps no table of
//! tool names.

use std::collections::BTreeSet;
use std::sync::Arc;

use vak_llm::ToolDefinition;

/// One turn's tools, split for the request.
#[derive(Debug, Clone, Default)]
pub struct ToolSurface {
    /// Sent with full schemas.
    pub core: Vec<ToolDefinition>,
    /// Schema withheld; reachable through `find_tools`, or Anthropic
    /// `defer_loading` on legs that support it.
    pub deferred: Vec<ToolDefinition>,
}

/// Split the admitted `tools` for one turn.
///
/// * `progressive` off loads everything.
/// * A tool that declares itself always loaded, a presentation tool (an
///   output shape needs its schema), and a tool that declares no domain (an
///   unknown tool fails open) are loaded.
/// * `carried` names the tools earlier turns of this session loaded; they
///   stay loaded (cache stability, docs/design/68 §6), so the set only
///   grows, in the tools' fixed order.
/// * Every other tool is deferred.
pub fn build_tool_surface(
    tools: &[Arc<dyn vak_tools::Tool>],
    progressive: bool,
    carried: &BTreeSet<String>,
) -> ToolSurface {
    let mut surface = ToolSurface::default();
    for tool in tools {
        let definition = ToolDefinition::new(tool.name(), tool.description(), tool.schema());
        let loaded = !progressive
            || tool.always_loaded()
            || tool.presents_cards()
            || tool.serves().is_empty()
            || carried.contains(tool.name());
        if loaded {
            surface.core.push(definition);
        } else {
            surface.deferred.push(definition);
        }
    }
    surface
}

/// The prompt's catalogue of admitted tools that may defer: one line each,
/// name and first sentence, no schema. Presentation tools are already loaded
/// and callers exclude them. The catalogue depends only on what is admitted
/// — never on the turn's reading — so it
/// stays byte-stable in the cached prefix while the loaded set moves. Empty
/// when nothing can defer.
pub fn tool_catalogue<'a>(entries: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut lines: Vec<String> = entries
        .into_iter()
        .map(|(name, description)| format!("- {name} — {}", first_sentence(description)))
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    lines.sort();
    format!(
        "\nMore tools (a tool not in your schemas this turn is loaded by calling `find_tools` with its name or purpose; call it before using one):\n{}\n",
        lines.join("\n")
    )
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
    use serde_json::Value;

    struct Fake {
        name: &'static str,
        serves: &'static [&'static str],
        always: bool,
        card: bool,
    }

    #[async_trait::async_trait]
    impl vak_tools::Tool for Fake {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "Does a thing. Then more detail nobody needs up front."
        }
        fn schema(&self) -> Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _: &Value, _: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
            vak_tools::ToolOutput::ok("")
        }
        fn serves(&self) -> Vec<&str> {
            self.serves.to_vec()
        }
        fn always_loaded(&self) -> bool {
            self.always
        }
        fn presents_cards(&self) -> bool {
            self.card
        }
    }

    fn tool(name: &'static str, serves: &'static [&'static str]) -> Arc<dyn vak_tools::Tool> {
        Arc::new(Fake {
            name,
            serves,
            always: false,
            card: false,
        })
    }

    fn fixture() -> Vec<Arc<dyn vak_tools::Tool>> {
        vec![
            Arc::new(Fake {
                name: "read",
                serves: &["filesystem"],
                always: true,
                card: false,
            }),
            tool("bash", &["code-exec"]),
            tool("webfetch", &["web", "live-data"]),
            tool("plugin_thing", &[]),
            Arc::new(Fake {
                name: "emit_chart_card",
                serves: &[],
                always: false,
                card: true,
            }),
        ]
    }

    fn names(defs: &[ToolDefinition]) -> Vec<&str> {
        defs.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn without_progressive_disclosure_everything_is_loaded() {
        let surface = build_tool_surface(&fixture(), false, &BTreeSet::new());
        assert_eq!(surface.core.len(), 5);
        assert!(surface.deferred.is_empty());
    }

    #[test]
    fn a_tool_s_own_declaration_decides_what_is_loaded() {
        let surface = build_tool_surface(&fixture(), true, &BTreeSet::new());
        assert_eq!(
            names(&surface.core),
            vec!["read", "plugin_thing", "emit_chart_card"]
        );
        assert_eq!(names(&surface.deferred), vec!["bash", "webfetch"]);
    }

    #[test]
    fn a_tool_an_earlier_turn_loaded_stays_loaded() {
        let carried = BTreeSet::from(["bash".to_string(), "emit_absent_card".to_string()]);
        let surface = build_tool_surface(&fixture(), true, &carried);
        assert_eq!(
            names(&surface.core),
            vec!["read", "bash", "plugin_thing", "emit_chart_card"],
            "fixed order, one more loaded, nothing unadmitted"
        );
    }

    /// The catalogue is a function of the admitted tools only, so the cached
    /// prefix does not move from turn to turn.
    #[test]
    fn the_catalogue_is_stable_sorted_and_schema_free() {
        let entries = vec![
            (
                "webfetch",
                "Fetch a URL. Long detail nobody needs up front.",
            ),
            ("bash", "Run a command."),
        ];
        let catalogue = tool_catalogue(entries.clone());
        let mut reversed = entries;
        reversed.reverse();
        assert_eq!(catalogue, tool_catalogue(reversed));
        assert!(catalogue.contains("- bash — Run a command.\n- webfetch — Fetch a URL.\n"));
        assert!(!catalogue.contains("nobody needs"));
        assert!(tool_catalogue(Vec::new()).is_empty());
    }
}
