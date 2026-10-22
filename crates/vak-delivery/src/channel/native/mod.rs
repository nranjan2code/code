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
