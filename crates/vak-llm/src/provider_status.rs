//! Provider-side account diagnostics that are safe to expose to operators.
//!
//! Credentials are used only for the request and are never included in the
//! returned status or error text.

use serde::Serialize;
use std::time::Duration;

use crate::error::LlmError;
use crate::registry::ProviderAuth;

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

/// Inspect provider account metadata when the provider publishes it.
/// OpenRouter is currently the only built-in provider with a documented
/// authenticated key and credit introspection endpoint.
pub async fn inspect(provider: &str, auth: &ProviderAuth) -> Result<ProviderStatus, LlmError> {
    if provider != "openrouter" {
        return Err(LlmError::InvalidRequest(format!(
            "provider '{provider}' does not expose supported account metadata"
        )));
    }
    let base = auth
        .base_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or("https://openrouter.ai/api/v1")
        .trim_end_matches('/');
    let client = client()?;
    let key_response = client
        .get(format!("{base}/key"))
        .bearer_auth(&auth.api_key)
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    let (key_status, key_json) = json(key_response).await?;
    if !(200..300).contains(&key_status) {
        return Err(status_error(key_status, &key_json, &auth.api_key));
    }
    let key = key_json.get("data").unwrap_or(&key_json);

    let credits_response = client
        .get(format!("{base}/credits"))
        .bearer_auth(&auth.api_key)
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;
    let (credits_status, credits_json) = json(credits_response).await?;
    if !(200..300).contains(&credits_status) {
        return Err(status_error(credits_status, &credits_json, &auth.api_key));
    }
    let credits = credits_json.get("data").unwrap_or(&credits_json);
    let rate_limit = key.get("rate_limit");
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
            },
        ));
        assert!(matches!(result, Err(LlmError::InvalidRequest(_))));
    }
}
