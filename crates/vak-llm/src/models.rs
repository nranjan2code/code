//! Live model discovery: ask each provider what the supplied key can
//! actually reach, rather than shipping a curated table that drifts every
//! time a provider ships a model.
//!
//! Three response shapes cover every provider we speak to:
//!   - OpenAI-compatible (`openai`, `openai-responses`, `openrouter`,
//!     `openrouter-responses`, `opencode-zen`, `nvidia`, `ollama`, Bedrock Mantle):
//!     `GET {base}/models`
//!     → `{ "data": [{ "id" }] }`
//!   - Anthropic: same path but `x-api-key` + `anthropic-version` headers.
//!   - Google: `GET {base}/models` with `x-goog-api-key` → `{ "models": [{ "name": "models/x" }] }`

use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::error::LlmError;
use crate::registry::ProviderAuth;
use crate::{Provider, RequestAdmission, Usage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelContext {
    pub input_tokens: u64,
    pub output_tokens: Option<u64>,
    /// Provider-reported quantisation label (Ollama `/api/show`
    /// `details.quantization_level`, e.g. "Q4_K_M"). `None` when the
    /// provider does not publish one — a `CapacityProfile` keys on this so a
    /// requantised model is measured fresh rather than inheriting a stale
    /// profile (docs/design/68-context-engine.md §1).
    pub quantisation: Option<String>,
}

const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
/// Hard stop on paging so a malformed cursor can never loop forever.
const MAX_PAGES: usize = 20;

fn http() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .build()
        .map_err(|e| LlmError::Network(e.to_string()))
}

/// Send a provider metadata request through the same account request and
/// concurrency gate used by inference. Discovery endpoints do not have a
/// model-token estimate; a received HTTP response still consumes one request.
async fn send_admitted(
    provider: &dyn Provider,
    route_provider: &str,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, LlmError> {
    let cancel = CancellationToken::new();
    let started = tokio::time::Instant::now();
    let mut admission = tokio::time::timeout(
        DISCOVERY_TIMEOUT,
        RequestAdmission::acquire_account_request(provider, route_provider, &cancel),
    )
    .await
    .map_err(|_| LlmError::Network("provider discovery admission timed out".into()))??;
    let remaining = DISCOVERY_TIMEOUT.saturating_sub(started.elapsed());
    let response = tokio::time::timeout(remaining, request.send())
        .await
        .map_err(|_| LlmError::Network("provider discovery request timed out".into()))?
        .map_err(|error| LlmError::Network(error.to_string()))?;

    if let Some(retry_after) = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(crate::openai::parse_retry_after)
        && matches!(response.status().as_u16(), 429 | 503 | 529)
    {
        crate::RateLimitGate::for_key(provider.rate_limit_key()).observe_limit(
            Some(Duration::from_secs(retry_after)),
            Duration::from_secs(1),
        );
    }
    admission.settle(&Usage::default());
    Ok(response)
}

/// Map a non-success status onto the same error taxonomy the chat paths
/// use, so callers can distinguish "your key is wrong" from "provider is
/// down" without parsing strings.
fn status_error(status: u16, body: String, retry_after_secs: Option<u64>) -> LlmError {
    let parsed = serde_json::from_str::<serde_json::Value>(&body).ok();
    let code = parsed
        .as_ref()
        .and_then(|value| value.pointer("/error/code").or_else(|| value.get("code")))
        .and_then(serde_json::Value::as_str);
    let message = if body.trim().is_empty() {
        "no response body".to_string()
    } else {
        body.chars().take(400).collect()
    };
    match status {
        401 | 403 => LlmError::Auth(message),
        429 => match crate::google::google_daily_quota_exhaustion(&body) {
            Some(quota) => LlmError::QuotaExhausted(quota),
            None => LlmError::from_rate_limit_code(code, message, retry_after_secs),
        },
        400 => LlmError::classify_400(message),
        404 | 422 => LlmError::InvalidRequest(message),
        503 | 529 => match retry_after_secs {
            Some(retry_after_secs) => LlmError::OverloadedWithRetryAfter {
                message,
                retry_after_secs,
            },
            None => LlmError::Overloaded(message),
        },
        _ => LlmError::Api { status, message },
    }
}

async fn read_json(res: reqwest::Response) -> Result<serde_json::Value, LlmError> {
    let status = res.status().as_u16();
    let retry_after_secs = res
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(crate::openai::parse_retry_after);
    let body = res
        .text()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, body, retry_after_secs));
    }
    serde_json::from_str(&body).map_err(|e| LlmError::Parse(e.to_string()))
}

/// Ask `provider` which models `auth` unlocks. Returns ids sorted and
/// de-duplicated; never falls back to a baked-in list.
pub async fn list_models(provider: &str, auth: &ProviderAuth) -> Result<Vec<String>, LlmError> {
    let base = auth
        .base_url
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .or_else(|| default_base_url(provider))
        .ok_or_else(|| LlmError::InvalidRequest(format!("no base url for '{provider}'")))?
        .trim_end_matches('/')
        .to_string();
    let client = http()?;
    let provider_adapter = crate::registry::default_registry().get(provider, auth)?;

    let mut ids = match provider {
        // Anthropic pages with `has_more`/`last_id` and defaults to 20 per
        // page, so a single request would silently truncate the catalogue.
        "anthropic" => {
            // The chat base url carries no version segment, so add one here
            // rather than teaching every caller about it.
            let url = if base.ends_with("/v1") {
                format!("{base}/models")
            } else {
                format!("{base}/v1/models")
            };
            let mut out = Vec::new();
            let mut after: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let mut req = client
                    .get(&url)
                    .header("x-api-key", &auth.api_key)
                    .header("anthropic-version", crate::anthropic::ANTHROPIC_VERSION)
                    .query(&[("limit", "1000")]);
                if let Some(cursor) = &after {
                    req = req.query(&[("after_id", cursor.as_str())]);
                }
                let res = send_admitted(provider_adapter.as_ref(), provider, req).await?;
                let json = read_json(res).await?;
                out.extend(collect_data_ids(json.clone()));
                if json.get("has_more").and_then(|v| v.as_bool()) != Some(true) {
                    break;
                }
                match json.get("last_id").and_then(|v| v.as_str()) {
                    Some(cursor) => after = Some(cursor.to_string()),
                    None => break,
                }
            }
            out
        }
        // Google pages with `nextPageToken` and defaults to 50 per page.
        "google" => {
            let mut out = Vec::new();
            let mut token: Option<String> = None;
            for _ in 0..MAX_PAGES {
                let mut req = client
                    .get(format!("{base}/models"))
                    .header("x-goog-api-key", &auth.api_key)
                    .query(&[("pageSize", "1000")]);
                if let Some(cursor) = &token {
                    req = req.query(&[("pageToken", cursor.as_str())]);
                }
                let res = send_admitted(provider_adapter.as_ref(), provider, req).await?;
                let json = read_json(res).await?;
                out.extend(collect_google_names(&json));
                match json.get("nextPageToken").and_then(|v| v.as_str()) {
                    Some(cursor) if !cursor.is_empty() => token = Some(cursor.to_string()),
                    _ => break,
                }
            }
            out
        }
        // Everything else speaks the OpenAI listing shape, which returns
        // the full set in one response.
        _ => {
            let res = send_admitted(
                provider_adapter.as_ref(),
                provider,
                client
                    .get(format!("{base}/models"))
                    .bearer_auth(&auth.api_key),
            )
            .await?;
            collect_data_ids(read_json(res).await?)
        }
    };

    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(LlmError::Parse(
            "provider model catalogue contained no model ids".into(),
        ));
    }
    Ok(ids)
}

fn context_from_json(provider: &str, json: &serde_json::Value) -> Option<ModelContext> {
    let data = json.get("data").unwrap_or(json);
    match provider {
        "google" => Some(ModelContext {
            input_tokens: data.get("inputTokenLimit")?.as_u64()?,
            output_tokens: data.get("outputTokenLimit").and_then(|v| v.as_u64()),
            quantisation: None,
        }),
        "ollama" => {
            // 1. Check modelfile parameters for "num_ctx <N>"
            let param_ctx = data
                .get("parameters")
                .and_then(|p| p.as_str())
                .and_then(|p| {
                    for line in p.lines() {
                        let mut parts = line.split_whitespace();
                        if parts.next() == Some("num_ctx")
                            && let Some(val) = parts.next().and_then(|v| v.parse::<u64>().ok())
                        {
                            return Some(val);
                        }
                    }
                    None
                });

            // 2. Check details.context_length
            let details_ctx = data
                .get("details")
                .and_then(|d| d.get("context_length"))
                .and_then(|v| v.as_u64());

            // 3. Check model_info for any key ending with "context_length" (e.g. qwen35.context_length, gemma4.context_length)
            let model_info_ctx = data
                .get("model_info")
                .and_then(|mi| mi.as_object())
                .and_then(|obj| {
                    obj.iter()
                        .find(|(k, _)| k.ends_with(".context_length") || *k == "context_length")
                        .and_then(|(_, v)| v.as_u64())
                });

            // Unknown stays unknown: an invented window once capped every
            // model whose metadata did not name one at 8,192 tokens.
            let input_tokens = param_ctx.or(details_ctx).or(model_info_ctx)?;

            // `/api/show` reports the running quantisation under
            // `details.quantization_level` (e.g. "Q4_K_M").
            let quantisation = data
                .get("details")
                .and_then(|d| d.get("quantization_level"))
                .and_then(|v| v.as_str())
                .map(str::to_string);

            // Ollama publishes no output limit: the reply shares the window.
            Some(ModelContext {
                input_tokens,
                output_tokens: None,
                quantisation,
            })
        }
        _ => {
            // OpenRouter nests it under `top_provider`; Anthropic reports
            // `max_input_tokens`/`max_tokens`; vLLM-style servers `max_model_len`;
            // others `context_length`/`context_window`. OpenAI itself publishes
            // none, which stays `None` (the configured window applies).
            let top = |key: &str| {
                data.get("top_provider")
                    .and_then(|v| v.get(key))
                    .and_then(|v| v.as_u64())
            };
            let field = |key: &str| data.get(key).and_then(|v| v.as_u64());
            let input_tokens = top("context_length")
                .or_else(|| field("context_length"))
                .or_else(|| field("max_input_tokens"))
                .or_else(|| field("max_model_len"))
                .or_else(|| field("context_window"))
                .or_else(|| field("max_context_length"))?;
            Some(ModelContext {
                input_tokens,
                output_tokens: top("max_completion_tokens")
                    .or_else(|| field("max_completion_tokens"))
                    .or_else(|| field("max_output_tokens"))
                    .or_else(|| field("max_tokens")),
                quantisation: None,
            })
        }
    }
}

/// [`model_context`] for one model, asked once per process and kept: an
/// adapter whose request needs the model's published limits (Ollama's
/// window for `num_ctx`, Anthropic's maximum output for the required
/// `max_tokens`) reads them here. A failed or empty lookup is not kept, so
/// the next request asks again.
pub async fn cached_model_context(
    provider: &str,
    auth: &ProviderAuth,
    model: &str,
) -> Option<ModelContext> {
    type Key = (String, Option<String>, String);
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<Key, ModelContext>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (
        provider.to_string(),
        auth.base_url.clone(),
        model.to_string(),
    );
    if let Some(known) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return Some(known.clone());
    }
    let found = model_context(provider, auth, model).await.ok().flatten()?;
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, found.clone());
    Some(found)
}

/// Fetch the provider-reported context limits for one model. Providers that
/// do not publish machine-readable limits return `None`; callers must retain
/// their conservative configured limit in that case.
pub async fn model_context(
    provider: &str,
    auth: &ProviderAuth,
    model: &str,
) -> Result<Option<ModelContext>, LlmError> {
    let base = auth
        .base_url
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .or_else(|| default_base_url(provider))
        .ok_or_else(|| LlmError::InvalidRequest(format!("no base url for '{provider}'")))?
        .trim_end_matches('/');
    let client = http()?;
    let response = match provider {
        "google" => {
            let provider_adapter = crate::registry::default_registry().get(provider, auth)?;
            send_admitted(
                provider_adapter.as_ref(),
                provider,
                client
                    .get(format!("{base}/models/{model}"))
                    .header("x-goog-api-key", &auth.api_key),
            )
            .await
        }
        "openrouter" => {
            let provider_adapter = crate::registry::default_registry().get(provider, auth)?;
            send_admitted(
                provider_adapter.as_ref(),
                provider,
                client
                    .get(format!("{base}/models/{model}"))
                    .bearer_auth(&auth.api_key),
            )
            .await
        }
        "ollama" => {
            let provider_adapter = crate::registry::default_registry().get(provider, auth)?;
            let root = base.trim_end_matches("/v1");
            let req = client
                .post(format!("{root}/api/show"))
                .json(&serde_json::json!({ "name": model }));
            let req = if !auth.api_key.is_empty() {
                req.bearer_auth(&auth.api_key)
            } else {
                req
            };
            match send_admitted(provider_adapter.as_ref(), provider, req).await {
                Ok(res) if res.status().is_success() => {
                    let json = read_json(res).await?;
                    if let Some(caps) = ollama_capabilities(&json) {
                        record_capabilities(root, model, caps);
                    }
                    return Ok(context_from_json("ollama", &json));
                }
                _ => return Ok(None),
            }
        }
        "anthropic" => {
            let provider_adapter = crate::registry::default_registry().get(provider, auth)?;
            let url = if base.ends_with("/v1") {
                format!("{base}/models/{model}")
            } else {
                format!("{base}/v1/models/{model}")
            };
            send_admitted(
                provider_adapter.as_ref(),
                provider,
                client
                    .get(url)
                    .header("x-api-key", &auth.api_key)
                    .header("anthropic-version", crate::anthropic::ANTHROPIC_VERSION),
            )
            .await
        }
        // OpenAI-compatible endpoints: ask for the one model. A server that
        // does not publish limits answers without them, and that is `None`.
        "openai"
        | "openai-responses"
        | "openrouter-responses"
        | "bedrock"
        | "opencode-zen"
        | "nvidia" => {
            let provider_adapter = crate::registry::default_registry().get(provider, auth)?;
            let response = send_admitted(
                provider_adapter.as_ref(),
                provider,
                client
                    .get(format!("{base}/models/{model}"))
                    .bearer_auth(&auth.api_key),
            )
            .await;
            return match response {
                Ok(res) if res.status().is_success() => {
                    Ok(context_from_json(provider, &read_json(res).await?))
                }
                _ => Ok(None),
            };
        }
        _ => return Ok(None),
    }
    .map_err(|e| LlmError::Network(e.to_string()))?;
    Ok(context_from_json(provider, &read_json(response).await?))
}

/// What Ollama's `/api/show` reports a model can do: its `capabilities`
/// list names `thinking` when the model's reasoning can be switched.
fn ollama_capabilities(json: &serde_json::Value) -> Option<vak_control::ModelCapabilities> {
    let listed = json.get("capabilities")?.as_array()?;
    let thinks = listed.iter().any(|c| c.as_str() == Some("thinking"));
    Some(vak_control::ModelCapabilities {
        reasoning: if thinks {
            vak_control::ReasoningSupport::Toggle
        } else {
            vak_control::ReasoningSupport::Fixed
        },
        ..Default::default()
    })
}

/// Anthropic per-model capability flags this build conditions behaviour on
/// (docs/design/68-context-engine.md §11 "Anthropic" row). The Models API's
/// `capabilities` object carries many more fields; these are the ones an
/// adapter actually reads before it decides to send an optional parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnthropicCapabilities {
    pub effort: bool,
    /// Best-effort key name — Anthropic's public docs describe the
    /// `capabilities` tree's `effort`/`thinking`/`image_input` members but
    /// do not (yet) enumerate a fast-mode entry. Absent or unrecognized
    /// shapes read as `false`, which only ever means "do not try fast mode
    /// for this model" — never a hard failure (invariant 9: unknown stays
    /// explicitly unavailable, it is never guessed into `true`).
    pub fast_mode: bool,
}

fn capability_supported(json: &serde_json::Value, name: &str) -> bool {
    json.pointer(&format!("/capabilities/{name}/supported"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Fetches one Anthropic model's capability flags from `GET /v1/models/{id}`
/// (invariant 9: discovered, never hardcoded — a static per-model-id table
/// would drift the moment a new model shipped or an old key lost access to
/// one). Meaningful for the `anthropic` provider only; other providers do
/// not publish this shape yet.
pub async fn anthropic_model_capabilities(
    auth: &ProviderAuth,
    model: &str,
) -> Result<AnthropicCapabilities, LlmError> {
    let base = auth
        .base_url
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .or_else(|| default_base_url("anthropic"))
        .ok_or_else(|| LlmError::InvalidRequest("no base url for 'anthropic'".into()))?
        .trim_end_matches('/')
        .to_string();
    let url = if base.ends_with("/v1") {
        format!("{base}/models/{model}")
    } else {
        format!("{base}/v1/models/{model}")
    };
    let provider_adapter = crate::registry::default_registry().get("anthropic", auth)?;
    let response = send_admitted(
        provider_adapter.as_ref(),
        "anthropic",
        http()?
            .get(&url)
            .header("x-api-key", &auth.api_key)
            .header("anthropic-version", crate::anthropic::ANTHROPIC_VERSION),
    )
    .await?;
    let json = read_json(response).await?;
    Ok(AnthropicCapabilities {
        effort: capability_supported(&json, "effort"),
        fast_mode: capability_supported(&json, "fast_mode"),
    })
}

/// In-process memory of what each model on each wire was reported or
/// shown to honour (docs/design/88-control-plane.md §4), and of each
/// Anthropic model's fast-mode support. Never persisted; it only narrows
/// what an adapter sends next, and every dispatch records what it knew in
/// `AssistantMessage::applied`.
#[derive(Default)]
struct CapabilityCache {
    models: std::collections::HashMap<String, vak_control::ModelCapabilities>,
    fast_mode: std::collections::HashMap<String, bool>,
}

fn capability_cache() -> &'static std::sync::Mutex<CapabilityCache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<CapabilityCache>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(CapabilityCache::default()))
}

fn capability_key(wire: &str, model: &str) -> String {
    let wire = wire.trim_end_matches('/');
    format!("{}\n{model}", wire.strip_suffix("/v1").unwrap_or(wire))
}

/// What `model` on `wire` (the adapter's endpoint) can honour: `reach`,
/// what the adapter can pass through at all, narrowed by what the provider
/// reported and what a refused request showed.
pub fn capabilities(
    wire: &str,
    reach: vak_control::ModelCapabilities,
    model: &str,
) -> vak_control::ModelCapabilities {
    capability_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .models
        .get(&capability_key(wire, model))
        .map_or(reach, |known| reach.meet(*known))
}

/// Records what was reported or shown about `model` on `wire`, narrowing
/// what was known.
pub fn record_capabilities(wire: &str, model: &str, caps: vak_control::ModelCapabilities) {
    let mut cache = capability_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = cache.models.entry(capability_key(wire, model)).or_default();
    *entry = entry.meet(caps);
}

/// The settings of one dispatch: what the model can honour of those the
/// step asked for, sent once more without them when the provider refuses
/// the request, and learned from when the request without them is taken.
/// No wording of the refusal is read: the second answer is the evidence.
pub struct DispatchSettings {
    wire: String,
    model: String,
    capabilities: vak_control::ModelCapabilities,
    pub settings: vak_control::StepSettings,
    refused: Option<vak_control::StepSettings>,
    /// The constraints this dispatch first sent, when a refusal made it send
    /// the request again without them (strict tools, schema output).
    dropped_constraints: Option<vak_control::StepSettings>,
    forced: vak_control::Forced,
}

impl DispatchSettings {
    pub fn new(
        wire: &str,
        reach: vak_control::ModelCapabilities,
        model: &str,
        asked: vak_control::StepSettings,
    ) -> Self {
        let capabilities = capabilities(wire, reach, model);
        Self {
            wire: wire.to_string(),
            model: model.to_string(),
            capabilities,
            settings: capabilities.honour(asked),
            refused: None,
            dropped_constraints: None,
            forced: vak_control::Forced::NotAsked,
        }
    }

    /// Notes whether the request offers tools: strict tools mean nothing
    /// without them, so none is asked for.
    pub fn offering_tools(mut self, offers: bool) -> Self {
        self.settings.strict_tools &= offers;
        self
    }

    /// Notes whether the request asks for its output in a schema, and asks
    /// for it to be enforced where the model can take that.
    pub fn asking_output(mut self, asks: bool) -> Self {
        self.settings.schema_output = asks && self.capabilities.schema_output.sends();
        self
    }

    /// Notes that the request names a tool the step must call, so the
    /// receipt says how it was put to the provider.
    pub fn requiring(mut self, must_call: bool) -> Self {
        self.forced = match (must_call, self.capabilities.tool_choice.sends()) {
            (false, _) => vak_control::Forced::NotAsked,
            (true, true) => vak_control::Forced::Choice,
            (true, false) => vak_control::Forced::OnlyOffered,
        };
        self
    }

    /// After a request the provider refused as invalid: whether to send it
    /// again, first without strict tools, then without the optional
    /// settings. Each is dropped once.
    pub fn retry_without_settings(&mut self) -> bool {
        if self.settings.strict_tools || self.settings.schema_output {
            self.dropped_constraints = Some(vak_control::StepSettings {
                strict_tools: self.settings.strict_tools,
                schema_output: self.settings.schema_output,
                ..Default::default()
            });
            self.settings.strict_tools = false;
            self.settings.schema_output = false;
            return true;
        }
        if self.refused.is_some() || self.settings == vak_control::StepSettings::default() {
            return false;
        }
        self.refused = Some(self.settings);
        self.settings = vak_control::StepSettings::default();
        true
    }

    /// After a request the provider took: records what a refusal before it
    /// showed, and returns what this dispatch sent.
    pub fn accepted(mut self) -> vak_control::Applied {
        // Taken as soon as the constraints were dropped: they were what the
        // provider refused. Taken only once the settings went too: the
        // settings were, and the constraints were never tried without them.
        let refused = match (self.dropped_constraints, self.refused) {
            (Some(constraints), None) => Some(constraints),
            (_, refused) => refused,
        };
        if let Some(sent) = refused {
            let refusing = vak_control::ModelCapabilities::default().refusing(sent);
            record_capabilities(&self.wire, &self.model, refusing);
            self.capabilities = self.capabilities.meet(refusing);
        }
        vak_control::Applied {
            settings: self.settings,
            capabilities: self.capabilities,
            forced: self.forced,
        }
    }
}

/// How a wire is told to call one named tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolChoiceWire {
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
    Google,
    /// No such switch: the request offers only that tool.
    OnlyOffered,
}

/// Puts a step's required tool into a request body already carrying its
/// tools. Nothing is done for a step that requires none.
pub fn require_tool(body: &mut serde_json::Value, must_call: Option<&str>, wire: ToolChoiceWire) {
    let Some(name) = must_call else {
        return;
    };
    match wire {
        ToolChoiceWire::Anthropic => {
            body["tool_choice"] = serde_json::json!({"type": "tool", "name": name});
        }
        ToolChoiceWire::OpenAiChat => {
            body["tool_choice"] =
                serde_json::json!({"type": "function", "function": {"name": name}});
        }
        ToolChoiceWire::OpenAiResponses => {
            body["tool_choice"] = serde_json::json!({"type": "function", "name": name});
        }
        ToolChoiceWire::Google => {
            body["toolConfig"] = serde_json::json!({
                "functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": [name]}
            });
        }
        ToolChoiceWire::OnlyOffered => {
            if let Some(tools) = body["tools"].as_array_mut() {
                tools.retain(|tool| tool["function"]["name"] == name);
            }
        }
    }
}

/// Whether the adapter should attempt `speed: "fast"` for `model`. Defaults
/// to `false` (an unknown model is never assumed to support a research
/// preview) until [`record_anthropic_capabilities`] has looked it up.
pub fn anthropic_fast_mode_allowed(model: &str) -> bool {
    capability_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .fast_mode
        .get(model)
        .copied()
        .unwrap_or(false)
}

/// Whether `model`'s fast-mode support has already been learned one way or
/// the other, so a caller can skip a redundant discovery fetch.
pub fn anthropic_fast_mode_known(model: &str) -> bool {
    capability_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .fast_mode
        .contains_key(model)
}

/// Records a freshly discovered capability set
/// ([`anthropic_model_capabilities`]) so later requests for `model` skip
/// both the network round trip and the conservative first-attempt default.
pub fn record_anthropic_capabilities(wire: &str, model: &str, caps: AnthropicCapabilities) {
    record_capabilities(
        wire,
        model,
        vak_control::ModelCapabilities {
            reasoning: if caps.effort {
                vak_control::ReasoningSupport::Graded
            } else {
                vak_control::ReasoningSupport::Fixed
            },
            ..Default::default()
        },
    );
    capability_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .fast_mode
        .insert(model.to_string(), caps.fast_mode);
}

/// NVIDIA NIM's hosted, OpenAI-compatible endpoint.
pub const NVIDIA_DEFAULT_BASE_URL: &str = "https://integrate.api.nvidia.com/v1";

/// The provider's documented API host, for callers that never set an
/// override. These are endpoints, not a model catalogue — the model list
/// itself always comes off the wire.
fn default_base_url(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some(crate::anthropic::DEFAULT_BASE_URL),
        "openai" => Some(crate::openai::OPENAI_DEFAULT_BASE_URL),
        "openai-responses" => Some(crate::openai_responses::OPENAI_RESPONSES_DEFAULT_BASE_URL),
        "google" => Some(crate::google::GOOGLE_DEFAULT_BASE_URL),
        "openrouter" => Some("https://openrouter.ai/api/v1"),
        "openrouter-responses" => Some("https://openrouter.ai/api/v1"),
        "bedrock" => Some("https://bedrock-mantle.us-east-1.api.aws/v1"),
        "opencode-zen" => Some("https://opencode.ai/zen/v1"),
        "nvidia" => Some(NVIDIA_DEFAULT_BASE_URL),
        "ollama" => Some("http://localhost:11434/v1"),
        _ => None,
    }
}

/// Google returns fully qualified `models/gemini-x`; the chat path wants
/// the bare id.
fn collect_google_names(json: &serde_json::Value) -> Vec<String> {
    json.get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                .map(|n| n.trim_start_matches("models/").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn collect_data_ids(json: serde_json::Value) -> Vec<String> {
    json.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn openai_shape_yields_ids() {
        let json = serde_json::json!({ "data": [{ "id": "gpt-4.1" }, { "id": "o3" }] });
        assert_eq!(collect_data_ids(json), vec!["gpt-4.1", "o3"]);
    }

    #[test]
    fn missing_data_is_still_empty_for_the_parser() {
        assert!(collect_data_ids(serde_json::json!({})).is_empty());
    }

    #[test]
    fn google_names_are_stripped_of_the_models_prefix() {
        let json = serde_json::json!({ "models": [{ "name": "models/gemini-2.5-pro" }] });
        assert_eq!(collect_google_names(&json), vec!["gemini-2.5-pro"]);
    }

    #[test]
    fn context_metadata_prefers_provider_specific_limit() {
        let json = serde_json::json!({
            "data": {"context_length": 131072, "top_provider": {"context_length": 65536, "max_completion_tokens": 8192}}
        });
        assert_eq!(
            context_from_json("openrouter", &json),
            Some(ModelContext {
                input_tokens: 65536,
                output_tokens: Some(8192),
                quantisation: None,
            })
        );
    }

    #[test]
    fn every_supported_provider_has_a_default_endpoint() {
        for p in [
            "anthropic",
            "openai",
            "openai-responses",
            "google",
            "openrouter",
            "openrouter-responses",
            "opencode-zen",
            "nvidia",
            "ollama",
        ] {
            assert!(default_base_url(p).is_some(), "{p} has no default base url");
        }
    }

    #[test]
    fn ollama_context_extracted_from_model_info() {
        let json = serde_json::json!({
            "model_info": {
                "qwen35.context_length": 32768
            }
        });
        assert_eq!(
            context_from_json("ollama", &json),
            Some(ModelContext {
                input_tokens: 32768,
                output_tokens: None,
                quantisation: None,
            })
        );
    }

    #[test]
    fn context_is_read_from_anthropic_and_vllm_shapes_and_absent_for_openai() {
        let anthropic =
            serde_json::json!({"id": "m", "max_input_tokens": 200000, "max_tokens": 64000});
        let got = context_from_json("anthropic", &anthropic).unwrap();
        assert_eq!(
            (got.input_tokens, got.output_tokens),
            (200_000, Some(64_000))
        );
        let vllm = serde_json::json!({"id": "m", "max_model_len": 32768});
        assert_eq!(
            context_from_json("openai", &vllm).unwrap().input_tokens,
            32_768
        );
        let openai = serde_json::json!({"id": "gpt", "object": "model", "owned_by": "openai"});
        assert!(context_from_json("openai", &openai).is_none());
    }

    #[test]
    fn ollama_context_prefers_modelfile_num_ctx() {
        let json = serde_json::json!({
            "parameters": "temperature 0.7\nnum_ctx 16384\ntop_p 0.9",
            "model_info": {
                "general.context_length": 131072
            }
        });
        assert_eq!(
            context_from_json("ollama", &json),
            Some(ModelContext {
                input_tokens: 16384,
                output_tokens: None,
                quantisation: None,
            })
        );
    }

    #[test]
    fn ollama_context_parses_quantisation_from_details() {
        let json = serde_json::json!({
            "details": {"quantization_level": "Q4_K_M"},
            "model_info": {"general.context_length": 8192}
        });
        assert_eq!(
            context_from_json("ollama", &json),
            Some(ModelContext {
                input_tokens: 8192,
                output_tokens: None,
                quantisation: Some("Q4_K_M".to_string()),
            })
        );
    }

    #[test]
    fn unauthorised_maps_to_auth_error() {
        assert!(matches!(
            status_error(401, "bad key".into(), None),
            LlmError::Auth(_)
        ));
    }

    #[test]
    fn capability_supported_reads_the_nested_flag() {
        let json = serde_json::json!({
            "capabilities": {
                "effort": {"supported": true},
                "thinking": {"supported": false},
            }
        });
        assert!(capability_supported(&json, "effort"));
        assert!(!capability_supported(&json, "thinking"));
        // Absent / unrecognized keys read as false, never guessed true.
        assert!(!capability_supported(&json, "fast_mode"));
        assert!(!capability_supported(&serde_json::json!({}), "effort"));
    }

    // Each test below uses its own unique model id: the capability cache is
    // a process-wide static, and tests in this module run concurrently.
    #[test]
    fn strict_tools_refused_alone_are_learned_and_the_settings_kept() {
        use vak_control::{Depth, ModelCapabilities, Reasoning, StepSettings, Support};
        let reach = ModelCapabilities {
            strict_tools: Support::Unknown,
            reasoning: vak_control::ReasoningSupport::Graded,
            ..Default::default()
        };
        let asked = StepSettings {
            reasoning: Reasoning::Depth(Depth::High),
            strict_tools: true,
            ..Default::default()
        };
        let mut dispatch =
            DispatchSettings::new("wire-strict", reach, "test-model-strict-1", asked)
                .offering_tools(true);
        assert!(dispatch.settings.strict_tools);
        assert!(dispatch.retry_without_settings());
        assert!(!dispatch.settings.strict_tools, "strict tools go first");
        assert_eq!(dispatch.settings.reasoning, Reasoning::Depth(Depth::High));
        let applied = dispatch.accepted();
        assert_eq!(applied.capabilities.strict_tools, Support::No);
        let next = DispatchSettings::new("wire-strict", reach, "test-model-strict-1", asked);
        assert!(
            !next.settings.strict_tools,
            "learned for this wire and model"
        );
        assert_eq!(next.settings.reasoning, Reasoning::Depth(Depth::High));
        let without_tools =
            DispatchSettings::new("wire-strict", reach, "test-model-strict-2", asked)
                .offering_tools(false);
        assert!(!without_tools.settings.strict_tools);
    }

    #[test]
    fn strict_tools_are_not_blamed_when_the_settings_were_refused() {
        use vak_control::{Depth, ModelCapabilities, Reasoning, StepSettings, Support};
        let reach = ModelCapabilities {
            strict_tools: Support::Unknown,
            reasoning: vak_control::ReasoningSupport::Graded,
            ..Default::default()
        };
        let asked = StepSettings {
            reasoning: Reasoning::Depth(Depth::High),
            strict_tools: true,
            ..Default::default()
        };
        let mut dispatch =
            DispatchSettings::new("wire-strict", reach, "test-model-strict-3", asked)
                .offering_tools(true);
        assert!(dispatch.retry_without_settings());
        assert!(dispatch.retry_without_settings(), "then the settings");
        assert!(!dispatch.retry_without_settings(), "nothing left to drop");
        let applied = dispatch.accepted();
        assert_eq!(applied.capabilities.strict_tools, Support::Unknown);
    }

    #[test]
    fn a_refused_setting_is_learned_only_once_the_request_without_it_is_taken() {
        use vak_control::{Depth, ModelCapabilities, Reasoning, ReasoningSupport, StepSettings};
        let asked = StepSettings {
            reasoning: Reasoning::Depth(Depth::High),
            ..Default::default()
        };
        let reach = ModelCapabilities {
            reasoning: ReasoningSupport::Graded,
            ..Default::default()
        };
        let mut dispatch = DispatchSettings::new("wire-a", reach, "test-model-refused-1", asked);
        assert_eq!(dispatch.settings, asked);
        assert!(dispatch.retry_without_settings());
        assert!(!dispatch.retry_without_settings(), "sent again once only");
        assert_eq!(dispatch.settings, StepSettings::default());
        let applied = dispatch.accepted();
        assert_eq!(applied.capabilities.reasoning, ReasoningSupport::Fixed);
        let next = DispatchSettings::new("wire-a", reach, "test-model-refused-1", asked);
        assert_eq!(next.settings.reasoning, Reasoning::Default);
        let other_wire = DispatchSettings::new("wire-b", reach, "test-model-refused-1", asked);
        assert_eq!(
            other_wire.settings, asked,
            "learned for the wire that refused"
        );
    }

    #[test]
    fn fast_mode_defaults_to_not_allowed_and_not_known_for_an_unseen_model() {
        assert!(!anthropic_fast_mode_allowed("test-model-fast-unseen-3"));
        assert!(!anthropic_fast_mode_known("test-model-fast-unseen-3"));
    }

    #[test]
    fn recording_discovered_capabilities_makes_fast_mode_known_and_gates_on_the_flag() {
        record_anthropic_capabilities(
            "wire-c",
            "test-model-fast-supported-4",
            AnthropicCapabilities {
                effort: true,
                fast_mode: true,
            },
        );
        assert!(anthropic_fast_mode_known("test-model-fast-supported-4"));
        assert!(anthropic_fast_mode_allowed("test-model-fast-supported-4"));

        record_anthropic_capabilities(
            "wire-c",
            "test-model-fast-unsupported-5",
            AnthropicCapabilities {
                effort: true,
                fast_mode: false,
            },
        );
        assert!(anthropic_fast_mode_known("test-model-fast-unsupported-5"));
        assert!(!anthropic_fast_mode_allowed(
            "test-model-fast-unsupported-5"
        ));
    }

    #[test]
    fn a_required_tool_is_named_in_each_wires_own_way() {
        let tools = || {
            serde_json::json!({"tools": [
                {"type": "function", "function": {"name": "read"}},
                {"type": "function", "function": {"name": "emit_steps_card"}},
            ]})
        };
        let mut untouched = tools();
        require_tool(&mut untouched, None, ToolChoiceWire::Anthropic);
        assert_eq!(untouched, tools());

        let named = |wire| {
            let mut body = tools();
            require_tool(&mut body, Some("emit_steps_card"), wire);
            body
        };
        assert_eq!(
            named(ToolChoiceWire::Anthropic)["tool_choice"],
            serde_json::json!({"type": "tool", "name": "emit_steps_card"})
        );
        assert_eq!(
            named(ToolChoiceWire::OpenAiChat)["tool_choice"]["function"]["name"],
            "emit_steps_card"
        );
        assert_eq!(
            named(ToolChoiceWire::OpenAiResponses)["tool_choice"]["name"],
            "emit_steps_card"
        );
        assert_eq!(
            named(ToolChoiceWire::Google)["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"]
                [0],
            "emit_steps_card"
        );
        let only = named(ToolChoiceWire::OnlyOffered);
        assert!(only.get("tool_choice").is_none());
        assert_eq!(only["tools"].as_array().map(Vec::len), Some(1));
        assert_eq!(only["tools"][0]["function"]["name"], "emit_steps_card");
    }
}
