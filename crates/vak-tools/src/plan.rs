//! Reading a `plan` call a conversation recorded before 8.28.0, when a
//! model declared its deliverables with a `plan` tool
//! (docs/design/86-model-plans-runtime-checks.md, superseded). No tool by
//! that name is offered any more; a projection of an older answer still
//! lays it out by the plan it recorded.

use serde_json::Value;
use vak_intent::{Deliverable, DeliverableForm};

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

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
