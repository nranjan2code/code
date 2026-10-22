use super::{Element, Message, inline, split_units, table_panels, table_record, units};
use serde_json::{Value, json};

const SLACK_BLOCKS: usize = 50;
const SLACK_MESSAGE_CHARS: usize = 40_000;
const SLACK_TABLE_CHARS: usize = 10_000;
const TABLE_COLUMNS: usize = 20;

pub(super) fn compile(elements: &[Element]) -> Vec<Message> {
    let mut blocks = Vec::new();
    for element in elements {
        slack_elements(element, &mut blocks);
    }
    let mut messages = Vec::new();
    let mut batch = Vec::new();
    let mut chars = 0;
    for block in blocks {
        // One table per message avoids aggregate table-character limits.
        if !batch.is_empty()
            && (chars + json_text_units(&block) + 2 > SLACK_MESSAGE_CHARS
                || batch.len() == SLACK_BLOCKS
                || block["type"] == "table"
                || batch.iter().any(|b: &Value| b["type"] == "table"))
        {
            messages.push(slack_message(std::mem::take(&mut batch)));
            chars = 0;
        }
        chars += json_text_units(&block) + 2;
        batch.push(block);
    }
    if !batch.is_empty() {
        messages.push(slack_message(batch));
    }
    messages
}
fn json_text_units(value: &Value) -> usize {
    match value {
        Value::String(text) => units(text),
        Value::Array(items) => items.iter().map(json_text_units).sum(),
        Value::Object(fields) => fields.values().map(json_text_units).sum(),
        _ => 0,
    }
}
fn slack_message(blocks: Vec<Value>) -> Message {
    // Top-level text is the complete accessible/notification representation
    // of this message's blocks. Slack shows the blocks in the conversation.
    let fallback = blocks
        .iter()
        .map(slack_plain)
        .collect::<Vec<_>>()
        .join("\n\n");
    Message {
        method: "chat.postMessage",
        body: json!({"blocks":blocks,"text":fallback,"mrkdwn":false,"parse":"none","link_names":false,"unfurl_links":false,"unfurl_media":false}),
    }
}
fn slack_plain(value: &Value) -> String {
    match value["type"].as_str().unwrap_or_default() {
        "text" | "raw_text" => value["text"].as_str().unwrap_or_default().into(),
        "link" => format!(
            "{} ({})",
            value["text"].as_str().unwrap_or_default(),
            value["url"].as_str().unwrap_or_default()
        ),
        "rich_text_list" => {
            let offset = value["offset"].as_u64().unwrap_or(0);
            value["elements"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(i, item)| {
                    format!(
                        "{} {}",
                        if value["style"] == "ordered" {
                            format!("{}.", offset + i as u64 + 1)
                        } else {
                            "•".into()
                        },
                        slack_plain(item)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "table" => value["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|row| {
                row.as_array()
                    .into_iter()
                    .flatten()
                    .map(slack_plain)
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => value["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .map(slack_plain)
            .collect::<Vec<_>>()
            .join(if value["type"] == "rich_text_section" {
                ""
            } else {
                "\n"
            }),
    }
}
/// Reserve the exact native wrapper when applying Slack's 40,000-character
/// message bound. Rich-text sections do not inherit section-block's 3,000
/// limit; no independent readability cap is invented here.
fn slack_text_budget() -> usize {
    let section = slack_section("", None);
    let pre = json!({"type":"rich_text_preformatted","elements":section["elements"]});
    let wrapper = |element| json_text_units(&json!({"type":"rich_text","elements":[element]}));
    SLACK_MESSAGE_CHARS - wrapper(section).max(wrapper(pre)) - 2
}
fn slack_section(text: &str, style: Option<&str>) -> Value {
    let mut element = json!({"type":"text","text":text});
    if let Some(style) = style {
        element["style"] = json!({style:true});
    }
    json!({"type":"rich_text_section","elements":[element]})
}
fn slack_run(run: &inline::Run) -> Value {
    let mut value = if let Some(url) = &run.url {
        json!({"type":"link","url":url,"text":run.text})
    } else {
        json!({"type":"text","text":run.text})
    };
    let mut style = serde_json::Map::new();
    for (enabled, key) in [
        (run.code, "code"),
        (run.strike, "strike"),
        (run.italic, "italic"),
        (run.bold, "bold"),
    ] {
        if enabled {
            style.insert(key.into(), json!(true));
        }
    }
    if !style.is_empty() {
        value["style"] = Value::Object(style);
    }
    value
}
fn slack_rich(elements: &[Value]) -> Value {
    json!({"type":"rich_text","elements":[{"type":"rich_text_section","elements":elements}]})
}
fn slack_prose(runs: Vec<inline::Run>, out: &mut Vec<Value>) {
    let base = json_text_units(&slack_rich(&[])) + 2;
    let mut elements = Vec::new();
    let mut used = base;
    for run in runs {
        let value = slack_run(&run);
        let size = json_text_units(&value);
        if size + base > SLACK_MESSAGE_CHARS {
            if !elements.is_empty() {
                out.push(slack_rich(&std::mem::take(&mut elements)));
                used = base;
            }
            slack_elements(
                &Element::Text(format!(
                    "{} ({})",
                    run.text,
                    run.url.as_deref().unwrap_or_default()
                )),
                out,
            );
            continue;
        }
        if used + size > SLACK_MESSAGE_CHARS && !elements.is_empty() {
            out.push(slack_rich(&std::mem::take(&mut elements)));
            used = base;
        }
        used += size;
        elements.push(value);
    }
    if !elements.is_empty() {
        out.push(slack_rich(&elements));
    }
}
fn slack_elements(element: &Element, out: &mut Vec<Value>) {
    match element {
        Element::Heading(s) | Element::Text(s) | Element::Context(s) | Element::Code(s) => {
            for part in split_units(s, slack_text_budget()) {
                if part.is_empty() {
                    continue;
                }
                let section = slack_section(
                    &part,
                    match element {
                        Element::Heading(_) => Some("bold"),
                        Element::Context(_) => Some("italic"),
                        _ => None,
                    },
                );
                let body = if matches!(element, Element::Code(_)) {
                    json!({"type":"rich_text_preformatted","elements":section["elements"]})
                } else {
                    section
                };
                out.push(json!({"type":"rich_text","elements":[body]}));
            }
        }
        Element::Prose(source) => {
            for runs in inline::chunks(inline::runs(source), slack_text_budget()) {
                slack_prose(runs, out);
            }
        }

        Element::Field { label, value } => {
            for part in split_units(label, slack_text_budget()) {
                if !part.is_empty() {
                    out.push(
                        json!({"type":"rich_text","elements":[slack_section(&part,Some("bold"))]}),
                    );
                }
            }
            slack_elements(&Element::Text(value.clone()), out);
        }
        Element::Link { text, url } => {
            for part in split_units(text, slack_text_budget()) {
                if part.is_empty() {
                    continue;
                }
                let value = json!({"type":"link","url":url,"text":part});
                let block = slack_rich(&[value]);
                if json_text_units(&block) + 2 > SLACK_MESSAGE_CHARS {
                    slack_elements(&Element::Text(format!("{part} ({url})")), out);
                } else {
                    out.push(block);
                }
            }
        }

        Element::Details { summary, body } => {
            slack_elements(&Element::Heading(summary.clone()), out);
            for element in body {
                slack_elements(element, out);
            }
        }
        Element::List { ordered, items } => {
            for (i, item) in items.iter().enumerate() {
                let mut blocks = Vec::new();
                for element in item {
                    slack_elements(element, &mut blocks);
                }
                if let Some(first) = blocks.first_mut()
                    && first["type"] == "rich_text"
                    && first["elements"][0]["type"] == "rich_text_section"
                {
                    let section = first["elements"][0].clone();
                    first["elements"] = json!([{"type":"rich_text_list","style":if *ordered {"ordered"}else{"bullet"},"offset":if *ordered {i}else{0},"elements":[section]}]);
                }
                out.extend(blocks);
            }
        }
        Element::Table { header, rows } => {
            for (header, rows) in table_panels(header, rows, TABLE_COLUMNS) {
                let empty = rows.is_empty();
                let cell = |s: &String| json!({"type":"raw_text","text":s});
                let mut batch = vec![header.iter().map(cell).collect::<Vec<_>>()];
                let mut chars = header.iter().map(|s| units(s)).sum::<usize>();
                for row in rows {
                    let size = row.iter().map(|s| units(s)).sum::<usize>();
                    if size + header.iter().map(|s| units(s)).sum::<usize>() > SLACK_TABLE_CHARS {
                        if batch.len() > 1 {
                            out.push(json!({"type":"table","rows":std::mem::take(&mut batch)}));
                        }
                        batch = vec![header.iter().map(cell).collect()];
                        chars = header.iter().map(|s| units(s)).sum();
                        for element in table_record(&header, &row) {
                            slack_elements(&element, out);
                        }
                        continue;
                    }
                    if batch.len() == 100 || chars + size > SLACK_TABLE_CHARS {
                        out.push(json!({"type":"table","rows":batch}));
                        batch = vec![header.iter().map(cell).collect()];
                        chars = header.iter().map(|s| units(s)).sum();
                    }
                    chars += size;
                    batch.push(row.iter().map(cell).collect());
                }
                if batch.len() > 1 {
                    out.push(json!({"type":"table","rows":batch}));
                } else if empty {
                    slack_elements(&Element::Context(header.join(" | ")), out);
                }
            }
        }
    }
}
