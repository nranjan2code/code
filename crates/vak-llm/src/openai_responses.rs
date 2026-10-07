//! OpenAI Responses API adapter (`POST /v1/responses`) — the native wire
//! format for GPT-5.x-class models. Distinct from chat-completions: typed
//! content parts, `function_call` items, and event-name-keyed SSE deltas.

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

pub const OPENAI_RESPONSES_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Debug, Clone, Default)]
pub struct OpenAiResponsesConfig {
    pub api_key: String,
    pub base_url: String,
    /// When true, and the request carries `ChatRequest::cache`, send
    /// `prompt_cache_key` so the provider can route repeat traffic to the
    /// same cache-warm backend.
    pub cache_key: bool,
    /// When true, also send OpenRouter's `session_id` alongside
    /// `prompt_cache_key` (for the `openrouter-responses` route).
    pub openrouter: bool,
}

#[derive(Clone)]
pub struct OpenAiResponsesProvider {
    http: reqwest::Client,
    config: OpenAiResponsesConfig,
    gate: ProviderGate,
}

impl OpenAiResponsesProvider {
    pub fn new(config: OpenAiResponsesConfig) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| LlmError::Network(e.to_string()))?;
        Ok(OpenAiResponsesProvider {
            gate: ProviderGate::new(&config.base_url, &config.api_key),
            http,
            config,
        })
    }
}

/// The messages an incremental chained request must send: everything after
/// the last assistant message. `previous_response_id` already carries the
/// server's record of that assistant turn and everything before it, so
/// replaying it here would duplicate history the provider already has.
fn messages_since_last_assistant(messages: &[Message]) -> &[Message] {
    match messages.iter().rposition(|m| m.role == Role::Assistant) {
        Some(idx) => &messages[idx + 1..],
        None => messages,
    }
}

/// The Responses output items the provider's tool search writes
/// (docs/design/68-context-engine.md §5, §11). They are kept as opaque
/// `ContentBlock::Provider` blocks and sent back unchanged, because the
/// tools a search loaded are callable only while its items are in `input`.
const TOOL_SEARCH_ITEMS: [&str; 2] = ["tool_search_call", "tool_search_output"];

/// Whether an opaque block's `kind` is one of this wire's tool search items.
pub fn is_tool_search_item(kind: &str) -> bool {
    TOOL_SEARCH_ITEMS.contains(&kind)
}

pub fn build_body(
    config: &OpenAiResponsesConfig,
    request: &ChatRequest,
) -> Result<Value, LlmError> {
    let messages: &[Message] = match &request.previous_response_id {
        Some(_) => messages_since_last_assistant(&request.messages),
        None => &request.messages,
    };
    // Tool search is in use exactly when the request still carries a
    // deferred tool: `stream` drops them for a model that refused it.
    let tool_search = request.tools.iter().any(|t| t.defer);
    let mut input: Vec<Value> = Vec::with_capacity(messages.len());
    for m in messages {
        append_input_item(&mut input, m, tool_search)?;
    }

    // `previous_response_id` only resolves against a response the provider
    // actually retained, so a request that is (or may become) a chain link
    // must opt into `store`. A plain one-shot request with neither cache
    // hints nor a chain to continue keeps the old `store: false` default.
    let store = request.cache.is_some() || request.previous_response_id.is_some();
    let mut body = serde_json::json!({
        "model": request.model,
        "input": input,
        "stream": true,
        "store": store,
    });
    if let Some(system) = &request.system {
        body["instructions"] = Value::String(system.clone());
    }
    if let Some(previous) = &request.previous_response_id {
        body["previous_response_id"] = Value::String(previous.clone());
    }
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
                let mut tool = serde_json::json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                });
                if t.defer {
                    tool["defer_loading"] = serde_json::json!(true);
                }
                tool
            })
            .chain(tool_search.then(|| serde_json::json!({"type": "tool_search"})))
            .collect();
        body["tools"] = Value::Array(tools);
    }
    if let Some(effort) = request.effort {
        body["reasoning"] = serde_json::json!({ "effort": effort.three_level() });
    }
    Ok(body)
}

fn append_input_item(out: &mut Vec<Value>, m: &Message, tool_search: bool) -> Result<(), LlmError> {
    match m.role {
        Role::User => {
            let mut text = String::new();
            let mut outputs: Vec<(String, String)> = Vec::new();
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
                    } => outputs.push((tool_use_id.clone(), content.clone())),
                    ContentBlock::ToolUse { .. } => {
                        return Err(LlmError::InvalidRequest(
                            "tool_use blocks must appear in assistant messages".into(),
                        ));
                    }
                    ContentBlock::Thinking { .. } | ContentBlock::Provider { .. } => {}
                }
            }
            for (call_id, output) in outputs {
                out.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
            if !text.is_empty() || !images.is_empty() {
                let mut content: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    content.push(serde_json::json!({"type": "input_text", "text": text}));
                }
                for img in images {
                    content.push(serde_json::json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", img.media_type, img.data),
                    }));
                }
                out.push(serde_json::json!({
                    "role": "user",
                    "content": content,
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
                    // This wire's own tool search items go back in order,
                    // ahead of the call they loaded a tool for; another
                    // wire's opaque blocks, and these once tool search is
                    // off, are left out. They go without their `id`, as a
                    // function call does: an item sent by id is refused
                    // unless the reasoning item it followed comes with it,
                    // and reasoning is not replayed on this wire.
                    ContentBlock::Provider { kind, raw }
                        if tool_search && is_tool_search_item(kind) =>
                    {
                        let mut item = raw.clone();
                        if let Some(fields) = item.as_object_mut() {
                            fields.remove("id");
                        }
                        out.push(item);
                    }
                    ContentBlock::Thinking { .. }
                    | ContentBlock::ToolResult { .. }
                    | ContentBlock::Image { .. }
                    | ContentBlock::Provider { .. } => {}
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

fn map_status_error(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
    let parsed = serde_json::from_str::<Value>(body).ok();
    let code = parsed
        .as_ref()
        .and_then(|v| v.pointer("/error/code").or_else(|| v.get("code")))
        .and_then(Value::as_str);
    let message = parsed
        .as_ref()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str().map(String::from))
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    match status {
        401 | 403 => LlmError::Auth(message),
        400 => LlmError::classify_400(message),
        413 => LlmError::Context(message),
        404 | 422 => LlmError::InvalidRequest(message),
        429 => LlmError::from_rate_limit_code(code, message, retry_after),
        503 | 529 => match retry_after {
            Some(retry_after_secs) => LlmError::OverloadedWithRetryAfter {
                message,
                retry_after_secs,
            },
            None => LlmError::Overloaded(message),
        },
        _ => LlmError::Api { status, message },
    }
}

struct Accumulator {
    message: AssistantMessage,
    saw_completed: bool,
    /// call_id → position of the ToolUse block in `message.content`.
    tool_pos: std::collections::HashMap<String, usize>,
    raw_json: std::collections::HashMap<String, String>,
    last_event: Option<String>,
    retry_after_secs: Option<u64>,
}

impl Accumulator {
    fn new(model: &str) -> Self {
        Accumulator {
            message: AssistantMessage::empty(model),
            saw_completed: false,
            tool_pos: std::collections::HashMap::new(),
            raw_json: std::collections::HashMap::new(),
            last_event: None,
            retry_after_secs: None,
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
        self.last_event = Some(kind.clone());

        match kind.as_str() {
            "response.failed" | "error" => {
                let error = v
                    .pointer("/response/error")
                    .or_else(|| v.get("error"))
                    .unwrap_or(&v);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("provider returned a streamed failure");
                let code = error.get("code").and_then(Value::as_str).unwrap_or("");
                let message = format!("{code}: {message}");
                let retry_after_secs = error
                    .get("retry_after_secs")
                    .or_else(|| error.get("retry_after"))
                    .and_then(Value::as_u64)
                    .or(self.retry_after_secs);
                Err(match code {
                    "rate_limit_exceeded" | "insufficient_quota" => {
                        LlmError::from_rate_limit_code(Some(code), message, retry_after_secs)
                    }
                    "server_error" => LlmError::Overloaded(message),
                    "invalid_api_key" | "authentication_error" => LlmError::Auth(message),
                    "context_length_exceeded" | "invalid_request_error" => {
                        LlmError::classify_400(message)
                    }
                    _ => LlmError::Api {
                        status: 500,
                        message,
                    },
                })
            }
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
                    if let Some(item_id) = item.get("id").and_then(|i| i.as_str()) {
                        self.tool_pos.insert(item_id.to_string(), pos);
                    }
                    if !call_id.is_empty() {
                        self.tool_pos.insert(call_id.clone(), pos);
                    }
                    return Ok(Some(StreamEvent::ToolUseStart {
                        index: pos,
                        id: call_id,
                        name,
                        partial: self.message.clone(),
                    }));
                }
                Ok(None)
            }
            "response.output_item.done" => {
                let item = v.get("item").cloned().unwrap_or(Value::Null);
                if let Some(kind) = item.get("type").and_then(|t| t.as_str())
                    && is_tool_search_item(kind)
                {
                    self.message.content.push(ContentBlock::Provider {
                        kind: kind.to_string(),
                        raw: item,
                    });
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
                if let Some(id) = v.pointer("/response/id").and_then(|i| i.as_str()) {
                    self.message.response_id = Some(id.to_string());
                }
                if let Some(usage) = v.pointer("/response/usage") {
                    // Same normalization as the chat adapter: Responses'
                    // `input_tokens` already includes
                    // `input_tokens_details.cached_tokens`, so the cached
                    // share must be subtracted to get the non-cached
                    // remainder every adapter reports as `input_tokens`
                    // (docs/design/68-context-engine.md §1).
                    let raw_input = usage
                        .get("input_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                    let cached_tokens = usage
                        .pointer("/input_tokens_details/cached_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                    self.message.usage = Usage {
                        input_tokens: raw_input.saturating_sub(cached_tokens),
                        output_tokens: usage
                            .get("output_tokens")
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0),
                        cache_read_input_tokens: (cached_tokens > 0).then_some(cached_tokens),
                        cache_creation_input_tokens: None,
                        ..Default::default()
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

    fn defers_tools(&self, model: &str) -> bool {
        crate::models::tool_search_allowed(&self.config.base_url, model)
    }

    fn circuit_key(&self) -> String {
        crate::gate::route_identity(self.name(), &self.config.base_url, &self.config.api_key)
    }

    fn rate_limit_key(&self) -> String {
        crate::openai::account_capacity_key(&self.config.base_url, &self.config.api_key)
    }

    async fn refresh_capacity(&self, cancel: &CancellationToken) -> Result<(), LlmError> {
        if self.config.openrouter {
            crate::provider_status::refresh_openrouter_capacity(
                &self.config.base_url,
                &self.config.api_key,
                cancel,
            )
            .await?;
        }
        Ok(())
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let provider_permit = self.gate.acquire(&cancel).await?;
        let capacity_ticket = crate::RateLimitGate::capacity_observation_ticket(
            self.rate_limit_key(),
            &request.model,
        );
        let url = format!("{}/responses", self.config.base_url.trim_end_matches('/'));
        let mut request = request;
        if request.effort.is_some()
            && !crate::models::effort_allowed(&self.config.base_url, &request.model)
        {
            request.effort = None;
        }
        if !crate::models::tool_search_allowed(&self.config.base_url, &request.model) {
            request.tools.retain(|tool| !tool.defer);
        }
        // A model that refuses the reasoning setting, or tool search, is
        // asked once more without it and remembered (docs/design/01-llm.md).
        let response = loop {
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
            if status.is_success() {
                break response;
            }
            let observation = crate::openai::openai_capacity_observation(response.headers());
            let project_observation =
                crate::openai::openai_project_capacity_observation(response.headers());
            let retry_after = crate::openai::retry_after_from_headers(response.headers());
            let text = response.text().await.unwrap_or_default();
            if status.as_u16() == 400
                && request.effort.is_some()
                && crate::models::rejects_effort(&text)
            {
                crate::models::mark_effort_unsupported(&self.config.base_url, &request.model);
                request.effort = None;
                continue;
            }
            if status.as_u16() == 400
                && request.tools.iter().any(|tool| tool.defer)
                && crate::models::rejects_tool_search(&text)
            {
                crate::models::mark_tool_search_unsupported(&self.config.base_url, &request.model);
                request.tools.retain(|tool| !tool.defer);
                continue;
            }
            capacity_ticket.observe_model(observation);
            capacity_ticket.observe_account(project_observation);
            return Err(map_status_error(status.as_u16(), &text, retry_after));
        };

        capacity_ticket.observe_model(crate::openai::openai_capacity_observation(
            response.headers(),
        ));
        capacity_ticket.observe_account(crate::openai::openai_project_capacity_observation(
            response.headers(),
        ));

        let model = request.model.clone();
        let stream_retry_after = crate::openai::retry_after_from_headers(response.headers());
        let (mut sink, stream_rx) = channel(256);
        let mut byte_stream = response.bytes_stream();
        let mut decoder = SseDecoder::new();
        let mut acc = Accumulator::new(&model);
        acc.retry_after_secs = stream_retry_after;

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
                                    match acc.convert(&frame.data) {
                                        Ok(Some(event)) => {
                                            sink.push(event);
                                            // The terminal response is authoritative. Waiting
                                            // for the HTTP body to close can hang a finished
                                            // step or replace it with a transport failure.
                                            if acc.saw_completed {
                                                sink.close_message(acc.message.clone()).await;
                                                return;
                                            }
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
                                if acc.saw_completed {
                                    sink.close_message(acc.message.clone()).await;
                                } else {
                                    sink.close_error(LlmError::Parse(
                                        format!("stream closed before response.completed (last event: {})",
                                            acc.last_event.as_deref().unwrap_or("none")),
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

    #[test]
    fn responses_http_quota_code_is_terminal() {
        let error = map_status_error(
            429,
            r#"{"error":{"code":"insufficient_quota","message":"budget exhausted"}}"#,
            Some(10),
        );
        assert!(matches!(error, LlmError::QuotaExhausted(_)));
        assert!(!error.is_retryable());
    }
    use crate::types::CacheHints;

    fn config() -> OpenAiResponsesConfig {
        OpenAiResponsesConfig::default()
    }

    #[test]
    fn effort_is_sent_as_reasoning_effort_and_only_when_set() {
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![Message::user_text("hi")];
        assert!(
            build_body(&config(), &req)
                .unwrap()
                .get("reasoning")
                .is_none()
        );
        req.effort = Some(crate::types::Effort::Max);
        let body = build_body(&config(), &req).unwrap();
        assert_eq!(body["reasoning"]["effort"], "high");
        req.effort = Some(crate::types::Effort::Low);
        let body = build_body(&config(), &req).unwrap();
        assert_eq!(body["reasoning"]["effort"], "low");
    }

    #[test]
    fn cache_key_on_sends_prompt_cache_key() {
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![Message::user_text("hi")];
        req.cache = Some(CacheHints {
            session_key: "sess-1".into(),
            breakpoints: Vec::new(),
        });
        let cfg = OpenAiResponsesConfig {
            cache_key: true,
            ..config()
        };
        let body = build_body(&cfg, &req).unwrap();
        assert_eq!(body["prompt_cache_key"], "sess-1");
        assert!(body.get("session_id").is_none());
        // Cache hints imply this response might be chained from later.
        assert_eq!(body["store"], true);
    }

    #[test]
    fn openrouter_flag_adds_session_id() {
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![Message::user_text("hi")];
        req.cache = Some(CacheHints {
            session_key: "sess-1".into(),
            breakpoints: Vec::new(),
        });
        let cfg = OpenAiResponsesConfig {
            cache_key: true,
            openrouter: true,
            ..config()
        };
        let body = build_body(&cfg, &req).unwrap();
        assert_eq!(body["session_id"], "sess-1");
    }

    #[test]
    fn no_cache_hints_keeps_store_false() {
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![Message::user_text("hi")];
        let body = build_body(&config(), &req).unwrap();
        assert_eq!(body["store"], false);
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn previous_response_id_chains_only_messages_after_the_last_assistant_turn() {
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![
            Message::user_text("first"),
            Message::assistant(vec![ContentBlock::ToolUse {
                id: "t1".into(),
                name: "search".into(),
                input: serde_json::json!({}),
            }]),
            Message {
                role: Role::User,
                content: vec![ContentBlock::tool_result("t1", "result")],
            },
        ];
        req.previous_response_id = Some("resp_abc".into());
        let body = build_body(&config(), &req).unwrap();
        assert_eq!(body["previous_response_id"], "resp_abc");
        let input = body["input"].as_array().unwrap();
        // Only the tool-result message (after the last assistant turn) is
        // sent; the earlier user/assistant exchange is already server-side.
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "function_call_output");
    }

    #[test]
    fn no_previous_response_id_sends_full_history() {
        let mut req = ChatRequest::new("gpt-5.6");
        req.messages = vec![
            Message::user_text("first"),
            Message::assistant(vec![ContentBlock::text("answer")]),
        ];
        let body = build_body(&config(), &req).unwrap();
        assert_eq!(body["input"].as_array().unwrap().len(), 2);
        assert!(body.get("previous_response_id").is_none());
    }

    #[test]
    fn streamed_rate_limit_keeps_structured_retry_timing() {
        let mut acc = Accumulator::new("model");
        let error = acc
            .convert(
                r#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded","message":"limited","retry_after_secs":13}}}"#,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            LlmError::RateLimit {
                retry_after_secs: Some(13),
                ..
            }
        ));
    }

    #[test]
    fn streamed_rate_limit_uses_response_retry_header_when_error_has_none() {
        let mut acc = Accumulator::new("model");
        acc.retry_after_secs = Some(7);
        let error = acc
            .convert(
                r#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded","message":"limited"}}}"#,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            LlmError::RateLimit {
                retry_after_secs: Some(7),
                ..
            }
        ));
    }

    fn deferring_request() -> ChatRequest {
        let mut req = ChatRequest::new("gpt-test");
        req.tools = vec![
            ToolDefinition::new("core_tool", "always visible", serde_json::json!({})),
            ToolDefinition::new("rare_tool", "rarely needed", serde_json::json!({})).deferred(),
        ];
        req
    }

    #[test]
    fn a_deferred_tool_is_marked_and_the_search_entry_is_sent_with_it() {
        let mut req = deferring_request();
        req.messages = vec![Message::user_text("hi")];
        let body = build_body(&config(), &req).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        assert!(tools[0].get("defer_loading").is_none());
        assert_eq!(tools[1]["defer_loading"], true);
        assert_eq!(tools[2], serde_json::json!({"type": "tool_search"}));

        req.tools.retain(|tool| !tool.defer);
        let body = build_body(&config(), &req).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1, "no deferred tool, no search entry");
    }

    #[test]
    fn tool_search_items_are_kept_and_sent_back_in_order() {
        let mut acc = Accumulator::new("gpt-test");
        let call = r#"{"type":"tool_search_call","id":"tsc_1","execution":"server","call_id":null,"status":"completed","arguments":{"paths":["rare_tool"]}}"#;
        let output = r#"{"type":"tool_search_output","id":"tso_1","execution":"server","call_id":null,"status":"completed","tools":[{"type":"function","name":"rare_tool"}]}"#;
        for item in [call, output] {
            acc.convert(&format!(
                r#"{{"type":"response.output_item.done","item":{item}}}"#
            ))
            .unwrap();
        }
        acc.convert(
            r#"{"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"rare_tool"}}"#,
        )
        .unwrap();
        acc.convert(r#"{"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"rare_tool","arguments":"{}"}}"#)
            .unwrap();
        let kinds: Vec<&str> = acc
            .message
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Provider { kind, .. } => kind.as_str(),
                ContentBlock::ToolUse { .. } => "tool_use",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            ["tool_search_call", "tool_search_output", "tool_use"]
        );

        let mut req = deferring_request();
        req.messages = vec![
            Message::user_text("hi"),
            Message::assistant(acc.message.content.clone()),
        ];
        let body = build_body(&config(), &req).unwrap();
        let input = body["input"].as_array().unwrap();
        let sent: Vec<&str> = input
            .iter()
            .filter_map(|item| item.get("type").and_then(Value::as_str))
            .collect();
        assert_eq!(
            sent,
            ["tool_search_call", "tool_search_output", "function_call"]
        );
        let mut expected = serde_json::from_str::<Value>(call).unwrap();
        expected.as_object_mut().unwrap().remove("id");
        assert_eq!(input[1], expected, "sent as written, without its id");

        req.tools.retain(|tool| !tool.defer);
        let body = build_body(&config(), &req).unwrap();
        let sent: Vec<&str> = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item.get("type").and_then(Value::as_str))
            .collect();
        assert_eq!(
            sent,
            ["function_call"],
            "search items go only with tool search"
        );
    }

    #[test]
    fn another_wires_opaque_block_is_never_sent() {
        let mut req = deferring_request();
        req.messages = vec![
            Message::user_text("hi"),
            Message::assistant(vec![
                ContentBlock::Provider {
                    kind: "server_tool_use".into(),
                    raw: serde_json::json!({"type": "server_tool_use", "id": "x"}),
                },
                ContentBlock::text("done"),
            ]),
        ];
        let body = build_body(&config(), &req).unwrap();
        let input = body["input"].as_array().unwrap();
        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["role"], "assistant");
    }
}
