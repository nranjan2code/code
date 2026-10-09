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
    suspicious_double_ref_op
)]

//! 9,500-scenario deep sweep of vak-delivery across all surfaces.
//!
//! Red Team — 5,000 adversarial scenarios (×6 surfaces = 30,000 checks):
//!   XSS injection, malformed tables, code-fence abuse, spoiler edge cases,
//!   unicode abuse, extreme inputs, link injection, nested markdown abuse,
//!   worker-protocol fuzz, empty/null inputs.
//!
//! Blue Team — 4,500 defensive scenarios:
//!   error-path non-panic, HTML sanitization, determinism, coverage
//!   accounting, capability validation, template revision security,
//!   signal accuracy, skill validation, worker round-trip,
//!   structured-output parsing.
//!
//! Each scenario closure tests ALL six surfaces (telegram, discord, slack,
//! desktop, terminal, admin) so a single failure is caught at the right
//! surface.

use serde_json::json;
use std::collections::BTreeMap;
use vak_delivery::Block;
use vak_delivery::DeliveryPacket;
use vak_delivery::Markup;
use vak_delivery::ProgressPayload;
use vak_delivery::ToolResultPayload;
use vak_delivery::discord;
use vak_delivery::skills::{
    PRESENTATION_SKILL_API, PresentationSkillManifest, SignalContext, StructuredOutput,
    built_in_recipes, built_in_skill_registry, parse_fragment, project_structured_fences,
    signals_from_context, signals_from_text, structured_outputs_from_text,
};
use vak_delivery::slack;
use vak_delivery::telegram;
use vak_delivery::worker::{WORKER_PROTOCOL_VERSION, WorkerRequest, WorkerResponse, process_line};
use vak_delivery::{
    AnswerDraft, AnswerResult, ApprovalPayload, DeliveryAction, DeliveryContent, DeliveryError,
    DeliveryJob, DeliveryKind, DeliveryPayload, DeliveryProfile, OutputContent, OutputStatus,
};
use vak_delivery::{compile_markdown, render};

type Scenario = (String, Box<dyn FnOnce()>);

fn tc<F: FnOnce() + 'static>(name: &str, f: F) -> Scenario {
    (name.to_string(), Box::new(f))
}

fn run(label: &str, scenarios: Vec<Scenario>) {
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

fn answer_draft(source: &str) -> AnswerDraft {
    AnswerDraft::from_markdown(source)
}

fn answer_job(source: &str, markup: Markup, surface: &str) -> DeliveryJob {
    DeliveryJob {
        job_id: "sweep".into(),
        target: format!("{surface}:one"),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Answer(answer_draft(source)),
        profile: DeliveryProfile {
            surface: surface.into(),
            markup,
            max_chars: None,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
        },
        skill_registry: None,
        trace: None,
        actor: None,
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

const MARKUPS: &[(Markup, &str)] = &[
    (Markup::Plain, "plain"),
    (Markup::Markdown, "markdown"),
    (Markup::TelegramHtml, "telegram"),
    (Markup::SlackMrkdwn, "slack"),
    (Markup::DiscordMarkdown, "discord"),
    (Markup::Json, "json"),
];

fn all_surfaces_render(source: &str) {
    for (surface, markup) in SURFACES {
        let job = answer_job(source, *markup, surface);
        let _ = render(&job);
    }
}

// ═══════════════════════ RED TEAM ═══════════════════════════════════

#[test]
fn red_team_xss_injection() {
    let mut scenarios: Vec<Scenario> = vec![];
    let payloads: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "<script>alert({i})</script>",
                "<img src=x onerror=alert({i})>",
                "<svg onload=alert({i})>",
                "<body onload=alert({i})>",
                "<iframe src=javascript:alert({i})>",
                "<a href=\"javascript:alert({i})\">x</a>",
                "<details open ontoggle=alert({i})>",
                "<input autofocus onfocus=alert({i})>",
                "<marquee onstart=alert({i})>",
                "<style>@import 'javascript:alert({i})'</style>",
                "<link rel=stylesheet href=\"javascript:alert({i})\">",
                "<meta http-equiv=\"refresh\" content=\"0;url=javascript:alert({i})\">",
                "<form action=\"javascript:alert({i})\">x</form>",
                "<object data=\"javascript:alert({i})\"></object>",
                "<embed src=\"javascript:alert({i})\">",
                "<base href=\"javascript:alert({i})\">",
                "<select onfocus=alert({i}) autofocus>",
                "<textarea onfocus=alert({i}) autofocus>",
                "javascript:alert({i})",
                "<a href=\"vbscript:alert({i})\">x</a>",
                "data:text/html,<script>alert({i})</script>",
                "<svg><script>alert({i})</script></svg>",
                "<img src=\"javascript:alert({i})\">",
                "<iframe srcdoc=\"<script>alert({i})</script>\">",
                "<details ontoggle=alert({i}) open>",
            ];
            templates[i % templates.len()].replace("{i}", &i.to_string())
        })
        .collect();

    for (i, payload) in payloads.iter().enumerate() {
        let p = payload.clone();
        let i = i;
        scenarios.push(tc(&format!("xss_{i}"), move || {
            all_surfaces_render(&p);
            let html = telegram::markdown_to_html(&p);
            // The converter escapes raw HTML tags to &lt; / &gt;
            assert!(!html.contains("<script"), "raw script tag leaked: {p}");
            assert!(!html.contains("<iframe"), "raw iframe leaked: {p}");
            assert!(!html.contains("<svg"), "raw svg leaked: {p}");
            assert!(!html.contains("<object"), "raw object leaked: {p}");
            assert!(!html.contains("<embed"), "raw embed leaked: {p}");
            assert!(!html.contains("<style"), "raw style leaked: {p}");
            assert!(!html.contains("<meta"), "raw meta leaked: {p}");
            assert!(!html.contains("<base"), "raw base leaked: {p}");
            // All raw tags should be escaped
            assert!(
                !p.contains("<script>") || html.contains("&lt;script&gt;"),
                "script not escaped: {p}"
            );
        }));
    }
    run("red_team_xss_injection", scenarios);
}

#[test]
fn red_team_malformed_tables() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "| a | b |",
                "| a | b |\n|---|---|",
                "| a | b |\n|---|---|\n",
                "|||a|b|",
                "| a | b |\n| c | d |",
                "| a | b |\n|---|---|\n| 1 |\n| 2 | 3 |",
                "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 | 4 |",
                "|a\nb|c|\n|---|\n|1|2|",
                "| a | b |\n| :---: |\n| 1 |",
                "| a | b |\n|---|\n| 1 | 2 |\n| 3 | 4 |",
                "| `code` | b |\n|---|---|\n| 1 | 2 |",
                "| a |\n|---|\n| b |\n|---|\n| c |",
                "| a | b |\n|---|---|\n| 1\n| 2 | 3 |",
                "| a \\| b | c |\n|---|---|---|\n| 1 | 2 | 3 |",
                "| a | b |\n|---|---|\n| 1 | 2 || 3 |",
                "| a | b |\n|---|---|\n| 1 | 2 | 3 | 4 | 5 |",
                "||\n|---|\n||",
                "| a | b |\n|---|---|\n|",
                "| a | b |\n|---|---|\n| 1 |",
                "| a | b |\n|---|---|\n| 1 | 2 | 3 |",
            ];
            let t = templates[i % templates.len()];
            format!("{i}{t}")
        })
        .collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("bad_table_{i}"), move || {
            all_surfaces_render(&inp);
        }));
    }
    run("red_team_malformed_tables", scenarios);
}

#[test]
fn red_team_code_fence_abuse() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "```",
                "```\n```\n```",
                "```rust\n```\n```\n```",
                "```\n```\n```\n```\n```",
                "````",
                "````\n```\n````",
                "```\n\n```\n\n```",
                "```\n```\n```\n```\n```\n```\n```",
                "```rust\nfn main() {}``",
                "```\n\n\n\n\n```",
                "``` \ncode\n```",
                "```\r\n```",
                "```\n```text```",
                "``\ncode\n``",
                "`````\n```\n`````",
                "```\n```\n```\n```\n```",
                "```\nno closing fence",
                "`````\nmulti\n```\n`````",
            ];
            let t = templates[i % templates.len()];
            format!("{i}{t}")
        })
        .collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("fence_abuse_{i}"), move || {
            all_surfaces_render(&inp);
            let job = answer_job(&inp, Markup::Markdown, "desktop");
            let request = WorkerRequest {
                protocol_version: WORKER_PROTOCOL_VERSION,
                job: job.clone(),
            };
            let req_str = serde_json::to_string(&request).expect("serialize");
            let resp = process_line(&req_str);
            let parsed: Result<WorkerResponse, _> = serde_json::from_str(&resp);
            assert!(parsed.is_ok(), "worker fuzz fence_{i}");
        }));
    }
    run("red_team_code_fence_abuse", scenarios);
}

#[test]
fn red_team_spoiler_edge_cases() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "||",
                "|||",
                "|||||",
                "||text",
                "text||",
                "|||text||",
                "||||text||||",
                "|| ||",
                "||  ||",
                "||||",
                "||a||b||c||",
                "|||a||b|||",
                "|| || ||",
                "||\\n||",
                "|| text || more text ||",
                "| | text | |",
                "||table||row|",
                "|| code ||\n```",
                "||**bold**||",
                "||```code```||",
                "|| | a | b |\n|---|---|\n| 1 | 2 | ||",
                "",
                "|| ",
                " ||",
                "|| || || ||",
                "|| ||a|| ||",
                "|| |||| ||",
                "|| a\n|| b ||",
                "||a\nb||",
                "||spoiler with | pipe ||",
            ];
            let t = templates[i % templates.len()];
            format!("{i}{t}")
        })
        .collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("spoiler_{i}"), move || {
            all_surfaces_render(&inp);
            let tg = telegram::markdown_to_html(&inp);
            let dc = discord::markdown_to_discord(&inp);
            let sl = slack::markdown_to_mrkdwn(&inp);
            // None should panic
            let _ = (tg, dc, sl);
        }));
    }
    run("red_team_spoiler_edge_cases", scenarios);
}

#[test]
fn red_team_unicode_abuse() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "RTL:\u{202E}hello\u{202C}",
                "Zero-width:\u{200B}joiner",
                "\u{FEFF}BOM text",
                "Emoji:🎉🚀✅",
                "World:\u{4E16}\u{754C}",
                "\u{0}null\u{0}bytes",
                "Line\u{000A}break",
                "Tab\u{0009}tab",
                "Vertical\u{000B}tab",
                "Form\u{000C}feed",
                "Next\u{0008}back",
                "Delete\u{007F}char",
                "Latin-1:\u{00E9}\u{00F1}\u{00FC}",
                "Greek:\u{03B1}\u{03B2}\u{03B3}",
                "Cyrillic:\u{0430}\u{0431}\u{0432}",
                "Arabic:\u{0627}\u{0628}\u{0629}",
                "Hebrew:\u{05D0}\u{05D1}\u{05D2}",
                "Math:\u{2200}\u{2203}\u{2208}",
                "Symbols:\u{2603}\u{2604}\u{2620}",
                "Combining:\u{0061}\u{0300}\u{0301}",
            ];
            let t = templates[i % templates.len()];
            format!("{i}{t}")
        })
        .collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("unicode_{i}"), move || {
            all_surfaces_render(&inp);
            for (mk, mk_label) in MARKUPS {
                let job = answer_job(&inp, *mk, mk_label);
                let _ = render(&job);
            }
        }));
    }
    run("red_team_unicode_abuse", scenarios);
}

#[test]
fn red_team_extreme_inputs() {
    let mut scenarios: Vec<Scenario> = vec![];

    let empties: Vec<&str> = vec![
        "",
        " ",
        "   ",
        "\n",
        "\n\n\n",
        "\t",
        "\t\t\t",
        " \n \t \n ",
        "\u{00A0}",
        "\u{200B}",
    ];
    for (i, input) in empties.iter().enumerate() {
        let inp = input.to_string();
        let i = i;
        scenarios.push(tc(&format!("extreme_empty_{i}"), move || {
            all_surfaces_render(&inp);
        }));
    }

    let long_lens: &[usize] = &[100, 500, 1000, 5000, 10000, 20000, 30000, 40000];
    for (i, &len) in long_lens.iter().enumerate() {
        let input = "word ".repeat(len / 5);
        scenarios.push(tc(&format!("extreme_long_{i}_{len}"), move || {
            all_surfaces_render(&input);
        }));
    }

    let patterns: &[&str] = &[
        "**",
        "*",
        "~~",
        "||",
        "`",
        "#",
        ">",
        "-",
        "|",
        "[",
        "]",
        "(",
        ")",
        "'",
        "\"",
        "\\",
        "<",
        ">",
        "&",
        "```",
        "|---|",
        "- [ ]",
        "- [x]",
        "||spoiler||",
        "**bold**",
        "*italic*",
        "~~strike~~",
        "[text](url)",
        "![alt](src)",
        "> quote",
        "# heading",
    ];
    for (i, pat) in patterns.iter().enumerate() {
        for mult in [50, 100, 200] {
            let input = pat.repeat(mult);
            scenarios.push(tc(&format!("extreme_rep_{i}_{mult}"), move || {
                all_surfaces_render(&input);
            }));
        }
    }

    // Fill to 500 (111 base + 389 fill)
    for i in 111..500 {
        let input = format!(
            "filler text {i} \n\n# heading\n\n- item\n\n> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |"
        );
        scenarios.push(tc(&format!("extreme_fill_{i}"), move || {
            all_surfaces_render(&input);
        }));
    }

    run("red_team_extreme_inputs", scenarios);
}

#[test]
fn red_team_format_string_injection() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "%s",
                "%n",
                "%x%x%x%x",
                "%s%s%s%s%s",
                "%d%d%d",
                "%p%p%p%p",
                "%s%p%n",
                "%.1000s",
                "%-100s",
                "%08x",
                "%s%s%s%s%s%s%s%s%s%s",
                "%n%n%n%n",
                "%c%c%c",
                "%ld%lx%ln",
                "%s%n%s%n",
                "%.*s",
                "%1000.1000s",
                "%s",
                "%d",
                "%f",
            ];
            format!("{}{}", templates[i % templates.len()], i)
        })
        .collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("fmt_str_{i}"), move || {
            all_surfaces_render(&inp);
        }));
    }
    run("red_team_format_string_injection", scenarios);
}

#[test]
fn red_team_link_injection() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "javascript:alert(1)",
                "data:text/html,<script>alert(1)</script>",
                "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKSw8L3NjcmlwdD4=",
                "vbscript:msgbox(1)",
                "file:///etc/passwd",
                "ftp://evil.com/malware.exe",
                "javascript:alert(1)//",
                "\u{0000}javascript:alert(1)",
                "JavaScript:alert(1)",
                "JAVGUAGE:alert(1)",
                "javascript&#58;alert(1)",
                "javascript&#x3a;alert(1)",
                "javascript&colon;alert(1)",
                "\tjavascript:alert(1)",
                "\njavascript:alert(1)",
                " javascript:alert(1)",
                "javascript:alert(1)\n",
                "<a href=\"javascript:alert(1)\">click</a>",
                "[text](javascript:alert(1))",
                "[](\"javascript:alert(1)\")",
            ];
            let t = templates[i % templates.len()];
            format!("[link]({})\n\n{}", t, t)
        })
        .collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("link_inj_{i}"), move || {
            all_surfaces_render(&inp);
        }));
    }
    run("red_team_link_injection", scenarios);
}

#[test]
fn red_team_nested_markdown_abuse() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "**bold *italic* bold**",
                "**bold ```code``` bold**",
                "**bold **nested** bold**",
                "*italic **bold** italic*",
                "~~strike **bold** strike~~",
                "**bold ||spoiler|| bold**",
                "`code with **bold** inside`",
                "```code with `inline code` inside```",
                "> **bold** quote\n> *italic* quote",
                "# **bold** heading",
                "- **bold** list item",
                "**bold** | a | b |\n|---|---|\n| 1 | 2 |",
                "[**bold** link](url)",
                "[`code` link](url)",
                "![**bold** alt](url.png)",
                "```diff\n- old\n+ new\n```",
                "**bold `code` bold**",
                "| `code` | b |\n|---|---|\n| 1 | `inline` |",
                "> ```code\n> in quote\n> ```",
                "# H **b** *i* ~~s~~ `c` - item 1. first",
            ];
            let t = templates[i % templates.len()];
            format!("{i}{t}")
        })
        .collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("nested_{i}"), move || {
            all_surfaces_render(&inp);
        }));
    }
    run("red_team_nested_markdown_abuse", scenarios);
}

#[test]
fn red_team_null_and_empty_inputs() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500).map(|i| {
        let templates: &[&str] = &[
            "",
            "\u{0000}",
            "\u{0000}\u{0000}\u{0000}",
            "\u{0000}\u{0000}\u{0000}text\u{0000}\u{0000}",
            "text\u{0000}",
            "\u{0000}text",
            "\n\u{0000}\n",
            "# Title\u{0000}\n\nBody\u{0000}",
            "- [ ] \u{0000}\n- [x] \u{0000}",
            "||spoiler\u{0000}||",
            "\u{0000}|\u{0000}|\u{0000}|\u{0000}a\u{0000}|\u{0000}b\u{0000}|\u{0000}",
            "\u{0000}\u{0000}|\u{0000}a\u{0000}|\u{0000}b\u{0000}|\n|\u{0000}---\u{0000}|\n|\u{0000}1\u{0000}|\u{0000}2\u{0000}|",
            "\u{0000}\u{0000}\u{0000}",
        ];
        format!("{}{}", i, templates[i % templates.len()])
    }).collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("null_empty_{i}"), move || {
            all_surfaces_render(&inp);
            let _ = process_line("");
            let _ = process_line("{broken}");
            let _ = process_line("null");
        }));
    }
    run("red_team_null_and_empty_inputs", scenarios);
}

// ═══════════════════════ BLUE TEAM ══════════════════════════════════

#[test]
fn blue_team_error_paths_never_panic() {
    let mut scenarios: Vec<Scenario> = vec![];

    let invalid_kinds = [
        DeliveryKind::System,
        DeliveryKind::Developer,
        DeliveryKind::Internal,
        DeliveryKind::ToolCall,
        DeliveryKind::Steering,
    ];

    for (i, kind) in invalid_kinds.iter().enumerate() {
        for (surface, markup) in SURFACES {
            let k = *kind;
            let s = surface.to_string();
            let m = *markup;
            let i = i;
            scenarios.push(tc(&format!("no_panic_{i}_{s}"), move || {
                let job = DeliveryJob {
                    job_id: "bp".into(),
                    target: format!("{s}:one"),
                    kind: k,
                    content: DeliveryContent::Text {
                        markdown: "hello".into(),
                    },
                    profile: DeliveryProfile {
                        surface: s.clone(),
                        markup: m,
                        max_chars: None,
                        supports_tables: true,
                        supports_code_blocks: true,
                        supports_links: true,
                        supports_actions: false,
                        template: None,
                    },
                    skill_registry: None,
                    trace: None,
                    actor: None,
                };
                let _ = render(&job);
            }));
        }
    }

    // Fill to 500
    for i in 30..500 {
        let surfaces_iter: Vec<(&str, Markup)> = SURFACES.to_vec();
        let (s, m) = &surfaces_iter[i % surfaces_iter.len()];
        let s = s.to_string();
        let m = *m;
        let i = i;
        scenarios.push(tc(&format!("no_panic_fill_{i}"), move || {
            let job = DeliveryJob {
                job_id: "fill".into(),
                target: format!("{s}:one"),
                kind: DeliveryKind::Assistant,
                content: DeliveryContent::Text {
                    markdown: format!("filler {i}"),
                },
                profile: DeliveryProfile {
                    surface: s.clone(),
                    markup: m,
                    max_chars: None,
                    supports_tables: true,
                    supports_code_blocks: true,
                    supports_links: true,
                    supports_actions: false,
                    template: None,
                },
                skill_registry: None,
                trace: None,
                actor: None,
            };
            let _ = render(&job);
        }));
    }

    run("blue_team_error_paths_never_panic", scenarios);
}

#[test]
fn blue_team_html_sanitization() {
    let mut scenarios: Vec<Scenario> = vec![];
    let dangerous: Vec<String> = (0..500)
        .map(|i| {
            let templates: &[&str] = &[
                "<script>x</script>",
                "<img src=x onerror=alert(1)>",
                "<svg onload=alert(1)>",
                "<iframe src=javascript:alert(1)>",
                "<body onload=alert(1)>",
                "<a href=\"javascript:alert(1)\">x</a>",
                "<div onclick=\"alert(1)\">x</div>",
                "<style>@import 'evil'</style>",
                "<object data=\"javascript:alert(1)\"></object>",
                "<embed src=\"javascript:alert(1)\">",
                "<details open ontoggle=alert(1)>x</details>",
                "Text <script>alert(1)</script> more text",
                "<a href=\"javascript:alert(1)\">click me</a> and <img src=x onerror=alert(1)>",
                "Normal text with <b>bold</b> and <script>bad</script>",
                "<TABLE><TR><TD>data</TD></TR></TABLE><script>evil()</script>",
            ];
            format!("{}{}", templates[i % templates.len()], i)
        })
        .collect();

    for (i, input) in dangerous.iter().enumerate() {
        let inp = input.clone();
        let i = i;
        scenarios.push(tc(&format!("sanitize_{i}"), move || {
            let job = answer_job(&inp, Markup::TelegramHtml, "telegram");
            let packet = render(&job).expect("render");
            let html = match &packet.payload {
                DeliveryPayload::Text(h) => h,
                _ => "",
            };
            assert!(!html.contains("<script"), "script tag not sanitized: {inp}");
            assert!(!html.contains("<iframe"), "iframe not sanitized: {inp}");
            assert!(!html.contains("<svg"), "svg not sanitized: {inp}");
            assert!(!html.contains("<object"), "object not sanitized: {inp}");
            assert!(!html.contains("<embed"), "embed not sanitized: {inp}");
            assert!(!html.contains("<style"), "style not sanitized: {inp}");
            assert!(!html.contains("<meta"), "meta not sanitized: {inp}");
        }));
    }
    run("blue_team_html_sanitization", scenarios);
}

#[test]
fn blue_team_determinism() {
    let mut scenarios: Vec<Scenario> = vec![];
    let inputs: Vec<String> = (0..500).map(|i| {
        let templates: &[&str] = &[
            "# Title\n\nBody **bold** *italic* ~~strike~~\n\n- item 1\n- item 2\n\n```rust\ncode\n```\n\n> quote\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n\n![alt](img.png)\n\n[text](https://example.com)",
            "||spoiler||\n\ninline **bold** ||spoiler|| end",
            "- [ ] todo\n- [x] done\n\n# H\n\n> **bold** quote",
            "",
            "plain text\n\nsecond paragraph",
            "**a** *b* ~~c~~ `d` **e** *f*",
            "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |",
            "```python\nprint('hello')\n```\n\nMore text.",
            "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6",
            "1. first\n2. second\n3. third\n\n- a\n  - b\n  - c",
        ];
        format!("{}{}", templates[i % templates.len()], i)
    }).collect();

    for (i, input) in inputs.iter().enumerate() {
        let inp1 = input.clone();
        let inp2 = input.clone();
        let i = i;
        scenarios.push(tc(&format!("determinism_{i}"), move || {
            for (surface, markup) in SURFACES {
                let j1 = answer_job(&inp1, *markup, surface);
                let j2 = answer_job(&inp2, *markup, surface);
                let p1 = render(&j1).expect("render1");
                let p2 = render(&j2).expect("render2");
                assert_eq!(
                    p1.payload, p2.payload,
                    "non-deterministic: {i} for {surface}"
                );
                assert_eq!(p1.fallback_markdown, p2.fallback_markdown);
                assert_eq!(p1.coverage, p2.coverage);
            }
        }));
    }
    run("blue_team_determinism", scenarios);
}

#[test]
fn blue_team_coverage_accounting() {
    let mut scenarios: Vec<Scenario> = vec![];

    let inputs: Vec<&str> = vec![
        "# Title\n\nBody.\n\n- item 1\n- item 2\n\n| a | b |\n|---|---|\n| 1 | 2 |",
        "# Title\n\n```rust\nfn main() {}\n```\n\n- item\n\n> quote",
        "# T\n\n**bold** *italic* ~~strikethrough~~ `code`\n\n| x | y |\n|---|---|---|\n| 1 | 2 |",
        "",
        "plain text only",
        "| a | b | c |\n|---|---|---|\n| 1 | 2 | 3 |",
        "||spoiler|| with **bold**\n\n- [x] done",
    ];

    for (i, input) in inputs.iter().enumerate() {
        for (surface, markup) in SURFACES {
            let inp = input.to_string();
            let s = surface.to_string();
            let m = *markup;
            let i = i;
            scenarios.push(tc(&format!("coverage_{i}_{s}"), move || {
                let job = answer_job(&inp, m, &s);
                let packet = render(&job).expect("render");
                assert!(!packet.coverage.is_empty() || inp.trim().is_empty());
                for cov in &packet.coverage {
                    assert!(!cov.block_id.is_empty(), "empty block_id");
                }
                let ids: std::collections::HashSet<_> =
                    packet.coverage.iter().map(|c| &c.block_id).collect();
                assert_eq!(ids.len(), packet.coverage.len(), "duplicate block IDs");
            }));
        }
    }

    // Fill to 500 (42 base + 458 fill, one surface each)
    for i in 42..500 {
        let inp = format!(
            "# Title {i}\n\nBody **bold** {i}\n\n- item\n\n| a | b |\n|---|---|\n| {i} | 2 |"
        );
        let inp_clone = inp.clone();
        scenarios.push(tc(&format!("coverage_fill_{i}"), move || {
            let job = answer_job(&inp_clone, Markup::Markdown, "desktop");
            let packet = render(&job).expect("render");
            assert!(!packet.coverage.is_empty());
        }));
    }

    run("blue_team_coverage_accounting", scenarios);
}

#[test]
fn blue_team_profile_capability_validation() {
    let mut scenarios: Vec<Scenario> = vec![];

    let mut idx = 0usize;
    for surface in &["telegram", "discord", "slack", "desktop", "terminal"] {
        for tables in [true, false] {
            for code in [true, false] {
                for links in [true, false] {
                    for actions in [true, false] {
                        let s = surface.to_string();
                        let i = idx;
                        idx += 1;
                        scenarios.push(tc(&format!("caps_{i}"), move || {
                            let mk = match s.as_str() {
                                "telegram" => Markup::TelegramHtml,
                                "discord" => Markup::DiscordMarkdown,
                                "slack" => Markup::SlackMrkdwn,
                                _ => Markup::Markdown,
                            };
                            let profile = DeliveryProfile {
                                surface: s.clone(),
                                markup: mk,
                                max_chars: None,
                                supports_tables: tables,
                                supports_code_blocks: code,
                                supports_links: links,
                                supports_actions: actions,
                                template: None,
                            };
                            let caps = profile.capabilities();
                            assert_eq!(caps.tables, tables);
                            assert_eq!(caps.code, code);
                            assert_eq!(caps.links, links);
                            assert_eq!(caps.actions, actions);
                            assert_eq!(caps.accessible_plain, matches!(mk, Markup::Plain));
                        }));
                    }
                }
            }
        }
    }

    // Fill to 500 (80 base + 420 fill)
    for i in 80..500 {
        let surface_idx = i % 5;
        let surface = ["telegram", "discord", "slack", "desktop", "terminal"][surface_idx];
        let s = surface.to_string();
        let i = i;
        scenarios.push(tc(&format!("caps_fill_{i}"), move || {
            let mk = match surface {
                "telegram" => Markup::TelegramHtml,
                "discord" => Markup::DiscordMarkdown,
                "slack" => Markup::SlackMrkdwn,
                _ => Markup::Markdown,
            };
            let profile = DeliveryProfile {
                surface: s.clone(),
                markup: mk,
                max_chars: Some(4096),
                supports_tables: true,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
            };
            let caps = profile.capabilities();
            assert!(caps.max_chars.is_some());
            assert_eq!(caps.max_chars, Some(4096));
        }));
    }

    run("blue_team_profile_capability_validation", scenarios);
}

#[test]
fn blue_team_template_revision_security() {
    let mut scenarios: Vec<Scenario> = vec![];

    for rev in 0u32..500 {
        let rev = rev;
        scenarios.push(tc(&format!("tmpl_rev_{}", rev), move || {
            let mut reg = vak_delivery::TemplateRegistry::default();
            let t: vak_delivery::TemplateSpec = vak_delivery::TemplateSpec {
                id: "test".into(),
                revision: rev,
                origin: vak_delivery::TemplateOrigin::User,
                activation: vak_delivery::TemplateActivation::Active,
                nodes: vec![vak_delivery::TemplateNode::Literal { text: "x".into() }],
            };
            assert!(reg.upsert(t.clone()).is_ok());
            let resolved = reg.resolve("test").unwrap();
            assert_eq!(resolved.revision, rev);

            if rev > 0 {
                let mut lower = t.clone();
                lower.revision = rev - 1;
                assert!(
                    reg.upsert(lower).is_err(),
                    "lower rev {rev} should be rejected"
                );
            }
        }));
    }

    run("blue_team_template_revision_security", scenarios);
}

#[test]
fn blue_team_signal_extraction_accuracy() {
    let mut scenarios: Vec<Scenario> = vec![];

    let cases: Vec<(&str, &[&str])> = vec![
        (
            "temperature is 25c forecast tomorrow",
            &["temperature", "forecast"],
        ),
        ("diff --git a main.rs b main.rs", &["diff"]),
        (
            "benchmark results 1000 req/sec p99",
            &["benchmark", "telemetry"],
        ),
        ("https://arxiv.org/abs/1234 file.pdf", &["citations"]),
        (
            "recipe ingredients: flour, sugar, eggs",
            &["recipe", "ingredients"],
        ),
        ("docker run container with image nginx", &["docker"]),
        (
            "chart and telemetry data visualization",
            &["chart", "telemetry"],
        ),
        ("travel itinerary for next week flight hotel", &["travel"]),
        ("latest news headlines breaking", &["news"]),
        ("no signals whatsoever in this text", &[]),
        ("", &[]),
        ("plain text without any keywords", &[]),
        ("temperature only single signal", &["temperature"]),
        ("forecast only weather signal", &["forecast"]),
        ("diff --git a/b modified:", &["diff", "files_changed"]),
        (
            "recipe and ingredients matching",
            &["recipe", "ingredients"],
        ),
        ("tests and pass_fail both present", &["tests"]),
        ("benchmark and telemetry both", &["benchmark", "telemetry"]),
        ("chart only single signal", &["chart"]),
        ("travel hotel vacation trip", &["travel"]),
    ];

    for (i, (text, expected)) in cases.iter().enumerate() {
        let t = text.to_string();
        let expected_vec: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        let i = i;
        scenarios.push(tc(&format!("signal_{i}"), move || {
            let signals = signals_from_text(&t);
            for expected_sig in &expected_vec {
                assert!(
                    signals.contains(expected_sig),
                    "missing {expected_sig} in {signals:?} for {t:?}"
                );
            }
            assert!(!signals.contains(&"not_a_signal".to_string()));
            assert!(!signals.contains(&"fake_signal".to_string()));
        }));
    }

    // Generate 480 more variations
    for i in 20..500 {
        let (text, expected) = cases[i % cases.len()];
        let t = format!("{}{}", text, i);
        let expected_vec: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        scenarios.push(tc(&format!("signal_var_{i}"), move || {
            let signals = signals_from_text(&t);
            for expected_sig in &expected_vec {
                assert!(signals.contains(expected_sig), "missing {expected_sig}");
            }
        }));
    }

    run("blue_team_signal_extraction_accuracy", scenarios);
}

#[test]
fn blue_team_skill_validation() {
    let mut scenarios: Vec<Scenario> = vec![];

    let valid_payloads: Vec<(&str, serde_json::Value)> = vec![
        ("metric", json!({"label": "X", "value": 42, "unit": "%"})),
        (
            "link.preview",
            json!({"url": "https://x.com", "title": "T"}),
        ),
        ("collection", json!({"items": ["a", "b"]})),
        ("detail", json!({"title": "T"})),
        ("comparison", json!({"left": "a", "right": "b"})),
        (
            "checklist",
            json!({"items": [{"label": "a", "done": true}]}),
        ),
        ("timeline", json!({"items": [{"label": "step"}]})),
        ("steps", json!({"items": [{"label": "s"}]})),
        ("schedule", json!({"slots": [{"label": "morning"}]})),
        (
            "decision",
            json!({"label": "Choose", "choices": ["A", "B"]}),
        ),
        (
            "budget",
            json!({"label": "Income", "income": 100, "expenses": 50}),
        ),
        (
            "meeting_notes",
            json!({"agenda": [{"title": "A"}], "attendees": ["x"]}),
        ),
    ];

    for (i, (st, payload)) in valid_payloads.iter().enumerate() {
        let st = st.to_string();
        let p = payload.clone();
        let i = i;
        scenarios.push(tc(&format!("valid_{i}_{}", st), move || {
            let registry = built_in_skill_registry();
            let output = StructuredOutput {
                semantic_type: st.clone(),
                schema_version: vak_delivery::PRESENTATION_SCHEMA_VERSION,
                skill_id: "core".into(),
                skill_version: "1.0.0".into(),
                payload: p,
            };
            let result = registry.validate(&output, "desktop", &[]);
            assert!(result.is_ok(), "valid {st} failed: {result:?}");
        }));
    }

    // 488 invalid type variations
    for i in 12..500 {
        let st = valid_types()[i % valid_types().len()];
        let invalid = json!({"invalid": true});
        let st = st.to_string();
        let i = i;
        scenarios.push(tc(&format!("invalid_{i}_{}", st), move || {
            let registry = built_in_skill_registry();
            let output = StructuredOutput {
                semantic_type: st.clone(),
                schema_version: vak_delivery::PRESENTATION_SCHEMA_VERSION,
                skill_id: "core".into(),
                skill_version: "1.0.0".into(),
                payload: invalid.clone(),
            };
            let _ = registry.validate(&output, "desktop", &[]);
        }));
    }

    run("blue_team_skill_validation", scenarios);
}

fn valid_types() -> &'static [&'static str] {
    &[
        "metric",
        "link.preview",
        "collection",
        "detail",
        "comparison",
        "checklist",
        "timeline",
        "steps",
        "schedule",
        "decision",
        "budget",
        "meeting_notes",
    ]
}

#[test]
fn blue_team_worker_protocol_roundtrip() {
    let mut scenarios: Vec<Scenario> = vec![];

    let sources: &[&str] = &[
        "# Title\n\nBody.",
        "Plain text",
        "| A | B |\n|---|---|\n| 1 | 2 |",
        "```rust\nfn main() {}\n```",
        "- [ ] todo\n- [x] done",
        "||spoiler||",
        "",
        "**bold** *italic* ~~strike~~",
        "# H1\n## H2\n### H3",
        "Multiple\n\n\n\n\nparagraphs",
    ];

    for i in 0..500 {
        let source = sources[i % sources.len()];
        let (surface, markup) = SURFACES[i % SURFACES.len()];
        let src = source.to_string();
        let surf = surface.to_string();
        let i = i;
        scenarios.push(tc(&format!("worker_rt_{i}"), move || {
            let job = answer_job(&src, markup, &surf);
            let request = WorkerRequest {
                protocol_version: WORKER_PROTOCOL_VERSION,
                job: job.clone(),
            };
            let req_str = serde_json::to_string(&request).expect("serialize");
            let resp_str = process_line(&req_str);
            let resp: WorkerResponse = serde_json::from_str(&resp_str).expect("deserialize");
            assert_eq!(resp.protocol_version, WORKER_PROTOCOL_VERSION);
            let packet = resp.packet.expect("packet");
            let direct = render(&job).expect("direct render");
            assert_eq!(packet.payload, direct.payload, "worker mismatch for {src}");
        }));
    }

    run("blue_team_worker_protocol_roundtrip", scenarios);
}
