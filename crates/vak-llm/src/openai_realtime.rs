//! Provider-neutral helpers for the OpenAI Realtime wire protocol.
//!
//! The model and voice are deliberately supplied by discovery/configuration;
//! this module contains no catalogue or fallback identifiers.  Keeping the
//! wire messages here makes the streaming transport usable by HTTP, desktop,
//! and channel surfaces without duplicating protocol details.

use crate::error::LlmError;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeConfig {
    pub model: String,
    pub voice: Option<String>,
    pub input_format: String,
    pub output_format: String,
}

fn account_capacity_key(endpoint: &str, api_key: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(endpoint) else {
        return crate::gate::route_identity("openai", endpoint, api_key);
    };
    let secure = url.scheme() == "wss" || url.scheme() == "https";
    let _ = url.set_scheme(if secure { "https" } else { "http" });
    let path = url.path().trim_end_matches('/');
    let path = path.strip_suffix("/realtime").unwrap_or(path).to_string();
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    crate::openai::account_capacity_key(url.as_str().trim_end_matches('/'), api_key)
}

impl RealtimeConfig {
    pub fn validate(&self) -> Result<(), LlmError> {
        for (name, value) in [
            ("model", self.model.as_str()),
            ("input_format", self.input_format.as_str()),
            ("output_format", self.output_format.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(LlmError::InvalidRequest(format!(
                    "realtime {name} is required"
                )));
            }
        }
        if self.voice.as_deref().is_some_and(|v| v.trim().is_empty()) {
            return Err(LlmError::InvalidRequest(
                "realtime voice cannot be empty".into(),
            ));
        }
        Ok(())
    }
}

/// Build the initial session update. Empty optional voice is omitted so the
/// provider can apply its configured default.
pub fn build_session_update(
    config: &RealtimeConfig,
    instructions: Option<&str>,
) -> Result<Value, LlmError> {
    config.validate()?;
    let mut session = json!({
        "modalities": ["text", "audio"],
        "input_audio_format": config.input_format,
        "output_audio_format": config.output_format,
    });
    if let Some(voice) = config.voice.as_deref().filter(|v| !v.trim().is_empty()) {
        session["voice"] = json!(voice);
    }
    if let Some(text) = instructions.filter(|v| !v.trim().is_empty()) {
        session["instructions"] = json!(text);
    }
    Ok(json!({"type":"session.update", "session": session}))
}

pub fn build_audio_append(audio: &[u8]) -> Result<Value, LlmError> {
    if audio.is_empty() {
        return Err(LlmError::InvalidRequest(
            "realtime audio cannot be empty".into(),
        ));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(audio);
    Ok(json!({"type":"input_audio_buffer.append", "audio": encoded}))
}

pub fn build_response_create() -> Value {
    json!({"type":"response.create", "response": {"modalities":["audio","text"]}})
}

/// Execute one OpenAI Realtime turn over a websocket. The endpoint is
/// supplied by configuration so compatible providers can use the same
/// transport. Audio is returned as the concatenated `response.audio.delta`
/// payload; all other provider events are ignored by this low-level adapter.
pub async fn round_trip(
    api_key: &str,
    endpoint: &str,
    config: &RealtimeConfig,
    audio: &[u8],
    instructions: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, LlmError> {
    config.validate()?;
    if api_key.trim().is_empty() || endpoint.trim().is_empty() {
        return Err(LlmError::InvalidRequest(
            "realtime credentials and endpoint are required".into(),
        ));
    }
    if audio.is_empty() {
        return Err(LlmError::InvalidRequest(
            "realtime audio cannot be empty".into(),
        ));
    }
    let account = account_capacity_key(endpoint, api_key);
    let mut quota = crate::RateLimitGate::reserve_model(
        account.clone(),
        &config.model,
        (audio.len() as u64 / 3).max(1),
        cancel,
    )
    .await?;
    let _permit = match crate::RateLimitGate::admit_account(account.clone(), cancel).await {
        Ok(permit) => permit,
        Err(error) => {
            quota.release();
            return Err(error);
        }
    };
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    let url = format!(
        "{endpoint}{separator}model={}",
        percent_encoding::utf8_percent_encode(&config.model, percent_encoding::NON_ALPHANUMERIC)
    );
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("OpenAI-Beta", "realtime=v1")
        .body(())
        .map_err(|e| LlmError::InvalidRequest(e.to_string()))?;
    let capacity_ticket =
        crate::RateLimitGate::capacity_observation_ticket(account.clone(), &config.model);
    let (mut socket, handshake) = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        result = tokio_tungstenite::connect_async(request) => result.map_err(|e| LlmError::Network(e.to_string()))?,
    };
    capacity_ticket.observe_model(crate::openai::openai_capacity_observation(
        handshake.headers(),
    ));
    capacity_ticket.observe_account(crate::openai::openai_project_capacity_observation(
        handshake.headers(),
    ));
    let send = |value: Value| Message::Text(value.to_string());
    for value in [
        build_session_update(config, instructions)?,
        build_audio_append(audio)?,
        build_response_create(),
    ] {
        tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
            result = socket.send(send(value)) => result.map_err(|e| LlmError::Network(e.to_string()))?,
        }
    }
    const MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;
    let mut output = Vec::new();
    while let Some(message) = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted {
            // Realtime audio is returned as bytes, while the shared LLM
            // abort contract stores textual assistant messages. The caller
            // still owns the already-emitted audio buffer and can preserve it.
            partial: None,
        }),
        message = socket.next() => message,
    } {
        let message = message.map_err(|e| LlmError::Network(e.to_string()))?;
        let Message::Text(text) = message else {
            continue;
        };
        let event: Value =
            serde_json::from_str(&text).map_err(|e| LlmError::Parse(e.to_string()))?;
        match event.get("type").and_then(Value::as_str) {
            Some("response.audio.delta") => {
                let Some(delta) = event.get("delta").and_then(Value::as_str) else {
                    continue;
                };
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(delta)
                    .map_err(|e| LlmError::Parse(e.to_string()))?;
                if output.len().saturating_add(bytes.len()) > MAX_AUDIO_BYTES {
                    return Err(LlmError::InvalidRequest(
                        "provider audio exceeds 16 MiB".into(),
                    ));
                }
                output.extend(bytes);
            }
            Some("error") => {
                let message = event.to_string();
                if message.contains("rate_limit") || message.contains("429") {
                    crate::RateLimitGate::for_key(account.clone())
                        .observe_limit(None, std::time::Duration::from_secs(1));
                }
                return Err(LlmError::InvalidRequest(message));
            }
            Some("response.done") => break,
            _ => {}
        }
    }
    if output.is_empty() {
        return Err(LlmError::Parse(
            "provider returned empty realtime audio".into(),
        ));
    }
    quota.settle_estimate();
    Ok(output)
}

use base64::Engine;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn session_payload_requires_discovered_model_and_omits_voice_default() {
        let cfg = RealtimeConfig {
            model: "discovered-model".into(),
            voice: None,
            input_format: "pcm16".into(),
            output_format: "pcm16".into(),
        };
        let body = build_session_update(&cfg, Some("be concise")).unwrap();
        assert_eq!(body["type"], "session.update");
        assert!(body["session"].get("voice").is_none());
        assert_eq!(body["session"]["instructions"], "be concise");
    }
    #[test]
    fn audio_append_is_base64_and_rejects_empty() {
        let body = build_audio_append(&[1, 2, 3]).unwrap();
        assert_eq!(body["audio"], "AQID");
        assert!(build_audio_append(&[]).is_err());
    }
}
