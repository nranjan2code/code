//! Native Ollama adapter (`POST /api/chat`, streaming NDJSON) — replaces the
//! OpenAI-compatible route for Ollama so `keep_alive` and `options.num_ctx`
//! reach the server: the compat endpoint silently ignores both
//! (docs/design/68-context-engine.md §8).

use futures::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::error::LlmError;
use crate::gate::ProviderGate;
use crate::stream::{EventStream, StreamEvent, channel};
use crate::types::{
    AssistantMessage, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition, Usage,
};

pub const OLLAMA_DEFAULT_BASE_URL: &str = "http://localhost:11434";

#[derive(Debug, Clone)]
pub struct OllamaConfig {
    /// Empty for a local, unauthenticated server; set when Ollama sits
    /// behind an auth-terminating proxy.
    pub api_key: String,
    pub base_url: String,
    /// Sent on every request so the runner does not evict the model between
    /// turns under the default 5-minute idle unload.
    pub keep_alive: String,
    /// `options.num_ctx`; omitted from the request body when `None` so the
    /// server's own modelfile default applies.
    pub num_ctx: Option<u64>,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        OllamaConfig {
            api_key: String::new(),
            base_url: OLLAMA_DEFAULT_BASE_URL.to_string(),
            keep_alive: "30m".to_string(),
            num_ctx: None,
        }
    }
}

#[derive(Clone)]
pub struct OllamaProvider {
    http: reqwest::Client,
    config: OllamaConfig,
    gate: ProviderGate,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(OllamaProvider {
            gate: ProviderGate::new(&config.base_url, &config.api_key),
            http,
            config,
        })
    }
}

/// `window` is the model's own context length, used for `num_ctx` when the
/// operator pinned none: without it the server's small default window cut
/// Vak's prompt short.
pub fn build_body(
    config: &OllamaConfig,
    request: &ChatRequest,
    window: Option<u64>,
) -> Result<Value, LlmError> {
    let mut messages: Vec<Value> = Vec::with_capacity(request.messages.len() + 1);
    if let Some(system) = &request.system {
        messages.push(serde_json::json!({"role": "system", "content": system}));
    }
    // A result names the call it answers and that call's tool: Ollama's
    // renderers pair them by these fields, and without them gemma4's
    // template labelled every result `response:unknown`.
    let tool_names: std::collections::HashMap<&str, &str> = request
        .messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, .. } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    for m in &request.messages {
        append_message(&mut messages, m, &tool_names)?;
    }

    let mut options = serde_json::Map::new();
    if let Some(num_ctx) = config.num_ctx.or(window) {
        options.insert("num_ctx".to_string(), serde_json::json!(num_ctx));
    }
    if let Some(max_tokens) = request.max_tokens {
        options.insert("num_predict".to_string(), serde_json::json!(max_tokens));
    }

    let mut body = serde_json::json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        "keep_alive": config.keep_alive,
        "options": options,
    });
    if let Some(think) = request.think {
        body["think"] = Value::Bool(think);
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

fn append_message(
    out: &mut Vec<Value>,
    m: &Message,
    tool_names: &std::collections::HashMap<&str, &str>,
) -> Result<(), LlmError> {
    match m.role {
        Role::User => {
            let mut text_parts: Vec<&str> = Vec::new();
            let mut images: Vec<&str> = Vec::new();
            let mut tool_results: Vec<&ContentBlock> = Vec::new();
            for b in &m.content {
                match b {
                    ContentBlock::Text { text } => text_parts.push(text),
                    // Ollama's native wire wants raw base64, unlike the
                    // compat endpoint's data-URL form.
                    ContentBlock::Image { source } => images.push(source.data.as_str()),
                    ContentBlock::ToolResult { .. } => tool_results.push(b),
                    ContentBlock::ToolUse { .. } => {
                        return Err(LlmError::InvalidRequest(
                            "tool_use blocks must appear in assistant messages".into(),
                        ));
                    }
                    // No provider needs thinking replayed across the wire
                    // back to it; Ollama is no exception (docs/design/68 §10).
                    ContentBlock::Thinking { .. } => {}
                    // Only the Anthropic adapter understands server-side
                    // tool search; Ollama skips this opaque block entirely
                    // (docs/design/68 §5/§12).
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
                let mut msg = serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": content,
                });
                if let Some(name) = tool_names.get(tool_use_id.as_str()) {
                    msg["tool_name"] = Value::String((*name).to_string());
                }
                out.push(msg);
            }
            if !text_parts.is_empty() || !images.is_empty() {
                let mut msg = serde_json::json!({
                    "role": "user",
                    "content": text_parts.join("\n"),
                });
                if !images.is_empty() {
                    msg["images"] = serde_json::json!(images);
                }
                out.push(msg);
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
                            "function": {"name": name, "arguments": input},
                        }));
                    }
                    ContentBlock::Thinking { .. }
                    | ContentBlock::ToolResult { .. }
                    | ContentBlock::Image { .. }
                    | ContentBlock::Provider { .. } => {}
                }
            }
            let mut msg = serde_json::json!({"role": "assistant", "content": text});
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
        .and_then(|v| v.get("error").and_then(|m| m.as_str().map(String::from)))
        .unwrap_or_else(|| body.chars().take(500).collect());
    match status {
        401 | 403 => LlmError::Auth(message),
        400 => LlmError::classify_400(message),
        413 => LlmError::Context(message),
        404 | 422 => LlmError::InvalidRequest(message),
        429 => LlmError::RateLimit {
            message,
            retry_after_secs: None,
        },
        503 | 529 => LlmError::Overloaded(message),
        _ => LlmError::Api { status, message },
    }
}

/// Splits a byte stream into newline-delimited JSON records — Ollama's
/// `/api/chat` wire format has no `data:`/event framing, just one JSON
/// object per line.
#[derive(Default)]
struct NdjsonDecoder {
    buf: Vec<u8>,
    cursor: usize,
}

impl NdjsonDecoder {
    fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    fn next_line(&mut self) -> Option<String> {
        loop {
            let Some(nl) = self.buf[self.cursor..].iter().position(|&b| b == b'\n') else {
                self.compact();
                return None;
            };
            let start = self.cursor;
            let end = self.cursor + nl;
            let mut line = &self.buf[start..end];
            self.cursor = end + 1;
            if line.last() == Some(&b'\r') {
                line = &line[..line.len() - 1];
            }
            if line.is_empty() {
                continue;
            }
            let text = String::from_utf8_lossy(line).into_owned();
            self.compact();
            return Some(text);
        }
    }

    fn compact(&mut self) {
        if self.cursor > 0 {
            self.buf.drain(..self.cursor);
            self.cursor = 0;
        }
    }
}

struct Accumulator {
    message: AssistantMessage,
    /// Count of tool-call blocks materialized so far, used to synthesize
    /// `call_<n>` ids when Ollama omits them.
    tool_calls_seen: usize,
}

impl Accumulator {
    fn new(model: &str) -> Self {
        Accumulator {
            message: AssistantMessage::empty(model),
            tool_calls_seen: 0,
        }
    }

    fn convert(&mut self, line: &str) -> Result<Option<StreamEvent>, LlmError> {
        let v: Value = serde_json::from_str(line)
            .map_err(|e| LlmError::Parse(format!("bad ndjson line: {e}")))?;

        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            return Err(LlmError::classify_400(err.to_string()));
        }

        let mut event = None;
        if let Some(message) = v.get("message") {
            if let Some(text) = message
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
            if let Some(thinking) = message
                .get("thinking")
                .and_then(|c| c.as_str())
                .filter(|t| !t.is_empty())
            {
                append_thinking_block(&mut self.message.content, thinking);
                event = Some(StreamEvent::ThinkingDelta {
                    delta: thinking.to_string(),
                    partial: self.message.clone(),
                });
            }
            if let Some(calls) = message.get("tool_calls").and_then(|c| c.as_array()) {
                for call in calls {
                    let name = call
                        .pointer("/function/name")
                        .and_then(|n| n.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let input = call
                        .pointer("/function/arguments")
                        .cloned()
                        .unwrap_or(Value::Object(Default::default()));
                    let id = call
                        .get("id")
                        .and_then(|i| i.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{}", self.tool_calls_seen));
                    self.tool_calls_seen += 1;
                    let pos = self.message.content.len();
                    self.message.content.push(ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input,
                    });
                    self.message.stop_reason = StopReason::ToolUse;
                    event = Some(StreamEvent::ToolUseStart {
                        index: pos,
                        id,
                        name,
                        partial: self.message.clone(),
                    });
                }
            }
        }

        if v.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
            self.message.usage = Usage {
                input_tokens: v
                    .get("prompt_eval_count")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
                output_tokens: v.get("eval_count").and_then(|x| x.as_u64()).unwrap_or(0),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                prefill_ms: v
                    .get("prompt_eval_duration")
                    .and_then(|x| x.as_u64())
                    .map(|ns| ns / 1_000_000),
                load_ms: v
                    .get("load_duration")
                    .and_then(|x| x.as_u64())
                    .map(|ns| ns / 1_000_000),
            };
            if self.message.stop_reason != StopReason::ToolUse {
                self.message.stop_reason = match v.get("done_reason").and_then(|r| r.as_str()) {
                    Some("length") => StopReason::MaxTokens,
                    _ => StopReason::EndTurn,
                };
            }
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

fn append_thinking_block(content: &mut Vec<ContentBlock>, text: &str) {
    if let Some(ContentBlock::Thinking { text: last, .. }) = content.last_mut() {
        last.push_str(text);
        return;
    }
    content.push(ContentBlock::Thinking {
        text: text.to_string(),
        signature: None,
    });
}

#[async_trait::async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
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
        let url = format!("{}/api/chat", self.config.base_url.trim_end_matches('/'));
        let window = match self.config.num_ctx {
            Some(_) => None,
            None => {
                let auth = crate::registry::ProviderAuth {
                    api_key: self.config.api_key.clone(),
                    base_url: Some(self.config.base_url.clone()),
                    ..Default::default()
                };
                crate::models::cached_model_context("ollama", &auth, &request.model)
                    .await
                    .map(|context| context.input_tokens)
            }
        };
        let body = build_body(&self.config, &request, window)?;
        let mut req = self.http.post(&url).json(&body);
        if !self.config.api_key.is_empty() {
            req = req.bearer_auth(&self.config.api_key);
        }
        let send_fut = req.send();
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
        let mut decoder = NdjsonDecoder::default();
        let mut acc = Accumulator::new(&model);

        tokio::spawn(async move {
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
                                while let Some(line) = decoder.next_line() {
                                    match acc.convert(&line) {
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
                                        "stream closed before done:true".into(),
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
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn body_takes_the_model_window_and_caps_output_only_when_asked() {
        let mut req = ChatRequest::new("gemma3:e2b");
        req.messages = vec![Message::user_text("hi")];
        let body = build_body(&OllamaConfig::default(), &req, None).unwrap();
        assert_eq!(body["keep_alive"], "30m");
        assert!(body["options"].get("num_ctx").is_none());
        assert!(body["options"].get("num_predict").is_none());
        assert_eq!(body["stream"], true);
        req.max_tokens = Some(512);
        let body = build_body(&OllamaConfig::default(), &req, Some(131_072)).unwrap();
        assert_eq!(body["options"]["num_ctx"], 131_072);
        assert_eq!(body["options"]["num_predict"], 512);
    }

    #[test]
    fn body_includes_num_ctx_when_configured() {
        let mut req = ChatRequest::new("gemma3:e2b");
        req.messages = vec![Message::user_text("hi")];
        let config = OllamaConfig {
            num_ctx: Some(8192),
            keep_alive: "10m".into(),
            ..Default::default()
        };
        let body = build_body(&config, &req, Some(131_072)).unwrap();
        assert_eq!(body["options"]["num_ctx"], 8192, "the operator's pin wins");
        assert_eq!(body["keep_alive"], "10m");
    }

    #[test]
    fn tool_use_round_trips_with_object_arguments() {
        let mut req = ChatRequest::new("gemma3:e2b");
        req.messages = vec![
            Message::user_text("search"),
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "call_0".into(),
                name: "search".into(),
                input: serde_json::json!({"q": "vak"}),
            }]),
            Message {
                role: Role::User,
                content: vec![ContentBlock::tool_result("call_0", "result text")],
            },
        ];
        let body = build_body(&OllamaConfig::default(), &req, None).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        let call = &msgs[1]["tool_calls"][0];
        // Ollama's native wire wants a JSON object, not an escaped string.
        assert_eq!(
            call["function"]["arguments"],
            serde_json::json!({"q": "vak"})
        );
        assert_eq!(msgs[2]["role"], "tool");
        assert_eq!(msgs[2]["content"], "result text");
        // Ollama's renderers pair a result with its call by these; without
        // them gemma4 read every result as `response:unknown`.
        assert_eq!(msgs[2]["tool_call_id"], "call_0");
        assert_eq!(msgs[2]["tool_name"], "search");
    }

    #[test]
    fn thinking_is_never_sent_back_to_ollama() {
        let mut req = ChatRequest::new("gemma3:e2b");
        req.messages = vec![Message::assistant(vec![
            ContentBlock::Thinking {
                text: "reasoning".into(),
                signature: None,
            },
            ContentBlock::text("answer"),
        ])];
        let body = build_body(&OllamaConfig::default(), &req, None).unwrap();
        assert_eq!(body["messages"][0]["content"], "answer");
    }

    #[test]
    fn over_length_400_becomes_context_error() {
        let error = map_status_error(
            400,
            r#"{"error":"POST predict: this model's max context length (4096) exceeds the model's maximum context length"}"#,
        );
        assert!(matches!(error, LlmError::Context(_)));
    }

    #[test]
    fn unrelated_400_stays_invalid_request() {
        let error = map_status_error(400, r#"{"error":"model 'x' not found"}"#);
        assert!(matches!(error, LlmError::InvalidRequest(_)));
    }

    const FIXTURE_LINES: &[&str] = &[
        r#"{"model":"gemma3:e2b","message":{"role":"assistant","thinking":"let me "},"done":false}"#,
        r#"{"model":"gemma3:e2b","message":{"role":"assistant","thinking":"think"},"done":false}"#,
        r#"{"model":"gemma3:e2b","message":{"role":"assistant","content":"The "},"done":false}"#,
        r#"{"model":"gemma3:e2b","message":{"role":"assistant","content":"answer is 4."},"done":false}"#,
        r#"{"model":"gemma3:e2b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","total_duration":1000000,"load_duration":50000000,"prompt_eval_count":21,"prompt_eval_duration":120000000,"eval_count":9,"eval_duration":300000000}"#,
    ];

    #[test]
    fn stream_fixture_accumulates_text_thinking_and_usage() {
        let mut acc = Accumulator::new("gemma3:e2b");
        let mut end = None;
        for line in FIXTURE_LINES {
            if let Some(event) = acc.convert(line).unwrap()
                && let StreamEvent::End { message } = event
            {
                end = Some(message);
            }
        }
        let msg = end.expect("fixture must end with done:true");
        assert_eq!(msg.text_content(), "The answer is 4.");
        let thinking = msg
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::Thinking { text, .. } => Some(text.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(thinking, "let me think");
        assert_eq!(msg.usage.input_tokens, 21);
        assert_eq!(msg.usage.output_tokens, 9);
        assert_eq!(msg.usage.prefill_ms, Some(120));
        assert_eq!(msg.usage.load_ms, Some(50));
        assert_eq!(msg.stop_reason, StopReason::EndTurn);
    }

    const TOOL_CALL_LINES: &[&str] = &[
        r#"{"model":"gemma3:e2b","message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"search","arguments":{"q":"vak"}}}]},"done":false}"#,
        r#"{"model":"gemma3:e2b","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":10,"eval_count":5}"#,
    ];

    #[test]
    fn stream_fixture_generates_call_id_when_ollama_omits_one() {
        let mut acc = Accumulator::new("gemma3:e2b");
        let mut end = None;
        for line in TOOL_CALL_LINES {
            if let Some(event) = acc.convert(line).unwrap()
                && let StreamEvent::End { message } = event
            {
                end = Some(message);
            }
        }
        let msg = end.unwrap();
        assert_eq!(msg.stop_reason, StopReason::ToolUse);
        let ContentBlock::ToolUse { id, name, input } = &msg.content[0] else {
            panic!("expected a tool_use block");
        };
        assert_eq!(id, "call_0");
        assert_eq!(name, "search");
        assert_eq!(input, &serde_json::json!({"q": "vak"}));
    }

    #[test]
    fn ndjson_decoder_splits_on_newlines_across_pushes() {
        let mut decoder = NdjsonDecoder::default();
        decoder.push(b"{\"a\":1}\n{\"b\":");
        assert_eq!(decoder.next_line().as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(decoder.next_line(), None);
        decoder.push(b"2}\n");
        assert_eq!(decoder.next_line().as_deref(), Some(r#"{"b":2}"#));
    }
}
