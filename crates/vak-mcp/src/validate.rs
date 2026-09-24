use serde_json::Value;

/// Validate the portable JSON-Schema subset used by MCP tool declarations.
/// This deliberately runs before `tools/call`, turning provider mistakes into
/// actionable broker errors without adding a schema dependency to the client.
pub fn validate_arguments(arguments: &Value, schema: &Value) -> Result<(), String> {
    validate(arguments, schema, "$")
}

fn validate(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    if let Some(types) = schema.get("type") {
        let valid = match types {
            Value::String(kind) => matches_type(value, kind),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| matches_type(value, kind)),
            _ => true,
        };
        if !valid {
            return Err(format!("invalid MCP arguments: {path} must be {}", types));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.iter().any(|candidate| candidate == value)
    {
        return Err(format!(
            "invalid MCP arguments: {path} is not an allowed enum value"
        ));
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array)
        && let Some(object) = value.as_object()
    {
        for name in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(name) {
                return Err(format!("invalid MCP arguments: {path}.{name} is required"));
            }
        }
    }
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        let mut matched = 0usize;
        let mut last_err = String::new();
        for sub in one_of {
            match validate(value, sub, &format!("{path} (oneOf)")) {
                Ok(()) => matched += 1,
                Err(e) => last_err = e,
            }
        }
        if matched != 1 {
            return Err(if matched == 0 {
                last_err
            } else {
                format!(
                    "invalid MCP arguments: {path} matched {matched} of {} oneOf branches",
                    one_of.len()
                )
            });
        }
        return Ok(());
    }
    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(Value::as_object),
        value.as_object(),
    ) {
        for (name, child_schema) in properties {
            if let Some(child) = object.get(name) {
                validate(child, child_schema, &format!("{path}.{name}"))?;
            }
        }
    }
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false)
        && let Some(object) = value.as_object()
    {
        let known: std::collections::HashSet<&str> = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| props.keys().map(|k| k.as_str()).collect())
            .unwrap_or_default();
        for name in object.keys() {
            if !known.contains(name.as_str()) {
                return Err(format!(
                    "invalid MCP arguments: unexpected parameter `{name}` for {path}"
                ));
            }
        }
    }
    if let Some(items) = schema.get("items")
        && let Some(array) = value.as_array()
    {
        for (index, child) in array.iter().enumerate() {
            validate(child, items, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

fn matches_type(value: &Value, kind: &str) -> bool {
    match kind {
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

#[cfg(test)]
mod tests {
    use super::validate_arguments;
    use serde_json::json;

    #[test]
    fn validates_required_and_nested_types() {
        let schema = json!({
            "type": "object",
            "required": ["input"],
            "properties": {"input": {"type": "string"}}
        });
        assert!(validate_arguments(&json!({"input": "query"}), &schema).is_ok());
        assert!(validate_arguments(&json!({}), &schema).is_err());
        assert!(validate_arguments(&json!({"input": {"query": "query"}}), &schema).is_err());
    }

    #[test]
    fn validates_array_items() {
        let schema = json!({"type": "array", "items": {"type": "string"}});
        assert!(validate_arguments(&json!(["one", "two"]), &schema).is_ok());
        assert!(validate_arguments(&json!(["one", 2]), &schema).is_err());
    }

    #[test]
    fn validates_server_declared_enum_options_before_call() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "depth": {"type": "string", "enum": ["basic", "advanced"]}
            },
            "required": ["query"]
        });
        assert!(validate_arguments(&json!({"query": "weather"}), &schema).is_ok());
        assert!(
            validate_arguments(&json!({"query": "weather", "depth": "advanced"}), &schema).is_ok()
        );
        assert!(
            validate_arguments(
                &json!({"query": "weather", "depth": "unsupported"}),
                &schema
            )
            .is_err()
        );
    }

    #[test]
    fn validates_one_of_and_additional_properties() {
        let schema = json!({
            "oneOf": [
                {"properties": {"q": {"type": "string"}}, "required": ["q"], "additionalProperties": false},
                {"properties": {"id": {"type": "integer"}}, "required": ["id"], "additionalProperties": false}
            ]
        });
        assert!(validate_arguments(&json!({"q": "hello"}), &schema).is_ok());
        assert!(validate_arguments(&json!({"id": 7}), &schema).is_ok());
        // neither branch
        assert!(validate_arguments(&json!({"nope": 1}), &schema).is_err());
        // unexpected key under a matching branch
        assert!(validate_arguments(&json!({"q": "x", "extra": 1}), &schema).is_err());
    }
}
