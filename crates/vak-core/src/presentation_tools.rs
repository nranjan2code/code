//! The card tools of an answer (docs/design/92-card-forms.md).
//!
//! Each `CardShape` is one form and its card tool, `emit_<form>_card`: its
//! flat arguments and the stored card its builder makes from them. A form's
//! own kind is its name; an active pack may add kinds to it
//! (docs/design/91-presentation-capabilities.md). It is a `Tool` so a turn
//! admits it like any other capability and a channel policy can narrow it;
//! the loop shows its cards (`show_card` in vak-agent).
//!
//! The arguments follow the measured rules of docs/design/89-constrained-
//! tool-arguments.md and docs/design/30-render-architecture.md §30.1: the
//! fields sit at the top level, the schema uses only the keywords every
//! provider carries (`type`, `properties`, `items`, `required`, `enum`,
//! `description`), no data travels as a key, no record in a list holds a
//! list of records, and one argument name is one shape on every tool.
//!
//! `execute()` checks a card against the form's validator in vak-delivery;
//! the card is recorded as a `Presentation` from the call's own arguments,
//! never from result text.

use async_trait::async_trait;
use serde_json::Value;
use vak_tools::Fault;
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

/// One form: the tool a model calls for it and how its arguments become the
/// stored card.
struct CardShape {
    /// The form, which is also its own kind (`vak_delivery::FORMS`).
    form: &'static str,
    /// Tool name, `emit_<form>_card`.
    name: &'static str,
    description: &'static str,
    /// This form's own reading of the arguments before the schema check
    /// (`Tool::fill_defaults`): a structure only this form can read.
    fill: fn(&mut Map<String, Value>),
    /// A card that shows what a command printed (`Tool::output_of_command`).
    output_of_command: Option<vak_tools::CommandOutput>,
    /// A card that shows the files this turn changed (`Tool::changes_made`).
    changes_made: Option<vak_tools::ChangesMade>,
    /// A card that shows what an ability answered (`Tool::data_of_call`)
    /// and how its records become the filled arguments.
    data_of_call: Option<(vak_tools::CallData, FillFromData)>,
    /// What only a call in the conversation can have produced for this
    /// card to show (`Tool::grounded_in`): it loads once one has.
    grounded_in: Option<vak_tools::Grounds>,
    /// The arguments, as `{"properties": {...}, "required": [...]}`.
    fields: fn() -> Value,
    /// The stored card for validated arguments, or a refusal the model can
    /// act on.
    build: fn(&Map<String, Value>) -> Result<Value, String>,
    /// Fields of the stored card that may follow published values
    /// (`Tool::followable_fields`, docs/design/98 §2).
    followable: &'static [&'static [&'static str]],
}

type Map<K, V> = serde_json::Map<K, V>;

/// How an ability's records become a card's filled arguments.
type FillFromData = fn(&mut Map<String, Value>, &[Value]) -> Result<(), String>;

fn string(description: &str) -> Value {
    serde_json::json!({"type": "string", "description": description})
}

fn integer(description: &str) -> Value {
    serde_json::json!({"type": "integer", "description": description})
}

fn boolean(description: &str) -> Value {
    serde_json::json!({"type": "boolean", "description": description})
}

fn texts(description: &str) -> Value {
    serde_json::json!({"type": "array", "description": description, "items": {"type": "string"}})
}

fn one_of(values: &[&str], description: &str) -> Value {
    serde_json::json!({"type": "string", "enum": values, "description": description})
}

fn list_of(description: &str, properties: Value, required: &[&str]) -> Value {
    serde_json::json!({
        "type": "array",
        "description": description,
        "items": {"type": "object", "properties": properties, "required": required}
    })
}

fn close_records(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        for property in properties.values_mut() {
            close_records(property);
        }
        object.insert("additionalProperties".into(), Value::Bool(false));
    }
    if let Some(items) = object.get_mut("items") {
        close_records(items);
    }
}

fn fields(properties: Value, required: &[&str]) -> Value {
    serde_json::json!({"properties": properties, "required": required})
}

const TITLE: &str = "Optional. A short heading for the content, in your own words.";

/// The arguments' own fields the stored card keeps as they are, dropping
/// empty text.
fn copy(args: &Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|key| {
            let value = args.get(*key)?;
            match value {
                Value::String(text) if text.trim().is_empty() => None,
                Value::String(text) => {
                    Some(((*key).to_string(), Value::String(text.trim().into())))
                }
                Value::Null => None,
                other => Some(((*key).to_string(), other.clone())),
            }
        })
        .collect()
}

fn records<'a>(args: &'a Map<String, Value>, key: &str) -> &'a [Value] {
    args.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn record_copy(record: &Value, keys: &[&str]) -> Value {
    Value::Object(copy(record.as_object().unwrap_or(&Map::new()), keys))
}

fn text<'a>(record: &'a Value, key: &str) -> &'a str {
    record
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

/// A chart scalar keeps text as text. Numeric-looking category labels are
/// identities, and a numeric field must already have passed argument reading.
fn scalar(value: &Value) -> Value {
    match value {
        Value::Number(_) | Value::String(_) => value.clone(),
        Value::Null => Value::String(String::new()),
        other => Value::String(other.to_string()),
    }
}

// ---- summary -----------------------------------------------------------------

fn summary_fields() -> Value {
    fields(
        serde_json::json!({
            "headline": string("The answer in one line."),
            "text": string("Optional. One to three sentences that give the answer itself."),
            "points": texts("Optional. Key points, one short line each.")
        }),
        &["headline"],
    )
}

fn build_summary(args: &Map<String, Value>) -> Result<Value, String> {
    if text(&Value::Object(args.clone()), "headline").is_empty() {
        return Err("`headline` is empty: give the answer in one line".into());
    }
    Ok(Value::Object(copy(args, &["headline", "text", "points"])))
}

// ---- steps -------------------------------------------------------------------

fn steps_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "steps": list_of(
                "The steps in the order they are done. Alternatives for a step go in `choices`, each naming its step, never inside the step.",
                serde_json::json!({
                    "label": string("What to do, in a few words."),
                    "detail": string("Optional. What helps someone do it."),
                    "time": string("Optional. When it happens, as the person gave it (Saturday, 9:00)."),
                    "duration_minutes": integer("Optional. How many whole minutes it takes.")
                }),
                &["label"],
            ),
            "choices": list_of(
                "Optional. Alternatives the person can choose between for a step.",
                serde_json::json!({
                    "step": string("The label of the step this is an option for, as given in steps."),
                    "label": string("The option's name."),
                    "detail": string("Optional. One line about it."),
                    "facts": string("Optional. Short facts about it, separated by semicolons.")
                }),
                &["step", "label"],
            ),
            "total_duration_minutes": integer("Optional. The overall time the person gave; the step durations must add up to it.")
        }),
        &["steps"],
    )
}

fn positive_minutes(value: &Value, what: &str) -> Result<u64, String> {
    value
        .as_u64()
        .filter(|minutes| *minutes > 0)
        .ok_or_else(|| format!("{what} must be a whole number greater than zero"))
}

fn build_steps(args: &Map<String, Value>) -> Result<Value, String> {
    let total = args
        .get("total_duration_minutes")
        .map(|value| positive_minutes(value, "`total_duration_minutes`"))
        .transpose()?;
    let mut steps = Vec::new();
    let mut summed = 0_u64;
    for (index, step) in records(args, "steps").iter().enumerate() {
        let label_only = step
            .as_str()
            .map(|label| serde_json::json!({"label": label}));
        let step = label_only.as_ref().unwrap_or(step);
        let mut built = copy(
            step.as_object().unwrap_or(&Map::new()),
            &["label", "detail", "time"],
        );
        if !built.contains_key("label") {
            return Err(format!("step {} has no label", index + 1));
        }
        match step.get("duration_minutes") {
            Some(value) => {
                let minutes =
                    positive_minutes(value, &format!("step {} `duration_minutes`", index + 1))?;
                summed = summed
                    .checked_add(minutes)
                    .ok_or("the step durations exceed the supported total")?;
                let duration = format!("{minutes} min");
                let time = match built.get("time").and_then(Value::as_str) {
                    Some(time) => format!("{time} · {duration}"),
                    None => duration,
                };
                built.insert("time".into(), Value::String(time));
            }
            None if total.is_some() => {
                return Err(format!(
                    "step {} has no duration, but `total_duration_minutes` is set; give every step `duration_minutes` so they add up to it",
                    index + 1
                ));
            }
            None => {}
        }
        steps.push(Value::Object(built));
    }
    if steps.is_empty() {
        return Err("`steps` is empty: give each step with its label".into());
    }
    if let Some(expected) = total
        && summed != expected
    {
        return Err(format!(
            "the step durations add up to {summed} minutes, but `total_duration_minutes` is {expected}; adjust them so they sum exactly"
        ));
    }
    for option in records(args, "choices") {
        let step = text(option, "step");
        let found = steps
            .iter()
            .position(|item| text(item, "label") == step)
            .ok_or_else(|| {
                format!(
                    "choice `{}` names step `{step}`, which is not the label of any step",
                    text(option, "label")
                )
            })?;
        let mut choice = copy(
            option.as_object().unwrap_or(&Map::new()),
            &["label", "detail"],
        );
        let facts: Vec<Value> = text(option, "facts")
            .split(';')
            .map(str::trim)
            .filter(|fact| !fact.is_empty())
            .map(|fact| Value::String(fact.into()))
            .collect();
        if !facts.is_empty() {
            choice.insert("facts".into(), Value::Array(facts));
        }
        let item = &mut steps[found];
        match item.get_mut("choices").and_then(Value::as_array_mut) {
            Some(choices) => choices.push(Value::Object(choice)),
            None => item["choices"] = Value::Array(vec![Value::Object(choice)]),
        }
    }
    let mut payload = copy(args, &["title"]);
    payload.insert("steps".into(), Value::Array(steps));
    Ok(Value::Object(payload))
}

/// A step sent as a line of text is its label; a whole number of minutes
/// written as text is that number.
fn fill_steps(fields: &mut Map<String, Value>) {
    fn minutes(value: &mut Value) {
        if let Some(n) = value
            .as_str()
            .and_then(|text| text.trim().parse::<u64>().ok())
        {
            *value = Value::from(n);
        }
    }
    if let Some(total) = fields.get_mut("total_duration_minutes") {
        minutes(total);
    }
    if let Some(steps) = fields.get_mut("steps").and_then(Value::as_array_mut) {
        for step in steps.iter_mut() {
            if let Value::String(label) = step {
                *step = serde_json::json!({"label": label.clone()});
            }
            if let Some(duration) = step.get_mut("duration_minutes") {
                minutes(duration);
            }
        }
    }
}

// ---- list --------------------------------------------------------------------

fn list_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "checkable": boolean("Optional. true when the items are to be ticked off."),
            "items": list_of(
                "The items, in the order to show them.",
                serde_json::json!({
                    "label": string("The item, in a few words."),
                    "detail": string("Optional. One line about it."),
                    "url": string("Optional. A link for it, copied exactly from a retrieved result.")
                }),
                &["label"],
            )
        }),
        &["items"],
    )
}

fn build_list(args: &Map<String, Value>) -> Result<Value, String> {
    let items: Vec<Value> = records(args, "items")
        .iter()
        .map(|item| match item {
            Value::String(label) => serde_json::json!({"label": label.trim()}),
            item => record_copy(item, &["label", "detail", "url"]),
        })
        .filter(|item| !text(item, "label").is_empty())
        .collect();
    if items.is_empty() {
        return Err("`items` is empty: give each item with its label".into());
    }
    let mut payload = copy(args, &["title", "checkable"]);
    payload.insert("items".into(), Value::Array(items));
    Ok(Value::Object(payload))
}

/// An item sent as a line of text is its label.
fn fill_list(fields: &mut Map<String, Value>) {
    if let Some(items) = fields.get_mut("items").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            if let Value::String(label) = item {
                *item = serde_json::json!({"label": label.clone()});
            }
        }
    }
}

// ---- details -----------------------------------------------------------------

fn details_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "text": string("Optional. One or two sentences shown above the facts."),
            "fields": list_of(
                "The facts, or the questions with their answers.",
                serde_json::json!({
                    "label": string("What the fact is, or the question."),
                    "detail": string("Its value, or the answer.")
                }),
                &["label", "detail"],
            )
        }),
        &["fields"],
    )
}

fn build_details(args: &Map<String, Value>) -> Result<Value, String> {
    let facts: Vec<Value> = records(args, "fields")
        .iter()
        .map(|fact| record_copy(fact, &["label", "detail"]))
        .collect();
    if facts.is_empty() {
        return Err("`fields` is empty: give each fact as {label, detail}".into());
    }
    if let Some(at) = facts
        .iter()
        .position(|fact| text(fact, "label").is_empty() || text(fact, "detail").is_empty())
    {
        return Err(format!("field {} needs both a label and a detail", at + 1));
    }
    let mut payload = copy(args, &["title", "text"]);
    payload.insert("fields".into(), Value::Array(facts));
    Ok(Value::Object(payload))
}

// ---- table -------------------------------------------------------------------

fn table_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "rows": {
                "type": "array",
                "items": {
                    "type": "string",
                    "description": "One line of the table with its cells separated by |: the first line names every column, each later line gives one row's cells in the same order. A cell may be `= expression` over what a controls card earlier in this answer publishes (= a * b)."
                },
                "description": "The table, one string per line: first the column names in one string, then one string for each row. Example: [\"Name | Value\", \"A | 1\", \"B | 2\"]."
            },
            "numeric_columns": texts("Optional. Column labels whose non-empty cells are numeric decimals. The labels must match one column exactly; numeric text is kept exactly as written. Use ASCII signs and a decimal point, without grouping commas or unit/percent suffixes; put units in the column label. Leave a missing value empty, not a dash or N/A. Non-finite values are not accepted. A cell beginning with = remains a live expression.")
        }),
        &["rows"],
    )
}

fn build_table(args: &Map<String, Value>) -> Result<Value, String> {
    let lines = table_lines(args.get("rows").and_then(Value::as_array).ok_or_else(|| {
        "`rows` must be an array of row strings or arrays of string cells".to_string()
    })?)?;
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (labels, rows, ambiguous_labels) = table_rows(&lines)?;
    if labels.len() < 2 {
        return Err(
            "every line has one cell, so this is not a table: a table's first string \
             names two or more columns separated by | (\"Name | Value\"), and each later string \
             gives one row's cells the same way (\"A | 1\"). For one value per item use \
             emit_list_card; for labels with their values use emit_details_card."
                .into(),
        );
    }
    let numeric = table_numeric_flags(
        &labels,
        &ambiguous_labels,
        &rows,
        args.get("numeric_columns"),
    )?;
    let columns: Vec<Value> = labels
        .iter()
        .zip(&numeric)
        .map(|(label, numeric)| serde_json::json!({"label": label, "numeric": numeric}))
        .collect();
    let rows: Vec<Value> = rows
        .iter()
        .map(|row| Value::Array(row.iter().map(|cell| Value::String(cell.clone())).collect()))
        .collect();
    let mut payload = copy(args, &["title"]);
    payload.insert("columns".into(), Value::Array(columns));
    payload.insert("rows".into(), Value::Array(rows));
    Ok(Value::Object(payload))
}

fn table_numeric_flags(
    labels: &[String],
    ambiguous_labels: &[String],
    rows: &[Vec<String>],
    declarations: Option<&Value>,
) -> Result<Vec<bool>, String> {
    let Some(declarations) = declarations else {
        return Ok(vec![false; labels.len()]);
    };
    let Some(declarations) = declarations.as_array() else {
        return Err("`numeric_columns` must be a list of column labels".into());
    };
    let mut numeric = vec![false; labels.len()];
    let mut seen = std::collections::BTreeSet::new();
    for declaration in declarations {
        let Some(label) = declaration
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Err("each `numeric_columns` entry must be a non-empty column label".into());
        };
        if !seen.insert(label.to_string()) {
            return Err(format!(
                "`numeric_columns` repeats `{label}`; name each column once"
            ));
        }
        if ambiguous_labels.iter().any(|ambiguous| ambiguous == label) {
            return Err(format!(
                "`numeric_columns` label `{label}` is ambiguous because the table header repeats it"
            ));
        }
        let Some(column) = labels.iter().position(|candidate| candidate == label) else {
            return Err(format!(
                "`numeric_columns` names `{label}`, which is not a table column"
            ));
        };
        for (row_index, row) in rows.iter().enumerate() {
            let cell = &row[column];
            if cell.is_empty() || cell.starts_with('=') {
                continue;
            }
            if !plain_decimal(cell) {
                return Err(format!(
                    "cell {} in numeric column `{label}` must be a plain decimal number or a `= expression`. Use ASCII signs, no grouping commas or unit suffixes, and an empty cell for a missing value. Correct the cell rather than removing its numeric declaration",
                    row_index + 1
                ));
            }
        }
        numeric[column] = true;
    }
    Ok(numeric)
}

/// A finite decimal's written grammar, without converting it to a machine
/// number: optional sign, digits, optional fractional digits, and optional
/// signed exponent. Commas, NaN and Infinity are deliberately excluded.
fn plain_decimal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut at = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let integer_start = at;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    let integer_digits = at - integer_start;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let fraction_start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if integer_digits == 0 && at == fraction_start {
            return false;
        }
    } else if integer_digits == 0 {
        return false;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent_start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == exponent_start {
            return false;
        }
    }
    at == bytes.len()
}

/// Normalize both accepted table input shapes before applying the same
/// delimiter, header, and width checks. Escaped bars remain cell content.
fn table_lines(rows: &[Value]) -> Result<Vec<String>, String> {
    if rows.iter().all(Value::is_string) {
        return Ok(rows
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect());
    }
    if !rows.iter().all(Value::is_array) {
        return Err("`rows` must use one consistent row shape".into());
    }
    rows.iter()
        .enumerate()
        .map(|(index, row)| match row {
            Value::String(line) => Ok(line.clone()),
            Value::Array(cells) => cells
                .iter()
                .enumerate()
                .map(|(cell_index, cell)| {
                    let text = match cell {
                        Value::String(text) if !text.contains(['\n', '\r']) => {
                            text.replace('|', "\\|")
                        }
                        Value::String(_) => {
                            return Err(format!(
                                "`rows[{index}][{cell_index}]` cannot contain a line break"
                            ));
                        }
                        Value::Number(number) => number.to_string(),
                        Value::Bool(value) => value.to_string(),
                        _ => {
                            return Err(format!(
                                "`rows[{index}][{cell_index}]` must be a string, number, or boolean cell"
                            ));
                        }
                    };
                    Ok(text)
                })
                .collect::<Result<Vec<_>, _>>()
                // Outer separators keep a final escaped literal bar distinct
                // from the optional trailing delimiter used by Markdown.
                .map(|mut cells| {
                    // A matrix row is data, even when every cell happens to
                    // look like a Markdown separator (`--- | ---`). Preserve
                    // that distinction across the legacy row-string parser.
                    if matrix_row_is_separator(&cells) {
                        cells[0].insert(0, '\u{e000}');
                    }
                    format!("| {} |", cells.join(" | "))
                }),
            _ => Err(format!(
                "`rows[{index}]` must be a row string or an array of string cells"
            )),
        })
        .collect()
}

fn matrix_row_is_separator(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim_matches(':');
            !cell.is_empty() && cell.chars().all(|character| character == '-')
        })
}

/// A table's column labels and rows from its row strings, header first.
/// Cells are split at `|` (outer bars optional, `\|` is a literal bar),
/// separator rows such as `---|---` are skipped, labels are unique (a
/// repeat gets ` (2)`), an empty label is `Column N`, a short row is padded,
/// and a cell's outer `**`/`__` emphasis is dropped.
type ParsedTableRows = (Vec<String>, Vec<Vec<String>>, Vec<String>);

fn table_rows(lines: &[&str]) -> Result<ParsedTableRows, String> {
    let separator = |row: &Vec<String>| {
        row.iter().all(|cell| {
            let cell = cell.trim_matches(':');
            !cell.is_empty() && cell.chars().all(|c| c == '-')
        })
    };
    let rows: Vec<Vec<String>> = lines
        .iter()
        .flat_map(|row| row.lines())
        .map(str::trim)
        .filter(|row| !row.is_empty())
        .map(markdown_cells)
        .filter_map(|mut row| {
            let matrix_data_row = row
                .first_mut()
                .is_some_and(|cell| cell.starts_with('\u{e000}'));
            if matrix_data_row {
                row[0] = row[0].trim_start_matches('\u{e000}').to_string();
            }
            (matrix_data_row || !separator(&row)).then_some(row)
        })
        .collect();
    if rows.len() < 2 {
        return Err("`rows` needs the header row first, then at least one row".into());
    }
    let (labels, ambiguous_labels) = unique_table_labels(&rows[0]);
    let mut data = Vec::new();
    for (at, mut row) in rows.into_iter().skip(1).enumerate() {
        if row.len() > labels.len() {
            return Err(format!(
                "row {} has {} cells but the header has {}",
                at + 1,
                row.len(),
                labels.len()
            ));
        }
        row.resize(labels.len(), String::new());
        data.push(row);
    }
    if data.is_empty() {
        return Err("`rows` has a header but no rows".into());
    }
    Ok((labels, data, ambiguous_labels))
}

fn unique_table_labels(headers: &[String]) -> (Vec<String>, Vec<String>) {
    let mut labels = Vec::with_capacity(headers.len());
    let mut ambiguous_labels = Vec::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for (at, label) in headers.iter().enumerate() {
        let base = if label.is_empty() {
            format!("Column {}", at + 1)
        } else {
            label.clone()
        };
        *counts.entry(base.clone()).or_default() += 1;
        let mut unique = base.clone();
        let mut n = 2;
        while labels.contains(&unique) {
            unique = format!("{base} ({n})");
            n += 1;
        }
        labels.push(unique);
    }
    for (label, count) in counts {
        if count > 1 {
            ambiguous_labels.push(label);
        }
    }
    (labels, ambiguous_labels)
}

fn markdown_cells(line: &str) -> Vec<String> {
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut cell)),
            other => cell.push(other),
        }
    }
    cells.push(cell);
    cells
        .into_iter()
        .map(|cell| {
            let cell = cell.trim();
            ["**", "__"]
                .iter()
                .find_map(|mark| {
                    cell.strip_prefix(mark)
                        .and_then(|inner| inner.strip_suffix(mark))
                })
                .unwrap_or(cell)
                .trim()
                .to_string()
        })
        .collect()
}

/// A table's rows are lines: a table sent as one text is its lines, and a
/// matrix of cells is written as the row strings the builder reads.
fn fill_table(fields: &mut Map<String, Value>) {
    if let Some(Value::String(text)) = fields.get("rows") {
        let rows: Vec<Value> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| Value::String(line.to_string()))
            .collect();
        if !rows.is_empty() {
            fields.insert("rows".into(), Value::Array(rows));
        }
    }
    if let Some(rows) = fields.get_mut("rows").and_then(Value::as_array_mut)
        && let Ok(lines) = table_lines(rows)
    {
        *rows = lines.into_iter().map(Value::String).collect();
    }
}

// ---- comparison --------------------------------------------------------------

fn comparison_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "options": list_of(
                "Each option being compared, two to four, in the order you discuss them.",
                serde_json::json!({
                    "name": string("The option's name."),
                    "verdict": string("Optional. One line on when this option is the better choice."),
                    "strengths": string("Optional. Its strengths, separated by semicolons."),
                    "weaknesses": string("Optional. Its weaknesses, separated by semicolons.")
                }),
                &["name"],
            ),
            "aspects": list_of(
                "Optional. What the options differ on: one entry for each aspect of each option.",
                serde_json::json!({
                    "aspect": string("What is compared, such as Effort."),
                    "option": string("The name of the option this value is for, as given in options."),
                    "value": string("That option's value for the aspect.")
                }),
                &["aspect", "option", "value"],
            ),
            "recommendation": string("Optional. Which option you recommend and why, in one or two sentences.")
        }),
        &["options"],
    )
}

fn build_comparison(args: &Map<String, Value>) -> Result<Value, String> {
    let points = |text: Option<&Value>| -> Vec<Value> {
        text.and_then(Value::as_str)
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|point| !point.is_empty())
            .map(|point| Value::String(point.to_string()))
            .collect()
    };
    let mut options = Vec::new();
    for (at, option) in records(args, "options").iter().enumerate() {
        let name = text(option, "name");
        if name.is_empty() {
            return Err(format!("options[{at}].name is empty"));
        }
        let mut built = serde_json::json!({"name": name});
        if !text(option, "verdict").is_empty() {
            built["verdict"] = Value::String(text(option, "verdict").into());
        }
        for key in ["strengths", "weaknesses"] {
            let listed = points(option.get(key));
            if !listed.is_empty() {
                built[key] = Value::Array(listed);
            }
        }
        options.push(built);
    }
    if options.len() < 2 {
        return Err("a comparison needs at least two options; give each one's name".into());
    }
    let names: Vec<String> = options
        .iter()
        .map(|option| text(option, "name").to_lowercase())
        .collect();
    let mut aspects: Vec<(String, Vec<Value>)> = Vec::new();
    for (at, cell) in records(args, "aspects").iter().enumerate() {
        let (aspect, option) = (text(cell, "aspect"), text(cell, "option"));
        if aspect.is_empty() {
            return Err(format!("aspects[{at}].aspect is empty"));
        }
        let Some(column) = names.iter().position(|name| *name == option.to_lowercase()) else {
            return Err(format!(
                "aspects[{at}].option is `{option}`, which is not the name of an option"
            ));
        };
        let row = match aspects.iter().position(|(label, _)| label == aspect) {
            Some(row) => row,
            None => {
                aspects.push((
                    aspect.to_string(),
                    vec![Value::String(String::new()); names.len()],
                ));
                aspects.len() - 1
            }
        };
        aspects[row].1[column] = Value::String(text(cell, "value").into());
    }
    let mut payload = copy(args, &["title", "recommendation"]);
    payload.insert("options".into(), Value::Array(options));
    if !aspects.is_empty() {
        payload.insert(
            "aspects".into(),
            Value::Array(
                aspects
                    .into_iter()
                    .map(|(label, values)| serde_json::json!({"label": label, "values": values}))
                    .collect(),
            ),
        );
    }
    Ok(Value::Object(payload))
}

/// Strengths and weaknesses are texts this form splits at semicolons; a
/// list of them is the same points.
fn fill_comparison(fields: &mut Map<String, Value>) {
    let Some(options) = fields.get_mut("options").and_then(Value::as_array_mut) else {
        return;
    };
    for option in options.iter_mut().filter_map(Value::as_object_mut) {
        for key in ["strengths", "weaknesses"] {
            let joined = option
                .get(key)
                .and_then(Value::as_array)
                .and_then(|points| {
                    points
                        .iter()
                        .map(Value::as_str)
                        .collect::<Option<Vec<&str>>>()
                });
            if let Some(points) = joined {
                option.insert(key.into(), Value::String(points.join("; ")));
            }
        }
    }
}

// ---- stats -------------------------------------------------------------------

fn stats_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "readings": list_of(
                "Each figure, in the order to show them.",
                serde_json::json!({
                    "label": string("What the figure is."),
                    "follows": string("Optional. An expression over the names a controls card earlier in this answer publishes (a * b); the figure is computed from it and changes as the person changes them."),
                    "value": string("Optional when follows is given. The figure, or words for a reading that is not a number."),
                    "unit": string("Optional. Its unit, for a number, as a symbol or a short name."),
                    "change": string("Optional. How it moved, as given (+3%).")
                }),
                &["label"],
            )
        }),
        &["readings"],
    )
}

fn build_stats(args: &Map<String, Value>) -> Result<Value, String> {
    let mut readings = Vec::new();
    let mut labels = std::collections::BTreeSet::new();
    for (at, reading) in records(args, "readings").iter().enumerate() {
        let label = text(reading, "label");
        if label.is_empty() {
            return Err(format!("readings[{at}].label is empty"));
        }
        if !labels.insert(label.to_lowercase()) {
            return Err(format!(
                "two readings are labelled `{label}`; give each its own"
            ));
        }
        // A figure is shown as written ("13,400", "light rain"); a number
        // sent as a number stays one.
        let mut built = record_copy(reading, &["label", "value", "unit", "change"]);
        // A figure that follows published values is bound when the card is
        // recorded (docs/design/98 §2).
        let follows = text(reading, "follows");
        if !follows.is_empty() {
            built["value"] = Value::String(format!("= {follows}"));
        }
        if built.get("value").is_none() {
            return Err(format!("readings[{at}] needs value or follows"));
        }
        readings.push(built);
    }
    if readings.is_empty() {
        return Err("`readings` is empty: give each figure with its label and value".into());
    }
    let mut payload = copy(args, &["title"]);
    payload.insert("readings".into(), Value::Array(readings));
    Ok(Value::Object(payload))
}

/// Readings sent as one label-to-value object are the same readings as a
/// list.
fn fill_stats(fields: &mut Map<String, Value>) {
    let Some(Value::Object(readings)) = fields.get("readings") else {
        return;
    };
    let listed = readings
        .iter()
        .map(|(label, value)| {
            let value = match value {
                Value::String(value) => value.clone(),
                Value::Number(value) => value.to_string(),
                _ => return None,
            };
            Some(serde_json::json!({"label": label, "value": value}))
        })
        .collect::<Option<Vec<_>>>();
    if let Some(listed) = listed {
        fields.insert("readings".into(), Value::Array(listed));
    }
}

// ---- chart -------------------------------------------------------------------

fn chart_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "chart_type": one_of(&["line", "bar", "area"], "How to draw it: line or area for a value over an ordered x, bar for values across categories."),
            "x_scale": one_of(&["category", "number", "time"], "Optional. category (default) keeps equal spacing and exact labels; number spaces finite numeric x values by magnitude; time spaces RFC3339 timestamps by elapsed time."),
            "x_label": string("Optional. What the x axis measures."),
            "y_label": string("Optional. What the y axis measures, with its unit."),
            "accessible_summary": string("Optional. One sentence saying what the chart shows."),
            "follows": string("Optional. The names of series a controls card earlier in this answer publishes, separated by commas; the chart draws them and changes as the person changes the controls. Leave data out then."),
            "data": list_of(
                "Optional when follows is given. Every point of every series. Each series is one measure; chart unlike measures separately.",
                serde_json::json!({
                    "series": string("The name of the series this point belongs to."),
                    "x": string("The x value: a category or a number, as text."),
                    "y": {"type": "number", "description": "Optional. The measured value. Omit for a missing observation (leaves a gap)."}
                }),
                &["series", "x"],
            )
        }),
        &["chart_type"],
    )
}

fn build_chart(args: &Map<String, Value>) -> Result<Value, String> {
    let axis = args
        .get("x_scale")
        .and_then(Value::as_str)
        .unwrap_or("category");
    if !["category", "number", "time"].contains(&axis) {
        return Err("`x_scale` must be category, number or time".into());
    }
    let mut series: Vec<(String, Vec<Value>)> = Vec::new();
    for point in records(args, "data") {
        let x = scalar(&point["x"]);
        let valid = match axis {
            "number" => {
                x.as_f64().filter(|value| value.is_finite()).is_some()
                    || x.as_str().is_some_and(|value| {
                        serde_json::from_str::<Value>(value)
                            .ok()
                            .and_then(|value| value.as_f64())
                            .is_some_and(f64::is_finite)
                    })
            }
            "time" => x
                .as_str()
                .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_ok()),
            _ => true,
        };
        if !valid {
            return Err(format!("each `x` must match the explicit `{axis}` scale"));
        }

        let name = text(point, "series").to_string();
        let y = match point.get("y").cloned().unwrap_or(Value::Null) {
            number @ (Value::Number(_) | Value::Null) => number,
            _ => {
                return Err(format!(
                    "each point's `y` must be a number or null; series `{name}` at x `{}` has none",
                    scalar(&point["x"])
                ));
            }
        };
        let entry = serde_json::json!({"x": scalar(&point["x"]), "y": y});
        match series.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, points)) => {
                if points.iter().any(|existing| existing["x"] == entry["x"]) {
                    return Err("each series must have at most one observation per x value".into());
                }
                points.push(entry);
            }
            None => series.push((name, vec![entry])),
        }
    }
    // Series that follow a controls card's are filled when the card is
    // recorded (docs/design/98 §2).
    let followed: Vec<String> = text(&Value::Object(args.clone()), "follows")
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();
    if series.is_empty() && followed.is_empty() {
        return Err("give `data` (every point, each with its series, x and y) or `follows`".into());
    }
    let mut payload = copy(
        args,
        &[
            "title",
            "chart_type",
            "x_scale",
            "x_label",
            "y_label",
            "accessible_summary",
        ],
    );
    if !payload.contains_key("accessible_summary") {
        payload.insert(
            "accessible_summary".into(),
            Value::String(chart_summary(args, &series)),
        );
    }
    let mut drawn: Vec<Value> = series
        .into_iter()
        .map(|(name, points)| serde_json::json!({"name": name, "points": points}))
        .collect();
    drawn.extend(
        followed
            .iter()
            .map(|name| serde_json::json!({"name": name, "follows": name, "points": []})),
    );
    payload.insert("series".into(), Value::Array(drawn));
    Ok(Value::Object(payload))
}

/// What a chart shows, in words, from its own data: each series with its
/// first and last point. It is the chart's text form on a surface that
/// draws none.
fn chart_summary(args: &Map<String, Value>, series: &[(String, Vec<Value>)]) -> String {
    let shown = |value: &Value| {
        value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_string)
    };
    let lines: Vec<String> = series
        .iter()
        .filter_map(|(name, points)| {
            let (first, last) = (points.first()?, points.last()?);
            let name = if name.is_empty() { "Series" } else { name };
            Some(if points.len() == 1 {
                format!("{name}: {} at {}", shown(&first["y"]), shown(&first["x"]))
            } else {
                format!(
                    "{name}: {} at {} to {} at {}, {} points",
                    shown(&first["y"]),
                    shown(&first["x"]),
                    shown(&last["y"]),
                    shown(&last["x"]),
                    points.len()
                )
            })
        })
        .collect();
    let title = text(&Value::Object(args.clone()), "title").to_string();
    let title = if title.is_empty() {
        "Chart".into()
    } else {
        title
    };
    format!("{title}. {}.", lines.join("; "))
}

// ---- callout -----------------------------------------------------------------

fn callout_fields() -> Value {
    fields(
        serde_json::json!({
            "tone": one_of(&["tip", "note", "warning", "important"], "tip: a helpful suggestion; note: worth knowing; warning: a risk; important: must not be missed."),
            "title": string(TITLE),
            "text": string("The note itself, in one to three sentences.")
        }),
        &["tone", "text"],
    )
}

fn build_callout(args: &Map<String, Value>) -> Result<Value, String> {
    let payload = copy(args, &["tone", "title", "text"]);
    if !payload.contains_key("text") {
        return Err("`text` is empty".into());
    }
    Ok(Value::Object(payload))
}

// ---- sources -----------------------------------------------------------------

fn sources_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "sources": list_of(
                "The retrieved sources, in order, each with what it found.",
                serde_json::json!({
                    "title": string("Copy the source title from the retrieved result."),
                    "url": string("Copy the exact source URL from retrieved evidence, not a guessed homepage or search URL."),
                    "finding": string("What this source found that answers the request, in one sentence, supported by its text."),
                    "source_name": string("Optional. The publisher, only when the retrieved result names it."),
                    "published_at": string("Optional. The publication date, only when the retrieved result gives it.")
                }),
                &["title", "url", "finding"],
            )
        }),
        &["sources"],
    )
}

fn build_sources(args: &Map<String, Value>) -> Result<Value, String> {
    let sources: Vec<Value> = records(args, "sources")
        .iter()
        .map(|source| {
            record_copy(
                source,
                &["title", "url", "finding", "source_name", "published_at"],
            )
        })
        .collect();
    if sources.is_empty() {
        return Err("`sources` is empty: give each retrieved source with its finding".into());
    }
    let mut payload = copy(args, &["title"]);
    payload.insert("sources".into(), Value::Array(sources));
    Ok(Value::Object(payload))
}

// ---- calendar ----------------------------------------------------------------

fn calendar_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "events": list_of(
                "The events, each on its date.",
                serde_json::json!({
                    "title": string("What the event is."),
                    "date": string("Its date as YYYY-MM-DD."),
                    "start": string("Optional. Its start time as HH:MM, 24-hour."),
                    "end": string("Optional. Its end time as HH:MM, 24-hour."),
                    "location": string("Optional. Where it is."),
                    "detail": string("Optional. One line about it.")
                }),
                &["title", "date"],
            )
        }),
        &["events"],
    )
}

fn build_calendar(args: &Map<String, Value>) -> Result<Value, String> {
    let mut events: Vec<Value> = records(args, "events")
        .iter()
        .map(|event| {
            record_copy(
                event,
                &["title", "date", "start", "end", "location", "detail"],
            )
        })
        .collect();
    if events.is_empty() {
        return Err("`events` is empty: give each event with its title and date".into());
    }
    events.sort_by(|a, b| {
        (text(a, "date"), text(a, "start")).cmp(&(text(b, "date"), text(b, "start")))
    });
    let mut payload = copy(args, &["title"]);
    payload.insert("events".into(), Value::Array(events));
    Ok(Value::Object(payload))
}

// ---- map ---------------------------------------------------------------------

fn map_fields() -> Value {
    fields(
        serde_json::json!({
            "connect": one_of(&["none", "route", "area"], "How the places connect: none for separate places, route for a path through them in the order given (two or more), area for the corners of one area in order around it (three or more)."),
            "title": string(TITLE),
            "places": list_of(
                "The places, in order for a route or an area.",
                serde_json::json!({
                    "label": string("The place's name, short."),
                    "position": string("Its latitude and longitude in decimal degrees, latitude first, separated by a comma (12.5, -45.25). South and west are negative."),
                    "detail": string("Optional. One line about it.")
                }),
                &["label", "position"],
            )
        }),
        &["connect", "places"],
    )
}

/// Decimal degrees written `latitude, longitude`, each in range.
fn position(text: &str) -> Option<(f64, f64)> {
    let (latitude, longitude) = text.split_once(',')?;
    let latitude: f64 = latitude.trim().parse().ok()?;
    let longitude: f64 = longitude.trim().parse().ok()?;
    (latitude.is_finite()
        && longitude.is_finite()
        && latitude.abs() <= 90.0
        && longitude.abs() <= 180.0)
        .then_some((latitude, longitude))
}

fn build_map(args: &Map<String, Value>) -> Result<Value, String> {
    let mut places = Vec::new();
    for (at, place) in records(args, "places").iter().enumerate() {
        if text(place, "label").is_empty() {
            return Err(format!("places[{at}].label is empty"));
        }
        let Some((latitude, longitude)) = position(text(place, "position")) else {
            return Err(format!(
                "places[{at}].position must be latitude and longitude in decimal degrees, latitude first: \"12.5, -45.25\""
            ));
        };
        let mut built = copy(
            place.as_object().unwrap_or(&Map::new()),
            &["label", "detail"],
        );
        built.insert("latitude".into(), Value::from(latitude));
        built.insert("longitude".into(), Value::from(longitude));
        places.push(Value::Object(built));
    }
    if places.is_empty() {
        return Err("`places` is empty: give each place with its label and position".into());
    }
    let mut payload = copy(args, &["title", "connect"]);
    payload.insert("places".into(), Value::Array(places));
    Ok(Value::Object(payload))
}

/// A place sent with numeric `latitude` and `longitude` is the same place
/// with its `position`.
fn fill_map(fields: &mut Map<String, Value>) {
    let Some(Value::Array(places)) = fields.get_mut("places") else {
        return;
    };
    for place in places.iter_mut().filter_map(Value::as_object_mut) {
        if place.contains_key("position") {
            continue;
        }
        let degrees = |key: &str| match place.get(key) {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        };
        if let (Some(latitude), Some(longitude)) = (degrees("latitude"), degrees("longitude")) {
            place.remove("latitude");
            place.remove("longitude");
            place.insert(
                "position".into(),
                Value::String(format!("{latitude}, {longitude}")),
            );
        }
    }
}

// ---- controls ----------------------------------------------------------------

fn number(description: &str) -> Value {
    serde_json::json!({"type": "number", "description": description})
}

const EXPRESSION: &str = "How it is computed from the inputs and earlier outputs, by their names: numbers, + - * / % ^, parentheses, comparisons (< <= > >= == != give 1 or 0), and if(condition, then, else), min, max, round(x, places), floor, ceil, abs, sqrt, log, exp, sum, avg. A toggle reads 1 when on, a date as a count of days. Example: amount * (1 + rate / 100) / count.";

fn controls_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "inputs": list_of(
                "What the person sets, with the values the card starts from.",
                serde_json::json!({
                    "name": string("The name expressions use for it: letters, digits and _, starting with a letter (amount, rate_percent)."),
                    "kind": one_of(CONTROL_KINDS, "number for any number, range for a number on a slider between min and max, choice for one of the options, toggle for on or off, date for a day."),
                    "value": string("Its value in the example, as text: a number for number, range and choice (a choice's value is one of its options' values), true or false for a toggle, YYYY-MM-DD for a date."),
                    "min": number("Optional for number, needed for range. The smallest value."),
                    "max": number("Optional for number, needed for range. The largest value."),
                    "step": number("Optional. How much the value moves at a time."),
                    "options": string("Optional, needed for choice. The options separated by semicolons, each a number or a name with its number: Low=10; Middle=15; High=20."),
                    "unit": string("Optional. The unit shown beside it (the unit's symbol or name)."),
                    "label": string("What it is called on the card.")
                }),
                &["name", "kind", "value", "label"],
            ),
            "outputs": list_of(
                "The results, computed in this order, each shown as the person changes the inputs.",
                serde_json::json!({
                    "name": string("The name later outputs use for it."),
                    "expression": string(EXPRESSION),
                    "vary": string("Optional. An input's name: the output is also computed across a range of that input, as a series a later chart can follow."),
                    "from": string("Optional, needed with vary. Where the range starts: a number or an expression over the inputs."),
                    "to": string("Optional, needed with vary. Where the range ends: a number or an expression over the inputs."),
                    "step": string("Optional, needed with vary. How far apart the points are: a number or an expression over the inputs."),
                    "decimals": integer("Optional. How many decimal places it is shown with; whole numbers when left out."),
                    "unit": string("Optional. The unit shown beside it."),
                    "label": string("What it is called on the card.")
                }),
                &["name", "expression", "label"],
            )
        }),
        &["inputs", "outputs"],
    )
}

/// The kinds of input, as the card tool offers them.
pub const CONTROL_KINDS: &[&str] = vak_delivery::controls::INPUT_KINDS;

/// A choice's options, written `Low=10; Usual=15` or `10; 15`.
fn control_options(text: &str) -> Result<Vec<Value>, String> {
    text.split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (label, number) = part.rsplit_once('=').unwrap_or((part, part));
            let value: f64 = number
                .trim()
                .parse()
                .map_err(|_| format!("option `{part}` needs a number: `Name=10` or `10`"))?;
            Ok(serde_json::json!({"label": label.trim(), "value": value}))
        })
        .collect()
}

fn build_controls(args: &Map<String, Value>) -> Result<Value, String> {
    let mut inputs = Vec::new();
    for (at, input) in records(args, "inputs").iter().enumerate() {
        let mut built = copy(
            input.as_object().unwrap_or(&Map::new()),
            &["name", "label", "kind", "unit", "min", "max", "step"],
        );
        let kind = text(input, "kind");
        let written = text(input, "value");
        let value = match kind {
            "toggle" => match written {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => {
                    return Err(format!(
                        "inputs[{at}].value must be true or false for a toggle"
                    ));
                }
            },
            "date" => Value::String(written.to_string()),
            _ => {
                let options = match kind {
                    "choice" => control_options(text(input, "options"))
                        .map_err(|error| format!("inputs[{at}].options: {error}"))?,
                    _ => Vec::new(),
                };
                // A choice's value may name its option.
                let named = options
                    .iter()
                    .find(|option| option["label"].as_str() == Some(written))
                    .and_then(|option| option["value"].as_f64());
                let number = match named {
                    Some(number) => number,
                    None => written.parse::<f64>().map_err(|_| {
                        format!("inputs[{at}].value must be a number for a {kind}, not `{written}`")
                    })?,
                };
                if kind == "choice" {
                    built.insert("options".into(), Value::Array(options));
                }
                Value::from(number)
            }
        };
        built.insert("value".into(), value);
        inputs.push(Value::Object(built));
    }
    let mut outputs = Vec::new();
    for (at, output) in records(args, "outputs").iter().enumerate() {
        let mut built = copy(
            output.as_object().unwrap_or(&Map::new()),
            &[
                "name",
                "label",
                "expression",
                "unit",
                "vary",
                "from",
                "to",
                "step",
            ],
        );
        let decimals = match output.get("decimals") {
            None => 0,
            Some(value) => value
                .as_u64()
                .filter(|places| *places <= 12)
                .ok_or_else(|| {
                    format!("outputs[{at}].decimals must be a whole number from 0 to 12")
                })?,
        };
        built.insert("decimals".into(), Value::from(decimals));
        outputs.push(Value::Object(built));
    }
    let mut payload = copy(args, &["title"]);
    payload.insert("inputs".into(), Value::Array(inputs));
    payload.insert("outputs".into(), Value::Array(outputs));
    let mut payload = Value::Object(payload);
    // What each output comes to with the values the card starts from, as
    // the runtime computes it: the card's worked example, and its text form.
    let computed = vak_delivery::controls::compute(&payload, &std::collections::BTreeMap::new())?;
    if let Some(outputs) = payload["outputs"].as_array_mut() {
        for (output, (_, value)) in outputs.iter_mut().zip(computed) {
            let decimals = output["decimals"].as_u64().unwrap_or(0);
            let scale = 10_f64.powi(decimals as i32);
            output["equals"] = vak_delivery::controls::number((value * scale).round() / scale);
        }
    }
    Ok(payload)
}

// ---- link and media ----------------------------------------------------------

fn link_fields() -> Value {
    fields(
        serde_json::json!({
            "url": string("The page's URL, copied exactly."),
            "title": string("Optional. The page's title."),
            "description": string("Optional. A short description of the page."),
            "site_name": string("Optional. The site's name."),
            "image_url": string("Optional. A thumbnail image for the page.")
        }),
        &["url"],
    )
}

/// A link given no title is titled by the site it points at.
fn build_link(args: &Map<String, Value>) -> Result<Value, String> {
    let mut card = copy(
        args,
        &["url", "title", "description", "site_name", "image_url"],
    );
    let url = card
        .get("url")
        .and_then(Value::as_str)
        .ok_or("give `url`: the page's URL")?
        .to_string();
    if !card.contains_key("title") {
        let site = url
            .split_once("://")
            .map_or(url.as_str(), |(_, rest)| rest)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
            .to_string();
        card.insert(
            "title".into(),
            Value::String(if site.is_empty() { url } else { site }),
        );
    }
    Ok(Value::Object(card))
}

fn media_fields() -> Value {
    fields(
        serde_json::json!({
            "format": one_of(&["image", "video", "audio"], "What the file is."),
            "url": string("The file's URL or workspace path."),
            "alt": string("Optional. What it shows or says, in words."),
            "title": string(TITLE)
        }),
        &["format", "url"],
    )
}

/// What a media card shows is its `alt`, else its title, else the file's
/// own name.
fn build_media(args: &Map<String, Value>) -> Result<Value, String> {
    let mut card = copy(args, &["format", "url", "alt", "title"]);
    let url = card
        .get("url")
        .and_then(Value::as_str)
        .ok_or("give `url`: the file's URL or workspace path")?
        .to_string();
    if !card.contains_key("alt") {
        let alt = card
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| address_name(&url))
            .ok_or("give `alt`: what the file shows, in words")?;
        card.insert("alt".into(), Value::String(alt));
    }
    Ok(Value::Object(card))
}

/// The last path segment of an address, without query or fragment.
fn address_name(address: &str) -> Option<String> {
    address
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

// ---- previews ----------------------------------------------------------------

// A preview shows a file or markup, and each is its own form with the one
// field it needs required: "give this or that" is prose no schema keyword
// every provider carries can enforce (refused twice live, 2026-10-13). A
// preview's isolation is the client's decision, so the card keeps no field
// that could widen its frame.
fn file_preview_fields() -> Value {
    fields(
        serde_json::json!({
            "artifact_path": string("The workspace path of the HTML or UI file to preview."),
            "title": string(TITLE),
            "status": string("Optional. The state of the work, in a word.")
        }),
        &["artifact_path"],
    )
}

fn build_file_preview(args: &Map<String, Value>) -> Result<Value, String> {
    Ok(Value::Object(copy(
        args,
        &["artifact_path", "title", "status"],
    )))
}

fn markup_preview_fields() -> Value {
    fields(
        serde_json::json!({
            "html": string("The HTML markup to preview, written in full."),
            "libraries": {
                "type": "array",
                "description": "The libraries the markup draws with, put into its frame with no network; an empty list when it uses none. diagram: Mermaid, each diagram's source written in <pre class=\"mermaid\">...</pre>.",
                "items": {"type": "string", "enum": vak_delivery::skills::MARKUP_LIBRARIES}
            },
            "title": string(TITLE),
            "status": string("Optional. The state of the work, in a word.")
        }),
        &["html", "libraries"],
    )
}

fn build_markup_preview(args: &Map<String, Value>) -> Result<Value, String> {
    Ok(Value::Object(copy(
        args,
        &["html", "libraries", "title", "status"],
    )))
}

// ---- code results ------------------------------------------------------------

fn diff_fields() -> Value {
    fields(
        serde_json::json!({
            "paths": texts("Optional. The changed files to show, as the calls that wrote them named them; every file this turn changed when left out.")
        }),
        &[],
    )
}

/// `files` is never the model's: the loop fills it from the files this
/// turn changed before the card is built.
fn build_diff(args: &Map<String, Value>) -> Result<Value, String> {
    if records(args, "files").is_empty() {
        return Err("no file was changed in this turn".into());
    }
    for file in records(args, "files") {
        for key in ["additions", "deletions"] {
            if file[key].as_u64().is_none() {
                return Err(format!("`{key}` must be a whole number, 0 or more"));
            }
        }
    }
    Ok(Value::Object(copy(args, &["files"])))
}

fn test_report_fields() -> Value {
    fields(
        serde_json::json!({
            "command": string("The command that ran the tests, exactly as a call in this conversation ran it. Its exit code and duration are taken from that call."),
            "suite_name": string("Optional. The suite's name."),
            "tests": list_of(
                "One entry per test that ran.",
                serde_json::json!({
                    "name": string("The test's name."),
                    "status": one_of(&["passed", "failed", "skipped"], "How the test ended."),
                    "duration_ms": integer("Optional. How long it took, in milliseconds."),
                    "message": string("Optional. The failure or skip message.")
                }),
                &["name", "status"],
            )
        }),
        &["command", "tests"],
    )
}

fn build_test_report(args: &Map<String, Value>) -> Result<Value, String> {
    if records(args, "tests").iter().any(|test| {
        test.get("duration_ms")
            .is_some_and(|ms| ms.as_u64().is_none())
    }) {
        return Err("`duration_ms` must be a whole number, 0 or more".into());
    }
    // How the run ended is the loop's, from the call that ran `command`.
    if !args.get("duration_ms").is_some_and(Value::is_u64) {
        return Err("no call in this conversation ran this command".into());
    }
    let tests = records(args, "tests");
    let all_passed = !tests.is_empty() && tests.iter().all(|test| text(test, "status") == "passed");
    match args.get("exit_code").and_then(Value::as_i64) {
        Some(0) => {}
        Some(code) if all_passed => {
            return Err(format!(
                "the command exited {code}, so its tests did not all pass: report the ones that failed"
            ));
        }
        None if all_passed => {
            return Err(
                "the command was stopped before it finished, so its tests did not all pass: report what ran"
                    .into(),
            );
        }
        _ => {}
    }
    Ok(Value::Object(copy(
        args,
        &["command", "suite_name", "tests", "exit_code", "duration_ms"],
    )))
}

fn terminal_fields() -> Value {
    fields(
        serde_json::json!({
            "command": string("The command exactly as a call in this conversation ran it. What it printed is taken from that call.")
        }),
        &["command"],
    )
}

/// `output` is never the model's: the loop fills it from the call that ran
/// `command` before the card is built.
fn build_terminal(args: &Map<String, Value>) -> Result<Value, String> {
    if !args.get("output").is_some_and(Value::is_string) {
        return Err("no call in this conversation ran this command".into());
    }
    let mut card = copy(args, &["command", "exit_code", "duration_ms"]);
    card.insert("output".into(), args["output"].clone());
    Ok(Value::Object(card))
}

fn fill_nothing(_: &mut Map<String, Value>) {}

// ---- the forms ---------------------------------------------------------------

const fn shape(
    form: &'static str,
    name: &'static str,
    description: &'static str,
    fields: fn() -> Value,
    build: fn(&Map<String, Value>) -> Result<Value, String>,
    fill: fn(&mut Map<String, Value>),
) -> CardShape {
    CardShape {
        form,
        name,
        description,
        fill,
        output_of_command: None,
        changes_made: None,
        data_of_call: None,
        grounded_in: None,
        fields,
        build,
        followable: &[],
    }
}

const FROM: &str = "The ability that produced the selected call, by its name. Vak checks this against the recorded call.";
const CALL_ID: &str = "The exact recorded call ID whose result supplies this card. This card is an immutable snapshot; to refresh it, run the ability again and make a new card with that call's ID.";
const LIST: &str = "Optional. Where the records are in its answer: a key, or keys joined by dots (rows, result.items). Leave it out when the answer is the list itself.";

/// A record's value as a table cell: text as written, numbers and truth
/// values as they read, nothing for a missing value, anything else as its
/// JSON. A line break reads as a space, since a cell is one line.
fn cell_of(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.replace(['\n', '\r'], " "),
        Some(other @ (Value::Number(_) | Value::Bool(_))) => other.to_string(),
        Some(other) => other.to_string(),
    }
}

fn records_as_objects(records: &[Value]) -> Result<Vec<&Map<String, Value>>, String> {
    if records.is_empty() {
        return Err("the list in the ability's answer is empty: there is nothing to show".into());
    }
    records
        .iter()
        .enumerate()
        .map(|(at, record)| {
            record.as_object().ok_or_else(|| {
                format!("record {at} of the ability's answer is not an object with keys")
            })
        })
        .collect()
}

fn ability_table_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "from": string(FROM),
            "call_id": string(CALL_ID),
            "list": string(LIST),
            "columns": texts("Optional. The keys of a record to show as columns, in order. Every key of the first record when left out."),
            "numeric_columns": texts("Optional. Column labels whose non-empty cells are numeric decimals. The labels must match one column exactly; numeric text is kept exactly as written. Use ASCII signs and a decimal point, without grouping commas or unit/percent suffixes; put units in the column label. Leave a missing value empty, not a dash or N/A. Non-finite values are not accepted. A cell beginning with = remains a live expression.")
        }),
        &["from", "call_id"],
    )
}

/// The table's rows from an ability's records: the header first, then one
/// row of cells per record, every value the record's own.
fn fill_table_from(args: &mut Map<String, Value>, records: &[Value]) -> Result<(), String> {
    let objects = records_as_objects(records)?;
    let named: Vec<String> = args
        .get("columns")
        .and_then(Value::as_array)
        .map(|columns| {
            columns
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|column| !column.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let columns = if named.is_empty() {
        // A result can be heterogeneous. Infer across all recorded rows so
        // a later-only field cannot silently disappear from the answer.
        objects
            .iter()
            .flat_map(|record| record.keys().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    } else {
        named
    };
    let (labels, _) = unique_table_labels(&columns);
    if let Some(missing) = columns.iter().find(|column| {
        !objects
            .iter()
            .any(|record| record.contains_key(column.as_str()))
    }) {
        let keys: Vec<String> = objects[0].keys().map(|key| format!("`{key}`")).collect();
        return Err(format!(
            "no record has the key `{missing}`; a record's keys are {}",
            keys.join(", ")
        ));
    }
    let mut numeric_columns: Vec<String> = match args.get("numeric_columns") {
        None => Vec::new(),
        Some(Value::Array(declarations)) => declarations
            .iter()
            .map(|declaration| {
                declaration
                    .as_str()
                    .map(str::trim)
                    .filter(|label| !label.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        "each `numeric_columns` entry must be a non-empty column label".to_string()
                    })
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("`numeric_columns` must be a list of column labels".into()),
    };
    for (index, column) in columns.iter().enumerate() {
        let mut saw_number = false;
        let all_nonempty_numbers = objects.iter().all(|record| match record.get(column) {
            None | Some(Value::Null) => true,
            Some(Value::Number(_)) => {
                saw_number = true;
                true
            }
            Some(Value::String(value)) if value.is_empty() => true,
            Some(_) => false,
        });
        let label = &labels[index];
        if all_nonempty_numbers
            && saw_number
            && !numeric_columns.iter().any(|existing| existing == label)
        {
            numeric_columns.push(label.clone());
        }
    }
    if !numeric_columns.is_empty() || args.contains_key("numeric_columns") {
        args.insert(
            "numeric_columns".into(),
            Value::Array(numeric_columns.into_iter().map(Value::String).collect()),
        );
    }
    let mut rows = vec![Value::Array(
        columns
            .iter()
            .map(|column| Value::String(column.clone()))
            .collect(),
    )];
    rows.extend(objects.iter().map(|record| {
        Value::Array(
            columns
                .iter()
                .map(|column| Value::String(cell_of(record.get(column))))
                .collect(),
        )
    }));
    args.insert("rows".into(), Value::Array(rows));
    Ok(())
}

fn build_ability_table(args: &Map<String, Value>) -> Result<Value, String> {
    if !args.contains_key("rows") {
        return Err(
            "this card is filled from an ability's answer: name the ability in `from`".into(),
        );
    }
    build_table(args)
}

fn ability_chart_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(TITLE),
            "chart_type": one_of(&["line", "bar", "area"], "How to draw it: line or area for a value over an ordered x, bar for values across categories."),
            "from": string(FROM),
            "call_id": string(CALL_ID),
            "list": string(LIST),
            "x": string("The key of a record that holds its x value: a category or a number."),
            "y": string("The key of a record that holds the value to plot, a number. One chart shows one measure."),
            "series": string("Optional. The key of a record that holds the name of its series; one series named after `y` when left out."),
            "x_scale": one_of(&["category", "number", "time"], "Optional. category (default) keeps equal spacing and exact labels; number spaces finite numeric x values by magnitude; time spaces RFC3339 timestamps by elapsed time."),
            "x_label": string("Optional. What the x axis measures."),
            "y_label": string("Optional. What the y axis measures, with its unit."),
            "accessible_summary": string("Optional. One sentence saying what the chart shows.")
        }),
        &["chart_type", "from", "call_id", "x", "y"],
    )
}

/// The chart's points from an ability's records: each record's x, y and
/// series, every value the record's own.
fn fill_chart_from(args: &mut Map<String, Value>, records: &[Value]) -> Result<(), String> {
    let objects = records_as_objects(records)?;
    let key = |name: &str| {
        args.get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_string)
    };
    let (Some(x), Some(y)) = (key("x"), key("y")) else {
        return Err("name the record keys for `x` and `y`".into());
    };
    let series = key("series");
    let mut data = Vec::new();
    for (at, record) in objects.iter().enumerate() {
        let value = match record.get(&y) {
            Some(number @ Value::Number(_)) => number.clone(),
            _ => {
                return Err(format!(
                    "record {at} has no number at `{y}`; a chart's `y` names a key holding a number"
                ));
            }
        };
        let Some(x_value) = record.get(&x) else {
            return Err(format!("record {at} has no key `{x}`"));
        };
        let name = match &series {
            Some(series) => cell_of(record.get(series)),
            None => y.clone(),
        };
        data.push(serde_json::json!({"series": name, "x": cell_of(Some(x_value)), "y": value}));
    }
    args.insert("data".into(), Value::Array(data));
    Ok(())
}

fn build_ability_chart(args: &Map<String, Value>) -> Result<Value, String> {
    if !args.contains_key("data") {
        return Err(
            "this card is filled from an ability's answer: name the ability in `from`".into(),
        );
    }
    let mut chart = args.clone();
    for filled_from in ["from", "list", "x", "y", "series"] {
        chart.remove(filled_from);
    }
    build_chart(&chart)
}

const SHAPES: &[CardShape] = &[
    shape(
        "summary",
        "emit_summary_card",
        "Lead the answer: a headline and one to three sentences that give the answer itself, with key points.",
        summary_fields,
        build_summary,
        fill_nothing,
    ),
    shape(
        "steps",
        "emit_steps_card",
        "Things done in order, one after another. Each step has a short label, and a detail, time or duration when they help. A step can offer alternatives the person picks one of (`choices`), and their pick comes back to you.",
        steps_fields,
        build_steps,
        fill_steps,
    ),
    shape(
        "list",
        "emit_list_card",
        "Items in no particular order. Mark it checkable when the items are to be ticked off.",
        list_fields,
        build_list,
        fill_list,
    ),
    shape(
        "details",
        "emit_details_card",
        "Labelled facts about one subject, or questions with their answers.",
        details_fields,
        build_details,
        fill_nothing,
    ),
    CardShape {
        followable: &[&["rows", "*", "*"]],
        ..shape(
            "table",
            "emit_table_card",
            "Records under the same columns, each cell a short value. Rows are strings with the header first and cells separated by |.",
            table_fields,
            build_table,
            fill_table,
        )
    },
    shape(
        "comparison",
        "emit_comparison_card",
        "Two to four options side by side: each one's name, verdict, strengths and weaknesses, the aspects they differ on, and which you recommend.",
        comparison_fields,
        build_comparison,
        fill_comparison,
    ),
    CardShape {
        followable: &[&["readings", "*", "value"]],
        ..shape(
            "stats",
            "emit_stats_card",
            "A few key figures shown large: each a label and value, with its unit and change when known.",
            stats_fields,
            build_stats,
            fill_stats,
        )
    },
    shape(
        "chart",
        "emit_chart_card",
        "Numbers over time or across categories, as a line, area or bar chart. Give every point with the name of its series; one chart shows one measure.",
        chart_fields,
        build_chart,
        fill_nothing,
    ),
    shape(
        "callout",
        "emit_callout_card",
        "One short note set apart, with its tone: a tip, a note, a warning or something important.",
        callout_fields,
        build_callout,
        fill_nothing,
    ),
    CardShape {
        grounded_in: Some(vak_tools::Grounds::Retrieved),
        ..shape(
            "sources",
            "emit_sources_card",
            "Findings drawn from sources you retrieved, each tied to the exact title and URL it came from.",
            sources_fields,
            build_sources,
            fill_nothing,
        )
    },
    shape(
        "calendar",
        "emit_calendar_card",
        "Events placed on dates and times.",
        calendar_fields,
        build_calendar,
        fill_nothing,
    ),
    shape(
        "map",
        "emit_map_card",
        "Places on a map: where they are relative to each other, a route through them in order, or an area they outline. Give each place's latitude and longitude.",
        map_fields,
        build_map,
        fill_map,
    ),
    shape(
        "controls",
        "emit_controls_card",
        "Inputs a person changes and results that update as they do: a calculation, a conversion or a what-if. Each result is an expression over the inputs; Vakyartha computes it. Later cards of the answer can follow its values: a stat through `follows`, a table cell written `= name`.",
        controls_fields,
        build_controls,
        fill_nothing,
    ),
    shape(
        "link",
        "emit_link_card",
        "A preview of one web page: its URL and title.",
        link_fields,
        build_link,
        fill_nothing,
    ),
    shape(
        "media",
        "emit_media_card",
        "An image, video or audio file, by its URL or workspace path.",
        media_fields,
        build_media,
        fill_nothing,
    ),
    CardShape {
        grounded_in: Some(vak_tools::Grounds::Wrote),
        ..shape(
            "file_preview",
            "emit_file_preview_card",
            "A live preview of an HTML or UI file in the workspace, by its path. For markup with no file, use emit_markup_preview_card. The page is shown in Vakyartha's look in every theme: plain elements (headings, tables, buttons, inputs, links) are styled already, the classes vak-card, vak-grid, vak-muted and vak-primary (a button) are there to use, and the colours are CSS variables: --vak-bg, --vak-surface, --vak-fg, --vak-soft, --vak-muted, --vak-accent, --vak-on-accent, --vak-rule, --vak-good, --vak-warn, --vak-bad, --vak-series-1 to --vak-series-4, --vak-font, --vak-font-mono, --vak-radius. Use them and set no colours or fonts of your own, so the page matches the app.",
            file_preview_fields,
            build_file_preview,
            fill_nothing,
        )
    },
    shape(
        "markup_preview",
        "emit_markup_preview_card",
        "A live preview of HTML markup written in the call itself, with no file. For a file in the workspace, use emit_file_preview_card. The page is shown in Vakyartha's look in every theme: plain elements (headings, tables, buttons, inputs, links) are styled already, the classes vak-card, vak-grid, vak-muted and vak-primary (a button) are there to use, and the colours are CSS variables: --vak-bg, --vak-surface, --vak-fg, --vak-soft, --vak-muted, --vak-accent, --vak-on-accent, --vak-rule, --vak-good, --vak-warn, --vak-bad, --vak-series-1 to --vak-series-4, --vak-font, --vak-font-mono, --vak-radius. Use them and set no colours or fonts of your own, so the page matches the app.",
        markup_preview_fields,
        build_markup_preview,
        fill_nothing,
    ),
    CardShape {
        grounded_in: Some(vak_tools::Grounds::Wrote),
        changes_made: Some(vak_tools::ChangesMade {
            paths: "paths",
            files: "files",
        }),
        ..shape(
            "diff",
            "emit_diff_card",
            "The changes this turn's calls made to files, file by file; what changed in each is taken from the file itself.",
            diff_fields,
            build_diff,
            fill_nothing,
        )
    },
    CardShape {
        grounded_in: Some(vak_tools::Grounds::Ran),
        output_of_command: Some(vak_tools::CommandOutput {
            command: "command",
            output: None,
            exit_code: "exit_code",
            duration_ms: "duration_ms",
        }),
        ..shape(
            "test_report",
            "emit_test_report_card",
            "The results of a test run a call in this conversation executed; how the run ended is taken from that call.",
            test_report_fields,
            build_test_report,
            fill_nothing,
        )
    },
    CardShape {
        grounded_in: Some(vak_tools::Grounds::Ran),
        output_of_command: Some(vak_tools::CommandOutput {
            command: "command",
            output: Some("output"),
            exit_code: "exit_code",
            duration_ms: "duration_ms",
        }),
        ..shape(
            "terminal",
            "emit_terminal_card",
            "A command you ran, shown with the output it really printed, which Vak takes from that call.",
            terminal_fields,
            build_terminal,
            fill_nothing,
        )
    },
    CardShape {
        grounded_in: Some(vak_tools::Grounds::Answered),
        data_of_call: Some((
            vak_tools::CallData {
                from: "from",
                call_id: "call_id",
                list: "list",
                filled: &["rows"],
            },
            fill_table_from,
        )),
        ..shape(
            "table",
            "emit_ability_table_card",
            "A table snapshot from one exact recorded ability call: provide its call ID and ability name, then where its list is. Vak fills every row from that result. To refresh, run the ability again and make a new card with the new call ID.",
            ability_table_fields,
            build_ability_table,
            fill_nothing,
        )
    },
    CardShape {
        grounded_in: Some(vak_tools::Grounds::Answered),
        data_of_call: Some((
            vak_tools::CallData {
                from: "from",
                call_id: "call_id",
                list: "list",
                filled: &["data"],
            },
            fill_chart_from,
        )),
        ..shape(
            "chart",
            "emit_ability_chart_card",
            "A chart snapshot from one exact recorded ability call: provide its call ID and ability name, then where its list is and the keys for x and y. Vak fills every point from that result. To refresh, run the ability again and make a new card with the new call ID.",
            ability_chart_fields,
            build_ability_chart,
            fill_nothing,
        )
    },
];

fn shape_of_form(form: &str) -> Option<&'static CardShape> {
    SHAPES.iter().find(|shape| shape.form == form)
}

/// The card tool whose own kind is `kind`.
pub fn emit_tool_for(kind: &str) -> Option<&'static str> {
    shape_of_form(kind).map(|shape| shape.name)
}

/// One valid set of arguments per form, as a model would send them: the
/// single source for the conformance tests here and in vak-server, which
/// follows the full call → ledger → projection path.
#[doc(hidden)]
pub fn conformance_cases() -> Vec<(&'static str, &'static str, Value)> {
    // A card filled from an ability's answer has no arguments alone that
    // show it; its own tests call an ability first.
    SHAPES
        .iter()
        .filter(|shape| shape.data_of_call.is_none())
        .map(|shape| (shape.name, shape.form, fixture_for(shape.form)))
        .collect()
}

fn fixture_for(form: &str) -> Value {
    match form {
        "summary" => {
            serde_json::json!({"headline": "Keep the shelves as they are", "text": "Sorting by kind is easy to follow.", "points": ["Restock on Saturdays"]})
        }
        "steps" => {
            serde_json::json!({"title": "Morning", "steps": [{"label": "Boil water", "duration_minutes": 5}, {"label": "Brew"}],
            "choices": [{"step": "Brew", "label": "French press", "facts": "4 min; coarse grind"}]})
        }
        "list" => {
            serde_json::json!({"checkable": true, "items": [{"label": "Rice"}, {"label": "Tins", "detail": "Two of each"}]})
        }
        "details" => {
            serde_json::json!({"title": "Pune", "fields": [{"label": "State", "detail": "Maharashtra"}]})
        }
        "table" => serde_json::json!({"rows": ["Name | Role", "Alice | Lead"]}),
        "comparison" => serde_json::json!({
            "options": [{"name": "By shelf", "verdict": "Simple", "strengths": "Easy; familiar"}, {"name": "By meal", "weaknesses": "Needs relabelling"}],
            "aspects": [{"aspect": "Effort", "option": "By shelf", "value": "Low"}, {"aspect": "Effort", "option": "By meal", "value": "High"}],
            "recommendation": "Keep by shelf."
        }),
        "stats" => {
            serde_json::json!({"readings": [{"label": "Uptime", "value": "99.9", "unit": "%"}]})
        }
        "chart" => {
            serde_json::json!({"chart_type": "bar", "data": [{"series": "Share (%)", "x": "Adoption", "y": 88}, {"series": "Share (%)", "x": "Success", "y": 66}]})
        }
        "callout" => serde_json::json!({"tone": "tip", "text": "Check dates monthly."}),
        "sources" => {
            serde_json::json!({"sources": [{"title": "Index", "url": "https://example.com/index", "finding": "Adoption rose."}]})
        }
        "calendar" => {
            serde_json::json!({"events": [{"title": "Morning standup with the team", "date": "2026-10-15", "start": "09:00", "end": "09:15"}]})
        }
        "map" => serde_json::json!({"connect": "route", "places": [
            {"label": "Pune Station", "position": "18.5286, 73.8746"},
            {"label": "Shaniwar Wada", "position": "18.5195, 73.8553", "detail": "The old palace, open from eight in the morning"}]}),
        "controls" => {
            serde_json::json!({"title": "Split the restaurant bill between friends, with a tip", "inputs": [
            {"name": "bill", "kind": "number", "value": "1200", "min": 0, "unit": "₹", "label": "Bill"},
            {"name": "tip", "kind": "choice", "value": "Usual", "options": "None=0; Usual=15; Generous=20", "unit": "%", "label": "Tip"},
            {"name": "people", "kind": "range", "value": "3", "min": 1, "max": 12, "step": 1, "label": "People"}],
            "outputs": [{"name": "each", "expression": "bill * (1 + tip / 100) / people", "unit": "₹", "label": "Each pays"}]})
        }
        "link" => serde_json::json!({"url": "https://example.com/page", "title": "Page"}),
        "media" => {
            serde_json::json!({"format": "image", "url": "https://example.com/a.png", "alt": "A chart"})
        }
        "file_preview" => serde_json::json!({"artifact_path": "index.html"}),
        "markup_preview" => serde_json::json!({"html": "<p>Hello</p>", "libraries": []}),
        "diff" => {
            serde_json::json!({"files": [{"filename": "a.rs", "hunks": "@@ -1 +1 @@", "additions": 1, "deletions": 0}]})
        }
        "test_report" => {
            serde_json::json!({"command": "cargo test", "exit_code": 0, "duration_ms": 1200, "tests": [{"name": "it_works", "status": "passed"}]})
        }
        "terminal" => serde_json::json!({"command": "ls", "output": "a.rs", "exit_code": 0}),
        _ => Value::Null,
    }
}

// ---- packs -------------------------------------------------------------------

/// The kinds active presentation packs add to forms, by form
/// (docs/design/91-presentation-capabilities.md §2). A pack names the form
/// whose payload it reads in its spec's `block`; while it is the selected
/// pack for a kind, that kind is a kind of the form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackKinds {
    kinds: std::collections::BTreeMap<String, Vec<String>>,
    packs: Vec<ActivePack>,
}

/// A pack that adds kinds to a form, as the capability registry lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePack {
    pub id: String,
    pub block: String,
    pub kinds: Vec<String>,
    pub plugin: Option<String>,
    pub digest: String,
}

impl PackKinds {
    pub fn from_library(
        library: &vak_presentation::PresentationLibrary,
        workspace_owner: &str,
    ) -> Self {
        let mut kinds: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        let mut packs: Vec<ActivePack> = Vec::new();
        for stored in library.definitions() {
            let Some(block) = stored.spec.block.as_deref() else {
                continue;
            };
            if shape_of_form(block).is_none() {
                continue;
            }
            for kind in &stored.spec.accepts {
                if vak_delivery::is_form(kind) {
                    continue;
                }
                let selected = library
                    .select_preferred(kind, "user", workspace_owner)
                    .is_some_and(|chosen| {
                        chosen.spec.id == stored.spec.id
                            && chosen.spec.revision == stored.spec.revision
                    });
                let listed = kinds.entry(block.to_string()).or_default();
                if selected && !listed.contains(kind) {
                    listed.push(kind.clone());
                    match packs.iter_mut().find(|pack| pack.id == stored.spec.id) {
                        Some(pack) => pack.kinds.push(kind.clone()),
                        None => packs.push(ActivePack {
                            id: stored.spec.id.clone(),
                            block: block.to_string(),
                            kinds: vec![kind.clone()],
                            plugin: stored.origin.plugin_id.clone(),
                            digest: stored.digest.clone(),
                        }),
                    }
                }
            }
        }
        kinds.retain(|_, listed| !listed.is_empty());
        Self { kinds, packs }
    }

    /// Keeps only the packs `keep` admits, with the kinds they add.
    pub fn retain_packs(&mut self, mut keep: impl FnMut(&str) -> bool) {
        let dropped: Vec<ActivePack> = self
            .packs
            .iter()
            .filter(|pack| !keep(&pack.id))
            .cloned()
            .collect();
        self.packs
            .retain(|pack| !dropped.iter().any(|gone| gone.id == pack.id));
        for pack in dropped {
            if let Some(listed) = self.kinds.get_mut(&pack.block) {
                listed.retain(|kind| !pack.kinds.contains(kind));
            }
        }
        self.kinds.retain(|_, listed| !listed.is_empty());
    }

    /// The packs that add kinds, for the capability registry.
    pub fn packs(&self) -> &[ActivePack] {
        &self.packs
    }

    fn of(&self, shape: &CardShape) -> &[String] {
        self.kinds.get(shape.form).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The form a pack kind belongs to.
    pub fn block_of(&self, kind: &str) -> Option<&str> {
        self.kinds
            .iter()
            .find(|(_, listed)| listed.iter().any(|listed| listed == kind))
            .map(|(block, _)| block.as_str())
    }
}

// ---- the tool ----------------------------------------------------------------

pub struct EmitCardTool {
    shape: &'static CardShape,
    /// Kinds active packs add to this form.
    extra: Vec<String>,
}

impl EmitCardTool {
    /// One tool per form, with only the form's own kind.
    pub fn all() -> Vec<Self> {
        Self::with_packs(&PackKinds::default())
    }

    /// The card tools for a turn: each form's own kind and the kinds the
    /// active packs add to it.
    pub fn with_packs(packs: &PackKinds) -> Vec<Self> {
        SHAPES
            .iter()
            .map(|shape| EmitCardTool {
                shape,
                extra: packs.of(shape).to_vec(),
            })
            .collect()
    }

    fn kinds(&self) -> Vec<&str> {
        std::iter::once(self.shape.form)
            .chain(self.extra.iter().map(String::as_str))
            .collect()
    }
}

#[async_trait]
impl Tool for EmitCardTool {
    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(vak_permission::Reach::Display)
    }

    fn name(&self) -> &str {
        self.shape.name
    }

    fn description(&self) -> &str {
        self.shape.description
    }

    fn schema(&self) -> Value {
        let own = (self.shape.fields)();
        let mut properties = own["properties"].as_object().cloned().unwrap_or_default();
        if self.kinds().len() > 1 {
            properties.insert(
                "semantic_type".into(),
                serde_json::json!({
                    "type": "string",
                    "enum": self.kinds(),
                    "description": format!("Optional. Which layout to draw it with; leave it out for the regular {} card.", self.shape.form)
                }),
            );
        }
        properties.insert(
            "label".into(),
            string("Optional. A few words naming this part, shown above it."),
        );
        let required = own["required"].as_array().cloned().unwrap_or_default();
        serde_json::json!({"type": "object", "properties": properties, "required": required})
    }

    /// The schema closed at every record, so a field it does not name is
    /// refused and the call written again, never dropped by the builder
    /// while the model is told the card was shown. The advertised schema
    /// stays open: not every provider carries `additionalProperties`.
    fn call_schema(&self, _args: &Value) -> Value {
        let mut schema = self.schema();
        close_records(&mut schema);
        schema
    }

    fn presents_cards(&self) -> bool {
        true
    }

    fn followable_fields(&self) -> Vec<Vec<String>> {
        self.shape
            .followable
            .iter()
            .map(|path| path.iter().map(|step| (*step).to_string()).collect())
            .collect()
    }

    fn card_block(&self) -> Option<&str> {
        Some(self.shape.form)
    }

    fn output_of_command(&self) -> Option<vak_tools::CommandOutput> {
        self.shape.output_of_command
    }

    fn changes_made(&self) -> Option<vak_tools::ChangesMade> {
        self.shape.changes_made
    }

    fn data_of_call(&self) -> Option<vak_tools::CallData> {
        self.shape.data_of_call.map(|(spec, _)| spec)
    }

    fn grounded_in(&self) -> Option<vak_tools::Grounds> {
        self.shape.grounded_in
    }

    fn fill_from_data(&self, args: &mut Value, records: &[Value]) -> Result<(), String> {
        let Some((_, fill)) = self.shape.data_of_call else {
            return Ok(());
        };
        let Some(args) = args.as_object_mut() else {
            return Err("the card's arguments are not an object".into());
        };
        fill(args, records)
    }

    fn card_kinds(&self) -> Vec<String> {
        self.kinds().into_iter().map(str::to_string).collect()
    }

    fn fill_defaults(&self, args: &mut Value) {
        if let Some(fields) = args.as_object_mut() {
            // A kind this tool does not draw is dropped, so a refusal never
            // blames it, unless it names another form: then the call is
            // refused with that form's tool.
            let stray = fields
                .get("semantic_type")
                .and_then(Value::as_str)
                .is_some_and(|kind| !self.kinds().contains(&kind) && emit_tool_for(kind).is_none());
            if stray
                || self.kinds().len() == 1
                    && fields.get("semantic_type").and_then(Value::as_str) == Some(self.shape.form)
            {
                fields.remove("semantic_type");
            }
            (self.shape.fill)(fields);
        }
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        match validate_call(
            self.shape,
            &self.extra,
            args,
            &vak_delivery::built_in_skill_registry(),
        ) {
            Ok(output) => {
                let mut ack = format!("Accepted for presentation ({}).", output.semantic_type);
                // What a later card of the answer can follow (docs/design/98 §2).
                if output.block == "controls"
                    && let Ok(published) = vak_delivery::controls::published(&output.payload)
                {
                    let names: Vec<&str> =
                        published.iter().map(|(name, _)| name.as_str()).collect();
                    let series: Vec<&str> = output.payload["outputs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|output| output.get("vary").is_some())
                        .filter_map(|output| output["name"].as_str())
                        .collect();
                    if !series.is_empty() {
                        ack.push_str(&format!(
                            " Its series {} can be drawn by a later chart card with follows set to their names.",
                            series.join(", ")
                        ));
                    }
                    if let Some(last) = names.last() {
                        ack.push_str(&format!(
                            " It publishes {}. A later stats card follows them with a reading like {{\"label\": \"…\", \"follows\": \"{last}\"}}; a table cell with = {last}.",
                            names.join(", ")
                        ));
                    }
                }
                ToolOutput::ok(ack)
            }
            Err(reason) => ToolOutput::error(
                Fault::Shape,
                format!("Card not displayed: {reason}. Fix these fields."),
            ),
        }
    }
}

fn validate_call(
    shape: &CardShape,
    extra: &[String],
    args: &Value,
    skills: &vak_delivery::SkillRegistry,
) -> Result<vak_delivery::StructuredOutput, String> {
    let Some(args) = args.as_object() else {
        return Err("the arguments must be an object".into());
    };
    let kind = match args.get("semantic_type") {
        None => shape.form.to_string(),
        Some(Value::String(kind)) if kind == shape.form || extra.contains(kind) => kind.clone(),
        // A layout no active pack draws is the form itself; a kind of
        // another form is that form's call to make.
        Some(Value::String(kind)) => match emit_tool_for(kind) {
            Some(tool) => {
                return Err(format!(
                    "`{kind}` is drawn by {tool}; call that tool for it"
                ));
            }
            None => shape.form.to_string(),
        },
        Some(_) => return Err("`semantic_type` must be a string".into()),
    };
    let mut own = args.clone();
    own.remove("semantic_type");
    // Every card takes `label` beside its own fields: the model's own
    // words for it, shown above it.
    let label = own
        .remove("label")
        .and_then(|label| label.as_str().map(str::trim).map(str::to_string))
        .filter(|label| !label.is_empty());
    let known = (shape.fields)()["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut filled = shape
        .output_of_command
        .map(|spec| spec.filled())
        .unwrap_or_default();
    if let Some((spec, _)) = shape.data_of_call {
        filled.extend(spec.filled);
    }
    if let Some(spec) = shape.changes_made {
        filled.push(spec.files);
    }
    let unknown: Vec<String> = own
        .keys()
        .filter(|key| !known.contains_key(key.as_str()) && !filled.contains(&key.as_str()))
        .map(|key| format!("`{key}`"))
        .collect();
    if !unknown.is_empty() {
        let takes: Vec<String> = known.keys().map(|key| format!("`{key}`")).collect();
        return Err(format!(
            "unexpected {}; {} takes {}",
            unknown.join(", "),
            shape.name,
            takes.join(", ")
        ));
    }
    let mut payload = (shape.build)(&own)?;
    if let (Some(label), Some(object)) = (label, payload.as_object_mut()) {
        object.insert("label".into(), Value::String(label));
    }
    vak_delivery::structured_output(&kind, Some(shape.form), payload, skills)
        .map_err(|e| e.to_string())
}

// ---- recording ---------------------------------------------------------------

/// Everything needed to write a `PresentationRecord` for a call that just
/// validated. `vak-agent`'s tool path has no session-log access (invariant
/// 14), so this is reached through `AgentConfig::presentation_rebuild`, a
/// closure `Core` installs.
pub struct PresentationInfo {
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub schema_version: u32,
    /// Canonical (key-sorted) form — see `vak_session::types::canonicalize_json`.
    pub payload: Value,
    pub title: String,
    pub identity_digest: String,
    /// The form that built it.
    pub block: Option<String>,
    pub data_call_id: Option<String>,
}

/// Validates a card call and returns its `Presentation` ledger entry. `name`
/// is the card tool the call was made to, or a fence's kind, which names
/// the form that owns it. A call is the form it was made to: a stray kind in
/// its arguments never sends it to another form. `None` when the call does
/// not validate.
pub fn presentation_info(
    name: &str,
    input: &Value,
    skills: &vak_delivery::SkillRegistry,
) -> Option<PresentationInfo> {
    presentation_info_with(name, input, skills, &PackKinds::default())
}

/// [`presentation_info`] for a turn whose card tools carry the kinds the
/// active packs add.
pub fn presentation_info_with(
    name: &str,
    input: &Value,
    skills: &vak_delivery::SkillRegistry,
    packs: &PackKinds,
) -> Option<PresentationInfo> {
    let shape = SHAPES
        .iter()
        .find(|shape| shape.name == name)
        .or_else(|| shape_of_form(name))
        .or_else(|| packs.block_of(name).and_then(shape_of_form))?;
    let output = validate_call(shape, packs.of(shape), input, skills).ok()?;
    let payload = vak_session::types::canonicalize_json(&output.payload);
    Some(PresentationInfo {
        title: title_for(&output.semantic_type, &payload),
        identity_digest: identity_digest(&output.semantic_type, &payload),
        semantic_type: output.semantic_type,
        skill_id: output.skill_id,
        skill_version: output.skill_version,
        schema_version: u32::from(output.schema_version),
        payload,
        block: Some(output.block),
        data_call_id: shape.data_of_call.and_then(|(spec, _)| {
            input
                .get(spec.call_id)
                .and_then(Value::as_str)
                .map(str::to_owned)
        }),
    })
}

fn title_for(kind: &str, payload: &Value) -> String {
    ["title", "label", "headline"]
        .iter()
        .map(|key| text(payload, key))
        .find(|title| !title.is_empty())
        .unwrap_or(kind)
        .to_string()
}

fn labels(payload: &Value, list: &str, key: &str) -> String {
    payload
        .get(list)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| text(item, key))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}

fn canonical_compact(payload: &Value) -> String {
    serde_json::to_string(&vak_session::types::canonicalize_json(payload)).unwrap_or_default()
}

/// What tells a card apart from another of its form in a `TurnCard` line
/// (docs/design/68-context-engine.md §10): the fields that carry its
/// identity, read from the stored shape, never a character truncation.
pub fn identity_digest(kind: &str, payload: &Value) -> String {
    let title = title_for(kind, payload);
    let has = |key: &str| payload.get(key).is_some();
    if has("readings") {
        canonical_compact(payload)
    } else if has("columns") && has("rows") {
        let rows = payload["rows"].as_array();
        format!(
            "title: {title} | columns: {} | rows: {} | first: {}",
            labels(payload, "columns", "label"),
            rows.map_or(0, Vec::len),
            canonical_compact(rows.and_then(|rows| rows.first()).unwrap_or(&Value::Null))
        )
    } else if has("series") {
        let series: Vec<String> = payload["series"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                format!(
                    "{}({})",
                    text(s, "name"),
                    s["points"].as_array().map_or(0, Vec::len)
                )
            })
            .collect();
        format!(
            "title: {title} | series: {} | summary: {}",
            series.join(","),
            text(payload, "accessible_summary")
        )
    } else if has("sources") {
        let sources: Vec<String> = payload["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                format!(
                    "{} ({}): {}",
                    text(s, "title"),
                    text(s, "url"),
                    text(s, "finding")
                )
            })
            .collect();
        format!("title: {title} | sources: {}", sources.join(" ~ "))
    } else if has("events") {
        let events: Vec<String> = payload["events"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| format!("{} {}", text(e, "date"), text(e, "title")))
            .collect();
        format!("title: {title} | events: {}", events.join(","))
    } else if has("places") {
        format!(
            "title: {title} | connect: {} | places: {}",
            text(payload, "connect"),
            labels(payload, "places", "label")
        )
    } else if has("steps") {
        format!(
            "title: {title} | steps: {}",
            labels(payload, "steps", "label")
        )
    } else if has("items") {
        format!(
            "title: {title} | items: {}",
            labels(payload, "items", "label")
        )
    } else if has("fields") {
        format!(
            "title: {title} | fields: {}",
            labels(payload, "fields", "label")
        )
    } else if has("options") {
        format!(
            "title: {title} | options: {}",
            labels(payload, "options", "name")
        )
    } else {
        let mut keys: Vec<&str> = payload
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        keys.sort();
        format!("title: {title} | keys: {}", keys.join(","))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> EmitCardTool {
        EmitCardTool::all()
            .into_iter()
            .find(|tool| tool.name() == name)
            .unwrap()
    }

    /// A skill Vakyartha ships names only card tools that exist: a card
    /// retired or renamed leaves no skill telling the model to call it.
    #[test]
    fn every_card_tool_a_shipped_skill_names_exists() {
        let names: Vec<String> = EmitCardTool::all()
            .iter()
            .map(|tool| tool.name().to_string())
            .collect();
        let packages = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
        let mut checked = 0;
        for package in std::fs::read_dir(&packages).unwrap().flatten() {
            let Ok(skills) = std::fs::read_dir(package.path().join("skills")) else {
                continue;
            };
            for skill in skills.flatten() {
                let Ok(text) = std::fs::read_to_string(skill.path().join("SKILL.md")) else {
                    continue;
                };
                checked += 1;
                for (start, _) in text.match_indices("emit_") {
                    let named: String = text[start..]
                        .chars()
                        .take_while(|c| c.is_ascii_lowercase() || *c == '_')
                        .collect();
                    if named.ends_with("_card") {
                        assert!(
                            names.contains(&named),
                            "{} names {named}, which is not a card tool",
                            skill.path().display()
                        );
                    }
                }
            }
        }
        assert!(
            checked > 0,
            "no shipped skills under {}",
            packages.display()
        );
    }

    /// A call's arguments as they reach the builder: read by the one
    /// argument reader, then given the tool's own defaults.
    fn sent(tool: &EmitCardTool, args: &mut Value) {
        vak_tools::read_arguments(&tool.schema(), args);
        tool.fill_defaults(args);
    }

    fn built(name: &str, mut args: Value) -> Result<Value, String> {
        let tool = tool(name);
        sent(&tool, &mut args);
        validate_call(
            tool.shape,
            &[],
            &args,
            &vak_delivery::built_in_skill_registry(),
        )
        .map(|output| output.payload)
    }

    #[test]
    fn the_forms_are_exactly_the_delivery_forms_in_order() {
        let (own, from_call): (Vec<&CardShape>, Vec<&CardShape>) = SHAPES
            .iter()
            .partition(|shape| shape.data_of_call.is_none());
        let forms: Vec<&str> = own.iter().map(|shape| shape.form).collect();
        assert_eq!(forms, vak_delivery::FORMS);
        for shape in own {
            assert_eq!(shape.name, format!("emit_{}_card", shape.form));
        }
        // A card filled from an ability's answer presents a form already
        // listed, by a tool of its own.
        for shape in from_call {
            assert!(vak_delivery::FORMS.contains(&shape.form));
            assert_eq!(shape.name, format!("emit_ability_{}_card", shape.form));
        }
    }

    #[test]
    fn inferred_ability_columns_include_later_only_fields() {
        let records = vec![
            json!({"id":"00123"}),
            json!({"id":"00007", "later_only":"retained"}),
        ];
        let mut args = json!({"from":"records", "call_id":"selected"});
        tool("emit_ability_table_card")
            .fill_from_data(&mut args, &records)
            .unwrap();
        let card = built("emit_ability_table_card", args).unwrap();
        assert_eq!(card["columns"][0]["label"], "id");
        assert_eq!(card["columns"][1]["label"], "later_only");
        assert_eq!(card["rows"], json!([["00123", ""], ["00007", "retained"]]));
    }

    #[test]
    fn an_abilitys_records_fill_a_table_and_a_chart_with_their_own_values() {
        let records: Vec<Value> = (0..300)
            .map(|at| serde_json::json!({"city": format!("City {at}"), "people": at * 1000, "note": "a | b\nc"}))
            .collect();
        let mut table = serde_json::json!({"from": "census", "call_id": "c1", "list": "rows", "columns": ["city", "people", "note"]});
        let table_tool = tool("emit_ability_table_card");
        table_tool.fill_from_data(&mut table, &records).unwrap();
        let card = built("emit_ability_table_card", table).unwrap();
        let rows = card["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 300);
        assert_eq!(card["columns"][1]["numeric"], true);
        for (at, row) in rows.iter().enumerate() {
            assert_eq!(row[0], format!("City {at}"));
            assert_eq!(row[1], (at * 1000).to_string());
            assert_eq!(row[2], "a | b c");
        }

        let typed = vec![
            json!({"id": "00123", "amount": 9007199254740993_u64}),
            json!({"id": "1,5", "amount": 2_u64}),
        ];
        let mut table = json!({"from": "records", "call_id": "c2", "columns": ["id", "amount"]});
        table_tool.fill_from_data(&mut table, &typed).unwrap();
        let card = built("emit_ability_table_card", table).unwrap();
        assert_eq!(
            card["columns"],
            json!([
                {"label": "id", "numeric": false}, {"label": "amount", "numeric": true}
            ])
        );
        assert_eq!(card["rows"][0], json!(["00123", "9007199254740993"]));
        assert_eq!(card["rows"][1], json!(["1,5", "2"]));

        let explicit_values = vec![
            json!({"id": "00123", "kind": "a"}),
            json!({"id": "00002", "kind": "b"}),
        ];
        let mut declared = json!({"from": "records", "call_id": "c3", "columns": ["id", "kind"], "numeric_columns": ["id"]});
        table_tool
            .fill_from_data(&mut declared, &explicit_values)
            .unwrap();
        let card = built("emit_ability_table_card", declared).unwrap();
        assert_eq!(card["columns"][0]["numeric"], true);
        assert_eq!(card["rows"][0][0], "00123");
        for declaration in [json!(1), json!([1]), json!([""])] {
            let mut malformed = json!({
                "from": "records", "call_id": "c4", "columns": ["id", "kind"], "numeric_columns": declaration
            });
            assert!(
                table_tool
                    .fill_from_data(&mut malformed, &explicit_values)
                    .is_err()
            );
        }

        let mut chart = serde_json::json!({"from": "census", "call_id": "c5", "chart_type": "bar", "x": "city", "y": "people"});
        tool("emit_ability_chart_card")
            .fill_from_data(&mut chart, &records[..3])
            .unwrap();
        let card = built("emit_ability_chart_card", chart).unwrap();
        assert_eq!(card["series"][0]["name"], "people");
        assert_eq!(card["series"][0]["points"][2]["y"], 2000);

        let mut missing = serde_json::json!({"from": "census", "columns": ["town"]});
        let refused = table_tool
            .fill_from_data(&mut missing, &records)
            .unwrap_err();
        assert!(refused.contains("`town`"), "{refused}");
        let mut empty = serde_json::json!({"from": "census"});
        assert!(table_tool.fill_from_data(&mut empty, &[]).is_err());
    }

    #[test]
    fn every_card_conformance_case_passes_the_pre_execution_schema_gate() {
        let skills = vak_delivery::built_in_skill_registry();
        for (name, form, mut args) in conformance_cases() {
            let tool = tool(name);
            sent(&tool, &mut args);
            vak_tools::validate_input(&tool.schema(), &args)
                .unwrap_or_else(|error| panic!("{form}: {error}"));
            // What the model sends, before the runtime fills in what the
            // command printed and how it ended, is closed too.
            let mut written = args.clone();
            if let (Some(spec), Some(fields)) = (tool.output_of_command(), written.as_object_mut())
            {
                for filled in [Some(spec.exit_code), Some(spec.duration_ms), spec.output] {
                    filled.map(|key| fields.remove(key));
                }
            }
            if let (Some(spec), Some(fields)) = (tool.changes_made(), written.as_object_mut()) {
                fields.remove(spec.files);
            }
            vak_tools::validate_input(&tool.call_schema(&written), &written)
                .unwrap_or_else(|error| panic!("{form} as written: {error}"));
            let info = presentation_info(name, &args, &skills)
                .unwrap_or_else(|| panic!("{form}: refused a schema-valid call"));
            assert_eq!(
                (info.semantic_type.as_str(), info.block.as_deref()),
                (form, Some(form))
            );
        }
    }

    /// A controls card's results are the runtime's: computed from the
    /// values it starts from, never a number the model wrote
    /// (docs/design/96, P1).
    #[test]
    fn a_controls_card_computes_its_own_results() {
        let card = built("emit_controls_card", fixture_for("controls")).unwrap();
        assert_eq!(card["inputs"][1]["value"], 15.0);
        assert_eq!(card["inputs"][1]["options"][2]["label"], "Generous");
        assert_eq!(card["outputs"][0]["equals"], 460.0);
        assert_eq!(card["outputs"][0]["decimals"], 0);
        let mut unknown = fixture_for("controls");
        unknown["outputs"][0]["expression"] = json!("bill / guests");
        let refused = built("emit_controls_card", unknown).unwrap_err();
        assert!(refused.contains("`guests`"), "{refused}");
        let mut member = fixture_for("controls");
        member["outputs"][0]["expression"] = json!("bill * tip.value");
        let refused = built("emit_controls_card", member).unwrap_err();
        assert!(refused.contains("write `tip`"), "{refused}");
        let loan = json!({"inputs": [
            {"name": "p", "kind": "number", "value": "100000", "label": "Loan"},
            {"name": "r", "kind": "range", "value": "12", "min": 1, "max": 30, "label": "Rate"},
            {"name": "n", "kind": "number", "value": "12", "label": "Months"}],
            "outputs": [{"name": "emi", "expression": "p * r / 1200 / (1 - (1 + r / 1200) ^ (-n))",
            "decimals": 2, "label": "Monthly payment"}]});
        assert_eq!(
            built("emit_controls_card", loan).unwrap()["outputs"][0]["equals"],
            8884.88
        );
    }

    /// A chart that follows a controls card's series is a valid card before
    /// its points are bound (docs/design/98 §2).
    #[test]
    fn a_chart_may_follow_a_series_before_its_points_are_bound() {
        let chart = built(
            "emit_chart_card",
            json!({"chart_type": "line", "follows": "amount"}),
        )
        .unwrap();
        assert_eq!(chart["series"][0]["follows"], "amount");
        assert_eq!(chart["series"][0]["points"], json!([]));
        assert!(built("emit_chart_card", json!({"chart_type": "line"})).is_err());
    }

    /// A markup preview names the libraries it uses from a closed list, and
    /// the stored card keeps them for the client to inline (docs/design/96
    /// P3).
    #[test]
    fn a_markup_preview_names_its_libraries_from_the_vendored_list() {
        let card = built(
            "emit_markup_preview_card",
            json!({"html": "<pre class=\"mermaid\">graph TD; A-->B</pre>", "libraries": ["diagram"]}),
        )
        .unwrap();
        assert_eq!(card["libraries"], json!(["diagram"]));
        assert!(
            built(
                "emit_markup_preview_card",
                json!({"html": "<p>x</p>", "libraries": ["anything"]}),
            )
            .is_err()
        );
    }

    /// Choices written inside a step were dropped by the builder while the
    /// model was told the card was shown (live, 2026-10-17); the call's
    /// schema refuses them, so the call is written again.
    #[test]
    fn a_field_the_card_does_not_name_is_refused_not_dropped() {
        let steps = tool("emit_steps_card");
        let mut args = json!({
            "steps": [
                {"label": "Boil water"},
                {"label": "Brew", "choices": [{"step": "Brew", "fact": "4 min"}]}
            ]
        });
        steps.fill_defaults(&mut args);
        vak_tools::validate_input(&steps.schema(), &args)
            .expect("the advertised schema stays open for every provider");
        let refused = vak_tools::validate_input(&steps.call_schema(&args), &args).unwrap_err();
        assert!(refused.contains("`choices`"), "{refused}");
        assert!(
            steps
                .schema()
                .to_string()
                .find("additionalProperties")
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_valid_call_is_acked_and_an_invalid_one_is_a_repairable_error() {
        let ctx = ToolContext::new(std::env::temp_dir());
        let chart = tool("emit_chart_card");
        let out = chart.execute(&fixture_for("chart"), &ctx).await;
        assert!(!out.is_error(), "{}", out.content);
        assert!(out.content.starts_with("Accepted for presentation"));
        assert!(!out.content.contains("Shown to the user"));
        let out = chart
            .execute(&json!({"chart_type": "bar", "data": []}), &ctx)
            .await;
        assert!(
            out.is_error() && out.content.contains("give `data`"),
            "{}",
            out.content
        );
    }

    #[test]
    fn no_argument_shape_crashes_a_card_tool() {
        let bends = [
            json!(null),
            json!(""),
            json!("one line"),
            json!("a: b\nc"),
            json!(7),
            json!([]),
            json!({}),
            json!(["x"]),
            json!([{}]),
            json!([{"label": ""}]),
            json!([["a"], []]),
            json!(["a | b", "1"]),
        ];
        let registry = vak_delivery::built_in_skill_registry();
        for tool in EmitCardTool::all() {
            let fixture = fixture_for(tool.shape.form);
            let keys: Vec<String> = fixture
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .chain(["semantic_type".to_string(), "label".to_string()])
                .collect();
            let mut calls = vec![json!({}), fixture.clone()];
            for key in &keys {
                let mut without = fixture.clone();
                without.as_object_mut().unwrap().remove(key);
                calls.push(without);
                for bend in &bends {
                    let mut bent = fixture.clone();
                    bent[key.as_str()] = bend.clone();
                    calls.push(bent);
                }
            }
            for mut call in calls {
                let given = call.clone();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    sent(&tool, &mut call);
                    if vak_tools::validate_input(&tool.schema(), &call).is_ok() {
                        let _ = validate_call(tool.shape, &[], &call, &registry);
                    }
                }));
                assert!(outcome.is_ok(), "{} crashed on {given}", tool.name());
            }
        }
    }

    /// The contract's rules (docs/design/30-render-architecture.md §30.1),
    /// checked on every tool's advertised schema.
    #[test]
    fn every_card_schema_is_flat_and_portable() {
        const KEYWORDS: &[&str] = &[
            "type",
            "properties",
            "items",
            "required",
            "enum",
            "description",
        ];
        fn walk(schema: &Value, path: &str, record_lists: usize, out: &mut Vec<String>) {
            let Some(object) = schema.as_object() else {
                return;
            };
            for key in object.keys() {
                if !KEYWORDS.contains(&key.as_str()) {
                    out.push(format!("{path}: `{key}` is not carried by every provider"));
                }
            }
            if object.get("type").is_some_and(|t| !t.is_string()) {
                out.push(format!("{path}: a union `type`"));
            }
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for (name, property) in properties {
                    walk(property, &format!("{path}.{name}"), record_lists, out);
                }
            }
            if let Some(items) = object.get("items") {
                let nested = record_lists
                    + usize::from(items.get("type").and_then(Value::as_str) == Some("object"));
                if nested > 1 {
                    out.push(format!(
                        "{path}: a list of records inside a list of records"
                    ));
                }
                walk(items, &format!("{path}[]"), nested, out);
            }
        }
        let mut problems = Vec::new();
        for tool in EmitCardTool::all() {
            let schema = tool.schema();
            walk(&schema, tool.name(), 0, &mut problems);
            if schema["properties"].get("semantic_type").is_some() {
                problems.push(format!("{}: one kind and still asks for one", tool.name()));
            }
            if schema.to_string().contains("|---") {
                problems.push(format!(
                    "{}: asks for a value that starts with |",
                    tool.name()
                ));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// Nothing about a card tool's arguments is left to guess.
    #[test]
    fn every_card_argument_is_described() {
        fn check(tool: &str, properties: &Value, required: &Value, problems: &mut Vec<String>) {
            for (name, property) in properties.as_object().unwrap() {
                let description = property["description"].as_str().unwrap_or_default();
                let needed = required
                    .as_array()
                    .is_some_and(|required| required.iter().any(|key| key == name));
                if description.trim().is_empty() {
                    problems.push(format!("{tool}.{name} has no description"));
                }
                if needed && description.starts_with("Optional") {
                    problems.push(format!(
                        "{tool}.{name} is required but described as optional"
                    ));
                }
                if !needed && !description.starts_with("Optional") {
                    problems.push(format!("{tool}.{name} is optional but not described so"));
                }
                if property["items"]["type"] == "object" {
                    check(
                        &format!("{tool}.{name}[]"),
                        &property["items"]["properties"],
                        &property["items"]["required"],
                        problems,
                    );
                }
            }
        }
        let mut problems = Vec::new();
        for tool in EmitCardTool::all() {
            let schema = tool.schema();
            assert!(!tool.description().trim().is_empty());
            check(
                tool.name(),
                &schema["properties"],
                &schema["required"],
                &mut problems,
            );
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// One argument name is one shape on every card tool.
    #[test]
    fn one_argument_name_is_one_shape_on_every_block() {
        fn strip(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    map.remove("description");
                    map.values_mut().for_each(strip);
                }
                Value::Array(items) => items.iter_mut().for_each(strip),
                _ => {}
            }
        }
        let mut seen: std::collections::BTreeMap<String, (String, Value)> = Default::default();
        let mut problems = Vec::new();
        for tool in EmitCardTool::all() {
            for (name, property) in tool.schema()["properties"]
                .as_object()
                .into_iter()
                .flatten()
            {
                let mut ours = json!({"type": property["type"], "items": property["items"], "enum": property["enum"]});
                strip(&mut ours);
                match seen.get(name) {
                    Some((owner, theirs)) if *theirs != ours => problems.push(format!(
                        "`{name}` is {theirs} in {owner} and {ours} in {}",
                        tool.name()
                    )),
                    Some(_) => {}
                    None => {
                        seen.insert(name.clone(), (tool.name().to_string(), ours));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn every_card_tool_is_allowed_in_every_mode_without_a_rule() {
        let dir = tempfile::tempdir().unwrap();
        let engine = crate::build_engine_with(&vak_config::Config::default(), &[]).unwrap();
        for tool in EmitCardTool::all() {
            for mode in [
                vak_permission::Mode::ReadOnly,
                vak_permission::Mode::WorkspaceWrite,
                vak_permission::Mode::FullAccess,
            ] {
                assert!(
                    matches!(
                        engine.evaluate(tool.name(), &json!({}), mode, dir.path()),
                        vak_permission::Decision::Allow
                    ),
                    "{} in {mode:?}",
                    tool.name()
                );
            }
            assert_eq!(tool.permission().reach, vak_permission::Reach::Display);
        }
    }

    #[test]
    fn every_card_tool_is_loaded_for_any_request_once_its_facts_are_held() {
        let tools: Vec<std::sync::Arc<dyn Tool>> = EmitCardTool::all()
            .into_iter()
            .map(|tool| std::sync::Arc::new(tool) as std::sync::Arc<dyn Tool>)
            .collect();
        let fresh = crate::capability::build_tool_surface(
            &tools,
            true,
            &Default::default(),
            &Default::default(),
        );
        let grounded = SHAPES
            .iter()
            .filter(|shape| shape.grounded_in.is_some())
            .count();
        assert_eq!(fresh.core.len(), SHAPES.len() - grounded);
        assert_eq!(fresh.deferred.len(), grounded);
        let every = std::collections::BTreeSet::from([
            vak_tools::Grounds::Ran,
            vak_tools::Grounds::Wrote,
            vak_tools::Grounds::Retrieved,
            vak_tools::Grounds::Answered,
        ]);
        let held = crate::capability::build_tool_surface(&tools, true, &Default::default(), &every);
        assert!(held.deferred.is_empty());
        assert_eq!(held.core.len(), SHAPES.len());
    }

    #[test]
    fn a_card_that_shows_a_call_s_output_is_grounded_in_that_call() {
        for shape in SHAPES {
            if shape.changes_made.is_some() {
                assert_eq!(
                    shape.grounded_in,
                    Some(vak_tools::Grounds::Wrote),
                    "{}",
                    shape.name
                );
            }
            if shape.output_of_command.is_some() {
                assert_eq!(
                    shape.grounded_in,
                    Some(vak_tools::Grounds::Ran),
                    "{}",
                    shape.name
                );
            }
            if shape.data_of_call.is_some() {
                assert_eq!(
                    shape.grounded_in,
                    Some(vak_tools::Grounds::Answered),
                    "{}",
                    shape.name
                );
            }
        }
    }

    #[test]
    fn a_kind_of_another_form_is_refused_with_its_tool() {
        let refused = built(
            "emit_table_card",
            json!({"semantic_type": "chart", "rows": ["A | B", "1 | 2"]}),
        )
        .unwrap_err();
        assert!(refused.contains("emit_chart_card"), "{refused}");
        let own = built(
            "emit_table_card",
            json!({"semantic_type": "table", "rows": ["A | B", "1 | 2"]}),
        );
        assert!(own.is_ok());
        let stray = built(
            "emit_table_card",
            json!({"semantic_type": "budget", "rows": ["A | B", "1 | 2"]}),
        );
        assert!(stray.is_ok(), "a kind nothing draws is dropped, not blamed");
        let info = presentation_info(
            "emit_markup_preview_card",
            &json!({"semantic_type": "media", "html": "<p>x</p>"}),
            &vak_delivery::built_in_skill_registry(),
        );
        assert!(info.is_none(), "a call is the form it was made to");
    }

    #[test]
    fn a_label_is_the_eyebrow_of_every_form() {
        let details = built(
            "emit_details_card",
            json!({"label": "Summary", "fields": [{"label": "Best", "detail": "Notion"}]}),
        )
        .unwrap();
        assert_eq!(details["label"], "Summary");
        assert_eq!(
            details["fields"][0],
            json!({"label": "Best", "detail": "Notion"})
        );
    }

    #[test]
    fn table_rows_become_the_stored_grid() {
        let payload = built("emit_table_card", json!({
            "title": "Lighthouses",
            "rows": ["| **Name** | Country | Height (m) |", "| :--- | --- | ---: |", "Tower of Hercules | Spain | 55", "Bell Rock | Scotland | 35.3"]
        }))
        .unwrap();
        assert_eq!(
            payload["columns"],
            json!([
                {"label": "Name", "numeric": false}, {"label": "Country", "numeric": false}, {"label": "Height (m)", "numeric": false}
            ])
        );
        assert_eq!(payload["rows"][1], json!(["Bell Rock", "Scotland", "35.3"]));
        let declared = built(
            "emit_table_card",
            json!({
                "rows": ["Name | Height", "Tower | 00123", "Bell | 9007199254740993"],
                "numeric_columns": ["Height"]
            }),
        )
        .unwrap();
        assert_eq!(declared["columns"][1]["numeric"], true);
        assert_eq!(
            declared["rows"],
            json!([["Tower", "00123"], ["Bell", "9007199254740993"]])
        );
        let decimal_forms = built(
            "emit_table_card",
            json!({
                "rows": ["Name | Amount", "A | .1", "B | 1.", "C | +1e-2"],
                "numeric_columns": ["Amount"]
            }),
        )
        .unwrap();
        assert_eq!(
            decimal_forms["rows"],
            json!([["A", ".1"], ["B", "1."], ["C", "+1e-2"]])
        );
        let lexical = built(
            "emit_table_card",
            json!({
                "rows": ["ID | Locale value | Large integer", "00123 | 1,5 | 9007199254740993"]
            }),
        )
        .unwrap();
        assert_eq!(
            lexical["rows"],
            json!([["00123", "1,5", "9007199254740993"]])
        );
        for (rows, numeric_columns) in [
            (json!(["Name | Amount", "A | 1,5"]), json!(["Amount"])),
            (json!(["Name | Amount", "A | NaN"]), json!(["Amount"])),
            (json!(["Name | Amount", "A | Infinity"]), json!(["Amount"])),
            (json!(["Name | Amount", "A | 1"]), json!(["Missing"])),
            (
                json!(["Name | Amount | Amount", "A | 1 | 2"]),
                json!(["Amount"]),
            ),
            (
                json!(["Name | Amount", "A | 1"]),
                json!(["Amount", "Amount"]),
            ),
        ] {
            assert!(
                built(
                    "emit_table_card",
                    json!({"rows": rows, "numeric_columns": numeric_columns})
                )
                .is_err()
            );
        }

        let mut expression = built(
            "emit_table_card",
            json!({
                "rows": ["Name | Total", "A | = left * right"]
            }),
        )
        .unwrap();
        let followable = vec![vec!["rows".into(), "*".into(), "*".into()]];
        let scope = std::collections::BTreeMap::from([
            ("left".to_string(), (2.0, "controls-1".to_string())),
            ("right".to_string(), (3.0, "controls-1".to_string())),
        ]);
        vak_delivery::controls::bind(
            "table",
            &followable,
            &mut expression,
            &scope,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(expression["rows"][0][1], 6);
        assert_eq!(expression["bound"][0]["path"], json!(["rows", 0, 1]));
        let matrix = built(
            "emit_table_card",
            json!({"rows": [["Name", "Value"], ["---", "---"], ["A", "B"]]}),
        )
        .unwrap();
        assert_eq!(
            matrix["rows"].as_array().unwrap().len(),
            2,
            "a separator-looking matrix row is data"
        );
        let one_cell =
            built("emit_table_card", json!({"rows": ["Category", "Day 1"]})).unwrap_err();
        assert!(one_cell.contains("emit_list_card"), "{one_cell}");
        let text = built("emit_table_card", json!({"rows": "A | B\n1 | 2"})).unwrap();
        assert_eq!(text["rows"], json!([["1", "2"]]));
    }

    #[test]
    fn steps_carry_durations_and_options() {
        let payload = built(
            "emit_steps_card",
            json!({
                "steps": [{"label": "Boil", "duration_minutes": "5"}, "Brew"],
                "choices": [{"step": "Brew", "label": "Press", "facts": "4 min; coarse"}],
                "total_duration_minutes": 5
            }),
        );
        let refused = payload.unwrap_err();
        assert!(refused.contains("step 2 has no duration"), "{refused}");
        let payload = built(
            "emit_steps_card",
            json!({
                "steps": [{"label": "Boil", "duration_minutes": "5", "time": "9:00"}, "Brew"],
                "choices": [{"step": "Brew", "label": "Press", "facts": "4 min; coarse"}]
            }),
        )
        .unwrap();
        assert_eq!(payload["steps"][0]["time"], "9:00 · 5 min");
        assert_eq!(
            payload["steps"][1]["choices"][0]["facts"],
            json!(["4 min", "coarse"])
        );
        let wrong = built(
            "emit_steps_card",
            json!({"steps": ["A"], "choices": [{"step": "B", "label": "x"}]}),
        )
        .unwrap_err();
        assert!(wrong.contains("not the label of any step"), "{wrong}");
    }

    #[test]
    fn charts_require_explicit_geometry_and_preserve_missing_observations() {
        let chart = built(
            "emit_chart_card",
            json!({"chart_type": "line", "x_scale": "number", "data": [
                {"series": "A", "x": "1", "y": 2}, {"series": "A", "x": "10", "y": null}
            ]}),
        )
        .unwrap();
        assert_eq!(chart["x_scale"], "number");
        assert!(chart["series"][0]["points"][1]["y"].is_null());
        for (scale, x) in [
            ("number", "001"),
            ("number", "1,5"),
            ("time", "2026-01-01"),
            ("other", "x"),
        ] {
            assert!(
                built(
                    "emit_chart_card",
                    json!({"chart_type": "line", "x_scale": scale,
                "data": [{"series": "A", "x": x, "y": 1}]})
                )
                .is_err()
            );
        }
        assert!(
            built(
                "emit_chart_card",
                json!({"chart_type": "line", "x_scale": "time",
            "data": [{"series": "A", "x": "2026-01-01T00:00:00Z", "y": 1}]})
            )
            .is_ok()
        );
        assert!(
            built(
                "emit_chart_card",
                json!({"chart_type": "line", "data": [
                    {"series": "A", "x": "x", "y": 1}, {"series": "A", "x": "x", "y": 2}
                ]})
            )
            .is_err()
        );
    }

    #[test]
    fn a_chart_groups_points_into_series_and_summarises_itself() {
        let payload = built("emit_chart_card", json!({"chart_type": "line", "data": [
            {"series": "A", "x": "2024", "y": 1000}, {"series": "B", "x": "2024", "y": 2}, {"series": "A", "x": "2025", "y": 3}
        ]}))
        .unwrap();
        assert_eq!(
            payload["series"][0]["points"],
            json!([{"x": "2024", "y": 1000}, {"x": "2025", "y": 3}])
        );
        assert!(
            payload["accessible_summary"]
                .as_str()
                .unwrap()
                .contains("A: 1000 at 2024 to 3 at 2025")
        );
        let refused = built(
            "emit_chart_card",
            json!({"chart_type": "bar", "data": [{"series": "A", "x": "Q1", "y": "lots"}]}),
        )
        .unwrap_err();
        assert!(refused.contains("must be a number"), "{refused}");
    }

    #[test]
    fn chart_categories_keep_their_lexical_identity_and_values_require_numbers() {
        let categories = ["00123", "1,5", "1,234.50", "9007199254740993"];
        let data: Vec<Value> = categories
            .iter()
            .map(|category| json!({"series": "A", "x": category, "y": 1}))
            .collect();
        let payload = built(
            "emit_chart_card",
            json!({"chart_type": "bar", "data": data}),
        )
        .unwrap();
        let points = payload["series"][0]["points"].as_array().unwrap();
        for (point, category) in points.iter().zip(categories) {
            assert_eq!(point["x"], category);
        }
        for value in ["1,5", "1,000"] {
            let error = built(
                "emit_chart_card",
                json!({"chart_type": "bar", "data": [{"series": "A", "x": "A", "y": value}]}),
            )
            .unwrap_err();
            assert!(error.contains("must be a number"), "{error}");
        }

        let mut ability = json!({"from": "source", "call_id": "c6", "chart_type": "bar", "x": "id", "y": "amount"});
        tool("emit_ability_chart_card")
            .fill_from_data(&mut ability, &[json!({"id": "00123", "amount": 2})])
            .unwrap();
        let payload = built("emit_ability_chart_card", ability).unwrap();
        assert_eq!(payload["series"][0]["points"][0]["x"], "00123");
    }

    #[test]
    fn comparison_aspects_are_one_cell_for_each_option() {
        let payload = built("emit_comparison_card", fixture_for("comparison")).unwrap();
        assert_eq!(
            payload["aspects"],
            json!([{"label": "Effort", "values": ["Low", "High"]}])
        );
        assert_eq!(
            payload["options"][0]["strengths"],
            json!(["Easy", "familiar"])
        );
        let listed = built(
            "emit_comparison_card",
            json!({"options": [{"name": "A", "strengths": ["x", "y"]}, {"name": "B"}]}),
        )
        .unwrap();
        assert_eq!(listed["options"][0]["strengths"], json!(["x", "y"]));
        assert!(built("emit_comparison_card", json!({"options": [{"name": "A"}]})).is_err());
    }

    #[test]
    fn a_calendar_is_in_date_order_and_refuses_unreadable_dates() {
        let payload = built("emit_calendar_card", json!({"events": [
            {"title": "B", "date": "2026-10-16"}, {"title": "A", "date": "2026-10-15", "start": "10:00"}
        ]}))
        .unwrap();
        assert_eq!(payload["events"][0]["title"], "A");
        assert!(
            built(
                "emit_calendar_card",
                json!({"events": [{"title": "A", "date": "Thursday"}]})
            )
            .unwrap_err()
            .contains("YYYY-MM-DD")
        );
        assert!(
            built(
                "emit_calendar_card",
                json!({"events": [{"title": "A", "date": "2026-10-15", "start": "9am"}]})
            )
            .unwrap_err()
            .contains("HH:MM")
        );
    }

    #[test]
    fn a_map_reads_positions_and_refuses_what_it_cannot_place() {
        let payload = built(
            "emit_map_card",
            json!({"connect": "none", "places": [
                {"label": "A", "position": " -33.8568 , 151.2153 "},
                {"label": "B", "latitude": 51.5, "longitude": "-0.12"}
            ]}),
        )
        .unwrap();
        assert_eq!(payload["places"][0]["latitude"], json!(-33.8568));
        assert_eq!(payload["places"][1]["longitude"], json!(-0.12));
        for bad in ["Sydney", "151.2, 95.0", "33.8 S, 151.2 E"] {
            assert!(
                built(
                    "emit_map_card",
                    json!({"connect": "none", "places": [{"label": "A", "position": bad}]})
                )
                .unwrap_err()
                .contains("decimal degrees"),
                "{bad}"
            );
        }
        assert!(
            built(
                "emit_map_card",
                json!({"connect": "area", "places": [
                    {"label": "A", "position": "1, 1"}, {"label": "B", "position": "1, 2"}]})
            )
            .unwrap_err()
            .contains("at least three")
        );
    }

    #[test]
    fn lists_and_stats_read_their_plain_shapes() {
        let list = built(
            "emit_list_card",
            json!({"items": ["Rice", {"label": "Tins", "url": "https://a.example"}]}),
        )
        .unwrap();
        assert_eq!(
            list["items"],
            json!([{"label": "Rice"}, {"label": "Tins", "url": "https://a.example"}])
        );
        let stats = built(
            "emit_stats_card",
            json!({"readings": {"Uptime": "99.9", "Users": 4}}),
        )
        .unwrap();
        assert_eq!(
            stats["readings"],
            json!([{"label": "Uptime", "value": "99.9"}, {"label": "Users", "value": "4"}])
        );
        assert!(
            built(
                "emit_stats_card",
                json!({"readings": [{"label": "A", "value": 1}, {"label": "a", "value": 2}]})
            )
            .is_err()
        );
    }

    #[test]
    fn media_and_links_fill_what_they_can_from_the_address() {
        let media = built(
            "emit_media_card",
            json!({"format": "image", "url": "https://a.example/cat.png?x=1"}),
        )
        .unwrap();
        assert_eq!(media["alt"], "cat.png");
        let link = built("emit_link_card", json!({"url": "https://news.example/a/b"})).unwrap();
        assert_eq!(link["title"], "news.example");
    }

    #[test]
    fn a_test_report_agrees_with_how_its_run_ended() {
        let report = |exit: Value, status: &str| {
            let mut args = json!({"command": "cargo test", "duration_ms": 900,
                "tests": [{"name": "a", "status": "passed"}, {"name": "b", "status": status}]});
            if !exit.is_null() {
                args["exit_code"] = exit;
            }
            build_test_report(args.as_object().unwrap())
        };
        assert!(report(Value::from(0), "passed").is_ok());
        assert!(report(Value::from(101), "failed").is_ok());
        assert!(
            report(Value::from(101), "passed")
                .unwrap_err()
                .contains("exited 101")
        );
        assert!(
            report(Value::Null, "passed")
                .unwrap_err()
                .contains("stopped")
        );
        let never = build_test_report(
            json!({"command": "cargo test", "tests": []})
                .as_object()
                .unwrap(),
        )
        .unwrap_err();
        assert!(never.contains("no call"), "{never}");
    }

    #[test]
    fn an_unknown_field_is_named_with_the_fields_the_tool_takes() {
        let refused = built(
            "emit_callout_card",
            json!({"tone": "tip", "text": "x", "colour": "red"}),
        )
        .unwrap_err();
        assert!(
            refused.contains("`colour`") && refused.contains("`text`"),
            "{refused}"
        );
    }

    fn library_with_pack(enabled: bool) -> vak_presentation::PresentationLibrary {
        let spec: vak_presentation::PresentationSpec = serde_json::from_value(json!({
            "schema_version": 1,
            "id": "garden.brief",
            "revision": 1,
            "accepts": ["garden.brief"],
            "block": "sources",
            "root": {"primitive": "sources", "props": {"*": {"kind": "binding", "path": "$"}}}
        }))
        .unwrap();
        let digest = vak_presentation::digest(&spec).unwrap();
        let mut library = vak_presentation::PresentationLibrary::default();
        library
            .register(vak_presentation::StoredPresentation {
                spec,
                digest,
                origin: vak_presentation::PresentationOrigin {
                    scope: vak_presentation::LibraryScope::Workspace,
                    owner: "/w".into(),
                    plugin_id: Some("garden".into()),
                    generation: None,
                },
                enabled,
            })
            .unwrap();
        if enabled {
            library
                .activate(
                    "garden.brief",
                    1,
                    vak_presentation::LibraryScope::Workspace,
                    "/w",
                )
                .unwrap();
        }
        library
    }

    /// A kind an active pack names for a form is a kind of that form's card
    /// tool, built by the form and recorded with it; a disabled pack adds
    /// nothing (docs/design/91-presentation-capabilities.md §2).
    #[test]
    fn an_active_pack_adds_its_kinds_to_its_block() {
        let packs = PackKinds::from_library(&library_with_pack(true), "/w");
        let sources = EmitCardTool::with_packs(&packs)
            .into_iter()
            .find(|tool| tool.name() == "emit_sources_card")
            .unwrap();
        assert_eq!(
            sources.card_kinds(),
            vec!["sources".to_string(), "garden.brief".to_string()]
        );
        let mut args = fixture_for("sources");
        args["semantic_type"] = json!("garden.brief");
        let skills = vak_delivery::built_in_skill_registry();
        let info = presentation_info_with("emit_sources_card", &args, &skills, &packs).unwrap();
        assert_eq!(
            (info.semantic_type.as_str(), info.block.as_deref()),
            ("garden.brief", Some("sources"))
        );
        assert_eq!(
            presentation_info("emit_sources_card", &args, &skills)
                .unwrap()
                .semantic_type,
            "sources"
        );
        assert_eq!(
            PackKinds::from_library(&library_with_pack(false), "/w"),
            PackKinds::default()
        );
    }

    /// Every pack a package in `packages/` ships names a real form, and
    /// compiles to a rich card from that form's own payload.
    #[test]
    fn every_package_pack_compiles_from_its_block_payload() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages");
        let skills = vak_delivery::built_in_skill_registry();
        let mut checked = 0;
        for package in std::fs::read_dir(&root).unwrap() {
            let Ok(files) = std::fs::read_dir(package.unwrap().path().join("presentation")) else {
                continue;
            };
            for file in files {
                let path = file.unwrap().path();
                let spec = vak_presentation::parse_spec(&std::fs::read(&path).unwrap())
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                let block = spec
                    .block
                    .clone()
                    .unwrap_or_else(|| panic!("{} names no block", path.display()));
                let shape = shape_of_form(&block)
                    .unwrap_or_else(|| panic!("{}: no form {block}", path.display()));
                for kind in &spec.accepts {
                    assert!(!vak_delivery::is_form(kind), "{kind} is a built-in form");
                    let mut args = fixture_for(shape.form);
                    args["semantic_type"] = json!(kind);
                    let packs = PackKinds {
                        kinds: [(block.clone(), vec![kind.clone()])].into(),
                        packs: Vec::new(),
                    };
                    let info = presentation_info_with(shape.name, &args, &skills, &packs)
                        .unwrap_or_else(|| panic!("{kind}: the {block} form refused it"));
                    let compiled = vak_presentation::compile(
                        &spec,
                        &vak_presentation::CompileInput {
                            semantic_type: kind.clone(),
                            payload: info.payload,
                            fallback_text: info.title,
                        },
                    );
                    assert!(
                        matches!(compiled, vak_presentation::CompiledPresentation::Rich(_)),
                        "{}: {kind}: {compiled:?}",
                        path.display()
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn identity_digests_name_what_tells_cards_apart() {
        let skills = vak_delivery::built_in_skill_registry();
        let digest = |form: &str| {
            presentation_info(&format!("emit_{form}_card"), &fixture_for(form), &skills)
                .unwrap()
                .identity_digest
        };
        assert!(digest("table").contains("columns: Name,Role | rows: 1"));
        assert!(digest("sources").contains("Index (https://example.com/index): Adoption rose."));
        assert!(digest("chart").contains("series: Share (%)(2)"));
        assert!(digest("calendar").contains("2026-10-15 Morning standup"));
        assert!(digest("steps").contains("steps: Boil water,Brew"));
        assert!(digest("list").contains("items: Rice,Tins"));
        assert!(digest("stats").contains("Uptime"));
        assert!(digest("markup_preview").contains("keys: html"));
    }

    #[test]
    fn presentation_info_is_none_for_an_invalid_call() {
        let skills = vak_delivery::built_in_skill_registry();
        assert!(
            presentation_info(
                "emit_chart_card",
                &json!({"chart_type": "pie", "data": []}),
                &skills
            )
            .is_none()
        );
        assert!(presentation_info("emit_nothing_card", &json!({}), &skills).is_none());
        let fence = presentation_info("callout", &fixture_for("callout"), &skills).unwrap();
        assert_eq!(fence.block.as_deref(), Some("callout"));
    }
}
