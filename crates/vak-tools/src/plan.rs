//! `plan` — the model declares what this answer will deliver, as data
//! (docs/design/86-model-plans-runtime-checks.md §2.1).
//!
//! Like `recall`, it is answered by the agent loop, which holds the turn's
//! state; reaching `execute` is a wiring fault. The runtime checks each
//! declared deliverable against what the turn actually produced; the model
//! never marks one done (invariant 33).

use crate::Fault;
use async_trait::async_trait;
use serde_json::{Value, json};
use vak_intent::{Deliverable, DeliverableForm};

use crate::{Tool, ToolContext, ToolOutput};

/// Reads a `plan` call's arguments. Every entry needs a name and a form;
/// the error names the entry and the field.
pub fn parse_plan_args(args: &Value) -> Result<Vec<Deliverable>, String> {
    let Some(Value::Array(entries)) = args.get("deliverables") else {
        return Err("deliverables must be a list of {name, form}".into());
    };
    if entries.is_empty() {
        return Err("deliverables needs at least one entry".into());
    }
    let mut plan: Vec<Deliverable> = Vec::with_capacity(entries.len());
    for (at, entry) in entries.iter().enumerate() {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("deliverables[{at}].name is missing"))?;
        let form = entry
            .get("form")
            .and_then(Value::as_str)
            .and_then(DeliverableForm::parse)
            .ok_or_else(|| {
                format!(
                    "deliverables[{at}].form must be one of {}",
                    DeliverableForm::ALL.map(DeliverableForm::as_str).join(", ")
                )
            })?;
        if plan
            .iter()
            .any(|known| known.name.eq_ignore_ascii_case(name))
        {
            return Err(format!(
                "deliverables[{at}].name repeats \"{name}\"; give each deliverable once"
            ));
        }
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
            .map(str::to_string);
        let questions = match entry.get("questions") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    item.as_str()
                        .map(str::trim)
                        .filter(|question| !question.is_empty())
                        .map(str::to_string)
                        .ok_or_else(|| {
                            format!(
                                "deliverables[{at}].questions[{index}] must be a non-empty question"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => {
                return Err(format!(
                    "deliverables[{at}].questions must be a list of questions"
                ));
            }
        };
        plan.push(Deliverable {
            questions,
            name: name.to_string(),
            form,
            kind,
        });
    }
    Ok(plan)
}

/// The `plan` tool. `forms` are the card forms an entry's `form` may name
/// besides the five delivery words (docs/design/86 §2.1: a form that names
/// a kind of card is a card of that kind); the host passes its form
/// vocabulary, so a wire that enforces the schema accepts them.
#[derive(Debug, Clone, Default)]
pub struct PlanTool {
    forms: Vec<String>,
}

impl PlanTool {
    pub fn with_forms(forms: &[&str]) -> Self {
        Self {
            forms: forms.iter().map(|form| (*form).to_string()).collect(),
        }
    }
}

#[async_trait]
impl Tool for PlanTool {
    fn name(&self) -> &str {
        "plan"
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn looks_up(&self) -> bool {
        false
    }

    fn planned(&self, args: &Value) -> Option<Vec<Deliverable>> {
        parse_plan_args(args).ok()
    }

    fn description(&self) -> &str {
        "Say what your answer will deliver before you produce it: one entry for each part \
         of the answer, in reading order, named in a few words, with how it reaches the \
         person. Plan the requested outcomes, not optional enrichment or every available card. \
         Vakyartha checks each entry against what you actually produce and tells you if one \
         is missing. For parts that need evidence, declare the questions the sources must \
         settle, including the date or scope that matters. Output receipts do not prove \
         these questions answered. Skip it for a reply that is one direct answer."
    }

    fn schema(&self) -> Value {
        let forms: Vec<&str> = DeliverableForm::ALL
            .map(DeliverableForm::as_str)
            .into_iter()
            .chain(self.forms.iter().map(String::as_str))
            .collect();
        let card_forms = if self.forms.is_empty() {
            String::new()
        } else {
            format!(
                " A card form ({}) when it is shown as cards of that form.",
                self.forms.join(", ")
            )
        };
        json!({
            "type": "object",
            "properties": {
                "deliverables": {
                    "type": "array",
                    "description": "Each part of the answer, in reading order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "The part in a few words, such as the thing the person asked for."
                            },
                            "questions": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Questions this part needs evidence to settle, including material scope/date. Empty for a part needing no external evidence. These are requirements, never self-reported completion."
                            },
                            "form": {
                                "type": "string",
                                "enum": forms,
                                "description": format!("How it reaches the person. card: shown as one or more cards; text: written in your reply; file: a file saved for review; change: a change you make; check: something you run to prove a result.{card_forms}")
                            }
                        },
                        "required": ["name", "form"]
                    }
                }
            },
            "required": ["deliverables"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        match parse_plan_args(args) {
            Ok(_) => ToolOutput::error(
                Fault::Tool,
                r#"{"type":"internal","message":"plan must be answered by the agent loop, not dispatched directly"}"#,
            ),
            Err(message) => ToolOutput::error(
                Fault::Shape,
                format!(r#"{{"type":"invalid_arguments","message":"{message}"}}"#),
            ),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    #[test]
    fn the_plan_tool_declares_its_plan_and_that_it_looks_nothing_up() {
        use crate::Tool;
        let tool = super::PlanTool::default();
        assert!(!tool.looks_up());
        let plan = tool
            .planned(&serde_json::json!({
                "deliverables": [{ "name": "the totals", "form": "file" }]
            }))
            .unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].form, vak_intent::DeliverableForm::File);
        assert!(tool.planned(&serde_json::json!({})).is_none());
    }

    use super::*;

    #[test]
    fn reads_each_deliverable_with_its_form() {
        let plan = parse_plan_args(&json!({"deliverables": [
            {"name": "weekly routine", "form": "card"},
            {"name": "shelf vs meal comparison", "form": "Card"},
            {"name": "short caveat", "form": "text"}
        ]}))
        .unwrap();
        assert_eq!(plan.len(), 3);
        assert_eq!(plan[1].form, DeliverableForm::Card);
        assert_eq!(plan[2].form, DeliverableForm::Text);
    }

    /// A wire that enforces the schema accepts a card form only when the
    /// schema lists it (docs/design/86 §2.1).
    #[test]
    fn the_form_lists_the_card_forms_it_is_given() {
        use crate::Tool;
        let schema = PlanTool::with_forms(&["table", "details"]).schema();
        let forms = &schema["properties"]["deliverables"]["items"]["properties"]["form"]["enum"];
        assert_eq!(
            forms,
            &json!([
                "card", "text", "file", "change", "check", "table", "details"
            ])
        );
        let bare = PlanTool::default().schema();
        assert_eq!(
            bare["properties"]["deliverables"]["items"]["properties"]["form"]["enum"],
            json!(["card", "text", "file", "change", "check"])
        );
    }

    #[test]
    fn evidence_questions_are_declared_not_inferred_from_the_topic() {
        let plan = parse_plan_args(&json!({"deliverables": [{
            "name": "findings", "form": "card",
            "questions": ["Who owns the service as of the requested date?", "Which effects have independent measurements?"]
        }]})).unwrap();
        assert_eq!(plan[0].questions.len(), 2);
        assert!(
            parse_plan_args(
                &json!({"deliverables": [{"name": "x", "form": "text", "questions": [""]}]})
            )
            .is_err()
        );
        assert!(
            parse_plan_args(
                &json!({"deliverables": [{"name": "x", "form": "text", "questions": "done"}]})
            )
            .is_err()
        );
        let old =
            parse_plan_args(&json!({"deliverables": [{"name": "x", "form": "text"}]})).unwrap();
        assert!(old[0].questions.is_empty());
    }

    #[test]
    fn errors_name_the_entry_and_field() {
        let missing = parse_plan_args(&json!({"deliverables": [{"name": "x"}]})).unwrap_err();
        assert!(missing.contains("deliverables[0].form"), "{missing}");
        let empty = parse_plan_args(&json!({"deliverables": []})).unwrap_err();
        assert!(empty.contains("at least one"), "{empty}");
        let twice = parse_plan_args(&json!({"deliverables": [
            {"name": "A", "form": "card"}, {"name": "a", "form": "text"}
        ]}))
        .unwrap_err();
        assert!(twice.contains("repeats"), "{twice}");
    }
}
