use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::error::LlmError;
use crate::gate::ProviderGate;
use crate::sse::SseDecoder;
use crate::stream::{EventSink, EventStream, StreamEvent, channel};
use crate::turn::{current_turn_boundary, strip_thinking};
use crate::types::{
    AssistantMessage, ChatRequest, ContentBlock, Effort, Message, Role, StopReason, Usage,
};

/// Anthropic accepts at most 4 `cache_control` breakpoints per request. The
/// stable system prompt always claims one when present, leaving the rest
/// for message-level breakpoints named by `ChatRequest::cache`.
const MAX_CACHE_BREAKPOINTS: usize = 4;

/// Anthropic's server-side tool search tool (docs/design/68 §5/§11):
/// prepended to `tools` whenever any tool in the request is deferred, so the
/// model can discover a deferred schema without it ever entering the prefix.
const TOOL_SEARCH_TOOL: &str = "tool_search_tool_regex_20251119";

/// Beta flag for the opt-in fast-mode research preview (docs/design/68 §11
/// "Anthropic" row / the Anthropic API "Fast Mode" quick reference): sent
/// only alongside `"speed": "fast"`, and only for a model the capability
/// cache has confirmed supports it.
const FAST_MODE_BETA: &str = "fast-mode-2026-02-01";

pub const ANTHROPIC_VERSION: &str = "2023-06-01";
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

fn learned_organization_keys() -> &'static Mutex<HashMap<String, (String, std::time::Instant)>> {
    static KEYS: OnceLock<Mutex<HashMap<String, (String, std::time::Instant)>>> = OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn anthropic_credential_identity(base_url: &str, api_key: &str) -> String {
    crate::gate::credential_id(base_url.trim_end_matches('/'), api_key)
}

fn anthropic_account_capacity_key(base_url: &str, api_key: &str, fast_mode: bool) -> String {
    let credential = anthropic_credential_identity(base_url, api_key);
    let mut keys = learned_organization_keys()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = std::time::Instant::now();
    if let Some((organization, seen)) = keys.get_mut(&credential) {
        *seen = now;
        return anthropic_capacity_mode_key(organization, fast_mode);
    }
    anthropic_capacity_mode_key(
        &format!("anthropic-account:credential:{credential}"),
        fast_mode,
    )
}

fn anthropic_capacity_mode_key(identity: &str, fast_mode: bool) -> String {
    if fast_mode {
        format!("{identity}:fast-mode")
    } else {
        identity.to_string()
    }
}

fn remember_anthropic_organization(
    base_url: &str,
    api_key: &str,
    fast_mode: bool,
    headers: &reqwest::header::HeaderMap,
) -> Option<String> {
    let organization = headers
        .get("anthropic-organization-id")
        .and_then(|value| value.to_str().ok())?
        .trim();
    if organization.is_empty() {
        return None;
    }
    // Organization IDs are private account metadata. Keep only a one-way
    // fingerprint in the process-wide identity map and rate-limit registry.
    let key = format!(
        "anthropic-account:organization:{}",
        crate::gate::credential_id(base_url.trim_end_matches('/'), organization)
    );
    let mut keys = learned_organization_keys()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let now = std::time::Instant::now();
    let credential = anthropic_credential_identity(base_url, api_key);
    if keys.len() >= 1024
        && !keys.contains_key(&credential)
        && let Some(oldest) = keys
            .iter()
            .min_by_key(|(_, (_, seen))| *seen)
            .map(|(credential, _)| credential.clone())
    {
        keys.remove(&oldest);
    }
    keys.insert(credential, (key.clone(), now));
    Some(anthropic_capacity_mode_key(&key, fast_mode))
}

#[derive(Debug, Clone)]
pub struct AnthropicConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// Opt-in fast-mode research preview (config key under
    /// `[providers.anthropic]`, default off): premium pricing, its own
    /// rate-limit bucket, and restricted to specific models — the adapter
    /// only ever sends `speed: "fast"` when this is true AND the model's
    /// discovered capabilities confirm support
    /// (`models::anthropic_fast_mode_allowed`).
    pub fast_mode: bool,
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

/// `effort_allowed` and `fast_mode` are resolved by the caller (`stream`)
/// from the in-process capability cache (`models.rs`) before this is
/// called, so building the body stays a pure function of its inputs.
pub fn build_body(
    request: &ChatRequest,
    effort_allowed: bool,
    fast_mode: bool,
) -> Result<Value, LlmError> {
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
        unwrap_provider_blocks(&mut value);
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
        let mut tools: Vec<Value> = Vec::with_capacity(request.tools.len() + 1);
        if request.tools.iter().any(|t| t.defer) {
            tools.push(serde_json::json!({
                "type": TOOL_SEARCH_TOOL,
                "name": "tool_search_tool_regex",
            }));
        }
        for t in &request.tools {
            let mut tool = serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.parameters,
            });
            // A deferred tool never carries `cache_control`: its schema is
            // never in the stable prefix, so there is nothing to mark
            // cacheable (docs/design/68 §5/§11).
            if t.defer {
                tool["defer_loading"] = serde_json::json!(true);
            }
            tools.push(tool);
        }
        body["tools"] = Value::Array(tools);
    }
    // Reasoning depth, never thinking on/off (docs/design/68 §11): current
    // models either reject an explicit `{"type":"disabled"}`/`budget_tokens`
    // outright or silently degrade tool-call reliability under it, so this
    // adapter never sends either — `thinking` is simply omitted, which runs
    // adaptive on every current model. `effort` is the one supported dial,
    // and `think == Some(false)` (a side-dispatch that wants a fast, cheap
    // answer) maps onto its lowest level only when the caller has not
    // already asked for a specific one.
    if fast_mode {
        body["speed"] = serde_json::json!("fast");
    }
    if effort_allowed {
        let effort = request
            .effort
            .or_else(|| (request.think == Some(false)).then_some(Effort::Low));
        if let Some(effort) = effort {
            body["output_config"] = serde_json::json!({ "effort": effort.as_str() });
        }
    }
    Ok(body)
}

/// Replaces a serialized `ContentBlock::Provider` (`{"type":"provider",
/// "kind":"server_tool_use", "raw": {...}}`) with its `raw` value, which
/// already carries the provider's own `"type"` and every original field —
/// so a `server_tool_use` / `tool_search_tool_result` block the API sent
/// round-trips back to it byte-for-byte (docs/design/68 §5/§12). Other
/// content blocks are left untouched.
fn unwrap_provider_blocks(message: &mut Value) {
    let Some(content) = message.get_mut("content").and_then(|c| c.as_array_mut()) else {
        return;
    };
    for block in content.iter_mut() {
        if block.get("type").and_then(|t| t.as_str()) == Some("provider")
            && let Some(raw) = block.get("raw").cloned()
        {
            *block = raw;
        }
    }
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

/// Provider phrasing that identifies a 400 as caused specifically by the
/// `output_config.effort` parameter, distinct from an unrelated validation
/// failure — same "key off the provider's own wording" approach as
/// `LlmError::classify_400`'s over-length markers.
fn rejects_effort(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("effort") || normalized.contains("output_config")
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
        413 => LlmError::Context(message),
        404 | 422 => LlmError::InvalidRequest(message),
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
                    "text" => {
                        self.message.content.push(ContentBlock::text(String::new()));
                        self.index_map.insert(idx, our_idx);
                        Ok(None)
                    }
                    // A block type this build does not otherwise interpret
                    // (`server_tool_use`, `tool_search_tool_result`, and any
                    // future addition) is kept opaque rather than silently
                    // folded into an empty text block, so it round-trips
                    // unchanged (docs/design/68 §5/§12).
                    other => {
                        self.message.content.push(ContentBlock::Provider {
                            kind: other.to_string(),
                            raw: block.clone(),
                        });
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
                        match &mut self.message.content[our_idx] {
                            ContentBlock::ToolUse { input, .. } => {
                                *input = parsed;
                            }
                            // `server_tool_use` streams its input the same
                            // way `tool_use` does; keep the opaque block's
                            // raw JSON in sync so it round-trips complete.
                            ContentBlock::Provider { raw: block_raw, .. } => {
                                if let Some(obj) = block_raw.as_object_mut() {
                                    obj.insert("input".to_string(), parsed);
                                }
                                return Ok(None);
                            }
                            _ => return Ok(None),
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
                if let Some(&our_idx) = self.index_map.get(&idx) {
                    let raw = self.raw_json.get(&idx).cloned();
                    match &mut self.message.content[our_idx] {
                        ContentBlock::ToolUse { input, .. } => {
                            let raw = raw.unwrap_or_default();
                            *input = if raw.trim().is_empty() {
                                Value::Object(Default::default())
                            } else {
                                serde_json::from_str(&raw).map_err(|e| {
                                    LlmError::Parse(format!(
                                        "tool input json invalid at block stop: {e}"
                                    ))
                                })?
                            };
                        }
                        ContentBlock::Provider { raw: block_raw, .. } => {
                            if let Some(raw) = raw
                                && !raw.trim().is_empty()
                                && let Ok(parsed) = serde_json::from_str::<Value>(&raw)
                                && let Some(obj) = block_raw.as_object_mut()
                            {
                                obj.insert("input".to_string(), parsed);
                            }
                        }
                        _ => {}
                    }
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

    fn rate_limit_key(&self) -> String {
        anthropic_account_capacity_key(&self.config.base_url, &self.config.api_key, false)
    }

    fn rate_limit_key_for_model(&self, model: &str) -> String {
        let fast_mode = self.config.fast_mode && crate::models::anthropic_fast_mode_allowed(model);
        anthropic_account_capacity_key(&self.config.base_url, &self.config.api_key, fast_mode)
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let provider_permit = self.gate.acquire(&cancel).await?;
        // Fast mode is an opt-in, model-restricted research preview
        // (docs/design/68 §11): discover support once per model id in the
        // background — never blocking this request — so a later request
        // for the same model has an answer cached. This request itself
        // conservatively skips fast mode until that lookup lands.
        if self.config.fast_mode && !crate::models::anthropic_fast_mode_known(&request.model) {
            let auth = crate::registry::ProviderAuth {
                api_key: self.config.api_key.clone(),
                base_url: Some(self.config.base_url.clone()),
                ..Default::default()
            };
            let model = request.model.clone();
            tokio::spawn(async move {
                if let Ok(caps) = crate::models::anthropic_model_capabilities(&auth, &model).await {
                    crate::models::record_anthropic_capabilities(&model, caps);
                }
            });
        }

        let mut effort_allowed = crate::models::anthropic_effort_allowed(&request.model);
        let mut fast_mode =
            self.config.fast_mode && crate::models::anthropic_fast_mode_allowed(&request.model);
        let mut capacity_identity =
            anthropic_account_capacity_key(&self.config.base_url, &self.config.api_key, fast_mode);
        let mut capacity_ticket = crate::RateLimitGate::capacity_observation_ticket(
            capacity_identity.clone(),
            &request.model,
        );
        let mut response = self
            .send_once(&request, effort_allowed, fast_mode, &cancel)
            .await?;

        // One narrow, same-turn retry per failure class (docs/design/68
        // §11): an unsupported `output_config.effort` 400s naming the
        // parameter, and fast mode has its own rate-limit bucket that can
        // 429 while standard speed would not. Each retry strips exactly the
        // feature that failed and resends once; a second failure is final.
        if !response.status().is_success() {
            let status = response.status().as_u16();
            if status == 400 && effort_allowed {
                let headers = response.headers().clone();
                let observation = anthropic_capacity_observation(&headers);
                let retry_after = retry_after_header(&response);
                let text = response.text().await.unwrap_or_default();
                if rejects_effort(&text) {
                    crate::models::mark_anthropic_effort_unsupported(&request.model);
                    effort_allowed = false;
                    response = self
                        .send_once(&request, effort_allowed, fast_mode, &cancel)
                        .await?;
                } else {
                    observe_anthropic_capacity_response(
                        &capacity_ticket,
                        &capacity_identity,
                        &self.config.base_url,
                        &self.config.api_key,
                        &request.model,
                        status,
                        retry_after,
                        &headers,
                        observation,
                        fast_mode,
                    );
                    return Err(map_status_error(status, &text, retry_after));
                }
            } else if status == 429 && fast_mode {
                let headers = response.headers().clone();
                let retry_after = retry_after_header(&response);
                observe_anthropic_capacity_response(
                    &capacity_ticket,
                    &capacity_identity,
                    &self.config.base_url,
                    &self.config.api_key,
                    &request.model,
                    status,
                    retry_after,
                    &headers,
                    anthropic_capacity_observation(&headers),
                    true,
                );
                fast_mode = false;
                capacity_identity = anthropic_account_capacity_key(
                    &self.config.base_url,
                    &self.config.api_key,
                    false,
                );
                capacity_ticket = crate::RateLimitGate::capacity_observation_ticket(
                    capacity_identity.clone(),
                    &request.model,
                );
                response = self
                    .send_once(&request, effort_allowed, fast_mode, &cancel)
                    .await?;
            }
        }

        let status = response.status();
        if !status.is_success() {
            let headers = response.headers().clone();
            let observation = anthropic_capacity_observation(&headers);
            let retry_after = retry_after_header(&response);
            let text = response.text().await.unwrap_or_default();
            observe_anthropic_capacity_response(
                &capacity_ticket,
                &capacity_identity,
                &self.config.base_url,
                &self.config.api_key,
                &request.model,
                status.as_u16(),
                retry_after,
                &headers,
                observation,
                fast_mode,
            );
            return Err(map_status_error(status.as_u16(), &text, retry_after));
        }

        observe_anthropic_capacity_response(
            &capacity_ticket,
            &capacity_identity,
            &self.config.base_url,
            &self.config.api_key,
            &request.model,
            status.as_u16(),
            None,
            response.headers(),
            anthropic_capacity_observation(response.headers()),
            fast_mode,
        );

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

impl AnthropicProvider {
    async fn send_once(
        &self,
        request: &ChatRequest,
        effort_allowed: bool,
        fast_mode: bool,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, LlmError> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let body = build_body(request, effort_allowed, fast_mode)?;
        let mut req = self
            .http
            .post(&url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION);
        if fast_mode {
            req = req.header("anthropic-beta", FAST_MODE_BETA);
        }
        let send_fut = req.json(&body).send();
        tokio::select! {
            _ = cancel.cancelled() => Err(LlmError::Aborted { partial: None }),
            r = send_fut => r.map_err(|e| LlmError::Network(e.to_string())),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn observe_anthropic_capacity_response(
    original_ticket: &crate::CapacityObservationTicket,
    original_identity: &str,
    base_url: &str,
    api_key: &str,
    model: &str,
    status: u16,
    retry_after: Option<u64>,
    headers: &reqwest::header::HeaderMap,
    observation: crate::CapacityObservation,
    fast_mode: bool,
) {
    original_ticket.observe_model(observation.clone());
    let Some(organization_identity) =
        remember_anthropic_organization(base_url, api_key, fast_mode, headers)
    else {
        return;
    };
    if organization_identity != original_identity {
        crate::RateLimitGate::capacity_observation_ticket(&organization_identity, model)
            .observe_model(observation);
    }
    if matches!(status, 429 | 529) {
        crate::RateLimitGate::for_key(organization_identity).observe_limit(
            retry_after.map(std::time::Duration::from_secs),
            std::time::Duration::from_secs(1),
        );
    }
}

fn retry_after_header(response: &reqwest::Response) -> Option<u64> {
    crate::openai::retry_after_from_headers(response.headers())
}

fn anthropic_capacity_observation(
    headers: &reqwest::header::HeaderMap,
) -> crate::CapacityObservation {
    let number = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
    };
    let input_remaining = number("anthropic-ratelimit-input-tokens-remaining");
    let input_limit = number("anthropic-ratelimit-input-tokens-limit");
    let output_remaining = number("anthropic-ratelimit-output-tokens-remaining");
    let output_limit = number("anthropic-ratelimit-output-tokens-limit");
    let has_component_limits = input_remaining.is_some()
        || input_limit.is_some()
        || output_remaining.is_some()
        || output_limit.is_some();
    crate::CapacityObservation {
        tokens_remaining: (!has_component_limits)
            .then(|| number("anthropic-ratelimit-tokens-remaining"))
            .flatten(),
        tokens_limit: (!has_component_limits)
            .then(|| number("anthropic-ratelimit-tokens-limit"))
            .flatten(),
        input_tokens_remaining: input_remaining,
        input_tokens_limit: input_limit,
        output_tokens_remaining: output_remaining,
        output_tokens_limit: output_limit,
        input_tokens_reset_after_secs: headers
            .get("anthropic-ratelimit-input-tokens-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(crate::openai::parse_provider_reset),
        output_tokens_reset_after_secs: headers
            .get("anthropic-ratelimit-output-tokens-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(crate::openai::parse_provider_reset),
        requests_remaining: number("anthropic-ratelimit-requests-remaining"),
        requests_limit: number("anthropic-ratelimit-requests-limit"),
        reset_after_secs: (!has_component_limits)
            .then(|| {
                headers
                    .get("anthropic-ratelimit-tokens-reset")
                    .and_then(|v| v.to_str().ok())
                    .and_then(crate::openai::parse_provider_reset)
            })
            .flatten(),
        requests_reset_after_secs: headers
            .get("anthropic-ratelimit-requests-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(crate::openai::parse_provider_reset),
        daily_remaining: None,
        daily_limit: None,
        daily_requests_remaining: None,
        daily_requests_limit: None,
        daily_reset_after_secs: None,
        ..Default::default()
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
    use crate::types::{CacheBreakpoint, CacheHints, ToolDefinition};

    #[test]
    fn anthropic_capacity_parser_separates_input_output_and_request_windows() {
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in [
            ("anthropic-ratelimit-input-tokens-remaining", "800"),
            ("anthropic-ratelimit-input-tokens-limit", "1000"),
            ("anthropic-ratelimit-output-tokens-remaining", "400"),
            ("anthropic-ratelimit-output-tokens-limit", "500"),
            ("anthropic-ratelimit-requests-remaining", "9"),
            ("anthropic-ratelimit-requests-limit", "10"),
        ] {
            headers.insert(name, value.parse().unwrap());
        }
        let observed = anthropic_capacity_observation(&headers);
        assert_eq!(observed.input_tokens_remaining, Some(800));
        assert_eq!(observed.input_tokens_limit, Some(1000));
        assert_eq!(observed.output_tokens_remaining, Some(400));
        assert_eq!(observed.output_tokens_limit, Some(500));
        assert_eq!(observed.requests_remaining, Some(9));
        assert_eq!(observed.tokens_remaining, None);
        assert_eq!(observed.daily_remaining, None);
    }

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
        let body = build_body(&req, true, false).unwrap();
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
        let body = build_body(&req, true, false).unwrap();
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
        let body = build_body(&req, true, false).unwrap();
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
        let body = build_body(&req, true, false).unwrap();
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
        let body = build_body(&req, true, false).unwrap();
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

    #[test]
    fn deferred_tools_render_defer_loading_and_prepend_the_search_tool() {
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.messages = vec![Message::user_text("hi")];
        req.tools = vec![
            ToolDefinition::new("core_tool", "always visible", serde_json::json!({})),
            ToolDefinition::new("rare_tool", "rarely needed", serde_json::json!({})).deferred(),
        ];
        let body = build_body(&req, true, false).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "tool_search_tool_regex_20251119");
        let core = tools.iter().find(|t| t["name"] == "core_tool").unwrap();
        assert!(core.get("defer_loading").is_none());
        assert!(core.get("cache_control").is_none());
        let rare = tools.iter().find(|t| t["name"] == "rare_tool").unwrap();
        assert_eq!(rare["defer_loading"], true);
        assert!(
            rare.get("cache_control").is_none(),
            "a deferred tool must never carry cache_control"
        );
    }

    #[test]
    fn no_deferred_tools_means_no_search_tool_is_sent() {
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.messages = vec![Message::user_text("hi")];
        req.tools = vec![ToolDefinition::new(
            "core_tool",
            "always visible",
            serde_json::json!({}),
        )];
        let body = build_body(&req, true, false).unwrap();
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "core_tool");
    }

    #[test]
    fn a_provider_block_replays_verbatim_on_the_wire() {
        let raw = serde_json::json!({
            "type": "server_tool_use",
            "id": "srvtoolu_1",
            "name": "tool_search_tool_regex",
            "input": {"pattern": "weather"}
        });
        let mut req = ChatRequest::new("claude-sonnet-4-5");
        req.messages = vec![Message::assistant(vec![ContentBlock::Provider {
            kind: "server_tool_use".into(),
            raw: raw.clone(),
        }])];
        let body = build_body(&req, true, false).unwrap();
        let sent = &body["messages"][0]["content"][0];
        assert_eq!(sent, &raw, "a provider block must round-trip unchanged");
        assert_ne!(sent["type"], "provider");
    }

    #[test]
    fn a_provider_block_streams_and_finalizes_its_input() {
        let mut acc = Accumulator::new("claude-sonnet-4-5");
        acc.convert(
            &serde_json::json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {
                    "type": "server_tool_use",
                    "id": "srvtoolu_1",
                    "name": "tool_search_tool_regex",
                }
            })
            .to_string(),
        )
        .unwrap();
        acc.convert(
            &serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": "{\"pattern\""}
            })
            .to_string(),
        )
        .unwrap();
        acc.convert(
            &serde_json::json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": ":\"weather\"}"}
            })
            .to_string(),
        )
        .unwrap();
        acc.convert(&serde_json::json!({"type": "content_block_stop", "index": 0}).to_string())
            .unwrap();
        let ContentBlock::Provider { kind, raw } = &acc.message.content[0] else {
            unreachable!("expected a Provider block");
        };
        assert_eq!(kind, "server_tool_use");
        assert_eq!(raw["input"]["pattern"], "weather");
        assert_eq!(raw["type"], "server_tool_use");
    }

    #[test]
    fn explicit_effort_is_rendered_under_output_config() {
        let mut req = ChatRequest::new("claude-opus-5");
        req.messages = vec![Message::user_text("hi")];
        req.effort = Some(Effort::XHigh);
        let body = build_body(&req, true, false).unwrap();
        assert_eq!(body["output_config"]["effort"], "xhigh");
    }

    #[test]
    fn think_false_falls_back_to_low_effort_when_effort_is_unset() {
        let mut req = ChatRequest::new("claude-haiku-4-5");
        req.messages = vec![Message::user_text("classify this")];
        req.think = Some(false);
        let body = build_body(&req, true, false).unwrap();
        assert_eq!(body["output_config"]["effort"], "low");
    }

    #[test]
    fn explicit_effort_wins_over_the_think_false_fallback() {
        let mut req = ChatRequest::new("claude-opus-5");
        req.messages = vec![Message::user_text("hi")];
        req.think = Some(false);
        req.effort = Some(Effort::Max);
        let body = build_body(&req, true, false).unwrap();
        assert_eq!(body["output_config"]["effort"], "max");
    }

    #[test]
    fn no_output_config_when_neither_effort_nor_think_false_is_set() {
        let mut req = ChatRequest::new("claude-sonnet-5");
        req.messages = vec![Message::user_text("hi")];
        let body = build_body(&req, true, false).unwrap();
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn effort_is_withheld_when_not_allowed_even_if_requested() {
        let mut req = ChatRequest::new("claude-haiku-4-5");
        req.messages = vec![Message::user_text("hi")];
        req.effort = Some(Effort::High);
        let body = build_body(&req, false, false).unwrap();
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn the_adapter_never_sends_a_thinking_field() {
        // docs/design/68 §11: current models either 400 on an explicit
        // `{"type":"disabled"}`/`budget_tokens` or silently degrade under
        // it. This build must never construct either shape, whatever the
        // effort/think inputs are.
        for (effort, think) in [
            (None, None),
            (None, Some(false)),
            (Some(Effort::Low), None),
            (Some(Effort::Max), Some(false)),
        ] {
            let mut req = ChatRequest::new("claude-opus-5");
            req.messages = vec![Message::user_text("hi")];
            req.effort = effort;
            req.think = think;
            let body = build_body(&req, true, false).unwrap();
            assert!(body.get("thinking").is_none(), "{effort:?}/{think:?}");
        }
    }

    #[test]
    fn fast_mode_sets_the_speed_field_only_when_enabled() {
        let mut req = ChatRequest::new("claude-opus-5");
        req.messages = vec![Message::user_text("hi")];
        let on = build_body(&req, true, true).unwrap();
        assert_eq!(on["speed"], "fast");
        let off = build_body(&req, true, false).unwrap();
        assert!(off.get("speed").is_none());
    }

    #[test]
    fn rejects_effort_detects_the_parameter_by_name() {
        assert!(rejects_effort(
            "output_config.effort: extra fields not permitted"
        ));
        assert!(rejects_effort("Unknown parameter: 'effort'"));
        assert!(!rejects_effort("model does not exist"));
    }
}
