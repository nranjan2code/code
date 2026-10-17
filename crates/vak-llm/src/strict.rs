//! Strict tool schemas (docs/design/89-constrained-tool-arguments.md §4.2).
//!
//! A wire that enforces a tool's schema while the model writes its
//! arguments takes the schema in its own strict form. Vak keeps one
//! canonical schema per tool; this renders the form a dialect enforces, and
//! chooses which tools go strict within the limits the provider publishes.
//! A schema with no strict form (an object whose properties are not
//! declared) is sent as it is: the tool's own validation still reads it.

use serde_json::{Map, Value};

use crate::types::ToolDefinition;

/// The strict forms the providers document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// OpenAI Chat Completions and Responses: every property is required,
    /// an optional one takes `null`, and no object takes other properties.
    OpenAi,
    /// Anthropic (and Anthropic models elsewhere): no object takes other
    /// properties, and the published limits on strict tools apply.
    Anthropic,
}

/// Anthropic's published limits per request (Structured outputs, "Size and
/// complexity limits"): strict tools, optional parameters across them, and
/// parameters written as a union.
const ANTHROPIC_STRICT_TOOLS: usize = 20;
const ANTHROPIC_OPTIONAL: usize = 24;
const ANTHROPIC_UNIONS: usize = 16;

/// Keywords a strict schema does not take: bounds the grammar cannot
/// enforce (Anthropic refuses them). Dropping one widens what the wire
/// enforces; the tool's own validation still checks the canonical schema.
const UNENFORCED: [&str; 8] = [
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "maxItems",
];

/// Annotations a strict schema does not take; they constrain nothing.
const ANNOTATIONS: [&str; 3] = ["examples", "default", "title"];

/// Keywords no strict grammar renders: a schema that uses one has no strict
/// form, rather than one that silently means something else.
const UNRENDERED: [&str; 11] = [
    "not",
    "patternProperties",
    "propertyNames",
    "if",
    "then",
    "else",
    "dependentRequired",
    "dependentSchemas",
    "prefixItems",
    "contains",
    "uniqueItems",
];

/// The strict form of `schema`, or `None` when it has none.
pub fn strict_schema(schema: &Value, dialect: Dialect) -> Option<Value> {
    let mut schema = schema.clone();
    strict(&mut schema, dialect).then_some(schema)
}

fn strict(schema: &mut Value, dialect: Dialect) -> bool {
    let Some(object) = schema.as_object_mut() else {
        return true;
    };
    for keyword in UNENFORCED.iter().chain(&ANNOTATIONS) {
        object.remove(*keyword);
    }
    if UNRENDERED
        .iter()
        .any(|keyword| object.contains_key(*keyword))
    {
        return false;
    }
    // Exactly one branch is not a grammar either dialect takes; any branch
    // is wider, and the tool's own validation still holds it to one.
    if let Some(branches) = object.remove("oneOf") {
        if object.contains_key("anyOf") {
            return false;
        }
        object.insert("anyOf".into(), branches);
    }
    if object
        .get("additionalProperties")
        .is_some_and(|extra| extra != &Value::Bool(false))
    {
        // A map of named entries: closing it would refuse every entry.
        return false;
    }
    if object
        .get("minItems")
        .and_then(Value::as_u64)
        .is_some_and(|min| min > 1)
    {
        object.remove("minItems");
    }
    if let Some(items) = object.get_mut("items")
        && !strict(items, dialect)
    {
        return false;
    }
    for union in ["anyOf", "allOf"] {
        if let Some(Value::Array(branches)) = object.get_mut(union) {
            for branch in branches {
                if !strict(branch, dialect) {
                    return false;
                }
            }
        }
    }
    let is_object = object.get("type").and_then(Value::as_str) == Some("object")
        || object.contains_key("properties");
    if !is_object {
        return true;
    }
    if !object.get("properties").is_some_and(Value::is_object) {
        // An object whose fields are not declared has no strict form.
        return false;
    }
    let required: Vec<String> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let Some(Value::Object(properties)) = object.get_mut("properties") else {
        return false;
    };
    for (name, property) in properties.iter_mut() {
        if !strict(property, dialect) {
            return false;
        }
        if dialect == Dialect::OpenAi && !required.contains(name) {
            nullable(property);
        }
    }
    if dialect == Dialect::OpenAi {
        let every: Vec<Value> = properties.keys().cloned().map(Value::String).collect();
        object.insert("required".into(), Value::Array(every));
    }
    object.insert("additionalProperties".into(), Value::Bool(false));
    true
}

/// An optional property in OpenAI's strict form: it is required and takes
/// `null`, which the argument reader reads as not given.
fn nullable(property: &mut Value) {
    let Some(object) = property.as_object_mut() else {
        return;
    };
    match object.get_mut("type") {
        Some(Value::String(kind)) => {
            let kind = kind.clone();
            object.insert("type".into(), serde_json::json!([kind, "null"]));
        }
        Some(Value::Array(kinds)) => {
            if !kinds.iter().any(|kind| kind == "null") {
                kinds.push(Value::String("null".into()));
            }
        }
        _ => {
            let inner = Value::Object(std::mem::take(object));
            let mut wrapped = Map::new();
            wrapped.insert("anyOf".into(), serde_json::json!([inner, {"type": "null"}]));
            *object = wrapped;
            return;
        }
    }
    if let Some(Value::Array(values)) = object.get_mut("enum")
        && !values.iter().any(Value::is_null)
    {
        values.push(Value::Null);
    }
}

/// Which of `tools` go strict, in the order given, and the strict schema of
/// each that does: tools that prefer it first (`ToolDefinition::prefer_strict`),
/// then the rest, within the dialect's published limits.
pub fn select(tools: &[ToolDefinition], dialect: Dialect) -> Vec<Option<Value>> {
    let mut chosen: Vec<Option<Value>> = vec![None; tools.len()];
    let mut order: Vec<usize> = (0..tools.len()).collect();
    order.sort_by_key(|at| !tools[*at].prefer_strict);
    let (mut count, mut optional, mut unions) = (0usize, 0usize, 0usize);
    for at in order {
        let tool = &tools[at];
        let Some(schema) = strict_schema(&tool.parameters, dialect) else {
            continue;
        };
        if dialect == Dialect::Anthropic {
            let (own_optional, own_unions) = weigh(&tool.parameters);
            if count + 1 > ANTHROPIC_STRICT_TOOLS
                || optional + own_optional > ANTHROPIC_OPTIONAL
                || unions + own_unions > ANTHROPIC_UNIONS
            {
                continue;
            }
            optional += own_optional;
            unions += own_unions;
        }
        count += 1;
        chosen[at] = Some(schema);
    }
    chosen
}

/// The optional properties and the unions in a schema, at every depth:
/// what Anthropic's limits count.
fn weigh(schema: &Value) -> (usize, usize) {
    let Some(object) = schema.as_object() else {
        return (0, 0);
    };
    let mut optional = 0;
    let mut unions = usize::from(
        object.contains_key("anyOf") || object.get("type").is_some_and(Value::is_array),
    );
    if let Some(Value::Object(properties)) = object.get("properties") {
        let required: Vec<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for (name, property) in properties {
            if !required.contains(&name.as_str()) {
                optional += 1;
            }
            let (inner_optional, inner_unions) = weigh(property);
            optional += inner_optional;
            unions += inner_unions;
        }
    }
    if let Some(items) = object.get("items") {
        let (inner_optional, inner_unions) = weigh(items);
        optional += inner_optional;
        unions += inner_unions;
    }
    (optional, unions)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn steps() -> Value {
        json!({"type": "object", "properties": {
            "title": {"type": "string"},
            "steps": {"type": "array", "minItems": 1, "items": {"type": "object", "properties": {
                "label": {"type": "string"},
                "minutes": {"type": "integer", "minimum": 1}
            }, "required": ["label"]}},
            "tone": {"type": "string", "enum": ["tip", "note"]}
        }, "required": ["steps"]})
    }

    #[test]
    fn openai_strict_requires_every_property_and_lets_optional_ones_be_null() {
        let strict = strict_schema(&steps(), Dialect::OpenAi).unwrap();
        assert_eq!(strict["additionalProperties"], false);
        assert_eq!(strict["required"], json!(["steps", "title", "tone"]));
        assert_eq!(
            strict["properties"]["title"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            strict["properties"]["tone"]["enum"],
            json!(["tip", "note", null])
        );
        let item = &strict["properties"]["steps"]["items"];
        assert_eq!(item["additionalProperties"], false);
        assert_eq!(
            item["properties"]["minutes"]["type"],
            json!(["integer", "null"])
        );
        assert!(item["properties"]["minutes"].get("minimum").is_none());
        assert_eq!(
            strict["properties"]["steps"]["type"], "array",
            "required stays as it was"
        );
    }

    #[test]
    fn anthropic_strict_closes_objects_and_keeps_required_as_written() {
        let strict = strict_schema(&steps(), Dialect::Anthropic).unwrap();
        assert_eq!(strict["additionalProperties"], false);
        assert_eq!(strict["required"], json!(["steps"]));
        assert_eq!(strict["properties"]["title"]["type"], "string");
    }

    #[test]
    fn one_of_is_rendered_as_any_of_and_each_branch_goes_strict() {
        let schema = json!({"type": "object", "properties": {
            "ops": {"type": "array", "items": {"oneOf": [
                {"type": "object", "properties": {"op": {"type": "string", "enum": ["a"]}}, "required": ["op"]},
                {"type": "object", "properties": {"op": {"type": "string", "enum": ["b"]}, "n": {"type": "integer"}}, "required": ["op"]}
            ]}}
        }, "required": ["ops"], "examples": [{"ops": []}]});
        let strict = strict_schema(&schema, Dialect::OpenAi).unwrap();
        assert!(strict.get("examples").is_none());
        let items = &strict["properties"]["ops"]["items"];
        assert!(items.get("oneOf").is_none());
        assert_eq!(items["anyOf"][1]["additionalProperties"], false);
        assert_eq!(items["anyOf"][1]["required"], json!(["n", "op"]));
    }

    #[test]
    fn a_map_or_an_unrendered_keyword_has_no_strict_form() {
        let map = json!({"type": "object", "properties": {"cells": {
            "type": "object", "properties": {}, "additionalProperties": {"type": "string"}
        }}});
        assert!(strict_schema(&map, Dialect::OpenAi).is_none());
        let not = json!({"type": "object", "properties": {"x": {"not": {"type": "null"}}}});
        assert!(strict_schema(&not, Dialect::Anthropic).is_none());
    }

    #[test]
    fn an_object_of_undeclared_fields_has_no_strict_form() {
        let open = json!({"type": "object", "properties": {"evidence": {"type": "object"}}});
        assert!(strict_schema(&open, Dialect::OpenAi).is_none());
        assert!(strict_schema(&json!({"type": "object"}), Dialect::Anthropic).is_none());
    }

    /// Every wire puts an asked-for output schema in its own place, and only
    /// when the dispatch's settings say the model takes it.
    #[test]
    fn every_wire_renders_an_output_schema_in_its_own_form() {
        use crate::types::{ChatRequest, Message, OutputSchema};
        let mut req = ChatRequest::new("m");
        req.messages = vec![Message::user_text("hi")];
        req.output_schema = Some(OutputSchema {
            name: "steps".into(),
            schema: steps(),
        });
        let on = vak_control::StepSettings {
            schema_output: true,
            ..Default::default()
        };
        let off = vak_control::StepSettings::default();
        let anthropic = crate::anthropic::build_body(&req, 1024, on, false).unwrap();
        assert_eq!(anthropic["output_config"]["format"]["type"], "json_schema");
        assert_eq!(
            anthropic["output_config"]["format"]["schema"]["additionalProperties"],
            false
        );
        let chat =
            crate::openai::build_body(&crate::openai::OpenAiConfig::default(), &req, on).unwrap();
        assert_eq!(chat["response_format"]["json_schema"]["strict"], true);
        let responses = crate::openai_responses::build_body(
            &crate::openai_responses::OpenAiResponsesConfig::default(),
            &req,
            on,
        )
        .unwrap();
        assert_eq!(responses["text"]["format"]["name"], "steps");
        let google = crate::google::build_body(&req, on).unwrap();
        assert_eq!(
            google["generationConfig"]["responseMimeType"],
            "application/json"
        );
        let ollama =
            crate::ollama::build_body(&crate::ollama::OllamaConfig::default(), &req, None, on)
                .unwrap();
        assert_eq!(ollama["format"]["type"], "object");
        assert!(
            crate::ollama::build_body(&crate::ollama::OllamaConfig::default(), &req, None, off)
                .unwrap()
                .get("format")
                .is_none()
        );
        assert!(
            crate::anthropic::build_body(&req, 1024, off, false)
                .unwrap()
                .get("output_config")
                .is_none()
        );
    }

    #[test]
    fn preferred_tools_go_strict_first_within_the_published_limits() {
        let one_optional = json!({"type": "object", "properties": {
            "a": {"type": "string"}
        }});
        let mut tools: Vec<ToolDefinition> = (0..30)
            .map(|at| ToolDefinition::new(format!("t{at}"), "", one_optional.clone()))
            .collect();
        tools[29].prefer_strict = true;
        let chosen = select(&tools, Dialect::Anthropic);
        assert!(chosen[29].is_some(), "a preferred tool is chosen first");
        assert_eq!(chosen.iter().filter(|c| c.is_some()).count(), 20);
        let all = select(&tools, Dialect::OpenAi);
        assert!(all.iter().all(Option::is_some), "no published limit");
    }
}
