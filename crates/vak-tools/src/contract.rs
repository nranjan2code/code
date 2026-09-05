//! Schema-driven validation for model-proposed tool calls.
//!
//! Tool schemas are the contract boundary. The validator intentionally
//! implements the small, provider-neutral JSON Schema subset needed before a
//! call can reach a tool; tool-specific execution remains responsible for
//! deeper domain validation.

use serde_json::Value;

pub fn validate_input(schema: &Value, input: &Value) -> Result<(), String> {
    validate_value(schema, input, "arguments")
}

fn validate_value(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(types) = schema.get("type") {
        let matches = match types {
            Value::String(expected) => type_matches(expected, value),
            Value::Array(expected) => expected
                .iter()
                .filter_map(Value::as_str)
                .any(|expected| type_matches(expected, value)),
            _ => false,
        };
        if !matches {
            return Err(format!(
                "invalid tool arguments: {path} must be {}",
                schema_type_label(types)
            ));
        }
    }
    if let Some(enum_values) = schema.get("enum")
        && enum_values
            .as_array()
            .is_some_and(|values| !values.iter().any(|candidate| candidate == value))
    {
        return Err(format!(
            "invalid tool arguments: {path} is not an allowed value"
        ));
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array)
        && let Some(object) = value.as_object()
    {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                return Err(format!(
                    "invalid tool arguments: required parameter `{key}` was omitted"
                ));
            }
        }
    }
    if let (Some(properties), Some(object)) = (schema.get("properties"), value.as_object())
        && let Some(properties) = properties.as_object()
    {
        for (key, child_schema) in properties {
            if let Some(child) = object.get(key) {
                validate_value(child_schema, child, &format!("{path}.{key}"))?;
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (index, child) in array.iter().enumerate() {
            validate_value(items, child, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn schema_type_label(types: &Value) -> String {
    match types {
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" or "),
        _ => "the declared type".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_input;
    use serde_json::json;

    #[test]
    fn required_fields_are_checked_without_tool_names() {
        let schema = json!({
            "type": "object",
            "properties": {"action": {"type": "string"}},
            "required": ["action"]
        });
        assert!(validate_input(&schema, &json!({})).is_err());
        assert!(validate_input(&schema, &json!({"action": "list"})).is_ok());
    }

    #[test]
    fn nested_types_and_enums_are_checked() {
        let schema = json!({
            "type": "object",
            "properties": {
                "mode": {"type": "string", "enum": ["read", "write"]},
                "items": {"type": "array", "items": {"type": "integer"}}
            }
        });
        assert!(validate_input(&schema, &json!({"mode": "read", "items": [1, 2]})).is_ok());
        assert!(validate_input(&schema, &json!({"mode": "other"})).is_err());
        assert!(validate_input(&schema, &json!({"items": ["one"]})).is_err());
    }
}
