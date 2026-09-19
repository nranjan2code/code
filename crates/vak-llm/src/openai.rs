use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::error::LlmError;
use crate::gate::ProviderGate;
use crate::sse::SseDecoder;
use crate::stream::{EventStream, StreamEvent, channel};
use crate::types::{
    AssistantMessage, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition, Usage,
};

pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone, Default)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub base_url: String,
    /// When true, and the request carries `ChatRequest::cache`, send
    /// `prompt_cache_key` so the provider can route repeat traffic to the
    /// same cache-warm backend.
    pub cache_key: bool,
    /// When true, also send OpenRouter's `session_id` alongside
    /// `prompt_cache_key` — OpenRouter accepts both and uses `session_id`
    /// to pin the upstream that holds the cache.
    pub openrouter: bool,
}

/// Batch transcription through the OpenAI-compatible `/audio/transcriptions`
/// endpoint. The model is supplied by discovery/configuration; this adapter
/// deliberately has no baked-in model catalogue or default.
pub async fn transcribe(
    config: &OpenAiConfig,
    audio: &[u8],
    mime: &str,
    model: &str,
    cancel: &CancellationToken,
) -> Result<String, LlmError> {
    if audio.is_empty() || mime.trim().is_empty() || model.trim().is_empty() {
        return Err(LlmError::InvalidRequest(
            "audio, mime, and model are required".into(),
        ));
    }
    if cancel.is_cancelled() {
        return Err(LlmError::Aborted { partial: None });
    }
    let filename = if mime.contains("wav") {
        "audio.wav"
    } else if mime.contains("mpeg") || mime.contains("mp3") {
        "audio.mp3"
    } else if mime.contains("ogg") {
        "audio.ogg"
    } else if mime.contains("oga") {
        "audio.oga"
    } else if mime.contains("webm") {
        "audio.webm"
    } else if mime.contains("mp4") || mime.contains("m4a") {
        "audio.m4a"
    } else if mime.contains("flac") {
        "audio.flac"
    } else {
        "audio.bin"
    };
    let part = reqwest::multipart::Part::bytes(audio.to_vec())
        .file_name(filename)
        .mime_str(mime)
        .map_err(|e| LlmError::InvalidRequest(e.to_string()))?;
    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", model.to_string());
    let url = format!(
        "{}/audio/transcriptions",
        config.base_url.trim_end_matches('/')
    );
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        result = reqwest::Client::new().post(url).bearer_auth(&config.api_key).multipart(form).send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
    };
    let status = response.status().as_u16();
    let value: Value = response
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;
    if status >= 400 {
        return Err(LlmError::InvalidRequest(format!(
            "transcription provider returned HTTP {status}: {value}"
        )));
    }
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(LlmError::Parse(
            "provider returned an empty transcript".into(),
        ));
    }
    Ok(text)
}

/// Synthesize speech through an OpenAI-compatible `/audio/speech` endpoint.
/// The model is always explicit and the response is bounded before it is
/// materialized, so a misbehaving provider cannot exhaust the process.
pub async fn speak(
    config: &OpenAiConfig,
    text: &str,
    model: &str,
    voice: Option<&str>,
    format: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, LlmError> {
    if text.trim().is_empty() || model.trim().is_empty() || format.trim().is_empty() {
        return Err(LlmError::InvalidRequest(
            "text, model, and format are required".into(),
        ));
    }
    if cancel.is_cancelled() {
        return Err(LlmError::Aborted { partial: None });
    }
    let mut body = serde_json::json!({
        "model": model,
        "input": text,
        "response_format": format,
    });
    if let Some(voice) = voice.filter(|v| !v.trim().is_empty()) {
        body["voice"] = Value::String(voice.to_string());
    }
    let url = format!("{}/audio/speech", config.base_url.trim_end_matches('/'));
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        result = reqwest::Client::new().post(url).bearer_auth(&config.api_key).json(&body).send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
    };
    let status = response.status().as_u16();
    if status >= 400 {
        let body = response.text().await.unwrap_or_default();
        return Err(map_status_error(status, &body, None));
    }
    const MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_AUDIO_BYTES as u64)
    {
        return Err(LlmError::InvalidRequest(
            "provider audio exceeds 16 MiB".into(),
        ));
    }
    let mut output = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        chunk = stream.next() => chunk,
    } {
        let chunk = chunk.map_err(|e| LlmError::Network(e.to_string()))?;
        if output.len().saturating_add(chunk.len()) > MAX_AUDIO_BYTES {
            return Err(LlmError::InvalidRequest(
                "provider audio exceeds 16 MiB".into(),
            ));
        }
        output.extend_from_slice(&chunk);
    }
    if output.is_empty() {
        return Err(LlmError::Parse("provider returned empty audio".into()));
    }
    Ok(output)
}

#[derive(Clone)]
pub struct OpenAiCompletionsProvider {
    http: reqwest::Client,
    config: OpenAiConfig,
    gate: ProviderGate,
}

impl OpenAiCompletionsProvider {
    pub fn new(config: OpenAiConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(OpenAiCompletionsProvider {
            http,
            gate: ProviderGate::new(&config.base_url, &config.api_key),
            config,
        })
    }
}

pub fn build_body(config: &OpenAiConfig, request: &ChatRequest) -> Result<Value, LlmError> {
    // Chat Completions carries no reasoning-item channel, so `Thinking`
    // blocks are dropped unconditionally by `append_message` below — there
    // is no turn boundary to compute here (contrast Anthropic/Google, which
    // must replay signed thinking within the current turn).
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
    if let Some(cache) = &request.cache {
        if config.cache_key {
            body["prompt_cache_key"] = serde_json::json!(cache.session_key);
        }
        if config.openrouter {
            body["session_id"] = serde_json::json!(cache.session_key);
        }
    }
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
            let mut image_parts: Vec<String> = Vec::new();
            let mut tool_results: Vec<&ContentBlock> = Vec::new();
            for b in &m.content {
                match b {
                    ContentBlock::Text { text } => text_parts.push(text),
                    ContentBlock::Image { source } => {
                        // Data URLs are the transport-agnostic form for the
                        // chat-completions API.
                        image_parts
                            .push(format!("data:{};base64,{}", source.media_type, source.data));
                    }
                    ContentBlock::ToolResult { .. } => tool_results.push(b),
                    ContentBlock::ToolUse { .. } => {
                        return Err(LlmError::InvalidRequest(
                            "tool_use blocks must appear in assistant messages".into(),
                        ));
                    }
                    ContentBlock::Thinking { .. } => {}
                    // Only the Anthropic adapter understands server-side
                    // tool search; every other adapter skips this opaque
                    // block entirely (docs/design/68 §5/§12).
                    ContentBlock::Provider { .. } => {}
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
            if !image_parts.is_empty() || !text_parts.is_empty() {
                let content = if image_parts.is_empty() {
                    serde_json::json!(text_parts.join("\n"))
                } else {
                    let mut parts: Vec<Value> = text_parts
                        .iter()
                        .map(|t| serde_json::json!({"type": "text", "text": t}))
                        .collect();
                    for url in &image_parts {
                        parts.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": {"url": url}
                        }));
                    }
                    serde_json::json!(parts)
                };
                out.push(serde_json::json!({
                    "role": "user",
                    "content": content,
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
                    ContentBlock::Thinking { .. }
                    | ContentBlock::ToolResult { .. }
                    | ContentBlock::Image { .. }
                    | ContentBlock::Provider { .. } => {}
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

fn map_status_error(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
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
        400 => match LlmError::classify_400(message.clone()) {
            over_length @ LlmError::Context(_) => over_length,
            _ => LlmError::invalid_request_for_endpoint("/v1/chat/completions", message),
        },
        404 | 413 | 422 => LlmError::invalid_request_for_endpoint("/v1/chat/completions", message),
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
                ..Default::default()
            };
        }

        let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
            return Ok(None);
        };

        #[allow(clippy::dbg_macro)]
        if std::env::var_os("VAK_LLM_DEBUG").is_some() {
            eprintln!(
                "[vak-llm] frame finish={:?} delta_keys={:?} content_len={}",
                choice.get("finish_reason"),
                choice.get("delta").map(|d| d
                    .as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()),
                self.message.content.len()
            );
        }
        if let Some(finish) = choice.get("finish_reason").and_then(|f| f.as_str()) {
            // Accumulated tool_use blocks are ground truth: some compat
            // endpoints close tool-call turns with finish reasons outside
            // the canonical set (e.g. plain "stop"). Trusting the label
            // would strand a dangling tool_use and kill the run.
            let has_tool_use = self
                .message
                .content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
            self.message.stop_reason = if has_tool_use {
                StopReason::ToolUse
            } else {
                match finish {
                    "tool_calls" | "function_call" => StopReason::ToolUse,
                    "length" => StopReason::MaxTokens,
                    _ => StopReason::EndTurn,
                }
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
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let body = build_body(&self.config, &request)?;
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
                        let partial = (!acc.message.content.is_empty()).then(|| Box::new(acc.message.clone()));
                        sink.close_error(LlmError::Aborted { partial }).await;
                        return;
                    }
                    chunk = byte_stream.next() => {
                        match chunk {
                            Some(Ok(bytes)) => {
                                decoder.push(&bytes);
                                while let Some(frame) = decoder.next_frame() {
                                    let data = frame.data.trim();
                                    if data == "[DONE]" {
                                        // Content is ground truth here as
                                        // everywhere else: endpoints that
                                        // skip finish_reason entirely (seen
                                        // on opencode-zen) still owe the loop
                                        // a ToolUse signal when tool calls
                                        // were streamed.
                                        let has_tool_use = acc.message.content.iter().any(
                                            |b| matches!(b, ContentBlock::ToolUse { .. }),
                                        );
                                        if has_tool_use {
                                            acc.message.stop_reason = StopReason::ToolUse;
                                        }
                                        sink.close_message(acc.message.clone()).await;
                                        return;
                                    }
                                    match acc.convert(data) {
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
                                // OpenAI-compatible proxies sometimes end the
                                // body after the last content chunk without
                                // [DONE]/finish_reason. A clean close with
                                // content is de facto completion (same as the
                                // [DONE] branch); empty content fails closed.
                                if acc.saw_end || !acc.message.content.is_empty() {
                                    // Content is ground truth here too: a
                                    // stream ending right after tool-call
                                    // deltas must surface ToolUse, or the
                                    // dangling call aborts the run.
                                    let has_tool_use = acc.message.content.iter().any(
                                        |b| matches!(b, ContentBlock::ToolUse { .. }),
                                    );
                                    if has_tool_use {
                                        acc.message.stop_reason = StopReason::ToolUse;
                                    }
                                    sink.close_message(acc.message.clone()).await;
                                } else {
                                    sink.close_error(LlmError::Parse(
                                        "stream closed before finish_reason".into(),
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

#[cfg(test)]
mod build_body_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::types::CacheHints;

    fn req_with_cache() -> ChatRequest {
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![Message::user_text("hi")];
        req.cache = Some(CacheHints {
            session_key: "sess-1".into(),
            breakpoints: Vec::new(),
        });
        req
    }

    #[test]
    fn cache_key_off_sends_neither_hint() {
        let config = OpenAiConfig::default();
        let body = build_body(&config, &req_with_cache()).unwrap();
        assert!(body.get("prompt_cache_key").is_none());
        assert!(body.get("session_id").is_none());
    }

    #[test]
    fn cache_key_on_sends_prompt_cache_key_only() {
        let config = OpenAiConfig {
            cache_key: true,
            ..Default::default()
        };
        let body = build_body(&config, &req_with_cache()).unwrap();
        assert_eq!(body["prompt_cache_key"], "sess-1");
        assert!(body.get("session_id").is_none());
    }

    #[test]
    fn openrouter_flag_adds_session_id_alongside_prompt_cache_key() {
        let config = OpenAiConfig {
            cache_key: true,
            openrouter: true,
            ..Default::default()
        };
        let body = build_body(&config, &req_with_cache()).unwrap();
        assert_eq!(body["prompt_cache_key"], "sess-1");
        assert_eq!(body["session_id"], "sess-1");
    }

    #[test]
    fn no_cache_hint_on_request_sends_nothing_even_when_enabled() {
        let config = OpenAiConfig {
            api_key: String::new(),
            base_url: String::new(),
            cache_key: true,
            openrouter: true,
        };
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![Message::user_text("hi")];
        let body = build_body(&config, &req).unwrap();
        assert!(body.get("prompt_cache_key").is_none());
        assert!(body.get("session_id").is_none());
    }
}
