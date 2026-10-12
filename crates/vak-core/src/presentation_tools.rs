//! The card tools of an answer (docs/design/87-composed-answers.md).
//!
//! Each `CardShape` is one card tool, called by its own name: its flat
//! arguments, the stored card its builder makes from them, and every
//! `semantic_type` drawn from that card (the block's own kind first, then
//! each type whose renderer and pack read the same payload). It is a `Tool`
//! so a turn admits it like any other capability, a channel policy can
//! narrow it, and a plugin can add one; the loop shows its cards
//! (`show_card` in vak-agent).
//!
//! The precise flat schema per shape is what makes cards reliable, measured
//! on 2026-09-18 against a local model (free-text fences valid 1 time in 5,
//! a precise per-shape schema 5 in 5, one loose schema worse than fences)
//! and again on 2026-10-12 against one nested call of every block
//! (docs/audits/cards-in-answers-2026-10-11.md). One argument name is one
//! shape on every card tool, so the same idea reads the same everywhere.
//!
//! `execute()` checks a card against the real `SkillRegistry`; the card is
//! recorded as a `Presentation` from the call's own arguments, never from
//! result text.

use async_trait::async_trait;
use serde_json::Value;
use vak_tools::Fault;
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

/// One card tool: the flat arguments a model writes, and how they become the
/// stored card.
///
/// The arguments follow four rules, each measured live (docs/design/
/// 30-render-architecture.md §30.1): the card's fields sit at the top level
/// beside `semantic_type`, never inside a `payload` envelope (13 of 16
/// dropped calls left that envelope open); the schema uses only `type`,
/// `properties`, `items`, `required`, `enum` and `description`, the keywords
/// every provider carries to a model (Ollama drops `oneOf`,
/// `additionalProperties` and bounds, so a shape stated only through them
/// reached the model as a bare object); no data travels as an object key (a
/// label such as `Year Built` is not a key every call syntax can write); and
/// no record holds a list of records (a timeline's options and a chart's
/// points are their own lists, joined by a name). The model-facing table
/// schema stays a flat list of Markdown row strings; the table tool also
/// canonicalizes the conventional matrix form before validation. The stored card, and
/// every renderer, digest and channel that reads it, is unchanged: `build`
/// turns the arguments into it.
struct CardShape {
    /// Tool name, e.g. `emit_chart_card`.
    name: &'static str,
    description: &'static str,
    /// Every `semantic_type` this shape can produce: the block's own kind
    /// first, then each type whose renderer and pack read the payload this
    /// shape builds (docs/audits/cards-in-answers-2026-10-11.md). Each is a
    /// kind the tool shows. `SkillRegistry` re-validates the type
    /// server-side, so a list drifting from skills.rs fails closed.
    semantic_types: &'static [&'static str],
    /// The type a call that names none takes; `None` where the kinds are
    /// unlike things (an image or a video), so the call must name one.
    default_type: Option<&'static str>,
    /// This shape's own reading of the arguments before the schema check
    /// (`Tool::fill_defaults`): repairs only this shape can make.
    fill: fn(&mut Map<String, Value>),
    /// Where the card's `label`, the model's own words for it, is kept.
    eyebrow: Eyebrow,
    /// The tool's arguments beside `semantic_type`, as
    /// `{"properties": {...}, "required": [...]}`.
    fields: fn() -> Value,
    /// The stored card payload for validated arguments, or a refusal the
    /// model can act on.
    build: fn(&Map<String, Value>) -> Result<Value, String>,
}

type Map<K, V> = serde_json::Map<K, V>;

/// Where a card keeps its `label`. A payload whose keys are its facts
/// (`details` and its kinds) has no key of its own for a heading, so the
/// label is its title there, never one of the facts.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Eyebrow {
    /// Beside the payload's own fields, as `label`.
    Label,
    /// As the payload's `title`, when the card gave none.
    Title,
}

fn string(description: &str) -> Value {
    if description.is_empty() {
        serde_json::json!({"type": "string"})
    } else {
        serde_json::json!({"type": "string", "description": description})
    }
}

fn integer(description: &str) -> Value {
    if description.is_empty() {
        serde_json::json!({"type": "integer"})
    } else {
        serde_json::json!({"type": "integer", "description": description})
    }
}

fn list_of(description: &str, properties: Value, required: &[&str]) -> Value {
    serde_json::json!({
        "type": "array",
        "description": description,
        "items": {"type": "object", "properties": properties, "required": required}
    })
}

fn fields(properties: Value, required: &[&str]) -> Value {
    serde_json::json!({"properties": properties, "required": required})
}

/// The arguments' own fields that the stored card keeps as they are.
fn copy(args: &Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    keys.iter()
        .filter_map(|key| {
            args.get(*key)
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect()
}

fn records<'a>(args: &'a Map<String, Value>, key: &str) -> &'a [Value] {
    args.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// A value the schema asks for as text: a number written as text becomes a
/// JSON number, a number sent as a number stays one, anything else is text.
fn scalar(value: &Value) -> Value {
    match value {
        Value::Number(_) => value.clone(),
        Value::String(text) => number_or_text(text),
        Value::Null => Value::String(String::new()),
        other => Value::String(other.to_string()),
    }
}

/// A number written as text, as a JSON number; anything else stays text.
fn number_or_text(text: &str) -> Value {
    let trimmed = text.trim();
    let plain = trimmed.replace(',', "");
    match plain.parse::<f64>() {
        Ok(n) if n.is_finite() && !plain.is_empty() => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                Value::from(n as i64)
            } else {
                Value::from(n)
            }
        }
        _ => Value::String(trimmed.to_string()),
    }
}

fn details_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
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
    let mut payload = copy(args, &["title"]);
    if let Some(text) = args.get("text") {
        payload.insert("summary".into(), text.clone());
    }
    let facts = args
        .get("fields")
        .and_then(Value::as_array)
        .filter(|facts| !facts.is_empty())
        .ok_or("fields has no items; give each fact as {label, detail}")?;
    for (at, fact) in facts.iter().enumerate() {
        let text = |key: &str| {
            fact.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
        };
        let label = text("label");
        if label.is_empty() {
            return Err(format!("field {} has no label", at + 1));
        }
        if payload.contains_key(label) {
            return Err(format!(
                "two fields are labelled `{label}`; give each its own label"
            ));
        }
        payload.insert(label.to_string(), Value::String(text("detail").to_string()));
    }
    Ok(Value::Object(payload))
}

fn research_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "sources": list_of(
                "The retrieved sources, in order, each with what it found.",
                serde_json::json!({
                    "title": string("Copy the source title from the retrieved result."),
                    "url": string("Copy the exact source URL from retrieved evidence, not a guessed homepage or search URL."),
                    "finding": string("What this source found that answers the request, in one sentence, supported by its text."),
                    "source_name": string("Optional; include only if the retrieved result identifies this publisher."),
                    "published_at": string("Optional; include only when the retrieved result supplies a publication date.")
                }),
                &["title", "url", "finding"],
            )
        }),
        &["sources"],
    )
}

/// Each source's `finding` becomes a takeaway citing that source: one list
/// of flat records, each ending in a string (docs/design/30-render-architecture.md
/// §30.1), where a separate takeaway list numbered by source ended records
/// in an integer the model often left unclosed.
fn build_research(args: &Map<String, Value>) -> Result<Value, String> {
    let mut payload = copy(args, &["title"]);
    let mut sources = Vec::new();
    let mut takeaways = Vec::new();
    for (at, source) in records(args, "sources").iter().enumerate() {
        let mut kept = source.clone();
        if let Some(fields) = kept.as_object_mut() {
            fields.remove("finding");
        }
        sources.push(kept);
        takeaways
            .push(serde_json::json!({"text": source["finding"], "citation_indices": [at + 1]}));
    }
    payload.insert("sources".into(), Value::Array(sources));
    payload.insert("takeaways".into(), Value::Array(takeaways));
    Ok(Value::Object(payload))
}

fn diff_fields() -> Value {
    fields(
        serde_json::json!({
            "files": list_of(
                "One entry per changed file.",
                serde_json::json!({
                    "filename": string("The file's workspace path."),
                    "additions": integer("Lines added, 0 or more."),
                    "deletions": integer("Lines removed, 0 or more."),
                    "hunks": string("Unified diff hunk text for this file.")
                }),
                &["filename", "hunks", "additions", "deletions"],
            )
        }),
        &["files"],
    )
}

fn build_diff(args: &Map<String, Value>) -> Result<Value, String> {
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
            "suite_name": string("Optional. The suite or command that ran."),
            "tests": list_of(
                "One entry per test that ran.",
                serde_json::json!({
                    "name": string("The test's name."),
                    "status": {
                        "type": "string",
                        "enum": ["passed", "failed", "skipped"],
                        "description": "How the test ended."
                    },
                    "duration_ms": integer("Optional. How long it took, in milliseconds."),
                    "message": string("Optional. The failure or skip message.")
                }),
                &["name", "status"],
            )
        }),
        &["tests"],
    )
}

fn build_test_report(args: &Map<String, Value>) -> Result<Value, String> {
    if records(args, "tests").iter().any(|test| {
        test.get("duration_ms")
            .is_some_and(|ms| ms.as_u64().is_none())
    }) {
        return Err("`duration_ms` must be a whole number, 0 or more".into());
    }
    Ok(Value::Object(copy(args, &["suite_name", "tests"])))
}

fn terminal_fields() -> Value {
    fields(
        serde_json::json!({
            "command": string("Optional. The command as it was run."),
            "output": string("What it printed."),
            "exit_code": integer("Optional. Its exit code."),
            "duration_ms": integer("Optional. How long it took, in milliseconds.")
        }),
        &["output"],
    )
}

fn build_terminal(args: &Map<String, Value>) -> Result<Value, String> {
    Ok(Value::Object(copy(
        args,
        &["command", "output", "exit_code", "duration_ms"],
    )))
}

fn table_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "rows": {
                "type": "array",
                "items": {
                    "type": "string",
                    "description": "One row: one cell for each header, in the same order, separated by |."
                },
                "description": "The table as separate strings in display order: the header first, then one string for each row. Example: [\"Item | Amount\", \"Rice | 2 kg\"]."
            }
        }),
        &["rows"],
    )
}

fn build_table(args: &Map<String, Value>) -> Result<Value, String> {
    let lines = table_lines(args.get("rows").and_then(Value::as_array).ok_or_else(|| {
        "`rows` must be an array of row strings or arrays of string cells".to_string()
    })?)?;
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (labels, rows) = table_rows(&lines)?;
    if labels.len() < 2 {
        return Err(format!(
            "every row is one cell, so this is a list, not a table: put a row's cells in one string separated by |, the header first (\"{} | …\"), then one string per row",
            labels.first().map_or("Column", String::as_str)
        ));
    }
    let numeric: Vec<bool> = (0..labels.len())
        .map(|column| {
            let cells: Vec<&str> = rows
                .iter()
                .map(|row| row[column].as_str())
                .filter(|cell| !cell.is_empty())
                .collect();
            !cells.is_empty() && cells.iter().all(|cell| number_or_text(cell).is_number())
        })
        .collect();
    let columns: Vec<Value> = labels
        .iter()
        .zip(&numeric)
        .map(|(label, numeric)| {
            let mut column = serde_json::json!({"key": label, "label": label});
            if *numeric {
                column["isNumeric"] = Value::Bool(true);
            }
            column
        })
        .collect();
    let rows: Vec<Value> = rows
        .iter()
        .map(|row| {
            Value::Object(
                labels
                    .iter()
                    .zip(row)
                    .zip(&numeric)
                    .map(|((label, cell), numeric)| {
                        let value = if *numeric && !cell.is_empty() {
                            number_or_text(cell)
                        } else {
                            Value::String(cell.clone())
                        };
                        (label.clone(), value)
                    })
                    .collect(),
            )
        })
        .collect();
    let mut payload = copy(args, &["title"]);
    payload.insert("columns".into(), Value::Array(columns));
    payload.insert("rows".into(), Value::Array(rows));
    Ok(Value::Object(payload))
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
fn table_rows(lines: &[&str]) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
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
    let mut labels: Vec<String> = Vec::new();
    for (at, label) in rows[0].iter().enumerate() {
        let base = if label.is_empty() {
            format!("Column {}", at + 1)
        } else {
            label.clone()
        };
        let mut unique = base.clone();
        let mut n = 2;
        while labels.contains(&unique) {
            unique = format!("{base} ({n})");
            n += 1;
        }
        labels.push(unique);
    }
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
    Ok((labels, data))
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

fn duration_from_item_text(item: &Map<String, Value>) -> Option<u64> {
    let from_field = |name: &str| {
        item.get(name)
            .and_then(Value::as_str)
            .and_then(duration_from_text)
    };
    let durations = ["time", "detail"]
        .into_iter()
        .filter_map(from_field)
        .collect::<Vec<_>>();
    match durations.as_slice() {
        [duration] => Some(*duration),
        [first, rest @ ..] if rest.iter().all(|duration| duration == first) => Some(*first),
        _ => None,
    }
}

fn duration_from_text(text: &str) -> Option<u64> {
    let words = text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let mut durations = Vec::new();
    for pair in words.windows(2) {
        let Ok(value) = pair[0].parse::<u64>() else {
            continue;
        };
        let unit = pair[1].to_ascii_lowercase();
        let minutes = match unit.as_str() {
            "min" | "mins" | "minute" | "minutes" => Some(value),
            "hr" | "hrs" | "hour" | "hours" => value.checked_mul(60),
            _ => None,
        };
        if let Some(minutes) = minutes.filter(|minutes| *minutes > 0) {
            durations.push(minutes);
        }
    }
    (durations.len() == 1).then(|| durations[0])
}

fn build_timeline(args: &Map<String, Value>) -> Result<Value, String> {
    let mut items: Vec<Value> = records(args, "items").to_vec();
    let total_duration =
        match args.get("total_duration_minutes") {
            Some(value) => Some(value.as_u64().filter(|minutes| *minutes > 0).ok_or_else(
                || "`total_duration_minutes` must be a whole number greater than zero".to_string(),
            )?),
            None => None,
        };
    let mut summed_duration = 0_u64;
    for (index, item) in items.iter_mut().enumerate() {
        let duration = match item.get("duration_minutes") {
            Some(value) => Some(value.as_u64().filter(|minutes| *minutes > 0).ok_or_else(
                || {
                    format!(
                        "step {} `duration_minutes` must be a whole number greater than zero",
                        index + 1
                    )
                },
            )?),
            None => None,
        };
        if let Some(duration) = duration {
            summed_duration = summed_duration
                .checked_add(duration)
                .ok_or_else(|| "the step durations exceed the supported total".to_string())?;
            let object = item
                .as_object_mut()
                .ok_or_else(|| format!("step {} must be an object", index + 1))?;
            object.remove("duration_minutes");
            if let Some(detail) = object
                .get("detail")
                .and_then(Value::as_str)
                .and_then(|detail| strip_duration_prefix(detail, duration))
            {
                object.insert("detail".into(), Value::String(detail));
            }
            let duration_text = format!("{duration} min");
            let time = object
                .get("time")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|time| !time.is_empty())
                .map(|time| format!("{time} · {duration_text}"))
                .unwrap_or(duration_text);
            object.insert("time".into(), Value::String(time));
        } else if total_duration.is_some() {
            return Err(format!(
                "step {} has no duration, but `total_duration_minutes` is set; give every step `duration_minutes` so they add up to it",
                index + 1,
            ));
        }
    }
    if let Some(expected) = total_duration
        && summed_duration != expected
    {
        return Err(format!(
            "the step durations add up to {summed_duration} minutes, but `total_duration_minutes` is {expected}; adjust the item durations so they sum exactly"
        ));
    }
    for option in records(args, "options") {
        let step = option["step"].as_str().unwrap_or_default().trim();
        let found = items
            .iter()
            .position(|item| {
                item["label"]
                    .as_str()
                    .is_some_and(|label| label.trim() == step)
            })
            .or_else(|| {
                items.iter().position(|item| {
                    item["label"]
                        .as_str()
                        .is_some_and(|label| label.trim().eq_ignore_ascii_case(step))
                })
            })
            .ok_or_else(|| {
                format!(
                    "option `{}` names step `{step}`, which is not the label of any item",
                    option["label"].as_str().unwrap_or_default()
                )
            })?;
        let mut choice = copy(
            option.as_object().unwrap_or(&Map::new()),
            &["label", "detail"],
        );
        let facts: Vec<Value> = option["facts"]
            .as_str()
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|fact| !fact.is_empty())
            .map(|fact| Value::String(fact.to_string()))
            .collect();
        if !facts.is_empty() {
            choice.insert("facts".into(), Value::Array(facts));
        }
        let item = &mut items[found];
        match item.get_mut("options").and_then(Value::as_array_mut) {
            Some(options) => options.push(Value::Object(choice)),
            None => item["options"] = Value::Array(vec![Value::Object(choice)]),
        }
    }
    let mut payload = copy(args, &["title"]);
    payload.insert("items".into(), Value::Array(items));
    Ok(Value::Object(payload))
}

fn strip_duration_prefix(detail: &str, minutes: u64) -> Option<String> {
    let mut units = vec![
        format!("{minutes} minutes"),
        format!("{minutes} minute"),
        format!("{minutes} mins"),
        format!("{minutes} min"),
    ];
    if minutes.is_multiple_of(60) {
        units.extend([
            format!("{} hours", minutes / 60),
            format!("{} hour", minutes / 60),
            format!("{} hrs", minutes / 60),
            format!("{} hr", minutes / 60),
        ]);
    }
    let lower = detail.to_ascii_lowercase();
    let rest = units
        .iter()
        .find_map(|unit| lower.strip_prefix(unit).map(|_| &detail[unit.len()..]))?;
    Some(
        rest.trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, ':' | '-' | '–' | '·' | '(')
        })
        .trim_end_matches(')')
        .trim()
        .to_string(),
    )
}

fn recipe_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "servings": integer("Optional. How many it serves."),
            "prep_time_minutes": integer("Optional. Minutes of preparation."),
            "cook_time_minutes": integer("Optional. Minutes of cooking."),
            "ingredients": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Each ingredient with its amount, for example 1.5 cups flour."
            },
            "method": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Each step of the method, in order."
            }
        }),
        &["title", "ingredients", "method"],
    )
}

fn build_recipe(args: &Map<String, Value>) -> Result<Value, String> {
    if args
        .get("servings")
        .is_some_and(|n| !n.as_u64().is_some_and(|n| (1..=10_000).contains(&n)))
    {
        return Err("`servings` must be a whole number from 1 to 10000".into());
    }
    let mut payload = copy(
        args,
        &[
            "title",
            "servings",
            "prep_time_minutes",
            "cook_time_minutes",
            "ingredients",
        ],
    );
    if let Some(method) = args.get("method") {
        payload.insert("steps".into(), method.clone());
    }
    Ok(Value::Object(payload))
}

fn ui_preview_fields() -> Value {
    fields(
        serde_json::json!({
            "status": string("Optional. The state of the work, in a word."),
            "title": string(""),
            "artifact_path": string("The workspace path of the file to preview. Give this or html."),
            "html": string("The markup to preview, when there is no file. Give this or artifact_path.")
        }),
        &[],
    )
}

fn build_ui_preview(args: &Map<String, Value>) -> Result<Value, String> {
    let given = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    };
    if !given("artifact_path") && !given("html") {
        return Err("nothing to preview; give artifact_path or html".into());
    }
    Ok(Value::Object(copy(
        args,
        &["status", "title", "artifact_path", "html"],
    )))
}

fn chart_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "chart_type": {
                "type": "string",
                "enum": ["line", "bar", "area"],
                "description": "How to draw it."
            },
            "x_label": string("Optional. What the x axis measures."),
            "y_label": string("Optional. What the y axis measures."),
            "accessible_summary": string("Optional: one sentence saying what the chart shows."),
            "data": list_of(
                "Every point of every series.",
                serde_json::json!({
                    "series": string("The name of the series this point belongs to."),
                    "x": string("The x value: a category or a number, as text."),
                    "y": {"type": "number", "description": "The value at this point."}
                }),
                &["series", "x", "y"],
            )
        }),
        &["chart_type", "data"],
    )
}

fn build_chart(args: &Map<String, Value>) -> Result<Value, String> {
    let mut series: Vec<(String, Vec<Value>)> = Vec::new();
    for point in records(args, "data") {
        let name = point["series"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        let x = scalar(&point["x"]);
        // A y written as text ("24,850.3") is still a number; anything
        // else cannot be drawn, and saying so lets the call be repaired.
        let y = match scalar(&point["y"]) {
            number @ Value::Number(_) => number,
            _ => {
                return Err(format!(
                    "each point's `y` must be a number; series `{name}` at x `{}` has none",
                    point["x"]
                        .as_str()
                        .map_or_else(|| point["x"].to_string(), str::to_string)
                ));
            }
        };
        let entry = serde_json::json!({"x": x, "y": y});
        match series.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, points)) => points.push(entry),
            None => series.push((name, vec![entry])),
        }
    }
    // A chart with no points is an empty frame, summary or not (found on a
    // deployed host, 2026-10-10: two cards said "No data points supplied").
    if series.is_empty() {
        return Err("give `data`: every point, each with its series, x and y".into());
    }
    let mut payload = copy(
        args,
        &[
            "title",
            "chart_type",
            "x_label",
            "y_label",
            "accessible_summary",
        ],
    );
    if !payload
        .get("accessible_summary")
        .and_then(Value::as_str)
        .is_some_and(|summary| !summary.trim().is_empty())
    {
        payload.insert(
            "accessible_summary".into(),
            Value::String(chart_summary(args, &series)),
        );
    }
    payload.insert(
        "series".into(),
        Value::Array(
            series
                .into_iter()
                .map(|(name, points)| serde_json::json!({"name": name, "points": points}))
                .collect(),
        ),
    );
    Ok(Value::Object(payload))
}

/// What a chart shows, in words, from its own data: each series with its
/// first and last point. The stored chart needs one (it is the chart's text
/// form on a surface that draws none); measured on gemma4, 30 of 32 chart
/// calls left the sentence out, with every point right.
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
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Chart");
    format!("{title}. {}.", lines.join("; "))
}

fn media_fields() -> Value {
    fields(
        serde_json::json!({
            "url": string("The page's URL for link.preview; the file's URL or workspace path for media.image, media.video and media.audio."),
            "title": string(""),
            "thumbnail_url": string("For link.preview only: a thumbnail image for the page."),
            "description": string("For link.preview: a short description of the page."),
            "site_name": string("For link.preview: the site's name."),
            "alt": string("For media.image, media.video and media.audio: what it shows, in words.")
        }),
        &[],
    )
}

fn build_media(args: &Map<String, Value>) -> Result<Value, String> {
    Ok(Value::Object(copy(
        args,
        &[
            "url",
            "title",
            "thumbnail_url",
            "description",
            "site_name",
            "alt",
        ],
    )))
}

/// Models sometimes fill an optional unit with a placeholder. Do not surface
/// that placeholder as if it were a measurement unit on a metric card.
fn metric_unit(value: &Value) -> Option<&str> {
    value.as_str().map(str::trim).filter(|unit| {
        !unit.is_empty()
            && !["n/a", "none", "not applicable", "—", "-"]
                .iter()
                .any(|placeholder| unit.eq_ignore_ascii_case(placeholder))
    })
}

// ---- composed-answer blocks (docs/design/87-composed-answers.md) ----------

fn lead_fields() -> Value {
    fields(
        serde_json::json!({
            "headline": string("The answer in one line."),
            "text": string("Optional. One to three sentences that give the answer itself."),
            "points": {
                "type": "array",
                "description": "Optional. Key points, one short line each.",
                "items": {"type": "string"}
            }
        }),
        &["headline"],
    )
}

fn build_lead(args: &Map<String, Value>) -> Result<Value, String> {
    if args
        .get("headline")
        .and_then(Value::as_str)
        .is_none_or(|headline| headline.trim().is_empty())
    {
        return Err("`headline` is empty: give the answer in one line".into());
    }
    Ok(Value::Object(copy(args, &["headline", "text", "points"])))
}

fn steps_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "steps": list_of(
                "The steps in the order they are done.",
                serde_json::json!({
                    "label": string("What to do, in a few words."),
                    "detail": string("Optional. What helps someone do it."),
                    "time": string("Optional. When it happens, as the person gave it (Saturday, 9:00, first Sunday of the month)."),
                    "duration_minutes": integer("Optional. How many whole minutes it takes.")
                }),
                &["label"],
            ),
            "total_duration_minutes": integer("Optional. The overall time the person gave; the step durations must add up to it.")
        }),
        &["steps"],
    )
}

fn checklist_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "items": {
                "type": "array",
                "description": "Each thing to bring, buy or do, as one short line.",
                "items": {"type": "string"}
            }
        }),
        &["items"],
    )
}

fn build_checklist(args: &Map<String, Value>) -> Result<Value, String> {
    let items: Vec<Value> = records(args, "items")
        .iter()
        .filter_map(|item| match item {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Object(object) => object
                .get("label")
                .and_then(Value::as_str)
                .map(|label| label.trim().to_string()),
            _ => None,
        })
        .filter(|label| !label.is_empty())
        .map(|label| serde_json::json!({"label": label}))
        .collect();
    if items.is_empty() {
        return Err("`items` is empty: give each thing as one short line".into());
    }
    let mut payload = copy(args, &["title"]);
    payload.insert("items".into(), Value::Array(items));
    Ok(Value::Object(payload))
}

fn comparison_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "options": list_of(
                "Each option being compared, in the order you discuss them.",
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
        let name = option
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("options[{at}].name is empty"))?;
        let mut built = serde_json::json!({"name": name});
        if let Some(verdict) = option.get("verdict").and_then(Value::as_str) {
            built["verdict"] = Value::String(verdict.trim().to_string());
        }
        let strengths = points(option.get("strengths"));
        if !strengths.is_empty() {
            built["strengths"] = Value::Array(strengths);
        }
        let weaknesses = points(option.get("weaknesses"));
        if !weaknesses.is_empty() {
            built["weaknesses"] = Value::Array(weaknesses);
        }
        options.push(built);
    }
    if options.len() < 2 {
        return Err("a comparison needs at least two options; give each one's name".into());
    }
    let names: Vec<String> = options
        .iter()
        .map(|option| option["name"].as_str().unwrap_or_default().to_lowercase())
        .collect();
    let mut aspects: Vec<(String, Vec<Value>)> = Vec::new();
    for (at, cell) in records(args, "aspects").iter().enumerate() {
        let text = |key: &str| {
            cell.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
        };
        let (aspect, option) = (text("aspect"), text("option"));
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
        aspects[row].1[column] = Value::String(text("value").to_string());
    }
    let aspects: Vec<Value> = aspects
        .into_iter()
        .map(|(label, values)| serde_json::json!({"label": label, "values": values}))
        .collect();
    let mut payload = copy(args, &["title", "recommendation"]);
    payload.insert("options".into(), Value::Array(options));
    if !aspects.is_empty() {
        payload.insert("aspects".into(), Value::Array(aspects));
    }
    Ok(Value::Object(payload))
}

fn stats_fields() -> Value {
    fields(
        serde_json::json!({
            "title": string(""),
            "readings": list_of(
                "Each figure, in the order to show them.",
                serde_json::json!({
                    "label": string("What the figure is."),
                    "value": string("The figure, or a word for a descriptive reading such as light rain."),
                    "unit": string("Optional. Its unit, for a number (°C, km/h, %, $)."),
                    "change": string("Optional. How it moved, as given (+3% since last week).")
                }),
                &["label", "value"],
            )
        }),
        &["readings"],
    )
}

fn build_stats(args: &Map<String, Value>) -> Result<Value, String> {
    let mut readings = Vec::new();
    let mut labels = std::collections::BTreeSet::new();
    for (at, reading) in records(args, "readings").iter().enumerate() {
        let label = reading
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .ok_or_else(|| format!("readings[{at}].label is empty"))?;
        if !labels.insert(label.to_lowercase()) {
            return Err(format!(
                "two readings are labelled `{label}`; give each its own"
            ));
        }
        let mut built = serde_json::json!({"label": label, "value": scalar(&reading["value"])});
        if let Some(unit) = reading.get("unit").and_then(metric_unit) {
            built["unit"] = Value::String(unit.to_string());
        }
        if let Some(change) = reading
            .get("change")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|change| !change.is_empty())
        {
            built["change"] = Value::String(change.to_string());
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

fn callout_fields() -> Value {
    fields(
        serde_json::json!({
            "tone": {
                "type": "string",
                "enum": ["tip", "note", "warning", "important"],
                "description": "Optional. tip: a helpful suggestion; note: worth knowing; warning: a risk; important: must not be missed."
            },
            "title": string(""),
            "text": string("The note itself, in one to three sentences.")
        }),
        &["text"],
    )
}

fn build_callout(args: &Map<String, Value>) -> Result<Value, String> {
    if args
        .get("text")
        .and_then(Value::as_str)
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err("`text` is empty".into());
    }
    let mut payload = copy(args, &["title", "text"]);
    let tone = args
        .get("tone")
        .and_then(Value::as_str)
        .filter(|tone| ["tip", "note", "warning", "important"].contains(tone))
        .unwrap_or("note");
    payload.insert("tone".into(), Value::String(tone.to_string()));
    Ok(Value::Object(payload))
}

/// The block's `steps` are the stored card's `items`, the list every
/// timeline renderer and pack reads.
fn build_steps(args: &Map<String, Value>) -> Result<Value, String> {
    let mut args = args.clone();
    if let Some(steps) = args.remove("steps") {
        args.insert("items".into(), steps);
    }
    build_timeline(&args)
}

fn fill_nothing(_: &mut Map<String, Value>) {}

/// Strengths and weaknesses are texts this block splits at semicolons; a
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

/// The visual style written as the type (`line` beside `chart_type` line)
/// is a chart of that style.
fn fill_chart(fields: &mut Map<String, Value>) {
    if matches!(
        fields.get("semantic_type").and_then(Value::as_str),
        Some("line" | "bar" | "area")
    ) {
        fields.insert("semantic_type".into(), Value::String("chart".into()));
    }
}

/// Durations written as text are minutes: the total, and each step's own,
/// read from `duration_minutes`, one unambiguous spelling of it, or one
/// clear duration in the step's time or detail.
fn fill_steps(fields: &mut Map<String, Value>) {
    if let Some(minutes) = fields
        .get("total_duration_minutes")
        .and_then(Value::as_str)
        .and_then(duration_from_text)
    {
        fields.insert("total_duration_minutes".into(), Value::from(minutes));
    }
    let Some(steps) = fields.get_mut("steps").and_then(Value::as_array_mut) else {
        return;
    };
    for step in steps.iter_mut().filter_map(Value::as_object_mut) {
        if let Some(minutes) = step
            .get("duration_minutes")
            .and_then(Value::as_str)
            .and_then(duration_from_text)
        {
            step.insert("duration_minutes".into(), Value::from(minutes));
            continue;
        }
        if step.contains_key("duration_minutes") {
            continue;
        }
        // Spellings seen in real calls, taken only when exactly one is there.
        let aliases = ["duration", "time_minutes", "time_in_minutes"]
            .into_iter()
            .filter(|alias| step.contains_key(*alias))
            .collect::<Vec<_>>();
        if let [alias] = aliases.as_slice()
            && let Some(duration) = step.remove(*alias)
        {
            step.insert("duration_minutes".into(), duration);
            continue;
        }
        if let Some(duration) = duration_from_item_text(step) {
            step.insert("duration_minutes".into(), Value::from(duration));
        }
    }
}

/// Readings sent as one label-to-value object are the same readings as a
/// list; nested or non-scalar values are left for the schema to refuse.
fn fill_stats(fields: &mut Map<String, Value>) {
    let Some(Value::Object(readings)) = fields.get("readings") else {
        return;
    };
    let normalized = readings
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Value::String(value) => value.clone(),
                Value::Number(value) => value.to_string(),
                _ => return None,
            };
            let label = key
                .split(['_', '-'])
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let mut chars = part.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" ");
            (!label.is_empty()).then(|| serde_json::json!({"label": label, "value": value}))
        })
        .collect::<Option<Vec<_>>>();
    if let Some(normalized) = normalized {
        fields.insert("readings".into(), Value::Array(normalized));
    }
}

const SHAPES: &[CardShape] = &[
    CardShape {
        name: "emit_summary_card",
        description: "Lead the answer: a headline and one to three sentences that give the answer itself, with key points.",
        semantic_types: &["lead"],
        default_type: Some("lead"),
        fields: lead_fields,
        build: build_lead,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_steps_card",
        description: "Steps someone follows in order: a routine, a how-to, a schedule, a trip or any plan in time. Each step has a short label, and a detail, time or duration when they help.",
        semantic_types: &[
            "steps",
            "timeline",
            "plan.timeline",
            "itinerary",
            "schedule",
            "agenda",
            "milestones",
            "progress",
            "status",
            "lesson",
            "habit_plan",
            "project_plan",
            "home_project",
            "care_plan",
            "event_plan",
            "meal_plan",
            "decision",
            "decision_analysis",
            "coding.deployment",
            "coding.incident",
            "coding.release",
        ],
        default_type: Some("steps"),
        fields: steps_fields,
        build: build_steps,
        fill: fill_steps,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_checklist_card",
        description: "Things to bring, buy, read, note or do, in no particular order, each one short line.",
        semantic_types: &[
            "checklist",
            "shopping_list",
            "reading_list",
            "media_list",
            "collection",
            "notes",
            "meeting_notes",
            "follow_up",
            "reminder",
            "contact_log",
        ],
        default_type: Some("checklist"),
        fields: checklist_fields,
        build: build_checklist,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_comparison_card",
        description: "Two or more options side by side: each one's name, verdict, strengths and weaknesses, the aspects they differ on, and which you recommend.",
        semantic_types: &["comparison"],
        default_type: Some("comparison"),
        fields: comparison_fields,
        build: build_comparison,
        fill: fill_comparison,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_table_card",
        description: "Rows of data under the same columns. Rows are strings with the header first and cells separated by |.",
        semantic_types: &[
            "table",
            "data.grid",
            "dataframe",
            "comparison_table",
            "pros_cons",
            "decision_matrix",
            "criteria_matrix",
            "tradeoff_analysis",
            "scorecard",
            "inventory",
            "budget",
            "finance_summary",
            "invoice_summary",
            "travel_options",
            "coding.benchmark",
            "coding.dependencies",
        ],
        default_type: Some("table"),
        fields: table_fields,
        build: build_table,
        fill: fill_table,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_stats_card",
        description: "Key figures shown large: each a label and value, with its unit and change when known.",
        semantic_types: &["stats"],
        default_type: Some("stats"),
        fields: stats_fields,
        build: build_stats,
        fill: fill_stats,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_chart_card",
        description: "A chart of numbers over time or across categories. Give every point with the name of its series.",
        semantic_types: &[
            "chart",
            "trend",
            "timeseries",
            "bar_chart",
            "metric_chart",
            "comparison_chart",
            "telemetry.chart",
        ],
        default_type: Some("chart"),
        fields: chart_fields,
        build: build_chart,
        fill: fill_chart,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_callout_card",
        description: "A short note set apart from the rest: a tip, a note, a warning, or something important not to miss.",
        semantic_types: &["callout"],
        default_type: Some("callout"),
        fields: callout_fields,
        build: build_callout,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_details_card",
        description: "Labelled facts about one subject, or questions with their answers: a place, a person or thing, a set of readings, a board, a form or a document.",
        semantic_types: &[
            "details",
            "overview",
            "summary",
            "detail",
            "faq",
            "metric",
            "telemetry.metric",
            "map",
            "route_map",
            "calendar",
            "availability",
            "board",
            "entity",
            "evidence",
            "document",
            "graph",
            "form",
            "action",
            "transaction",
            "alert",
            "conversation",
            "progress_dashboard",
            "simulation",
            "search_results",
            "coding.search",
            "coding.architecture",
        ],
        default_type: Some("details"),
        fields: details_fields,
        build: build_details,
        fill: fill_nothing,
        eyebrow: Eyebrow::Title,
    },
    CardShape {
        name: "emit_research_card",
        description: "Findings drawn from sources you retrieved, each tied to the exact title and URL it came from.",
        semantic_types: &[
            "research.synthesis",
            "research_brief",
            "news",
            "social.reddit.research_brief",
            "social.reddit.source_review",
            "social.reddit.takeaway_board",
            "social.youtube.research_brief",
            "social.youtube.source_review",
            "social.youtube.takeaway_board",
            "social.x.research_brief",
            "social.x.source_review",
            "social.x.takeaway_board",
            "social.linkedin.research_brief",
            "social.linkedin.source_review",
            "social.linkedin.takeaway_board",
        ],
        default_type: Some("research.synthesis"),
        fields: research_fields,
        build: build_research,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_recipe_card",
        description: "A recipe: ingredients and the method to make it.",
        semantic_types: &[
            "recipe",
            "recipe.card",
            "recipe_summary",
            "lifestyle.recipe",
            "lifestyle.culinary_recipe",
        ],
        default_type: Some("recipe"),
        fields: recipe_fields,
        build: build_recipe,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_media_card",
        description: "A link preview (url and title) or an image, video or audio (url and alt).",
        semantic_types: &["link.preview", "media.image", "media.video", "media.audio"],
        default_type: None,
        fields: media_fields,
        build: build_media,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_ui_preview_card",
        description: "A preview of an HTML or UI file you wrote to the workspace.",
        semantic_types: &["ui.preview"],
        default_type: Some("ui.preview"),
        fields: ui_preview_fields,
        build: build_ui_preview,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_diff_card",
        description: "A diff of the code changes you made.",
        semantic_types: &["coding.diff"],
        default_type: Some("coding.diff"),
        fields: diff_fields,
        build: build_diff,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_test_report_card",
        description: "The results of a test run you actually executed.",
        semantic_types: &["test.report"],
        default_type: Some("test.report"),
        fields: test_report_fields,
        build: build_test_report,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
    CardShape {
        name: "emit_terminal_card",
        description: "A command you ran and its real captured output.",
        semantic_types: &["terminal.view"],
        default_type: Some("terminal.view"),
        fields: terminal_fields,
        build: build_terminal,
        fill: fill_nothing,
        eyebrow: Eyebrow::Label,
    },
];

/// Repair payload shapes that `SkillRegistry::validate` checks strictly but
/// that a shared per-*shape* schema can't fully pin down on its own (a
/// handful of the ~109 semantic_types have stricter per-field-name or
/// derived-value requirements than their sibling types in the same shape
/// category). Fixing these here — deterministically, from data the model
/// already gave us — is more reliable than asking a small local model to
/// track field-name synonyms or keep derived counts consistent by hand.
fn normalize_payload(semantic_type: &str, mut payload: Value) -> Value {
    match semantic_type {
        // `itinerary` validates each item's `title`; the shared timeline
        // schema asks the model for `label`. Same data, two field names.
        "itinerary" => {
            if let Some(items) = payload.get_mut("items").and_then(Value::as_array_mut) {
                for item in items {
                    if item.get("title").is_none()
                        && let Some(label) = item.get("label").cloned()
                    {
                        item["title"] = label;
                    }
                }
            }
        }
        // `plan.timeline` validates a top-level `title`; not required by the
        // shared schema since most sibling timeline types don't need one.
        "plan.timeline" => {
            if payload.get("title").and_then(Value::as_str).is_none() {
                payload["title"] = Value::String("Plan".into());
            }
        }
        // `test.report` validates that any `total`/`passed`/`failed`/`skipped`
        // counts the model supplies match the actual `tests` array — so
        // derive them instead of trusting the model to keep them in sync.
        "test.report" => {
            if let Some(tests) = payload.get("tests").and_then(Value::as_array).cloned() {
                let count_where = |status: &str| {
                    tests
                        .iter()
                        .filter(|t| t.get("status").and_then(Value::as_str) == Some(status))
                        .count() as u64
                };
                payload["total"] = Value::from(tests.len() as u64);
                payload["passed"] = Value::from(count_where("passed"));
                payload["failed"] = Value::from(count_where("failed"));
                payload["skipped"] = Value::from(count_where("skipped"));
            }
        }
        // A preview's isolation is the client's decision, never the card's:
        // a model-supplied `sandbox` or `connect_src` would widen the frame
        // that runs its own markup.
        "ui.preview" => {
            if let Some(map) = payload.as_object_mut() {
                map.remove("sandbox");
                map.remove("connect_src");
            }
        }
        _ => {}
    }
    payload
}

/// One representative-but-minimal set of arguments per shape, valid for
/// that shape's schema (and, where the schema alone isn't enough, the
/// stricter per-type validators in `vak_delivery::skills::validate_payload`).
/// Every `semantic_type` this shape's tool can emit is then executed with
/// the SAME arguments, to prove the shared shape (plus `normalize_payload`
/// for the couple of known type-specific exceptions) genuinely renders for
/// every type the tool claims to support — not just one hand-picked example.
#[allow(clippy::panic)]
fn fixture_for(shape_name: &str) -> Value {
    match shape_name {
        "emit_details_card" => serde_json::json!({
            "title": "T",
            "text": "S",
            "fields": [{"label": "Owner", "detail": "Ada"}]
        }),
        "emit_summary_card" => serde_json::json!({
            "headline": "Keep the shelves as they are",
            "text": "Sorting by kind is already easy to follow.",
            "points": ["Restock on Saturdays"]
        }),
        "emit_checklist_card" => serde_json::json!({"items": ["Rice", "Tins"]}),
        "emit_comparison_card" => serde_json::json!({
            "options": [
                {"name": "By shelf", "verdict": "Simple", "strengths": "Easy; familiar"},
                {"name": "By meal", "weaknesses": "Needs relabelling"}
            ],
            "aspects": [
                {"aspect": "Effort", "option": "By shelf", "value": "Low"},
                {"aspect": "Effort", "option": "By meal", "value": "High"}
            ],
            "recommendation": "Keep by shelf."
        }),
        "emit_stats_card" => serde_json::json!({
            "readings": [{"label": "Uptime", "value": "99.9", "unit": "%"}]
        }),
        "emit_callout_card" => serde_json::json!({"tone": "tip", "text": "Check dates monthly."}),
        "emit_research_card" => serde_json::json!({
            "sources": [{"title": "Src", "url": "https://example.com", "finding": "Point"}]
        }),
        "emit_diff_card" => serde_json::json!({
            "files": [{"filename": "a.rs", "hunks": "@@ -1 +1 @@", "additions": 1, "deletions": 0}]
        }),
        "emit_test_report_card" => serde_json::json!({
            "tests": [{"name": "it_works", "status": "passed"}]
        }),
        "emit_terminal_card" => serde_json::json!({"command": "ls", "output": "a.rs"}),
        "emit_table_card" => serde_json::json!({"rows": ["Name | Role", "Alice | Lead"]}),
        "emit_steps_card" => serde_json::json!({
            "title": "T",
            "steps": [{"label": "Step 1", "detail": "d"}]
        }),
        "emit_recipe_card" => serde_json::json!({
            "title": "Soup",
            "ingredients": ["1 cup water"],
            "method": ["Boil"]
        }),
        "emit_ui_preview_card" => {
            serde_json::json!({"title": "Preview", "html": "<p>Hello</p>"})
        }
        "emit_chart_card" => serde_json::json!({
            "chart_type": "line",
            "accessible_summary": "flat",
            "data": [{"series": "s1", "x": "1", "y": 2.0}]
        }),
        "emit_media_card" => serde_json::json!({"url": "https://example.com", "title": "Link"}),
        other => panic!("no fixture defined for shape {other} — add one"),
    }
}

/// `emit_media_card` carries two kinds of card: a link preview (url+title)
/// and media (url+alt). Override the shared fixture for the
/// media types.
fn fixture_override(semantic_type: &str) -> Option<Value> {
    match semantic_type {
        "media.image" => Some(serde_json::json!({"url": "https://example.com/a.png", "alt": "a"})),
        "media.video" => Some(serde_json::json!({"url": "https://example.com/a.mp4", "alt": "a"})),
        "media.audio" => Some(serde_json::json!({"url": "https://example.com/a.mp3", "alt": "a"})),
        _ => None,
    }
}

/// Every `(tool, semantic_type, arguments)` the tools claim to support, with
/// schema-valid arguments that carry the type — the single source for
/// conformance tests here and in vak-server (which checks the full call →
/// ledger → projection path).
#[doc(hidden)]
pub fn conformance_cases() -> Vec<(&'static str, &'static str, Value)> {
    let mut out = Vec::new();
    for shape in SHAPES {
        for &semantic_type in shape.semantic_types {
            let mut args =
                fixture_override(semantic_type).unwrap_or_else(|| fixture_for(shape.name));
            args["semantic_type"] = Value::String(semantic_type.to_string());
            out.push((shape.name, semantic_type, args));
        }
    }
    out
}

pub struct EmitCardTool {
    shape: &'static CardShape,
}

impl EmitCardTool {
    /// One `EmitCardTool` per registered shape category — call this once
    /// per entry in `SHAPES` when assembling the tool list for a turn.
    pub fn all() -> Vec<Self> {
        SHAPES.iter().map(|shape| EmitCardTool { shape }).collect()
    }
}

#[async_trait]
impl Tool for EmitCardTool {
    fn name(&self) -> &str {
        self.shape.name
    }

    fn description(&self) -> &str {
        self.shape.description
    }

    fn schema(&self) -> Value {
        let own = (self.shape.fields)();
        let mut properties = own["properties"].as_object().cloned().unwrap_or_default();
        if let Some(title) = properties.get_mut("title") {
            let needed = own["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|name| name == "title"));
            *title = string(if needed {
                "A short heading for the content, in your own words."
            } else {
                "Optional. A short heading for the content, in your own words."
            });
        }
        let mut required: Vec<Value> = own["required"].as_array().cloned().unwrap_or_default();
        let default = self.shape.default_type;
        let description = match default {
            Some("chart") => "The card category, not its visual style. Use `chart` for a regular chart; put line, bar or area in `chart_type`. Omit this to use chart.".to_string(),
            Some(default) => format!("The card category (use {default} for a regular card). This is not the card's visual style."),
            None => "The card category, not its visual style.".to_string(),
        };
        if self.shape.semantic_types.len() > 1 {
            properties.insert(
                "semantic_type".into(),
                serde_json::json!({
                    "type": "string",
                    "enum": self.shape.semantic_types,
                    "description": description
                }),
            );
        }
        properties.insert(
            "label".into(),
            string(
                "Optional. A few words of your own naming this part of the answer, shown above it.",
            ),
        );
        properties.insert(
            "answers".into(),
            string("The deliverable from your plan this card delivers, by the name you gave it there. A planned card counts as shown only when a card names it here; leave it out for a card your plan does not list."),
        );
        if default.is_none() {
            required.insert(0, Value::String("semantic_type".into()));
        }
        serde_json::json!({"type": "object", "properties": properties, "required": required})
    }

    fn presents_cards(&self) -> bool {
        true
    }

    fn card_kinds(&self) -> Vec<String> {
        self.shape
            .semantic_types
            .iter()
            .map(|kind| (*kind).to_string())
            .collect()
    }

    fn fill_defaults(&self, args: &mut Value) {
        if let Some(fields) = args.as_object_mut() {
            // A block with one type takes no `semantic_type`; a model that
            // sends one anyway (a topic name from habit) gets the block's own.
            if let [only] = self.shape.semantic_types {
                fields.insert("semantic_type".into(), Value::String((*only).into()));
            } else if let Some(default) = self.shape.default_type
                && !fields.contains_key("semantic_type")
            {
                fields.insert("semantic_type".into(), Value::String(default.into()));
            }
            (self.shape.fill)(fields);
        }
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        match validate_call(self.shape, args, &vak_delivery::built_in_skill_registry()) {
            Ok(output) => ToolOutput::ok(format!("Shown to the user ({}).", output.semantic_type)),
            Err(reason) => ToolOutput::error(
                Fault::Shape,
                format!("Card not displayed: {reason}. Fix these fields."),
            ),
        }
    }
}

fn validate_call(
    shape: &CardShape,
    args: &Value,
    skills: &vak_delivery::SkillRegistry,
) -> Result<vak_delivery::StructuredOutput, String> {
    let Some(args) = args.as_object() else {
        return Err("the arguments must be an object".into());
    };
    let requested_semantic_type = match args.get("semantic_type") {
        _ if shape.semantic_types.len() == 1 => shape.semantic_types[0],
        Some(Value::String(semantic_type)) => semantic_type.as_str(),
        Some(_) => return Err("`semantic_type` must be a string".into()),
        None => shape.default_type.ok_or_else(|| {
            format!(
                "missing `semantic_type`; it is one of {:?}",
                shape.semantic_types
            )
        })?,
    };
    let semantic_type = requested_semantic_type;
    if !shape.semantic_types.contains(&semantic_type) {
        return Err(format!(
            "`{semantic_type}` is not a kind of this block; its kinds are {:?}",
            shape.semantic_types
        ));
    }
    let mut own = args.clone();
    own.remove("semantic_type");
    // Every block takes these two beside its own fields: `answers` names
    // the plan deliverable it delivers (read by the loop, never stored),
    // and `label` is the model's own words for the block, its eyebrow.
    own.remove("answers");
    let label = own
        .remove("label")
        .and_then(|label| label.as_str().map(str::trim).map(str::to_string))
        .filter(|label| !label.is_empty());
    let fields = (shape.fields)();
    let known = fields["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let unknown: Vec<String> = own
        .keys()
        .filter(|key| !known.contains_key(key.as_str()))
        .map(|key| format!("`{key}`"))
        .collect();
    if !unknown.is_empty() {
        let takes: Vec<String> = known.keys().map(|key| format!("`{key}`")).collect();
        return Err(format!(
            "unexpected {}; {} takes {} beside `semantic_type`",
            unknown.join(", "),
            shape.name,
            takes.join(", ")
        ));
    }
    // Builders see the chosen type beside their own fields.
    own.insert(
        "semantic_type".into(),
        Value::String(semantic_type.to_string()),
    );
    let mut payload = (shape.build)(&own)?;
    if let (Some(label), Some(object)) = (label, payload.as_object_mut()) {
        match shape.eyebrow {
            Eyebrow::Label => {
                object.insert("label".into(), Value::String(label));
            }
            Eyebrow::Title => {
                object
                    .entry("title")
                    .or_insert_with(|| Value::String(label));
            }
        }
    }
    let payload = match semantic_type.strip_prefix("media.") {
        Some(kind) => stored_media(kind, payload)?,
        None if semantic_type == "link.preview" => stored_link(payload)?,
        None => payload,
    };
    let envelope = serde_json::json!({
        "semantic_type": semantic_type,
        "payload": normalize_payload(semantic_type, payload),
    });
    vak_delivery::parse_fragment_with(&envelope.to_string(), skills).map_err(|e| e.to_string())
}

/// The stored media card from the tool's arguments. The tool asks for the
/// kind once, as the `semantic_type`, and for one address, `url`, the field
/// a link preview uses too. Measured on gemma4, 12 runs a variant
/// (docs/design/30-render-architecture.md §30.1): with `media_type` beside
/// `semantic_type` the model left `semantic_type` out, with `source` beside
/// `url` it wrote the address in `url`, and with a link preview's
/// `image_url` in the schema an image's address went there. What it shows
/// is its `alt`, else its title or description, else the file's own name.
fn stored_media(kind: &str, payload: Value) -> Result<Value, String> {
    let Value::Object(mut card) = payload else {
        return Ok(payload);
    };
    let source = card_text(&card, "url")
        .ok_or_else(|| format!("give `url`: the {kind} file's URL or workspace path"))?;
    let alt = ["alt", "title", "description"]
        .iter()
        .find_map(|key| card_text(&card, key))
        .or_else(|| address_name(&source))
        .ok_or_else(|| format!("give `alt`: what the {kind} shows, in words"))?;
    card.remove("url");
    card.remove("thumbnail_url");
    card.insert("source".into(), Value::String(source));
    card.insert("alt".into(), Value::String(alt));
    card.insert("media_type".into(), Value::String(kind.into()));
    Ok(Value::Object(card))
}

/// The stored link preview: its thumbnail is `image_url`, and a preview
/// given no title is titled by the site it points at.
fn stored_link(payload: Value) -> Result<Value, String> {
    let Value::Object(mut card) = payload else {
        return Ok(payload);
    };
    let url = card_text(&card, "url").ok_or("give `url`: the page's URL")?;
    if let Some(thumbnail) = card.remove("thumbnail_url") {
        card.insert("image_url".into(), thumbnail);
    }
    if card_text(&card, "title").is_none() {
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

fn card_text(card: &Map<String, Value>, key: &str) -> Option<String> {
    card.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
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

/// The `emit_*_card` tool that carries `semantic_type`, if any.
pub fn emit_tool_for(semantic_type: &str) -> Option<&'static str> {
    SHAPES
        .iter()
        .find(|shape| shape.semantic_types.contains(&semantic_type))
        .map(|shape| shape.name)
}

/// Names of every tool that declares `presents_cards()`, for the permission
/// engine: a card is Vak's own display channel and needs no approval.
pub fn presenting_tool_names() -> Vec<String> {
    EmitCardTool::all()
        .iter()
        .filter(|tool| tool.presents_cards())
        .map(|tool| tool.name().to_string())
        .collect()
}

/// Rebuilds an `emit_*_card` call's validated output from the call's own
/// arguments — the ledger records these untruncated, so nothing depends on
/// the tool result text (which the tool framework line-truncates at ~2000
/// characters, silently destroying any larger card's JSON). Private: the
/// only consumers are this module's own conformance tests and
/// `presentation_info`, which turns this into a `Presentation` ledger entry
/// at the moment a card validates (docs/design/68-context-engine.md §10).
/// `vak-server`'s projection used to call a public version of this
/// (`card_output_from_call`) to rebuild the card for display on every
/// snapshot; it now reads the written `Presentation` entry instead, so
/// nothing outside this crate needs to re-validate a call's arguments.
fn rebuild_call(
    name: &str,
    input: &Value,
    skills: &vak_delivery::SkillRegistry,
) -> Option<vak_delivery::StructuredOutput> {
    let shape = SHAPES.iter().find(|shape| shape.name == name)?;
    validate_call(shape, input, skills).ok()
}

/// Everything needed to write a `PresentationRecord` for a call that just
/// validated: the canonical payload, the schema-driven title and identity
/// digest, and which skill/version/schema owns the type. `vak-agent`'s
/// tool-execution path has no session-log access (AGENTS.md invariant 14:
/// tools cross a broker boundary), so this is exposed through
/// `AgentConfig::presentation_rebuild`, a closure `Core` installs — the
/// agent loop stays free of card-shape knowledge and calls this indirectly.
pub struct PresentationInfo {
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub schema_version: u32,
    /// Canonical (key-sorted) form — see `vak_session::types::canonicalize_json`.
    pub payload: Value,
    pub title: String,
    pub identity_digest: String,
}

/// Validates an `emit_*_card` call and returns everything needed to write
/// its `Presentation` ledger entry. `None` when the call does not validate
/// (the tool's own `execute()` already rejected it in that case, so this is
/// only ever called for a call that already succeeded — see
/// `AgentConfig::presentation_rebuild`'s call site in `Core`).
pub fn presentation_info(
    name: &str,
    input: &Value,
    skills: &vak_delivery::SkillRegistry,
) -> Option<PresentationInfo> {
    let output = rebuild_call(name, input, skills)?;
    let payload = vak_session::types::canonicalize_json(&output.payload);
    let title = title_for(&output.semantic_type, &payload);
    let identity_digest = identity_digest(&output.semantic_type, &payload);
    Some(PresentationInfo {
        semantic_type: output.semantic_type,
        skill_id: output.skill_id,
        skill_version: output.skill_version,
        schema_version: u32::from(output.schema_version),
        payload,
        title,
        identity_digest,
    })
}

/// Title fallback for a card whose payload has no `title` field (only the
/// metric shape lacks one; every other shape's schema asks the model for
/// one).
fn title_for(semantic_type: &str, payload: &Value) -> String {
    if let Some(title) = payload.get("title").and_then(Value::as_str)
        && !title.trim().is_empty()
    {
        return title.to_string();
    }
    payload
        .get("label")
        .and_then(Value::as_str)
        .or_else(|| payload.get("location").and_then(Value::as_str))
        .unwrap_or(semantic_type)
        .to_string()
}

fn compact_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn canonical_compact(payload: &Value) -> String {
    let canonical = vak_session::types::canonicalize_json(payload);
    serde_json::to_string(&canonical).unwrap_or_default()
}

fn research_digest(payload: &Value) -> String {
    let takeaways: Vec<String> = payload
        .get("takeaways")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|t| t.get("text").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let sources: Vec<String> = payload
        .get("sources")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|s| {
                    let title = s.get("title").and_then(Value::as_str).unwrap_or("");
                    let url = s.get("url").and_then(Value::as_str).unwrap_or("");
                    format!("{title} ({url})")
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "takeaways: {} | sources: {}",
        takeaways.join(" ~ "),
        sources.join(", ")
    )
}

fn table_digest(payload: &Value) -> String {
    let title = payload.get("title").and_then(Value::as_str).unwrap_or("");
    let columns: Vec<String> = payload
        .get("columns")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|c| c.get("key").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let rows = payload.get("rows").and_then(Value::as_array);
    let row_count = rows.map(Vec::len).unwrap_or(0);
    let first_row = rows.and_then(|r| r.first()).cloned().unwrap_or(Value::Null);
    format!(
        "title: {title} | columns: {} | rows: {row_count} | first: {}",
        columns.join(","),
        canonical_compact(&first_row)
    )
}

fn chart_digest(payload: &Value) -> String {
    let title = payload.get("title").and_then(Value::as_str).unwrap_or("");
    let summary = payload
        .get("accessible_summary")
        .and_then(Value::as_str)
        .unwrap_or("");
    let series: Vec<String> = payload
        .get("series")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|s| {
                    let name = s.get("name").and_then(Value::as_str).unwrap_or("");
                    let points = s
                        .get("points")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0);
                    format!("{name}({points})")
                })
                .collect()
        })
        .unwrap_or_default();
    format!(
        "title: {title} | series: {} | summary: {summary}",
        series.join(",")
    )
}

fn entity_digest(payload: &Value) -> String {
    let title = payload.get("title").and_then(Value::as_str).unwrap_or("");
    let entity_type = payload.get("type").and_then(Value::as_str).unwrap_or("");
    let mut fields: Vec<String> = payload
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(key, _)| key.as_str() != "title" && key.as_str() != "type")
                .map(|(key, value)| format!("{key}={}", compact_scalar(value)))
                .collect()
        })
        .unwrap_or_default();
    fields.sort();
    format!(
        "title: {title} | type: {entity_type} | fields: {}",
        fields.join(",")
    )
}

fn items_digest(payload: &Value) -> String {
    let title = payload.get("title").and_then(Value::as_str).unwrap_or("");
    let labels: Vec<String> = payload
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("label")
                        .or_else(|| item.get("title"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    format!("title: {title} | items: {}", labels.join(","))
}

fn default_digest(semantic_type: &str, payload: &Value) -> String {
    let title = payload
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(semantic_type);
    let mut keys: Vec<&str> = payload
        .as_object()
        .map(|obj| obj.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort();
    format!("title: {title} | keys: {}", keys.join(","))
}

/// Schema-driven identity digest (docs/design/68-context-engine.md §10):
/// which fields make a presentation distinguishable from another of the
/// same `semantic_type`. Dispatches by shape (the same grouping `SHAPES`
/// already uses — a `table`-shaped type and a `chart`-shaped type get
/// digested the same way as their siblings), with `entity` and the
/// checklist/timeline/itinerary family called out explicitly per the
/// design. Never a character-count truncation: each branch names the
/// fields that carry identity for its shape instead of cutting the payload
/// short.
pub fn identity_digest(semantic_type: &str, payload: &Value) -> String {
    // By the stored payload's shape, so a card recorded under a type no
    // block emits any more is still told apart from its neighbours.
    let has = |key: &str| payload.get(key).is_some();
    match semantic_type {
        "stats" | "metric" => canonical_compact(payload),
        "entity" => entity_digest(payload),
        _ if has("columns") && has("rows") => table_digest(payload),
        _ if has("series") || has("accessible_summary") => chart_digest(payload),
        _ if has("takeaways") || has("sources") => research_digest(payload),
        _ if payload.get("items").is_some_and(Value::is_array) => items_digest(payload),
        _ => default_digest(semantic_type, payload),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A call's arguments as they reach the builder: read by the one
    /// argument reader, then given the tool's own defaults.
    fn sent(tool: &EmitCardTool, args: &mut Value) {
        vak_tools::read_arguments(&tool.schema(), args);
        tool.fill_defaults(args);
    }

    fn shape(name: &str) -> &'static CardShape {
        SHAPES.iter().find(|shape| shape.name == name).unwrap()
    }

    fn built(name: &str, args: serde_json::Value) -> Result<Value, String> {
        validate_call(shape(name), &args, &vak_delivery::built_in_skill_registry())
            .map(|output| output.payload)
    }

    #[test]
    fn comparison_aspects_are_one_typed_cell_for_each_option() {
        let payload = built(
            "emit_comparison_card",
            serde_json::json!({
                "options": [{"name": "Basil"}, {"name": "Mint"}],
                "aspects": [
                    {"aspect": "Sun", "option": "Mint", "value": "Part shade"},
                    {"aspect": "Sun", "option": "basil", "value": "Morning sun"},
                    {"aspect": "Water", "option": "Mint", "value": "Often"}
                ]
            }),
        )
        .unwrap();
        assert_eq!(
            payload["aspects"],
            serde_json::json!([
                {"label": "Sun", "values": ["Morning sun", "Part shade"]},
                {"label": "Water", "values": ["", "Often"]}
            ])
        );
        let stray = built(
            "emit_comparison_card",
            serde_json::json!({
                "options": [{"name": "Basil"}, {"name": "Mint"}],
                "aspects": [{"aspect": "Sun", "option": "Thyme", "value": "Full"}]
            }),
        );
        assert!(stray.unwrap_err().contains("not the name of an option"));
    }

    #[test]
    fn a_record_text_sent_as_a_list_is_read_as_its_items() {
        let tool = EmitCardTool {
            shape: shape("emit_comparison_card"),
        };
        let mut args = serde_json::json!({"options": [
            {"name": "Train", "strengths": ["No parking", "Can read"], "weaknesses": []},
            {"name": "Drive", "strengths": "Faster"}
        ]});
        sent(&tool, &mut args);
        vak_tools::validate_input(&tool.schema(), &args).unwrap();
        let payload = built("emit_comparison_card", args).unwrap();
        assert_eq!(
            payload["options"][0]["strengths"],
            serde_json::json!(["No parking", "Can read"])
        );
        assert!(payload["options"][0].get("weaknesses").is_none());
    }

    /// No argument shape crashes a card tool. Each tool's own fixture is
    /// bent one field at a time (removed, emptied, retyped) and sent through
    /// the path a model's call takes; every answer is a card or an error
    /// naming what to fix (invariant 3). Found live: `emit_details_card`
    /// indexed a `fields` the call had not sent.
    #[test]
    fn no_argument_shape_crashes_a_card_tool() {
        let bends = [
            serde_json::json!(null),
            serde_json::json!(""),
            serde_json::json!("one line"),
            serde_json::json!("a: b\nc"),
            serde_json::json!(7),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!(["x"]),
            serde_json::json!([{}]),
            serde_json::json!([{"label": ""}]),
            serde_json::json!([["a"], []]),
            serde_json::json!(["a | b", "1"]),
        ];
        let registry = vak_delivery::built_in_skill_registry();
        for shape in SHAPES {
            let tool = EmitCardTool { shape };
            let fixture = fixture_for(shape.name);
            let keys: Vec<String> = fixture
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .chain(["semantic_type".to_string(), "answers".to_string()])
                .collect();
            let mut calls = vec![serde_json::json!({}), fixture.clone()];
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
                        let _ = validate_call(shape, &call, &registry);
                    }
                }));
                assert!(outcome.is_ok(), "{} crashed on {given}", shape.name);
            }
        }
    }

    /// The contract's rules (docs/design/30-render-architecture.md §30.1),
    /// checked on every tool's advertised schema: only the keywords every
    /// provider carries to a model, no list of records inside a record that
    /// is itself in a list, `semantic_type` required first, and no value
    /// asked to start with `|`.
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
            assert!(
                schema["properties"].get("payload").is_none(),
                "{}",
                tool.name()
            );
            walk(&schema, tool.name(), 0, &mut problems);
            if tool.shape.semantic_types.len() == 1 {
                assert!(
                    schema["properties"].get("semantic_type").is_none(),
                    "{} has one type and asks for none",
                    tool.name()
                );
            }
            if tool.name() == "emit_steps_card" {
                let item_schema = &schema["properties"]["steps"]["items"];
                assert_eq!(item_schema["required"], serde_json::json!(["label"]));
            }
            let default = tool.shape.default_type;
            if default.is_none() && schema["required"][0] != "semantic_type" {
                problems.push(format!(
                    "{}: semantic_type is not required first",
                    tool.name()
                ));
            }
            if default.is_some()
                && schema["required"].as_array().is_some_and(|required| {
                    required.contains(&Value::String("semantic_type".into()))
                })
            {
                problems.push(format!(
                    "{}: defaulted semantic_type is still required",
                    tool.name()
                ));
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

    /// A table is its rows as `A | B` strings, header first (20 of 20 live
    /// against 4 of 20 for one Markdown string, whose leading `|` the model
    /// fused into `table="|`), and becomes the stored grid: keyed columns,
    /// rows by label, numeric columns as numbers.
    /// A block has its own type: a missing one is filled, and a topic name
    /// sent from habit is replaced by the block's own.
    #[test]
    fn a_block_takes_its_own_type() {
        let tool = |name: &str| {
            EmitCardTool::all()
                .into_iter()
                .find(|tool| tool.name() == name)
                .unwrap()
        };
        let mut table = serde_json::json!({"rows": ["A", "1"]});
        sent(&tool("emit_table_card"), &mut table);
        assert_eq!(table["semantic_type"], "table");
        let mut chosen = serde_json::json!({"semantic_type": "budget", "rows": ["A", "1"]});
        sent(&tool("emit_table_card"), &mut chosen);
        assert_eq!(
            chosen["semantic_type"], "budget",
            "a kind the block draws is kept"
        );
        let mut details = serde_json::json!({"title": "T"});
        sent(&tool("emit_details_card"), &mut details);
        assert_eq!(details["semantic_type"], "details");
    }

    #[test]
    fn matrix_rows_that_look_like_markdown_separators_remain_data() {
        let payload = built(
            "emit_table_card",
            serde_json::json!({"rows":[["Name","Value"],["---","---"],["A","B"]]}),
        )
        .unwrap();
        assert_eq!(payload["rows"].as_array().unwrap().len(), 2);
        assert_eq!(payload["rows"][0]["Name"], "---");
        assert_eq!(payload["rows"][0]["Value"], "---");
    }

    #[test]
    fn a_table_of_one_cell_rows_is_refused_with_how_to_write_it() {
        let refused = built(
            "emit_table_card",
            serde_json::json!({"rows": ["Category", "Subcategory", "Day 1", "Total"]}),
        )
        .unwrap_err();
        assert!(refused.contains("separated by |"), "{refused}");
    }

    #[test]
    fn table_rows_become_the_stored_grid() {
        let payload = built(
            "emit_table_card",
            serde_json::json!({
                "title": "Lighthouses",
                "rows": [
                    "| **Name** | Country | Height (m) |",
                    "| :--- | --- | ---: |",
                    "Tower of Hercules | Spain | 55",
                    "Bell Rock | Scotland | 35.3"
                ]
            }),
        )
        .unwrap();
        assert_eq!(payload["columns"][0]["label"], "Name");
        assert_eq!(payload["columns"][2]["isNumeric"], true);
        assert!(payload["columns"][1].get("isNumeric").is_none());
        assert_eq!(payload["rows"][1]["Country"], "Scotland");
        assert_eq!(payload["rows"][0]["Height (m)"], 55);
        assert_eq!(payload["rows"][1]["Height (m)"], 35.3);
        let refused = built(
            "emit_table_card",
            serde_json::json!({"rows": ["Name | Country"]}),
        );
        assert!(refused.unwrap_err().contains("at least one row"));
        let refused = built(
            "emit_table_card",
            serde_json::json!({"rows": ["Name", "Bob | Smith"]}),
        );
        assert!(
            refused
                .unwrap_err()
                .contains("2 cells but the header has 1")
        );
    }

    /// Nothing about a card tool's arguments is left to guess: every
    /// argument, and every field of a record argument, says what it is, and
    /// one the call must send is never described as optional. Checked over
    /// whatever tools exist, since tools are added and removed at runtime.
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
                let items = &property["items"];
                if items["type"] == "object" {
                    check(
                        &format!("{tool}.{name}[]"),
                        &items["properties"],
                        &items["required"],
                        problems,
                    );
                }
            }
        }
        let mut problems = Vec::new();
        for shape in SHAPES {
            let schema = EmitCardTool { shape }.schema();
            assert!(!shape.description.trim().is_empty());
            check(
                shape.name,
                &schema["properties"],
                &schema["required"],
                &mut problems,
            );
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// Found live (2026-10-11): the first, complete answer to a two-part
    /// request was a Markdown table sent as one string, and schema
    /// admission refused it before the builder, which already reads one
    /// row per line. A JSON array sent as text was refused the same way.
    #[test]
    fn an_array_field_sent_as_one_string_is_read_as_its_items() {
        let tool = EmitCardTool {
            shape: shape("emit_table_card"),
        };
        let mut markdown = serde_json::json!({
            "semantic_type": "comparison_table",
            "rows": "| Method | Upper shelf | Lower shelf |\n|---|---|---|\n| By kind | Rice | Snacks |\n| By meal | Breakfast | Dinner |"
        });
        sent(&tool, &mut markdown);
        vak_tools::validate_input(&tool.schema(), &markdown).unwrap();
        let payload = built("emit_table_card", markdown).unwrap();
        assert_eq!(payload["columns"].as_array().unwrap().len(), 3);
        assert_eq!(payload["rows"][1]["Upper shelf"], "Breakfast");

        let mut json_text = serde_json::json!({
            "semantic_type": "table",
            "rows": "[\"Step | When\", \"Restock | Saturday\"]"
        });
        sent(&tool, &mut json_text);
        vak_tools::validate_input(&tool.schema(), &json_text).unwrap();
        let payload = built("emit_table_card", json_text).unwrap();
        assert_eq!(payload["rows"][0]["When"], "Saturday");

        let checklist = EmitCardTool {
            shape: shape("emit_details_card"),
        };
        let mut listed = serde_json::json!({
            "semantic_type": "faq",
            "title": "T",
            "summary": "S",
            "fields": "- Owner: Ada\n- Due: Friday"
        });
        sent(&checklist, &mut listed);
        assert_eq!(
            listed["fields"], "- Owner: Ada\n- Due: Friday",
            "text is not read as a list of records"
        );
    }

    /// Steps sent as plain strings become labelled steps before schema
    /// admission (moved from the agent loop, which knew one card's shape).
    #[test]
    fn a_step_sent_as_text_is_its_label() {
        let tool = EmitCardTool {
            shape: shape("emit_steps_card"),
        };
        let mut args = serde_json::json!({"steps": [
            "Day 1 (Sunday): Check soil; water only if dry",
            "Day 2 — Look for yellow leaves",
            {"label": "Day 3", "detail": "Check light"}
        ]});
        sent(&tool, &mut args);
        vak_tools::validate_input(&tool.schema(), &args).unwrap();
        assert_eq!(
            args["steps"],
            serde_json::json!([
                {"label": "Day 1 (Sunday): Check soil; water only if dry"},
                {"label": "Day 2 — Look for yellow leaves"},
                {"label": "Day 3", "detail": "Check light"}
            ]),
            "a step sent as text is its label, whole"
        );
        assert!(built("emit_steps_card", args).is_ok());
    }

    #[test]
    fn nested_table_rows_become_the_same_stored_grid() {
        let tool = EmitCardTool {
            shape: shape("emit_table_card"),
        };
        let mut arguments = serde_json::json!({
            "semantic_type": "comparison_table",
            "title": "Internal Issue Tracker",
            "rows": [
                ["Option", "Time", "Cost", "Risk"],
                ["Hosted tool", "2 days", "$500/month", "Low"],
                ["Extend existing app", "1 week", "$4k", "Medium"]
            ]
        });
        sent(&tool, &mut arguments);
        vak_tools::validate_input(&tool.schema(), &arguments).unwrap();
        let payload = built("emit_table_card", arguments).unwrap();
        assert_eq!(payload["columns"][0]["label"], "Option");
        assert_eq!(payload["rows"][0]["Option"], "Hosted tool");
        assert_eq!(payload["rows"][1]["Cost"], "$4k");

        let escaped = built(
            "emit_table_card",
            serde_json::json!({"rows": [["Item", "Note"], ["A", "keeps | literal"]]}),
        )
        .unwrap();
        assert_eq!(escaped["rows"][0]["Note"], "keeps | literal");
        let trailing_pipe = built(
            "emit_table_card",
            serde_json::json!({"rows": [["Name", "Note"], ["Pipe", "literal ends|"]]}),
        )
        .unwrap();
        assert_eq!(trailing_pipe["rows"][0]["Note"], "literal ends|");
        assert!(table_lines(&[serde_json::json!(["Cell\nsecond line"])]).is_err());

        let mut numeric = serde_json::json!({
            "semantic_type": "table",
            "rows": [["Team", "Open"], ["Team A", 14], ["Team B", 10]]
        });
        sent(&tool, &mut numeric);
        vak_tools::validate_input(&tool.schema(), &numeric).unwrap();
        let numeric = built("emit_table_card", numeric).unwrap();
        assert_eq!(numeric["rows"][0]["Open"], 14);
        assert_eq!(numeric["rows"][1]["Open"], 10);

        let malformed = serde_json::json!({"rows": [["Header"], "mixed row shape"]});
        assert!(built("emit_table_card", malformed).is_err());
    }

    #[test]
    fn timeline_checks_requested_total_duration_and_projects_each_duration() {
        let invalid = built(
            "emit_steps_card",
            serde_json::json!({
                "semantic_type": "plan.timeline",
                "total_duration_minutes": 45,
                "steps": [
                    {"label": "Welcome", "duration_minutes": 5},
                    {"label": "Activity", "duration_minutes": 20},
                    {"label": "Close", "duration_minutes": 10}
                ]
            }),
        )
        .unwrap_err();
        assert!(invalid.contains("add up to 35 minutes"), "{invalid}");

        let valid = built(
            "emit_steps_card",
            serde_json::json!({
                "semantic_type": "plan.timeline",
                "total_duration_minutes": 45,
                "steps": [
                    {"label": "Welcome", "duration_minutes": 10},
                    {"label": "Activity", "duration_minutes": 25},
                    {"label": "Close", "duration_minutes": 10}
                ]
            }),
        )
        .unwrap();
        assert_eq!(valid["items"][0]["time"], "10 min");
        assert!(valid["items"][0].get("duration_minutes").is_none());
    }

    #[test]
    fn timeline_infers_clear_durations_from_local_model_detail_before_total_check() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|tool| tool.name() == "emit_steps_card")
            .unwrap();
        let mut args = serde_json::json!({
            "semantic_type": "timeline",
            "title": "60-Minute Board Game Night Schedule",
            "total_duration_minutes": 60,
            "steps": [
                {"label": "Setup", "detail": "5 min (Get games ready)"},
                {"label": "Game 1", "detail": "20 min (Play first game)"},
                {"label": "Break", "detail": "5 min (Snack time)"},
                {"label": "Game 2", "detail": "20 min (Play second game)"},
                {"label": "Wrap-up", "detail": "10 min (Clean up/Debrief)"}
            ]
        });
        sent(&tool, &mut args);
        let payload = built("emit_steps_card", args).unwrap();
        assert_eq!(payload["items"][0]["time"], "5 min");
        assert_eq!(payload["items"][0]["detail"], "Get games ready");
        assert_eq!(payload["items"][4]["time"], "10 min");
    }

    #[test]
    fn timeline_normalizes_duration_field_spellings_seen_from_local_models() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|tool| tool.name() == "emit_steps_card")
            .unwrap();
        for alias in ["duration", "time_minutes", "time_in_minutes"] {
            let mut args = serde_json::json!({
                "total_duration_minutes": 60,
                "steps": [
                    {"label": "Prepare"},
                    {"label": "Sow"},
                    {"label": "Clean up"}
                ]
            });
            args["steps"][0][alias] = serde_json::json!(20);
            args["steps"][1][alias] = serde_json::json!(30);
            args["steps"][2][alias] = serde_json::json!(10);
            sent(&tool, &mut args);
            assert!(args["steps"][0].get(alias).is_none());
            assert_eq!(args["steps"][0]["duration_minutes"], 20);
            let payload = built("emit_steps_card", args).unwrap();
            assert_eq!(payload["items"][2]["time"], "10 min");
        }
    }

    #[test]
    fn timeline_converts_duration_strings_before_schema_validation() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|tool| tool.name() == "emit_steps_card")
            .unwrap();
        let mut args = serde_json::json!({
            "semantic_type": "timeline",
            "total_duration_minutes": "90 min",
            "steps": [
                {"label":"Welcome", "duration_minutes":"10 min"},
                {"label":"Swap", "duration_minutes":"60 minutes"},
                {"label":"Wrap up", "duration_minutes":"20 mins"}
            ]
        });
        sent(&tool, &mut args);
        assert_eq!(args["total_duration_minutes"], 90);
        assert_eq!(args["steps"][0]["duration_minutes"], 10);
        let payload = built("emit_steps_card", args).unwrap();
        assert_eq!(payload["items"][0]["time"], "10 min");
        assert_eq!(payload["items"][2]["time"], "20 min");
    }

    #[test]
    fn chart_style_alias_is_normalized_before_admission_schema_validation() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|tool| tool.name() == "emit_chart_card")
            .unwrap();
        let mut call = serde_json::json!({"semantic_type":"line","chart_type":"line","data":[{"series":"Requests","x":"Mon","y":12}]});
        sent(&tool, &mut call);
        assert_eq!(call["semantic_type"], "chart");
        vak_tools::validate_input(&tool.schema(), &call)
            .expect("normalized call passes pre-execution schema");
        assert!(
            presentation_info(
                "emit_chart_card",
                &call,
                &vak_delivery::built_in_skill_registry()
            )
            .is_some()
        );
    }

    #[test]
    fn chart_points_group_into_series_and_research_sources_are_checked() {
        let payload = built(
            "emit_chart_card",
            serde_json::json!({
                "chart_type": "bar",
                "accessible_summary": "rainfall",
                "data": [
                    {"series": "2025", "x": "Jan", "y": 50},
                    {"series": "2026", "x": "Jan", "y": 40},
                    {"series": "2025", "x": "Feb", "y": 45}
                ]
            }),
        )
        .unwrap();
        assert_eq!(payload["series"][0]["name"], "2025");
        assert_eq!(payload["series"][0]["points"].as_array().unwrap().len(), 2);
        assert_eq!(payload["series"][0]["points"][0]["x"], "Jan");
        // A summary is no stand-in for data, a y given as text is read as a
        // number, and one that is not a number is refused by name.
        assert!(
            built(
                "emit_chart_card",
                serde_json::json!({"chart_type": "line", "accessible_summary": "rose", "data": []}),
            )
            .unwrap_err()
            .contains("`data`")
        );
        let texty = built(
            "emit_chart_card",
            serde_json::json!({"chart_type": "line", "data": [
                {"series": "NIFTY 50", "x": "Mon", "y": "24,850.5"}
            ]}),
        )
        .unwrap();
        assert_eq!(texty["series"][0]["points"][0]["y"], 24850.5);
        assert!(
            built(
                "emit_chart_card",
                serde_json::json!({"chart_type": "line", "data": [
                    {"series": "NIFTY 50", "x": "Mon", "y": "up"}
                ]}),
            )
            .unwrap_err()
            .contains("`y` must be a number")
        );
        let research = built(
            "emit_research_card",
            serde_json::json!({
                "sources": [
                    {"title": "A", "url": "https://a.example", "finding": "First."},
                    {"title": "B", "url": "https://b.example", "finding": "Second."}
                ]
            }),
        )
        .unwrap();
        assert_eq!(research["takeaways"][1]["text"], "Second.");
        assert_eq!(research["takeaways"][1]["citation_indices"][0], 2);
        assert!(research["sources"][0].get("finding").is_none());
    }

    #[test]
    fn a_chart_without_a_summary_gets_one_from_its_points() {
        let payload = built(
            "emit_chart_card",
            serde_json::json!({
                "chart_type": "line",
                "title": "API latency",
                "data": [
                    {"series": "p99", "x": "Hour 1", "y": 150},
                    {"series": "p99", "x": "Hour 2", "y": 180}
                ]
            }),
        )
        .unwrap();
        assert_eq!(
            payload["accessible_summary"],
            "API latency. p99: 150 at Hour 1 to 180 at Hour 2, 2 points."
        );
        let own = built(
            "emit_chart_card",
            serde_json::json!({
                "chart_type": "bar",
                "accessible_summary": "Rising",
                "data": [{"series": "a", "x": "1", "y": 1}]
            }),
        )
        .unwrap();
        assert_eq!(own["accessible_summary"], "Rising");
    }

    /// Found live: with `media_type` in the schema beside `semantic_type`,
    /// the model filled `media_type` and left `semantic_type` out, and every
    /// image, video and audio card was refused. The stored card still names
    /// its kind; the tool derives it.
    #[test]
    fn a_media_card_takes_its_kind_once_as_the_semantic_type() {
        let schema = (shape("emit_media_card").fields)();
        assert!(schema["properties"].get("media_type").is_none());
        assert!(schema["properties"].get("source").is_none());
        for kind in ["image", "video", "audio"] {
            let payload = built(
                "emit_media_card",
                serde_json::json!({
                    "semantic_type": format!("media.{kind}"),
                    "url": "https://example.com/a",
                    "title": "A title"
                }),
            )
            .unwrap();
            assert_eq!(payload["media_type"], kind);
            assert_eq!(payload["source"], "https://example.com/a");
            assert_eq!(payload["alt"], "A title");
            assert!(payload.get("url").is_none());
        }
        let refused = built(
            "emit_media_card",
            serde_json::json!({"semantic_type": "media.audio", "title": "Episode one"}),
        );
        assert!(refused.unwrap_err().contains("give `url`"));
        let named = built(
            "emit_media_card",
            serde_json::json!({"semantic_type": "media.image", "url": "https://example.com/p/tower.jpg?w=2"}),
        )
        .unwrap();
        assert_eq!(named["alt"], "tower.jpg");
        let link = built(
            "emit_media_card",
            serde_json::json!({
                "semantic_type": "link.preview",
                "url": "https://www.rust-lang.org/learn",
                "thumbnail_url": "https://www.rust-lang.org/logo.png"
            }),
        )
        .unwrap();
        assert_eq!(link["title"], "www.rust-lang.org");
        assert_eq!(link["image_url"], "https://www.rust-lang.org/logo.png");
        assert!(link.get("thumbnail_url").is_none());
        assert!(schema["properties"].get("image_url").is_none());
        let refused = built(
            "emit_media_card",
            serde_json::json!({"semantic_type": "media.image", "url": "s", "alt": "a", "media_type": "image"}),
        );
        assert!(refused.unwrap_err().contains("unexpected `media_type`"));
    }

    #[test]
    fn an_unknown_field_is_named_with_the_fields_the_tool_takes() {
        let refused = built(
            "emit_stats_card",
            serde_json::json!({"metric_data": {"height": "330 m"}, "name": "Eiffel Tower"}),
        )
        .unwrap_err();
        assert!(
            refused.contains("unexpected `metric_data`, `name`"),
            "{refused}"
        );
        assert!(refused.contains("`readings`"), "{refused}");
    }

    #[test]
    fn flat_metric_reading_maps_are_normalized_before_schema_validation() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|tool| tool.name() == "emit_stats_card")
            .unwrap();
        let mut args = serde_json::json!({
            "semantic_type": "metric",
            "readings": {"start_time": "6 pm", "total_budget": "$60"}
        });

        sent(&tool, &mut args);

        assert_eq!(
            args["readings"],
            serde_json::json!([
                {"label": "Start Time", "value": "6 pm"},
                {"label": "Total Budget", "value": "$60"}
            ])
        );
        vak_tools::validate_input(&tool.schema(), &args).unwrap();
        let payload = built("emit_stats_card", args).unwrap();
        assert_eq!(payload["readings"][0]["label"], "Start Time");
        assert_eq!(payload["readings"][1]["value"], "$60");

        let mut nested = serde_json::json!({
            "semantic_type": "metric",
            "readings": {"nested": {"value": "not an ordinary reading"}}
        });
        sent(&tool, &mut nested);
        assert!(nested["readings"].is_object());
        assert!(vak_tools::validate_input(&tool.schema(), &nested).is_err());
    }

    #[test]
    fn labelled_details_and_readings_never_travel_as_keys() {
        let tool = EmitCardTool {
            shape: shape("emit_details_card"),
        };
        let mut args = serde_json::json!({
            "title": "Eiffel Tower",
            "fields": [
                {"label": "Year built", "detail": "1889"},
                {"label": "Location", "detail": "Paris: Champ de Mars"},
                {"label": "Is it open daily?", "detail": "Yes."}
            ]
        });
        sent(&tool, &mut args);
        vak_tools::validate_input(&tool.schema(), &args).unwrap();
        let payload = built("emit_details_card", args).unwrap();
        assert_eq!(payload["Year built"], "1889");
        assert_eq!(payload["Location"], "Paris: Champ de Mars");
        assert_eq!(payload["Is it open daily?"], "Yes.");
        for refused in [
            serde_json::json!({"title": "T"}),
            serde_json::json!({"fields": []}),
            serde_json::json!({"fields": [{"label": " ", "detail": "x"}]}),
            serde_json::json!({"fields": [
                {"label": "A", "detail": "1"},
                {"label": "A", "detail": "2"}
            ]}),
        ] {
            assert!(built("emit_details_card", refused).is_err());
        }
        let stats = built(
            "emit_stats_card",
            serde_json::json!({"readings": [{"label": "CPU usage", "value": "42", "unit": "%"}]}),
        )
        .unwrap();
        assert_eq!(
            stats["readings"],
            serde_json::json!([{"label": "CPU usage", "value": 42, "unit": "%"}]),
            "a reading keeps its label as a value, never as a key"
        );
    }

    /// Real trigger: "Vak wants to use emit_stats_card — this needs your
    /// approval" under a card that had already rendered. Cards are Vak's own
    /// display channel, so every shape is open in every mode, through the
    /// engine `Core` builds — the one the capability preflight reads too.
    #[test]
    fn every_card_tool_is_allowed_in_every_mode_without_a_rule() {
        let dir = tempfile::tempdir().unwrap();
        let engine = crate::build_engine_with(&vak_config::Config::default(), &[]).unwrap();
        let args = serde_json::json!({});
        assert!(!SHAPES.is_empty());
        for shape in SHAPES {
            for mode in [
                vak_permission::Mode::ReadOnly,
                vak_permission::Mode::WorkspaceWrite,
                vak_permission::Mode::FullAccess,
            ] {
                assert!(
                    matches!(
                        engine.evaluate(shape.name, &args, mode, dir.path()),
                        vak_permission::Decision::Allow
                    ),
                    "{} in {mode:?}",
                    shape.name
                );
            }
        }
        assert_eq!(presenting_tool_names().len(), SHAPES.len());
    }

    #[test]
    fn every_shape_has_a_unique_tool_name() {
        let names: std::collections::HashSet<&str> = SHAPES.iter().map(|s| s.name).collect();
        assert_eq!(names.len(), SHAPES.len(), "tool names must be unique");
    }

    #[test]
    fn every_shape_has_at_least_one_semantic_type() {
        for shape in SHAPES {
            assert!(
                !shape.semantic_types.is_empty(),
                "{} has no semantic types",
                shape.name
            );
        }
    }

    #[test]
    fn no_semantic_type_is_claimed_by_two_shapes() {
        let mut seen = std::collections::HashMap::new();
        for shape in SHAPES {
            for &t in shape.semantic_types {
                if let Some(prev) = seen.insert(t, shape.name) {
                    panic!(
                        "semantic_type `{t}` claimed by both {prev} and {}",
                        shape.name
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn a_valid_call_is_acked_and_rebuilds_into_a_card() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|t| t.name() == "emit_chart_card")
            .unwrap();
        let args = serde_json::json!({
            "semantic_type": "chart",
            "chart_type": "line",
            "accessible_summary": "flat line",
            "data": [{"series": "s1", "x": "1", "y": 2.0}]
        });
        let out = tool
            .execute(&args, &ToolContext::new(std::env::temp_dir()))
            .await;
        assert!(!out.is_error(), "expected Ok, got: {}", out.content);
        assert!(out.content.contains("Shown to the user"), "{}", out.content);
        assert!(out.content.contains("chart"), "{}", out.content);
        assert!(!out.content.contains("emit"), "{}", out.content);
        assert!(
            !out.content.contains("other distinct cards"),
            "successful tool output should acknowledge this card without inviting more: {}",
            out.content
        );
        let card = rebuild_call(
            "emit_chart_card",
            &args,
            &vak_delivery::built_in_skill_registry(),
        )
        .expect("the call's own arguments must rebuild into a card");
        assert_eq!(card.semantic_type, "chart");
    }

    #[tokio::test]
    async fn an_invalid_payload_is_a_repairable_tool_error_not_a_silent_drop() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|t| t.name() == "emit_chart_card")
            .unwrap();
        let args = serde_json::json!({
            "semantic_type": "chart",
            "chart_type": "line",
            "data": []
        });
        let out = tool
            .execute(&args, &ToolContext::new(std::env::temp_dir()))
            .await;
        assert!(
            out.is_error(),
            "validator-rejected card must be an error the model sees"
        );
        assert!(out.content.contains("Fix these fields"), "{}", out.content);
    }

    #[tokio::test]
    async fn a_card_far_larger_than_the_tool_output_line_limit_still_rebuilds() {
        let long = "x".repeat(6000);
        let args = serde_json::json!({
            "semantic_type": "research.synthesis",
            "sources": [{"title": "S", "url": "https://example.com", "finding": long}]
        });
        let card = rebuild_call(
            "emit_research_card",
            &args,
            &vak_delivery::built_in_skill_registry(),
        )
        .expect("size must not matter: the card comes from the call arguments");
        assert_eq!(card.semantic_type, "research.synthesis");
    }

    #[tokio::test]
    async fn execute_rejects_a_semantic_type_outside_its_own_shape() {
        let tool = EmitCardTool::all()
            .into_iter()
            .find(|t| t.name() == "emit_chart_card")
            .unwrap();
        let args = serde_json::json!({"semantic_type": "recipe.card"});
        let ctx = ToolContext::new(std::env::temp_dir());
        let out = tool.execute(&args, &ctx).await;
        assert!(out.is_error(), "a chart tool must refuse a recipe type");
    }

    #[test]
    fn every_card_conformance_case_passes_the_pre_execution_schema_gate() {
        let tools = EmitCardTool::all();
        let mut failures = Vec::new();
        for (name, semantic_type, mut args) in conformance_cases() {
            let tool = tools
                .iter()
                .find(|tool| tool.name() == name)
                .expect("shape tool exists");
            sent(tool, &mut args);
            if let Err(error) = vak_tools::validate_input(&tool.schema(), &args) {
                failures.push(format!("{semantic_type} via {name}: {error}"));
                continue;
            }
            if presentation_info(name, &args, &vak_delivery::built_in_skill_registry()).is_none() {
                failures.push(format!(
                    "{semantic_type} via {name}: builder/registry rejected schema-valid call"
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} pre-execution card gate failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[tokio::test]
    async fn every_registered_semantic_type_across_all_shapes_renders() {
        let skills = vak_delivery::built_in_skill_registry();
        let mut failures = Vec::new();
        for tool in EmitCardTool::all() {
            for (name, semantic_type, args) in conformance_cases()
                .into_iter()
                .filter(|(name, _, _)| *name == tool.name())
            {
                let ctx = ToolContext::new(std::env::temp_dir());
                let out = tool.execute(&args, &ctx).await;
                if out.is_error() {
                    failures.push(format!(
                        "{semantic_type} ({name}): rejected: {}",
                        out.content
                    ));
                    continue;
                }
                let card = rebuild_call(name, &args, &skills);
                if card.as_ref().map(|c| c.semantic_type.as_str()) != Some(semantic_type) {
                    failures.push(format!(
                        "{semantic_type} ({name}): did not rebuild into a card"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} registered types failed:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn every_starter_presentation_pack_compiles_with_its_emitter_payload() {
        let skills = vak_delivery::built_in_skill_registry();
        let cases = conformance_cases();
        let mut failures = Vec::new();

        for seed in vak_presentation::seeds::built_in_seed_pack() {
            if seed.spec.accepts.is_empty() {
                failures.push(format!("{} accepts no semantic type", seed.spec.id));
                continue;
            }
            for semantic_type in &seed.spec.accepts {
                let Some((tool, _, args)) =
                    cases.iter().find(|(_, kind, _)| *kind == semantic_type)
                else {
                    // Every pack draws a type some block can produce: a pack
                    // nothing can reach is a card the model lost.
                    failures.push(format!(
                        "{} accepts {semantic_type}, which no card block produces",
                        seed.spec.id
                    ));
                    continue;
                };
                let Some(info) = presentation_info(tool, args, &skills) else {
                    failures.push(format!(
                        "{} emitter fixture for {semantic_type} was rejected",
                        seed.spec.id
                    ));
                    continue;
                };
                let rendered = vak_presentation::compile(
                    &seed.spec,
                    &vak_presentation::CompileInput {
                        semantic_type: semantic_type.clone(),
                        payload: info.payload,
                        fallback_text: info.title,
                    },
                );
                match rendered {
                    vak_presentation::CompiledPresentation::Rich(tree) => {
                        if matches!(
                            semantic_type.as_str(),
                            "recipe.card"
                                | "recipe"
                                | "recipe_summary"
                                | "lifestyle.recipe"
                                | "lifestyle.culinary_recipe"
                        ) {
                            let ingredient = tree
                                .root
                                .children
                                .first()
                                .and_then(|list| list.children.first())
                                .and_then(|item| item.props.get("text"))
                                .and_then(Value::as_str);
                            let step = tree
                                .root
                                .children
                                .get(1)
                                .and_then(|list| list.children.first())
                                .and_then(|item| item.props.get("text"))
                                .and_then(Value::as_str);
                            if ingredient != Some("1 cup water") || step != Some("Boil") {
                                failures.push(format!(
                                    "{} ({semantic_type}) compiled a recipe with empty or altered emitter strings: ingredient={ingredient:?}, step={step:?}",
                                    seed.spec.id
                                ));
                            }
                        }
                    }
                    fallback => failures.push(format!(
                        "{} ({semantic_type}) fell back instead of compiling rich: {fallback:?}",
                        seed.spec.id
                    )),
                }
            }
        }

        assert!(
            failures.is_empty(),
            "{} of the {} built-in presentation packs did not compile:\n{}",
            failures.len(),
            vak_presentation::seeds::built_in_seed_pack().len(),
            failures.join("\n")
        );
    }

    /// A details card's keys are its facts, so its label is its title,
    /// never a fact named "label"; another card keeps it as `label`.
    #[test]
    fn a_label_is_never_one_of_the_facts() {
        let details = built(
            "emit_details_card",
            serde_json::json!({"label": "Summary", "fields": [{"label": "Best", "detail": "Notion"}]}),
        )
        .unwrap();
        assert_eq!(details["title"], "Summary");
        assert!(details.get("label").is_none(), "{details}");
        assert_eq!(details["Best"], "Notion");
        let titled = built(
            "emit_details_card",
            serde_json::json!({"label": "Summary", "title": "Apps", "fields": [{"label": "Best", "detail": "Notion"}]}),
        )
        .unwrap();
        assert_eq!(titled["title"], "Apps");
        let lead = built(
            "emit_summary_card",
            serde_json::json!({"label": "Summary", "headline": "Notion"}),
        )
        .unwrap();
        assert_eq!(lead["label"], "Summary");
    }

    /// One argument name is one shape on every card tool, so the same idea
    /// reads the same everywhere and a schema built from several of them
    /// never loses a type (docs/audits/cards-in-answers-2026-10-11.md, F2).
    #[test]
    fn one_argument_name_is_one_shape_on_every_block() {
        let shape = |property: &Value| serde_json::json!({"type": property["type"], "items": property["items"], "enum": property["enum"]});
        let mut seen: std::collections::BTreeMap<String, (String, Value)> = Default::default();
        let mut problems = Vec::new();
        for tool in EmitCardTool::all() {
            let schema = tool.schema();
            for (name, property) in schema["properties"].as_object().into_iter().flatten() {
                if name == "semantic_type" {
                    continue;
                }
                let mut ours = shape(property);
                strip_descriptions(&mut ours);
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

    fn strip_descriptions(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("description");
                map.values_mut().for_each(strip_descriptions);
            }
            Value::Array(items) => items.iter_mut().for_each(strip_descriptions),
            _ => {}
        }
    }

    #[test]
    fn every_block_type_is_registered_and_has_one_loaded_block() {
        let tools: Vec<std::sync::Arc<dyn Tool>> = EmitCardTool::all()
            .into_iter()
            .map(|tool| std::sync::Arc::new(tool) as std::sync::Arc<dyn Tool>)
            .collect();
        let surface = crate::capability::build_tool_surface(&tools, true, &Default::default());
        assert!(surface.deferred.is_empty());
        // Every block type is registered and has exactly one block. A type
        // only history carries still renders, but no block emits it.
        let registered = vak_delivery::skills::built_in_semantic_types();
        for semantic_type in SHAPES.iter().flat_map(|shape| shape.semantic_types.iter()) {
            assert!(
                registered.iter().any(|known| known == semantic_type),
                "block type `{semantic_type}` is not registered"
            );
            let matches: Vec<_> = SHAPES
                .iter()
                .filter(|shape| shape.semantic_types.contains(semantic_type))
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "block type `{semantic_type}` must map to exactly one block"
            );
            assert!(
                surface
                    .core
                    .iter()
                    .any(|definition| definition.name == matches[0].name),
                "mapped emitter `{}` is not loaded for an unpredicted request",
                matches[0].name
            );
        }
    }

    #[test]
    fn every_emit_tool_is_reachable_from_its_types() {
        for shape in SHAPES {
            for t in shape.semantic_types {
                assert_eq!(emit_tool_for(t), Some(shape.name));
            }
        }
    }

    #[test]
    fn identity_digest_for_metric_is_the_whole_compact_payload() {
        let payload = serde_json::json!({"label": "Uptime", "value": 99.9, "unit": "%"});
        let digest = identity_digest("metric", &payload);
        // The whole card, canonical and compact — every field survives.
        assert!(digest.contains("\"label\":\"Uptime\""), "{digest}");
        assert!(digest.contains("\"value\":99.9"), "{digest}");
        assert!(digest.contains("\"unit\":\"%\""), "{digest}");
    }

    #[test]
    fn identity_digest_for_research_synthesis_is_takeaways_and_sources() {
        let payload = serde_json::json!({
            "sources": [{"title": "Reuters", "url": "https://example.com/a"}],
            "takeaways": [{"text": "Markets fell", "citation_indices": [1]}]
        });
        let digest = identity_digest("research.synthesis", &payload);
        assert!(digest.contains("Markets fell"), "{digest}");
        assert!(digest.contains("Reuters"), "{digest}");
        assert!(digest.contains("https://example.com/a"), "{digest}");
        // Not the raw snippet field, which the design excludes.
        assert!(!digest.contains("snippet"), "{digest}");
    }

    #[test]
    fn identity_digest_for_table_is_title_columns_row_count_and_first_row() {
        let payload = serde_json::json!({
            "title": "Q3 Budget",
            "columns": [{"key": "dept", "label": "Department"}, {"key": "spend", "label": "Spend"}],
            "rows": [{"dept": "Eng", "spend": 100}, {"dept": "Sales", "spend": 50}]
        });
        let digest = identity_digest("table", &payload);
        assert!(digest.contains("Q3 Budget"), "{digest}");
        assert!(
            digest.contains("dept") && digest.contains("spend"),
            "{digest}"
        );
        assert!(digest.contains("rows: 2"), "{digest}");
        assert!(digest.contains("Eng"), "{digest}");
        assert!(
            !digest.contains("Sales"),
            "digest must not include every row: {digest}"
        );
    }

    #[test]
    fn identity_digest_for_chart_is_title_series_points_and_summary() {
        let payload = serde_json::json!({
            "title": "Revenue",
            "chart_type": "line",
            "accessible_summary": "rising trend",
            "series": [{"name": "actual", "points": [{"x": 1, "y": 2.0}, {"x": 2, "y": 3.0}]}]
        });
        let digest = identity_digest("chart", &payload);
        assert!(digest.contains("Revenue"), "{digest}");
        assert!(digest.contains("actual(2)"), "{digest}");
        assert!(digest.contains("rising trend"), "{digest}");
    }

    #[test]
    fn identity_digest_for_entity_is_title_type_and_fields() {
        let payload = serde_json::json!({
            "title": "Paris",
            "type": "city",
            "population": "2.1M",
            "country": "France"
        });
        let digest = identity_digest("entity", &payload);
        assert!(digest.contains("Paris"), "{digest}");
        assert!(digest.contains("type: city"), "{digest}");
        assert!(digest.contains("population=2.1M"), "{digest}");
        assert!(digest.contains("country=France"), "{digest}");
    }

    #[test]
    fn identity_digest_for_checklist_timeline_itinerary_is_title_and_item_labels() {
        let payload = serde_json::json!({
            "title": "Trip",
            "items": [{"label": "Fly to Paris"}, {"label": "Check into hotel"}]
        });
        for semantic_type in ["checklist", "timeline", "itinerary"] {
            let digest = identity_digest(semantic_type, &payload);
            assert!(digest.contains("Trip"), "{semantic_type}: {digest}");
            assert!(digest.contains("Fly to Paris"), "{semantic_type}: {digest}");
            assert!(
                digest.contains("Check into hotel"),
                "{semantic_type}: {digest}"
            );
        }
    }

    #[test]
    fn ui_preview_never_carries_isolation_settings_from_the_model() {
        let payload = normalize_payload(
            "ui.preview",
            serde_json::json!({
                "title": "T",
                "html": "<p>x</p>",
                "sandbox": "allow-scripts allow-same-origin",
                "connect_src": "https://*"
            }),
        );
        assert_eq!(
            payload,
            serde_json::json!({"title": "T", "html": "<p>x</p>"})
        );
        let schema = ui_preview_fields();
        assert!(schema["properties"].get("sandbox").is_none());
        assert!(schema["properties"].get("connect_src").is_none());
    }

    #[test]
    fn identity_digest_for_anything_else_is_title_plus_top_level_keys() {
        let payload =
            serde_json::json!({"title": "Custom", "status": "ready", "artifact_path": "a.html"});
        let digest = identity_digest("ui.preview", &payload);
        assert!(digest.contains("Custom"), "{digest}");
        assert!(digest.contains("status"), "{digest}");
        assert!(digest.contains("artifact_path"), "{digest}");
    }

    #[test]
    fn presentation_info_rebuilds_a_validated_call_into_a_ledger_ready_record() {
        let skills = vak_delivery::built_in_skill_registry();
        let args = serde_json::json!({
            "semantic_type": "chart",
            "title": "Revenue",
            "chart_type": "line",
            "accessible_summary": "flat",
            "data": [{"series": "s1", "x": "1", "y": 2.0}]
        });
        let info = presentation_info("emit_chart_card", &args, &skills)
            .expect("a valid call must rebuild");
        assert_eq!(info.semantic_type, "chart");
        assert_eq!(info.title, "Revenue");
        assert!(
            info.identity_digest.contains("s1(1)"),
            "{}",
            info.identity_digest
        );
        assert!(info.schema_version > 0);
        assert!(!info.skill_id.is_empty());
    }

    #[test]
    fn presentation_info_is_none_for_an_invalid_call() {
        let skills = vak_delivery::built_in_skill_registry();
        let args = serde_json::json!({"semantic_type": "chart", "data": []});
        assert!(presentation_info("emit_chart_card", &args, &skills).is_none());
    }
}
