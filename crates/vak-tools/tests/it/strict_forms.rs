use serde_json::Value;
use vak_llm::strict::{Dialect, strict_schema};

/// What a strict grammar renders on every wire Vak sends strict tools to.
const STRICT_KEYWORDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "anyOf",
    "description",
    "const",
    "minItems",
    "pattern",
    "format",
];

fn foreign(schema: &Value, path: &str, out: &mut Vec<String>) {
    match schema {
        Value::Object(object) => {
            for (key, value) in object {
                let at = format!("{path}/{key}");
                if key == "properties" {
                    for (name, property) in value.as_object().into_iter().flatten() {
                        foreign(property, &format!("{at}/{name}"), out);
                    }
                    continue;
                }
                if !STRICT_KEYWORDS.contains(&key.as_str()) {
                    out.push(at.clone());
                }
                foreign(value, &at, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                foreign(value, path, out);
            }
        }
        _ => {}
    }
}

/// A strict form a wire refuses fails the whole request, and the retry
/// without it turns strict off for every tool on that model
/// (2026-10-17: `office_apply`'s `oneOf` did).
#[test]
fn every_tools_strict_form_uses_only_what_a_strict_grammar_takes() {
    let mut out = Vec::new();
    for definition in vak_tools::definitions(&vak_tools::default_tools()) {
        for dialect in [Dialect::OpenAi, Dialect::Anthropic] {
            if let Some(schema) = strict_schema(&definition.parameters, dialect) {
                foreign(&schema, &definition.name, &mut out);
            }
        }
    }
    assert!(out.is_empty(), "keywords no strict grammar takes: {out:?}");
}
