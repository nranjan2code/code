//! Markdown transcript renderer (docs/design/29-personal-os.md P4). One
//! shared renderer for the TUI export and the server-side export so both
//! surfaces emit byte-identical markdown for the same session. Pure:
//! messages in, string out.

use vak_llm::{ContentBlock, Message, Role};

/// Render a projected message list exactly as `export_transcript` has
/// always written it: `# Vak transcript`, then one `## {idx} · {role}`
/// section per message with block-level lines between.
pub fn render_markdown(msgs: &[Message]) -> String {
    let mut out = String::from("# Vak transcript\n\n");
    for (idx, m) in msgs.iter().enumerate() {
        let role = match m.role {
            Role::User => "user",
            _ => "assistant",
        };
        out.push_str(&format!("## {idx} · {role}\n\n"));
        for block in &m.content {
            match block {
                ContentBlock::Text { text } => {
                    out.push_str(text.trim());
                    out.push_str("\n\n");
                }
                ContentBlock::Thinking { text, .. } => {
                    out.push_str(&format!("> thinking: {}\n\n", text.trim()));
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    out.push_str(&format!("- tool `{name}` `{input}`\n"));
                }
                ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let mark = if *is_error { "✗" } else { "→" };
                    out.push_str(&format!(
                        "- {mark} result: {}\n",
                        content.replace('\n', " ")
                    ));
                }
                ContentBlock::Image { source } => {
                    out.push_str(&format!("- image ({})\n", source.media_type));
                }
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn output_equals_legacy_tui_export_byte_for_byte() {
        let msgs = vec![
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::text("  fix the flaky test  \nsecond line  "),
                    ContentBlock::image_base64("image/png", "aGVsbG8="),
                ],
            },
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Thinking {
                        text: " consider retries ".into(),
                        signature: Some("sig".into()),
                    },
                    ContentBlock::ToolUse {
                        id: "t1".into(),
                        name: "bash".into(),
                        input: serde_json::json!({"command": "cargo test"}),
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "ok\nwith newline".into(),
                        is_error: false,
                    },
                    ContentBlock::tool_error("t2", "boom"),
                    ContentBlock::text("done."),
                ],
            },
        ];

        // Expected string transcribed from the TUI export path this module
        // replaces (note: `text.trim()` keeps interior line-trailing
        // spaces, and each message section ends with a blank line); any
        // divergence from it is a parity bug.
        let expected = "# Vak transcript\n\
\n\
## 0 · user\n\
\n\
fix the flaky test  \nsecond line\n\
\n\
- image (image/png)\n\
\n\
## 1 · assistant\n\
\n\
> thinking: consider retries\n\
\n\
- tool `bash` `{\"command\":\"cargo test\"}`\n\
- → result: ok with newline\n\
- ✗ result: boom\n\
done.\n\
\n\
\n";
        assert_eq!(render_markdown(&msgs), expected);
    }

    #[test]
    fn empty_transcript_renders_header_only() {
        assert_eq!(render_markdown(&[]), "# Vak transcript\n\n");
    }
}
