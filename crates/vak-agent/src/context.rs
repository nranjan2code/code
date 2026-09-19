//! Compaction mechanics and the stable-prefix digest.
//!
//! Budgeting policy lives in `capacity` (`CapacityProfile`) and `planner`
//! (`WorkingSetPlanner`) now — this module only builds the summarizer
//! request and renders a segment for it (docs/design/68-context-engine.md
//! §4). `ContextPolicy` and the chars/4 `estimate_tokens` are deleted:
//! every estimate goes through `CapacityProfile::estimate_tokens`.

use sha2::{Digest, Sha256};
use vak_llm::{ChatRequest, ContentBlock, Message, ToolDefinition};

/// SHA-256 hex digest of the stable prefix — the system prompt plus the
/// tool schemas in dispatch order — recorded on every `WorkReceipt` so a
/// change in either is visible in the ledger as a cache-breaking event
/// (docs/design/68-context-engine.md §6/§7). Tool schemas are hashed via
/// their serialized JSON form so a reordering or a schema edit changes the
/// digest exactly when it would change the bytes a provider actually caches.
pub fn prefix_digest(system_prefix: &str, tools: &[ToolDefinition]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(system_prefix.as_bytes());
    for tool in tools {
        hasher.update(tool.name.as_bytes());
        hasher.update(tool.description.as_bytes());
        hasher.update(
            serde_json::to_vec(&tool.parameters)
                .unwrap_or_default()
                .as_slice(),
        );
    }
    format!("{:x}", hasher.finalize())
}

pub const COMPACTION_SYSTEM: &str = "\
You are a context compactor for an agent session. Produce a dense \
structured summary of the conversation so far. Keep: the original task, \
current state, what was created or changed (files with paths, plus any \
other artifact or external effect), key decisions, errors \
hit and their fixes, and open items. Drop pleasantries and redundant tool \
output. Maximum 400 words.";

pub fn compaction_prompt(transcript: &str) -> String {
    format!(
        "Summarize this session segment for continuation. The summary \
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

/// Renders messages to a readable transcript for summarization. Tool
/// calls carry their name+input (paths live there); results are labeled
/// explicitly instead of masquerading as empty user turns.
pub fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        let role = match m.role {
            vak_llm::Role::User => "user",
            vak_llm::Role::Assistant => "assistant",
        };
        let mut wrote_header = false;
        for b in &m.content {
            match b {
                ContentBlock::ToolUse { name, input, .. } => {
                    if !wrote_header {
                        out.push_str(&format!("[{role}]\n"));
                        wrote_header = true;
                    }
                    out.push_str(&format!(
                        "[tool-call] {name} {}\n",
                        serde_json::to_string(input).unwrap_or_default()
                    ));
                }
                ContentBlock::ToolResult { content, .. } => {
                    if !wrote_header {
                        out.push_str(&format!("[{role}]\n"));
                        wrote_header = true;
                    }
                    let preview: String = content.chars().take(600).collect();
                    out.push_str(&format!("[tool-result]\n{preview}\n"));
                }
                _ => {}
            }
        }
        let text = m.text_content();
        if !text.is_empty() || !wrote_header {
            out.push_str(&format!("[{role}]\n{text}\n"));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn prefix_digest_is_stable_for_identical_input() {
        let tools = vec![ToolDefinition::new(
            "read",
            "reads a file",
            serde_json::json!({}),
        )];
        assert_eq!(
            prefix_digest("You are vak.", &tools),
            prefix_digest("You are vak.", &tools)
        );
    }

    #[test]
    fn prefix_digest_changes_with_prefix_or_tools() {
        let tools = vec![ToolDefinition::new(
            "read",
            "reads a file",
            serde_json::json!({}),
        )];
        let base = prefix_digest("You are vak.", &tools);
        assert_ne!(base, prefix_digest("You are Bob.", &tools));
        assert_ne!(base, prefix_digest("You are vak.", &[]));
        let other_tools = vec![ToolDefinition::new(
            "write",
            "writes a file",
            serde_json::json!({}),
        )];
        assert_ne!(base, prefix_digest("You are vak.", &other_tools));
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
