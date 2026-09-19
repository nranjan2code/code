use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::error::LlmError;
use crate::gate::ProviderGate;
use crate::sse::SseDecoder;
use crate::stream::{EventSink, EventStream, StreamEvent, channel};
use crate::turn::{current_turn_boundary, strip_thinking};
use crate::types::{
    AssistantMessage, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition, Usage,
};

/// Anthropic accepts at most 4 `cache_control` breakpoints per request. The
/// stable system prompt always claims one when present, leaving the rest
/// for message-level breakpoints named by `ChatRequest::cache`.
const MAX_CACHE_BREAKPOINTS: usize = 4;

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[derive(Clone)]
pub struct AnthropicProvider {
    http: reqwest::Client,
    config: AnthropicConfig,
    gate: ProviderGate,
}

impl AnthropicProvider {
    pub fn new(config: AnthropicConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(AnthropicProvider {
            gate: ProviderGate::new(&config.base_url, &config.api_key),
            http,
            config,
        })
    }
}

pub fn build_body(request: &ChatRequest) -> Result<Value, LlmError> {
    let boundary = current_turn_boundary(&request.messages);
    let system_takes_a_slot = request.system.is_some();
    let message_breakpoints = select_message_breakpoints(
        request.cache.as_ref(),
        MAX_CACHE_BREAKPOINTS.saturating_sub(usize::from(system_takes_a_slot)),
    );

    let mut messages = Vec::with_capacity(request.messages.len());
    for (i, m) in request.messages.iter().enumerate() {
        validate_message(m)?;
        let stripped;
        let rendered = if i < boundary {
            stripped = strip_thinking(m);
            &stripped
        } else {
            m
        };
        let mut value =
            serde_json::to_value(rendered).map_err(|e| LlmError::Parse(e.to_string()))?;
        if message_breakpoints.contains(&i) {
            mark_last_block_ephemeral(&mut value);
        }
        messages.push(value);
    }

    let mut body = serde_json::json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "messages": messages,
        "stream": true,
    });
    if let Some(system) = &request.system {
        // Prompt caching: the system prompt is stable across turns, so mark
        // it ephemeral-cachable — long sessions stop re-paying full input
        // cost every request.
        body["system"] = serde_json::json!([{
            "type": "text",
            "text": system,
            "cache_control": {"type": "ephemeral"}
        }]);
    }
    if let Some(t) = request.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t: &ToolDefinition| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    Ok(body)
}

/// Picks which message indices get a `cache_control` breakpoint, bounded by
/// `budget`. When the hinted set is larger than the budget, the oldest
/// (lowest-index) breakpoints are dropped first — the newest history is the
/// one most likely to be replayed unchanged on the next turn.
fn select_message_breakpoints(
    cache: Option<&crate::types::CacheHints>,
    budget: usize,
) -> std::collections::BTreeSet<usize> {
    let Some(cache) = cache else {
        return std::collections::BTreeSet::new();
    };
    let mut indices: Vec<usize> = cache
        .breakpoints
        .iter()
        .filter_map(|b| b.after_message)
        .collect();
    indices.sort_unstable();
    indices.dedup();
    if indices.len() > budget {
        indices = indices.split_off(indices.len() - budget);
    }
    indices.into_iter().collect()
}

/// Attaches `cache_control: {"type": "ephemeral"}` to the last content
/// block of an already-serialized message, matching Anthropic's rule that a
/// breakpoint marks the end of the cached range.
fn mark_last_block_ephemeral(message: &mut Value) {
    if let Some(last) = message
        .get_mut("content")
        .and_then(|c| c.as_array_mut())
        .and_then(|arr| arr.last_mut())
        && let Some(obj) = last.as_object_mut()
    {
        obj.insert(
            "cache_control".to_string(),
            serde_json::json!({"type": "ephemeral"}),
        );
    }
}

fn validate_message(m: &Message) -> Result<(), LlmError> {
    for block in &m.content {
        match (m.role, block) {
            (Role::Assistant, ContentBlock::ToolResult { .. }) => {
                return Err(LlmError::InvalidRequest(
                    "tool_result blocks must appear in user messages".into(),
                ));
            }
            (Role::User, ContentBlock::ToolUse { .. }) => {
                return Err(LlmError::InvalidRequest(
                    "tool_use blocks must appear in assistant messages".into(),
                ));
            }
            (Role::Assistant, ContentBlock::Image { .. }) => {
                return Err(LlmError::InvalidRequest(
                    "image blocks must appear in user messages".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn map_status_error(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str().map(String::from))
        })
        .unwrap_or_else(|| body.chars().take(500).collect());

    match status {
        401 | 403 => LlmError::Auth(message),
        400 => LlmError::classify_400(message),
        404 | 413 | 422 => LlmError::InvalidRequest(message),
        429 => LlmError::RateLimit {
            message,
            retry_after_secs: retry_after,
        },
        503 | 529 => LlmError::Overloaded(message),
        _ => LlmError::Api { status, message },
    }
}

struct Accumulator {
    message: AssistantMessage,
    index_map: std::collections::HashMap<u64, usize>,
    raw_json: std::collections::HashMap<u64, String>,
}

impl Accumulator {
    fn new(model: &str) -> Self {
        Accumulator {
            message: AssistantMessage::empty(model),
            index_map: std::collections::HashMap::new(),
            raw_json: std::collections::HashMap::new(),
        }
    }

    fn convert(&mut self, data: &str) -> Result<Option<StreamEvent>, LlmError> {
        let v: Value = serde_json::from_str(data)
            .map_err(|e| LlmError::Parse(format!("bad event json: {e}")))?;
        let kind = v
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or_else(|| LlmError::Parse("event missing type".into()))?
            .to_string();

        match kind.as_str() {
            "ping" => Ok(None),
            "message_start" => {
                if let Some(model) = v.pointer("/message/model").and_then(|m| m.as_str()) {
                    self.message.model = model.to_string();
                }
                if let Some(usage) = parse_usage(&v) {
                    self.message.usage = usage;
                }
                Ok(Some(StreamEvent::Start {
                    partial: self.message.clone(),
                }))
            }
            "content_block_start" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                let block = v.get("content_block").cloned().unwrap_or(Value::Null);
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                let our_idx = self.message.content.len();
                match block_type {
                    "tool_use" => {
                        let id = block
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or_default()
                            .to_string();
                        self.message.content.push(ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: Value::Object(Default::default()),
                        });
                        self.index_map.insert(idx, our_idx);
                        Ok(Some(StreamEvent::ToolUseStart {
                            index: our_idx,
                            id,
                            name,
                            partial: self.message.clone(),
                        }))
                    }
                    "thinking" => {
                        self.message.content.push(ContentBlock::Thinking {
                            text: String::new(),
                            signature: None,
                        });
                        self.index_map.insert(idx, our_idx);
                        Ok(None)
                    }
                    _ => {
                        self.message.content.push(ContentBlock::text(String::new()));
                        self.index_map.insert(idx, our_idx);
                        Ok(None)
                    }
                }
            }
            "content_block_delta" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                let delta = v.get("delta").cloned().unwrap_or(Value::Null);
                let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let Some(&our_idx) = self.index_map.get(&idx) else {
                    return Ok(None);
                };
                match delta_type {
                    "text_delta" => {
                        let text = delta.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        append_text(&mut self.message.content[our_idx], text);
                        Ok(Some(StreamEvent::TextDelta {
                            delta: text.to_string(),
                            partial: self.message.clone(),
                        }))
                    }
                    "thinking_delta" => {
                        let text = delta.get("thinking").and_then(|t| t.as_str()).unwrap_or("");
                        append_thinking(&mut self.message.content[our_idx], text);
                        Ok(Some(StreamEvent::ThinkingDelta {
                            delta: text.to_string(),
                            partial: self.message.clone(),
                        }))
                    }
                    "signature_delta" => {
                        let sig = delta
                            .get("signature")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        if let ContentBlock::Thinking { signature, .. } =
                            &mut self.message.content[our_idx]
                        {
                            *signature = Some(sig.to_string());
                        }
                        Ok(None)
                    }
                    "input_json_delta" => {
                        let json = delta
                            .get("partial_json")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let raw = self.raw_json.entry(idx).or_default();
                        raw.push_str(json);
                        let parsed: Value =
                            serde_json::from_str(raw).unwrap_or(Value::Object(Default::default()));
                        if let ContentBlock::ToolUse { input, .. } =
                            &mut self.message.content[our_idx]
                        {
                            *input = parsed;
                        }
                        Ok(Some(StreamEvent::ToolInputDelta {
                            index: our_idx,
                            delta: json.to_string(),
                            partial: self.message.clone(),
                        }))
                    }
                    _ => Ok(None),
                }
            }
            "content_block_stop" => {
                let idx = v.get("index").and_then(|i| i.as_u64()).unwrap_or(0);
                if let Some(&our_idx) = self.index_map.get(&idx)
                    && let ContentBlock::ToolUse { input, .. } = &mut self.message.content[our_idx]
                {
                    let raw = self.raw_json.get(&idx).cloned().unwrap_or_default();
                    *input = if raw.trim().is_empty() {
                        Value::Object(Default::default())
                    } else {
                        serde_json::from_str(&raw).map_err(|e| {
                            LlmError::Parse(format!("tool input json invalid at block stop: {e}"))
                        })?
                    };
                }
                Ok(None)
            }
            "message_delta" => {
                if let Some(reason) = v.pointer("/delta/stop_reason").and_then(|r| r.as_str()) {
                    self.message.stop_reason = match reason {
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        _ => StopReason::EndTurn,
                    };
                }
                if let Some(out) = v.pointer("/usage/output_tokens").and_then(|u| u.as_u64()) {
                    self.message.usage.output_tokens = out;
                }
                Ok(None)
            }
            "message_stop" => Ok(Some(StreamEvent::End {
                message: self.message.clone(),
            })),
            "error" => {
                let err_type = v
                    .pointer("/error/type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("api_error");
                let msg = v
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown provider error");
                Err(match err_type {
                    "overloaded_error" => LlmError::Overloaded(msg.to_string()),
                    "rate_limit_error" => LlmError::RateLimit {
                        message: msg.to_string(),
                        retry_after_secs: None,
                    },
                    "invalid_request_error" => LlmError::InvalidRequest(msg.to_string()),
                    "authentication_error" | "permission_error" => LlmError::Auth(msg.to_string()),
                    _ => LlmError::Api {
                        status: 0,
                        message: format!("{err_type}: {msg}"),
                    },
                })
            }
            // Unknown event types are additive provider extensions; a new
            // type from the server must never fail an in-flight request.
            _other => Ok(None),
        }
    }
}

fn append_text(block: &mut ContentBlock, text: &str) {
    if let ContentBlock::Text { text: t } = block {
        t.push_str(text);
    }
}

fn append_thinking(block: &mut ContentBlock, text: &str) {
    if let ContentBlock::Thinking { text: t, .. } = block {
        t.push_str(text);
    }
}

fn parse_usage(v: &Value) -> Option<Usage> {
    let u = v.pointer("/message/usage")?;
    Some(Usage {
        input_tokens: u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        cache_read_input_tokens: u.get("cache_read_input_tokens").and_then(|x| x.as_u64()),
        cache_creation_input_tokens: u
            .get("cache_creation_input_tokens")
            .and_then(|x| x.as_u64()),
        ..Default::default()
    })
}

#[async_trait::async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn circuit_key(&self) -> String {
        crate::gate::route_identity(self.name(), &self.config.base_url, &self.config.api_key)
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let provider_permit = self.gate.acquire(&cancel).await?;
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let body = build_body(&request)?;
        let send_fut = self
            .http
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send();
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
            r = send_fut => match r {
                Ok(r) => r,
                Err(e) => return Err(LlmError::Network(e.to_string())),
            },
        };

        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            let text = response.text().await.unwrap_or_default();
            return Err(map_status_error(status.as_u16(), &text, retry_after));
        }

        let model = request.model.clone();
        let (sink, stream_rx) = channel(256);
        let mut byte_stream = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut acc = Accumulator::new(&model);

        tokio::spawn(async move {
            drive_stream(&mut byte_stream, &mut decoder, &mut acc, sink, cancel).await;
        });

        Ok(stream_rx.with_guard(provider_permit))
    }
}

async fn drive_stream<S>(
    byte_stream: &mut S,
    decoder: &mut SseDecoder,
    acc: &mut Accumulator,
    mut sink: EventSink,
    cancel: CancellationToken,
) where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin,
{
    let mut saw_end = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let partial = (!acc.message.content.is_empty()).then(|| Box::new(acc.message.clone()));
                sink.close_error(LlmError::Aborted { partial }).await;
                return;
            }
            chunk = byte_stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        decoder.push(&bytes);
                        while let Some(frame) = decoder.next_frame() {
                            match acc.convert(&frame.data) {
                                Ok(Some(event)) => {
                                    if matches!(event, StreamEvent::End { .. }) {
                                        saw_end = true;
                                    }
                                    sink.push(event);
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    sink.close_error(e).await;
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        sink.close_error(LlmError::Network(e.to_string())).await;
                        return;
                    }
                    None => {
                        if saw_end {
                            sink.close_message(acc.message.clone()).await;
                        } else {
                            sink.close_error(LlmError::Parse(
                                "stream closed before message_stop".into(),
                            )).await;
                        }
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod build_body_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::types::{CacheBreakpoint, CacheHints};

    fn message_with_thinking(text: &str) -> Message {
        Message::assistant(vec![
            ContentBlock::Thinking {
                text: "reasoning".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::text(text),
        ])
    }

    #[test]
    fn no_cache_hints_only_caches_the_system_prompt() {
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.system = Some("be helpful".into());
        req.messages = vec![Message::user_text("hi")];
        let body = build_body(&req).unwrap();
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert!(body["messages"][0].get("cache_control").is_none());
    }

    #[test]
    fn breakpoints_mark_the_last_block_of_named_messages() {
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.system = Some("be helpful".into());
        req.messages = vec![
            Message::user_text("first"),
            Message::assistant(vec![ContentBlock::text("first answer")]),
            Message::user_text("second"),
        ];
        req.cache = Some(CacheHints {
            session_key: "s1".into(),
            breakpoints: vec![CacheBreakpoint {
                after_message: Some(1),
            }],
        });
        let body = build_body(&req).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[1]["content"][0]["cache_control"]["type"], "ephemeral");
        assert!(msgs[0].get("cache_control").is_none());
        assert!(msgs[2]["content"][0].get("cache_control").is_none());
    }

    #[test]
    fn breakpoint_budget_drops_the_oldest_first_when_over_four_total() {
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.system = Some("be helpful".into()); // claims one of the 4 slots
        req.messages = (0..6)
            .map(|i| Message::user_text(format!("m{i}")))
            .collect();
        req.cache = Some(CacheHints {
            session_key: "s1".into(),
            breakpoints: (0..6)
                .map(|i| CacheBreakpoint {
                    after_message: Some(i),
                })
                .collect(),
        });
        let body = build_body(&req).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        let cached: Vec<usize> = (0..6)
            .filter(|&i| msgs[i]["content"][0].get("cache_control").is_some())
            .collect();
        // 4 total breakpoints minus the system prompt's slot leaves 3
        // message breakpoints, keeping the newest (highest-index) ones.
        assert_eq!(cached, vec![3, 4, 5]);
    }

    #[test]
    fn thinking_before_the_current_turn_is_stripped() {
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.messages = vec![
            Message::user_text("first"),
            message_with_thinking("first answer"),
            Message::user_text("second, a fresh directive"),
            message_with_thinking("second answer"),
        ];
        let body = build_body(&req).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        // Turn 1 (indices 0-1) is closed: thinking must not survive.
        assert!(
            !msgs[1]["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["type"] == "thinking")
        );
        // Turn 2 (indices 2-3) is the current turn: thinking is replayed.
        assert!(
            msgs[3]["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["type"] == "thinking")
        );
    }

    #[test]
    fn thinking_survives_mid_turn_tool_continuation() {
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.messages = vec![
            Message::user_text("do the thing"),
            Message::assistant(vec![
                ContentBlock::Thinking {
                    text: "reasoning".into(),
                    signature: Some("sig".into()),
                },
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "search".into(),
                    input: serde_json::json!({}),
                },
            ]),
            Message {
                role: Role::User,
                content: vec![ContentBlock::tool_result("t1", "result")],
            },
        ];
        let body = build_body(&req).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        // The whole exchange is one turn (the tool-result message is not a
        // fresh directive), so thinking in message 1 must still be present.
        assert!(
            msgs[1]["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["type"] == "thinking")
        );
    }
}
