//! Form-aware channel presentation. Pure projections, never transport or state.
//!
//! The form owns information structure; each provider compiler owns its wire
//! vocabulary and published limits. No topic, kind alias or payload-key reader
//! decides layout. See docs/design/102-channel-cards.md.

mod forms;
mod inline;
mod native;

use crate::{DeliveryPacket, StructuredOutput};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Element {
    Heading(String),
    Text(String),
    Prose(String),
    Context(String),
    Field {
        label: String,
        value: String,
    },
    Code(String),
    Link {
        text: String,
        url: String,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Element>>,
    },
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Details {
        summary: String,
        body: Vec<Element>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Card {
    pub elements: Vec<Element>,
    /// Interactive/visual features replaced by an explicit static projection.
    pub limitations: Vec<String>,
}

/// Compile only current typed forms. Unknown forms are errors, not guessed
/// schemas. Plugin kinds must declare a supported block through their pack.
fn card(output: &StructuredOutput) -> Result<Card, String> {
    if output.schema_version != crate::PRESENTATION_SCHEMA_VERSION {
        return Err("channel card has an unsupported presentation schema".into());
    }
    crate::validate_form(&output.block, &output.payload).map_err(|e| e.to_string())?;
    forms::project(output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Telegram API method; other providers have one create-message endpoint.
    pub method: &'static str,
    /// Routing and authentication are added by the transport, never here.
    pub body: Value,
}

/// Transport status (queued, unavailable, refused) has no answer packet.
/// Keep it literal and bounded by the basic-message protocol.
pub fn notice(surface: &str, text: &str) -> Result<Vec<Message>, String> {
    native::notice_chunks(surface, text)?
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| text_message(surface, part, false))
        .collect()
}

fn text_message(surface: &str, text: String, formatted: bool) -> Result<Message, String> {
    let (method, mut body) = match surface {
        "telegram" => (
            "sendMessage",
            serde_json::json!({"text":text,"link_preview_options":{"is_disabled":true}}),
        ),
        "slack" => (
            "chat.postMessage",
            serde_json::json!({"text":text,"parse":"none","link_names":false,"unfurl_links":false,"unfurl_media":false}),
        ),
        "discord" => (
            "createMessage",
            serde_json::json!({"content":text,"allowed_mentions":{"parse":[]}}),
        ),
        _ => return Err("channel has no text message compiler".into()),
    };
    if surface == "telegram" && formatted {
        body["parse_mode"] = serde_json::json!("HTML");
    }
    if surface == "slack" && !formatted {
        body["mrkdwn"] = serde_json::json!(false);
    }
    Ok(Message { method, body })
}

/// One request plan, used by both direct replies and effect-backed pushes.
/// Cards are emitted before their trailing narration, as the gateway's
/// card-followup contract specifies. No card is also sent as fallback text.
pub fn messages(packet: &DeliveryPacket) -> Result<Vec<Message>, String> {
    let mut messages = Vec::new();
    for output in packet.structured_cards() {
        messages.extend(native::compile(&packet.surface, &card(output)?)?);
    }
    for chunk in packet
        .channel_text_chunks()
        .into_iter()
        .filter(|s| !s.is_empty())
    {
        messages.push(text_message(&packet.surface, chunk, true)?);
    }
    if let Some(message) = messages.last_mut() {
        if packet.surface == "telegram" && !packet.actions.is_empty() {
            let buttons: Vec<_> = packet.actions.iter().map(|action| {
                let request = action.data.get("request_id").map(String::as_str).unwrap_or_default();
                serde_json::json!({"text": action.label, "callback_data": format!("{}:{request}", action.verb)})
            }).collect();
            message.body["reply_markup"] = serde_json::json!({"inline_keyboard": [buttons]});
        } else if let Some(request) = packet.actions.iter().find_map(|a| a.data.get("request_id")) {
            let prompt = format!("Reply `yes {request}` to approve or `no {request}` to deny.");
            // A separate final message keeps controls out of card overflow and
            // preserves provider limits for a full final content chunk.
            messages.push(Message {
                method: if packet.surface == "slack" {
                    "chat.postMessage"
                } else {
                    "createMessage"
                },
                body: if packet.surface == "slack" {
                    serde_json::json!({"text":prompt})
                } else {
                    serde_json::json!({"content":prompt,"allowed_mentions":{"parse":[]}})
                },
            });
        }
    }
    Ok(messages)
}

fn plain(elements: &[Element]) -> String {
    elements
        .iter()
        .map(|element| match element {
            Element::Heading(s) | Element::Text(s) | Element::Context(s) | Element::Code(s) => {
                s.clone()
            }
            Element::Prose(s) => inline::runs(s)
                .into_iter()
                .map(|r| match r.url {
                    Some(url) => format!("{} ({url})", r.text),
                    None => r.text,
                })
                .collect::<Vec<_>>()
                .join(""),
            Element::Field { label, value } => format!("{label}: {value}"),
            Element::Link { text, url } => format!("{text} ({url})"),
            Element::List { ordered, items } => items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    format!(
                        "{} {}",
                        if *ordered {
                            format!("{}.", i + 1)
                        } else {
                            "•".into()
                        },
                        plain(item)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Element::Table { header, rows } => std::iter::once(header.join(" | "))
                .chain(rows.iter().map(|r| r.join(" | ")))
                .collect::<Vec<_>>()
                .join("\n"),
            Element::Details { summary, body } => format!("{summary}\n{}", plain(body)),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests;
