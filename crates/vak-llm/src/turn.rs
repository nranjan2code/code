//! Shared "current turn" boundary used by adapters that must decide which
//! messages are still part of the turn in progress (thinking blocks kept
//! verbatim, since the provider may require replay) versus a closed prior
//! turn (thinking stripped, since no provider needs it across turns).

use crate::types::{ContentBlock, Message, Role};

/// Index of the first message belonging to the current turn: the last user
/// message that carries a `Text` block and no `ToolResult` block. A
/// `ToolResult`-only user message is a mid-turn continuation, not a new
/// directive, so it does not start a new "current turn". When no such
/// message exists, the whole history is treated as current (index 0) —
/// the safe default, since it never strips a signature the provider needs.
pub fn current_turn_boundary(messages: &[Message]) -> usize {
    messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| {
            m.role == Role::User
                && m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::Text { .. }))
                && !m
                    .content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// A copy of `m` with every `Thinking` block removed.
pub fn strip_thinking(m: &Message) -> Message {
    Message {
        role: m.role,
        content: m
            .content
            .iter()
            .filter(|b| !matches!(b, ContentBlock::Thinking { .. }))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentBlock;

    #[test]
    fn boundary_is_the_last_fresh_directive() {
        let messages = vec![
            Message::user_text("first question"),
            Message::assistant(vec![ContentBlock::text("answer 1")]),
            Message::user_text("second question"),
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "search".into(),
                input: serde_json::json!({}),
            }]),
            Message {
                role: Role::User,
                content: vec![ContentBlock::tool_result("t1", "result")],
            },
        ];
        assert_eq!(current_turn_boundary(&messages), 2);
    }

    #[test]
    fn no_fresh_directive_defaults_to_the_whole_history() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result("t1", "result")],
        }];
        assert_eq!(current_turn_boundary(&messages), 0);
    }

    #[test]
    fn strip_thinking_removes_only_thinking_blocks() {
        let m = Message::assistant(vec![
            ContentBlock::Thinking {
                text: "hmm".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::text("visible"),
        ]);
        let stripped = strip_thinking(&m);
        assert_eq!(stripped.content, vec![ContentBlock::text("visible")]);
    }
}
