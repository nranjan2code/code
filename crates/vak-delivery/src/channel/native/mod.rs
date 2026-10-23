//! Provider compilers and structural continuation; limits come from the
//! published API contracts cited in docs/design/102-channel-cards.md.
mod discord;
mod slack;
mod telegram;

use super::{Card, Element, Message, inline, plain};

pub(super) fn compile(surface: &str, card: &Card) -> Result<Vec<Message>, String> {
    let mut elements = card.elements.clone();
    elements.extend(card.limitations.iter().cloned().map(Element::Context));
    match surface {
        "telegram" => Ok(telegram::compile(&elements)),
        "slack" => Ok(slack::compile(&elements)),
        "discord" => Ok(discord::compile(&elements)),
        _ => Err("channel has no native card compiler".into()),
    }
}

/// Slack's categorical chart contract, qualified against the published block
/// reference on 2026-10-23. Unsupported dimensions retain the full form fallback.
pub(super) fn slack_chart(output: &crate::StructuredOutput) -> Option<Vec<Message>> {
    use serde_json::{Value, json};
    if output.block != "chart" {
        return None;
    }
    let p = &output.payload;
    let kind = p["chart_type"].as_str()?;
    if !matches!(kind, "line" | "bar" | "area")
        || p["x_scale"] != "category"
        || p.get("follows").is_some()
    {
        return None;
    }
    let title = p["title"].as_str().unwrap_or("Chart");
    if title.is_empty() || units(title) > 50 {
        return None;
    }
    let series = p["series"].as_array()?;
    if series.is_empty() || series.len() > 12 {
        return None;
    }
    let mut names = std::collections::HashSet::new();
    let mut categories: Vec<String> = Vec::new();
    let mut compiled = Vec::new();
    for (index, series) in series.iter().enumerate() {
        let name = series["name"].as_str()?;
        if name.is_empty() || units(name) > 20 || !names.insert(name) {
            return None;
        }
        let points = series["points"].as_array()?;
        if points.is_empty() || points.len() > 20 {
            return None;
        }
        let mut labels = Vec::new();
        let mut data = Vec::new();
        for point in points {
            let label = point["x"].as_str()?;
            if label.is_empty() || units(label) > 20 || labels.iter().any(|s| s == label) {
                return None;
            }
            // Never parse lexical decimals or fill missing measurements.
            if !point["y"].is_number() {
                return None;
            }
            labels.push(label.to_string());
            data.push(json!({"label":label,"value":point["y"]}));
        }
        if index == 0 {
            categories = labels;
        } else if categories != labels {
            return None;
        }
        compiled.push(json!({"name":name,"data":data}));
    }
    let mut axis = json!({"categories":categories});
    for key in ["x_label", "y_label"] {
        if let Some(label) = p[key].as_str() {
            if units(label) > 50 {
                return None;
            }
            axis[key] = Value::String(label.into());
        }
    }
    let block = json!({"type":"data_visualization","title":title,
        "chart":{"type":kind,"series":compiled,"axis_config":axis}});
    let fallback = crate::structured_markdown(output);
    if units(&fallback) > 40_000 {
        return None;
    }
    let mut messages = Vec::new();
    // The authored summary is visible as well as available to screen readers.
    if let Some(summary) = p["accessible_summary"].as_str() {
        messages.extend(super::notice("slack", summary).ok()?);
    }
    messages.push(Message {
        method: "chat.postMessage",
        body: json!({
            "blocks":[block],"text":fallback,"mrkdwn":false,"parse":"none",
            "unfurl_links":false,"unfurl_media":false,"link_names":false
        }),
    });
    Some(messages)
}

pub(super) fn notice_chunks(surface: &str, text: &str) -> Result<Vec<String>, String> {
    let limit = match surface {
        "telegram" => 4096,
        "slack" => 40_000,
        "discord" => 2000,
        _ => return Err("channel has no text message compiler".into()),
    };
    Ok(split_units(text, limit))
}

#[cfg(test)]
pub(super) fn tg_cost(value: &serde_json::Value) -> (usize, usize) {
    telegram::cost(value)
}

/// Count conservatively in UTF-16 code units. This also bounds Unicode
/// scalar counts on providers whose documentation uses "characters".
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}
fn split_units(text: &str, limit: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (at, ch) in text.char_indices() {
        if used + ch.len_utf16() > limit {
            out.push(text[start..at].into());
            start = at;
            used = 0;
        }
        used += ch.len_utf16();
    }
    out.push(text[start..].into());
    out
}

// Column panels follow a provider's 20-column limit. They are mechanical
// partitions, never a judgment of which columns or records matter.
fn table_panels(
    header: &[String],
    rows: &[Vec<String>],
    columns: usize,
) -> Vec<(Vec<String>, Vec<Vec<String>>)> {
    if header.is_empty() {
        return vec![(Vec::new(), rows.to_vec())];
    }
    header
        .chunks(columns)
        .enumerate()
        .map(|(i, h)| {
            let start = i * columns;
            (
                h.to_vec(),
                rows.iter()
                    .map(|r| r.iter().skip(start).take(h.len()).cloned().collect())
                    .collect(),
            )
        })
        .collect()
}
fn table_record(header: &[String], row: &[String]) -> Vec<Element> {
    header
        .iter()
        .zip(row)
        .map(|(label, value)| Element::Field {
            label: label.clone(),
            value: value.clone(),
        })
        .collect()
}
