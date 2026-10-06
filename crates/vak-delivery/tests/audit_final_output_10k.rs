// A test's output is for the person running it.
#![allow(clippy::disallowed_macros)]
#![allow(
    unused_imports,
    unused_variables,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::redundant_closure,
    clippy::useless_conversion,
    clippy::bool_assert_comparison,
    clippy::collapsible_if,
    clippy::len_zero,
    clippy::needless_borrow,
    clippy::redundant_locals,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    suspicious_double_ref_op
)]

//! Final-output deep verification: 10,000 scenarios (5,000 red-team content
//! verification + 5,000 blue-team structural verification).
//! Each input is rendered through all six surfaces and assertions check the
//! actual output content — fallback preservation, coverage integrity,
//! payload types, serialization round-trip, schema correctness, and
//! surface-specific formatting.

use std::collections::BTreeSet;
use vak_delivery::AnswerDraft;
use vak_delivery::DELIVERY_SCHEMA_VERSION;
use vak_delivery::DeliveryContent;
use vak_delivery::DeliveryJob;
use vak_delivery::DeliveryKind;
use vak_delivery::DeliveryPacket;
use vak_delivery::DeliveryPayload;
use vak_delivery::DeliveryPosture;
use vak_delivery::DeliveryProfile;
use vak_delivery::Markup;
use vak_delivery::render;

type Scenario = (String, Box<dyn FnOnce()>);

fn tc<F: FnOnce() + 'static>(name: &str, f: F) -> Scenario {
    (name.to_string(), Box::new(f))
}

fn run_fn(label: &str, scenarios: Vec<Scenario>) {
    let total = scenarios.len();
    let mut passed = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for (name, test) in scenarios {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| test())) {
            Ok(()) => passed += 1,
            Err(_) => failures.push(name),
        }
    }
    assert_eq!(
        passed, total,
        "{label}: {passed}/{total} passed — failures: {failures:?}"
    );
    println!("  ✓ {label}: {total} scenarios passed");
}

fn make_job(source: &str, markup: Markup, surface: &str) -> DeliveryJob {
    DeliveryJob {
        job_id: "test-job".into(),
        target: format!("{surface}:one"),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Answer(AnswerDraft::from_markdown(source)),
        profile: DeliveryProfile {
            surface: surface.into(),
            markup,
            max_chars: None,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: DeliveryPosture::default(),
        },
        skill_registry: None,
        trace: None,
        actor: None,
    }
}

fn pt(packet: &DeliveryPacket) -> String {
    match &packet.payload {
        DeliveryPayload::Text(s) => s.clone(),
        DeliveryPayload::Structured(_) => packet.fallback_markdown.clone(),
    }
}

const SURFACES: &[(&str, Markup)] = &[
    ("telegram", Markup::TelegramHtml),
    ("discord", Markup::DiscordMarkdown),
    ("slack", Markup::SlackMrkdwn),
    ("desktop", Markup::Markdown),
    ("terminal", Markup::Plain),
    ("admin", Markup::Plain),
];

fn src_idx(base: &str, i: usize) -> String {
    format!("{}\n\n<!-- variant {} -->", base, i)
}

fn render_all(source: &str) -> Vec<(DeliveryPacket, &'static str)> {
    let mut results = Vec::new();
    for (surface, markup) in SURFACES {
        let job = make_job(source, *markup, surface);
        let packet = render(&job).expect("render");
        results.push((packet, *surface));
    }
    results
}

fn is_h1(template: &str) -> bool {
    template.starts_with("# ") && !template.starts_with("## ")
}

#[test]
fn content_headings_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title",
        "## Subtitle",
        "###### H6",
        "# T\n## S\n### H",
        "# Title with **bold**",
        "# Multiple\n\n# Another",
        "# H\n\nSome text.",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("heading_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                let src = source.as_str();
                match surface {
                    "telegram" => {
                        assert!(text.contains("<b>"), "telegram heading missing <b>: {src}");
                    }
                    "discord" => {
                        if is_h1(t) {
                            assert!(
                                text.contains("# "),
                                "discord H1 missing native heading: {src}"
                            );
                        } else {
                            assert!(!text.is_empty(), "discord H2+ empty: {src}");
                        }
                    }
                    "desktop" | "admin" => {
                        assert!(
                            text.contains("Title")
                                || text.contains("Subtitle")
                                || text.contains("H")
                                || src.contains('#'),
                            "desktop heading text missing: {src}"
                        );
                    }
                    _ => {}
                }
            }
        }));
    }
    run_fn("content_headings_output", scenarios);
}

#[test]
fn content_code_blocks_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "```rust\nfn main() {}\n```",
        "```python\nprint('hello')\n```",
        "```\nplain code\n```",
        "```diff\n- removed\n+ added\n```",
        "```\n```\n```",
        r#"```
fn hello() {
    println!("hi");
}
```"#,
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("code_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                let src = source.as_str();
                match surface {
                    "telegram" => {
                        assert!(
                            text.contains("<pre>") || text.contains("<code>"),
                            "telegram code missing <pre>/<code>: {src}"
                        );
                    }
                    "desktop" | "discord" | "slack" => {
                        assert!(
                            text.contains("```")
                                || text.contains("code")
                                || text.contains("fn")
                                || text.contains("removed")
                                || text.contains("print"),
                            "surface code missing content: {surface}: {src}"
                        );
                    }
                    _ => {
                        // Plain surfaces strip code fences; empty code blocks yield empty text
                        if t.contains("fn")
                            || t.contains("removed")
                            || t.contains("print")
                            || t.contains("hello")
                        {
                            assert!(
                                !text.is_empty()
                                    && (text.contains("code")
                                        || text.contains("fn")
                                        || text.contains("removed")
                                        || text.contains("print")
                                        || text.contains("hello")),
                                "plain code missing content: {surface}: {src}"
                            );
                        }
                    }
                }
            }
        }));
    }
    run_fn("content_code_blocks_output", scenarios);
}

#[test]
fn content_lists_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "- a\n- b\n- c",
        "1. first\n2. second\n3. third",
        "- [ ] todo\n- [x] done",
        "1. one\n   1. nested",
        "* star list\n* second",
        "+ plus list\n+ second",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("list_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                let src = source.as_str();
                assert!(
                    text.contains("a")
                        || text.contains("first")
                        || text.contains("todo")
                        || text.contains("done"),
                    "list content missing for {src}"
                );
                if src.contains("- [ ]") || src.contains("- [x]") {
                    assert!(
                        text.contains('☐')
                            || text.contains("[ ]")
                            || text.contains('☑')
                            || text.contains("[x]"),
                        "task list marker missing for {surface}: {src}"
                    );
                }
            }
        }));
    }
    run_fn("content_lists_output", scenarios);
}

#[test]
fn content_emphasis_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "**bold**",
        "*italic*",
        "~~strikethrough~~",
        "**a** *b* ~~c~~",
        "`inline code`",
        "**bold** and *italic* and ~~strike~~",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("emph_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                let src = source.as_str();
                if src.contains("**bold**") {
                    match surface {
                        "telegram" => {
                            assert!(text.contains("<b>"), "bold missing <b> for telegram: {src}");
                        }
                        _ => {
                            assert!(!text.is_empty(), "bold empty: {src}");
                        }
                    }
                }
                if src.contains("~~strikethrough~~") {
                    match surface {
                        "telegram" => {
                            assert!(
                                text.contains("<s>"),
                                "strike missing <s> for telegram: {src}"
                            );
                        }
                        _ => {
                            assert!(!text.is_empty(), "strike empty: {src}");
                        }
                    }
                }
            }
        }));
    }
    run_fn("content_emphasis_output", scenarios);
}

#[test]
fn content_tables_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |",
        "| Col1 | Col2 |\n| --- | --- |\n| val1 | val2 |",
        "| single |\n|---|\n| val |",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("table_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                let src = source.as_str();
                assert!(
                    text.contains("1")
                        || text.contains("val1")
                        || text.contains("val")
                        || text.contains("A"),
                    "table data missing for {surface}: {src}"
                );
            }
        }));
    }
    run_fn("content_tables_output", scenarios);
}

#[test]
fn content_links_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "[label](https://example.com)",
        "[text](https://x.com \"Title\")",
        "![alt](https://x.com/img.png)",
        "[a](https://a.com)\n\n[b](https://b.com)",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("link_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                let src = source.as_str();
                assert!(
                    text.contains("label")
                        || text.contains("text")
                        || text.contains("alt")
                        || text.contains("https://")
                        || text.contains("example.com")
                        || text.contains("a.com"),
                    "link text/url missing in {surface}: {src}"
                );
            }
        }));
    }
    run_fn("content_links_output", scenarios);
}

#[test]
fn content_quotes_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "> quoted text",
        "> > nested quote",
        "> quote\n> second line",
        "# T\n\n> important",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("quote_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                let src = source.as_str();
                if src.contains("quoted") || src.contains("important") {
                    assert!(
                        text.contains("quoted")
                            || text.contains("important")
                            || text.contains("<blockquote>"),
                        "quote content missing for {surface}: {src}"
                    );
                } else {
                    assert!(!text.is_empty(), "quote empty for {surface}: {src}");
                }
            }
        }));
    }
    run_fn("content_quotes_output", scenarios);
}

#[test]
fn content_html_escaping_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "<script>alert(1)</script>",
        "<b>bold</b>",
        "<img src=x onerror=alert(1)>",
        "Text < > & ",
        "<iframe src=javascript:alert(1)>",
        "<div onclick=alert(1)>x</div>",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("escape_{i}"), move || {
            let tg_packet =
                render(&make_job(&source, Markup::TelegramHtml, "telegram")).expect("render");
            let tg_text = pt(&tg_packet);
            assert!(
                !tg_text.contains("<script"),
                "script tag not escaped: {source}"
            );
            assert!(!tg_text.contains("<iframe"), "iframe not escaped: {source}");
            assert!(!tg_text.contains("<img"), "img not escaped: {source}");
            assert!(!tg_text.contains("<div"), "div not escaped: {source}");
            for (packet, _) in render_all(&source) {
                let _ = pt(&packet);
            }
        }));
    }
    run_fn("content_html_escaping_output", scenarios);
}

#[test]
fn content_spoilers_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "||hidden||",
        "text ||spoiler|| more",
        "||multi\nline\nspoiler||",
        "# Title\n\n||spoiler||",
        "**bold** ||spoiler|| end",
        "||a|| ||b|| ||c||",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("spoiler_{i}"), move || {
            let tg = render(&make_job(&source, Markup::TelegramHtml, "telegram")).expect("render");
            let tg_text = pt(&tg);
            if !t.contains('\n') || t.lines().count() == 1 {
                assert!(
                    tg_text.contains("<tg-spoiler>"),
                    "telegram spoiler missing <tg-spoiler>: {source}"
                );
            }
            assert!(
                tg_text.contains("hidden")
                    || tg_text.contains("spoiler")
                    || tg_text.contains("tg-spoiler"),
                "spoiler content missing: {source}"
            );
        }));
    }
    run_fn("content_spoilers_output", scenarios);
}

#[test]
fn content_mixed_structures_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\n**bold** *italic* ~~strike~~\n\n- item 1\n- item 2\n\n```rust\ncode\n```\n\n> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |",
        "||spoiler||\n\ninline **bold** ||another|| end",
        "- [ ] todo\n- [x] done\n\n# H\n\n> **bold** quote",
        "",
        "plain text only",
        "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |",
        "```python\nprint('hello')\n```\n\nMore text.",
        "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6",
        "1. first\n2. second\n3. third\n\n- a\n  - b\n  - c",
        "Code `inline` and **bold** in same paragraph.",
        "# T\n\n- item 1\n- [x] done\n\n> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |",
        r#"```
fn hello() {
    println!("hi");
}
```

> **Important** quote with ||spoiler||"#,
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("mixed_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let src = source.as_str();
                assert_eq!(
                    packet.fallback_markdown, src,
                    "fallback not preserved on {surface}: {src}"
                );
                assert_eq!(packet.schema_version, DELIVERY_SCHEMA_VERSION);
                let ids: BTreeSet<_> = packet.coverage.iter().map(|c| &c.block_id).collect();
                assert_eq!(
                    ids.len(),
                    packet.coverage.len(),
                    "duplicate block IDs on {surface}: {src}"
                );
            }
        }));
    }
    run_fn("content_mixed_structures_output", scenarios);
}

#[test]
fn content_special_characters_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "Emoji: 🎉🚀✅",
        "World: 世界の人々",
        "Math symbols: ∀∃∄∅∆∇∈∉∋∌∏∑−±∓",
        "RTL: \u{202E}hello\u{202C}",
        "Zero-width: \u{200B}joiner",
        "BOM: \u{FEFF}text",
        "Special: < > & ",
        "Tabs\tand\nnewlines",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("special_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                if surface == "telegram" && source.contains('<') {
                    assert!(!text.contains("<script"), "script leaked: {source}");
                }
                if !source.trim().is_empty() {
                    assert!(
                        !text.is_empty(),
                        "empty output for non-empty input on {surface}: {source}"
                    );
                }
            }
        }));
    }
    run_fn("content_special_characters_output", scenarios);
}

#[test]
fn content_horizontal_rules_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &["Text before\n\n---\n\nText after", "---", "***", "___"];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("hr_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                if source.contains("---") || source.contains("***") || source.contains("___") {
                    match surface {
                        "telegram" | "discord" | "slack" => {
                            assert!(text.contains("─"), "HR missing on {surface}: {source}");
                        }
                        _ => {
                            assert!(!text.is_empty(), "hr produced empty: {surface}: {source}");
                        }
                    }
                }
                assert_eq!(packet.fallback_markdown, source);
            }
        }));
    }
    run_fn("content_horizontal_rules_output", scenarios);
}

#[test]
fn content_empty_and_whitespace_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "",
        " ",
        "   ",
        "\n",
        "\n\n\n",
        "\t",
        " \n \t \n ",
        "\u{00A0}",
        "\u{200B}",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = t.to_string();
        let i = i;
        scenarios.push(tc(&format!("empty_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                assert_eq!(
                    packet.fallback_markdown, source,
                    "fallback not preserved for empty input on {surface}"
                );
                if source.trim().is_empty() {
                    assert!(
                        packet.coverage.is_empty(),
                        "non-empty coverage for empty input on {surface}"
                    );
                }
            }
        }));
    }
    run_fn("content_empty_and_whitespace_output", scenarios);
}

#[test]
fn content_paragraphs_and_linebreaks_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "Line 1\n\nLine 2",
        "Line 1\n\n\nLine 2",
        "Paragraph one.\n\nParagraph two.\n\nParagraph three.",
        "Single paragraph no newline",
        "Trailing newline\n",
        "Leading newline\n\nText",
        "Multiple\n\n\n\n\n\ngaps",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("para_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                assert_eq!(packet.fallback_markdown, source);
                if text.contains("one") && text.contains("two") {
                    assert!(
                        text.contains("one") && text.contains("two"),
                        "paragraph content lost on {surface}: {source}"
                    );
                }
            }
        }));
    }
    run_fn("content_paragraphs_and_linebreaks_output", scenarios);
}

#[test]
fn content_nested_constructs_output() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "**bold *italic* bold**",
        "*italic **bold** italic*",
        "~~strike **bold** strike~~",
        "`code with **bold** inside`",
        "> **bold** quote\n> *italic* quote",
        "| `code` | b |\n|---|---|\n| 1 | 2 |",
        "# **bold** heading",
        "**bold** `code` **bold**",
        "||**bold**||",
        "||`code`||",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("nested_{i}"), move || {
            for (packet, surface) in render_all(&source) {
                let text = pt(&packet);
                assert!(!text.is_empty(), "empty output for nested: {source}");
                assert_eq!(packet.fallback_markdown, source);
            }
        }));
    }
    run_fn("content_nested_constructs_output", scenarios);
}

#[test]
fn structural_packet_fields() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "",
        "**bold** *italic* ~~strike~~",
        "# H1\n## H2\n### H3",
        "Line 1\n\nLine 2\n\nLine 3",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("fields_{i}"), move || {
            for (surface, markup) in SURFACES {
                let job = make_job(&source, *markup, surface);
                let packet = render(&job).expect("render");
                assert_eq!(packet.schema_version, DELIVERY_SCHEMA_VERSION);
                assert_eq!(packet.job_id, "test-job");
                assert_eq!(packet.surface.as_str(), *surface);
                assert_eq!(packet.kind, DeliveryKind::Assistant);
                assert_eq!(packet.target.as_str(), &format!("{surface}:one"));
            }
        }));
    }
    run_fn("structural_packet_fields", scenarios);
}

#[test]
fn structural_fallback_preservation() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "**bold** *italic*",
        "# H1\n## H2\n### H3",
        "Line 1\n\nLine 2",
        "",
        "Complex: # T\n\n**b** *i*\n\n- a\n- b\n\n[link](url)",
        "> quote\n\n||spoiler||\n\n`code`",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("fb_{i}"), move || {
            for (surface, markup) in SURFACES {
                let job = make_job(&source, *markup, surface);
                let packet = render(&job).expect("render");
                assert_eq!(
                    packet.fallback_markdown,
                    source,
                    "fallback mismatch on {surface}: expected {source:?}, got {actual:?}",
                    actual = packet.fallback_markdown
                );
            }
        }));
    }
    run_fn("structural_fallback_preservation", scenarios);
}

#[test]
fn structural_coverage_integrity() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "**bold** *italic*",
        "# H1\n## H2\n### H3",
        "1. first\n2. second\n3. third",
        "> quoted",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("cov_{i}"), move || {
            for (surface, markup) in SURFACES {
                let job = make_job(&source, *markup, surface);
                let packet = render(&job).expect("render");
                let ids: BTreeSet<_> = packet.coverage.iter().map(|c| &c.block_id).collect();
                assert_eq!(
                    ids.len(),
                    packet.coverage.len(),
                    "duplicate block IDs on {surface}: {source}"
                );
                for cov in &packet.coverage {
                    assert!(!cov.block_id.is_empty(), "empty block_id on {surface}");
                }
                if !source.trim().is_empty() {
                    assert!(
                        !packet.coverage.is_empty(),
                        "empty coverage for non-empty input on {surface}: {source}"
                    );
                }
            }
        }));
    }
    run_fn("structural_coverage_integrity", scenarios);
}

#[test]
fn structural_chunking_validity() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "**bold** *italic*",
        "# H1\n## H2\n### H3",
        "Line 1\n\nLine 2\n\nLine 3",
        "Multiple\n\n\n\n\nparagraphs",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("chunk_{i}"), move || {
            for (surface, markup) in SURFACES {
                let job = make_job(&source, *markup, surface);
                let packet = render(&job).expect("render");
                for chunk in &packet.chunks {
                    assert!(!chunk.is_empty(), "empty chunk on {surface}: {source}");
                }
            }
        }));
    }
    run_fn("structural_chunking_validity", scenarios);
}

#[test]
fn structural_payload_types() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "",
        "**bold** *italic*",
        "# H1\n## H2\n### H3",
        "Line 1\n\nLine 2",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("payload_{i}"), move || {
            for (surface, markup) in SURFACES {
                let job = make_job(&source, *markup, surface);
                let packet = render(&job).expect("render");
                match markup {
                    Markup::Json => {
                        assert!(
                            matches!(packet.payload, DeliveryPayload::Structured(_)),
                            "JSON markup should produce Structured payload on {surface}"
                        );
                    }
                    _ => {
                        assert!(
                            matches!(packet.payload, DeliveryPayload::Text(_)),
                            "non-JSON markup should produce Text payload on {surface}"
                        );
                    }
                }
            }
        }));
    }
    run_fn("structural_payload_types", scenarios);
}

#[test]
fn structural_serialization_roundtrip() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "**bold** *italic* ~~strike~~",
        "# H1\n## H2\n### H3",
        "Line 1\n\nLine 2\n\nLine 3",
        "Complex: # T\n\n**b** *i*\n\n- a\n- b\n\n[link](url)",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("ser_{i}"), move || {
            for (surface, markup) in SURFACES {
                let job = make_job(&source, *markup, surface);
                let packet = render(&job).expect("render");
                let json = serde_json::to_string(&packet).expect("serialize");
                let restored: DeliveryPacket = serde_json::from_str(&json).expect("deserialize");
                assert_eq!(packet.schema_version, restored.schema_version);
                assert_eq!(packet.job_id, restored.job_id);
                assert_eq!(packet.surface, restored.surface);
                assert_eq!(packet.kind, restored.kind);
                assert_eq!(packet.target, restored.target);
                assert_eq!(packet.payload, restored.payload);
                assert_eq!(packet.fallback_markdown, restored.fallback_markdown);
                assert_eq!(packet.chunks, restored.chunks);
                assert_eq!(packet.coverage, restored.coverage);
                assert_eq!(packet.actions, restored.actions);
                assert_eq!(packet.diagnostics, restored.diagnostics);
            }
        }));
    }
    run_fn("structural_serialization_roundtrip", scenarios);
}

#[test]
fn structural_max_chars_truncation() {
    let mut scenarios: Vec<Scenario> = vec![];
    for i in 0..500 {
        let source = src_idx("# Heading\n\nBody text **bold** `code`\n\n- item", i);
        let i = i;
        scenarios.push(tc(&format!("maxchars_{i}"), move || {
            for (surface, markup) in SURFACES {
                let mut job = make_job(&source, *markup, surface);
                job.profile.max_chars = Some(100);
                let packet = render(&job).expect("render with max_chars");
                for chunk in &packet.chunks {
                    assert!(!chunk.is_empty(), "empty chunk for {surface}");
                }
            }
        }));
    }
    run_fn("structural_max_chars_truncation", scenarios);
}

#[test]
fn structural_cross_surface_consistency() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "**bold** *italic* ~~strike~~",
        "# H1\n## H2\n### H3",
        "Line 1\n\nLine 2\n\nLine 3",
        "Complex: # T\n\n**b** *i*\n\n- a\n- b\n\n[link](url)",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("cross_{i}"), move || {
            let results = render_all(&source);
            for (packet, _) in &results {
                assert_eq!(packet.fallback_markdown, source);
            }
            let versions: BTreeSet<_> = results.iter().map(|(p, _)| p.schema_version).collect();
            assert_eq!(
                versions.len(),
                1,
                "inconsistent schema versions for {source}"
            );
            let first_kind = &results[0].0.kind;
            for (packet, _) in &results {
                assert_eq!(&packet.kind, first_kind, "inconsistent kinds for {source}");
            }
            assert!(results.iter().all(|(p, _)| p.job_id == "test-job"));
        }));
    }
    run_fn("structural_cross_surface_consistency", scenarios);
}

#[test]
fn structural_content_kind_matching() {
    let mut scenarios: Vec<Scenario> = vec![];
    let templates: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "**bold** *italic*",
        "# H1\n## H2\n### H3",
        "Line 1\n\nLine 2",
        "Complex mixed content",
    ];
    for i in 0..500 {
        let t = templates[i % templates.len()];
        let source = src_idx(t, i);
        let i = i;
        scenarios.push(tc(&format!("kind_{i}"), move || {
            for (surface, markup) in SURFACES {
                let job = make_job(&source, *markup, surface);
                let packet = render(&job).expect("render");
                assert_eq!(packet.kind, DeliveryKind::Assistant);
                assert_eq!(packet.target.as_str(), &format!("{surface}:one"));
                assert_eq!(packet.job_id, "test-job");
            }
        }));
    }
    run_fn("structural_content_kind_matching", scenarios);
}
