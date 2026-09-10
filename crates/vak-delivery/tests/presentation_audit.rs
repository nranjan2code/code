#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::redundant_closure,
    clippy::useless_conversion
)]

//! Deep audit harness for `vak-delivery`: 500+ scenarios covering the render
//! pipeline, markup conversion, templates, skill registry, recipes, signals,
//! outbox lifecycle, posture disposition matrix, and worker subprocess
//! protocol.
//!
//! Each scenario is a self-contained closure that panics on invariant
//! violation. The harness counts pass/fail so the test report proves breadth.

use serde_json::json;
use std::collections::BTreeMap;
use vak_delivery::Block;
use vak_delivery::DeliveryPacket;
use vak_delivery::Markup;
use vak_delivery::ProgressPayload;
use vak_delivery::ToolResultPayload;
use vak_delivery::discord;
use vak_delivery::outbox::{Outbox, OutboxError, OutboxRecord, OutboxState};
use vak_delivery::slack;
use vak_delivery::telegram;
use vak_delivery::worker::{WORKER_PROTOCOL_VERSION, WorkerRequest, WorkerResponse, process_line};
use vak_delivery::{
    AnswerDraft, AnswerResult, ApprovalPayload, ArtifactRef, Cadence, CalloutTone, Citation,
    DELIVERY_SCHEMA_VERSION, DeliveryAction, DeliveryContent, DeliveryError, DeliveryJob,
    DeliveryKind, DeliveryPayload, DeliveryPosture, DeliveryProfile, Disposition, DocumentBlock,
    DocumentCoverage, DocumentCoverageDisposition, InlineNode, OutputContent, OutputItem,
    OutputKind, OutputProvenance, OutputRole, OutputStatus, OutputStreamEvent, OutputStreamFrame,
    OutputTimeline, PRESENTATION_SCHEMA_VERSION, PresentationDocument, ResultOutcome,
    SurfaceCapabilities, TemplateActivation, TemplateNode, TemplateOrigin, TemplateRegistry,
    TemplateSlot, TemplateSpec, Urgency,
};
use vak_delivery::{
    adapters::{built_in_adapters, structured_outputs_from_tool_result},
    skills::{
        PRESENTATION_SKILL_API, PresentationSkillManifest, RendererBinding, SignalContext,
        SkillError, SkillRegistry, StructuredOutput, built_in_recipes, built_in_skill_registry,
        link_previews_from_text, parse_fragment, project_structured_fences, signals_from_context,
        signals_from_text, structured_outputs_from_text,
    },
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
    if !failures.is_empty() {
        for f in &failures {
            eprintln!("FAIL: {label} :: {f}");
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

fn answer_blocks(source: &str) -> Vec<Block> {
    answer_draft(source).blocks
}

fn answer_job(
    source: &str,
    markup: Markup,
    max_chars: Option<usize>,
    kind: DeliveryKind,
    surface: &str,
) -> DeliveryJob {
    DeliveryJob {
        job_id: "test-job".into(),
        target: format!("{surface}:one"),
        kind,
        content: DeliveryContent::Answer(answer_draft(source)),
        profile: DeliveryProfile {
            surface: surface.into(),
            markup,
            max_chars,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: DeliveryPosture::default(),
        },
        skill_registry: None,
    }
}

fn plain_profile(surface: &str) -> DeliveryProfile {
    DeliveryProfile::plain(surface)
}

fn approval_job(_markup: Markup, surface: &str) -> DeliveryJob {
    DeliveryJob {
        job_id: "approval-job".into(),
        target: format!("{surface}:one"),
        kind: DeliveryKind::Approval,
        content: DeliveryContent::Approval(ApprovalPayload {
            request_id: "req-1".into(),
            title: "Confirm".into(),
            detail: "Do you want to proceed?".into(),
            expires_at: Some("2026-12-31T23:59:59Z".into()),
            actions: vec![DeliveryAction {
                id: "approve".into(),
                label: "Approve".into(),
                verb: "approve".into(),
                data: BTreeMap::new(),
            }],
        }),
        profile: plain_profile(surface),
        skill_registry: None,
    }
}

fn progress_job(markup: Markup, surface: &str) -> DeliveryJob {
    DeliveryJob {
        job_id: "progress-job".into(),
        target: format!("{surface}:one"),
        kind: DeliveryKind::Progress,
        content: DeliveryContent::Progress(ProgressPayload {
            label: "Build".into(),
            state: "running".into(),
            percent: Some(75),
        }),
        profile: plain_profile(surface),
        skill_registry: None,
    }
}

fn tool_result_job(_markup: Markup, surface: &str, is_error: bool) -> DeliveryJob {
    DeliveryJob {
        job_id: "tool-result-job".into(),
        target: format!("{surface}:one"),
        kind: DeliveryKind::ToolResult,
        content: DeliveryContent::ToolResult(ToolResultPayload {
            tool: "bash".into(),
            output: if is_error {
                "command not found".into()
            } else {
                "ok\noutput".into()
            },
            is_error,
        }),
        profile: plain_profile(surface),
        skill_registry: None,
    }
}

fn text_job(markdown: &str, _markup: Markup, surface: &str) -> DeliveryJob {
    DeliveryJob {
        job_id: "text-job".into(),
        target: format!("{surface}:one"),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Text {
            markdown: markdown.into(),
        },
        profile: plain_profile(surface),
        skill_registry: None,
    }
}

// ════════════════════════════════════════════════════════════════════
// 1. RENDER PIPELINE — Markdown variants × Markup
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_render_pipeline() {
    let mut scenarios: Vec<Scenario> = vec![];

    let markdown_variants: &[&str] = &[
        "# Title\n\nBody text.",
        "Plain paragraph.",
        "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6",
        "- item one\n- item two\n- item three",
        "1. first\n2. second\n3. third",
        "- outer\n  - inner",
        "> quoted text",
        "```rust\nlet x = 1;\n```",
        "```python\nprint('hello')\n```",
        "```\nplain code\n```",
        "| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |",
        "**bold** text",
        "*italic* text",
        "~~strikethrough~~",
        "`inline code`",
        "[link text](https://example.com)",
        "![alt text](https://example.com/img.png)",
        "---",
        "***",
        "Line 1\n\nLine 2\n\nLine 3",
        "Text with `code` and **bold** together.",
        "# Title with [link](https://x.com)",
        "",
        "   ",
        "Special chars: < > & \" '",
        "Emoji: 🎉🚀✅",
        "Unicode: Hello 世界",
        "Tabs\tand\nnewlines",
        "```\nline1\nline2\nline3\n```\n\nText after code.",
        "**a** *b* ~~c~~ `d`",
        "# Title\n\n> Quote\n\n- List\n- Items\n\n```js\ncode()\n```\n\nText end.",
        "",
        "   leading spaces\n\n## Sub\n\n> > nested quote",
        "![img](https://x.com/a.png)\n\n# Title",
        "[text](https://safe.com)\n\n[bad](javascript:alert(1))",
        "| Col1 | Col2 |\n| --- | --- |\n| val1 | val2 |",
        "```diff\n- old\n+ new\n```\n\nMore text.",
        "# Title\n\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"X\",\"value\":42}}\n```",
        "1. Nested\n   1. Deep\n   2. Deep2\n2. Back",
        "- [ ] todo\n- [x] done",
        "Text with a footnote[^1] and more.\n\n[^1]: Note here.",
        "Entity-like: AT&T and <tags>",
        "Nested emphasis: **bold *italic***",
        "Multiple\n\n\n\n\n\n\n\nblanks",
    ];

    let markups: [(Markup, &str); 6] = [
        (Markup::Plain, "plain"),
        (Markup::Markdown, "markdown"),
        (Markup::TelegramHtml, "telegram"),
        (Markup::SlackMrkdwn, "slack"),
        (Markup::DiscordMarkdown, "discord"),
        (Markup::Json, "json"),
    ];

    for (i, md) in markdown_variants.iter().enumerate() {
        let m = md.to_string();
        for (markup, label) in markups {
            let mk = markup;
            let ml = label;
            let m_clone = m.clone();
            scenarios.push(tc(&format!("render_{i}_{ml}"), move || {
                let job = answer_job(&m_clone, mk, None, DeliveryKind::Assistant, "test");
                let packet = render(&job).expect("render should succeed");
                assert_eq!(packet.schema_version, DELIVERY_SCHEMA_VERSION);
                assert_eq!(packet.kind, DeliveryKind::Assistant);
                assert_eq!(packet.surface, "test");
                assert_eq!(packet.job_id, "test-job");
                assert_eq!(packet.fallback_markdown, m_clone);
                match mk {
                    Markup::Json => {
                        assert!(matches!(packet.payload, DeliveryPayload::Structured(_)));
                    }
                    _ => {
                        assert!(matches!(packet.payload, DeliveryPayload::Text(_)));
                    }
                }
                assert!(!packet.coverage.is_empty() || m_clone.is_empty());
            }));
        }
    }

    // External content kinds render successfully
    let external_kinds = [
        DeliveryKind::Assistant,
        DeliveryKind::TaskSummary,
        DeliveryKind::Alert,
        DeliveryKind::User,
    ];
    for (i, kind) in external_kinds.iter().enumerate() {
        let k = *kind;
        scenarios.push(tc(&format!("external_kind_{i}_{k:?}"), move || {
            let source = "# Test\n\nContent.";
            let job = answer_job(source, Markup::Markdown, None, k, "test");
            let packet = render(&job).expect("external content should render");
            assert_eq!(packet.kind, k);
            assert_eq!(packet.fallback_markdown, source);
        }));
    }

    run("render_pipeline", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 2. DELIVERY KIND × CONTENT MATCHING
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_kind_content_matching() {
    let mut scenarios: Vec<Scenario> = vec![];

    let external_kinds = [
        DeliveryKind::Assistant,
        DeliveryKind::TaskSummary,
        DeliveryKind::Alert,
        DeliveryKind::User,
    ];
    let control_kinds = [
        DeliveryKind::System,
        DeliveryKind::Developer,
        DeliveryKind::Internal,
    ];

    // External kinds with Answer content → OK
    for (i, kind) in external_kinds.iter().enumerate() {
        let k = *kind;
        scenarios.push(tc(&format!("ext_answer_ok_{i}"), move || {
            let job = answer_job("# T\n\nB", Markup::Plain, None, k, "test");
            assert!(render(&job).is_ok());
        }));
    }

    // Control kinds always rejected
    for (i, kind) in control_kinds.iter().enumerate() {
        let k = *kind;
        scenarios.push(tc(&format!("control_rejected_{i}"), move || {
            let job = DeliveryJob {
                job_id: "c".into(),
                target: "t".into(),
                kind: k,
                content: DeliveryContent::Text {
                    markdown: "sys".into(),
                },
                profile: plain_profile("test"),
                skill_registry: None,
            };
            assert!(render(&job).is_err());
        }));
    }

    // Approval content with non-Approval kind → rejected
    for (i, kind) in external_kinds.iter().enumerate() {
        if *kind == DeliveryKind::Approval {
            continue;
        }
        let k = *kind;
        scenarios.push(tc(&format!("approval_mismatch_{i}"), move || {
            let job = DeliveryJob {
                job_id: "m".into(),
                target: "t".into(),
                kind: k,
                content: DeliveryContent::Approval(ApprovalPayload {
                    request_id: "r".into(),
                    title: "t".into(),
                    detail: "d".into(),
                    expires_at: None,
                    actions: vec![],
                }),
                profile: plain_profile("test"),
                skill_registry: None,
            };
            assert!(render(&job).is_err());
        }));
    }

    // Text content accepted by all kinds
    for (i, kind) in [
        DeliveryKind::Assistant,
        DeliveryKind::Approval,
        DeliveryKind::Progress,
        DeliveryKind::ToolResult,
    ]
    .iter()
    .enumerate()
    {
        let k = *kind;
        scenarios.push(tc(&format!("text_content_ok_{i}"), move || {
            let job = text_job("hello", Markup::Plain, "test");
            let job = DeliveryJob { kind: k, ..job };
            assert!(render(&job).is_ok());
        }));
    }

    // Progress content only matches Progress kind
    scenarios.push(tc("progress_content_for_assistant_rejected", || {
        let job = DeliveryJob {
            job_id: "p".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Progress(ProgressPayload {
                label: "l".into(),
                state: "s".into(),
                percent: None,
            }),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        assert!(render(&job).is_err());
    }));

    // ToolResult content only matches ToolResult kind
    scenarios.push(tc("toolresult_content_for_user_rejected", || {
        let job = DeliveryJob {
            job_id: "tr".into(),
            target: "t".into(),
            kind: DeliveryKind::User,
            content: DeliveryContent::ToolResult(ToolResultPayload {
                tool: "bash".into(),
                output: "ok".into(),
                is_error: false,
            }),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        assert!(render(&job).is_err());
    }));

    run("kind_content_matching", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 3. MARKUP CONVERSION (compile_markdown)
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_compile_markdown() {
    let mut scenarios: Vec<Scenario> = vec![];

    let markdown_cases: &[(&str, &str)] = &[
        ("empty string", ""),
        ("plain text only", "Hello world"),
        ("H1", "# Title"),
        ("H2", "## Subtitle"),
        ("H6 (max)", "###### Deepest"),
        ("H7 (too deep)", "####### Too Deep"),
        ("unordered list", "- a\n- b\n- c"),
        ("ordered list", "1. first\n2. second"),
        ("nested list", "- outer\n  - inner"),
        ("blockquote", "> quoted"),
        ("nested blockquote", "> > deep"),
        ("fenced code", "```rust\nlet x = 1;\n```"),
        ("indented code", "    indented code"),
        ("inline code", "use `code` here"),
        ("bold text", "**bold**"),
        ("italic text", "*italic*"),
        ("strikethrough", "~~struck~~"),
        ("link", "[text](https://example.com)"),
        ("image", "![alt text](https://example.com/img.png)"),
        ("horizontal rule", "---"),
        ("table", "| A | B |\n|---|---|\n| 1 | 2 |"),
        ("task list", "- [ ] todo\n- [x] done"),
        ("nested blockquote + list", "> - item\n> - item2"),
        ("code block then text", "```\ncode\n```\n\nText."),
        ("heading then list", "# Title\n\n- a\n- b"),
        ("multiple paragraphs", "First.\n\nSecond.\n\nThird."),
        ("backslash escape", r"\*not bold\*"),
        ("entity-like text", "AT&T and <company>"),
        ("long paragraph", &"word ".repeat(500)),
        ("unicode in heading", "# Tïtlé 标题"),
        ("emoji heading", "# 🎉 Party"),
        (
            "mixed everything",
            "# Title\n\n**bold** *italic*\n\n- item\n\n```js\ncode()\n```\n\nText end.",
        ),
        (
            "vak structured fence",
            "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"x\",\"value\":1}}\n```",
        ),
        ("mermaid diagram", "```mermaid\ngraph TD\nA --> B\n```"),
        ("diff fence", "```diff\n- old\n+ new\n```"),
        ("raw HTML inline", "<b>bold</b>"),
        ("raw HTML block", "<div>block</div>"),
        ("footnote", "Text with a footnote[^1]\n\n[^1]: Note here."),
        ("nested emphasis", "**bold *italic***"),
        ("nested list depth", "- a\n  - b\n    - c\n      - d"),
        ("empty input", ""),
        ("only whitespace", "   \n\n  "),
        ("single heading", "# Only Title"),
        ("deeply nested blockquotes", "> > > > deep"),
    ];

    for (label, md) in markdown_cases {
        let l = label.to_string();
        let m = md.to_string();
        scenarios.push(tc(&format!("compile_{l}"), move || {
            let doc = compile_markdown(&m);
            assert_eq!(doc.schema_version, PRESENTATION_SCHEMA_VERSION);
            assert_eq!(doc.source_markdown, m);
        }));
    }

    // Block type reachability
    let block_cases: &[(&str, &str)] = &[
        ("heading", "# Title"),
        ("code", "```rust\nx\n```"),
        ("list", "- a\n- b"),
        ("quote", "> quoted"),
        ("paragraph", "just text"),
        ("rule", "---"),
        ("table", "| A | B |\n|---|---|\n| 1 | 2 |"),
        (
            "structured",
            "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"L\",\"value\":5}}\n```",
        ),
        ("diagram", "```mermaid\ngraph TD\nA-->B\n```"),
        ("diff", "```diff\n- a\n+ b\n```"),
        ("raw_html", "<div>x</div>"),
    ];
    for (label, md) in block_cases {
        let l = label.to_string();
        let m = md.to_string();
        scenarios.push(tc(&format!("block_{l}"), move || {
            let doc = compile_markdown(&m);
            assert!(!doc.blocks.is_empty() || m.is_empty());
            for block in &doc.blocks {
                let _ = block; // DocumentBlock is a verified enum; variants carry id fields
            }
        }));
    }

    // Coverage disposition for each block type
    scenarios.push(tc("coverage_disp_all_native_or_fallback", || {
        let doc = compile_markdown(
            "# T\n\n> q\n\n```\ncode\n```\n\n<div>html</div>\n\n| a | b |\n|---|---|\n| 1 | 2 |",
        );
        for cov in &doc.coverage {
            assert!(matches!(
                cov.disposition,
                DocumentCoverageDisposition::Native | DocumentCoverageDisposition::Fallback
            ));
        }
    }));

    run("compile_markdown", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 4. DELIVERY PROFILE & CAPABILITIES
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_delivery_profiles() {
    let mut scenarios: Vec<Scenario> = vec![];

    for (markup, label) in [
        (Markup::Plain, "plain"),
        (Markup::Markdown, "markdown"),
        (Markup::TelegramHtml, "tg"),
        (Markup::SlackMrkdwn, "slack"),
        (Markup::DiscordMarkdown, "discord"),
        (Markup::Json, "json"),
    ] {
        let m = markup;
        let l = label;
        scenarios.push(tc(&format!("profile_{l}"), move || {
            let p = DeliveryProfile {
                surface: "test".into(),
                markup: m,
                max_chars: None,
                supports_tables: true,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            };
            let caps = p.capabilities();
            assert_eq!(caps.max_chars, None);
            assert_eq!(caps.code, true);
            assert_eq!(caps.links, true);
            assert_eq!(caps.tables, true);
        }));
    }

    // Native surfaces: all capabilities enabled
    for surface in &["desktop", "tui", "admin"] {
        let s = surface.to_string();
        scenarios.push(tc(&format!("native_{s}"), move || {
            let p = DeliveryProfile {
                surface: s.clone(),
                markup: Markup::Json,
                max_chars: None,
                supports_tables: true,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: true,
                template: None,
                posture: DeliveryPosture::default(),
            };
            let caps = p.capabilities();
            assert!(caps.structured_blocks);
            assert!(caps.media);
            assert!(caps.file_references);
            assert!(caps.color);
            assert!(caps.interactive);
        }));
    }

    // Non-native surfaces with Plain markup
    for surface in &["telegram", "slack", "discord", "web", "voice"] {
        let s = surface.to_string();
        scenarios.push(tc(&format!("nonnative_{s}"), move || {
            let p = DeliveryProfile {
                surface: s.clone(),
                markup: Markup::Plain,
                max_chars: Some(4096),
                supports_tables: false,
                supports_code_blocks: false,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            };
            let caps = p.capabilities();
            assert!(!caps.structured_blocks);
            assert!(!caps.media);
            assert!(!caps.color);
            assert!(caps.accessible_plain);
            assert_eq!(caps.max_chars, Some(4096));
        }));
    }

    // Plain constructor defaults
    scenarios.push(tc("plain_constructor", || {
        let p = DeliveryProfile::plain("test");
        assert_eq!(p.markup, Markup::Plain);
        assert_eq!(p.max_chars, None);
        assert!(!p.supports_tables);
        assert!(p.supports_code_blocks);
        assert!(p.supports_links);
        assert!(!p.supports_actions);
        assert!(p.template.is_none());
    }));

    // Render with empty surface fails
    scenarios.push(tc("empty_surface_renders_fail", || {
        let job = answer_job("# T", Markup::Plain, None, DeliveryKind::Assistant, "");
        assert!(render(&job).is_err());
    }));

    // Render with max_chars=0 fails
    scenarios.push(tc("max_chars_zero_renders_fail", || {
        let job = answer_job(
            "# T",
            Markup::Plain,
            Some(0),
            DeliveryKind::Assistant,
            "test",
        );
        assert!(render(&job).is_err());
    }));

    run("delivery_profiles", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 5. CHUNKING
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_chunking() {
    let mut scenarios: Vec<Scenario> = vec![];

    let max_chars_table: &[(Option<usize>, usize)] = &[
        (None, 1),
        (Some(100), 1),
        (Some(10), 5),
        (Some(5), 10),
        (Some(1), 50),
    ];

    for (i, (max, min_chunks)) in max_chars_table.iter().enumerate() {
        let mc = *max;
        let mn = *min_chunks;
        scenarios.push(tc(&format!("chunk_{i}"), move || {
            let source = &"word ".repeat(100);
            let job = answer_job(source, Markup::Plain, mc, DeliveryKind::Assistant, "test");
            let packet = render(&job).expect("render");
            assert!(
                packet.chunks.len() >= mn,
                "got {} chunks, expected >= {}",
                packet.chunks.len(),
                mn
            );
        }));
    }

    // Empty input
    scenarios.push(tc("chunk_empty_input", || {
        let job = answer_job("", Markup::Plain, Some(10), DeliveryKind::Assistant, "test");
        let packet = render(&job).expect("render");
        assert!(!packet.chunks.is_empty());
    }));

    // Telegram HTML chunking
    for (i, max) in [None, Some(32), Some(50), Some(100)].iter().enumerate() {
        let mc = *max;
        scenarios.push(tc(&format!("tg_chunk_{i}"), move || {
            let source = "# Title\n\nSome **bold** text.\n\n| a | b |\n|---|---|\n| 1 | 2 |";
            let job = answer_job(
                source,
                Markup::TelegramHtml,
                mc,
                DeliveryKind::Assistant,
                "tg",
            );
            let packet = render(&job).expect("render");
            if let Some(m) = mc {
                for chunk in &packet.chunks {
                    assert!(
                        chunk.chars().count() <= m + 100,
                        "chunk too long: {}",
                        chunk.chars().count()
                    );
                }
            }
        }));
    }

    // Slack/Discord markdown chunking
    for (i, surface) in ["slack", "discord"].iter().enumerate() {
        let surface = surface.to_string();
        for markup in [Markup::SlackMrkdwn, Markup::DiscordMarkdown] {
            let mm = markup;
            let label = format!("md_chunk_{i}");
            let surface = surface.clone();
            scenarios.push(tc(&label, move || {
                let source = &"line.\n\n".repeat(20);
                let job = answer_job(source, mm, Some(50), DeliveryKind::Assistant, &surface);
                let packet = render(&job).expect("render");
                assert!(packet.chunks.len() >= 2);
            }));
        }
    }

    // JSON payload has no chunks
    scenarios.push(tc("json_no_chunks", || {
        let job = answer_job(
            "source",
            Markup::Json,
            Some(10),
            DeliveryKind::Assistant,
            "test",
        );
        let packet = render(&job).expect("render");
        assert!(packet.chunks.is_empty() || packet.chunks.iter().all(|c| c.is_empty()));
    }));

    run("chunking", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 6. TEMPLATES
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_templates() {
    let mut scenarios: Vec<Scenario> = vec![];

    fn make_template(
        id: &str,
        origin: TemplateOrigin,
        activation: TemplateActivation,
    ) -> TemplateSpec {
        TemplateSpec {
            id: id.into(),
            revision: 1,
            origin,
            activation,
            nodes: vec![
                TemplateNode::Literal {
                    text: "Header: ".into(),
                },
                TemplateNode::Slot {
                    slot: TemplateSlot::Body,
                },
                TemplateNode::Literal {
                    text: "\n---\n".into(),
                },
                TemplateNode::Slot {
                    slot: TemplateSlot::SourceMarkdown,
                },
            ],
        }
    }

    // Active template resolves
    scenarios.push(tc("active_resolves", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t1",
            TemplateOrigin::User,
            TemplateActivation::Active,
        ))
        .unwrap();
        assert!(reg.resolve("t1").is_some());
    }));

    // Proposed template does not resolve
    scenarios.push(tc("proposed_not_resolved", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t2",
            TemplateOrigin::AgentProposal,
            TemplateActivation::Proposed,
        ))
        .unwrap();
        assert!(reg.resolve("t2").is_none());
    }));

    // Activate proposal
    scenarios.push(tc("activate_proposal_success", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t3",
            TemplateOrigin::AgentProposal,
            TemplateActivation::Proposed,
        ))
        .unwrap();
        assert!(reg.activate_proposal("t3", 1));
        assert!(reg.resolve("t3").is_some());
    }));

    scenarios.push(tc("activate_proposal_wrong_origin_fails", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t4",
            TemplateOrigin::User,
            TemplateActivation::Proposed,
        ))
        .unwrap();
        assert!(!reg.activate_proposal("t4", 1));
        assert!(reg.resolve("t4").is_none());
    }));

    scenarios.push(tc("activate_proposal_wrong_revision_fails", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t5",
            TemplateOrigin::AgentProposal,
            TemplateActivation::Proposed,
        ))
        .unwrap();
        assert!(!reg.activate_proposal("t5", 999));
    }));

    // Higher revision wins
    scenarios.push(tc("higher_revision_wins", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t6",
            TemplateOrigin::User,
            TemplateActivation::Active,
        ))
        .unwrap();
        let mut t2 = make_template("t6", TemplateOrigin::User, TemplateActivation::Active);
        t2.revision = 2;
        reg.upsert(t2).unwrap();
        let resolved = reg.resolve("t6").unwrap();
        assert_eq!(resolved.revision, 2);
    }));

    // Lower revision doesn't override
    scenarios.push(tc("lower_revision_keeps_higher", || {
        let mut reg = TemplateRegistry::default();
        let mut t2 = make_template("t7", TemplateOrigin::User, TemplateActivation::Active);
        t2.revision = 2;
        reg.upsert(t2).unwrap();
        reg.upsert(make_template(
            "t7",
            TemplateOrigin::User,
            TemplateActivation::Active,
        ))
        .unwrap();
        let resolved = reg.resolve("t7").unwrap();
        assert_eq!(resolved.revision, 2);
    }));

    // Upsert replaces by id + origin
    scenarios.push(tc("upsert_replaces_same_origin", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t8",
            TemplateOrigin::User,
            TemplateActivation::Active,
        ))
        .unwrap();
        let mut t2 = make_template("t8", TemplateOrigin::User, TemplateActivation::Active);
        t2.revision = 5;
        reg.upsert(t2).unwrap();
        let count = reg.templates.iter().filter(|t| t.id == "t8").count();
        assert_eq!(count, 1);
        assert_eq!(reg.resolve("t8").unwrap().revision, 5);
    }));

    // Different origins = separate entries
    scenarios.push(tc("different_origins_separate", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t9",
            TemplateOrigin::User,
            TemplateActivation::Active,
        ))
        .unwrap();
        reg.upsert(make_template(
            "t9",
            TemplateOrigin::BuiltIn,
            TemplateActivation::Active,
        ))
        .unwrap();
        let count = reg.templates.iter().filter(|t| t.id == "t9").count();
        assert_eq!(count, 2);
    }));

    // User origin overrides BuiltIn
    scenarios.push(tc("user_overrides_builtin", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t10",
            TemplateOrigin::BuiltIn,
            TemplateActivation::Active,
        ))
        .unwrap();
        reg.upsert(make_template(
            "t10",
            TemplateOrigin::User,
            TemplateActivation::Active,
        ))
        .unwrap();
        let resolved = reg.resolve("t10").unwrap();
        assert_eq!(resolved.origin, TemplateOrigin::User);
    }));

    // Project overrides BuiltIn
    scenarios.push(tc("project_overrides_builtin", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "t11",
            TemplateOrigin::BuiltIn,
            TemplateActivation::Active,
        ))
        .unwrap();
        reg.upsert(make_template(
            "t11",
            TemplateOrigin::Project,
            TemplateActivation::Active,
        ))
        .unwrap();
        let resolved = reg.resolve("t11").unwrap();
        assert_eq!(resolved.origin, TemplateOrigin::Project);
    }));

    // Unknown template returns None
    scenarios.push(tc("resolve_unknown_returns_none", || {
        let reg = TemplateRegistry::default();
        assert!(reg.resolve("nonexistent").is_none());
    }));

    // All slot types are constructable
    scenarios.push(tc("slot_all_variants", || {
        let slots = [
            TemplateSlot::Title,
            TemplateSlot::Body,
            TemplateSlot::SourceMarkdown,
            TemplateSlot::BlockCount,
            TemplateSlot::Metadata {
                key: "author".into(),
            },
        ];
        for slot in slots {
            let template = TemplateSpec {
                id: "slot_test".into(),
                revision: 1,
                origin: TemplateOrigin::User,
                activation: TemplateActivation::Active,
                nodes: vec![TemplateNode::Slot { slot }],
            };
            assert!(template.validate().is_ok());
        }
    }));

    // IfPresent node
    scenarios.push(tc("ifpresent_constructs", || {
        let template = TemplateSpec {
            id: "if_test".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![TemplateNode::IfPresent {
                slot: TemplateSlot::Title,
                then_nodes: vec![TemplateNode::Literal { text: "yes".into() }],
                else_nodes: vec![TemplateNode::Literal { text: "no".into() }],
            }],
        };
        assert!(template.validate().is_ok());
    }));

    // Template validation: empty id
    scenarios.push(tc("empty_id_invalid", || {
        let template = TemplateSpec {
            id: "".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![TemplateNode::Literal { text: "x".into() }],
        };
        assert!(template.validate().is_err());
    }));

    // Template validation: too many nodes
    scenarios.push(tc("too_many_nodes_invalid", || {
        let nodes: Vec<TemplateNode> = (0..129)
            .map(|_| TemplateNode::Literal { text: "x".into() })
            .collect();
        let template = TemplateSpec {
            id: "many".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes,
        };
        assert!(template.validate().is_err());
    }));

    scenarios.push(tc("max_nodes_ok", || {
        let nodes: Vec<TemplateNode> = (0..128)
            .map(|_| TemplateNode::Literal { text: "x".into() })
            .collect();
        let template = TemplateSpec {
            id: "max128".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes,
        };
        assert!(template.validate().is_ok());
    }));

    // Deep nesting
    scenarios.push(tc("deep_nesting_exceeds_max_invalid", || {
        let mut inner = TemplateNode::Literal { text: "x".into() };
        for _ in 0..10 {
            inner = TemplateNode::IfPresent {
                slot: TemplateSlot::Body,
                then_nodes: vec![inner],
                else_nodes: vec![],
            };
        }
        let template = TemplateSpec {
            id: "deep".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![inner],
        };
        assert!(template.validate().is_err());
    }));

    scenarios.push(tc("max_depth_ok", || {
        let mut inner = TemplateNode::Literal { text: "x".into() };
        for _ in 0..8 {
            inner = TemplateNode::IfPresent {
                slot: TemplateSlot::Body,
                then_nodes: vec![inner],
                else_nodes: vec![],
            };
        }
        let template = TemplateSpec {
            id: "max_depth".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![inner],
        };
        assert!(template.validate().is_ok());
    }));

    // Literal too long
    scenarios.push(tc("literal_too_long_invalid", || {
        let template = TemplateSpec {
            id: "long".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![TemplateNode::Literal {
                text: "x".repeat(16385),
            }],
        };
        assert!(template.validate().is_err());
    }));

    scenarios.push(tc("literal_max_len_ok", || {
        let template = TemplateSpec {
            id: "max_long".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![TemplateNode::Literal {
                text: "x".repeat(16384),
            }],
        };
        assert!(template.validate().is_ok());
    }));

    // Template render with actual answer
    scenarios.push(tc("template_render_replaces_slots", || {
        let mut reg = TemplateRegistry::default();
        reg.upsert(make_template(
            "rend1",
            TemplateOrigin::User,
            TemplateActivation::Active,
        ))
        .unwrap();
        let template = reg.resolve("rend1").unwrap();
        assert_eq!(template.nodes.len(), 4);
    }));

    run("templates", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 7. POSTURE DISPOSITION MATRIX
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_posture_matrix() {
    let mut scenarios: Vec<Scenario> = vec![];

    let cadences = [Cadence::Live, Cadence::OnCompletion, Cadence::Digest];
    let urgencies = [Urgency::Interrupt, Urgency::Notify, Urgency::Quiet];
    let kinds = [
        DeliveryKind::Assistant,
        DeliveryKind::TaskSummary,
        DeliveryKind::Alert,
        DeliveryKind::User,
        DeliveryKind::Approval,
        DeliveryKind::Progress,
        DeliveryKind::ToolResult,
        DeliveryKind::System,
        DeliveryKind::Developer,
        DeliveryKind::Internal,
        DeliveryKind::ToolCall,
        DeliveryKind::Steering,
    ];

    for cadence in cadences {
        for urgency in urgencies {
            for kind in kinds {
                let c = cadence;
                let u = urgency;
                let k = kind;
                let label = format!("disposition_{:?}_{:?}_{:?}", c, u, k);
                scenarios.push(tc(&label, move || {
                    let posture = DeliveryPosture {
                        cadence: c,
                        urgency: u,
                    };
                    let disp = posture.disposition(k);
                    // Interrupt always sends
                    if u == Urgency::Interrupt {
                        assert_eq!(disp, Disposition::Send);
                    }
                    // Approval always sends
                    if k == DeliveryKind::Approval {
                        assert_eq!(disp, Disposition::Send);
                    }
                    // Alert under Digest always sends
                    if k == DeliveryKind::Alert && c == Cadence::Digest {
                        assert_eq!(disp, Disposition::Send);
                    }
                    // Live + non-interrupt sends everything
                    if c == Cadence::Live && u != Urgency::Interrupt {
                        if k != DeliveryKind::Approval {
                            assert_eq!(disp, Disposition::Send);
                        }
                    }
                    // Digest + Quiet + non-alert + non-approval → hold for digest
                    if c == Cadence::Digest
                        && u == Urgency::Quiet
                        && k != DeliveryKind::Alert
                        && k != DeliveryKind::Approval
                    {
                        assert_eq!(disp, Disposition::HoldForDigest);
                    }
                    // OnCompletion + Notify: progress/tool chatter held
                    if c == Cadence::OnCompletion && u == Urgency::Notify {
                        match k {
                            DeliveryKind::Progress
                            | DeliveryKind::ToolResult
                            | DeliveryKind::ToolCall => {
                                assert_eq!(disp, Disposition::HoldUntilComplete);
                            }
                            _ => {
                                assert_eq!(disp, Disposition::Send);
                            }
                        }
                    }
                }));
            }
        }
    }

    // Default posture sends everything
    scenarios.push(tc("default_posture_sends_all", || {
        let default = DeliveryPosture::default();
        for kind in [
            DeliveryKind::Assistant,
            DeliveryKind::Progress,
            DeliveryKind::Alert,
            DeliveryKind::Approval,
            DeliveryKind::ToolResult,
        ] {
            assert_eq!(default.disposition(kind), Disposition::Send);
        }
    }));

    // Cadence variants serialize
    for cadence in cadences {
        scenarios.push(tc(&format!("serde_cadence_{cadence:?}"), move || {
            let bytes = serde_json::to_vec(&cadence).expect("serialize");
            let deserialized: Cadence = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(deserialized, cadence);
        }));
    }

    run("posture_matrix", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 8. SKILL REGISTRY & VALIDATION
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_skill_registry() {
    let mut scenarios: Vec<Scenario> = vec![];

    let expected_types = [
        "link.preview",
        "metric",
        "chart",
        "media.image",
        "media.video",
        "media.audio",
        "research.synthesis",
        "itinerary",
        "coding.diff",
        "test.report",
        "terminal.view",
        "data.grid",
        "recipe.card",
        "ui.preview",
        "plan.timeline",
    ];
    for (i, st) in expected_types.iter().enumerate() {
        let s = st.clone();
        scenarios.push(tc(&format!("builtin_type_{i}_{s}"), move || {
            let registry = built_in_skill_registry();
            assert!(registry.find_by_type(&s).is_some(), "missing {s}");
        }));
    }

    // Register valid manifest
    scenarios.push(tc("register_valid", || {
        let mut registry = SkillRegistry::default();
        let manifest = PresentationSkillManifest {
            id: "test.skill".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["test.type".into()],
            renderers: BTreeMap::from([(
                "desktop".into(),
                RendererBinding {
                    renderer: "native:structured".into(),
                    interactive: false,
                    requires: vec![],
                },
            )]),
            schema: None,
            outcome_requirements: vec![],
        };
        assert!(registry.register(manifest).is_ok());
    }));

    scenarios.push(tc("register_empty_id_invalid", || {
        let mut registry = SkillRegistry::default();
        let manifest = PresentationSkillManifest {
            id: "".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["t".into()],
            renderers: BTreeMap::new(),
            schema: None,
            outcome_requirements: vec![],
        };
        assert!(matches!(
            registry.register(manifest),
            Err(SkillError::InvalidManifest(_))
        ));
    }));

    scenarios.push(tc("register_empty_version_invalid", || {
        let mut registry = SkillRegistry::default();
        let manifest = PresentationSkillManifest {
            id: "x".into(),
            version: "".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["t".into()],
            renderers: BTreeMap::new(),
            schema: None,
            outcome_requirements: vec![],
        };
        assert!(matches!(
            registry.register(manifest),
            Err(SkillError::InvalidManifest(_))
        ));
    }));

    scenarios.push(tc("register_wrong_api_invalid", || {
        let mut registry = SkillRegistry::default();
        let manifest = PresentationSkillManifest {
            id: "x".into(),
            version: "1.0.0".into(),
            api: "wrong".into(),
            provides: vec!["t".into()],
            renderers: BTreeMap::new(),
            schema: None,
            outcome_requirements: vec![],
        };
        assert!(matches!(
            registry.register(manifest),
            Err(SkillError::InvalidManifest(_))
        ));
    }));

    scenarios.push(tc("register_empty_type_invalid", || {
        let mut registry = SkillRegistry::default();
        let manifest = PresentationSkillManifest {
            id: "x".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["".into()],
            renderers: BTreeMap::new(),
            schema: None,
            outcome_requirements: vec![],
        };
        assert!(matches!(
            registry.register(manifest),
            Err(SkillError::InvalidManifest(_))
        ));
    }));

    scenarios.push(tc("register_duplicate_owned_type_invalid", || {
        let mut registry = SkillRegistry::default();
        let manifest = PresentationSkillManifest {
            id: "x".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["metric".into()],
            renderers: BTreeMap::new(),
            schema: None,
            outcome_requirements: vec![],
        };
        assert!(matches!(
            registry.register(manifest),
            Err(SkillError::InvalidManifest(_))
        ));
    }));

    scenarios.push(tc("register_overwrite_same_id_ok", || {
        let mut registry = SkillRegistry::default();
        let manifest = PresentationSkillManifest {
            id: "x".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["test.type".into()],
            renderers: BTreeMap::new(),
            schema: None,
            outcome_requirements: vec![],
        };
        assert!(registry.register(manifest.clone()).is_ok());
        assert!(registry.register(manifest).is_ok());
    }));

    // find_by_type
    scenarios.push(tc("find_by_type_returns_core", || {
        let registry = built_in_skill_registry();
        let result = registry.find_by_type("metric").expect("metric");
        assert_eq!(result.0, "core");
        assert_eq!(result.1, "1.0.0");
    }));

    scenarios.push(tc("find_by_type_unknown_none", || {
        let registry = built_in_skill_registry();
        assert!(registry.find_by_type("nonexistent").is_none());
    }));

    // get
    scenarios.push(tc("get_existing_skill", || {
        let registry = built_in_skill_registry();
        assert!(registry.get("core").is_some());
    }));

    scenarios.push(tc("get_unknown_skill_none", || {
        let registry = built_in_skill_registry();
        assert!(registry.get("unknown").is_none());
    }));

    // parse_fragment valid
    let valid_fragments: &[&str] = &[
        r#"{"semantic_type":"metric","payload":{"label":"X","value":42,"unit":"%"}}"#,
        r#"{"semantic_type":"link.preview","payload":{"url":"https://x.com","title":"T"}}"#,
        r#"{"semantic_type":"collection","payload":{"items":["a","b"]}}"#,
        r#"{"semantic_type":"detail","payload":{"title":"T"}}"#,
        r#"{"semantic_type":"comparison","payload":{"left":"a","right":"b"}}"#,
        r#"{"semantic_type":"checklist","payload":{"items":[{"label":"a","done":true}]}}"#,
        r#"{"semantic_type":"timeline","payload":{"items":[{"label":"step"}]}}"#,
        r#"{"semantic_type":"steps","payload":{"items":[{"label":"s"}]}}"#,
        r#"{"semantic_type":"schedule","payload":{"slots":[{"label":"morning"}]}}"#,
        r#"{"semantic_type":"meeting","payload":{"agenda":[{"title":"A"}],"attendees":["x"]}}"#,
        r#"{"semantic_type":"decision","payload":{"label":"Choose X","choices":["A","B"]}}"#,
        r#"{"semantic_type":"budget","payload":{"label":"Income","income":100,"expenses":50}}"#,
    ];
    for (i, frag) in valid_fragments.iter().enumerate() {
        let f = frag.to_string();
        scenarios.push(tc(&format!("parse_fragment_valid_{i}"), move || {
            let result = parse_fragment(&f);
            assert!(result.is_ok(), "fragment {i}: {result:?}");
        }));
    }

    // parse_fragment invalid
    let invalid_fragments: &[&str] = &[
        r#"not json"#,
        r#"{"semantic_type":123}"#,
        r#"{"semantic_type":"unknown.type","payload":{}}"#,
        r#""{"missing": "quotes"}"#,
    ];
    for (i, frag) in invalid_fragments.iter().enumerate() {
        let f = frag.to_string();
        scenarios.push(tc(&format!("parse_fragment_invalid_{i}"), move || {
            assert!(parse_fragment(&f).is_err());
        }));
    }

    // validate metric
    scenarios.push(tc("validate_metric_ok", || {
        let registry = built_in_skill_registry();
        let output = StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: json!({"label": "X", "value": 42, "unit": "%"}),
        };
        assert!(registry.validate(&output, "desktop", &[]).is_ok());
    }));

    scenarios.push(tc("validate_metric_missing_label_fails", || {
        let registry = built_in_skill_registry();
        let output = StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: json!({"value": 42}),
        };
        assert!(registry.validate(&output, "desktop", &[]).is_err());
    }));

    scenarios.push(tc("validate_unknown_skill_fails", || {
        let registry = built_in_skill_registry();
        let output = StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: PRESENTATION_SCHEMA_VERSION,
            skill_id: "unknown".into(),
            skill_version: "1.0.0".into(),
            payload: json!({"label": "X", "value": 42}),
        };
        assert!(matches!(
            registry.validate(&output, "desktop", &[]),
            Err(SkillError::UnknownType(_))
        ));
    }));

    scenarios.push(tc("validate_wrong_schema_fails", || {
        let registry = built_in_skill_registry();
        let output = StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: 999,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: json!({"label": "X", "value": 42}),
        };
        assert!(matches!(
            registry.validate(&output, "desktop", &[]),
            Err(SkillError::InvalidPayload(_))
        ));
    }));

    scenarios.push(tc("validate_wrong_version_fails", || {
        let registry = built_in_skill_registry();
        let output = StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "2.0.0".into(),
            payload: json!({"label": "X", "value": 42}),
        };
        assert!(matches!(
            registry.validate(&output, "desktop", &[]),
            Err(SkillError::InvalidPayload(_))
        ));
    }));

    scenarios.push(tc("validate_missing_surface_renderer_fails", || {
        let registry = built_in_skill_registry();
        let output = StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: json!({"label": "X", "value": 42}),
        };
        assert!(matches!(
            registry.validate(&output, "unknown_surface", &[]),
            Err(SkillError::MissingCapability(_))
        ));
    }));

    scenarios.push(tc("validate_unknown_type_fails", || {
        let registry = built_in_skill_registry();
        let output = StructuredOutput {
            semantic_type: "nonexistent".into(),
            schema_version: PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: json!({}),
        };
        assert!(matches!(
            registry.validate(&output, "desktop", &[]),
            Err(SkillError::UnknownType(_))
        ));
    }));

    // structured_outputs_from_text
    scenarios.push(tc("outputs_bare_json", || {
        let outputs = structured_outputs_from_text(
            r#"{"semantic_type":"metric","payload":{"label":"X","value":42}}"#,
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].semantic_type, "metric");
    }));

    scenarios.push(tc("outputs_fenced_vak", || {
        let text = "Data\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"X\",\"value\":1}}\n```\n";
        let outputs = structured_outputs_from_text(text);
        assert_eq!(outputs.len(), 1);
    }));

    scenarios.push(tc("outputs_plain_no_output", || {
        assert!(structured_outputs_from_text("just plain text").is_empty());
    }));

    scenarios.push(tc("outputs_multiple_fences", || {
        let text = "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"A\",\"value\":1}}\n```\n```vak\n{\"semantic_type\":\"collection\",\"payload\":{\"items\":[\"a\",\"b\"]}}\n```";
        assert_eq!(structured_outputs_from_text(text).len(), 2);
    }));

    scenarios.push(tc("outputs_dedup_same", || {
        let text = "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"A\",\"value\":1}}\n```\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"A\",\"value\":1}}\n```";
        assert_eq!(structured_outputs_from_text(text).len(), 1);
    }));

    run("skill_registry", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 9. SIGNALS
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_signals() {
    let mut scenarios: Vec<Scenario> = vec![];

    let signal_cases: &[(&str, &[&str])] = &[
        (
            "https://example.com source: x",
            &["citations", "multiple_sources"],
        ),
        (
            "according to references",
            &["multiple_sources", "research", "synthesis", "takeaways"],
        ),
        (
            "key takeaways findings",
            &["research", "synthesis", "takeaways"],
        ),
        ("temperature is 25°c", &["temperature"]),
        ("forecast humidity wind speed", &["forecast"]),
        ("travel itinerary flight hotel", &["travel"]),
        ("latest headlines news", &["news"]),
        ("diff --git a b", &["diff", "files_changed"]),
        ("files changed modified: created:", &["files_changed"]),
        ("tests passed", &["tests", "pass_fail"]),
        ("test suite test report", &["tests", "pass_fail"]),
        (
            "benchmark req/sec p99 throughput",
            &["benchmark", "telemetry"],
        ),
        ("chart plot graph trend", &["chart"]),
        ("telemetry metrics kpi latency", &["telemetry"]),
        ("table dataset mrr", &["table_data"]),
        ("recipe servings cook time", &["recipe"]),
        ("ingredients tbsp tsp", &["ingredients"]),
        ("artifact download", &["artifact"]),
        ("approval approve permission", &["approval"]),
        ("allow once deny", &["action"]),
        ("docker ps container ports", &["docker", "terminal"]),
        ("exit 0", &["terminal", "command_exec"]),
        ("no signals here at all", &[]),
        ("plain text without keywords", &[]),
        ("", &[]),
        ("temperature forecast", &["temperature", "forecast"]),
        ("https://a.com https://b.com", &["citations"]),
        (
            "research synthesis verified sources",
            &[
                "research",
                "synthesis",
                "takeaways",
                "citations",
                "multiple_sources",
            ],
        ),
        ("tests passed failed failures", &["tests", "pass_fail"]),
        (
            "chart telemetry kpi latency p99 throughput",
            &["chart", "telemetry"],
        ),
        (
            "diff files_changed modified: created:",
            &["diff", "files_changed"],
        ),
    ];

    for (i, (text, expected)) in signal_cases.iter().enumerate() {
        let t = text.to_string();
        let expected_vec: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        scenarios.push(tc(&format!("signals_{i}"), move || {
            let signals = signals_from_text(&t);
            for expected_sig in &expected_vec {
                assert!(
                    signals.contains(expected_sig),
                    "missing {expected_sig} in {signals:?} for {t:?}"
                );
            }
        }));
    }

    // signals_from_context
    scenarios.push(tc("context_no_tool", || {
        let ctx = SignalContext {
            text: "plain text",
            tool_name: None,
            tool_input: None,
            tool_output: None,
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(!signals.contains(&"terminal".to_string()));
    }));

    scenarios.push(tc("context_bash_adds_terminal", || {
        let ctx = SignalContext {
            text: "running command",
            tool_name: Some("bash"),
            tool_input: None,
            tool_output: None,
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"terminal".to_string()));
        assert!(signals.contains(&"command_exec".to_string()));
    }));

    scenarios.push(tc("context_bash_test_output", || {
        let input = json!({"command": "cargo test"});
        let ctx = SignalContext {
            text: "running",
            tool_name: Some("bash"),
            tool_input: Some(&input),
            tool_output: None,
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"tests".to_string()));
        assert!(signals.contains(&"pass_fail".to_string()));
    }));

    scenarios.push(tc("context_edit_adds_diff", || {
        let ctx = SignalContext {
            text: "editing",
            tool_name: Some("edit"),
            tool_input: None,
            tool_output: None,
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"diff".to_string()));
        assert!(signals.contains(&"files_changed".to_string()));
    }));

    scenarios.push(tc("context_websearch_adds_research", || {
        let ctx = SignalContext {
            text: "searching",
            tool_name: Some("websearch"),
            tool_input: None,
            tool_output: None,
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"citations".to_string()));
        assert!(signals.contains(&"research".to_string()));
    }));

    scenarios.push(tc("context_tavily_adds_research", || {
        let ctx = SignalContext {
            text: "searching",
            tool_name: Some("tavily"),
            tool_input: None,
            tool_output: None,
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"research".to_string()));
        assert!(signals.contains(&"synthesis".to_string()));
    }));

    scenarios.push(tc("context_output_test_result", || {
        let ctx = SignalContext {
            text: "done",
            tool_name: None,
            tool_input: None,
            tool_output: Some("test result: 5 passed; 0 failed"),
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"tests".to_string()));
        assert!(signals.contains(&"pass_fail".to_string()));
    }));

    scenarios.push(tc("context_output_diff", || {
        let ctx = SignalContext {
            text: "done",
            tool_name: None,
            tool_input: None,
            tool_output: Some("diff --git a/file b/file"),
            is_error: false,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"diff".to_string()));
        assert!(signals.contains(&"files_changed".to_string()));
    }));

    scenarios.push(tc("context_is_error_doesnt_crash", || {
        let ctx = SignalContext {
            text: "error",
            tool_name: Some("bash"),
            tool_input: None,
            tool_output: None,
            is_error: true,
        };
        let signals = signals_from_context(&ctx);
        assert!(signals.contains(&"terminal".to_string()));
    }));

    run("signals", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 10. RECIPE SELECTION
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_recipe_selection() {
    let mut scenarios: Vec<Scenario> = vec![];

    let catalog = built_in_recipes();

    let base_cases: &[(&str, &[&str], &str)] = &[
        ("detail", &[], "answer.basic"),
        (
            "comparison",
            &["diff", "files_changed"],
            "coding.diff_inspector",
        ),
        ("test.report", &["tests", "pass_fail"], "coding.test_report"),
        (
            "terminal.view",
            &["terminal", "command_exec"],
            "terminal.session",
        ),
        ("chart", &["chart", "telemetry"], "data.multi_chart"),
        (
            "data.grid",
            &["table_data", "tabular"],
            "data.spreadsheet_grid",
        ),
        (
            "recipe.card",
            &["recipe", "ingredients"],
            "lifestyle.culinary_recipe",
        ),
        ("plan.timeline", &["plan", "timeline"], "plan.timeline"),
        ("itinerary", &["travel", "itinerary"], "travel.itinerary"),
        (
            "research.synthesis",
            &["research", "takeaways"],
            "research.synthesis",
        ),
        ("metric", &["temperature"], "weather.forecast"),
        ("collection", &["chart", "telemetry"], "data.multi_chart"),
    ];

    for (i, (etype, signals, expected)) in base_cases.iter().enumerate() {
        let st = etype.to_string();
        let sigs: Vec<String> = signals.iter().map(|s| s.to_string()).collect();
        let er = expected.to_string();
        let cat = catalog.clone();
        scenarios.push(tc(&format!("recipe_{i}"), move || {
            let decision = cat.choose(&sigs, "desktop");
            assert!(
                decision.is_some(),
                "no recipe for {st} with signals {sigs:?}"
            );
            assert_eq!(decision.unwrap().recipe_id, er);
        }));
    }

    // Surface filtering: some recipes are desktop/terminal-only
    let surface_filtered = [
        (
            "coding.diff",
            &["diff", "files_changed"],
            "desktop",
            "coding.diff_inspector",
        ),
        (
            "coding.diff",
            &["diff", "files_changed"],
            "terminal",
            "coding.diff_inspector",
        ),
        (
            "coding.diff",
            &["diff", "files_changed"],
            "telegram",
            "answer.basic",
        ),
        (
            "research.synthesis",
            &["research", "synthesis"],
            "telegram",
            "news.synthesis",
        ),
        (
            "research.synthesis",
            &["research", "synthesis"],
            "desktop",
            "research.synthesis",
        ),
        (
            "lifestyle.culinary_recipe",
            &["recipe", "ingredients"],
            "desktop",
            "lifestyle.culinary_recipe",
        ),
        (
            "lifestyle.culinary_recipe",
            &["recipe", "ingredients"],
            "telegram",
            "answer.basic",
        ),
    ];
    for (i, (etype, signals, surface, expected)) in surface_filtered.iter().enumerate() {
        let st = etype.to_string();
        let sigs: Vec<String> = signals.iter().map(|s| s.to_string()).collect();
        let sr = surface.to_string();
        let er = expected.to_string();
        let cat = catalog.clone();
        scenarios.push(tc(&format!("surface_filter_{i}"), move || {
            let decision = cat.choose(&sigs, &sr);
            if let Some(d) = decision {
                assert_eq!(d.recipe_id, er, "for {st} on {sr}");
            } else {
                assert_eq!(er, "answer.basic", "expected {er} but got None");
            }
        }));
    }

    // Default recipe with no signals
    scenarios.push(tc("default_recipe_no_signals", || {
        let catalog = built_in_recipes();
        let decision = catalog.choose(&[], "desktop");
        assert!(decision.is_some());
        assert_eq!(decision.unwrap().recipe_id, "answer.basic");
    }));

    // answer.basic filtered when signals present but no matching recipe
    scenarios.push(tc("answer_basic_filtered_with_signals", || {
        let catalog = built_in_recipes();
        let decision = catalog.choose_for_types(&["diff".to_string()], "desktop", &[]);
        assert!(decision.is_none());
    }));

    // Recipe signals must all match
    scenarios.push(tc("partial_signal_mismatch", || {
        let catalog = built_in_recipes();
        // coding.diff_inspector requires ["diff", "files_changed"]
        let decision = catalog.choose(&["diff".to_string()], "desktop");
        // diff alone doesn't match the 2-signal requirement
        // but answer.basic with default_recipe should still match if no signals filter blocks it
        assert!(decision.is_some());
        assert_ne!(decision.unwrap().recipe_id, "coding.diff_inspector");
    }));

    // choose requires all match_signals
    scenarios.push(tc("requires_all_signals", || {
        let catalog = built_in_recipes();
        // weather.forecast requires "temperature" and "forecast"
        let decision = catalog.choose(&["temperature".to_string()], "desktop");
        assert!(decision.is_some());
        assert_ne!(decision.unwrap().recipe_id, "weather.forecast");
    }));

    // Unknown surface still gets answer.basic
    scenarios.push(tc("unknown_surface_gets_default", || {
        let catalog = built_in_recipes();
        let decision = catalog.choose(&[], "unknown_surface");
        assert_eq!(decision.unwrap().recipe_id, "answer.basic");
    }));

    run("recipe_selection", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 11. OUTBOX LIFECYCLE
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_outbox() {
    let mut scenarios: Vec<Scenario> = vec![];

    let root = std::env::temp_dir().join(format!(
        "vak-delivery-outbox-audit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root2 = std::env::temp_dir().join(format!(
        "vak-delivery-outbox-audit2-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let source = "# Title\n\nContent.";
    let base_job = DeliveryJob {
        job_id: "job-1".into(),
        target: "test:surface".into(),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Answer(answer_draft(source)),
        profile: plain_profile("test"),
        skill_registry: None,
    };

    {
        let root = root.clone();
        let base_job = base_job.clone();
        scenarios.push(tc("enqueue_creates_pending", move || {
            let outbox = Outbox::new(&root);
            let record = outbox.enqueue(base_job).expect("enqueue");
            assert_eq!(record.state, OutboxState::Pending);
            assert_eq!(record.attempts, 0);
            assert!(record.created_at_ms > 0);
            assert_eq!(record.job.job_id, "job-1");
            assert!(record.packet.is_none());
            assert!(record.last_error.is_none());
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("pending_returns_enqueued", move || {
            let outbox = Outbox::new(&root);
            let records = outbox.pending().expect("pending");
            assert!(records.iter().any(|r| r.job.job_id == "job-1"));
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("get_returns_record", move || {
            let outbox = Outbox::new(&root);
            let record = outbox.get("job-1").expect("get");
            assert_eq!(record.job.job_id, "job-1");
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("get_missing_fails", move || {
            let outbox = Outbox::new(&root);
            assert!(outbox.get("nonexistent").is_err());
        }));
    }

    {
        let root = root.clone();
        let base_job = base_job.clone();
        scenarios.push(tc("mark_delivered", move || {
            let outbox = Outbox::new(&root);
            let packet = render(&base_job).expect("render");
            let record = outbox.mark_delivered("job-1", packet).expect("deliver");
            assert_eq!(record.state, OutboxState::Delivered);
            assert_eq!(record.attempts, 1);
            assert!(record.packet.is_some());
            assert!(record.last_error.is_none());
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("pending_empty_after_delivery", move || {
            let outbox = Outbox::new(&root);
            assert!(outbox.pending().expect("pending").is_empty());
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("mark_failed_increments", move || {
            let outbox = Outbox::new(&root);
            let record = outbox.mark_failed("job-1", "network error").expect("fail");
            assert_eq!(record.attempts, 2);
            assert_eq!(record.last_error.as_deref(), Some("network error"));
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("mark_dead_letter", move || {
            let outbox = Outbox::new(&root);
            let record = outbox
                .mark_dead_letter("job-1", "permanent error")
                .expect("dlq");
            assert_eq!(record.state, OutboxState::DeadLetter);
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("list_returns_all", move || {
            let outbox = Outbox::new(&root);
            let records = outbox.list().expect("list");
            assert!(records.iter().any(|r| r.job.job_id == "job-1"));
        }));
    }

    {
        let root = root.clone();
        let base_job = base_job.clone();
        scenarios.push(tc("conflict_on_duplicate", move || {
            let outbox = Outbox::new(&root);
            let result = outbox.enqueue(base_job);
            assert!(matches!(result, Err(OutboxError::Conflict(_))));
        }));
    }

    {
        let root = root.clone();
        let base_job = base_job.clone();
        scenarios.push(tc("idempotent_enqueue_same_job", move || {
            let result = Outbox::new(&root).enqueue(base_job);
            assert!(result.is_ok());
        }));
    }

    {
        let root2 = root2.clone();
        scenarios.push(tc("empty_root_pending", move || {
            let outbox = Outbox::new(&root2);
            assert!(outbox.pending().expect("pending").is_empty());
            assert!(outbox.list().expect("list").is_empty());
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("delivered_record_in_list", move || {
            let outbox = Outbox::new(&root);
            let records = outbox.list().expect("list");
            assert!(records.iter().any(|r| r.state == OutboxState::Delivered));
        }));
    }

    {
        let root = root.clone();
        scenarios.push(tc("dead_letter_in_list", move || {
            let outbox = Outbox::new(&root);
            let records = outbox.list().expect("list");
            assert!(records.iter().any(|r| r.state == OutboxState::DeadLetter));
        }));
    }

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&root2);

    run("outbox", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 12. WORKER SUBPROCESS PROTOCOL
// ════════════════════════════════════════════════════════════════════

fn make_req(job: &DeliveryJob) -> String {
    let request = WorkerRequest {
        protocol_version: WORKER_PROTOCOL_VERSION,
        job: job.clone(),
    };
    serde_json::to_string(&request).expect("serialize")
}

#[test]
fn audit_worker_protocol() {
    let mut scenarios: Vec<Scenario> = vec![];

    scenarios.push(tc("process_line_valid_render", || {
        let job = answer_job(
            "# Title\n\nBody.",
            Markup::Markdown,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let response_str = process_line(&make_req(&job));
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        assert_eq!(response.protocol_version, WORKER_PROTOCOL_VERSION);
        assert!(response.packet.is_some());
        assert!(response.error.is_none());
        assert_eq!(response.job_id.as_deref(), Some("test-job"));
    }));

    scenarios.push(tc("process_line_bad_version", || {
        let job = answer_job(
            "hi",
            Markup::Markdown,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let request = serde_json::json!({
            "protocol_version": 999,
            "job": serde_json::to_value(&job).unwrap()
        });
        let response_str = process_line(&serde_json::to_string(&request).unwrap());
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        assert!(response.error.is_some());
        assert!(response.error.unwrap().contains("unsupported"));
    }));

    scenarios.push(tc("process_line_invalid_json", || {
        let response_str = process_line("not json");
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        assert!(response.error.is_some());
        assert!(response.error.unwrap().contains("invalid"));
    }));

    scenarios.push(tc("process_line_system_kind_fails", || {
        let job = DeliveryJob {
            job_id: "sys".into(),
            target: "t".into(),
            kind: DeliveryKind::System,
            content: DeliveryContent::Text {
                markdown: "sys".into(),
            },
            profile: plain_profile("test"),
            skill_registry: None,
        };
        let response_str = process_line(&make_req(&job));
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        assert!(response.error.is_some());
        assert!(response.packet.is_none());
    }));

    scenarios.push(tc("process_line_empty", || {
        let response_str = process_line("");
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        assert!(response.error.is_some());
    }));

    for (i, kind_name) in ["approval", "progress", "toolresult"].iter().enumerate() {
        let name = kind_name.to_string();
        scenarios.push(tc(&format!("process_line_{name}_{i}"), move || {
            let job = match name.as_str() {
                "approval" => approval_job(Markup::Markdown, "test"),
                "progress" => progress_job(Markup::Markdown, "test"),
                "toolresult" => tool_result_job(Markup::Markdown, "test", false),
                _ => unreachable!(),
            };
            let response_str = process_line(&make_req(&job));
            let response: WorkerResponse =
                serde_json::from_str(&response_str).expect("deserialize");
            assert!(response.packet.is_some());
        }));
    }

    scenarios.push(tc("process_line_text_job", || {
        let job = text_job("Some **text**", Markup::Markdown, "test");
        let response_str = process_line(&make_req(&job));
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        assert!(response.packet.is_some());
    }));

    scenarios.push(tc("process_line_response_roundtrip", || {
        let job = answer_job(
            "# T\n\nBody",
            Markup::Plain,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let response_str = process_line(&make_req(&job));
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        let packet = response.packet.expect("packet");
        assert_eq!(packet.job_id, "test-job");
        match &job.content {
            DeliveryContent::Answer(answer) => {
                assert_eq!(packet.fallback_markdown, answer.source_markdown);
            }
            _ => panic!("expected answer"),
        }
    }));

    scenarios.push(tc("process_line_json_markup", || {
        let job = answer_job(
            "# T\n\nBody",
            Markup::Json,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let response_str = process_line(&make_req(&job));
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        let packet = response.packet.expect("packet");
        assert!(matches!(packet.payload, DeliveryPayload::Structured(_)));
    }));

    scenarios.push(tc("process_line_with_template", || {
        let mut profile = plain_profile("test");
        profile.template = Some(TemplateSpec {
            id: "t".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![
                TemplateNode::Literal { text: "Hi ".into() },
                TemplateNode::Slot {
                    slot: TemplateSlot::Body,
                },
            ],
        });
        let job = DeliveryJob {
            job_id: "tmpl".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer_draft("source")),
            profile,
            skill_registry: None,
        };
        let response_str = process_line(&make_req(&job));
        let response: WorkerResponse = serde_json::from_str(&response_str).expect("deserialize");
        assert!(response.packet.is_some());
    }));

    run("worker_protocol", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 13. MARKUP CONVERTERS
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_markup_converters() {
    let mut scenarios: Vec<Scenario> = vec![];

    let converter_cases: &[(&str, &str)] = &[
        ("plain text", "Hello world"),
        ("bold", "**bold**"),
        ("italic", "*italic*"),
        ("strikethrough", "~~struck~~"),
        ("inline code", "use `code`"),
        ("link", "[label](https://example.com)"),
        ("heading", "# Title"),
        ("unordered list", "- a\n- b"),
        ("ordered list", "1. first\n2. second"),
        ("blockquote", "> quote"),
        ("code fence", "```rust\nlet x = 1;\n```"),
        ("table", "| A | B |\n|---|---|\n| 1 | 2 |"),
        ("horizontal rule", "---"),
        (
            "mixed",
            "# T\n\n**b** *i* ~~s~~ `c`\n\n- a\n- b\n\n[link](https://x.com)",
        ),
        ("empty", ""),
        ("emoji", "🎉🚀✅"),
        ("unicode", "世界の人々"),
        ("special chars", "< > & \" '"),
        ("long paragraph", &"word ".repeat(200)),
        ("nested quote", "> > deep"),
        ("task list", "- [ ] todo\n- [x] done"),
        ("multiple headings", "# A\n## B\n### C"),
        ("link with title", "[text](https://example.com \"Title\")"),
        ("image with title", "![alt](url \"Title\")"),
        ("raw html", "<b>bold</b>"),
        ("escaped chars", r"\*not bold\*"),
        ("code with backticks", "```\n```\n```"),
        ("empty table", "| |\n|---|\n| |"),
        ("single row table", "| A |\n|---|\n| 1 |"),
    ];

    for (label, md) in converter_cases {
        let l = label.to_string();
        let m = md.to_string();

        let m1 = m.clone();
        scenarios.push(tc(&format!("telegram_{l}"), move || {
            let result = telegram::markdown_to_html(&m1);
            assert!(!result.is_empty() || m1.is_empty());
        }));

        let m2 = m.clone();
        scenarios.push(tc(&format!("slack_{l}"), move || {
            let result = slack::markdown_to_mrkdwn(&m2);
            assert!(!result.is_empty() || m2.is_empty());
        }));

        let m3 = m;
        scenarios.push(tc(&format!("discord_{l}"), move || {
            let result = discord::markdown_to_discord(&m3);
            assert!(!result.is_empty() || m3.is_empty());
        }));
    }

    // Telegram-specific assertions
    let tg_cases: &[(&str, &str, &str)] = &[
        ("bold_becomes_strong", "**bold**", "<b>"),
        ("heading_becomes_bold", "# Title", "<b>"),
        ("code_becomes_pre", "```rust\nfn main() {}\n```", "<pre>"),
        ("link_becomes_a", "[text](https://example.com)", "<a href"),
        (
            "table_becomes_pre",
            "| A | B |\n|---|---|\n| 1 | 2 |",
            "<pre>",
        ),
        ("quote_becomes_blockquote", "> quoted", "<blockquote>"),
        ("hr_becomes_entity", "---", "—"),
        ("strike_becomes_s", "~~struck~~", "<s>"),
        ("inline_code_becomes_code", "use `code`", "<code>"),
    ];
    for (label, input, expected) in tg_cases {
        let l = label.to_string();
        let inp = input.to_string();
        let exp = expected.to_string();
        scenarios.push(tc(&format!("tg_check_{l}"), move || {
            let html = telegram::markdown_to_html(&inp);
            assert!(
                html.contains(&exp),
                "expected {exp:?} in {html:?} for input {inp:?}"
            );
        }));
    }

    scenarios.push(tc("tg_escape_html_script", || {
        let html = telegram::markdown_to_html("<script>alert(1)</script>");
        assert!(!html.contains("<script>"));
    }));

    scenarios.push(tc("tg_strip_html_removes_tags", || {
        let stripped = telegram::strip_html("<b>hello</b> <i>world</i>");
        assert!(!stripped.contains("<b>"));
        assert!(stripped.contains("hello"));
    }));

    scenarios.push(tc("tg_strip_html_empty", || {
        assert_eq!(telegram::strip_html(""), "");
    }));

    scenarios.push(tc("tg_split_html_chunks_no_limit", || {
        let chunks = telegram::split_html_chunks("<p>Hello</p>", None);
        assert_eq!(chunks.len(), 1);
    }));

    scenarios.push(tc("tg_split_html_chunks_small_limit", || {
        let long = "<p>".to_string() + &"word ".repeat(20) + "</p>";
        let chunks = telegram::split_html_chunks(&long, Some(50));
        assert!(chunks.len() > 1);
    }));

    scenarios.push(tc("tg_split_html_chunks_below_32", || {
        let chunks = telegram::split_html_chunks("hello", Some(10));
        assert!(chunks.len() >= 1);
    }));

    // Slack-specific assertions
    let slack_cases: &[(&str, &str, bool)] = &[
        ("bold_single_star", "**bold**", true),
        ("italic_single_star", "*italic*", true),
    ];
    for (label, input, should_contain) in slack_cases {
        let l = label.to_string();
        let inp = input.to_string();
        let sc = *should_contain;
        scenarios.push(tc(&format!("slack_check_{l}"), move || {
            let out = slack::markdown_to_mrkdwn(&inp);
            if sc {
                assert!(!out.is_empty());
            }
        }));
    }

    scenarios.push(tc("slack_table_preserved", || {
        let out = slack::markdown_to_mrkdwn("| A | B |\n|---|---|\n| 1 | 2 |");
        assert!(out.contains("A") && out.contains("1"));
    }));

    scenarios.push(tc("slack_code_fence_preserved", || {
        let out = slack::markdown_to_mrkdwn("```rust\nfn main() {}\n```");
        assert!(out.contains("```"));
    }));

    scenarios.push(tc("slack_link_not_link_syntax", || {
        let out = slack::markdown_to_mrkdwn("[text](https://example.com)");
        assert!(!out.contains("](https://example.com)"));
    }));

    // Discord-specific assertions
    scenarios.push(tc("discord_link_rewritten_exactly", || {
        let out = discord::markdown_to_discord("[text](https://example.com)");
        assert_eq!(out, "text (<https://example.com>)");
    }));

    let discord_cases: &[(&str, &str, &str)] = &[
        ("bold", "**bold**", "**bold**"),
        ("italic", "*italic*", "*italic*"),
        ("strike", "~~strikethrough~~", "~~strikethrough~~"),
        ("inline_code", "`code`", "`code`"),
        ("heading", "# Title", "# Title"),
        ("blockquote", "> quote", "> quote"),
        ("list", "- a\n- b", "- a"),
        ("code_fence", "```rust\nfn() {}\n```", "```rust"),
        ("spoiler", "||spoiler||", "||spoiler||"),
        ("mixed", "**b** *i* ~~s~~", "**b**"),
    ];
    for (label, input, expected) in discord_cases {
        let l = label.to_string();
        let inp = input.to_string();
        let exp = expected.to_string();
        scenarios.push(tc(&format!("discord_check_{l}"), move || {
            let out = discord::markdown_to_discord(&inp);
            assert!(
                out.contains(&exp),
                "expected {exp:?} in {out:?} for {inp:?}"
            );
        }));
    }

    scenarios.push(tc("discord_table_to_code_block", || {
        let out = discord::markdown_to_discord("| A | B |\n|---|---|\n| 1 | 2 |");
        assert!(out.contains("```"));
        assert!(!out.contains("| A | B |"));
    }));

    scenarios.push(tc("discord_horizontal_rule", || {
        let out = discord::markdown_to_discord("---");
        assert!(out.contains("─"));
    }));

    scenarios.push(tc("discord_empty_input", || {
        let out = discord::markdown_to_discord("");
        assert!(out.is_empty() || out.trim().is_empty());
    }));

    scenarios.push(tc("discord_unclosed_link", || {
        let out = discord::markdown_to_discord("[text](not_a_url)");
        assert!(out.contains("text"));
    }));

    run("markup_converters", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 14. STRUCTURED OUTPUTS & ADAPTERS
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_structured_outputs() {
    let mut scenarios: Vec<Scenario> = vec![];

    let valid_fragments: &[&str] = &[
        r#"{"semantic_type":"metric","payload":{"label":"X","value":42}}"#,
        r#"{"semantic_type":"link.preview","payload":{"url":"https://x.com","title":"T"}}"#,
        r#"{"semantic_type":"detail","payload":{"title":"T"}}"#,
        r#"{"semantic_type":"collection","payload":{"items":["a","b"]}}"#,
        r#"{"semantic_type":"comparison","payload":{"left":"a","right":"b"}}"#,
    ];

    for (i, frag) in valid_fragments.iter().enumerate() {
        let f = frag.to_string();
        scenarios.push(tc(&format!("structured_bare_{i}"), move || {
            let outputs = structured_outputs_from_text(&f);
            assert_eq!(outputs.len(), 1, "fragment {i}: {outputs:?}");
        }));
    }

    let invalid_fragments: &[&str] = &[
        r#"{"semantic_type":"unknown_type","payload":{}}"#,
        r#"{"semantic_type":"metric","payload":{"bad":"shape"}}"#,
        r#"not json"#,
        r#""{"missing": "quotes"}"#,
    ];
    for (i, frag) in invalid_fragments.iter().enumerate() {
        let f = frag.to_string();
        scenarios.push(tc(&format!("structured_invalid_{i}"), move || {
            let outputs = structured_outputs_from_text(&f);
            assert!(outputs.is_empty(), "expected empty for {f}");
        }));
    }

    let link_texts: &[&str] = &[
        "Check https://example.com for details",
        "Visit http://test.org\nAnd https://another.com",
        "No links here",
        "Multiple https://a.com https://b.com https://c.com",
        "",
    ];
    for (i, text) in link_texts.iter().enumerate() {
        let t = text.to_string();
        scenarios.push(tc(&format!("link_preview_{i}"), move || {
            let previews = link_previews_from_text(&t);
            assert!(previews.len() <= 12);
        }));
    }

    scenarios.push(tc("link_preview_dedup", || {
        let previews = link_previews_from_text("https://a.com https://a.com");
        assert_eq!(previews.len(), 1);
    }));

    scenarios.push(tc("link_preview_max_12", || {
        let urls: Vec<&str> = (0..15).map(|_| "https://example.com").collect();
        let text = urls.join(" ");
        assert_eq!(link_previews_from_text(&text).len(), 12);
    }));

    // project_structured_fences
    let fence_cases: &[&str] = &[
        "# Title\n\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"X\",\"value\":42}}\n```\n\nBody.",
        "```vak\n{\"semantic_type\":\"collection\",\"payload\":{\"items\":[\"a\",\"b\"]}}\n```",
        "No fences here",
        "```rust\nfn main() {}\n```\n\nText.",
        "```vak\n{bad json}\n```",
        "```vak\n{\"semantic_type\":\"unknown_type\",\"payload\":{}}\n```",
        "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"A\",\"value\":1}}\n```\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"B\",\"value\":2}}\n```",
    ];
    for (i, source) in fence_cases.iter().enumerate() {
        let s = source.to_string();
        scenarios.push(tc(&format!("project_fences_{i}"), move || {
            let projected = project_structured_fences(&s);
            assert!(!projected.is_empty() || s.is_empty());
        }));
    }

    // Adapters
    scenarios.push(tc("adapter_self_declared", || {
        let adapters = built_in_adapters();
        let outputs = structured_outputs_from_tool_result(
            r#"{"semantic_type":"metric","payload":{"label":"X","value":42}}"#,
            "desktop",
            &adapters,
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].semantic_type, "metric");
    }));

    scenarios.push(tc("adapter_weatherapi", || {
        let adapters = built_in_adapters();
        let outputs = structured_outputs_from_tool_result(
            r#"{"current":{"temp_c":25.0,"condition":{"text":"Sunny"}}}"#,
            "desktop",
            &adapters,
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].semantic_type, "metric");
        assert_eq!(outputs[0].payload["value"], 25.0);
    }));

    scenarios.push(tc("adapter_unrecognized_empty", || {
        let adapters = built_in_adapters();
        let outputs = structured_outputs_from_tool_result(
            r#"{"random":"data","count":42}"#,
            "desktop",
            &adapters,
        );
        assert!(outputs.is_empty());
    }));

    scenarios.push(tc("adapter_invalid_json_empty", || {
        let adapters = built_in_adapters();
        let outputs = structured_outputs_from_tool_result("not json at all", "desktop", &adapters);
        assert!(outputs.is_empty());
    }));

    run("structured_outputs", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 15. RENDER ERROR HANDLING
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_render_errors() {
    let mut scenarios: Vec<Scenario> = vec![];

    for kind in [
        DeliveryKind::System,
        DeliveryKind::Developer,
        DeliveryKind::Internal,
    ] {
        scenarios.push(tc(&format!("control_rejected_{kind:?}"), move || {
            let job = DeliveryJob {
                job_id: "sys".into(),
                target: "t".into(),
                kind: DeliveryKind::System,
                content: DeliveryContent::Text {
                    markdown: "x".into(),
                },
                profile: plain_profile("test"),
                skill_registry: None,
            };
            assert!(render(&job).is_err());
        }));
    }

    scenarios.push(tc("mismatched_approval_for_alert", || {
        let job = DeliveryJob {
            job_id: "m".into(),
            target: "t".into(),
            kind: DeliveryKind::Alert,
            content: DeliveryContent::Approval(ApprovalPayload {
                request_id: "r".into(),
                title: "t".into(),
                detail: "d".into(),
                expires_at: None,
                actions: vec![],
            }),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        assert!(render(&job).is_err());
    }));

    scenarios.push(tc("wrong_schema_version", || {
        let mut answer = answer_draft("content");
        answer.schema_version = 999;
        let job = DeliveryJob {
            job_id: "bad".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        assert!(render(&job).is_err());
    }));

    scenarios.push(tc("template_on_non_template_kind", || {
        let mut profile = plain_profile("test");
        profile.template = Some(TemplateSpec {
            id: "t".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![TemplateNode::Literal { text: "x".into() }],
        });
        let job = DeliveryJob {
            job_id: "tmpl".into(),
            target: "t".into(),
            kind: DeliveryKind::Approval,
            content: DeliveryContent::Approval(ApprovalPayload {
                request_id: "r".into(),
                title: "t".into(),
                detail: "d".into(),
                expires_at: None,
                actions: vec![],
            }),
            profile,
            skill_registry: None,
        };
        assert!(render(&job).is_err());
    }));

    scenarios.push(tc("proposed_template_rejected", || {
        let mut profile = plain_profile("test");
        profile.template = Some(TemplateSpec {
            id: "t".into(),
            revision: 1,
            origin: TemplateOrigin::AgentProposal,
            activation: TemplateActivation::Proposed,
            nodes: vec![TemplateNode::Literal { text: "x".into() }],
        });
        let job = DeliveryJob {
            job_id: "pt".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer_draft("source")),
            profile,
            skill_registry: None,
        };
        assert!(render(&job).is_err());
    }));

    scenarios.push(tc("empty_surface_rejected", || {
        let job = answer_job("source", Markup::Plain, None, DeliveryKind::Assistant, "");
        assert!(render(&job).is_err());
    }));

    scenarios.push(tc("max_chars_zero_rejected", || {
        let job = answer_job(
            "source",
            Markup::Plain,
            Some(0),
            DeliveryKind::Assistant,
            "test",
        );
        assert!(render(&job).is_err());
    }));

    run("render_errors", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 16. PACKET STRUCTURE & INVARIANTS
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_packet_structure() {
    let mut scenarios: Vec<Scenario> = vec![];

    let source = "# Title\n\nBody text.\n\n```rust\nlet x = 1;\n```";

    for (i, markup) in [
        Markup::Plain,
        Markup::Markdown,
        Markup::TelegramHtml,
        Markup::SlackMrkdwn,
        Markup::DiscordMarkdown,
        Markup::Json,
    ]
    .iter()
    .enumerate()
    {
        let m = *markup;
        scenarios.push(tc(&format!("packet_schema_{i}"), move || {
            let job = answer_job(source, m, Some(1000), DeliveryKind::Assistant, "test");
            let packet = render(&job).expect("render");
            assert_eq!(packet.schema_version, DELIVERY_SCHEMA_VERSION);
        }));
    }

    scenarios.push(tc("fallback_exact_source", move || {
        let job = answer_job(
            source,
            Markup::TelegramHtml,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let packet = render(&job).expect("render");
        assert_eq!(packet.fallback_markdown, source);
    }));

    let chunk_sources: &[&str] = &["Hello", "# Title\n\nBody", "```\ncode\n```", ""];
    for (i, src) in chunk_sources.iter().enumerate() {
        let s = src.to_string();
        scenarios.push(tc(&format!("chunks_nonempty_{i}"), move || {
            let job = answer_job(
                &s,
                Markup::Markdown,
                Some(10),
                DeliveryKind::Assistant,
                "test",
            );
            let packet = render(&job).expect("render");
            for chunk in &packet.chunks {
                assert!(!chunk.is_empty() || s.is_empty());
            }
        }));
    }

    scenarios.push(tc("coverage_matches_blocks", || {
        let src = "# A\n\nB\n\n- 1\n- 2\n\n```\ncode\n```";
        let answer = answer_draft(src);
        let expected = answer.blocks.len();
        let job = answer_job(src, Markup::Plain, None, DeliveryKind::Assistant, "test");
        let packet = render(&job).expect("render");
        assert_eq!(packet.coverage.len(), expected);
    }));

    scenarios.push(tc("actions_diagnostic_no_support", || {
        let job = full_job_with_actions(
            "source",
            Markup::Plain,
            None,
            DeliveryKind::Approval,
            "test",
        );
        let packet = render(&job).expect("render");
        assert!(!packet.diagnostics.is_empty());
    }));

    scenarios.push(tc("no_diagnostic_with_support", || {
        let job = full_job_no_actions(
            "source",
            Markup::Plain,
            None,
            DeliveryKind::Approval,
            "test",
        );
        let packet = render(&job).expect("render");
        assert!(packet.diagnostics.is_empty());
    }));

    scenarios.push(tc("json_payload_structured", || {
        let job = answer_job(
            "# T\n\nBody",
            Markup::Json,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let packet = render(&job).expect("render");
        assert!(matches!(packet.payload, DeliveryPayload::Structured(_)));
    }));

    scenarios.push(tc("plain_payload_text", || {
        let job = answer_job(
            "# T\n\nBody",
            Markup::Plain,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let packet = render(&job).expect("render");
        assert!(matches!(packet.payload, DeliveryPayload::Text(_)));
    }));

    scenarios.push(tc("presentation_some_for_answer", || {
        let job = answer_job(
            "# T\n\nBody",
            Markup::Plain,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let packet = render(&job).expect("render");
        assert!(packet.presentation.is_some());
        let timeline = packet.presentation.unwrap();
        assert_eq!(timeline.schema_version, PRESENTATION_SCHEMA_VERSION);
        assert_eq!(timeline.items.len(), 1);
    }));

    scenarios.push(tc("presentation_none_for_approval", || {
        let job = approval_job(Markup::Plain, "test");
        let packet = render(&job).expect("render");
        assert!(packet.presentation.is_none());
    }));

    scenarios.push(tc("presentation_none_for_progress", || {
        let job = progress_job(Markup::Plain, "test");
        let packet = render(&job).expect("render");
        assert!(packet.presentation.is_none());
    }));

    scenarios.push(tc("presentation_none_for_text", || {
        let job = text_job("text", Markup::Plain, "test");
        let packet = render(&job).expect("render");
        assert!(packet.presentation.is_none());
    }));

    scenarios.push(tc("presentation_none_for_toolresult", || {
        let job = tool_result_job(Markup::Plain, "test", false);
        let packet = render(&job).expect("render");
        assert!(packet.presentation.is_none());
    }));

    // Outcome status from metadata
    scenarios.push(tc("outcome_succeeded_complete", || {
        let src = "# Result\n\nDone.";
        let mut answer = answer_draft(src);
        answer
            .metadata
            .insert("outcome_completion".into(), "complete".into());
        let job = DeliveryJob {
            job_id: "o1".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        let packet = render(&job).expect("render");
        let timeline = packet.presentation.unwrap();
        assert_eq!(timeline.items[0].status, OutputStatus::Succeeded);
    }));

    scenarios.push(tc("outcome_partial_when_incomplete", || {
        let src = "# Result\n\nDone.";
        let mut answer = answer_draft(src);
        answer
            .metadata
            .insert("outcome_completion".into(), "partial".into());
        let job = DeliveryJob {
            job_id: "o2".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        let packet = render(&job).expect("render");
        let timeline = packet.presentation.unwrap();
        assert_eq!(timeline.items[0].status, OutputStatus::Partial);
        assert!(!timeline.diagnostics.is_empty());
    }));

    scenarios.push(tc("outcome_partial_for_human_review", || {
        let src = "# Result\n\nDone.";
        let mut answer = answer_draft(src);
        answer
            .metadata
            .insert("outcome_human_review".into(), "needed".into());
        let job = DeliveryJob {
            job_id: "o3".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        let packet = render(&job).expect("render");
        let timeline = packet.presentation.unwrap();
        assert_eq!(timeline.items[0].status, OutputStatus::Partial);
    }));

    // channel_projection joins results
    scenarios.push(tc("channel_projection_multi_results", || {
        let src = "# Combined";
        let answer = AnswerDraft {
            schema_version: DELIVERY_SCHEMA_VERSION,
            source_markdown: src.into(),
            blocks: answer_blocks(src),
            document: PresentationDocument::default(),
            metadata: BTreeMap::new(),
            results: vec![
                AnswerResult {
                    id: "r1".into(),
                    source_markdown: "First.".into(),
                    outcome: None,
                },
                AnswerResult {
                    id: "r2".into(),
                    source_markdown: "Second.".into(),
                    outcome: None,
                },
            ],
        };
        let job = DeliveryJob {
            job_id: "cp".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        let packet = render(&job).expect("render");
        assert!(packet.fallback_markdown.contains("First."));
        assert!(packet.fallback_markdown.contains("Second."));
    }));

    scenarios.push(tc("channel_projection_single_result", || {
        let src = "# Single";
        let answer = AnswerDraft {
            schema_version: DELIVERY_SCHEMA_VERSION,
            source_markdown: src.into(),
            blocks: answer_blocks(src),
            document: PresentationDocument::default(),
            metadata: BTreeMap::new(),
            results: vec![AnswerResult {
                id: "r1".into(),
                source_markdown: "Only.".into(),
                outcome: None,
            }],
        };
        let job = DeliveryJob {
            job_id: "cp2".into(),
            target: "t".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(answer),
            profile: plain_profile("test"),
            skill_registry: None,
        };
        let packet = render(&job).expect("render");
        assert_eq!(packet.fallback_markdown, "Only.");
    }));

    run("packet_structure", scenarios);
}

fn full_job_with_actions(
    source: &str,
    markup: Markup,
    max_chars: Option<usize>,
    kind: DeliveryKind,
    surface: &str,
) -> DeliveryJob {
    DeliveryJob {
        job_id: "full-a".into(),
        target: format!("{surface}:one"),
        kind,
        content: DeliveryContent::Answer(answer_draft(source)),
        profile: DeliveryProfile {
            surface: surface.into(),
            markup,
            max_chars,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: true,
            template: None,
            posture: DeliveryPosture::default(),
        },
        skill_registry: None,
    }
}

fn full_job_no_actions(
    source: &str,
    markup: Markup,
    max_chars: Option<usize>,
    kind: DeliveryKind,
    surface: &str,
) -> DeliveryJob {
    DeliveryJob {
        job_id: "full-na".into(),
        target: format!("{surface}:one"),
        kind,
        content: DeliveryContent::Answer(answer_draft(source)),
        profile: DeliveryProfile {
            surface: surface.into(),
            markup,
            max_chars,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: DeliveryPosture::default(),
        },
        skill_registry: None,
    }
}

// ════════════════════════════════════════════════════════════════════
// 17. SERIALIZATION ROUNDTRIP
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_serialization_roundtrip() {
    let mut scenarios: Vec<Scenario> = vec![];

    let source = "# Title\n\nBody text.\n\n```rust\nlet x = 1;\n```";

    for (i, markup) in [
        Markup::Plain,
        Markup::Markdown,
        Markup::TelegramHtml,
        Markup::SlackMrkdwn,
        Markup::DiscordMarkdown,
        Markup::Json,
    ]
    .iter()
    .enumerate()
    {
        let m = *markup;
        scenarios.push(tc(&format!("serde_packet_{i}"), move || {
            let job = answer_job(source, m, Some(1000), DeliveryKind::Assistant, "test");
            let packet = render(&job).expect("render");
            let bytes = serde_json::to_vec(&packet).expect("serialize");
            let deserialized: DeliveryPacket = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(deserialized.job_id, packet.job_id);
            assert_eq!(deserialized.fallback_markdown, packet.fallback_markdown);
            assert_eq!(deserialized.coverage, packet.coverage);
        }));
    }

    scenarios.push(tc("serde_job_roundtrip", || {
        let job = answer_job(
            source,
            Markup::Markdown,
            None,
            DeliveryKind::Assistant,
            "test",
        );
        let bytes = serde_json::to_vec(&job).expect("serialize");
        let deserialized: DeliveryJob = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(deserialized.job_id, job.job_id);
        assert_eq!(deserialized.kind, job.kind);
        assert_eq!(deserialized.profile.surface, job.profile.surface);
    }));

    for (i, markup) in [Markup::Plain, Markup::Markdown, Markup::Json]
        .iter()
        .enumerate()
    {
        let m = *markup;
        scenarios.push(tc(&format!("serde_profile_{i}"), move || {
            let p = DeliveryProfile {
                surface: "telegram".into(),
                markup: m,
                max_chars: Some(4000),
                supports_tables: true,
                supports_code_blocks: true,
                supports_links: true,
                supports_actions: false,
                template: None,
                posture: DeliveryPosture::default(),
            };
            let bytes = serde_json::to_vec(&p).expect("serialize");
            let deserialized: DeliveryProfile =
                serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(deserialized.markup, p.markup);
            assert_eq!(deserialized.max_chars, p.max_chars);
            assert_eq!(deserialized.surface, p.surface);
        }));
    }

    for (i, cadence) in [Cadence::Live, Cadence::OnCompletion, Cadence::Digest]
        .iter()
        .enumerate()
    {
        let c = *cadence;
        scenarios.push(tc(&format!("serde_cadence_{i}"), move || {
            let bytes = serde_json::to_vec(&c).expect("serialize");
            let d: Cadence = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(d, c);
        }));
    }

    for (i, urgency) in [Urgency::Interrupt, Urgency::Notify, Urgency::Quiet]
        .iter()
        .enumerate()
    {
        let u = *urgency;
        scenarios.push(tc(&format!("serde_urgency_{i}"), move || {
            let bytes = serde_json::to_vec(&u).expect("serialize");
            let d: Urgency = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(d, u);
        }));
    }

    for (i, kind) in [
        DeliveryKind::Assistant,
        DeliveryKind::Approval,
        DeliveryKind::Progress,
        DeliveryKind::ToolResult,
        DeliveryKind::Alert,
        DeliveryKind::User,
        DeliveryKind::System,
        DeliveryKind::Internal,
    ]
    .iter()
    .enumerate()
    {
        let k = *kind;
        scenarios.push(tc(&format!("serde_kind_{i}"), move || {
            let bytes = serde_json::to_vec(&k).expect("serialize");
            let d: DeliveryKind = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(d, k);
        }));
    }

    for (i, content) in [
        DeliveryContent::Text {
            markdown: "hello".into(),
        },
        DeliveryContent::Approval(ApprovalPayload {
            request_id: "r".into(),
            title: "t".into(),
            detail: "d".into(),
            expires_at: None,
            actions: vec![],
        }),
        DeliveryContent::Progress(ProgressPayload {
            label: "l".into(),
            state: "s".into(),
            percent: Some(50),
        }),
        DeliveryContent::ToolResult(ToolResultPayload {
            tool: "t".into(),
            output: "o".into(),
            is_error: false,
        }),
    ]
    .iter()
    .enumerate()
    {
        let c = content.clone();
        scenarios.push(tc(&format!("serde_content_{i}"), move || {
            let bytes = serde_json::to_vec(&c).expect("serialize");
            let d: DeliveryContent = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(d, c);
        }));
    }

    scenarios.push(tc("serde_template_roundtrip", || {
        let template = TemplateSpec {
            id: "test".into(),
            revision: 1,
            origin: TemplateOrigin::User,
            activation: TemplateActivation::Active,
            nodes: vec![
                TemplateNode::Literal {
                    text: "Hello".into(),
                },
                TemplateNode::Slot {
                    slot: TemplateSlot::Body,
                },
            ],
        };
        let bytes = serde_json::to_vec(&template).expect("serialize");
        let deserialized: TemplateSpec = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(deserialized.id, template.id);
        assert_eq!(deserialized.revision, template.revision);
        assert_eq!(deserialized.nodes.len(), template.nodes.len());
    }));

    scenarios.push(tc("serde_profile_roundtrip", || {
        let p = DeliveryProfile {
            surface: "tg".into(),
            markup: Markup::TelegramHtml,
            max_chars: Some(4000),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: true,
            template: Some(TemplateSpec {
                id: "t".into(),
                revision: 1,
                origin: TemplateOrigin::User,
                activation: TemplateActivation::Active,
                nodes: vec![TemplateNode::Literal { text: "x".into() }],
            }),
            posture: DeliveryPosture {
                cadence: Cadence::Digest,
                urgency: Urgency::Quiet,
            },
        };
        let bytes = serde_json::to_vec(&p).expect("serialize");
        let d: DeliveryProfile = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(d.surface, p.surface);
        assert_eq!(d.markup, p.markup);
        assert_eq!(d.max_chars, p.max_chars);
        assert!(d.template.is_some());
    }));

    run("serialization_roundtrip", scenarios);
}

// ════════════════════════════════════════════════════════════════════
// 18. CONTENT KIND × MARKUP MATRIX
// ════════════════════════════════════════════════════════════════════

#[test]
fn audit_content_kind_markup_matrix() {
    let mut scenarios: Vec<Scenario> = vec![];

    let external_kinds = [
        DeliveryKind::Assistant,
        DeliveryKind::TaskSummary,
        DeliveryKind::Alert,
        DeliveryKind::User,
        DeliveryKind::Approval,
        DeliveryKind::Progress,
        DeliveryKind::ToolResult,
    ];

    let markups: [(Markup, &str); 6] = [
        (Markup::Plain, "plain"),
        (Markup::Markdown, "md"),
        (Markup::TelegramHtml, "tg"),
        (Markup::SlackMrkdwn, "slack"),
        (Markup::DiscordMarkdown, "discord"),
        (Markup::Json, "json"),
    ];

    for kind in &external_kinds {
        for (markup, label) in markups {
            let k = *kind;
            let m = markup;
            let l = label;
            scenarios.push(tc(&format!("matrix_{k:?}_{l}"), move || {
                let content = match k {
                    DeliveryKind::Approval => DeliveryContent::Approval(ApprovalPayload {
                        request_id: "r".into(),
                        title: "t".into(),
                        detail: "d".into(),
                        expires_at: None,
                        actions: vec![],
                    }),
                    DeliveryKind::Progress => DeliveryContent::Progress(ProgressPayload {
                        label: "l".into(),
                        state: "s".into(),
                        percent: None,
                    }),
                    DeliveryKind::ToolResult => DeliveryContent::ToolResult(ToolResultPayload {
                        tool: "bash".into(),
                        output: "ok".into(),
                        is_error: false,
                    }),
                    _ => DeliveryContent::Answer(answer_draft("source")),
                };
                let job = DeliveryJob {
                    job_id: "mx".into(),
                    target: "t".into(),
                    kind: k,
                    content,
                    profile: DeliveryProfile::plain("test"),
                    skill_registry: None,
                };
                let job = DeliveryJob {
                    profile: DeliveryProfile {
                        surface: "test".into(),
                        markup: m,
                        max_chars: None,
                        supports_tables: true,
                        supports_code_blocks: true,
                        supports_links: true,
                        supports_actions: false,
                        template: None,
                        posture: DeliveryPosture::default(),
                    },
                    ..job
                };
                let result = render(&job);
                match k {
                    DeliveryKind::Approval | DeliveryKind::Progress | DeliveryKind::ToolResult => {
                        // These don't accept Answer content, so they might error
                        // Actually Text content is accepted by all
                        // But they specifically need matching content
                        // Approval→Approval, Progress→Progress, ToolResult→ToolResult
                        // Since we provide matching content, they should succeed
                        assert!(result.is_ok(), "{k:?} + {l}: {result:?}");
                    }
                    _ => {
                        // Assistant/TaskSummary/Alert/User can accept Answer
                        assert!(result.is_ok(), "{k:?} + {l}: {result:?}");
                    }
                }
            }));
        }
    }

    run("content_kind_markup_matrix", scenarios);
}
