//! Google Gemini adapter (`models/{model}:streamGenerateContent?alt=sse`).
//!
//! Wire quirks handled here: roles are `user`/`model`; function responses
//! ride in a user turn keyed by function NAME (resolved from prior assistant
//! `functionCall` parts); tool args are JSON objects, not strings; SSE
//! chunks carry complete `functionCall` objects.

use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::error::LlmError;
use crate::gate::ProviderGate;
use crate::sse::SseDecoder;
use crate::stream::{EventStream, StreamEvent, channel};
use crate::types::{AssistantMessage, ChatRequest, ContentBlock, Role, StopReason, ToolDefinition};

pub const GOOGLE_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub api_key: String,
    pub base_url: String,
}

#[derive(Clone)]
pub struct GoogleProvider {
    http: reqwest::Client,
    config: GoogleConfig,
    gate: ProviderGate,
}

impl GoogleProvider {
    pub fn new(config: GoogleConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(GoogleProvider {
            gate: ProviderGate::new(&config.base_url, &config.api_key),
            http,
            config,
        })
    }
}

pub fn build_body(request: &ChatRequest) -> Result<Value, LlmError> {
    let mut contents: Vec<Value> = Vec::with_capacity(request.messages.len());
    // function_call ids → names, resolved while walking history so
    // functionResponse parts can be keyed correctly.
    let mut id_to_name: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for m in &request.messages {
        match m.role {
            Role::User => {
                let mut parts: Vec<Value> = Vec::new();
                let mut text = String::new();
                let mut images: Vec<&crate::types::ImageSource> = Vec::new();
                for b in &m.content {
                    match b {
                        ContentBlock::Text { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        ContentBlock::Image { source } => images.push(source),
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            let name = id_to_name
                                .get(tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| tool_use_id.clone());
                            parts.push(serde_json::json!({
                                "functionResponse": {
                                    "name": name,
                                    "response": {"result": content},
                                }
                            }));
                        }
                        ContentBlock::ToolUse { .. } => {
                            return Err(LlmError::InvalidRequest(
                                "tool_use blocks must appear in assistant messages".into(),
                            ));
                        }
                        ContentBlock::Thinking { .. } => {}
                    }
                }
                if !text.is_empty() {
                    parts.push(serde_json::json!({"text": text}));
                }
                for img in images {
                    parts.push(serde_json::json!({
                        "inline_data": {"mime_type": img.media_type, "data": img.data}
                    }));
                }
                if !parts.is_empty() {
                    contents.push(serde_json::json!({"role": "user", "parts": parts}));
                }
            }
            Role::Assistant => {
                let mut parts: Vec<Value> = Vec::new();
                let mut text = String::new();
                let mut pending_sig: Option<String> = None;
                for b in &m.content {
                    match b {
                        ContentBlock::Text { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        ContentBlock::Thinking { signature, .. } => {
                            if let Some(sig) = signature {
                                pending_sig = Some(sig.clone());
                            }
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            // Flush pending text first so parts preserve
                            // assistant source order.
                            if !text.is_empty() {
                                let mut text_part = serde_json::json!({"text": text});
                                if let Some(sig) = &pending_sig {
                                    text_part["thoughtSignature"] = serde_json::json!(sig);
                                }
                                parts.push(text_part);
                                text = String::new();
                            }
                            id_to_name.insert(id.clone(), name.clone());
                            let mut call_part = serde_json::json!({
                                "functionCall": {
                                    "name": name,
                                    "args": input,
                                }
                            });
                            if let Some(sig) = &pending_sig {
                                call_part["thoughtSignature"] = serde_json::json!(sig);
                            }
                            parts.push(call_part);
                        }
                        ContentBlock::ToolResult { .. }
                        | ContentBlock::Image { .. } => {}
                    }
                }
                if !text.is_empty() {
                    let mut text_part = serde_json::json!({"text": text});
                    if let Some(sig) = &pending_sig {
                        text_part["thoughtSignature"] = serde_json::json!(sig);
                    }
                    parts.push(text_part);
                }
                if !parts.is_empty() {
                    contents.push(serde_json::json!({"role": "model", "parts": parts}));
                }
            }
        }
    }

    let mut body = serde_json::json!({ "contents": contents });
    if let Some(system) = &request.system {
        body["systemInstruction"] = serde_json::json!({"parts": [{"text": system}]});
    }
    if !request.tools.is_empty() {
        let decls: Vec<Value> = request
            .tools
            .iter()
            .map(|t: &ToolDefinition| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": sanitize_schema(&t.parameters),
                })
            })
            .collect();
        body["tools"] = serde_json::json!([{ "functionDeclarations": decls }]);
    }
    Ok(body)
}

fn sanitize_schema(val: &Value) -> Value {
    match val {
        Value::Object(map) => {
            let mut cleaned = serde_json::Map::new();
            for (k, v) in map {
                if matches!(
                    k.as_str(),
                    "additionalProperties"
                        | "$schema"
                        | "patternProperties"
                        | "definitions"
                        | "$defs"
                ) {
                    continue;
                }
                cleaned.insert(k.clone(), sanitize_schema(v));
            }
            Value::Object(cleaned)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sanitize_schema).collect()),
        other => other.clone(),
    }
}

fn map_status_error(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
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
            retry_after_secs: retry_after,
        },
        503 | 529 => LlmError::Overloaded(message),
        _ => LlmError::Api { status, message },
    }
}

struct Accumulator {
    message: AssistantMessage,
    saw_finish: bool,
}

impl Accumulator {
    fn new(model: &str) -> Self {
        Accumulator {
            message: AssistantMessage::empty(model),
            saw_finish: false,
        }
    }

    fn convert(&mut self, data: &str) -> Result<Option<StreamEvent>, LlmError> {
        let v: Value =
            serde_json::from_str(data).map_err(|e| LlmError::Parse(format!("bad chunk: {e}")))?;

        if let Some(err) = v.get("error") {
            return Err(LlmError::Api {
                status: 0,
                message: err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown gemini error")
                    .to_string(),
            });
        }

        if let Some(usage) = v.get("usageMetadata") {
            self.message.usage.input_tokens = usage
                .get("promptTokenCount")
                .and_then(|x| x.as_u64())
                .unwrap_or(self.message.usage.input_tokens);
            self.message.usage.output_tokens = usage
                .get("candidatesTokenCount")
                .and_then(|x| x.as_u64())
                .unwrap_or(self.message.usage.output_tokens);
        }

        let Some(candidate) = v.pointer("/candidates/0") else {
            return Ok(None);
        };

        let mut event: Option<StreamEvent> = None;

        if let Some(parts) = candidate
            .pointer("/content/parts")
            .and_then(|p| p.as_array())
        {
            for part in parts {
                let sig = part
                    .get("thoughtSignature")
                    .or_else(|| part.get("thought_signature"))
                    .or_else(|| {
                        part.get("functionCall").and_then(|c| {
                            c.get("thoughtSignature")
                                .or_else(|| c.get("thought_signature"))
                        })
                    })
                    .and_then(|s| s.as_str())
                    .map(String::from);

                if let Some(signature) = sig {
                    self.message.content.push(ContentBlock::Thinking {
                        text: String::new(),
                        signature: Some(signature),
                    });
                }

                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    if text.is_empty() {
                        continue;
                    }
                    append_text_block(&mut self.message.content, text);
                    event = Some(StreamEvent::TextDelta {
                        delta: text.to_string(),
                        partial: self.message.clone(),
                    });
                }
                if let Some(call) = part.get("functionCall") {
                    let name = call
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let input = call
                        .get("args")
                        .cloned()
                        .unwrap_or(Value::Object(Default::default()));
                    let pos = self.message.content.len();
                    self.message.content.push(ContentBlock::ToolUse {
                        id: format!("gemini-call-{pos}"),
                        name: name.clone(),
                        input,
                    });
                    self.message.stop_reason = StopReason::ToolUse;
                    event = Some(StreamEvent::ToolUseStart {
                        index: pos,
                        id: format!("gemini-call-{pos}"),
                        name,
                        partial: self.message.clone(),
                    });
                }
            }
        }

        if let Some(finish) = candidate.get("finishReason").and_then(|f| f.as_str()) {
            if self.message.stop_reason != StopReason::ToolUse {
                self.message.stop_reason = match finish {
                    "MAX_TOKENS" => StopReason::MaxTokens,
                    _ => StopReason::EndTurn,
                };
            }
            self.saw_finish = true;
            return Ok(Some(StreamEvent::End {
                message: self.message.clone(),
            }));
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
impl Provider for GoogleProvider {
    fn name(&self) -> &str {
        "google"
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
        let url = format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.config.base_url.trim_end_matches('/'),
            request.model
        );
        let body = build_body(&request)?;
        let send_fut = self
            .http
            .post(&url)
            .header("x-goog-api-key", &self.config.api_key)
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
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            let text = response.text().await.unwrap_or_default();
            return Err(map_status_error(status.as_u16(), &text, retry_after));
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
                                if acc.saw_finish {
                                    sink.close_message(acc.message.clone()).await;
                                } else {
                                    sink.close_error(LlmError::Parse(
                                        "stream closed before finishReason".into(),
                                    )).await;
                                }
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok(stream_rx.with_guard(provider_permit))
    }
}
