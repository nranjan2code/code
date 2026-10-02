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
use crate::turn::{current_turn_boundary, strip_thinking};
use crate::types::{
    AssistantMessage, ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition,
};

pub const GOOGLE_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub(crate) fn google_account_key(
    base_url: &str,
    api_key: &str,
    project_id: Option<&str>,
) -> String {
    let identity = project_id
        .filter(|id| !id.trim().is_empty())
        .map(|id| format!("project:{}", id.trim()))
        .unwrap_or_else(|| {
            format!(
                "credential:{}",
                crate::gate::credential_id(base_url, api_key)
            )
        });
    format!(
        "google-account:{}:{identity}",
        base_url.trim_end_matches('/')
    )
}

#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub api_key: String,
    pub project_id: Option<String>,
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
    let boundary = current_turn_boundary(&request.messages);
    let mut contents: Vec<Value> = Vec::with_capacity(request.messages.len());
    // function_call ids → names, resolved while walking history so
    // functionResponse parts can be keyed correctly.
    let mut id_to_name: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for (i, m) in request.messages.iter().enumerate() {
        let stripped;
        let m: &Message = if i < boundary {
            stripped = strip_thinking(m);
            &stripped
        } else {
            m
        };
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
                        // Only the Anthropic adapter understands
                        // server-side tool search; every other adapter
                        // skips this opaque block (docs/design/68 §5/§12).
                        ContentBlock::Provider { .. } => {}
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
                        | ContentBlock::Image { .. }
                        | ContentBlock::Provider { .. } => {}
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
                if k == "type"
                    && let Value::Array(types) = v
                {
                    let variants: Vec<_> = types
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|kind| serde_json::json!({"type": kind}))
                        .collect();
                    if !variants.is_empty() {
                        cleaned.insert("anyOf".into(), Value::Array(variants));
                    }
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
    let retry_after = retry_after.or_else(|| google_retry_delay(body));
    let parsed = serde_json::from_str::<Value>(body).ok();
    let quota_evidence = parsed
        .as_ref()
        .and_then(|v| v.pointer("/error/details"))
        .and_then(Value::as_array)
        .map(|details| {
            details
                .iter()
                .filter_map(|item| {
                    let kind = item.get("@type").and_then(Value::as_str)?;
                    if kind.ends_with("QuotaFailure") {
                        serde_json::to_string(item).ok()
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        });
    let message = parsed
        .as_ref()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str().map(String::from))
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    let message = quota_evidence
        .filter(|evidence| !evidence.is_empty())
        .map(|evidence| format!("{message} {evidence}"))
        .unwrap_or(message);
    match status {
        401 | 403 => LlmError::Auth(message),
        400 => LlmError::classify_400(message),
        404 | 413 | 422 => LlmError::InvalidRequest(message),
        429 => LlmError::RateLimit {
            message,
            retry_after_secs: retry_after,
        },
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

pub(crate) fn google_retry_delay(body: &str) -> Option<u64> {
    let value: Value = serde_json::from_str(body).ok()?;
    let details = value.pointer("/error/details")?.as_array()?;
    details
        .iter()
        .find(|detail| {
            detail
                .get("@type")
                .and_then(Value::as_str)
                .is_some_and(|name| name.ends_with("RetryInfo"))
        })?
        .get("retryDelay")?
        .as_str()
        .and_then(crate::openai::parse_provider_duration)
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
            // `promptTokenCount` is the whole prompt, cache hits included
            // (`cachedContentTokenCount` is a subset of it, not an
            // addition). Normalized `input_tokens` is only the non-cached
            // remainder, matching every other adapter
            // (docs/design/68-context-engine.md §1).
            if let Some(prompt_tokens) = usage.get("promptTokenCount").and_then(|x| x.as_u64()) {
                let cached_tokens = usage
                    .get("cachedContentTokenCount")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                self.message.usage.input_tokens = prompt_tokens.saturating_sub(cached_tokens);
                self.message.usage.cache_read_input_tokens =
                    (cached_tokens > 0).then_some(cached_tokens);
            }
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

    fn rate_limit_key(&self) -> String {
        google_account_key(
            &self.config.base_url,
            &self.config.api_key,
            self.config.project_id.as_deref(),
        )
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
            let headers = response.headers().clone();
            let retry_after = crate::openai::retry_after_from_headers(response.headers());
            let text = response.text().await.unwrap_or_default();
            let observation = google_capacity_observation(&headers, Some(&text));
            capacity_ticket.observe_model(observation);
            return Err(map_status_error(status.as_u16(), &text, retry_after));
        }

        capacity_ticket.observe_model(google_capacity_observation(response.headers(), None));

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

pub(crate) fn google_capacity_observation(
    headers: &reqwest::header::HeaderMap,
    body: Option<&str>,
) -> crate::CapacityObservation {
    let number = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
    };
    let mut observation = crate::CapacityObservation {
        tokens_remaining: number("x-ratelimit-remaining-tokens"),
        tokens_limit: number("x-ratelimit-limit-tokens"),
        requests_remaining: number("x-ratelimit-remaining-requests"),
        requests_limit: number("x-ratelimit-limit-requests"),
        reset_after_secs: headers
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(crate::openai::parse_provider_duration),
        daily_remaining: None,
        daily_limit: None,
        daily_requests_remaining: None,
        daily_requests_limit: None,
        daily_reset_after_secs: None,
        ..Default::default()
    };
    if let Some(body) = body {
        observe_google_quota_failure(&mut observation, body);
    }
    observation
}

fn observe_google_quota_failure(observation: &mut crate::CapacityObservation, body: &str) {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return;
    };
    let Some(details) = value.pointer("/error/details").and_then(Value::as_array) else {
        return;
    };
    let retry_after = google_retry_delay(body);
    let next_daily_reset = || {
        let now = chrono::Utc::now();
        let pacific = now.with_timezone(&chrono_tz::America::Los_Angeles);
        let next_day = pacific.date_naive() + chrono::Days::new(1);
        next_day
            .and_hms_opt(0, 0, 0)?
            .and_local_timezone(chrono_tz::America::Los_Angeles)
            .single()
            .map(|midnight| {
                (midnight.with_timezone(&chrono::Utc) - now)
                    .num_seconds()
                    .max(0) as u64
            })
    };
    for detail in details {
        if !detail
            .get("@type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.ends_with("QuotaFailure"))
        {
            continue;
        }
        let Some(violations) = detail.get("violations").and_then(Value::as_array) else {
            continue;
        };
        for violation in violations {
            let quota_id = violation
                .get("quotaId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let metric = violation
                .get("quotaMetric")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_daily = quota_id.contains("perday")
                || quota_id.contains("per_day")
                || quota_id.contains("daily");
            let is_token = metric.contains("token");
            let is_input_token =
                is_token && (metric.contains("input_token") || metric.contains("prompt_token"));
            let is_output_token = is_token && metric.contains("output_token");
            let limit = violation
                .get("quotaValue")
                .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
            if is_daily {
                if is_input_token {
                    observation.daily_input_remaining = Some(0);
                    observation.daily_input_limit = limit.or(observation.daily_input_limit);
                    observation.daily_input_reset_after_secs = next_daily_reset();
                } else if is_output_token {
                    observation.daily_output_remaining = Some(0);
                    observation.daily_output_limit = limit.or(observation.daily_output_limit);
                    observation.daily_output_reset_after_secs = next_daily_reset();
                } else if is_token {
                    // TPD is model-specific. If Google's metric does not
                    // identify input or output, enforce it against combined
                    // estimated demand instead of assuming an input-only cap.
                    observation.daily_remaining = Some(0);
                    observation.daily_limit = limit.or(observation.daily_limit);
                    observation.daily_reset_after_secs = next_daily_reset();
                } else {
                    observation.daily_requests_remaining = Some(0);
                    observation.daily_requests_limit = limit.or(observation.daily_requests_limit);
                    observation.daily_requests_reset_after_secs = next_daily_reset();
                }
            } else {
                observation.reset_after_secs = retry_after.or(observation.reset_after_secs);
                if is_token {
                    if is_output_token {
                        observation.output_tokens_remaining = Some(0);
                        observation.output_tokens_limit = limit.or(observation.output_tokens_limit);
                        observation.output_tokens_reset_after_secs =
                            retry_after.or(observation.output_tokens_reset_after_secs);
                    } else {
                        // Gemini documents its standard TPM dimension as
                        // input tokens; unknown token metrics use that
                        // conservative documented dimension.
                        observation.input_tokens_remaining = Some(0);
                        observation.input_tokens_limit = limit.or(observation.input_tokens_limit);
                        observation.input_tokens_reset_after_secs =
                            retry_after.or(observation.input_tokens_reset_after_secs);
                    }
                } else {
                    observation.requests_reset_after_secs =
                        retry_after.or(observation.requests_reset_after_secs);
                    observation.requests_remaining = Some(0);
                    observation.requests_limit = limit.or(observation.requests_limit);
                }
            }
        }
    }
}

#[cfg(test)]
mod build_body_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::types::Role;

    #[test]
    fn google_quota_failure_records_daily_token_exhaustion_without_inventing_usage() {
        let body = serde_json::json!({
            "error": {"details": [{
                "@type": "type.googleapis.com/google.rpc.QuotaFailure",
                "violations": [{
                    "quotaMetric": "generativelanguage.googleapis.com/generate_content_daily_input_token_count",
                    "quotaId": "GenerateContentInputTokensPerDayPerProjectPerModel",
                    "quotaValue": "120000"
                }]
            }]}
        }).to_string();
        let observed = google_capacity_observation(&reqwest::header::HeaderMap::new(), Some(&body));
        assert_eq!(observed.daily_input_remaining, Some(0));
        assert_eq!(observed.daily_input_limit, Some(120_000));
        assert!(observed.daily_input_reset_after_secs.unwrap() > 0);
        assert_eq!(observed.daily_remaining, None);
        assert_eq!(observed.tokens_remaining, None);
    }

    #[test]
    fn google_empty_success_headers_do_not_claim_a_known_quota() {
        let observed = google_capacity_observation(&reqwest::header::HeaderMap::new(), None);
        assert_eq!(observed.tokens_remaining, None);
        assert_eq!(observed.input_tokens_remaining, None);
        assert_eq!(observed.requests_remaining, None);
        assert_eq!(observed.daily_remaining, None);
        assert_eq!(observed.daily_requests_remaining, None);
    }

    #[test]
    fn provider_tool_schema_represents_json_type_unions_with_any_of() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {"value": {"type": ["string", "number"]}}
        });
        let output = sanitize_schema(&input);
        assert_eq!(
            output["properties"]["value"]["anyOf"],
            serde_json::json!([{"type": "string"}, {"type": "number"}])
        );
        assert!(output["properties"]["value"].get("type").is_none());
    }

    #[test]
    fn thought_signature_round_trips_into_the_request_body() {
        let mut req = ChatRequest::new("gemini-3-pro");
        req.messages = vec![
            Message::user_text("do the thing"),
            Message::assistant(vec![
                ContentBlock::Thinking {
                    text: String::new(),
                    signature: Some("thought-sig-abc".into()),
                },
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "search".into(),
                    input: serde_json::json!({"q": "x"}),
                },
            ]),
        ];
        let body = build_body(&req).unwrap();
        let parts = body["contents"][1]["parts"].as_array().unwrap();
        let call_part = parts
            .iter()
            .find(|p| p.get("functionCall").is_some())
            .unwrap();
        assert_eq!(call_part["thoughtSignature"], "thought-sig-abc");
    }

    #[test]
    fn thinking_before_the_current_turn_boundary_is_stripped() {
        let mut req = ChatRequest::new("gemini-3-pro");
        req.messages = vec![
            Message::user_text("first"),
            Message::assistant(vec![
                ContentBlock::Thinking {
                    text: String::new(),
                    signature: Some("old-sig".into()),
                },
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "search".into(),
                    input: serde_json::json!({}),
                },
            ]),
            Message::user_text("second, a fresh directive"),
        ];
        let body = build_body(&req).unwrap();
        let parts = body["contents"][1]["parts"].as_array().unwrap();
        assert!(
            parts.iter().all(|p| p.get("thoughtSignature").is_none()),
            "thinking from a closed turn must not carry a thought signature"
        );
    }

    #[test]
    fn tool_result_only_message_does_not_start_a_new_turn() {
        let mut req = ChatRequest::new("gemini-3-pro");
        req.messages = vec![
            Message::user_text("do the thing"),
            Message::assistant(vec![
                ContentBlock::Thinking {
                    text: String::new(),
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
        let parts = body["contents"][1]["parts"].as_array().unwrap();
        let call_part = parts
            .iter()
            .find(|p| p.get("functionCall").is_some())
            .unwrap();
        assert_eq!(call_part["thoughtSignature"], "sig");
    }
}
