//! Audit assertions for the desired behavior; failures reproduce current defects.
//! Compile against the current vak-delivery library using run-probes.py.
use vak_delivery::*;

fn packet(text: &str, surface: &str, markup: Markup, cap: usize) -> DeliveryPacket {
    render(&DeliveryJob {
        job_id: "audit".into(), target: format!("{surface}:audit"),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Answer(AnswerDraft::from_markdown(text)),
        profile: DeliveryProfile {
            surface: surface.into(), markup, max_chars: Some(cap),
            supports_tables: false, supports_code_blocks: true, supports_links: true,
            supports_actions: false, template: None, posture: DeliveryPosture::default(),
        }, skill_registry: None, trace: None, actor: None,
    }).unwrap()
}

#[test]
fn discord_heading_is_native_markdown() {
    assert_eq!(discord::markdown_to_discord("# Title"), "# Title");
}

#[test]
fn discord_masked_link_is_preserved() {
    assert_eq!(discord::markdown_to_discord("[site](https://example.com)"), "[site](https://example.com)");
}

#[test]
fn discord_inline_code_is_not_rewritten() {
    assert_eq!(discord::markdown_to_discord("`[x](https://example.com)`"), "`[x](https://example.com)`");
}

#[test]
fn slack_second_level_heading_is_formatted() {
    assert_eq!(slack::markdown_to_mrkdwn("## Summary"), "*Summary*");
}

#[test]
fn telegram_heading_preserves_url_and_valid_tag_case() {
    let html = telegram::markdown_to_html("# [Docs](https://example.com/CaseSensitive)");
    assert!(html.contains("<a href=\"https://example.com/CaseSensitive\">"), "{html}");
}

#[test]
fn telegram_inline_code_escapes_once() {
    assert_eq!(telegram::markdown_to_html("`a < b & c`"), "<code>a &lt; b &amp; c</code>");
}

#[test]
fn telegram_chunks_preserve_html_entities() {
    let chunks = telegram::split_html_chunks(&format!("{}&amp;z", "a".repeat(31)), Some(32));
    assert!(!chunks.iter().any(|s| s.ends_with('&') || s.starts_with("amp;")), "{chunks:?}");
}

#[test]
fn telegram_plain_fallback_decodes_entities() {
    assert_eq!(telegram::strip_html("<b>A &amp; B</b>"), "A & B");
}

#[test]
fn discord_long_line_obeys_cap() {
    let p = packet(&"x".repeat(5000), "discord", Markup::DiscordMarkdown, 1900);
    let lengths: Vec<_> = p.chunks.iter().map(|s| s.chars().count()).collect();
    assert!(lengths.iter().all(|n| *n <= 1900), "{lengths:?}");
}

#[test]
fn slack_long_code_line_obeys_cap() {
    let p = packet(&format!("```json\n{}\n```", "x".repeat(5000)), "slack", Markup::SlackMrkdwn, 3900);
    let lengths: Vec<_> = p.chunks.iter().map(|s| s.chars().count()).collect();
    assert!(lengths.iter().all(|n| *n <= 3900), "{lengths:?}");
}

#[test]
fn data_grid_readable_fallback_retains_rows() {
    let cards = structured_outputs_from_text(r#"{"semantic_type":"data.grid","payload":{"columns":[{"key":"region","label":"Region"}],"rows":[{"region":"APAC"}]}}"#);
    assert_eq!(cards.len(), 1, "fixture must be a validated card");
    let markdown = structured_markdown(&cards[0]);
    let readable = markdown.split_once("\n\n```json").map_or(markdown.as_str(), |(body, _)| body);
    assert!(readable.contains("APAC"), "readable={readable:?}");
}

#[test]
fn slack_plan_blocks_retain_nested_items() {
    let cards = structured_outputs_from_text(r#"{"semantic_type":"plan.timeline","payload":{"title":"Trip","summary":"Day plan","items":[{"label":"Museum","detail":"Arrive at noon"}]}}"#);
    assert_eq!(cards.len(), 1, "fixture must be a validated card");
    let blocks = slack::structured_card_blocks(&cards);
    assert!(format!("{blocks:?}").contains("Museum"), "{blocks:?}");
}

#[test]
fn ordinary_bold_remains_formatted() {
    assert_eq!(telegram::markdown_to_html("**hello**"), "<b>hello</b>");
    assert_eq!(slack::markdown_to_mrkdwn("**hello**"), "*hello*");
    assert_eq!(discord::markdown_to_discord("**hello**"), "**hello**");
}
