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
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            parameters,
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

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub cache: Option<CacheHints>,
    /// When set, an OpenAI Responses adapter chains from this prior
    /// response instead of replaying the full history: only messages after
    /// the last assistant message are sent, alongside this id.
    pub previous_response_id: Option<String>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>) -> Self {
        ChatRequest {
            model: model.into(),
            system: None,
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: 8192,
            temperature: None,
            cache: None,
            previous_response_id: None,
        }
    }
}
