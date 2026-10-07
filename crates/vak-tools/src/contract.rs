//! Schema-driven validation for model-proposed tool calls.
//!
//! Tool schemas are the contract boundary. The validator intentionally
//! implements the small, provider-neutral JSON Schema subset needed before a
//! call can reach a tool; tool-specific execution remains responsible for
//! deeper domain validation.

use serde_json::Value;

/// Every way `input` breaks `schema`, in one message: the first problem
/// alone let a small model fix one field per turn (measured live: three
/// retries of a card call that was missing both `semantic_type` and
/// `payload`, each told only of the first). A missing or unexpected parameter
/// also names the parameters the call takes.
pub fn validate_input(schema: &Value, input: &Value) -> Result<(), String> {
    let mut problems = Vec::new();
    check(schema, input, "arguments", &mut problems);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("invalid tool arguments: {}", problems.join("; ")))
    }
}

fn check(schema: &Value, value: &Value, path: &str, problems: &mut Vec<String>) {
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
            problems.push(format!("{path} must be {}", schema_type_label(types)));
            return;
        }
    }
    if let Some(enum_values) = schema.get("enum")
        && enum_values
            .as_array()
            .is_some_and(|values| !values.iter().any(|candidate| candidate == value))
    {
        problems.push(format!("{path} is not an allowed value"));
        return;
    }
    // `oneOf`: the value must match exactly one branch. Used for conditional
    // contracts (e.g. the `mcp` broker where `server`/`tool` are required only
    // when `action == "call"`). Each branch carries its own required/properties,
    // so matching a branch validates the whole per-branch contract.
    if let Some(one_of) = schema.get("oneOf").and_then(Value::as_array) {
        // A tagged union (every branch fixes one key, like `op`, to a single
        // value): report against the branch the value names, or list the
        // names, rather than whichever branch happened to be checked last.
        if let Some((key, names)) = discriminator(one_of) {
            match value.get(key).and_then(Value::as_str) {
                Some(name) => match one_of
                    .iter()
                    .zip(&names)
                    .find(|(_, candidate)| **candidate == name)
                {
                    Some((branch, _)) => check(branch, value, path, problems),
                    None => problems.push(format!(
                        "{path}.{key} {name:?} is not one of: {}",
                        names.join(", ")
                    )),
                },
                None => problems.push(format!(
                    "{path} needs `{key}`, one of: {}",
                    names.join(", ")
                )),
            }
            return;
        }
        let mut matched = 0usize;
        let mut last = Vec::new();
        for sub in one_of {
            let mut branch = Vec::new();
            check(sub, value, &format!("{path} (oneOf)"), &mut branch);
            if branch.is_empty() {
                matched += 1;
            } else {
                last = branch;
            }
        }
        match matched {
            1 => {}
            0 => problems.extend(last),
            _ => problems.push(format!(
                "{path} matched {matched} of {} oneOf branches",
                one_of.len()
            )),
        }
        return;
    }
    let Some(object) = value.as_object() else {
        if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
            for (index, child) in array.iter().enumerate() {
                check(items, child, &format!("{path}[{index}]"), problems);
            }
        }
        return;
    };
    let properties = schema.get("properties").and_then(Value::as_object);
    let required_keys: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|keys| keys.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    // The required parameters first, in the schema's order, then the rest.
    let takes = || {
        let optional = properties
            .into_iter()
            .flat_map(|props| props.keys())
            .map(String::as_str)
            .filter(|key| !required_keys.contains(key));
        required_keys
            .iter()
            .copied()
            .chain(optional)
            .map(|key| format!("`{key}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let before = problems.len();
    for key in &required_keys {
        if !object.contains_key(*key) {
            problems.push(format!("{path} is missing required `{key}`"));
        }
    }
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false)
        || problems.len() > before
    {
        for key in object.keys() {
            if !properties.is_some_and(|props| props.contains_key(key)) {
                problems.push(format!("unexpected parameter `{key}` for {path}"));
            }
        }
    }
    if problems.len() > before && properties.is_some() {
        problems.push(format!("{path} takes {}", takes()));
    }
    if let Some(properties) = properties {
        for (key, child_schema) in properties {
            if let Some(child) = object.get(key) {
                check(child_schema, child, &format!("{path}.{key}"), problems);
            }
        }
    }
}

/// The key and per-branch names when every `oneOf` branch pins the same
/// property to one allowed value.
fn discriminator(branches: &[Value]) -> Option<(&str, Vec<&str>)> {
    let first = branches.first()?.get("properties")?.as_object()?;
    let key = first.iter().find_map(|(key, property)| {
        let values = property.get("enum")?.as_array()?;
        (values.len() == 1 && values[0].is_string()).then_some(key.as_str())
    })?;
    let names = branches
        .iter()
        .map(|branch| {
            let values = branch
                .pointer(&format!("/properties/{key}/enum"))?
                .as_array()?;
            match values.as_slice() {
                [Value::String(name)] => Some(name.as_str()),
                _ => None,
            }
        })
        .collect::<Option<Vec<&str>>>()?;
    Some((key, names))
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
#[allow(clippy::unwrap_used)]
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

    #[test]
    fn one_of_enforces_exactly_one_branch() {
        let schema = json!({
            "type": "object",
            "oneOf": [
                {"properties": {"action": {"type": "string", "enum": ["list"]}}, "required": ["action"], "additionalProperties": false},
                {"properties": {"action": {"type": "string", "enum": ["call"]}, "server": {"type": "string"}, "tool": {"type": "string"}}, "required": ["action", "server", "tool"], "additionalProperties": false}
            ]
        });
        // list branch
        assert!(validate_input(&schema, &json!({"action": "list"})).is_ok());
        // call branch, complete
        assert!(
            validate_input(
                &schema,
                &json!({"action": "call", "server": "s", "tool": "t"})
            )
            .is_ok()
        );
        // call missing required `server` -> no branch matches -> error
        assert!(validate_input(&schema, &json!({"action": "call", "tool": "t"})).is_err());
        // unknown action -> neither branch matches -> error
        assert!(validate_input(&schema, &json!({"action": "ping"})).is_err());
        // list branch but with an unexpected param -> list branch invalid, call
        // branch invalid (action not "call") -> exactly zero match -> error
        assert!(validate_input(&schema, &json!({"action": "list", "server": "s"})).is_err());
    }

    #[test]
    fn every_problem_is_reported_at_once_with_the_parameters_the_call_takes() {
        let schema = json!({
            "type": "object",
            "properties": {
                "semantic_type": {"type": "string"},
                "payload": {"type": "object"}
            },
            "required": ["semantic_type", "payload"]
        });
        let error = validate_input(
            &schema,
            &json!({"command_run": "sed -n 3p big.txt", "output": "row 3"}),
        )
        .unwrap_err();
        assert!(error.starts_with("invalid tool arguments: "), "{error}");
        for part in [
            "arguments is missing required `semantic_type`",
            "arguments is missing required `payload`",
            "unexpected parameter `command_run`",
            "unexpected parameter `output`",
            "arguments takes `semantic_type`, `payload`",
        ] {
            assert!(error.contains(part), "missing {part:?} in {error}");
        }
    }

    /// A field missing inside a list item names the item, so it is not read
    /// as the call's own field (live: a research card refused twice for a
    /// `title` each source lacked, read as the card's own `title`).
    #[test]
    fn a_missing_nested_field_names_its_item() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "sources": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"title": {"type": "string"}, "url": {"type": "string"}},
                        "required": ["title"]
                    }
                }
            },
            "required": ["title"]
        });
        let error = validate_input(
            &schema,
            &json!({"title": "News", "sources": [{"url": "https://example.com"}]}),
        )
        .unwrap_err();
        assert!(
            error.contains("arguments.sources[0] is missing required `title`"),
            "{error}"
        );
        assert!(!error.contains("arguments is missing"), "{error}");
    }

    #[test]
    fn additional_properties_false_rejects_unknown_keys() {
        let schema = json!({
            "type": "object",
            "properties": {"action": {"type": "string"}},
            "required": ["action"],
            "additionalProperties": false
        });
        assert!(validate_input(&schema, &json!({"action": "list"})).is_ok());
        assert!(validate_input(&schema, &json!({"action": "list", "extra": 1})).is_err());
    }
}
