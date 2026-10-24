#![allow(clippy::unwrap_used)]

use super::*;
use serde_json::json;

fn output(form: &str, payload: Value) -> StructuredOutput {
    crate::structured_output(form, None, payload, &crate::built_in_skill_registry()).unwrap()
}
fn packet(surface: &str, cards: Vec<StructuredOutput>, followup: &str) -> DeliveryPacket {
    let mut packet = crate::render(&crate::DeliveryJob {
        job_id: "channel-test".into(),
        target: format!("{surface}:test"),
        kind: crate::DeliveryKind::Assistant,
        content: crate::DeliveryContent::Answer(crate::AnswerDraft::from_markdown(followup)),
        profile: crate::DeliveryProfile {
            surface: surface.into(),
            markup: match surface {
                "telegram" => crate::Markup::TelegramHtml,
                "slack" => crate::Markup::SlackMrkdwn,
                _ => crate::Markup::DiscordMarkdown,
            },
            max_chars: Some(2000),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
        },
        trace: None,
        actor: None,
    })
    .unwrap();
    let items = cards.into_iter().enumerate().map(|(i, output)| serde_json::from_value(json!({
        "id": format!("test-card-{i}"), "timestamp":"", "turn_id":"test-turn", "role":"assistant", "kind":"card", "status":"succeeded",
        "content":{"type":"structured","output":output}, "fallback_text":"", "actions":[]
    })).unwrap());
    packet
        .presentation
        .as_mut()
        .unwrap()
        .items
        .splice(0..0, items);
    packet
}
fn strings(value: &Value, values: &mut Vec<String>) {
    match value {
        Value::String(s) => values.push(s.clone()),
        Value::Array(a) => {
            for v in a {
                strings(v, values);
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                strings(v, values);
            }
        }
        _ => {}
    }
}
fn text(messages: &[Message]) -> String {
    let mut out = Vec::new();
    for message in messages {
        strings(&message.body, &mut out);
    }
    out.join("\n")
}

#[test]
fn every_current_form_has_a_native_projection_on_every_channel() {
    for form in crate::FORMS {
        let card = output(form, crate::skills::tests::sample(form));
        for surface in ["telegram", "slack", "discord"] {
            let requests = messages(&packet(surface, vec![card.clone()], "")).unwrap();
            assert!(!requests.is_empty(), "{surface}/{form}");
            let body = text(&requests);
            assert!(!body.contains("semantic_type"));
            assert!(!body.contains("schema_version"));
        }
    }
}

#[test]
fn summary_and_nested_sources_are_not_generic_payload_fields() {
    let summary = output(
        "summary",
        json!({"headline":"A measured result","label":"Observed at 10:02","points":["First point","Second point"]}),
    );
    let sources = output(
        "sources",
        json!({"title":"Evidence","label":"Retrieved","sources":[{"title":"Primary record","url":"https://example.com/source","finding":"The original finding.","published_at":"2026-10-22"}]}),
    );
    for surface in ["telegram", "slack", "discord"] {
        let requests = messages(&packet(
            surface,
            vec![summary.clone(), sources.clone()],
            "Following explanation",
        ))
        .unwrap();
        let body = text(&requests);
        for expected in [
            "A measured result",
            "Observed at 10:02",
            "First point",
            "Second point",
            "Primary record",
            "https://example.com/source",
            "The original finding.",
            "2026-10-22",
        ] {
            assert!(body.contains(expected), "{surface} lost {expected}");
        }
        assert!(!body.contains("headline\n"));
        assert!(!body.contains("[{\""));
        assert!(text(&requests[..requests.len() - 1]).contains("The original finding."));
        assert!(text(&requests[requests.len() - 1..]).contains("Following explanation"));
    }
}

#[test]
fn telegram_uses_rich_blocks_with_source_disclosure_and_no_html_retry() {
    let sources = output(
        "sources",
        json!({"sources":[{"title":"Record","url":"https://example.com","finding":"Supporting detail"}]}),
    );
    let requests = messages(&packet("telegram", vec![sources], "")).unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "sendRichMessage");
    assert_eq!(
        requests[0].body["rich_message"]["blocks"][0]["type"],
        "details"
    );
    assert_eq!(
        requests[0].body["rich_message"]["skip_entity_detection"],
        true
    );
    assert!(requests[0].body.get("text").is_none());
}

#[test]
fn kinds_cannot_change_layout_or_infer_a_domain() {
    let mut card = output(
        "summary",
        json!({"headline":"Some subject","points":["A point"]}),
    );
    let before = super::card(&card).unwrap();
    card.semantic_type = "owner.plugin.any_subject".into();
    assert_eq!(before, super::card(&card).unwrap());
    card.block = "unknown_form".into();
    assert!(super::card(&card).is_err());
}

#[test]
fn no_card_limit_discards_the_eleventh_or_later_card() {
    for surface in ["telegram", "slack", "discord"] {
        let cards = (0..53)
            .map(|i| output("summary", json!({"headline":format!("Result {i:03}")})))
            .collect();
        let requests = messages(&packet(surface, cards, "")).unwrap();
        let body = text(&requests);
        for i in 0..53 {
            assert!(body.contains(&format!("Result {i:03}")));
        }
    }
}

#[test]
fn overflow_keeps_unicode_and_tail_on_every_surface() {
    let content = format!("{}FINALENDMARKER", "🦜नमस्ते & <tag> ".repeat(16000));
    for surface in ["telegram", "slack", "discord"] {
        let requests = messages(&packet(
            surface,
            vec![output("summary", json!({"headline":"Long","text":content}))],
            "",
        ))
        .unwrap();
        assert!(requests.len() > 1);
        assert!(text(&requests).contains("FINALENDMARKER"));
        for request in &requests {
            if surface == "telegram" {
                let (chars, blocks) =
                    super::native::tg_cost(&request.body["rich_message"]["blocks"]);
                assert!(chars <= 32768);
                assert!(blocks <= 500);
            } else if surface == "discord" {
                for embed in request.body["embeds"].as_array().unwrap() {
                    assert!(
                        embed["description"]
                            .as_str()
                            .unwrap_or_default()
                            .encode_utf16()
                            .count()
                            <= 4096
                    );
                    assert!(
                        embed["title"]
                            .as_str()
                            .unwrap_or_default()
                            .encode_utf16()
                            .count()
                            <= 256
                    );
                }
            } else {
                assert!(request.body["blocks"].as_array().unwrap().len() <= 50);
            }
        }
    }
}

#[test]
fn tables_keep_lexical_scalars_and_paginate_rows_and_columns() {
    let columns = (0..23)
        .map(|i| json!({"label":format!("Column {i}"),"numeric":false}))
        .collect::<Vec<_>>();
    let rows = (0..120)
        .map(|i| {
            (0..23)
                .map(|j| json!(format!("{i:03}-{j:03}-9007199254740993")))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for surface in ["telegram", "slack", "discord"] {
        let requests = messages(&packet(
            surface,
            vec![output("table", json!({"columns":columns,"rows":rows}))],
            "",
        ))
        .unwrap();
        let body = text(&requests);
        for expected in [
            "000-000-9007199254740993",
            "119-022-9007199254740993",
            "Column 22",
        ] {
            assert!(body.contains(expected), "{surface} lost {expected}");
        }
    }
}

#[test]
fn oversized_table_cell_survives_as_a_labelled_record() {
    let content = format!("{}ENDCELL", "x".repeat(40000));
    for surface in ["telegram", "slack", "discord"] {
        let requests = messages(&packet(
            surface,
            vec![output(
                "table",
                json!({"columns":[{"label":"Value","numeric":false},{"label":"Other","numeric":false}],"rows":[[content,"second"]]}),
            )],
            "",
        ))
        .unwrap();
        assert!(text(&requests).contains("ENDCELL"));
    }
}

#[test]
fn remote_media_is_inert_and_mentions_cannot_become_instructions() {
    for surface in ["telegram", "slack", "discord"] {
        let card = output(
            "media",
            json!({"format":"image","url":"https://example.com/image.png","alt":"@everyone <@U123> <script>"}),
        );
        let requests = messages(&packet(surface, vec![card], "")).unwrap();
        assert!(text(&requests).contains("https://example.com/image.png"));
        assert!(!requests.iter().any(|r| r.body.get("image_url").is_some()));
        if surface == "discord" {
            assert_eq!(requests[0].body["allowed_mentions"]["parse"], json!([]));
        }
    }
}

#[test]
fn empty_reply_never_posts_a_blank_message() {
    for surface in ["telegram", "slack", "discord"] {
        assert!(messages(&packet(surface, vec![], "")).unwrap().is_empty());
    }
}

#[test]
fn approval_controls_land_after_the_last_card_continuation() {
    for surface in ["telegram", "slack", "discord"] {
        let mut packet = packet(
            surface,
            vec![output("summary", json!({"headline":"Approve this"}))],
            "",
        );
        packet.actions.push(crate::DeliveryAction {
            id: "approve".into(),
            verb: "approve".into(),
            label: "Approve".into(),
            data: std::collections::BTreeMap::from([("request_id".into(), "abc123".into())]),
        });
        let requests = messages(&packet).unwrap();
        let last = requests.last().unwrap();
        if surface == "telegram" {
            assert_eq!(
                last.body["reply_markup"]["inline_keyboard"][0][0]["callback_data"],
                "approve:abc123"
            );
        } else {
            assert!(text(&requests[requests.len() - 1..]).contains("yes abc123"));
        }
    }
}

#[test]
fn native_source_links_stay_complete_at_discord_boundaries() {
    let title = "Long source 🦜 * [ ] title ".repeat(1000);
    let card = output(
        "sources",
        json!({"sources":[{"title":title,"url":"https://example.com/path?q=one","finding":"Tail survives"}]}),
    );
    let requests = messages(&packet("discord", vec![card], "")).unwrap();
    assert!(text(&requests).contains("Tail survives"));
    for request in requests {
        for embed in request.body["embeds"].as_array().unwrap() {
            let description = embed["description"].as_str().unwrap_or_default();
            assert!(description.encode_utf16().count() <= 4096);
            if description.contains("](<") {
                assert!(description.contains("https://example.com/path?q=one>)"));
            }
        }
    }
}

#[test]
fn slack_accessible_fallback_is_complete_for_each_message() {
    let card = output(
        "sources",
        json!({"sources":[{"title":"Source title","url":"https://example.com/source","finding":"Complete finding"}]}),
    );
    let requests = messages(&packet("slack", vec![card], "")).unwrap();
    assert!(
        requests[0].body["text"]
            .as_str()
            .unwrap()
            .contains("Complete finding")
    );
    assert!(
        requests[0].body["text"]
            .as_str()
            .unwrap()
            .contains("https://example.com/source")
    );
    assert_eq!(requests[0].body["mrkdwn"], false);
}

#[test]
fn telegram_facts_are_visible_and_equal_shell_labels_are_not_repeated() {
    let card = output(
        "stats",
        json!({"title":"Readings","label":"Readings","readings":[{"label":"A value","value":"00123"}]}),
    );
    let requests = messages(&packet("telegram", vec![card], "")).unwrap();
    let blocks = requests[0].body["rich_message"]["blocks"]
        .as_array()
        .unwrap();
    assert!(blocks.iter().all(|b| b["type"] != "details"));
    assert_eq!(blocks.len(), 2);
    assert!(text(&requests).contains("00123"));
}

#[test]
fn terminal_output_uses_bounded_code_blocks_without_closing_on_payload_fences() {
    let source = format!("literal ``` in output\n{}ENDCODE", "abc\n".repeat(4000));
    let card = output(
        "terminal",
        json!({"command":"print-output","output":source,"exit_code":0}),
    );
    let requests = messages(&packet("discord", vec![card], "")).unwrap();
    assert!(text(&requests).contains("ENDCODE"));
    for request in requests {
        for embed in request.body["embeds"].as_array().unwrap() {
            assert!(
                embed["description"]
                    .as_str()
                    .unwrap_or_default()
                    .encode_utf16()
                    .count()
                    <= 4096
            );
        }
    }
}

#[test]
fn authored_prose_has_native_emphasis_and_explicit_inert_media_links() {
    let card = output(
        "summary",
        json!({"headline":"Formatted result","text":"A **strong** point with [evidence](https://example.com). ![Image alt](https://example.com/image.png) <script>inert</script>"}),
    );
    let telegram = messages(&packet("telegram", vec![card.clone()], "")).unwrap();
    let slack = messages(&packet("slack", vec![card.clone()], "")).unwrap();
    let discord = messages(&packet("discord", vec![card], "")).unwrap();
    assert!(telegram[0].body.to_string().contains("\"type\":\"bold\""));
    assert!(slack[0].body.to_string().contains("\"bold\":true"));
    assert!(text(&discord).contains("**strong**"));
    for requests in [telegram, slack, discord] {
        let body = text(&requests);
        assert!(body.contains("https://example.com/image.png"));
        assert!(body.contains("inert"));
        assert!(!requests.iter().any(|r| r.body.get("photo").is_some()));
    }
}

#[test]
fn unsafe_prose_link_retains_its_destination_as_literal_text() {
    let card = output(
        "summary",
        json!({"headline":"A result","text":"[A label](javascript:alert)"}),
    );
    for surface in ["telegram", "slack", "discord"] {
        let requests = messages(&packet(surface, vec![card.clone()], "")).unwrap();
        assert!(text(&requests).contains("javascript:alert"));
        assert!(
            !requests
                .iter()
                .any(|r| r.body.to_string().contains("\"url\":\"javascript:"))
        );
    }
}
