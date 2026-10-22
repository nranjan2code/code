use super::{Card, Element};
use crate::StructuredOutput;
use serde_json::Value;

fn text(p: &Value, key: &str) -> String {
    p[key].as_str().unwrap_or_default().into()
}
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "—".into(),
        _ => v.to_string(),
    }
}
fn items<'a>(p: &'a Value, key: &str) -> &'a [Value] {
    p[key].as_array().map(Vec::as_slice).unwrap_or_default()
}
fn joined(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}
fn add(out: &mut Vec<Element>, value: String) {
    if !value.is_empty() {
        out.push(Element::Prose(value));
    }
}
fn link(out: &mut Vec<Element>, label: String, url: String) {
    // Unsafe links remain visible as inert text; never ask the provider to
    // fetch media, execute markup, detect mentions or interpret an unsafe URL.
    if super::inline::link_allowed(&url) {
        out.push(Element::Link { text: label, url });
    } else {
        add(out, format!("{label} ({url})"));
    }
}
fn list(ordered: bool, values: Vec<Vec<Element>>) -> Element {
    Element::List {
        ordered,
        items: values,
    }
}
fn points(p: &Value, key: &str) -> Vec<Vec<Element>> {
    items(p, key)
        .iter()
        .map(|v| vec![Element::Prose(scalar(v))])
        .collect()
}
fn table(header: Vec<String>, rows: Vec<Vec<String>>) -> Element {
    Element::Table { header, rows }
}

pub(super) fn project(output: &StructuredOutput) -> Result<Card, String> {
    let p = &output.payload;
    let mut out = Vec::new();
    let mut limitations = Vec::new();
    let title = if output.block == "summary" {
        text(p, "headline")
    } else {
        text(p, "title")
    };
    if !title.is_empty() {
        out.push(Element::Heading(title.clone()));
    }
    let label = text(p, "label");
    if !label.is_empty() && label != title {
        out.push(Element::Context(label));
    }
    match output.block.as_str() {
        "summary" => {
            add(&mut out, text(p, "text"));
            if !items(p, "points").is_empty() {
                out.push(list(false, points(p, "points")));
            }
        }
        "steps" => {
            let mut steps = Vec::new();
            for step in items(p, "steps") {
                let mut body = vec![Element::Text(joined([
                    text(step, "time"),
                    text(step, "label"),
                ]))];
                add(&mut body, text(step, "detail"));
                for choice in items(step, "choices") {
                    let mut choices = vec![Element::Text(text(choice, "label"))];
                    add(&mut choices, text(choice, "detail"));
                    if !items(choice, "facts").is_empty() {
                        choices.push(list(false, points(choice, "facts")));
                    }
                    body.push(Element::Details {
                        summary: text(choice, "label"),
                        body: choices.into_iter().skip(1).collect(),
                    });
                }
                steps.push(body);
            }
            out.push(list(true, steps));
        }
        "list" => {
            let checkable = p["checkable"].as_bool() == Some(true);
            let mut rows = Vec::new();
            for item in items(p, "items") {
                let mut row = Vec::new();
                let label = format!(
                    "{}{}",
                    if checkable { "☐ " } else { "" },
                    text(item, "label")
                );
                if text(item, "url").is_empty() {
                    add(&mut row, label);
                } else {
                    link(&mut row, label, text(item, "url"));
                }
                add(&mut row, text(item, "detail"));
                rows.push(row);
            }
            out.push(list(false, rows));
            if checkable {
                limitations.push("Checklist shown as a static list.".into());
            }
        }
        "details" => {
            add(&mut out, text(p, "text"));
            for field in items(p, "fields") {
                out.push(Element::Field {
                    label: text(field, "label"),
                    value: text(field, "detail"),
                });
            }
        }
        "table" => out.push(table(
            items(p, "columns")
                .iter()
                .map(|c| text(c, "label"))
                .collect(),
            items(p, "rows")
                .iter()
                .map(|r| r.as_array().into_iter().flatten().map(scalar).collect())
                .collect(),
        )),
        "comparison" => {
            for option in items(p, "options") {
                out.push(Element::Heading(text(option, "name")));
                add(&mut out, text(option, "verdict"));
                for (key, label) in [("strengths", "Strengths"), ("weaknesses", "Weaknesses")] {
                    if !items(option, key).is_empty() {
                        out.push(Element::Details {
                            summary: label.into(),
                            body: vec![list(false, points(option, key))],
                        });
                    }
                }
            }
            if !items(p, "aspects").is_empty() {
                let mut headers = vec!["Aspect".into()];
                headers.extend(items(p, "options").iter().map(|o| text(o, "name")));
                out.push(table(
                    headers,
                    items(p, "aspects")
                        .iter()
                        .map(|a| {
                            std::iter::once(text(a, "label"))
                                .chain(items(a, "values").iter().map(scalar))
                                .collect()
                        })
                        .collect(),
                ));
            }
            if !text(p, "recommendation").is_empty() {
                out.push(Element::Field {
                    label: "Recommendation".into(),
                    value: text(p, "recommendation"),
                });
            }
        }
        "stats" => {
            for reading in items(p, "readings") {
                out.push(Element::Field {
                    label: text(reading, "label"),
                    value: joined([
                        if text(reading, "unit").is_empty() {
                            scalar(&reading["value"])
                        } else {
                            format!("{} {}", scalar(&reading["value"]), text(reading, "unit"))
                        },
                        text(reading, "change"),
                    ]),
                });
            }
        }
        "chart" => {
            limitations.push("Chart shown as its description and measured values.".into());
            add(&mut out, text(p, "accessible_summary"));
            for series in items(p, "series") {
                add(&mut out, text(series, "name"));
                out.push(table(
                    vec![text(p, "x_label"), text(p, "y_label")],
                    items(series, "points")
                        .iter()
                        .map(|v| vec![scalar(&v["x"]), scalar(&v["y"])])
                        .collect(),
                ));
            }
        }
        "callout" => {
            out.push(Element::Context(text(p, "tone")));
            add(&mut out, text(p, "text"));
        }
        "sources" => {
            for source in items(p, "sources") {
                let mut body = Vec::new();
                link(&mut body, text(source, "title"), text(source, "url"));
                let by = joined([text(source, "source_name"), text(source, "published_at")]);
                if !by.is_empty() {
                    body.push(Element::Context(by));
                }
                add(&mut body, text(source, "finding"));
                out.push(Element::Details {
                    summary: text(source, "title"),
                    body,
                });
            }
        }
        "calendar" => {
            for event in items(p, "events") {
                out.push(Element::Heading(text(event, "title")));
                add(
                    &mut out,
                    joined([
                        text(event, "date"),
                        text(event, "start"),
                        text(event, "end"),
                        text(event, "location"),
                    ]),
                );
                add(&mut out, text(event, "detail"));
            }
        }
        "map" => {
            limitations
                .push("Map shown as named coordinates; no interactive map is attached.".into());
            out.push(Element::Context(format!(
                "Connection: {}",
                text(p, "connect")
            )));
            for place in items(p, "places") {
                out.push(Element::Heading(text(place, "label")));
                add(
                    &mut out,
                    format!(
                        "{}, {}",
                        scalar(&place["latitude"]),
                        scalar(&place["longitude"])
                    ),
                );
                add(&mut out, text(place, "detail"));
            }
        }
        "controls" => {
            limitations
                .push("Controls shown at their recorded values; changes require the app.".into());
            for input in items(p, "inputs") {
                out.push(Element::Field {
                    label: text(input, "label"),
                    value: joined([scalar(&input["value"]), text(input, "unit")]),
                });
            }
            let computed = crate::controls::compute(p, &Default::default())?;
            for (output, (_, value)) in items(p, "outputs").iter().zip(computed) {
                let value = crate::controls::shown(value, output["decimals"].as_u64().unwrap_or(2));
                out.push(Element::Field {
                    label: text(output, "label"),
                    value: joined([value, text(output, "unit")]),
                });
                out.push(Element::Code(format!(
                    "{} = {}",
                    text(output, "name"),
                    text(output, "expression")
                )));
                if !text(output, "vary").is_empty() {
                    out.push(Element::Context(format!(
                        "Series: {} from {} to {} by {}",
                        text(output, "vary"),
                        text(output, "from"),
                        text(output, "to"),
                        text(output, "step")
                    )));
                }
            }
        }
        "link" => {
            link(&mut out, text(p, "title"), text(p, "url"));
            add(&mut out, text(p, "description"));
            add(&mut out, text(p, "site_name"));
            // Remote previews stay inert. The image is an explicit link.
            if !text(p, "image_url").is_empty() {
                link(&mut out, "Preview image".into(), text(p, "image_url"));
            }
        }
        "media" => {
            limitations.push("Media is linked; no remote media is fetched automatically.".into());
            link(
                &mut out,
                joined([text(p, "format"), text(p, "alt")]),
                text(p, "url"),
            );
        }
        "file_preview" => {
            limitations.push("File preview requires the app.".into());
            out.push(Element::Code(text(p, "artifact_path")));
            add(&mut out, text(p, "status"));
        }
        "markup_preview" => {
            limitations.push("HTML shown as source; active preview requires the app.".into());
            add(&mut out, text(p, "status"));
            out.push(Element::Code(text(p, "html")));
        }
        "diff" => {
            for file in items(p, "files") {
                out.push(Element::Heading(text(file, "filename")));
                out.push(Element::Context(format!(
                    "+{} −{}",
                    scalar(&file["additions"]),
                    scalar(&file["deletions"])
                )));
                out.push(Element::Code(text(file, "hunks")));
            }
        }
        "test_report" => {
            add(&mut out, text(p, "suite_name"));
            out.push(Element::Code(text(p, "command")));
            for test in items(p, "tests") {
                out.push(Element::Details {
                    summary: joined([text(test, "name"), text(test, "status")]),
                    body: vec![
                        Element::Text(text(test, "message")),
                        Element::Context(duration(test)),
                    ],
                });
            }
            add(&mut out, execution(p));
        }
        "terminal" => {
            out.push(Element::Code(text(p, "command")));
            out.push(Element::Code(text(p, "output")));
            add(&mut out, execution(p));
        }
        _ => return Err("channel card has no supported form".into()),
    }
    out.retain(|e| !matches!(e,Element::Text(s) | Element::Context(s) if s.is_empty()));
    Ok(Card {
        elements: out,
        limitations,
    })
}
fn duration(p: &Value) -> String {
    p.get("duration_ms")
        .map(|v| format!("{} ms", scalar(v)))
        .unwrap_or_default()
}
fn execution(p: &Value) -> String {
    joined([
        p.get("exit_code")
            .map(|v| format!("Exit code {}", scalar(v)))
            .unwrap_or_default(),
        duration(p),
    ])
}
