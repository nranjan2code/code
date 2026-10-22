use super::{Element, Message, inline, plain, split_units, table_panels, table_record, units};
use serde_json::{Value, json};

const TELEGRAM_CHARS: usize = 32_768;
const TELEGRAM_BLOCKS: usize = 500;
const TABLE_COLUMNS: usize = 20;

// Prose styles are flattened before encoding RichText. Authored HTML stays
// literal; only explicit safe links acquire URL behavior. No media is fetched.
fn tg_text(kind: &str, text: &str) -> Value {
    let mut value = json!({"type":kind,"text":text});
    if kind == "heading" {
        value["size"] = json!(2);
    }
    value
}
fn tg_link(text: &str, url: &str) -> Value {
    json!({"type":"paragraph","text":{"type":"url","text":text,"url":url}})
}

fn tg_run(run: &inline::Run) -> Value {
    let mut value = json!(run.text);
    for (enabled, kind) in [
        (run.code, "code"),
        (run.strike, "strikethrough"),
        (run.italic, "italic"),
        (run.bold, "bold"),
    ] {
        if enabled {
            value = json!({"type":kind,"text":value});
        }
    }
    if let Some(url) = &run.url {
        value = json!({"type":"url","url":url,"text":value});
    }
    value
}
pub(super) fn compile(elements: &[Element]) -> Vec<Message> {
    let mut blocks = Vec::new();
    for element in elements {
        tg_elements(element, &mut blocks);
    }
    let mut messages = Vec::new();
    let mut batch = Vec::new();
    let mut chars = 0;
    let mut count = 0;
    for block in blocks {
        let (c, n) = cost(&block);
        if !batch.is_empty() && (chars + c > TELEGRAM_CHARS || count + n > TELEGRAM_BLOCKS) {
            messages.push(tg_message(std::mem::take(&mut batch)));
            chars = 0;
            count = 0;
        }
        chars += c;
        count += n;
        batch.push(block);
    }
    if !batch.is_empty() {
        messages.push(tg_message(batch));
    }
    messages
}
fn tg_message(blocks: Vec<Value>) -> Message {
    Message {
        method: "sendRichMessage",
        body: json!({"rich_message":{"blocks":blocks,"skip_entity_detection":true}}),
    }
}
pub(super) fn cost(v: &Value) -> (usize, usize) {
    match v {
        Value::String(s) => (units(s), 0),
        Value::Array(a) => a
            .iter()
            .map(cost)
            .fold((0, 0), |(a, b), (c, d)| (a + c, b + d)),
        Value::Object(o) => {
            let mut total = (
                0,
                usize::from(matches!(
                    o.get("type").and_then(Value::as_str),
                    Some("paragraph" | "heading" | "pre" | "footer" | "details" | "list" | "table")
                )),
            );
            for (key, value) in o {
                if !matches!(
                    key.as_str(),
                    "type" | "url" | "size" | "is_header" | "align"
                ) {
                    let (c, n) = cost(value);
                    total.0 += c;
                    total.1 += n;
                }
            }
            // Table rows/list items count too, not only typed blocks.
            if o.get("type").and_then(Value::as_str) == Some("table") {
                total.1 += o.get("cells").and_then(Value::as_array).map_or(0, Vec::len);
            }
            if o.get("type").and_then(Value::as_str) == Some("list") {
                total.1 += o.get("items").and_then(Value::as_array).map_or(0, Vec::len);
            }
            total
        }
        _ => (0, 0),
    }
}
fn tg_elements(element: &Element, out: &mut Vec<Value>) {
    match element {
        Element::Heading(s) | Element::Text(s) | Element::Context(s) | Element::Code(s) => {
            let kind = match element {
                Element::Heading(_) => "heading",
                Element::Context(_) => "footer",
                Element::Code(_) => "pre",
                _ => "paragraph",
            };
            for part in split_units(s, TELEGRAM_CHARS) {
                if !part.is_empty() {
                    out.push(tg_text(kind, &part));
                }
            }
        }
        Element::Prose(source) => {
            for runs in inline::chunks(inline::runs(source), TELEGRAM_CHARS) {
                let content = runs.iter().map(tg_run).collect::<Vec<_>>();
                out.push(json!({"type":"paragraph","text":content}));
            }
        }
        Element::Field { label, value } => {
            if units(label) + 2 >= TELEGRAM_CHARS {
                tg_elements(&Element::Heading(label.clone()), out);
                tg_elements(&Element::Text(value.clone()), out);
            } else {
                for part in split_units(value, TELEGRAM_CHARS - units(label) - 1) {
                    out.push(
                        json!({"type":"paragraph","text":[{"type":"bold","text":label},"\n",part]}),
                    );
                }
            }
        }
        Element::Link { text, url } => {
            for part in split_units(text, TELEGRAM_CHARS) {
                if !part.is_empty() {
                    out.push(tg_link(&part, url));
                }
            }
        }
        Element::Details { summary, body } => {
            let mut nested = Vec::new();
            for element in body {
                tg_elements(element, &mut nested);
            }
            let detail = json!({"type":"details","summary":summary,"blocks":nested});
            let (chars, blocks) = cost(&detail);
            if chars <= TELEGRAM_CHARS && blocks <= TELEGRAM_BLOCKS && !body.is_empty() {
                out.push(detail);
            } else {
                // Oversized disclosure continues visibly; no hidden body is
                // truncated just to retain the collapsible container.
                tg_elements(&Element::Heading(summary.clone()), out);
                for element in body {
                    tg_elements(element, out);
                }
            }
        }
        Element::List { ordered, items } => {
            for (index, item) in items.iter().enumerate() {
                let mut body = Vec::new();
                for element in item {
                    tg_elements(element, &mut body);
                }
                let mut value = json!({"blocks":body});
                if *ordered {
                    value["value"] = json!(index + 1);
                    value["type"] = json!("1");
                }
                let block = json!({"type":"list","items":[value]});
                let (chars, blocks) = cost(&block);
                if chars <= TELEGRAM_CHARS && blocks <= TELEGRAM_BLOCKS {
                    out.push(block);
                } else {
                    tg_elements(
                        &Element::Text(format!(
                            "{} {}",
                            if *ordered {
                                format!("{}.", index + 1)
                            } else {
                                "•".into()
                            },
                            plain(item)
                        )),
                        out,
                    );
                }
            }
        }
        Element::Table { header, rows } => {
            for (header, rows) in table_panels(header, rows, TABLE_COLUMNS) {
                let empty = rows.is_empty();
                // Pack complete rows and repeat headers at continuation.
                let cells = |row: &[String], head: bool| {
                    row.iter()
                        .map(|v| {
                            let mut cell = json!({"text":v,"align":"left"});
                            if head {
                                cell["is_header"] = json!(true);
                            }
                            cell
                        })
                        .collect::<Vec<_>>()
                };
                let mut batch = vec![cells(&header, true)];
                for row in rows {
                    let mut trial = batch.clone();
                    trial.push(cells(&row, false));
                    let block = json!({"type":"table","cells":trial});
                    let (c, n) = cost(&block);
                    if c > TELEGRAM_CHARS || n > TELEGRAM_BLOCKS {
                        if batch.len() > 1 {
                            out.push(json!({"type":"table","cells":batch}));
                        }
                        batch = vec![cells(&header, true)];
                        let one =
                            json!({"type":"table","cells":[cells(&header,true),cells(&row,false)]});
                        if cost(&one).0 > TELEGRAM_CHARS {
                            table_record(&header, &row)
                                .iter()
                                .for_each(|e| tg_elements(e, out));
                            continue;
                        }
                    }
                    batch.push(cells(&row, false));
                }
                if batch.len() > 1 {
                    out.push(json!({"type":"table","cells":batch}));
                } else if empty {
                    // preserve even an empty table's columns
                    let header_text = header.join(" | ");
                    tg_elements(&Element::Context(header_text), out);
                }
            }
        }
    }
}
