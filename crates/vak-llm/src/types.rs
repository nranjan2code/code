use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// Base64 image in Anthropic's native wire shape — the serde
    /// passthrough in anthropic.rs sends it verbatim. User messages only.
    Image {
        source: ImageSource,
    },
    /// A provider-native block this build does not interpret: Anthropic's
    /// `server_tool_use` and `tool_search_tool_result`, and the Responses
    /// wire's `tool_search_call` and `tool_search_output` items
    /// (docs/design/68 §5/§11/§12). `raw` is the exact block the provider
    /// sent, `"type"` included; `kind` mirrors `raw["type"]` for cheap
    /// matching without re-parsing. Never executed by the agent loop:
    /// persisted, and sent back only by the adapter whose wire wrote it.
    /// Every adapter skips a kind that is not its own.
    Provider {
        kind: String,
        raw: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSource {
    pub r#type: String,
    pub media_type: String,
    pub data: String,
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }

    /// Base64-encoded image block (vision input).
    pub fn image_base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        ContentBlock::Image {
            source: ImageSource {
                r#type: "base64".into(),
                media_type: media_type.into(),
                data: data.into(),
            },
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: false,
        }
    }

    pub fn tool_error(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(s: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![ContentBlock::text(s)],
        }
    }

    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Message {
            role: Role::Assistant,
            content,
        }
    }

    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "input_schema")]
    pub parameters: Value,
    /// Scheduling hint (docs/design/68 §5/§11): an adapter that defers
    /// tools (`Provider::defers_tools`) renders `defer_loading: true`, keeps
    /// the schema out of the request and offers the provider's tool search.
    /// Every other adapter never reads it, and its caller sends it no
    /// deferred tool.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub defer: bool,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            parameters,
            defer: false,
        }
    }

    /// The single tool offered on every horizon-ladder probe rung
    /// (docs/design/68-context-engine.md §1): a no-op the model can only
    /// reach by following the instruction placed at the end of the probe
    /// prompt, so "was it called" is a clean instruction-following signal
    /// independent of the filler content around it.
    pub fn probe_ack() -> Self {
        ToolDefinition::new(
            "probe_ack",
            "Acknowledge that you read this far. Call this with {\"ok\": true} \
             and nothing else.",
            serde_json::json!({
                "type": "object",
                "properties": { "ok": { "type": "boolean" } },
                "required": ["ok"],
            }),
        )
    }

    /// Same tool, marked deferred (`defer_loading` on a wire that has it).
    pub fn deferred(mut self) -> Self {
        self.defer = true;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    /// Provider-reported prefill (prompt evaluation) latency, when the
    /// provider reports it directly rather than only through usage counts
    /// (Ollama's `prompt_eval_duration`). Used by the capacity probe to
    /// measure prefill throughput without inferring it from wall-clock time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill_ms: Option<u64>,
    /// Provider-reported model load latency folded into the same response
    /// (Ollama's `load_duration`), so the probe can separate "model was
    /// already warm" from "cold load" when explaining a slow first token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_ms: Option<u64>,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// Every prompt token the provider actually processed for this
    /// request, regardless of cache tier: `input_tokens` (never cached) +
    /// `cache_read_input_tokens` (served from cache) +
    /// `cache_creation_input_tokens` (written to cache). Every adapter
    /// normalizes to that split (docs/design/68-context-engine.md §1), so
    /// this is the number to use wherever a caller wants "how big was the
    /// prompt" rather than "how much fresh compute did it cost" —
    /// calibrating chars-per-token against `input_tokens` alone collapses
    /// toward zero as cache hits grow, because a full cache hit reports
    /// `input_tokens == 0` for a prompt that was not remotely empty.
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens
            + self.cache_read_input_tokens.unwrap_or(0)
            + self.cache_creation_input_tokens.unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
    pub stop_reason: StopReason,
    pub usage: Usage,
    pub model: String,
    /// Provider-assigned identity for this response, when the provider
    /// exposes one (OpenAI Responses `response.id`). `previous_response_id`
    /// on a later `ChatRequest` chains from this value to avoid replaying
    /// the turn's history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
}

impl AssistantMessage {
    pub fn empty(model: impl Into<String>) -> Self {
        AssistantMessage {
            content: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: model.into(),
            response_id: None,
        }
    }

    pub fn into_message(self) -> Message {
        Message {
            role: Role::Assistant,
            content: self.content,
        }
    }

    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A cache breakpoint's position relative to `ChatRequest::messages`.
/// `None` places it immediately after the stable prefix (system + tools),
/// before any message — the position a fresh session with no history yet
/// still wants cached. `Some(i)` places it after `messages[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheBreakpoint {
    pub after_message: Option<usize>,
}

/// Cache hints the assembler attaches to a request; each provider adapter
/// renders them in its own wire shape (§10/§11 of docs/design/68). Absent
/// entirely, adapters fall back to their unconditional defaults (e.g.
/// Anthropic still marks the system prompt ephemeral).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CacheHints {
    /// Stable identity for this session, sent as a routing/cache key to
    /// providers that key cache reuse off an opaque string rather than
    /// content-addressing the prefix (OpenAI `prompt_cache_key`, OpenRouter
    /// `session_id`).
    pub session_key: String,
    pub breakpoints: Vec<CacheBreakpoint>,
}

/// Reasoning depth, rendered by each adapter in its own wire's terms
/// (docs/design/01-llm.md "Reasoning effort"): Anthropic
/// `output_config.effort`, OpenAI Responses `reasoning.effort`, chat
/// completions `reasoning_effort`, Gemini `thinkingConfig.thinkingLevel`.
/// The native Ollama adapter ignores it: measured, a card call written
/// without thinking failed 20 of 20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::XHigh => "xhigh",
            Effort::Max => "max",
        }
    }

    /// The level on a wire that has three: anything above high is high.
    pub fn three_level(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High | Effort::XHigh | Effort::Max => "high",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    /// The output budget, when there is one: an operator cap or the bound
    /// model's published limit. `None` sends no cap where the provider
    /// allows it (the reply then shares the model's window); an adapter
    /// whose API requires one fills in the model's published maximum.
    pub max_tokens: Option<u32>,
    /// The tokens this request needs, its measured prompt estimate plus the
    /// room left for the reply, when the caller measured it. A provider that
    /// reserves a window per request (Ollama's `num_ctx`) sizes it to this
    /// rather than to the model's maximum, which reserved 16 GB on a 17 GB
    /// machine (docs/design/68-context-engine.md §8).
    pub context_need: Option<u64>,
    pub temperature: Option<f32>,
    pub cache: Option<CacheHints>,
    /// When set, an OpenAI Responses adapter chains from this prior
    /// response instead of replaying the full history: only messages after
    /// the last assistant message are sent, alongside this id.
    pub previous_response_id: Option<String>,
    /// Whether the model may spend tokens in a thinking channel before
    /// answering. `None` leaves the provider's default; `Some(false)` asks
    /// for a direct answer (Ollama `think`), which a strict-JSON
    /// classification needs — measured live, a thinking model spent its
    /// whole output budget deliberating and returned no JSON at all.
    /// Adapters without such a switch ignore it. On Anthropic this never
    /// disables thinking (current models reject that, or silently degrade
    /// tool-call reliability on the ones that still accept it) — instead,
    /// when `effort` is unset, the adapter reads `think == Some(false)` as
    /// "spend as little as possible" and sends `effort: low`.
    pub think: Option<bool>,
    /// Explicit reasoning depth (see [`Effort`]). `None` leaves the
    /// provider's default, except that the Anthropic adapter falls back to
    /// `Low` when `think == Some(false)` (see `think`).
    pub effort: Option<Effort>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>) -> Self {
        ChatRequest {
            model: model.into(),
            system: None,
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: None,
            context_need: None,
            temperature: None,
            cache: None,
            previous_response_id: None,
            think: None,
            effort: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn prompt_tokens_sums_fresh_and_both_cache_tiers() {
        let usage = Usage {
            input_tokens: 100,
            cache_read_input_tokens: Some(4_000),
            cache_creation_input_tokens: Some(300),
            ..Default::default()
        };
        assert_eq!(usage.prompt_tokens(), 4_400);
    }

    #[test]
    fn prompt_tokens_on_a_full_cache_hit_is_not_zero() {
        // input_tokens == 0 is what a 100%-cached prompt reports; the whole
        // point of `prompt_tokens` is that it does not collapse to zero here
        // the way reading `input_tokens` alone would.
        let usage = Usage {
            input_tokens: 0,
            cache_read_input_tokens: Some(12_000),
            cache_creation_input_tokens: None,
            ..Default::default()
        };
        assert_eq!(usage.prompt_tokens(), 12_000);
    }

    #[test]
    fn prompt_tokens_with_no_cache_fields_equals_input_tokens() {
        let usage = Usage {
            input_tokens: 42,
            ..Default::default()
        };
        assert_eq!(usage.prompt_tokens(), 42);
    }

    #[test]
    fn effort_renders_the_documented_wire_strings() {
        assert_eq!(Effort::Low.as_str(), "low");
        assert_eq!(Effort::Medium.as_str(), "medium");
        assert_eq!(Effort::High.as_str(), "high");
        assert_eq!(Effort::XHigh.as_str(), "xhigh");
        assert_eq!(Effort::Max.as_str(), "max");
    }

    #[test]
    fn chat_request_defaults_carry_no_effort() {
        let req = ChatRequest::new("m");
        assert_eq!(req.effort, None);
        assert_eq!(req.think, None);
    }
}
