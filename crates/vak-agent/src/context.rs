//! Long-horizon context management: token estimation, input budgets, and
//! automatic compaction. Policy lives here; ledger mechanics live in
//! vak-session. On overflow the loop summarizes older turns into a
//! compaction entry (never deletion) and retries the same contract; if the
//! projection still exceeds the budget, it fails closed.

use vak_llm::{ChatRequest, ContentBlock, Message};

#[derive(Debug, Clone)]
pub struct ContextPolicy {
    /// Model context window in tokens.
    pub context_window: u64,
    /// Reserve for the completion (max_tokens).
    pub max_output: u64,
    /// Fraction of the input budget at which compaction triggers.
    pub compact_threshold: f64,
    /// Recent messages always kept verbatim across compaction.
    pub keep_recent: usize,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        ContextPolicy {
            context_window: 128_000,
            max_output: 8192,
            compact_threshold: 0.8,
            keep_recent: 6,
        }
    }
}

impl ContextPolicy {
    pub fn input_budget(&self) -> u64 {
        self.context_window
            .saturating_sub(self.max_output)
            .max(1_000)
    }

    pub fn trigger_at(&self) -> u64 {
        (self.input_budget() as f64 * self.compact_threshold) as u64
    }
}

/// Chars/4 heuristic — planning estimate only, never treated as provider
/// usage (see docs/design/15-reliability.md).
pub fn estimate_tokens(messages: &[Message], system: Option<&str>) -> u64 {
    let mut chars: u64 = system.map(|s| s.len() as u64).unwrap_or(0);
    for m in messages {
        for b in &m.content {
            chars += match b {
                ContentBlock::Text { text } => text.len() as u64,
                ContentBlock::Thinking { text, .. } => text.len() as u64,
                ContentBlock::ToolUse { name, input, .. } => {
                    name.len() as u64
                        + serde_json::to_string(input)
                            .map(|s| s.len() as u64)
                            .unwrap_or(0)
                }
                ContentBlock::ToolResult { content, .. } => content.len() as u64,
            };
        }
    }
    chars.div_ceil(4)
}

pub const COMPACTION_SYSTEM: &str = "\
You are a context compactor for a coding-agent session. Produce a dense \
structured summary of the conversation so far. Keep: the original task, \
current state, files created/modified (with paths), key decisions, errors \
hit and their fixes, and open items. Drop pleasantries and redundant tool \
output. Maximum 400 words.";

pub fn compaction_prompt(transcript: &str) -> String {
    format!(
        "Summarize this coding-session segment for continuation. The summary \
         will replace these turns in context; later turns stay verbatim.\n\n\
         <segment>\n{transcript}\n</segment>"
    )
}

/// Builds the compaction request over a rendered transcript segment.
pub fn compaction_request(model: &str, transcript: &str) -> ChatRequest {
    let mut req = ChatRequest::new(model);
    req.system = Some(COMPACTION_SYSTEM.to_string());
    req.messages = vec![Message::user_text(compaction_prompt(transcript))];
    req.max_tokens = 1024;
    req
}

/// Renders messages to a readable transcript for summarization.
pub fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        let role = match m.role {
            vak_llm::Role::User => "user",
            vak_llm::Role::Assistant => "assistant",
        };
        out.push_str(&format!("[{role}]\n{}\n\n", m.text_content()));
        for b in &m.content {
            if let ContentBlock::ToolResult { content, .. } = b {
                let preview: String = content.chars().take(600).collect();
                out.push_str(&format!("[tool-result]\n{preview}\n\n"));
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn estimate_scales_with_content() {
        let short = vec![Message::user_text("hi")];
        let long = vec![Message::user_text("x".repeat(4000))];
        assert_eq!(estimate_tokens(&short, None), 1);
        assert!(estimate_tokens(&long, None) >= 1000);
        assert!(estimate_tokens(&short, Some("system ".repeat(4).as_str())) > 1);
    }

    #[test]
    fn budget_math_reserves_output() {
        let p = ContextPolicy {
            context_window: 10_000,
            max_output: 2_000,
            ..Default::default()
        };
        assert_eq!(p.input_budget(), 8_000);
        assert_eq!(p.trigger_at(), 6_400);
    }

    #[test]
    fn compaction_request_carries_marker_and_transcript() {
        let req = compaction_request("m", "SEGMENT TEXT");
        let system = req.system.clone().expect("system present");
        assert!(system.contains("context compactor"));
        let msg = &req.messages[0];
        assert!(msg.text_content().contains("SEGMENT TEXT"));
    }
}
