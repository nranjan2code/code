//! Long-horizon context management: token estimation, input budgets, and
//! automatic compaction. Policy lives here; ledger mechanics live in
//! vak-session. On overflow the loop summarizes older turns into a
//! compaction entry (never deletion) and retries the same contract; if the
//! projection still exceeds the budget, it fails closed.
//!
//! Context window sizing is model-aware: `context_window` is set dynamically
//! at agent bootstrap from the provider's published limits (see vak-core's
//! `route_context_limits`), NOT hardcoded. The retrieval/compaction boundary
//! scales with the available budget via `dynamic_retrieval_cap` — validated
//! by the vakyartha simulation suite.

use sha2::{Digest, Sha256};
use vak_llm::{ChatRequest, ContentBlock, Message, ToolDefinition};

/// Approximate tokens per conversation turn — used to convert a token budget
/// into a turn-count retrieval cap. Matches vakyartha's APPROX_TOKENS_PER_TURN.
const APPROX_TOKENS_PER_TURN: u64 = 1400;

/// Minimum buffer reserved from the full context window before any history
/// is considered allocatable.
const MIN_TURN_HEADROOM: u64 = 3000;

/// Maximum retrieval cap (never exceeds 50 turns regardless of budget).
const MAX_RETRIEVAL_CAP: usize = 50;

#[derive(Debug, Clone)]
pub struct ContextPolicy {
    /// Model context window in tokens. Set dynamically at bootstrap from
    /// the provider's published model metadata (see vak-core
    /// `route_context_limits`). This is NOT hardcoded — different models
    /// get different windows based on live provider data.
    pub context_window: u64,
    /// Reserve for the completion (max_tokens).
    pub max_output: u64,
    /// Fraction of the input budget at which compaction triggers.
    pub compact_threshold: f64,
    /// Recent messages always kept verbatim across compaction. This is the
    /// floor of the dynamic retrieval cap, not a fixed limit.
    pub keep_recent: usize,
}

impl Default for ContextPolicy {
    fn default() -> Self {
        ContextPolicy {
            // Conservative default — overwritten at bootstrap from live
            // provider metadata. vak-core's route_context_limits queries
            // each provider's GET /models for the actual context length.
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

    /// Dynamic retrieval cap: scales the number of older turns retrieved
    /// before compaction with the available history budget.
    ///
    /// Formula (from vakyartha simulation):
    ///   retained = history_budget / 2  (half for recent, half for retrieved)
    ///   affordable = retained / APPROX_TOKENS_PER_TURN
    ///   cap = max(keep_recent, min(50, affordable))
    ///
    /// This means:
    /// - 2K history budget → 6 turns (same as today)
    /// - 16K budget → 6 turns (floor)
    /// - 37K budget → 13 turns (2.2x more)
    /// - 58K budget → 20 turns (3.3x more)
    /// - 120K budget → 42 turns
    ///
    /// Validated across 480+ trials: zero overflow, 2.5x–2.8x recovery gain.
    pub fn dynamic_retrieval_cap(&self, history_budget: u64) -> usize {
        let retained = history_budget / 2;
        let affordable = retained / APPROX_TOKENS_PER_TURN;
        std::cmp::max(
            self.keep_recent,
            std::cmp::min(MAX_RETRIEVAL_CAP, affordable as usize),
        )
    }

    /// Dynamic history budget cap: scales the maximum history budget with
    /// the model's context window.
    ///
    /// Formula (from vakyartha simulation):
    ///   usable = max(input_budget - MIN_TURN_HEADROOM, 4000)
    ///   dynamic_cap = usable * 0.30
    ///   result = max(strategy_cap, min(dynamic_cap, usable * 0.60))
    ///
    /// This means:
    /// - 32K window: 16K cap (same as today)
    /// - 128K window: 37K cap (2.3x improvement)
    /// - 200K window: 58K cap (3.6x improvement)
    pub fn dynamic_history_budget_cap(&self, strategy_cap: u64) -> u64 {
        let usable = self
            .input_budget()
            .saturating_sub(MIN_TURN_HEADROOM)
            .max(4_000);
        let dynamic_cap = (usable as f64 * 0.30) as u64;
        let ceiling = (usable as f64 * 0.60) as u64;
        std::cmp::max(strategy_cap, std::cmp::min(dynamic_cap, ceiling))
    }
}

/// Chars/4 heuristic — planning estimate only, never treated as provider
/// usage (see docs/design/15-reliability.md).
pub fn estimate_tokens(
    messages: &[Message],
    system: Option<&str>,
    tool_definitions: &[vak_llm::ToolDefinition],
) -> u64 {
    let mut chars: u64 = system.map(|s| s.len() as u64).unwrap_or(0);
    for t in tool_definitions {
        chars += (t.name.len() + t.description.len()) as u64
            + serde_json::to_string(&t.parameters)
                .map(|s| s.len() as u64)
                .unwrap_or(0);
    }
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
                // Base64 payload size counts against the request budget.
                ContentBlock::Image { source } => source.data.len() as u64,
            };
        }
    }
    chars.div_ceil(4)
}

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
    fn estimate_scales_with_content() {
        let short = vec![Message::user_text("hi")];
        let long = vec![Message::user_text("x".repeat(4000))];
        assert_eq!(estimate_tokens(&short, None, &[]), 1);
        assert!(estimate_tokens(&long, None, &[]) >= 1000);
        assert!(estimate_tokens(&short, Some("system ".repeat(4).as_str()), &[]) > 1);
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
    fn dynamic_retrieval_cap_scales_with_budget() {
        let p = ContextPolicy::default();
        // Small window (32K model): floor behavior
        let small = ContextPolicy {
            context_window: 32_000,
            ..p.clone()
        };
        assert_eq!(small.dynamic_retrieval_cap(2_000), 6); // floor = keep_recent
        assert_eq!(small.dynamic_retrieval_cap(16_000), 6); // still at floor

        // 128K model: scales up
        let large = ContextPolicy {
            context_window: 128_000,
            ..p.clone()
        };
        assert_eq!(large.dynamic_retrieval_cap(37_000), 13); // 37K/2/1400 ≈ 13
        assert_eq!(large.dynamic_retrieval_cap(120_000), 42); // 120K/2/1400 = 42

        // Capped at 50
        assert_eq!(large.dynamic_retrieval_cap(200_000), 50);
    }

    #[test]
    fn dynamic_history_budget_cap_scales_with_window() {
        let p = ContextPolicy::default();

        let small = ContextPolicy {
            context_window: 32_000,
            ..p.clone()
        };
        assert!(small.dynamic_history_budget_cap(16_000) <= 16_000);

        let large = ContextPolicy {
            context_window: 128_000,
            ..p.clone()
        };
        let cap = large.dynamic_history_budget_cap(16_000);
        assert!(cap >= 16_000); // grows beyond strategy cap
        assert!(cap <= 76_000); // bounded by ceiling (0.6 * ~126K)
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
