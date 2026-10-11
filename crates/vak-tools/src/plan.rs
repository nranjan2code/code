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
        plan.push(Deliverable {
            name: name.to_string(),
            form,
            with: entry
                .get("with")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|tool| !tool.is_empty())
                .map(str::to_string),
        });
    }
    Ok(plan)
}

pub struct PlanTool;

#[async_trait]
impl Tool for PlanTool {
    fn name(&self) -> &str {
        "plan"
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Say what your answer will deliver before you produce it: one entry for each \
         distinct thing the person asked for, in their order, named in a few words, with \
         how it reaches them. Vakyartha checks each entry against what you actually produce \
         and tells you if one is missing. Skip it for a reply that is one direct answer."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "deliverables": {
                    "type": "array",
                    "description": "Each distinct thing the person asked for, in their order.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {
                                "type": "string",
                                "description": "The deliverable in a few words, such as the thing the person asked for."
                            },
                            "form": {
                                "type": "string",
                                "enum": DeliverableForm::ALL.map(DeliverableForm::as_str),
                                "description": "card: shown with a card tool; text: written in your reply; file: a file saved for review; change: a change you make; check: something you run to prove a result."
                            },
                            "with": {
                                "type": "string",
                                "description": "For a card: the name of the card tool you will call to show it."
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
