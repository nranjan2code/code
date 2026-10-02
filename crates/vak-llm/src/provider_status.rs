//! Provider-side account diagnostics that are safe to expose to operators.
//!
//! Credentials are used only for the request and are never included in the
//! returned status or error text.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use crate::error::LlmError;
use crate::registry::ProviderAuth;
use crate::{Provider, RequestAdmission, Usage};

#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub provider: String,
    pub reachable: bool,
    pub authenticated: bool,
    pub key_kind: Option<String>,
    pub is_free_tier: Option<bool>,
    pub usage_usd: Option<f64>,
    pub usage_daily_usd: Option<f64>,
    pub usage_monthly_usd: Option<f64>,
    pub limit_usd: Option<f64>,
    pub limit_remaining_usd: Option<f64>,
    pub free_model_daily_requests_limit: Option<u64>,
    pub free_model_daily_requests_remaining: Option<u64>,
    pub free_model_daily_requests_used: Option<u64>,
    pub credits_usd: Option<f64>,
    pub rate_limit_requests: Option<i64>,
    pub rate_limit_interval: Option<String>,
}

fn client() -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| LlmError::Network(e.to_string()))
}

async fn send_admitted(
    provider: &dyn Provider,
    route_provider: &str,
    request: reqwest::RequestBuilder,
    cancel: &CancellationToken,
) -> Result<reqwest::Response, LlmError> {
    let mut admission =
        RequestAdmission::acquire_account_request(provider, route_provider, cancel).await?;
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        response = request.send() => response.map_err(|error| LlmError::Network(error.to_string()))?,
    };
    admission.settle(&Usage::default());
    Ok(response)
}

async fn json(response: reqwest::Response) -> Result<(u16, serde_json::Value), LlmError> {
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    let value = serde_json::from_str(&body).map_err(|e| LlmError::Parse(e.to_string()))?;
    Ok((status, value))
}

fn status_error(status: u16, value: &serde_json::Value, secret: &str) -> LlmError {
    let message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("provider status request failed")
        .chars()
        .take(400)
        .collect::<String>();
    let message = if secret.is_empty() {
        message
    } else {
        message.replace(secret, "[redacted]")
    };
    match status {
        401 | 403 => LlmError::Auth(message),
        429 => LlmError::RateLimit {
            message,
            retry_after_secs: None,
        },
        _ => LlmError::Api { status, message },
    }
}

fn number(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(serde_json::Value::as_f64)
}

fn openrouter_refresh_locks() -> &'static Mutex<HashMap<String, Arc<AsyncMutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn openrouter_refreshed_at() -> &'static Mutex<HashMap<String, std::time::Instant>> {
    static LAST: OnceLock<Mutex<HashMap<String, std::time::Instant>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Refresh just OpenRouter's current-key capacity projection at most once per
/// minute for one account. The API key is used for Authorization only; cache
/// keys and signals contain its one-way credential fingerprint.
pub async fn refresh_openrouter_capacity(
    base_url: &str,
    api_key: &str,
    cancel: &CancellationToken,
) -> Result<(), LlmError> {
    let identity = crate::openai::account_capacity_key(base_url, api_key);
    let lock = {
        let mut locks = openrouter_refresh_locks()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if locks.len() >= 256 {
            locks.retain(|key, _| {
                openrouter_refreshed_at()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(key)
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(900))
            });
        }
        locks
            .entry(identity.clone())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    };
    let _single_flight = lock.lock().await;
    {
        let mut refreshed = openrouter_refreshed_at()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if refreshed
            .get(&identity)
            .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
        {
            return Ok(());
        }
        refreshed.insert(identity.clone(), std::time::Instant::now());
    }
    let base = base_url.trim_end_matches('/');
    let client = client()?;
    let provider_adapter = crate::registry::default_registry().get(
        "openrouter",
        &ProviderAuth {
            api_key: api_key.to_string(),
            base_url: Some(base_url.to_string()),
            ..ProviderAuth::default()
        },
    )?;
    let account_gate = crate::RateLimitGate::for_key(identity.clone());
    let observation_sequence = account_gate.next_observation_sequence();
    let response = send_admitted(
        provider_adapter.as_ref(),
        "openrouter",
        client.get(format!("{base}/key")).bearer_auth(api_key),
        cancel,
    )
    .await?;
    let status = response.status().as_u16();
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, &value, api_key));
    }
    observe_openrouter_key_capacity(&value, &account_gate, observation_sequence);
    Ok(())
}

fn observe_openrouter_key_capacity(
    key_json: &serde_json::Value,
    account_gate: &crate::RateLimitGate,
    observation_sequence: u64,
) {
    let key = key_json.get("data").unwrap_or(key_json);
    let Some(free_daily) = key.get("free_model_daily_requests") else {
        return;
    };
    let Some(remaining) = free_daily
        .get("remaining")
        .and_then(serde_json::Value::as_u64)
    else {
        return;
    };
    let limit = free_daily.get("limit").and_then(serde_json::Value::as_u64);
    let now = chrono::Utc::now();
    let reset_after_secs = (now.date_naive() + chrono::Days::new(1))
        .and_hms_opt(0, 0, 0)
        .map(|midnight| (midnight.and_utc() - now).num_seconds().max(0) as u64);
    account_gate.observe_ordered(
        observation_sequence,
        crate::CapacityObservation {
            daily_requests_remaining: Some(remaining),
            daily_requests_limit: limit,
            daily_requests_reset_after_secs: reset_after_secs,
            ..Default::default()
        },
    );
}

/// Inspect provider account metadata when the provider publishes it.
/// OpenRouter is currently the only built-in provider with a documented
/// authenticated key and credit introspection endpoint.
pub async fn inspect(provider: &str, auth: &ProviderAuth) -> Result<ProviderStatus, LlmError> {
    if provider != "openrouter" {
        return Err(LlmError::InvalidRequest(format!(
            "provider '{provider}' does not expose supported account metadata"
        )));
    }
    let identity_base = auth
        .base_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or("https://openrouter.ai/api/v1");
    let base = identity_base.trim_end_matches('/');
    let client = client()?;
    let account_key = crate::openai::account_capacity_key(identity_base, &auth.api_key);
    let provider_adapter = crate::registry::default_registry().get(provider, auth)?;
    let cancel = CancellationToken::new();
    let account_gate = crate::RateLimitGate::for_key(account_key);
    let observation_sequence = account_gate.next_observation_sequence();
    let key_response = send_admitted(
        provider_adapter.as_ref(),
        provider,
        client.get(format!("{base}/key")).bearer_auth(&auth.api_key),
        &cancel,
    )
    .await?;
    let (key_status, key_json) = json(key_response).await?;
    if !(200..300).contains(&key_status) {
        return Err(status_error(key_status, &key_json, &auth.api_key));
    }
    let key = key_json.get("data").unwrap_or(&key_json);
    observe_openrouter_key_capacity(&key_json, &account_gate, observation_sequence);

    let credits_response = send_admitted(
        provider_adapter.as_ref(),
        provider,
        client
            .get(format!("{base}/credits"))
            .bearer_auth(&auth.api_key),
        &cancel,
    )
    .await?;
    let (credits_status, credits_json) = json(credits_response).await?;
    if !(200..300).contains(&credits_status) {
        return Err(status_error(credits_status, &credits_json, &auth.api_key));
    }
    let credits = credits_json.get("data").unwrap_or(&credits_json);
    let rate_limit = key.get("rate_limit");
    let free_daily = key.get("free_model_daily_requests");
    Ok(ProviderStatus {
        provider: provider.to_string(),
        reachable: true,
        authenticated: true,
        key_kind: Some(
            if key
                .get("is_management_key")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                "openrouter-management-key".into()
            } else {
                "openrouter-api-key".into()
            },
        ),
        is_free_tier: key.get("is_free_tier").and_then(serde_json::Value::as_bool),
        usage_usd: number(key.get("usage")),
        usage_daily_usd: number(key.get("usage_daily")),
        usage_monthly_usd: number(key.get("usage_monthly")),
        limit_usd: number(key.get("limit")),
        limit_remaining_usd: number(key.get("limit_remaining")),
        free_model_daily_requests_limit: free_daily
            .and_then(|v| v.get("limit"))
            .and_then(serde_json::Value::as_u64),
        free_model_daily_requests_remaining: free_daily
            .and_then(|v| v.get("remaining"))
            .and_then(serde_json::Value::as_u64),
        free_model_daily_requests_used: free_daily
            .and_then(|v| v.get("used"))
            .and_then(serde_json::Value::as_u64),
        credits_usd: number(credits.get("total_credits")),
        rate_limit_requests: rate_limit
            .and_then(|value| value.get("requests"))
            .and_then(serde_json::Value::as_i64),
        rate_limit_interval: rate_limit
            .and_then(|value| value.get("interval"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_error_never_echoes_a_credential() {
        let error = status_error(
            401,
            &serde_json::json!({"error": {"message": "invalid key"}}),
            "secret",
        );
        assert_eq!(error.to_string(), "authentication failed: invalid key");
    }

    #[test]
    fn status_error_redacts_an_echoed_credential() {
        let error = status_error(
            401,
            &serde_json::json!({"error": {"message": "rejected secret-token"}}),
            "secret-token",
        );
        assert!(!error.to_string().contains("secret-token"));
        assert!(error.to_string().contains("[redacted]"));
    }

    #[test]
    fn unsupported_provider_is_explicit() {
        let result = futures::executor::block_on(inspect(
            "ollama",
            &ProviderAuth {
                api_key: "not-a-secret".into(),
                base_url: None,
                credential_id: None,
                options: Default::default(),
            },
        ));
        assert!(matches!(result, Err(LlmError::InvalidRequest(_))));
    }
}
