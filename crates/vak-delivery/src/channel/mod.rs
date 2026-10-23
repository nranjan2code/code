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

/// Select the target before lowering rich semantics to a static form.
fn compile_form(surface: &str, output: &StructuredOutput) -> Result<Vec<Message>, String> {
    if output.schema_version != crate::PRESENTATION_SCHEMA_VERSION {
        return Err("channel card has an unsupported presentation schema".into());
    }
    crate::validate_form(&output.block, &output.payload).map_err(|e| e.to_string())?;
    if surface == "slack"
        && let Some(messages) = native::slack_chart(output)
    {
        return Ok(messages);
    }
    native::compile(surface, &card(output)?)
}

/// Report what this target actually preserves, addressed by accepted item id.
pub(crate) fn disposition(surface: &str, item: &crate::OutputItem) -> crate::CoverageDisposition {
    use crate::{CoverageDisposition, OutputContent};
    let reason = match &item.content {
        OutputContent::Structured { output } => {
            if surface == "slack" && native::slack_chart(output).is_some() {
                return CoverageDisposition::Rendered;
            }
            match card(output) {
                Ok(card) if card.limitations.is_empty() => return CoverageDisposition::Rendered,
                Ok(card) => card.limitations.join(" "),
                Err(_) => "The rich form could not be compiled for this target.".into(),
            }
        }
        OutputContent::Adaptive { .. } => "Interactive presentation is preserved as readable text; use the conversation for its controls.".into(),
        OutputContent::Artifact { .. } => "File identity and review state are preserved as readable text; use the authorized conversation for file operations.".into(),
        _ => return CoverageDisposition::Rendered,
    };
    CoverageDisposition::PreservedInFallback { reason }
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

/// Compile accepted items in source order. Provider limits change continuation,
/// never which prose, card or result verdict belongs to the answer.
pub fn messages(packet: &DeliveryPacket) -> Result<Vec<Message>, String> {
    let mut messages = Vec::new();
    if let Some(timeline) = &packet.presentation {
        let mut notices = std::collections::HashSet::new();
        for item in timeline
            .items
            .iter()
            .filter(|item| crate::answer::is_material(item))
        {
            if let Some(notice) = crate::answer::notice(item)
                && notices.insert(crate::answer::notice_key(item, notice))
            {
                messages.extend(self::notice(&packet.surface, notice)?);
            }
            match &item.content {
                crate::OutputContent::Structured { output } => {
                    messages.extend(compile_form(&packet.surface, output)?)
                }
                _ => messages.extend(prose(&packet.surface, &crate::answer::text(item))?),
            }
        }
    } else {
        for chunk in packet
            .channel_text_chunks()
            .into_iter()
            .filter(|s| !s.is_empty())
        {
            messages.push(text_message(&packet.surface, chunk, true)?);
        }
    }
    for note in &packet.delivery_notices {
        messages.extend(notice(&packet.surface, note)?);
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

fn prose(surface: &str, markdown: &str) -> Result<Vec<Message>, String> {
    if markdown.trim().is_empty() {
        return Ok(Vec::new());
    }
    let profile = match surface {
        "telegram" => crate::Markup::TelegramHtml,
        "slack" => crate::Markup::SlackMrkdwn,
        "discord" => crate::Markup::DiscordMarkdown,
        _ => return Err("channel has no prose compiler".into()),
    };
    let rendered = crate::render_text(markdown, profile);
    let chunks = if surface == "telegram" {
        crate::telegram::split_html_chunks(&rendered, Some(4096))
    } else {
        crate::chunk_markdown_preserving_fences(
            &rendered,
            Some(if surface == "slack" { 40_000 } else { 2000 }),
        )
    };
    chunks
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| text_message(surface, part, true))
        .collect()
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
