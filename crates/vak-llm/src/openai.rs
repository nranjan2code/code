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

pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub base_url: String,
}

#[derive(Clone)]
pub struct OpenAiCompletionsProvider {
    http: reqwest::Client,
    config: OpenAiConfig,
}

impl OpenAiCompletionsProvider {
    pub fn new(config: OpenAiConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(OpenAiCompletionsProvider { http, config })
    }
}

pub fn build_body(request: &ChatRequest) -> Result<Value, LlmError> {
    let mut messages: Vec<Value> = Vec::with_capacity(request.messages.len() + 1);
    if let Some(system) = &request.system {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    for m in &request.messages {
        append_message(&mut messages, m)?;
    }

    let mut body = serde_json::json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t: &ToolDefinition| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    },
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }
    Ok(body)
}

fn append_message(out: &mut Vec<Value>, m: &Message) -> Result<(), LlmError> {
    match m.role {
        Role::User => {
            let mut text_parts: Vec<&str> = Vec::new();
            let mut tool_results: Vec<&ContentBlock> = Vec::new();
            for b in &m.content {
                match b {
                    ContentBlock::Text { text } => text_parts.push(text),
                    ContentBlock::ToolResult { .. } => tool_results.push(b),
                    ContentBlock::ToolUse { .. } => {
                        return Err(LlmError::InvalidRequest(
                            "tool_use blocks must appear in assistant messages".into(),
                        ));
                    }
                    ContentBlock::Thinking { .. } => {}
                }
            }
            for r in tool_results {
                let ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = r
                else {
                    unreachable!()
                };
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": content,
                }));
            }
            if !text_parts.is_empty() {
                out.push(serde_json::json!({
                    "role": "user",
                    "content": text_parts.join("\n"),
                }));
            }
        }
        Role::Assistant => {
            let mut text = String::new();
            let mut tool_calls: Vec<Value> = Vec::new();
            for b in &m.content {
                match b {
                    ContentBlock::Text { text: t } => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(serde_json::json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": serde_json::to_string(input)
                                    .map_err(|e| LlmError::Parse(e.to_string()))?,
                            },
                        }));
                    }
                    ContentBlock::Thinking { .. } | ContentBlock::ToolResult { .. } => {}
                }
            }
            let mut msg = serde_json::json!({"role": "assistant"});
            if !text.is_empty() || tool_calls.is_empty() {
                msg["content"] = Value::String(text);
            } else {
                msg["content"] = Value::Null;
            }
            if !tool_calls.is_empty() {
                msg["tool_calls"] = Value::Array(tool_calls);
            }
            out.push(msg);
        }
    }
    Ok(())
}

fn map_status_error(status: u16, body: &str) -> LlmError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.pointer("/message"))
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
    saw_end: bool,
    tool_pos: std::collections::HashMap<usize, usize>,
    raw_json: std::collections::HashMap<usize, String>,
}

impl Accumulator {
    fn new(model: &str) -> Self {
        Accumulator {
            message: AssistantMessage::empty(model),
            saw_end: false,
            tool_pos: std::collections::HashMap::new(),
            raw_json: std::collections::HashMap::new(),
        }
    }

    fn convert(&mut self, data: &str) -> Result<Option<StreamEvent>, LlmError> {
        let v: Value = serde_json::from_str(data)
            .map_err(|e| LlmError::Parse(format!("bad chunk json: {e}")))?;

        if let Some(usage) = v.get("usage").filter(|u| !u.is_null()) {
            self.message.usage = Usage {
                input_tokens: usage
                    .get("prompt_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
                output_tokens: usage
                    .get("completion_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
                cache_read_input_tokens: usage
                    .get("prompt_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(|x| x.as_u64()),
                cache_creation_input_tokens: None,
            };
        }

        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
            return Ok(None);
        };

        if let Some(finish) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            self.message.stop_reason = match finish {
                "tool_calls" | "function_call" => StopReason::ToolUse,
                "length" => StopReason::MaxTokens,
                _ => StopReason::EndTurn,
            };
            self.saw_end = true;
            return Ok(Some(StreamEvent::End {
                message: self.message.clone(),
            }));
        }

        let Some(delta) = choice.get("delta") else {
            return Ok(None);
        };

        let mut event = None;
        if let Some(text) = delta
            .get("content")
            .and_then(|c| c.as_str())
            .filter(|t| !t.is_empty())
        {
            append_text_block(&mut self.message.content, text);
            event = Some(StreamEvent::TextDelta {
                delta: text.to_string(),
                partial: self.message.clone(),
            });
        }

        if let Some(calls) = delta.get("tool_calls").and_then(|c| c.as_array()) {
            for call in calls {
                let idx = call.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                let pos = match self.tool_pos.get(&idx) {
                    Some(&p) => p,
                    None => {
                        self.message.content.push(ContentBlock::ToolUse {
                            id: String::new(),
                            name: String::new(),
                            input: Value::Object(Default::default()),
                        });
                        let p = self.message.content.len() - 1;
                        self.tool_pos.insert(idx, p);
                        p
                    }
                };
                if let ContentBlock::ToolUse { id, name, .. } = &mut self.message.content[pos] {
                    if let Some(new_id) = call.get("id").and_then(|i| i.as_str()) {
                        *id = new_id.to_string();
                    }
                    if let Some(fname) = call.pointer("/function/name").and_then(|n| n.as_str()) {
                        *name = fname.to_string();
                    }
                    if let Some(args) = call.pointer("/function/arguments").and_then(|a| a.as_str())
                    {
                        let raw = self.raw_json.entry(idx).or_default();
                        raw.push_str(args);
                        let parsed: Value =
                            serde_json::from_str(raw).unwrap_or(Value::Object(Default::default()));
                        if let ContentBlock::ToolUse { input, .. } = &mut self.message.content[pos]
                        {
                            *input = parsed;
                        }
                    }
                }
                if event.is_none()
                    && let Some(ContentBlock::ToolUse { id, name, .. }) =
                        self.message.content.get(pos)
                {
                    event = Some(StreamEvent::ToolUseStart {
                        index: pos,
                        id: id.clone(),
                        name: name.clone(),
                        partial: self.message.clone(),
                    });
                }
            }
        }

        Ok(event)
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
impl Provider for OpenAiCompletionsProvider {
    fn name(&self) -> &str {
        "openai-completions"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
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
        let (sink, stream_rx) = channel(256);
        let mut byte_stream = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut acc = Accumulator::new(&model);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        let partial = (!acc.message.content.is_empty()).then(|| acc.message.clone());
                        sink.close_error(LlmError::Aborted { partial });
                        return;
                    }
                    chunk = byte_stream.next() => {
                        match chunk {
                            Some(Ok(bytes)) => {
                                decoder.push(&bytes);
                                while let Some(frame) = decoder.next_frame() {
                                    let data = frame.data.trim();
                                    if data == "[DONE]" {
                                        sink.close_message(acc.message.clone());
                                        return;
                                    }
                                    match acc.convert(data) {
                                        Ok(Some(event)) => sink.push(event),
                                        Ok(None) => {}
                                        Err(e) => {
                                            sink.close_error(e);
                                            return;
                                        }
                                    }
                                }
                            }
                            Some(Err(e)) => {
                                sink.close_error(LlmError::Network(e.to_string()));
                                return;
                            }
                            None => {
                                if acc.saw_end {
                                    sink.close_message(acc.message.clone());
                                } else {
                                    sink.close_error(LlmError::Parse(
                                        "stream closed before finish_reason".into(),
                                    ));
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
