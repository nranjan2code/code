//! The request assembler (docs/design/68-context-engine.md §6/§10): the
//! byte-stable prefix and its digest, the per-turn tail attached to the
//! last user message, cache breakpoints, the char accounting every
//! `CapacityProfile` estimate is fed from, and the summariser request
//! behind a compaction packet.
//!
//! Budgeting policy lives in `capacity` (`CapacityProfile`) and `planner`
//! (`WorkingSetPlanner`); this module never decides what to send, only
//! how the chosen bytes are laid out; every token estimate goes through
//! `CapacityProfile::estimate_tokens`, fed by the char counts here.

use sha2::{Digest, Sha256};
use vak_llm::{
    CacheBreakpoint, ChatRequest, ContentBlock, Message, Role, ToolDefinition,
    current_turn_boundary,
};
use vak_session::TailSections;

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
    // A summary, not a deliberation: measured live, thinking made no
    // difference to the summary and cost 3x the latency.
    req.think = Some(false);
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

/// Host-supplied per-turn content for the request tail (docs/design/68-
/// context-engine.md §6/§10): the clock instant and the epistemic stance,
/// each rendered under its own tag alongside the session-derived tail
/// sections. Captured once per turn by the caller, not recomputed per step,
/// so the tail stays byte-identical across every step of one turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TailInput {
    /// Raw temporal context sentence, with no wrapping tag.
    pub temporal: String,
    /// Raw epistemic-stance text, with no wrapping tag.
    pub stance: String,
}

/// Attaches the turn's tail to the last user message
/// (docs/design/68-context-engine.md §6): after any `tool_result` blocks and
/// BEFORE any text, so the last thing the model reads is the user's own
/// words (the directive, or a runtime nudge) and never the runtime's
/// context. Observed live: with the tail appended after the directive, a
/// small model answered the `<stance>` block ("As an analytical agent, I can
/// handle tasks…") instead of the question. The tail never restates the
/// directive: an echo after a tool result reads as the user asking again
/// (measured live: "since the user is asking again…" followed by the same
/// card re-emitted up to nineteen times).
pub fn attach_tail(messages: &mut [Message], tail: &str) {
    if tail.is_empty() {
        return;
    }
    let Some(last) = messages.last_mut() else {
        return;
    };
    if last.role != Role::User {
        return;
    }
    let first_text = last
        .content
        .iter()
        .position(|block| matches!(block, ContentBlock::Text { .. }));
    let at = first_text.unwrap_or(last.content.len());
    last.content
        .insert(at, ContentBlock::text(tail.to_string()));
}

pub fn compose_tail(tail: &TailInput, sections: &TailSections) -> String {
    let mut out = String::new();
    let push_block = |out: &mut String, block: &str| {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(block);
    };
    if !tail.temporal.trim().is_empty() {
        push_block(
            &mut out,
            &format!("<turn_context>\n{}\n</turn_context>", tail.temporal.trim()),
        );
    }
    if let Some(intent) = &sections.intent {
        push_block(&mut out, intent);
    }
    if !tail.stance.trim().is_empty() {
        push_block(
            &mut out,
            &format!("<stance>\n{}\n</stance>", tail.stance.trim()),
        );
    }
    if let Some(work_contract) = &sections.work_contract {
        push_block(&mut out, work_contract);
    }
    if let Some(workspace) = &sections.workspace {
        push_block(&mut out, workspace);
    }
    if let Some(thread) = &sections.thread {
        push_block(&mut out, thread);
    }
    out
}

/// Cache breakpoints per §10: after the stable prefix, after the last
/// message of any previous turn, and on the last message of the request
/// being built (which, within a turn, moves forward with every step).
pub fn cache_breakpoints(messages: &[Message]) -> Vec<CacheBreakpoint> {
    let mut positions: Vec<Option<usize>> = vec![None];
    if !messages.is_empty() {
        let boundary = current_turn_boundary(messages);
        if boundary > 0 {
            positions.push(Some(boundary - 1));
        }
        positions.push(Some(messages.len() - 1));
    }
    positions.dedup();
    positions
        .into_iter()
        .map(|after_message| CacheBreakpoint { after_message })
        .collect()
}

/// Sum of text/tool_use/tool_result characters actually sent in `request` —
/// what `CapacityProfile::observe_usage` calibrates `tokens_per_char`
/// against (docs/design/68-context-engine.md §1 "Feedback"). Thinking and
/// image blocks are excluded: no provider bills prefill on them the way it
/// does on text, and images would swamp the char count relative to the
/// tokens they actually cost.
pub fn chat_request_chars(request: &ChatRequest) -> u64 {
    let system_chars = request.system.as_deref().map(str::len).unwrap_or(0) as u64;
    system_chars + messages_chars(&request.messages)
}

/// Sum of text/tool_use/tool_result characters in one message — the same
/// exclusions as `chat_request_chars` (thinking and images are never billed
/// like text on prefill).
pub fn message_chars(message: &Message) -> u64 {
    message
        .content
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text } => text.len() as u64,
            ContentBlock::ToolUse { input, .. } => input.to_string().len() as u64,
            ContentBlock::ToolResult { content, .. } => content.len() as u64,
            ContentBlock::Provider { raw, .. } => raw.to_string().len() as u64,
            ContentBlock::Thinking { .. } | ContentBlock::Image { .. } => 0,
        })
        .sum()
}

pub fn messages_chars(messages: &[Message]) -> u64 {
    messages.iter().map(message_chars).sum()
}

/// Character count of the stable prefix (system prompt + tool schemas),
/// turned into tokens by `CapacityProfile::estimate_tokens`
/// (docs/design/68-context-engine.md §4/§6).
pub fn prefix_chars(system: &str, tools: &[ToolDefinition]) -> u64 {
    let mut chars = system.len() as u64;
    for tool in tools {
        chars += (tool.name.len() + tool.description.len()) as u64
            + serde_json::to_string(&tool.parameters)
                .map(|s| s.len() as u64)
                .unwrap_or(0);
    }
    chars
}

/// Relative-change threshold for writing a `capacity-feedback` activity
/// (docs/design/68 §6): small usage-to-usage jitter in a measured EWMA
/// should not spam the ledger with an activity every turn.
pub const CAPACITY_FEEDBACK_CHANGE_THRESHOLD: f64 = 0.05;

pub fn relative_change(before: f64, after: f64) -> f64 {
    if before == 0.0 {
        if after == 0.0 { 0.0 } else { 1.0 }
    } else {
        ((after - before) / before).abs()
    }
}

/// Fields of a `CapacityProfile` that changed by more than
/// `CAPACITY_FEEDBACK_CHANGE_THRESHOLD`, rendered for an `Activity`'s
/// `data` map. Empty means nothing worth recording changed.
pub fn capacity_feedback_delta(
    before: &crate::capacity::CapacityProfile,
    after: &crate::capacity::CapacityProfile,
) -> std::collections::BTreeMap<String, String> {
    let mut delta = std::collections::BTreeMap::new();
    if relative_change(before.tokens_per_char.value, after.tokens_per_char.value)
        > CAPACITY_FEEDBACK_CHANGE_THRESHOLD
    {
        delta.insert(
            "tokens_per_char".into(),
            format!(
                "{} -> {}",
                before.tokens_per_char.value, after.tokens_per_char.value
            ),
        );
    }
    if relative_change(before.prefill_tps.value, after.prefill_tps.value)
        > CAPACITY_FEEDBACK_CHANGE_THRESHOLD
    {
        delta.insert(
            "prefill_tps".into(),
            format!(
                "{} -> {}",
                before.prefill_tps.value, after.prefill_tps.value
            ),
        );
    }
    if before.instruction_horizon.tokens != after.instruction_horizon.tokens {
        delta.insert(
            "instruction_horizon_tokens".into(),
            format!(
                "{} -> {}",
                before.instruction_horizon.tokens, after.instruction_horizon.tokens
            ),
        );
    }
    delta
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
