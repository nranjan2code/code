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

/// The first value in `args`, read by `read_arguments`, that `schema` asks
/// for as a list or a record and that is still text which does not parse:
/// a syntax fault, which only rewriting the text repairs
/// (docs/design/89-constrained-tool-arguments.md §4.3). It is named by its
/// path from the arguments, `arguments.rows`.
pub fn unreadable(schema: &Value, args: &Value) -> Option<String> {
    fault_in(schema, args, "arguments")
}

fn fault_in(schema: &Value, value: &Value, path: &str) -> Option<String> {
    let (list, record) = (accepts(schema, "array"), accepts(schema, "object"));
    if let Value::String(text) = value
        && (list || record)
    {
        return json_fault(text).map(|fault| fault.describe(path));
    }
    if list && let (Some(items), Value::Array(values)) = (schema.get("items"), value) {
        // The whole list written as text and sent as its one item: its
        // items are text, and that text is a list that does not read
        // (measured: a table's `rows` as `["[\"A | B\", \"C | D\",\"\"]"]`).
        if let [Value::String(text)] = values.as_slice()
            && !accepts(items, "array")
            && text
                .trim_start()
                .strip_prefix('[')
                .is_some_and(|rest| rest.trim_start().starts_with('"'))
            && let Some(fault) = json_fault(text)
        {
            return Some(fault.describe(&format!("{path}[0]")));
        }
        return values
            .iter()
            .enumerate()
            .find_map(|(at, item)| fault_in(items, item, &format!("{path}[{at}]")));
    }
    if let (Some(properties), Value::Object(fields)) =
        (schema.get("properties").and_then(Value::as_object), value)
    {
        return fields.iter().find_map(|(name, field)| {
            properties
                .get(name)
                .and_then(|property| fault_in(property, field, &format!("{path}.{name}")))
        });
    }
    None
}

/// Where text sent for a list or a record stops being JSON. Some wires
/// carry every argument as text and their provider reads it; text it
/// cannot read arrives here as sent, and the caller is told where it
/// breaks rather than that it "sent text".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonFault {
    /// What the parser expected or refused, without a position.
    pub problem: String,
    /// The value the fault is inside, from the text's root: `[1].rows`.
    pub inside: String,
    /// The text just before the fault, and from the fault on.
    pub before: String,
    pub after: String,
}

impl JsonFault {
    /// What the caller is told about the argument `name`.
    pub fn describe(&self, name: &str) -> String {
        let after = if self.after.is_empty() {
            "the end of the text".to_string()
        } else {
            format!("`{}`", self.after)
        };
        format!(
            "{name} is not valid JSON, so none of it was read: {} in {name}{}, after `{}` and before {after}",
            self.problem, self.inside, self.before
        )
    }
}

/// The fault in `text` when it is written as a JSON list or record and
/// does not read as one. Text that is not written as either has none.
pub fn json_fault(text: &str) -> Option<JsonFault> {
    let text = text.trim();
    if !text.starts_with(['[', '{']) {
        return None;
    }
    let error = serde_json::from_str::<Value>(text).err()?;
    // The byte the parser stopped at.
    let line_start = text
        .match_indices('\n')
        .nth(error.line().saturating_sub(2))
        .filter(|_| error.line() > 1)
        .map_or(0, |(at, _)| at + 1);
    let mut at = if error.is_eof() {
        text.len()
    } else {
        (line_start + error.column().saturating_sub(1)).min(text.len())
    };
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let message = error.to_string();
    let problem = message
        .rfind(" at line ")
        .map_or(message.as_str(), |end| &message[..end])
        .to_string();
    let before: String = {
        let mut shown: Vec<char> = text[..at].chars().rev().take(EXCERPT_BEFORE).collect();
        shown.reverse();
        shown.into_iter().collect()
    };
    let after: String = text[at..].chars().take(EXCERPT_AFTER).collect();
    Some(JsonFault {
        problem,
        inside: inside(&text[..at]),
        before,
        after,
    })
}

/// How much of the text is quoted on each side of a fault.
const EXCERPT_BEFORE: usize = 100;
const EXCERPT_AFTER: usize = 40;

/// The value a JSON text was inside when it was cut off, as a path of
/// list positions and record keys.
fn inside(read: &str) -> String {
    enum Open {
        List(usize),
        Record(Option<String>),
    }
    let mut open: Vec<Open> = Vec::new();
    let mut chars = read.chars();
    while let Some(c) = chars.next() {
        match c {
            '[' => open.push(Open::List(0)),
            '{' => open.push(Open::Record(None)),
            ']' | '}' => {
                open.pop();
            }
            ',' => match open.last_mut() {
                Some(Open::List(at)) => *at += 1,
                Some(Open::Record(key)) => *key = None,
                None => {}
            },
            '"' => {
                let mut string = String::new();
                let mut closed = false;
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            chars.next();
                        }
                        '"' => {
                            closed = true;
                            break;
                        }
                        c => string.push(c),
                    }
                }
                if closed && let Some(Open::Record(key @ None)) = open.last_mut() {
                    *key = Some(string);
                }
            }
            _ => {}
        }
    }
    // The innermost list is the one the fault is in, not an item of it.
    let last = open.len().saturating_sub(1);
    open.iter()
        .enumerate()
        .map(|(depth, frame)| match frame {
            Open::List(_) if depth == last => String::new(),
            Open::List(at) => format!("[{at}]"),
            Open::Record(Some(key)) => format!(".{key}"),
            Open::Record(None) => String::new(),
        })
        .collect()
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

/// Moves the first of `aliases` present to `key` when `key` is missing: a
/// tool's own `fill_defaults` uses it for a name it accepts for one of its
/// arguments. The value is moved as sent, never reshaped.
pub fn alias(args: &mut Value, key: &str, aliases: &[&str]) {
    let Some(fields) = args.as_object_mut() else {
        return;
    };
    if fields.contains_key(key) {
        return;
    }
    if let Some(value) = aliases.iter().find_map(|alias| fields.remove(*alias)) {
        fields.insert(key.to_string(), value);
    }
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

    #[test]
    fn text_that_is_not_json_says_where_it_breaks_and_what_it_was_inside() {
        // A list never closed before the next key (seen live, 2026-10-11).
        let text =
            r#"[{"type": "text", "text": "Hi."}, {"rows": ["A | B", "C | D", "type": "table"}]"#;
        let fault = json_fault(text).unwrap();
        assert_eq!(fault.inside, "[1].rows");
        assert_eq!(fault.problem, "expected `,` or `]`");
        assert!(fault.before.ends_with(r#""C | D", "type""#), "{fault:?}");
        assert!(fault.after.starts_with(r#": "table"}"#), "{fault:?}");
        let told = fault.describe("blocks");
        assert!(told.contains("in blocks[1].rows"), "{told}");
        // An escape JSON does not have, inside a string.
        let fault = json_fault(r#"[{"type": "text", "text": "Costs \$8 a month"}]"#).unwrap();
        assert_eq!(fault.inside, "[0].text");
        assert!(fault.problem.contains("escape"), "{fault:?}");
        // Cut off at the end, on a later line.
        let fault = json_fault("[{\"type\": \"steps\",\n \"steps\": [\"a\"").unwrap();
        assert_eq!(fault.inside, "[0].steps");
        assert!(
            fault
                .describe("blocks")
                .ends_with("before the end of the text")
        );
        // Text that reads, and text never written as JSON, has no fault.
        assert_eq!(json_fault("[1, 2]"), None);
        assert_eq!(json_fault("three steps"), None);
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

    #[test]
    fn unreadable_names_the_field_whose_text_is_broken_json() {
        let schema = serde_json::json!({"type": "object", "properties": {
            "title": {"type": "string"},
            "rows": {"type": "array", "items": {"type": "string"}}
        }});
        let mut args = serde_json::json!({"title": "T", "rows": "[\"A | B\", \"1 | 2\"]\n[\"x\"]"});
        read_arguments(&schema, &mut args);
        let fault = unreadable(&schema, &args).expect("rows is broken");
        assert!(
            fault.starts_with("arguments.rows is not valid JSON"),
            "{fault}"
        );
        let mut fine = serde_json::json!({"title": "T", "rows": "[\"A | B\"]"});
        read_arguments(&schema, &mut fine);
        assert!(unreadable(&schema, &fine).is_none());
        assert!(
            unreadable(&schema, &serde_json::json!({"title": "[not json"})).is_none(),
            "a field asked for as text is never a syntax fault"
        );
    }

    #[test]
    fn a_whole_list_written_as_its_one_item_is_unreadable() {
        let schema = serde_json::json!({"type": "object", "properties": {
            "rows": {"type": "array", "items": {"type": "string"}}
        }});
        let mut args = serde_json::json!({"rows": [
            "[\"Plant | Ranking\",\"ZZ | Top tier\",\"Pothos | High,\"\"]"
        ]});
        read_arguments(&schema, &mut args);
        let fault = unreadable(&schema, &args).expect("the one item is a broken list");
        assert!(
            fault.starts_with("arguments.rows[0] is not valid JSON"),
            "{fault}"
        );
        for fine in [
            serde_json::json!({"rows": ["[x] buy milk"]}),
            serde_json::json!({"rows": ["[draft | x"]}),
            serde_json::json!({"rows": ["[\"A | B\"", "1 | 2"]}),
        ] {
            assert!(unreadable(&schema, &fine).is_none(), "{fine}");
        }
    }
}
