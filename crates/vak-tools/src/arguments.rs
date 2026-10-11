//! The one reader for the arguments of every tool call
//! (docs/design/04-tools.md, "Reading arguments").
//!
//! A caller often sends the right value in another JSON shape: a list as
//! text, one record where a list of records is asked, a number in quotes.
//! A shape only one tool can read (a table's nested rows, a list for a
//! field that tool splits itself) is that tool's own `Tool::fill_defaults`.
//! `read_arguments` gives each such value the shape the tool's own schema
//! asks for, before validation, for every tool alike: built-in, loop-owned
//! or added at runtime. It is driven by the schema alone, never by a tool's
//! name, and converts only where nothing is lost and nothing is guessed.
//! Whatever it cannot read this way is left as sent, for validation to
//! refuse by name.

use serde_json::{Map, Value};

/// Gives `args` the shapes `schema` asks for, in place.
pub fn read_arguments(schema: &Value, args: &mut Value) {
    read(schema, args);
}

fn accepts(schema: &Value, kind: &str) -> bool {
    match schema.get("type") {
        Some(Value::String(expected)) => expected == kind,
        Some(Value::Array(expected)) => expected.iter().any(|item| item.as_str() == Some(kind)),
        _ => kind == "object" && schema.get("properties").is_some(),
    }
}

fn read(schema: &Value, value: &mut Value) {
    let (list, record) = (accepts(schema, "array"), accepts(schema, "object"));
    // A list or record sent as its JSON text.
    if let Some(decoded) = value
        .as_str()
        .filter(|_| list || record)
        .and_then(|text| serde_json::from_str::<Value>(text.trim()).ok())
        .filter(|decoded| (list && decoded.is_array()) || (record && decoded.is_object()))
    {
        *value = decoded;
    }
    if list {
        let items = schema.get("items");
        match value {
            Value::Object(fields)
                if items.is_some_and(|items| {
                    accepts(items, "object")
                        && required_of(items)
                            .iter()
                            .all(|name| fields.contains_key(*name))
                }) =>
            {
                *value = Value::Array(vec![value.take()]);
            }
            _ => {}
        }
        if let (Some(items), Value::Array(list)) = (items, value) {
            for item in list {
                read(items, item);
            }
        }
    } else if record {
        if let Some(only) = value.as_str().and_then(|text| record_of(schema, text)) {
            *value = only;
        }
        if let (Some(properties), Value::Object(fields)) =
            (schema.get("properties").and_then(Value::as_object), value)
        {
            let required = required_of(schema);
            fields.retain(|name, field| !field.is_null() || required.contains(&name.as_str()));
            for (name, field) in fields.iter_mut() {
                if let Some(property) = properties.get(name) {
                    read(property, field);
                }
            }
        }
    } else if accepts(schema, "string") {
        if let Some(text) = scalar_text(value).filter(|_| !value.is_string()) {
            *value = Value::String(text);
        }
    } else if let Value::String(text) = value {
        let text = text.trim();
        let exact = if accepts(schema, "integer") {
            text.parse::<i64>().ok().map(Value::from)
        } else if accepts(schema, "number") {
            text.parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
        } else if accepts(schema, "boolean") {
            text.parse::<bool>().ok().map(Value::Bool)
        } else {
            None
        };
        if let Some(exact) = exact {
            *value = exact;
        }
    }
}

fn required_of(schema: &Value) -> Vec<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// Text sent where a record is asked, when the record has exactly one
/// required field that takes free text: the text is that field. A record
/// with two such fields is never filled by cutting one text in two.
fn record_of(schema: &Value, text: &str) -> Option<Value> {
    let properties = schema.get("properties")?.as_object()?;
    let mut free = required_of(schema).into_iter().filter(|name| {
        properties
            .get(*name)
            .is_some_and(|property| accepts(property, "string") && property.get("enum").is_none())
    });
    let (only, None) = (free.next()?, free.next()) else {
        return None;
    };
    if required_of(schema).len() != 1 {
        return None;
    }
    let mut record = Map::new();
    record.insert(only.to_string(), Value::String(text.trim().to_string()));
    Some(Value::Object(record))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn read_as(schema: Value, mut args: Value) -> Value {
        read_arguments(&schema, &mut args);
        args
    }

    fn steps() -> Value {
        json!({"type": "object", "properties": {
            "title": {"type": "string"},
            "count": {"type": "integer"},
            "ratio": {"type": "number"},
            "done": {"type": "boolean"},
            "tags": {"type": "array", "items": {"type": "string"}},
            "items": {"type": "array", "items": {"type": "object",
                "properties": {
                    "label": {"type": "string"},
                    "detail": {"type": "string"},
                    "minutes": {"type": "integer"}
                },
                "required": ["label"]}}
        }, "required": ["items"]})
    }

    #[test]
    fn a_value_already_in_shape_is_left_as_sent() {
        let sent = json!({"title": "T", "count": 2, "tags": ["a"], "items": [
            {"label": "One", "detail": "- not a list", "minutes": 5}
        ]});
        assert_eq!(read_as(steps(), sent.clone()), sent);
    }

    #[test]
    fn a_list_sent_as_its_json_text_is_its_items_and_other_text_is_left() {
        let json_text = read_as(steps(), json!({"items": "[{\"label\": \"One\"}]"}));
        assert_eq!(json_text["items"], json!([{"label": "One"}]));
        // Found live: broken JSON read line by line became checklist items
        // the person was shown. Text that is not a list's JSON is left as
        // sent; reading lines out of it would be a guess.
        for text in ["- red\n- green", "[ \"red\" ]\n]", "red"] {
            let sent = json!({"tags": text, "items": []});
            assert_eq!(read_as(steps(), sent.clone()), sent);
        }
    }

    #[test]
    fn one_record_or_one_text_stands_for_a_list_of_them() {
        let one = read_as(steps(), json!({"items": {"label": "One"}}));
        assert_eq!(one["items"], json!([{"label": "One"}]));
        let map = json!({"items": {"start": "6 pm", "budget": "$60"}});
        assert_eq!(
            read_as(steps(), map.clone()),
            map,
            "an object that is not one of the list's records is left as sent"
        );
        let texts = read_as(steps(), json!({"items": ["One", "Two: with detail"]}));
        assert_eq!(
            texts["items"],
            json!([{"label": "One"}, {"label": "Two: with detail"}]),
            "the whole text is the one required field; it is never cut in two"
        );
    }

    #[test]
    fn text_is_never_cut_to_fill_two_required_fields() {
        let edit = json!({"type": "object", "properties": {
            "edits": {"type": "array", "items": {"type": "object",
                "properties": {"old": {"type": "string"}, "new": {"type": "string"}},
                "required": ["old", "new"]}}
        }});
        let sent = json!({"edits": ["before: after"]});
        assert_eq!(read_as(edit, sent.clone()), sent);
        let plan = json!({"type": "object", "properties": {
            "deliverables": {"type": "array", "items": {"type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "form": {"type": "string", "enum": ["card", "text"]}
                },
                "required": ["name", "form"]}}
        }});
        let sent = json!({"deliverables": ["routine"]});
        assert_eq!(
            read_as(plan, sent.clone()),
            sent,
            "a form cannot be invented"
        );
    }

    #[test]
    fn scalars_change_type_only_when_exact() {
        let read = read_as(
            steps(),
            json!({"title": 12, "count": " 4 ", "ratio": "0.5", "done": "true",
                   "items": [{"label": 7, "minutes": "ten"}]}),
        );
        assert_eq!(read["title"], "12");
        assert_eq!(read["count"], 4);
        assert_eq!(read["ratio"], 0.5);
        assert_eq!(read["done"], true);
        assert_eq!(read["items"][0]["label"], "7");
        assert_eq!(
            read["items"][0]["minutes"], "ten",
            "left for validation to refuse"
        );
        let listed = read_as(steps(), json!({"title": ["a", "b"], "items": []}));
        assert_eq!(
            listed["title"],
            json!(["a", "b"]),
            "a list is never joined into one text: the separator would be a guess"
        );
    }

    #[test]
    fn a_null_for_a_field_that_may_be_absent_is_absent() {
        let read = read_as(steps(), json!({"title": null, "items": null}));
        assert!(read.get("title").is_none());
        assert!(
            read["items"].is_null(),
            "a required field stays for validation"
        );
    }
}
