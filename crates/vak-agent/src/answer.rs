//! The answer document (docs/design/87-composed-answers.md §2,
//! docs/design/88-control-plane.md §4): the model writes its whole answer
//! as one `answer` call whose blocks are the cards this turn can show and
//! its prose. The tool is the loop's own and is made from whatever card
//! tools the turn admits, so nothing here names a block: a block's kind is
//! a `semantic_type` some admitted card tool declares, its fields are that
//! tool's arguments, and that tool validates it.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vak_tools::Tool;

/// The tool's name.
pub(crate) const ANSWER: &str = "answer";
/// The block kind that is prose rather than a card.
const TEXT: &str = "text";

fn cards(tools: &[Arc<dyn Tool>]) -> impl Iterator<Item = &Arc<dyn Tool>> {
    tools.iter().filter(|tool| tool.presents_cards())
}

/// Replaces the card tools in a turn's definitions with the one `answer`
/// tool, in the place the first of them held. A turn that admits no card
/// tool is left as it is.
pub(crate) fn offer(definitions: &mut Vec<vak_llm::ToolDefinition>, tools: &[Arc<dyn Tool>]) {
    let is_card = |name: &str| cards(tools).any(|tool| tool.name() == name);
    let Some(at) = definitions
        .iter()
        .position(|definition| is_card(&definition.name))
    else {
        return;
    };
    definitions.retain(|definition| !is_card(&definition.name));
    if definitions
        .iter()
        .all(|definition| definition.name != ANSWER)
    {
        definitions.insert(at.min(definitions.len()), definition(tools));
    }
}

/// The `answer` tool for the card tools admitted. Each block's fields are
/// the union of those tools' own arguments; the owning tool's schema is
/// what a block is validated against, so a field two tools define
/// differently is described for both and typed for neither.
fn definition(tools: &[Arc<dyn Tool>]) -> vak_llm::ToolDefinition {
    let mut kinds_listed = Vec::new();
    let mut every_kind = Vec::new();
    let mut fields: Map<String, Value> = Map::new();
    fields.insert(
        TEXT.into(),
        json!({"type": "string", "description": "For a `text` block: the prose, in Markdown."}),
    );
    for tool in cards(tools) {
        let schema = tool.schema();
        let own = tool.card_kinds();
        if own.is_empty() {
            continue;
        }
        let required: Vec<&str> = schema["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|field| *field != "semantic_type")
            .collect();
        kinds_listed.push(format!(
            "{}: {} Needs: {}.",
            own.iter()
                .map(|kind| format!("`{kind}`"))
                .collect::<Vec<_>>()
                .join(", "),
            tool.description(),
            if required.is_empty() {
                "nothing more".to_string()
            } else {
                required.join(", ")
            }
        ));
        every_kind.extend(own.iter().map(|kind| Value::String(kind.clone())));
        let Some(properties) = schema["properties"].as_object() else {
            continue;
        };
        for (name, property) in properties {
            if name == "semantic_type" {
                continue;
            }
            match fields.get_mut(name) {
                None => {
                    fields.insert(name.clone(), property.clone());
                }
                Some(known) if known == property => {}
                Some(known) => {
                    let both = format!(
                        "{} Or, in another kind of block: {}",
                        known["description"].as_str().unwrap_or_default(),
                        property["description"].as_str().unwrap_or_default()
                    );
                    *known = json!({ "description": both });
                }
            }
        }
    }
    kinds_listed.push(format!(
        "`{TEXT}`: prose, in the `text` field, only for a part none of the kinds above fits."
    ));
    every_kind.push(Value::String(TEXT.into()));
    let mut properties = Map::new();
    properties.insert(
        "type".into(),
        json!({
            "type": "string",
            "enum": every_kind,
            "description": "The kind of block. It decides which other fields the block takes."
        }),
    );
    properties.extend(fields);
    vak_llm::ToolDefinition::new(
        ANSWER,
        format!(
            "Give your whole answer to the person, once, as an ordered list of blocks. \
             Calling this ends your turn: put everything the answer needs in this one call \
             and write nothing after it. A part that is steps, figures, a comparison, a table \
             or any other kind below is a block of that kind, never `text`. Leave out the \
             fields a block's kind does not take. The kinds of block:\n- {}",
            kinds_listed.join("\n- ")
        ),
        json!({
            "type": "object",
            "properties": {
                "blocks": {
                    "type": "array",
                    "description": "The answer's blocks, in the order the person should read them.",
                    "items": {
                        "type": "object",
                        "properties": properties,
                        "required": ["type"]
                    }
                }
            },
            "required": ["blocks"]
        }),
    )
}

/// One block of an answer, read and ready to be checked by its tool.
pub(crate) enum Block {
    Text {
        text: String,
        /// The plan deliverable the block says it delivers.
        answers: Option<String>,
    },
    Card {
        tool: Arc<dyn Tool>,
        /// The owning tool's arguments, `semantic_type` included.
        input: Value,
    },
}

/// Reads an `answer` call's blocks. Each error names its block.
pub(crate) fn read(input: &Value, tools: &[Arc<dyn Tool>]) -> Result<Vec<Block>, Vec<String>> {
    let Some(blocks) = input.get("blocks").and_then(Value::as_array) else {
        return Err(vec!["blocks must be a list of blocks".into()]);
    };
    if blocks.is_empty() {
        return Err(vec!["blocks needs at least one block".into()]);
    }
    let mut read = Vec::with_capacity(blocks.len());
    let mut errors = Vec::new();
    for (at, block) in blocks.iter().enumerate() {
        let Some(fields) = block.as_object() else {
            errors.push(format!("blocks[{at}] must be an object with a `type`"));
            continue;
        };
        let Some(kind) = fields.get("type").and_then(Value::as_str).map(str::trim) else {
            errors.push(format!("blocks[{at}].type is missing"));
            continue;
        };
        if kind == TEXT {
            match fields.get(TEXT).and_then(Value::as_str).map(str::trim) {
                Some(text) if !text.is_empty() => read.push(Block::Text {
                    text: text.to_string(),
                    answers: fields
                        .get("answers")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string),
                }),
                _ => errors.push(format!("blocks[{at}].text is missing")),
            }
            continue;
        }
        let mut owners =
            cards(tools).filter(|tool| tool.card_kinds().iter().any(|own| own == kind));
        let (Some(tool), None) = (owners.next(), owners.next()) else {
            errors.push(format!(
                "blocks[{at}].type `{kind}` is not a kind of block this turn offers"
            ));
            continue;
        };
        let mut input = Map::new();
        input.insert("semantic_type".into(), Value::String(kind.into()));
        input.extend(
            fields
                .iter()
                // A wire that makes every field required gets the ones a
                // kind does not take back empty: an empty value is absent.
                .filter(|(name, value)| {
                    name.as_str() != "type"
                        && !value.is_null()
                        && value.as_str() != Some("")
                        && value.as_array().is_none_or(|items| !items.is_empty())
                })
                .map(|(name, value)| (name.clone(), value.clone())),
        );
        let mut input = Value::Object(input);
        let schema = tool.schema();
        tool.fill_defaults(&mut input);
        vak_tools::read_arguments(&schema, &mut input);
        // A block carries only the fields its own kind takes; the union
        // schema the model wrote against may have led it to add another's.
        if let (Some(fields), Some(own)) = (input.as_object_mut(), schema["properties"].as_object())
        {
            fields.retain(|name, _| name == "semantic_type" || own.contains_key(name));
        }
        match vak_tools::validate_input(&schema, &input) {
            Ok(()) => read.push(Block::Card {
                tool: tool.clone(),
                input,
            }),
            Err(message) => errors.push(format!("blocks[{at}] ({kind}): {message}")),
        }
    }
    if errors.is_empty() {
        Ok(read)
    } else {
        Err(errors)
    }
}

/// What the model is told when an answer's blocks did not all stand.
pub(crate) fn refused(errors: &[String]) -> String {
    format!(
        "Nothing was shown: every block must be valid for the answer to be given. Fix these and \
         call `{ANSWER}` again with all of its blocks:\n- {}",
        errors.join("\n- ")
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use vak_tools::{ToolContext, ToolOutput};

    struct Card {
        name: &'static str,
        schema: Value,
    }

    #[async_trait]
    impl Tool for Card {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "A block for tests."
        }
        fn schema(&self) -> Value {
            self.schema.clone()
        }
        fn presents_cards(&self) -> bool {
            true
        }
        async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
            ToolOutput::ok("shown")
        }
    }

    fn tools() -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(Card {
                name: "emit_steps_card",
                schema: json!({"type": "object", "properties": {
                    "semantic_type": {"type": "string", "enum": ["steps"]},
                    "label": {"type": "string", "description": "A heading."},
                    "steps": {"type": "array", "items": {"type": "string"}, "description": "The steps."}
                }, "required": ["semantic_type", "steps"]}),
            }),
            Arc::new(Card {
                name: "emit_callout_card",
                schema: json!({"type": "object", "properties": {
                    "semantic_type": {"type": "string", "enum": ["callout"]},
                    "label": {"type": "string", "description": "A heading."},
                    "text": {"type": "string", "description": "The note."}
                }, "required": ["semantic_type", "text"]}),
            }),
        ]
    }

    #[test]
    fn the_card_tools_are_replaced_by_one_answer_tool_in_their_place() {
        let tools = tools();
        let mut definitions = vec![
            vak_llm::ToolDefinition::new("read", "", json!({})),
            vak_llm::ToolDefinition::new("emit_steps_card", "", json!({})),
            vak_llm::ToolDefinition::new("grep", "", json!({})),
            vak_llm::ToolDefinition::new("emit_callout_card", "", json!({})),
        ];
        offer(&mut definitions, &tools);
        let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["read", "answer", "grep"]);
        let block = &definitions[1].parameters["properties"]["blocks"]["items"];
        assert_eq!(
            block["properties"]["type"]["enum"],
            json!(["steps", "callout", "text"])
        );
        assert!(block["properties"]["steps"].is_object());
        assert!(
            definitions[1].description.contains("`steps`")
                && definitions[1].description.contains("Needs: steps")
        );

        let mut plain = vec![vak_llm::ToolDefinition::new("read", "", json!({}))];
        offer(&mut plain, &[]);
        assert_eq!(plain.len(), 1);
    }

    #[test]
    fn each_block_is_read_by_the_tool_that_owns_its_kind() {
        let read = read(
            &json!({"blocks": [
                {"type": "steps", "label": "Week", "steps": ["Water", "Feed"], "text": null},
                {"type": "text", "text": "That is the routine."},
                {"type": "callout", "text": "Do not overwater.", "steps": ["stray"]},
            ]}),
            &tools(),
        )
        .ok()
        .unwrap();
        let shape: Vec<String> = read
            .iter()
            .map(|block| match block {
                Block::Text { text, .. } => format!("text:{text}"),
                Block::Card { tool, input } => format!("{}:{input}", tool.name()),
            })
            .collect();
        assert_eq!(
            shape,
            [
                r#"emit_steps_card:{"label":"Week","semantic_type":"steps","steps":["Water","Feed"]}"#,
                "text:That is the routine.",
                r#"emit_callout_card:{"semantic_type":"callout","text":"Do not overwater."}"#,
            ]
        );
    }

    #[test]
    fn every_fault_names_its_block() {
        let errors = read(
            &json!({"blocks": [
                {"type": "steps", "label": "No steps"},
                {"type": "weather"},
                {"type": "text"},
                "words",
            ]}),
            &tools(),
        )
        .err()
        .unwrap();
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert!(errors[0].starts_with("blocks[0] (steps):"), "{errors:?}");
        assert!(errors[1].contains("`weather` is not a kind"), "{errors:?}");
        assert!(errors[2].contains("blocks[2].text"), "{errors:?}");
        assert!(refused(&errors).contains("call `answer` again"));
        assert!(read(&json!({"blocks": "x"}), &tools()).is_err());
    }
}
