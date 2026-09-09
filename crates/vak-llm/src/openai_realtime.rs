//! Provider-neutral helpers for the OpenAI Realtime wire protocol.
//!
//! The model and voice are deliberately supplied by discovery/configuration;
//! this module contains no catalogue or fallback identifiers.  Keeping the
//! wire messages here makes the streaming transport usable by HTTP, desktop,
//! and channel surfaces without duplicating protocol details.

use crate::error::LlmError;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeConfig {
    pub model: String,
    pub voice: Option<String>,
    pub input_format: String,
    pub output_format: String,
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

use base64::Engine;

#[cfg(test)]
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
