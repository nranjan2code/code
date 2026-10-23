//! Ordered answer views over already-authorized presentation items.
//!
//! Audience filtering belongs to the caller. These functions never read a
//! store, grant access, infer a form, or reconstruct a card from tool output.

use crate::{OutputContent, OutputItem, OutputRole, OutputStatus, OutputTimeline};
use std::collections::HashSet;

/// Whether an item is answer material rather than a request or runtime activity.
pub fn is_material(item: &OutputItem) -> bool {
    item.role != OutputRole::User
        && match &item.content {
            OutputContent::Document { .. } => item.role == OutputRole::Assistant,
            OutputContent::Outcome { document, .. } => document.is_some(),
            OutputContent::Structured { .. }
            | OutputContent::Adaptive { .. }
            | OutputContent::Artifact { .. } => true,
            _ => false,
        }
}

/// Select one turn without changing item identities, result boundaries or order.
pub fn of_turn<'a>(timeline: &'a OutputTimeline, turn: &str) -> Vec<&'a OutputItem> {
    timeline
        .items
        .iter()
        .filter(|item| item.turn_id == turn && is_material(item))
        .collect()
}

pub fn latest(timeline: &OutputTimeline) -> Vec<&OutputItem> {
    let turn = timeline
        .items
        .iter()
        .rev()
        .find(|item| item.role == OutputRole::User)
        .or_else(|| timeline.items.iter().rev().find(|item| is_material(item)));
    turn.map(|item| of_turn(timeline, &item.turn_id))
        .unwrap_or_default()
}

/// Exact authored prose or the form's readable projection, never its audit appendix.
pub fn text(item: &OutputItem) -> String {
    match &item.content {
        OutputContent::Document { document }
        | OutputContent::Outcome {
            document: Some(document),
            ..
        } => crate::project_structured_fences(&document.source_markdown),
        OutputContent::Structured { output } => crate::structured_markdown(output),
        OutputContent::Adaptive { fallback_text, .. } => {
            crate::project_structured_fences(fallback_text)
        }
        OutputContent::Artifact { artifact } => {
            let mut text = artifact.name.clone();
            if let Some(description) = &artifact.description {
                text.push_str(&format!("\n{description}"));
            }
            if matches!(artifact.status, Some(crate::ArtifactStatus::Draft { .. })) {
                text.push_str("\nDraft — review required.");
            }
            text
        }
        _ => String::new(),
    }
}

/// A truthful notice stays attached even when the answer consists only of cards.
pub fn notice(item: &OutputItem) -> Option<&'static str> {
    let status = item
        .outcome
        .as_ref()
        .map(|outcome| outcome.status)
        .unwrap_or(item.status);
    match status {
        OutputStatus::Failed => {
            return Some("This result failed; any content shown is incomplete.");
        }
        OutputStatus::Denied => return Some("This result needs permission that was not granted."),
        OutputStatus::Cancelled => return Some("Stopped. Partial output is preserved."),
        OutputStatus::Running | OutputStatus::Pending => return Some("Work is still in progress."),
        _ => {}
    }
    if let Some(outcome) = &item.outcome {
        if outcome
            .human_review
            .as_deref()
            .is_some_and(|review| !review.is_empty() && review != "not_required")
        {
            return Some("Review is required before relying on this result.");
        }
        if outcome
            .evidence_state
            .as_deref()
            .is_some_and(|state| !matches!(state, "fresh" | "none"))
        {
            return Some(
                "The evidence is stale or unverified; review it before relying on this result.",
            );
        }
        if outcome
            .completion
            .as_deref()
            .is_some_and(|completion| completion != "complete")
        {
            return Some("This result is incomplete; review its evidence before relying on it.");
        }
    }
    (status == OutputStatus::Partial)
        .then_some("This result is incomplete; review its evidence before relying on it.")
}

/// One notice per result and kind of notice; never hide a later revised verdict.
pub fn notice_key(item: &OutputItem, notice: &str) -> (String, String) {
    (
        item.outcome
            .as_ref()
            .map(|outcome| outcome.result_id.clone())
            .unwrap_or_else(|| item.id.clone()),
        notice.into(),
    )
}

pub fn markdown<'a>(items: impl IntoIterator<Item = &'a OutputItem>) -> String {
    let mut seen = HashSet::new();
    let mut parts = Vec::new();
    for item in items.into_iter().filter(|item| is_material(item)) {
        if let Some(notice) = notice(item)
            && seen.insert(notice_key(item, notice))
        {
            parts.push(notice.to_string());
        }
        let text = text(item);
        if !text.trim().is_empty() {
            parts.push(text);
        }
    }
    parts.join("\n\n")
}

/// Speech keeps verdicts and explains visual content rather than silently dropping it.
pub fn spoken<'a>(items: impl IntoIterator<Item = &'a OutputItem>) -> String {
    let items: Vec<_> = items.into_iter().filter(|item| is_material(item)).collect();
    let mut seen = HashSet::new();
    let mut parts = Vec::new();
    let mut visual = false;
    for item in items {
        if let Some(notice) = notice(item)
            && seen.insert(notice_key(item, notice))
        {
            parts.push(notice.into());
        }
        match item.content {
            OutputContent::Document { .. } | OutputContent::Outcome { .. } => {
                parts.push(text(item))
            }
            _ => visual = true,
        }
    }
    if visual {
        parts.push("The conversation includes rich results. Open the answer to review the data and use its controls.".into());
    }
    parts
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Human transcript export over the same authorized answer projection.
pub fn transcript_markdown(timeline: &OutputTimeline) -> String {
    let mut out = String::from("# Vakyartha transcript\n\n");
    let mut seen = HashSet::new();
    let mut answer_open = false;
    for item in &timeline.items {
        if item.role == OutputRole::User {
            if let OutputContent::Document { document } = &item.content {
                out.push_str("## User\n\n");
                out.push_str(&document.source_markdown);
                out.push_str("\n\n");
                answer_open = false;
            }
        } else if is_material(item) {
            if !answer_open {
                out.push_str("## Assistant\n\n");
                answer_open = true;
            }
            if let Some(notice) = notice(item)
                && seen.insert(notice_key(item, notice))
            {
                out.push_str(notice);
                out.push_str("\n\n");
            }
            out.push_str(&text(item));
            out.push_str("\n\n");
        }
    }
    out
}

/// Inert, standalone HTML export. Raw HTML is displayed as text; links are
/// scheme-checked and media remains a link rather than making network requests.
pub fn html(title: &str, markdown: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let events = Parser::new_ext(
        markdown,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH,
    )
    .map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        })
        | Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let dest_url = if crate::presentation::safe_link(&dest_url) {
                dest_url
            } else {
                "#".into()
            };
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            })
        }
        Event::End(TagEnd::Image) => Event::End(TagEnd::Link),
        other => other,
    });
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, events);
    let title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;");
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"><title>{title}</title><style>:root {{color-scheme:light dark}}body {{font:16px/1.6 system-ui,sans-serif;max-width:65rem;margin:2rem auto;padding:1rem}}table {{border-collapse:collapse;display:block;overflow:auto}}td,th {{border:1px solid;padding:.5rem;text-align:left}}pre {{overflow:auto;padding:1rem;border:1px solid}}code {{font-size:14px}}blockquote {{border-left:3px solid;padding-left:1rem}}</style></head><body><h1>{title}</h1>{body}</body></html>"#
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;

    fn corpus() -> OutputTimeline {
        serde_json::from_str(include_str!("../tests/fixtures/rich-answer.json")).unwrap()
    }

    #[test]
    fn rich_answer_preserves_source_order_exact_scalars_and_result_state() {
        let timeline = corpus();
        let items = latest(&timeline);
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [
                "leading",
                "chart-entry",
                "between",
                "numbers",
                "sources-entry"
            ]
        );
        let rendered = markdown(items.iter().copied());
        let at = |part| rendered.find(part).unwrap();
        assert!(at("Leading context") < at("Observed series"));
        assert!(at("Observed series") < at("Note: these"));
        assert!(at("Note: these") < at("0007"));
        assert!(at("12345678901234567890.001") < at("Original observations"));
        assert_eq!(rendered.matches("Review is required").count(), 1);
        let draft = crate::AnswerDraft::from_items(items.into_iter().cloned().collect());
        let restored: crate::AnswerDraft =
            serde_json::from_value(serde_json::to_value(&draft).unwrap()).unwrap();
        assert_eq!(restored.items, draft.items);
        assert_eq!(
            restored.items[1]
                .provenance
                .as_ref()
                .unwrap()
                .presentation_id
                .as_deref(),
            Some("chart-entry")
        );
        assert_eq!(
            restored.items[1].outcome.as_ref().unwrap().status,
            OutputStatus::Partial
        );
    }

    #[test]
    fn exports_preserve_the_answer_and_html_is_inert() {
        let timeline = corpus();
        let md = transcript_markdown(&timeline);
        assert_eq!(md.matches("## User\n").count(), 1);
        assert_eq!(md.matches("## Assistant\n").count(), 1);
        let html = html(
            "<Transcript>",
            &format!(
                "{md}\n<script>alert(1)</script>\n[x](javascript:alert)\n![remote](https://example.com/image.png)"
            ),
        );
        assert!(md.contains("Compare the recorded observations"));
        assert!(html.contains("0007"));
        assert!(html.contains("12345678901234567890.001"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("href=\"javascript:"));
        assert!(!html.contains("<img"));
        assert!(html.contains("Content-Security-Policy"));
    }

    #[test]
    fn target_selection_preserves_native_charts_and_fallback_data() {
        let mut timeline = corpus();
        let OutputContent::Structured { output } = &mut timeline.items[2].content else {
            panic!("chart")
        };
        let packet = |output: &crate::StructuredOutput| {
            let mut timeline = corpus();
            timeline.items[2].content = OutputContent::Structured {
                output: output.clone(),
            };
            crate::channel::messages(&crate::DeliveryPacket {
                presentation: Some(timeline),
                ..crate::render(&crate::DeliveryJob {
                    job_id: "chart".into(),
                    target: "slack:fixture".into(),
                    kind: crate::DeliveryKind::Assistant,
                    content: crate::DeliveryContent::Answer(crate::AnswerDraft::from_markdown("")),
                    profile: crate::DeliveryProfile {
                        surface: "slack".into(),
                        markup: crate::Markup::SlackMrkdwn,
                        max_chars: None,
                        supports_tables: true,
                        supports_code_blocks: true,
                        supports_links: true,
                        supports_actions: false,
                        template: None,
                    },
                    trace: None,
                    actor: None,
                    skill_registry: None,
                })
                .unwrap()
            })
            .unwrap()
        };
        let messages = packet(output);
        let chart = messages
            .iter()
            .flat_map(|m| m.body["blocks"].as_array().into_iter().flatten())
            .find(|b| b["type"] == "data_visualization")
            .unwrap();
        assert_eq!(chart["chart"]["series"][0]["data"][3]["value"], 25);
        assert_eq!(chart["chart"]["axis_config"]["categories"][0], "R1");
        output.payload["series"][0]["points"][0]["y"] = serde_json::json!(null);
        let messages = packet(output);
        assert!(
            !messages
                .iter()
                .any(|m| m.body.to_string().contains("data_visualization"))
        );
        assert!(
            messages
                .iter()
                .any(|m| m.body.to_string().contains("measured values"))
        );
    }

    #[test]
    fn speech_and_card_only_answers_keep_the_verdict_and_visual_companion() {
        let timeline = corpus();
        let chart = &timeline.items[2];
        let spoken = spoken([chart]);
        assert!(spoken.contains("Review is required"));
        assert!(spoken.contains("Open the answer"));
        let text = markdown([chart]);
        assert!(text.contains("R4"));
        assert!(text.contains("25"));
        assert!(!text.contains("```json"));
    }

    #[test]
    fn every_actual_channel_consumes_the_same_ordered_answer() {
        for surface in ["telegram", "slack", "discord"] {
            let timeline = corpus();
            let items: Vec<_> = latest(&timeline).into_iter().cloned().collect();
            let expected = items.clone();
            let profile = crate::DeliveryProfile {
                surface: surface.into(),
                markup: crate::Markup::Markdown,
                max_chars: None,
                supports_tables: true,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
            };
            let mut packet = crate::render(&crate::DeliveryJob {
                job_id: "transport-job".into(),
                target: format!("{surface}:fixture"),
                kind: crate::DeliveryKind::Assistant,
                content: crate::DeliveryContent::Answer(crate::AnswerDraft::from_items(items)),
                profile,
                skill_registry: None,
                trace: None,
                actor: None,
            })
            .unwrap();
            assert_eq!(packet.presentation.as_ref().unwrap().items, expected);
            assert_eq!(
                packet
                    .coverage
                    .iter()
                    .map(|part| part.block_id.as_str())
                    .collect::<Vec<_>>(),
                [
                    "leading",
                    "chart-entry",
                    "between",
                    "numbers",
                    "sources-entry"
                ]
            );
            assert!(
                matches!(
                    &packet.coverage[1].disposition,
                    crate::CoverageDisposition::Rendered
                ) == (surface == "slack")
            );

            packet
                .delivery_notices
                .push("The file is held for review on this channel.".into());
            let requests = crate::channel::messages(&packet).unwrap();
            let wire = requests
                .iter()
                .map(|message| message.body.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            let at = |part| wire.find(part).unwrap();
            assert!(at("Leading context") < at("Observed series"), "{surface}");
            assert!(at("Observed series") < at("Note: these"), "{surface}");
            assert!(at("Note: these") < at("0007"), "{surface}");
            assert!(
                at("12345678901234567890.001") < at("Original observations"),
                "{surface}"
            );
            assert_eq!(wire.matches("Review is required").count(), 1, "{surface}");
            assert!(at("Original observations") < at("The file is held for review"));
            assert_eq!(wire.matches("The file is held for review").count(), 1);
        }
    }

    #[test]
    fn discard_and_late_live_updates_cannot_resurrect_a_settled_item() {
        use crate::{OutputStreamEvent as Event, presentation::apply_stream_event};
        let mut timeline = corpus();
        apply_stream_event(
            &mut timeline,
            Event::ItemRemoved {
                item_id: "leading".into(),
            },
        );
        assert!(!markdown(latest(&timeline)).contains("Leading context"));
        let mut late = timeline.items[1].clone();
        late.provenance.as_mut().unwrap().source = Some("live_event".into());
        late.outcome = None;
        apply_stream_event(&mut timeline, Event::ItemReplaced { item: late });
        assert!(timeline.items[1].outcome.is_some());
    }
}
