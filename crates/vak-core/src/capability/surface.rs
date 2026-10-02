//! The per-turn tool surface: which admitted tools are *loaded* (full schema
//! in the request) and which are *deferred* (named in the prompt's catalogue,
//! schema one `find_tools` call away). See docs/design/68-context-engine.md §5.
//!
//! This is presentation, never policy. Every tool here was already admitted
//! by [`super::turn`]; the split only decides what the model reads up front.
//! A wrong prediction therefore costs one `find_tools` round trip and is the
//! measured misread (`crate::misread`), never a missing capability.
//!
//! The split reads each tool's own declaration (`Tool::always_loaded`,
//! `Tool::serves`, `Tool::presents_cards`) — the harness keeps no table of
//! tool names.

use std::collections::BTreeSet;
use std::sync::Arc;

use vak_llm::ToolDefinition;

use super::domain::{Domain, Serves};

/// One turn's tools, split for the request.
#[derive(Debug, Clone, Default)]
pub struct ToolSurface {
    /// Sent with full schemas.
    pub core: Vec<ToolDefinition>,
    /// Schema withheld; reachable through `find_tools`, or Anthropic
    /// `defer_loading` on legs that support it.
    pub deferred: Vec<ToolDefinition>,
    /// Deferred because the turn's reading did not predict them — the set a
    /// later call measures a misread against. Card tools are excluded: a
    /// card is an output choice, not a reading of what the request needs.
    pub unpredicted: BTreeSet<String>,
}

/// Split the admitted `tools` for one turn.
///
/// * `required_domains` is the reading's prediction. `All` (progressive
///   disclosure off, or the kernel disabled) loads everything.
/// * `predicted_cards` names the card tools the request itself reads as
///   (`presentation_tools::predicted_card_tools`); other card tools defer.
/// * `carried` names the tools earlier turns of this session loaded; they
///   stay loaded (cache stability, docs/design/68 §6).
///
/// Always-loaded and undeclared tools are loaded whatever the prediction:
/// the first because the model cannot operate without them, the second
/// because deferring is a context saving and an unknown tool fails open.
pub fn build_tool_surface(
    tools: &[Arc<dyn vak_tools::Tool>],
    required_domains: &vak_intent::DomainSet,
    predicted_cards: &BTreeSet<String>,
    carried: &BTreeSet<String>,
) -> ToolSurface {
    let required: Option<BTreeSet<Domain>> = (!required_domains.is_unconstrained())
        .then(|| required_domains.iter().map(|d| Domain::parse(d)).collect());
    let mut surface = ToolSurface::default();
    for tool in tools {
        let definition = ToolDefinition::new(tool.name(), tool.description(), tool.schema());
        // A tool this session already had loaded stays loaded: providers cache
        // the tools array ahead of everything else, so a set that follows each
        // turn's reading rewrites the cache from the first byte every turn.
        // The set only grows, in the tools' fixed order.
        let loaded = match &required {
            None => true,
            Some(_) if carried.contains(tool.name()) => true,
            Some(_) if tool.always_loaded() => true,
            Some(_) if tool.presents_cards() => predicted_cards.contains(tool.name()),
            Some(required) => Serves::from_labels(tool.serves()).serves_any(required),
        };
        if loaded {
            surface.core.push(definition);
        } else {
            if !tool.presents_cards() {
                surface.unpredicted.insert(tool.name().to_string());
            }
            surface.deferred.push(definition);
        }
    }
    surface
}

/// The prompt's catalogue of the admitted tools that are not always loaded:
/// one line each, name and first sentence, no schema. `entries` are
/// `(name, description)` pairs. It depends only on what is admitted — never
/// on the turn's reading — so it stays byte-stable in the cached prefix while
/// the loaded set moves. Empty when nothing can defer.
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
        fn serves(&self) -> &'static [&'static str] {
            self.serves
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
    fn unconstrained_domains_load_everything() {
        let surface = build_tool_surface(
            &fixture(),
            &vak_intent::DomainSet::All,
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        assert_eq!(surface.core.len(), 5);
        assert!(surface.deferred.is_empty());
        assert!(surface.unpredicted.is_empty());
    }

    #[test]
    fn a_reading_loads_what_it_predicts_and_defers_the_rest() {
        let surface = build_tool_surface(
            &fixture(),
            &vak_intent::DomainSet::only(["web"]),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        assert_eq!(
            names(&surface.core),
            vec!["read", "webfetch", "plugin_thing"]
        );
        assert_eq!(names(&surface.deferred), vec!["bash", "emit_chart_card"]);
        assert_eq!(
            surface.unpredicted,
            BTreeSet::from(["bash".to_string()]),
            "a deferred card is an output choice, not a misread"
        );
    }

    #[test]
    fn a_predicted_card_tool_is_loaded() {
        let surface = build_tool_surface(
            &fixture(),
            &vak_intent::DomainSet::Empty,
            &BTreeSet::from(["emit_chart_card".to_string()]),
            &BTreeSet::new(),
        );
        assert!(names(&surface.core).contains(&"emit_chart_card"));
    }

    #[test]
    fn a_tool_an_earlier_turn_loaded_stays_loaded() {
        let carried = BTreeSet::from(["bash".to_string()]);
        let surface = build_tool_surface(
            &fixture(),
            &vak_intent::DomainSet::only(["web"]),
            &BTreeSet::new(),
            &carried,
        );
        assert_eq!(
            names(&surface.core),
            vec!["read", "bash", "webfetch", "plugin_thing"],
            "fixed order, one more loaded"
        );
        assert!(!surface.unpredicted.contains("bash"));
    }

    /// The catalogue is a function of the admitted tools only, so the cached
    /// prefix does not move when the reading does.
    #[test]
    fn the_catalogue_is_stable_sorted_and_schema_free() {
        let entries = [
            (
                "webfetch",
                "Fetch a URL. Long detail nobody needs up front.",
            ),
            ("bash", "Run a command."),
        ];
        let catalogue = tool_catalogue(entries);
        let mut reversed = entries;
        reversed.reverse();
        assert_eq!(catalogue, tool_catalogue(reversed));
        assert!(catalogue.contains("- bash — Run a command.\n- webfetch — Fetch a URL.\n"));
        assert!(!catalogue.contains("nobody needs"));
        assert!(tool_catalogue([]).is_empty());
    }
}
