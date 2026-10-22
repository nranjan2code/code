//! Cards a model shows itself: each is a call of the card tool that owns
//! its kind, offered by its own name with its own precise schema
//! (docs/audits/cards-in-answers-2026-10-11.md). The answer lays them out
//! in the order they were shown.

use std::sync::Arc;

use vak_tools::Tool;

/// The card tool of this kind among `tools`, when exactly one owns it. A
/// tool that fills its card from an ability's answer (`Tool::data_of_call`)
/// presents the kind but does not own it: the owner takes its data in its
/// own arguments.
pub(crate) fn owner<'a>(kind: &str, tools: &'a [Arc<dyn Tool>]) -> Option<&'a Arc<dyn Tool>> {
    let mut owners = tools.iter().filter(|tool| {
        tool.presents_cards()
            && tool.data_of_call().is_none()
            && tool.card_kinds().iter().any(|own| own == kind)
    });
    match (owners.next(), owners.next()) {
        (Some(tool), None) => Some(tool),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use vak_tools::ToolOutput;
    use vak_tools::context::ToolContext;

    struct Card(&'static str, &'static [&'static str]);

    #[async_trait]
    impl Tool for Card {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "a card"
        }
        fn schema(&self) -> Value {
            json!({"type": "object"})
        }
        fn presents_cards(&self) -> bool {
            true
        }
        fn card_kinds(&self) -> Vec<String> {
            self.1.iter().map(|kind| (*kind).to_string()).collect()
        }
        async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput::ok("shown")
        }
    }

    fn tools() -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(Card("steps_card", &["steps", "itinerary"])),
            Arc::new(Card("callout_card", &["callout"])),
        ]
    }

    #[test]
    fn a_kind_has_one_owner_or_none() {
        assert_eq!(owner("itinerary", &tools()).unwrap().name(), "steps_card");
        assert!(owner("poem", &tools()).is_none());
    }
}
