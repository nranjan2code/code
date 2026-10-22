//! Cards in an answer (docs/design/87-composed-answers.md §2). Each card
//! is a call of the card tool that owns its kind, offered by its own name
//! with its own precise schema: one nested call of every block was written
//! as broken JSON far more often on the same requests
//! (docs/audits/cards-in-answers-2026-10-11.md). The runtime composes the
//! answer from the turn's cards, each placed by the plan deliverable its
//! `deliverable` names.

use std::sync::Arc;

use serde_json::Value;
use vak_tools::Tool;

/// The presentation receipt and its accepted replacement address. The loop
/// keeps this small projection as cards arrive; checking a plan never replays
/// the ledger or counts a retired form.
pub(crate) struct Accepted {
    pub id: String,
    pub replaces: Option<String>,
    pub receipt: vak_judge::plan::CardReceipt,
}

#[derive(Default)]
pub(crate) struct Receipts(std::collections::BTreeMap<String, vak_judge::plan::CardReceipt>);

impl Receipts {
    pub fn accept(&mut self, card: Accepted) {
        if let Some(replaced) = card.replaces {
            self.0.remove(&replaced);
        }
        self.0.insert(card.id, card.receipt);
    }

    pub fn active(&self) -> Vec<vak_judge::plan::CardReceipt> {
        self.0.values().cloned().collect()
    }

    /// Delegation can deduplicate to a parent's existing presentation id.
    /// Seeing it again must not erase that parent's accepted attribution.
    pub fn observe_unattributed(&mut self, id: String) {
        self.0.entry(id).or_default();
    }
}

/// A `plan` deliverable whose form names a kind of card is a card of that
/// kind: the kind is kept so a step can be required to call the tool that
/// owns it. Read from the admitted tools' own kinds, at the call boundary,
/// before the plan is parsed and ledgered.
pub(crate) fn plan_forms(
    mut call: crate::PendingToolCall,
    tools: &[Arc<dyn Tool>],
) -> crate::PendingToolCall {
    if call.name != "plan" {
        return call;
    }
    let is_kind = |form: &str| {
        tools
            .iter()
            .filter(|tool| tool.presents_cards())
            .any(|tool| tool.card_kinds().iter().any(|kind| kind == form))
    };
    if let Some(deliverables) = call
        .input
        .get_mut("deliverables")
        .and_then(Value::as_array_mut)
    {
        for deliverable in deliverables.iter_mut().filter_map(Value::as_object_mut) {
            let Some(form) = deliverable
                .get("form")
                .and_then(Value::as_str)
                .map(str::trim)
                .map(str::to_string)
            else {
                continue;
            };
            if is_kind(&form) {
                deliverable.insert("form".into(), Value::String("card".into()));
                deliverable.insert("kind".into(), Value::String(form));
            } else if vak_delivery::is_form(&form) {
                // A form the plan's schema offers whose tool this turn
                // withdrew: still a card, with no tool a step can be
                // required to call.
                deliverable.insert("form".into(), Value::String("card".into()));
            }
        }
    }
    call
}

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
    use serde_json::json;
    use vak_tools::ToolOutput;
    use vak_tools::context::ToolContext;

    #[test]
    fn replacement_removes_only_the_addressed_plan_receipt() {
        let mut receipts = Receipts::default();
        let accepted = |id: &str, replaces: Option<&str>, form: &str| Accepted {
            id: id.into(),
            replaces: replaces.map(str::to_string),
            receipt: vak_judge::plan::CardReceipt {
                deliverable: Some(id.into()),
                semantic_type: form.into(),
                block: None,
            },
        };
        receipts.accept(accepted("chart", None, "chart"));
        receipts.observe_unattributed("chart".into());
        assert_eq!(receipts.active()[0].semantic_type, "chart");
        receipts.accept(accepted("other", None, "table"));
        receipts.accept(accepted("replacement", Some("chart"), "sources"));
        let active = receipts.active();
        assert_eq!(active.len(), 2);
        assert!(
            !active
                .iter()
                .any(|receipt| receipt.semantic_type == "chart")
        );
        assert!(
            active
                .iter()
                .any(|receipt| receipt.semantic_type == "table")
        );
        assert!(
            active
                .iter()
                .any(|receipt| receipt.semantic_type == "sources")
        );
    }

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
    fn a_plan_form_that_names_a_kind_is_a_card_of_that_kind() {
        let call = plan_forms(
            crate::PendingToolCall {
                id: "p".into(),
                name: "plan".into(),
                input: json!({"deliverables": [
                    {"name": "Trip", "form": "itinerary"},
                    {"name": "Why", "form": "text"},
                    {"name": "Odd", "form": "poem"}
                ]}),
            },
            &tools(),
        );
        let deliverables = call.input["deliverables"].as_array().unwrap();
        assert_eq!(deliverables[0]["form"], "card");
        assert_eq!(deliverables[0]["kind"], "itinerary");
        assert_eq!(deliverables[1]["form"], "text");
        assert!(deliverables[1].get("kind").is_none());
        assert_eq!(
            deliverables[2]["form"], "poem",
            "left for the plan's own refusal"
        );
        assert_eq!(owner("itinerary", &tools()).unwrap().name(), "steps_card");
        assert!(owner("poem", &tools()).is_none());
    }

    /// The plan's schema offers every form; one whose tool this turn does
    /// not have is still a card, with no tool a step could be required to call.
    #[test]
    fn a_form_with_no_tool_this_turn_is_a_card_without_a_kind() {
        let call = plan_forms(
            crate::PendingToolCall {
                id: "p".into(),
                name: "plan".into(),
                input: json!({"deliverables": [{"name": "Figures", "form": "stats"}]}),
            },
            &tools(),
        );
        let deliverable = &call.input["deliverables"][0];
        assert_eq!(deliverable["form"], "card");
        assert!(deliverable.get("kind").is_none());
    }
}
