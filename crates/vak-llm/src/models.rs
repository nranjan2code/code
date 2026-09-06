//! Live model discovery: ask each provider what the supplied key can
//! actually reach, rather than shipping a curated table that drifts every
//! time a provider ships a model.
//!
//! Three response shapes cover every provider we speak to:
//!   - OpenAI-compatible (`openai`, `openai-responses`, `openrouter`,
//!     `openrouter-responses`, `opencode-zen`, `ollama`): `GET {base}/models`
//!     → `{ "data": [{ "id" }] }`
//!   - Anthropic: same path but `x-api-key` + `anthropic-version` headers.
//!   - Google: `GET {base}/models?key=…` → `{ "models": [{ "name": "models/x" }] }`

use std::time::Duration;

use crate::error::LlmError;
use crate::registry::ProviderAuth;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelContext {
    pub input_tokens: u64,
    pub output_tokens: Option<u64>,
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

/// Map a non-success status onto the same error taxonomy the chat paths
/// use, so callers can distinguish "your key is wrong" from "provider is
/// down" without parsing strings.
fn status_error(status: u16, body: String) -> LlmError {
    let message = if body.trim().is_empty() {
        "no response body".to_string()
    } else {
        body.chars().take(400).collect()
    };
    match status {
        401 | 403 => LlmError::Auth(message),
        429 => LlmError::RateLimit {
            message,
            retry_after_secs: None,
        },
        400 | 404 | 422 => LlmError::InvalidRequest(message),
        503 | 529 => LlmError::Overloaded(message),
        _ => LlmError::Api { status, message },
    }
}

async fn read_json(res: reqwest::Response) -> Result<serde_json::Value, LlmError> {
    let status = res.status().as_u16();
    let body = res
        .text()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, body));
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
                let res = req
                    .send()
                    .await
                    .map_err(|e| LlmError::Network(e.to_string()))?;
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
                    .query(&[("key", auth.api_key.as_str()), ("pageSize", "1000")]);
                if let Some(cursor) = &token {
                    req = req.query(&[("pageToken", cursor.as_str())]);
                }
                let res = req
                    .send()
                    .await
                    .map_err(|e| LlmError::Network(e.to_string()))?;
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
            let res = client
                .get(format!("{base}/models"))
                .bearer_auth(&auth.api_key)
                .send()
                .await
                .map_err(|e| LlmError::Network(e.to_string()))?;
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
        }),
        _ => {
            let input_tokens = data
                .get("top_provider")
                .and_then(|v| v.get("context_length"))
                .and_then(|v| v.as_u64())
                .or_else(|| data.get("context_length").and_then(|v| v.as_u64()))?;
            Some(ModelContext {
                input_tokens,
                output_tokens: data
                    .get("top_provider")
                    .and_then(|v| v.get("max_completion_tokens"))
                    .and_then(|v| v.as_u64()),
            })
        }
    }
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
            client
                .get(format!("{base}/models/{model}"))
                .query(&[("key", auth.api_key.as_str())])
                .send()
                .await
        }
        "openrouter" => {
            client
                .get(format!("{base}/models/{model}"))
                .bearer_auth(&auth.api_key)
                .send()
                .await
        }
        _ => return Ok(None),
    }
    .map_err(|e| LlmError::Network(e.to_string()))?;
    Ok(context_from_json(provider, &read_json(response).await?))
}

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
        "opencode-zen" => Some("https://opencode.ai/zen/v1"),
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
                output_tokens: Some(8192)
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
            "ollama",
        ] {
            assert!(default_base_url(p).is_some(), "{p} has no default base url");
        }
    }

    #[test]
    fn unauthorised_maps_to_auth_error() {
        assert!(matches!(
            status_error(401, "bad key".into()),
            LlmError::Auth(_)
        ));
    }
}
