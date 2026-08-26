//! Transcript rendering and read-only viewer modals over server transcripts.

use vak_client::types::{ContentBlock, Message, SearchHit};

use crate::commands::short_hex;
use crate::data::ClientData;
use crate::events::trunc_one;
use crate::state::ModalView;

const VIEWER_LIMIT: usize = 200;

pub fn find_matches(rows: &[String], query: &str) -> Vec<usize> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let needle = trimmed.to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

pub fn build_transcript_rows(msgs: &[Message], start: usize) -> (Vec<String>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut anchors = Vec::new();
    let clamped = if msgs.is_empty() {
        0
    } else {
        start.min(msgs.len().saturating_sub(1))
    };
    for (idx, m) in msgs[clamped..].iter().enumerate() {
        let absolute = clamped + idx;
        match m.role {
            vak_client::types::Role::User => {
                anchors.push(rows.len());
                rows.push(format!("{absolute:>4} ▸ user"));
            }
            _ => rows.push(format!("{absolute:>4} ◆ assistant")),
        }
        for block in &m.content {
            match block {
                ContentBlock::Text { text } => {
                    for line in text.trim().lines().take(6) {
                        rows.push(format!("     {}", trunc_one(line.trim_end(), 160)));
                    }
                }
                ContentBlock::Thinking { .. } => {}
                ContentBlock::ToolUse { name, input, .. } => {
                    rows.push(format!(
                        "     · tool {name} {}",
                        trunc_one(&input.to_string(), 90)
                    ));
                }
                ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let mark = if *is_error { "✗" } else { "→" };
                    rows.push(format!(
                        "     {mark} {}",
                        trunc_one(content.replace('\n', " ").trim(), 130)
                    ));
                }
                ContentBlock::Image { source } => {
                    rows.push(format!("     ▣ image ({})", source.media_type));
                }
            }
        }
        rows.push(String::new());
    }
    (rows, anchors)
}

/// Opens another session's ledger purely for viewing: no session-slot swap,
/// so nothing can append to it and the active session stays untouched.
pub async fn session_view_modal(data: &ClientData, hit: &SearchHit) -> Result<ModalView, String> {
    session_transcript_modal(data, &hit.session_id, "opened read-only from search").await
}

/// Read-only modal over any session's server-side transcript.
pub async fn session_transcript_modal(
    data: &ClientData,
    session_id: &str,
    origin: &str,
) -> Result<ModalView, String> {
    let t = data.transcript(session_id).await?;
    let start = t.messages.len().saturating_sub(VIEWER_LIMIT);
    let (mut rows, anchors) = build_transcript_rows(&t.messages, start);
    rows.insert(
        0,
        format!("{} messages · {origin}", t.messages.len() - start),
    );
    Ok(ModalView {
        title: format!("session transcript · {}", short_hex(session_id)),
        scroll: rows.len().saturating_sub(1),
        rows,
        footer: "read-only view · /resume resumes your own sessions · Esc close".to_string(),
        anchors: anchors.into_iter().map(|a| a + 1).collect(),
        ..Default::default()
    })
}

#[cfg(test)]
mod transcript_render_tests {
    use super::*;

    #[test]
    fn transcript_rows_mark_user_prompts_as_jump_anchors() {
        let msgs = vec![
            Message::user_text("first prompt"),
            Message::assistant(vec![ContentBlock::text("reply one")]),
            Message::user_text("second prompt"),
            Message::assistant(vec![ContentBlock::tool_result("t1", "ok")]),
        ];
        let (rows, anchors) = build_transcript_rows(&msgs, 0);
        assert_eq!(anchors.len(), 2, "one anchor per user message");
        let plain = rows.join("\n");
        assert!(plain.contains("0 ▸ user"));
        assert!(plain.contains("2 ▸ user"));
        assert!(plain.contains("1 ◆ assistant"));
        assert!(plain.contains("→ ok"));
        assert!(rows[anchors[0]].contains("▸ user"));
        assert!(rows[anchors[1]].contains("▸ user"));
    }

    #[test]
    fn transcript_window_offset_keeps_absolute_indices() {
        let msgs: Vec<Message> = (0..6)
            .map(|i| Message::user_text(format!("prompt {i}")))
            .collect();
        let (rows, _) = build_transcript_rows(&msgs, 4);
        let plain = rows.join("\n");
        assert!(plain.contains("4 ▸ user"), "{plain}");
        assert!(!plain.contains("3 ▸ user"), "{plain}");
    }

    #[test]
    fn start_beyond_len_is_clamped_not_panicking() {
        let msgs = vec![Message::user_text("only")];
        let (rows, _) = build_transcript_rows(&msgs, 99);
        assert!(!rows.is_empty());
    }
}

#[cfg(test)]
mod transcript_search_tests {
    use super::find_matches;

    #[test]
    fn search_is_case_insensitive_and_ordered() {
        let rows: Vec<String> = vec![
            "0 ▸ user".into(),
            "     fix the parser".into(),
            "1 ◆ assistant".into(),
            "     Fix The Parser again".into(),
        ];
        assert_eq!(find_matches(&rows, "parser"), vec![1, 3]);
        assert_eq!(find_matches(&rows, "PARSER"), vec![1, 3]);
    }

    #[test]
    fn empty_or_blank_queries_match_nothing() {
        let rows = vec!["anything".to_string()];
        assert!(find_matches(&rows, "").is_empty());
        assert!(find_matches(&rows, "   ").is_empty());
    }

    #[test]
    fn no_hits_return_empty_vec() {
        let rows = vec!["alpha".to_string(), "beta".to_string()];
        assert!(find_matches(&rows, "gamma").is_empty());
    }
}
