//! OpenAI Responses API adapter (`POST /v1/responses`) — the native wire
//! format for GPT-5.x-class models. Distinct from chat-completions: typed
//! content parts, `function_call` items, and event-name-keyed SSE deltas.

use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::error::LlmError;
use crate::sse::SseDecoder;
use crate::stream::{EventStream, StreamEvent, channel};
use crate::types::{
    AssistantMessage, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition, Usage,
};

pub const OPENAI_RESPONSES_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone)]
pub struct OpenAiResponsesConfig {
    pub api_key: String,
    pub base_url: String,
}

#[derive(Clone)]
pub struct OpenAiResponsesProvider {
    http: reqwest::Client,
    config: OpenAiResponsesConfig,
}

impl OpenAiResponsesProvider {
    pub fn new(config: OpenAiResponsesConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(OpenAiResponsesProvider { http, config })
    }
}

pub fn build_body(request: &ChatRequest) -> Result<Value, LlmError> {
    let mut input: Vec<Value> = Vec::with_capacity(request.messages.len());
    for m in &request.messages {
        append_input_item(&mut input, m)?;
    }

    let mut body = serde_json::json!({
        "model": request.model,
        "input": input,
        "stream": true,
        "store": false,
    });
    if let Some(system) = &request.system {
        body["instructions"] = Value::String(system.clone());
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t: &ToolDefinition| {
                serde_json::json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    Ok(body)
}

fn append_input_item(out: &mut Vec<Value>, m: &Message) -> Result<(), LlmError> {
    match m.role {
        Role::User => {
            let mut text = String::new();
            let mut outputs: Vec<(String, String)> = Vec::new();
            for b in &m.content {
                match b {
                    ContentBlock::Text { text: t } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => outputs.push((tool_use_id.clone(), content.clone())),
                    ContentBlock::ToolUse { .. } => {
                        return Err(LlmError::InvalidRequest(
                            "tool_use blocks must appear in assistant messages".into(),
                        ));
                    }
                    ContentBlock::Thinking { .. } => {}
                }
            }
            for (call_id, output) in outputs {
                out.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
            if !text.is_empty() {
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [{"type": "input_text", "text": text}],
                }));
            }
        }
        Role::Assistant => {
            let mut text = String::new();
            for b in &m.content {
                match b {
                    ContentBlock::Text { text: t } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                    ContentBlock::ToolUse {
                        id,
                        name,
                        input: args,
                    } => {
                        out.push(serde_json::json!({
                            "type": "function_call",
                            "call_id": id,
                            "name": name,
                            "arguments": serde_json::to_string(args)
                                .map_err(|e| LlmError::Parse(e.to_string()))?,
                        }));
                    }
                    ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } => {}
                }
            }
            if !text.is_empty() {
                out.push(serde_json::json!({
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text}],
                }));
            }
        }
    }
    Ok(())
}

fn map_status_error(status: u16, body: &str) -> LlmError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str().map(String::from))
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    match status {
        401 | 403 => LlmError::Auth(message),
        400 | 404 | 413 | 422 => LlmError::InvalidRequest(message),
        429 => LlmError::RateLimit {
            message,
            retry_after_secs: None,
        },
        503 | 529 => LlmError::Overloaded(message),
        _ => LlmError::Api { status, message },
    }
}

struct Accumulator {
    message: AssistantMessage,
    saw_completed: bool,
    /// call_id → position of the ToolUse block in `message.content`.
    tool_pos: std::collections::HashMap<String, usize>,
    raw_json: std::collections::HashMap<String, String>,
}

impl Accumulator {
    fn new(model: &str) -> Self {
        Accumulator {
            message: AssistantMessage::empty(model),
            saw_completed: false,
            tool_pos: std::collections::HashMap::new(),
            raw_json: std::collections::HashMap::new(),
        }
    }

    fn convert(&mut self, data: &str) -> Result<Option<StreamEvent>, LlmError> {
        let v: Value =
            serde_json::from_str(data).map_err(|e| LlmError::Parse(format!("bad chunk: {e}")))?;
        let kind = v
            .get("type")
            .and_then(|t| t.as_str())
            .ok_or_else(|| LlmError::Parse("responses event missing type".into()))?
            .to_string();

        match kind.as_str() {
            "response.output_text.delta" => {
                let Some(text) = v.get("delta").and_then(|d| d.as_str()) else {
                    return Ok(None);
                };
                append_text_block(&mut self.message.content, text);
                Ok(Some(StreamEvent::TextDelta {
                    delta: text.to_string(),
                    partial: self.message.clone(),
                }))
            }
            "response.output_item.added" => {
                let item = v.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    // Any emitted function_call means the model wants tools.
                    self.message.stop_reason = StopReason::ToolUse;
                    let call_id = item
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let pos = self.message.content.len();
                    self.message.content.push(ContentBlock::ToolUse {
                        id: call_id.clone(),
                        name: name.clone(),
                        input: Value::Object(Default::default()),
                    });
                    self.tool_pos.insert(call_id.clone(), pos);
                    return Ok(Some(StreamEvent::ToolUseStart {
                        index: pos,
                        id: call_id,
                        name,
                        partial: self.message.clone(),
                    }));
                }
                Ok(None)
            }
            "response.function_call_arguments.delta" => {
                let Some(delta) = v.get("delta").and_then(|d| d.as_str()) else {
                    return Ok(None);
                };
                let call_id = v
                    .get("item_id")
                    .or_else(|| v.get("call_id"))
                    .and_then(|c| c.as_str())
                    .unwrap_or_default();
                // The added-item's `id` is used as item_id; map through both.
                let pos = self.resolve_tool_pos(call_id);
                let Some(pos) = pos else {
                    return Ok(None);
                };
                let raw = self.raw_json.entry(call_id.to_string()).or_default();
                raw.push_str(delta);
                let parsed: Value =
                    serde_json::from_str(raw).unwrap_or(Value::Object(Default::default()));
                if let ContentBlock::ToolUse { input, .. } = &mut self.message.content[pos] {
                    *input = parsed;
                }
                Ok(Some(StreamEvent::ToolInputDelta {
                    index: pos,
                    delta: delta.to_string(),
                    partial: self.message.clone(),
                }))
            }
            "response.completed" | "response.incomplete" => {
                if let Some(usage) = v.pointer("/response/usage") {
                    self.message.usage = Usage {
                        input_tokens: usage
                            .get("input_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0),
                        output_tokens: usage
                            .get("output_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0),
                        cache_read_input_tokens: usage
                            .pointer("/input_tokens_details/cached_tokens")
                            .and_then(|x| x.as_u64()),
                        cache_creation_input_tokens: None,
                    };
                }
                if kind == "response.incomplete" {
                    self.message.stop_reason = StopReason::MaxTokens;
                }
                self.saw_completed = true;
                Ok(Some(StreamEvent::End {
                    message: self.message.clone(),
                }))
            }
            _ => Ok(None),
        }
    }

    fn resolve_tool_pos(&self, call_or_item_id: &str) -> Option<usize> {
        if let Some(pos) = self.tool_pos.get(call_or_item_id) {
            return Some(*pos);
        }
        // item ids look like "fc_..."; the block id stores the call_id. Fall
        // back to the most recent tool block when only one exists.
        if self.tool_pos.len() == 1 {
            return self.tool_pos.values().next().copied();
        }
        None
    }
}

fn append_text_block(content: &mut Vec<ContentBlock>, text: &str) {
    if let Some(ContentBlock::Text { text: last }) = content.last_mut() {
        last.push_str(text);
        return;
    }
    content.push(ContentBlock::text(text));
}

#[async_trait::async_trait]
impl Provider for OpenAiResponsesProvider {
    fn name(&self) -> &str {
        "openai-responses"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let url = format!("{}/responses", self.config.base_url.trim_end_matches('/'));
        let body = build_body(&request)?;
        let send_fut = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
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
            let text = response.text().await.unwrap_or_default();
            return Err(map_status_error(status.as_u16(), &text));
        }

        let model = request.model.clone();
        let (mut sink, stream_rx) = channel(256);
        let mut byte_stream = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut acc = Accumulator::new(&model);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        let partial = (!acc.message.content.is_empty()).then(|| acc.message.clone());
                        sink.close_error(LlmError::Aborted { partial }).await;
                        return;
                    }
                    chunk = byte_stream.next() => {
                        match chunk {
                            Some(Ok(bytes)) => {
                                decoder.push(&bytes);
                                while let Some(frame) = decoder.next_frame() {
                                    match acc.convert(&frame.data) {
                                        Ok(Some(event)) => sink.push(event),
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
                                if acc.saw_completed {
                                    sink.close_message(acc.message.clone()).await;
                                } else {
                                    sink.close_error(LlmError::Parse(
                                        "stream closed before response.completed".into(),
                                    )).await;
                                }
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok(stream_rx)
    }
}
