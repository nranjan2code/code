//! Gemini Live API (`BidiGenerateContent`) session wrapper for
//! text-to-speech voice synthesis. Mirrors `google.rs`'s conventions
//! (config shape, `x-goog-api-key` auth, `map_status_error`-style error
//! mapping, `tokio::select!` cancellation) but speaks raw WebSocket frames
//! instead of SSE, since the Live API is bidirectional.
//!
//! Wire quirks handled here: the Live API takes a JSON `setup` message
//! first, then JSON `clientContent` turns, and streams back JSON
//! `serverContent` messages carrying base64 PCM audio in
//! `modelTurn.parts[].inlineData.data` until `turnComplete`. Audio comes
//! back as raw 24kHz/16-bit/mono PCM, which we wrap in a WAV container
//! before returning it — nothing downstream should have to know the wire
//! format.

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_util::sync::CancellationToken;

use crate::error::LlmError;

const LIVE_WS_HOST: &str = "generativelanguage.googleapis.com";
const LIVE_WS_PATH: &str =
    "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent";

/// Overall wall-clock budget for one `speak()` call — connect, setup
/// round-trip, and audio collection combined. Nothing upstream of this
/// module enforces a deadline (the `CancellationToken` passed in is only
/// ever caller-triggered, never time-based), so without this a stalled
/// socket — one that neither errors nor closes — would block the request
/// task forever. Generous enough for a one-sentence TTS turn over a slow
/// connection; short enough that a hung request can't tie up a task
/// indefinitely.
const LIVE_SESSION_TIMEOUT: Duration = Duration::from_secs(25);
/// Some native-audio model revisions emit the complete audio turn but omit
/// `turnComplete`. Once audio has arrived, a short quiet window is therefore
/// sufficient to finalize the response while still allowing trailing chunks.
const AUDIO_IDLE_TIMEOUT: Duration = Duration::from_secs(3);

/// Hard cap on synthesis input length. This is a paid, per-call API; an
/// unbounded `text` field lets any bearer-authenticated caller run up
/// billing (or hang the session far longer than `LIVE_SESSION_TIMEOUT`
/// allows for) with a single oversized request. Real callers — short bot
/// replies, one-line narration cues — sit nowhere near this.
pub const MAX_SPEAK_TEXT_CHARS: usize = 2_000;

/// Batch speech-to-text through Gemini's multimodal generateContent API.
/// The audio bytes are caller-bounded; the provider returns plain transcript
/// text so the voice session can feed it into the governed turn path.
pub async fn transcribe(
    config: &GoogleLiveConfig,
    audio: &[u8],
    mime: &str,
    cancel: &CancellationToken,
) -> Result<String, LlmError> {
    if audio.is_empty() || mime.trim().is_empty() {
        return Err(LlmError::InvalidRequest(
            "audio and mime are required".into(),
        ));
    }
    if cancel.is_cancelled() {
        return Err(LlmError::Aborted { partial: None });
    }
    if config.model.trim().is_empty() {
        return Err(LlmError::InvalidRequest(
            "a discovered Gemini model is required".into(),
        ));
    }
    let account = account_identity(config);
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
    let body = build_transcribe_request(audio, mime);
    let url = format!(
        "https://{LIVE_WS_HOST}/v1beta/models/{}:generateContent",
        config.model
    );
    let capacity_ticket =
        crate::RateLimitGate::capacity_observation_ticket(account.clone(), &config.model);
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        result = reqwest::Client::new().post(url).header("x-goog-api-key", &config.api_key).json(&body).send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
    };
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let value: Value = response
        .json()
        .await
        .map_err(|e| LlmError::Parse(e.to_string()))?;
    observe_aux_capacity(
        &capacity_ticket,
        &account,
        status,
        &headers,
        Some(&value.to_string()),
    );
    if status >= 400 {
        return Err(map_status_error_with_retry(
            status,
            &value.to_string(),
            crate::openai::retry_after_from_headers(&headers),
        ));
    }
    let text = value
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(LlmError::InvalidRequest(
            "provider returned an empty transcript".into(),
        ));
    }
    quota.settle_estimate();
    Ok(text)
}

fn build_transcribe_request(audio: &[u8], mime: &str) -> Value {
    use base64::Engine as _;
    json!({"contents":[{"parts":[
        {"inline_data":{"mime_type":mime,"data":base64::engine::general_purpose::STANDARD.encode(audio)}},
        {"text":"Transcribe this audio exactly. Return only the spoken words, without commentary."}
    ]}]})
}

/// Live API output is always 24kHz, 16-bit, mono PCM (scratchpad-validated
/// against the real API — see test_gemini_live.py's `SAMPLE_RATE_OUT`).
const OUTPUT_SAMPLE_RATE_HZ: u32 = 24_000;

#[derive(Debug, Clone)]
pub struct GoogleLiveConfig {
    pub api_key: String,
    pub project_id: Option<String>,
    pub model: String,
}

impl GoogleLiveConfig {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        GoogleLiveConfig {
            api_key: api_key.into(),
            project_id: None,
            model: model.into(),
        }
    }
}

fn account_identity(config: &GoogleLiveConfig) -> String {
    crate::google::google_account_key(
        "https://generativelanguage.googleapis.com/v1beta",
        &config.api_key,
        config.project_id.as_deref(),
    )
}

fn observe_aux_capacity(
    ticket: &crate::CapacityObservationTicket,
    account: &str,
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: Option<&str>,
) {
    ticket.observe_model(crate::google::google_capacity_observation(headers, body));
    if status == 429 || status == 503 {
        let delay = crate::openai::retry_after_from_headers(headers).map(Duration::from_secs);
        crate::RateLimitGate::for_key(account.to_string())
            .observe_limit(delay, Duration::from_secs(1));
    }
}

#[cfg(test)]
fn map_status_error(status: u16, body: &str) -> LlmError {
    map_status_error_with_retry(status, body, None)
}

fn map_status_error_with_retry(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(|m| m.as_str().map(String::from))
        })
        .unwrap_or_else(|| body.chars().take(500).collect());
    match status {
        401 | 403 => LlmError::Auth(message),
        400 | 404 | 413 | 422 => LlmError::InvalidRequest(message),
        429 => match crate::google::google_daily_quota_exhaustion(body) {
            Some(quota) => LlmError::QuotaExhausted(quota),
            None => LlmError::RateLimit {
                message,
                retry_after_secs: retry_after,
            },
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

/// Build the `setup` message's `generationConfig`/`speechConfig` payload.
/// Pure and unit-testable, mirroring `google::build_body`. `persona`
/// becomes the session's `systemInstruction`; `voice_name` selects the
/// provider-discovered prebuilt Live voice. When absent, the provider chooses
/// its configured default; the harness never invents a voice identifier.
pub fn build_live_config(persona: Option<&str>, voice_name: Option<&str>) -> Value {
    let mut generation_config = json!({"responseModalities": ["AUDIO"]});
    if let Some(voice) = voice_name.filter(|v| !v.trim().is_empty()) {
        generation_config["speechConfig"] = json!({"voiceConfig": {"prebuiltVoiceConfig": {
            "voiceName": voice,
        }}});
    }
    let mut setup = json!({ "generationConfig": generation_config });
    if let Some(persona) = persona.filter(|p| !p.trim().is_empty()) {
        setup["systemInstruction"] = json!({
            "parts": [{ "text": persona }]
        });
    }
    setup
}

fn build_setup_message(model: &str, persona: Option<&str>, voice_name: Option<&str>) -> Value {
    let mut config = build_live_config(persona, voice_name);
    config["model"] = json!(format!("models/{model}"));
    json!({ "setup": config })
}

fn build_client_content(text: &str) -> Value {
    json!({
        "clientContent": {
            "turns": [{ "role": "user", "parts": [{ "text": text }] }],
            "turnComplete": true,
        }
    })
}

/// Wrap raw 24kHz/16-bit/mono PCM samples in a minimal WAV container by
/// hand (no extra crate needed for a 44-byte canonical header).
fn wrap_wav(pcm: &[u8]) -> Vec<u8> {
    let channels: u16 = 1;
    let bits_per_sample: u16 = 16;
    let byte_rate = OUTPUT_SAMPLE_RATE_HZ * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_len = pcm.len() as u32;
    let riff_len = 36 + data_len;

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size (PCM)
    out.extend_from_slice(&1u16.to_le_bytes()); // audio format: PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&OUTPUT_SAMPLE_RATE_HZ.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

/// Open a Live session, send one text turn, collect the synthesized audio,
/// and return it as WAV bytes. Races cancellation the same way
/// `google::GoogleProvider::stream` does, and additionally enforces
/// `LIVE_SESSION_TIMEOUT` as a wall-clock backstop — see that constant's
/// doc comment for why a cancellation token alone isn't enough here.
pub async fn speak(
    config: &GoogleLiveConfig,
    text: &str,
    persona: Option<&str>,
    voice_name: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, LlmError> {
    if text.chars().count() > MAX_SPEAK_TEXT_CHARS {
        return Err(LlmError::InvalidRequest(format!(
            "text too long for voice synthesis: {} chars (max {MAX_SPEAK_TEXT_CHARS})",
            text.chars().count()
        )));
    }
    let account = account_identity(config);
    let mut quota = crate::RateLimitGate::reserve_model(
        account.clone(),
        &config.model,
        (text.chars().count() as u64 / 4).max(1),
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
    let result = if config.model.to_ascii_lowercase().contains("tts") {
        speak_batch(config, text, persona, voice_name, cancel).await
    } else {
        match tokio::time::timeout(
            LIVE_SESSION_TIMEOUT,
            speak_inner(config, text, persona, voice_name, cancel),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(LlmError::Network(format!(
                "live session timed out after {}s",
                LIVE_SESSION_TIMEOUT.as_secs()
            ))),
        }
    };
    if result.is_ok() {
        quota.settle_estimate();
    }
    result
}

/// Generate speech with a discovered Gemini TTS model through the Interactions
/// API. The Live socket path is reserved for models that actually support bidi
/// turns. TTS model ids come from account discovery; this branch tests the
/// advertised capability marker, never a baked-in model catalogue.
async fn speak_batch(
    config: &GoogleLiveConfig,
    text: &str,
    persona: Option<&str>,
    voice_name: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, LlmError> {
    use base64::Engine as _;
    let voice = voice_name
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("Kore");
    let body = build_tts_interaction_request(&config.model, text, persona, voice);
    let url = format!("https://{LIVE_WS_HOST}/v1beta/interactions");
    let account = account_identity(config);
    let capacity_ticket =
        crate::RateLimitGate::capacity_observation_ticket(account.clone(), &config.model);
    let response = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        result = reqwest::Client::new().post(url).header("x-goog-api-key", &config.api_key).json(&body).send() => result.map_err(|e| LlmError::Network(e.to_string()))?,
    };
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let value: Value = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        result = response.json() => result.map_err(|e| LlmError::Parse(e.to_string()))?,
    };
    observe_aux_capacity(
        &capacity_ticket,
        &account,
        status,
        &headers,
        Some(&value.to_string()),
    );
    if status >= 400 {
        return Err(map_status_error_with_retry(
            status,
            &value.to_string(),
            crate::openai::retry_after_from_headers(&headers),
        ));
    }
    let (encoded, mime) = interaction_audio(&value)
        .ok_or_else(|| LlmError::Parse("TTS response contained no audio".into()))?;
    let pcm = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .map_err(|e| LlmError::Parse(format!("invalid TTS audio: {e}")))?;
    if pcm.is_empty() {
        return Err(LlmError::Parse("TTS response contained empty audio".into()));
    }
    if mime
        .as_deref()
        .is_some_and(|mime| mime.to_ascii_lowercase().contains("wav"))
        || pcm.starts_with(b"RIFF")
    {
        Ok(pcm)
    } else {
        Ok(wrap_wav(&pcm))
    }
}

fn build_tts_interaction_request(
    model: &str,
    text: &str,
    persona: Option<&str>,
    voice: &str,
) -> Value {
    let mut annotations = Vec::new();
    if let Some(style) = persona.filter(|style| !style.trim().is_empty()) {
        annotations.push(json!({"type": "speech_metadata", "style": style}));
    }
    let mut content = json!({"type": "text", "text": text});
    if !annotations.is_empty() {
        content["annotations"] = Value::Array(annotations);
    }
    json!({
        "model": model,
        "input": [{"type": "user_input", "content": [content]}],
        "response_format": {"type": "audio"},
        "generation_config": {"speech_config": [{"voice": voice}]},
    })
}

fn interaction_audio(value: &Value) -> Option<(String, Option<String>)> {
    if let Some(data) = value.pointer("/output_audio/data").and_then(Value::as_str) {
        let mime = value
            .pointer("/output_audio/mime_type")
            .and_then(Value::as_str)
            .map(str::to_string);
        return Some((data.to_string(), mime));
    }
    let steps = value.get("steps")?.as_array()?;
    for step in steps.iter().rev() {
        if step.get("type").and_then(Value::as_str) != Some("model_output") {
            continue;
        }
        let content = step.get("content")?.as_array()?;
        for part in content.iter().rev() {
            if part.get("type").and_then(Value::as_str) == Some("audio") {
                let data = part.get("data")?.as_str()?.to_string();
                let mime = part
                    .get("mime_type")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                return Some((data, mime));
            }
        }
    }
    None
}

async fn speak_inner(
    config: &GoogleLiveConfig,
    text: &str,
    persona: Option<&str>,
    voice_name: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, LlmError> {
    let _ =
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider());
    let url = format!("wss://{LIVE_WS_HOST}{LIVE_WS_PATH}");
    let capacity_ticket =
        crate::RateLimitGate::capacity_observation_ticket(account_identity(config), &config.model);
    let request = Request::builder()
        .uri(url)
        .header("x-goog-api-key", &config.api_key)
        .body(())
        .map_err(|e| LlmError::InvalidRequest(e.to_string()))?;

    let connect_fut = tokio_tungstenite::connect_async(request);
    let (mut ws, response) = tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        r = connect_fut => match r {
            Ok(pair) => pair,
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
                let status = resp.status().as_u16();
                let body = resp
                    .body()
                    .as_ref()
                    .map(|b| String::from_utf8_lossy(b).to_string())
                    .unwrap_or_default();
                capacity_ticket.observe_model(crate::google::google_capacity_observation(
                    resp.headers(),
                    Some(&body),
                ));
                if status == 429 || status == 503 {
                    crate::RateLimitGate::observe_account_limit(
                        account_identity(config),
                        crate::google::google_retry_delay(&body).map(Duration::from_secs),
                        Duration::from_secs(1),
                    );
                }
                return Err(map_status_error_with_retry(
                    status,
                    &body,
                    crate::openai::retry_after_from_headers(resp.headers()),
                ));
            }
            Err(e) => return Err(LlmError::Network(e.to_string())),
        },
    };
    capacity_ticket.observe_model(crate::google::google_capacity_observation(
        response.headers(),
        None,
    ));

    let setup = build_setup_message(&config.model, persona, voice_name);
    let send_setup = ws.send(Message::Text(setup.to_string()));
    tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        r = send_setup => r.map_err(|e| LlmError::Network(e.to_string()))?,
    };

    // Wait for the server's `setupComplete` acknowledgement before sending
    // the turn — the Live API requires setup to round-trip first.
    loop {
        let next = ws.next();
        let msg = tokio::select! {
            _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
            m = next => m,
        };
        match msg {
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(&t)
                    .map_err(|e| LlmError::Parse(format!("bad setup response: {e}")))?;
                if v.get("setupComplete").is_some() {
                    break;
                }
                if let Some(err) = v.get("error") {
                    return Err(LlmError::Api {
                        status: 0,
                        message: err.to_string(),
                    });
                }
            }
            Some(Ok(Message::Binary(bytes))) => {
                let v: Value = serde_json::from_slice(&bytes)
                    .map_err(|e| LlmError::Parse(format!("bad setup response: {e}")))?;
                if v.get("setupComplete").is_some() {
                    break;
                }
                if let Some(err) = v.get("error") {
                    return Err(LlmError::Api {
                        status: 0,
                        message: err.to_string(),
                    });
                }
            }
            Some(Ok(Message::Close(frame))) => {
                let reason = frame.map(|f| f.reason.to_string()).unwrap_or_default();
                return Err(LlmError::Network(format!(
                    "live session closed during setup: {reason}"
                )));
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(LlmError::Network(e.to_string())),
            None => {
                return Err(LlmError::Network(
                    "live session closed before setupComplete".into(),
                ));
            }
        }
    }

    let turn = build_client_content(text);
    let send_turn = ws.send(Message::Text(turn.to_string()));
    tokio::select! {
        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
        r = send_turn => r.map_err(|e| LlmError::Network(e.to_string()))?,
    };

    let mut pcm = Vec::new();
    loop {
        let next = ws.next();
        let msg = tokio::select! {
            _ = cancel.cancelled() => {
                let partial = (!pcm.is_empty())
                    .then(|| Box::new(crate::types::AssistantMessage::empty(&config.model)));
                return Err(LlmError::Aborted { partial });
            }
            m = async {
                if pcm.is_empty() {
                    next.await
                } else {
                    tokio::time::timeout(AUDIO_IDLE_TIMEOUT, next)
                        .await
                        .unwrap_or_default()
                }
            } => m,
        };
        match msg {
            Some(Ok(Message::Text(t))) => {
                let v: Value = serde_json::from_str(&t)
                    .map_err(|e| LlmError::Parse(format!("bad server message: {e}")))?;
                if let Some(err) = v.get("error") {
                    return Err(LlmError::Api {
                        status: 0,
                        message: err.to_string(),
                    });
                }
                let Some(sc) = v.get("serverContent") else {
                    continue;
                };
                if let Some(parts) = sc.pointer("/modelTurn/parts").and_then(|p| p.as_array()) {
                    for part in parts {
                        if let Some(data) =
                            part.pointer("/inlineData/data").and_then(|d| d.as_str())
                        {
                            use base64::Engine as _;
                            match base64::engine::general_purpose::STANDARD.decode(data) {
                                Ok(bytes) => pcm.extend_from_slice(&bytes),
                                Err(e) => {
                                    return Err(LlmError::Parse(format!(
                                        "bad base64 audio chunk: {e}"
                                    )));
                                }
                            }
                        }
                    }
                }
                if sc
                    .get("turnComplete")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false)
                {
                    break;
                }
            }
            Some(Ok(Message::Binary(bytes))) => {
                let v: Value = serde_json::from_slice(&bytes)
                    .map_err(|e| LlmError::Parse(format!("bad server message: {e}")))?;
                if let Some(err) = v.get("error") {
                    return Err(LlmError::Api {
                        status: 0,
                        message: err.to_string(),
                    });
                }
                if let Some(sc) = v.get("serverContent") {
                    if let Some(parts) = sc.pointer("/modelTurn/parts").and_then(|p| p.as_array()) {
                        for part in parts {
                            if let Some(data) =
                                part.pointer("/inlineData/data").and_then(|d| d.as_str())
                            {
                                use base64::Engine as _;
                                let bytes = base64::engine::general_purpose::STANDARD
                                    .decode(data)
                                    .map_err(|e| {
                                        LlmError::Parse(format!("bad base64 audio chunk: {e}"))
                                    })?;
                                pcm.extend_from_slice(&bytes);
                            }
                        }
                    }
                    if sc
                        .get("turnComplete")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
            }
            Some(Ok(Message::Close(_))) => break,
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(LlmError::Network(e.to_string())),
            None => break,
        }
    }

    let _ = ws.close(None).await;

    if pcm.is_empty() {
        return Err(LlmError::Parse("no audio returned by live session".into()));
    }

    Ok(wrap_wav(&pcm))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn build_live_config_omits_unspecified_voice_without_persona() {
        let cfg = build_live_config(None, None);
        assert!(cfg.pointer("/generationConfig/speechConfig").is_none());
        assert_eq!(
            cfg.pointer("/generationConfig/responseModalities/0")
                .and_then(|v| v.as_str()),
            Some("AUDIO")
        );
        assert!(cfg.get("systemInstruction").is_none());
    }

    #[test]
    fn build_live_config_honors_voice_name_and_persona() {
        let cfg = build_live_config(Some("warm and upbeat"), Some("Puck"));
        assert_eq!(
            cfg.pointer("/generationConfig/speechConfig/voiceConfig/prebuiltVoiceConfig/voiceName")
                .and_then(|v| v.as_str()),
            Some("Puck")
        );
        assert_eq!(
            cfg.pointer("/systemInstruction/parts/0/text")
                .and_then(|v| v.as_str()),
            Some("warm and upbeat")
        );
    }

    #[test]
    fn build_live_config_ignores_blank_persona_and_voice() {
        let cfg = build_live_config(Some("   "), Some(""));
        assert!(cfg.get("systemInstruction").is_none());
        assert!(cfg.pointer("/generationConfig/speechConfig").is_none());
    }

    #[test]
    fn interactions_tts_request_keeps_transcript_verbatim_and_uses_style_metadata() {
        let body = build_tts_interaction_request(
            "models/gemini-account-tts-model",
            "Hello there.",
            Some("Speak gently"),
            "Kore",
        );
        assert_eq!(body["model"], "models/gemini-account-tts-model");
        assert_eq!(body["input"][0]["content"][0]["text"], "Hello there.");
        assert_eq!(
            body["input"][0]["content"][0]["annotations"][0]["style"],
            "Speak gently"
        );
        assert_eq!(body["response_format"]["type"], "audio");
        assert_eq!(
            body["generation_config"]["speech_config"][0]["voice"],
            "Kore"
        );
    }

    #[test]
    fn interaction_tts_audio_supports_rest_steps_and_sdk_shapes() {
        let rest = json!({"steps":[{"type":"model_output","content":[
            {"type":"text","text":"hello"},
            {"type":"audio","mime_type":"audio/wav","data":"d2F2"}
        ]}]});
        assert_eq!(
            interaction_audio(&rest),
            Some(("d2F2".into(), Some("audio/wav".into())))
        );
        let sdk = json!({"output_audio":{"mime_type":"audio/wav","data":"d2F2"}});
        assert_eq!(
            interaction_audio(&sdk),
            Some(("d2F2".into(), Some("audio/wav".into())))
        );
    }

    #[tokio::test]
    async fn speak_rejects_text_over_the_length_cap() {
        let config = GoogleLiveConfig::new("test-key-not-used", "discovered-model");
        let text: String = "a".repeat(MAX_SPEAK_TEXT_CHARS + 1);
        let cancel = CancellationToken::new();
        let err = speak(&config, &text, None, None, &cancel)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn transcribe_rejects_cancelled_request_without_network() {
        let config = GoogleLiveConfig::new("test-key-not-used", "discovered-model");
        let cancel = CancellationToken::new();
        cancel.cancel();
        let err = transcribe(&config, b"audio", "audio/pcm", &cancel)
            .await
            .unwrap_err();
        assert!(matches!(err, LlmError::Aborted { .. }));
    }

    #[test]
    fn provider_status_errors_preserve_classification_and_message() {
        assert!(
            matches!(map_status_error(401, r#"{"error":{"message":"bad key"}}"#), LlmError::Auth(message) if message == "bad key")
        );
        assert!(
            matches!(map_status_error(429, r#"{"error":{"message":"slow down"}}"#), LlmError::RateLimit { message, .. } if message == "slow down")
        );
        assert!(
            matches!(map_status_error(503, r#"{"error":{"message":"busy"}}"#), LlmError::Overloaded(message) if message == "busy")
        );

        let daily = r#"{"error":{"details":[{"@type":"type.googleapis.com/google.rpc.QuotaFailure","violations":[{"quotaId":"GenerateContentRequestsPerDayPerProjectPerModel"}]}]}}"#;
        assert!(matches!(
            map_status_error(429, daily),
            LlmError::QuotaExhausted(_)
        ));
    }

    #[test]
    fn transcription_request_contains_audio_and_strict_instruction() {
        let request = build_transcribe_request(&[0, 1, 2], "audio/pcm");
        assert_eq!(
            request
                .pointer("/contents/0/parts/0/inline_data/mime_type")
                .and_then(Value::as_str),
            Some("audio/pcm")
        );
        assert!(
            request
                .pointer("/contents/0/parts/0/inline_data/data")
                .and_then(Value::as_str)
                .is_some()
        );
        assert!(
            request
                .pointer("/contents/0/parts/1/text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("only the spoken words")
        );
    }

    /// Opt-in live smoke test. The credential is read only from the process
    /// environment and is never printed or persisted.
    #[tokio::test]
    #[ignore = "requires GEMINI_API_KEY and network access"]
    async fn live_smoke_synthesizes_audio() {
        let Ok(key) = smoke_key() else {
            return;
        };
        let Ok(model) = std::env::var("VAK_GEMINI_LIVE_MODEL") else {
            return;
        };
        let config = GoogleLiveConfig::new(key, model);
        let bytes = speak(
            &config,
            "Say hello briefly.",
            None,
            None,
            &CancellationToken::new(),
        )
        .await
        .expect("configured live provider should synthesize");
        assert!(bytes.starts_with(b"RIFF"));
        assert!(bytes.len() > 44);
    }

    #[tokio::test]
    #[ignore = "requires GEMINI_API_KEY and network access"]
    async fn live_smoke_transcribes_audio() {
        let Ok(key) = smoke_key() else {
            return;
        };
        let Ok(model) = std::env::var("VAK_GEMINI_TRANSCRIBE_MODEL") else {
            return;
        };
        let config = GoogleLiveConfig::new(key, model);
        let audio = vec![0u8; 3200];
        let text = transcribe(&config, &audio, "audio/pcm", &CancellationToken::new())
            .await
            .expect("configured live provider should transcribe");
        assert!(!text.trim().is_empty());
    }

    fn smoke_key() -> Result<String, std::env::VarError> {
        std::env::var("GEMINI_API_KEY").or_else(|_| std::env::var("GOOGLE_API_KEY"))
    }

    #[test]
    fn wrap_wav_produces_valid_riff_header() {
        let pcm = vec![0u8, 1, 2, 3, 4, 5, 6, 7];
        let wav = wrap_wav(&pcm);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(wav.len(), 44 + pcm.len());
    }
}
