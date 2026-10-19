//! The per-turn tool surface: which admitted tools are *loaded* (full schema
//! in the request) and which are *deferred* (named in the prompt's catalogue,
//! schema loaded once it is called by name). See docs/design/68-context-engine.md §5.
//!
//! This is presentation, never policy. Every tool here was already admitted
//! by [`super::turn`]; the split only decides what the model reads up front,
//! so a deferred tool costs one call by name, never a capability.
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
    /// Schema withheld; listed in the catalogue and loaded once called by
    /// name.
    pub deferred: Vec<ToolDefinition>,
}

/// Split the admitted `tools` for one turn.
///
/// * `progressive` off loads everything.
/// * A tool that declares itself always loaded, a presentation tool (an
///   output shape needs its schema), and a tool that declares no domain (an
///   unknown tool fails open) are loaded.
/// * A card that shows only what a call can have produced
///   (`Tool::grounded_in`) is loaded once `held`, the conversation's facts
///   (`vak_tools::grounds_held`), has its fact; the loop promotes it in
///   place at the step one appears.
/// * `carried` names the tools earlier turns of this session loaded; they
///   stay loaded (cache stability, docs/design/68 §6), so the set only
///   grows, in the tools' fixed order. Facts only accumulate too.
/// * Every other tool is deferred.
pub fn build_tool_surface(
    tools: &[Arc<dyn vak_tools::Tool>],
    progressive: bool,
    carried: &BTreeSet<String>,
    held: &BTreeSet<vak_tools::Grounds>,
) -> ToolSurface {
    let mut surface = ToolSurface::default();
    for tool in tools {
        let definition = ToolDefinition::new(tool.name(), tool.description(), tool.schema());
        let loaded = !progressive
            || carried.contains(tool.name())
            || match tool.grounded_in() {
                Some(grounds) => held.contains(&grounds),
                None => tool.always_loaded() || tool.presents_cards() || tool.serves().is_empty(),
            };
        if loaded {
            surface.core.push(definition);
        } else {
            surface.deferred.push(definition);
        }
    }
    surface
}

/// Drop a card that shows what a tool that answers data answered
/// (`Grounds::Answered`) when no tool of the turn answers data: its fact can
/// never come to be held, so every call of it could only be refused. Read
/// from the tools' own declarations (`Tool::grounded_in`,
/// `Tool::answers_data`), never their names; an ability admitted later
/// brings the card back at the next turn (invariant 31).
pub fn drop_ungroundable(tools: &mut Vec<Arc<dyn vak_tools::Tool>>) {
    let answers = tools.iter().any(|tool| tool.answers_data());
    if !answers {
        tools.retain(|tool| tool.grounded_in() != Some(vak_tools::Grounds::Answered));
    }
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
        .map(|(name, description)| {
            format!("- {name} — {}", vak_llm::types::first_sentence(description))
        })
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    lines.sort();
    format!(
        "\nMore tools (call any of these by its name; its schema is applied for you):\n{}\n",
        lines.join("\n")
    )
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
        grounded: Option<vak_tools::Grounds>,
        answers: bool,
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
        fn grounded_in(&self) -> Option<vak_tools::Grounds> {
            self.grounded
        }
        fn answers_data(&self) -> bool {
            self.answers
        }
    }

    fn tool(name: &'static str, serves: &'static [&'static str]) -> Arc<dyn vak_tools::Tool> {
        Arc::new(Fake {
            name,
            serves,
            always: false,
            card: false,
            grounded: None,
            answers: false,
        })
    }

    fn fixture() -> Vec<Arc<dyn vak_tools::Tool>> {
        vec![
            Arc::new(Fake {
                name: "read",
                serves: &["filesystem"],
                always: true,
                card: false,
                grounded: None,
                answers: false,
            }),
            tool("bash", &["code-exec"]),
            tool("webfetch", &["web", "live-data"]),
            tool("plugin_thing", &[]),
            Arc::new(Fake {
                name: "emit_chart_card",
                serves: &[],
                always: false,
                card: true,
                grounded: None,
                answers: false,
            }),
        ]
    }

    #[test]
    fn a_card_of_answered_data_needs_a_tool_that_answers() {
        let answered = |name| -> Arc<dyn vak_tools::Tool> {
            Arc::new(Fake {
                name,
                serves: &[],
                always: false,
                card: true,
                grounded: Some(vak_tools::Grounds::Answered),
                answers: false,
            })
        };
        let mut tools = fixture();
        tools.push(answered("emit_answered_card"));
        drop_ungroundable(&mut tools);
        assert!(tools.iter().all(|tool| tool.name() != "emit_answered_card"));
        assert!(tools.iter().any(|tool| tool.name() == "emit_chart_card"));

        let mut tools = fixture();
        tools.push(answered("emit_answered_card"));
        tools.push(Arc::new(Fake {
            name: "an_ability",
            serves: &["documents"],
            always: false,
            card: false,
            grounded: None,
            answers: true,
        }));
        drop_ungroundable(&mut tools);
        assert!(tools.iter().any(|tool| tool.name() == "emit_answered_card"));
    }

    fn names(defs: &[ToolDefinition]) -> Vec<&str> {
        defs.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn without_progressive_disclosure_everything_is_loaded() {
        let surface = build_tool_surface(&fixture(), false, &BTreeSet::new(), &BTreeSet::new());
        assert_eq!(surface.core.len(), 5);
        assert!(surface.deferred.is_empty());
    }

    #[test]
    fn a_tool_s_own_declaration_decides_what_is_loaded() {
        let surface = build_tool_surface(&fixture(), true, &BTreeSet::new(), &BTreeSet::new());
        assert_eq!(
            names(&surface.core),
            vec!["read", "plugin_thing", "emit_chart_card"]
        );
        assert_eq!(names(&surface.deferred), vec!["bash", "webfetch"]);
    }

    #[test]
    fn a_tool_an_earlier_turn_loaded_stays_loaded() {
        let carried = BTreeSet::from(["bash".to_string(), "emit_absent_card".to_string()]);
        let surface = build_tool_surface(&fixture(), true, &carried, &BTreeSet::new());
        assert_eq!(
            names(&surface.core),
            vec!["read", "bash", "plugin_thing", "emit_chart_card"],
            "fixed order, one more loaded, nothing unadmitted"
        );
    }

    /// The catalogue is a function of the admitted tools only, so the cached
    /// prefix does not move from turn to turn.
    #[test]
    fn a_card_that_shows_what_a_call_made_loads_once_the_conversation_has_one() {
        let mut tools = fixture();
        tools.push(Arc::new(Fake {
            name: "emit_terminal_card",
            serves: &[],
            always: false,
            card: true,
            grounded: Some(vak_tools::Grounds::Ran),
            answers: false,
        }));
        let fresh = build_tool_surface(&tools, true, &BTreeSet::new(), &BTreeSet::new());
        assert!(names(&fresh.deferred).contains(&"emit_terminal_card"));
        assert!(
            names(&fresh.core).contains(&"emit_chart_card"),
            "{:?}",
            names(&fresh.core)
        );
        let ran = BTreeSet::from([vak_tools::Grounds::Ran]);
        let after = build_tool_surface(&tools, true, &BTreeSet::new(), &ran);
        assert!(names(&after.core).contains(&"emit_terminal_card"));
        let wrote = BTreeSet::from([vak_tools::Grounds::Wrote]);
        let other = build_tool_surface(&tools, true, &BTreeSet::new(), &wrote);
        assert!(names(&other.deferred).contains(&"emit_terminal_card"));
    }

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
