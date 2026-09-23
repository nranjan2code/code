//! The voice surface (docs/design/49-live-voice.md): `/voice/transcribe`,
//! `/voice/speak`, `/voice/providers` and `WS /voice/session`.
//!
//! Every route resolves its provider through [`vak_voice::VoiceProvider`] and
//! its credentials through the canonical secret chain, and every paid call
//! draws on one shared per-minute [`RequestWindow`]. Transcription has one
//! implementation, [`transcribe`], used by the channel bridges' batch route
//! and by the socket alike, so the two cannot drift in which providers,
//! models or encodings they accept.

use crate::AppState;
use axum::{
    Json,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures::SinkExt;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use vak_voice::protocol::{ClientControl, DiscardReason, ServerControl, VOICE_PROTOCOL_VERSION};
use vak_voice::{SpeakFormat, VoiceError, VoiceProvider, audio, vad::SpeechEvidence};

/// Rolling one-minute budget of paid voice requests for this process.
pub(crate) struct RequestWindow {
    window: Mutex<(Instant, usize)>,
}

impl RequestWindow {
    pub(crate) fn new() -> Self {
        Self {
            window: Mutex::new((Instant::now(), 0)),
        }
    }

    pub(crate) fn admit(&self, limit: usize) -> bool {
        self.admit_at(limit, Instant::now())
    }

    fn admit_at(&self, limit: usize, now: Instant) -> bool {
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if now.saturating_duration_since(window.0) >= Duration::from_secs(60) {
            *window = (now, 0);
        }
        if window.1 >= limit {
            return false;
        }
        window.1 += 1;
        true
    }
}

/// Why a voice route could not serve a request.
#[derive(Debug)]
pub(crate) enum RouteError {
    Disabled,
    Config(String),
    NoCredential(VoiceProvider),
    Provider(vak_llm::LlmError),
    Local(VoiceError),
}

impl RouteError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Disabled => StatusCode::CONFLICT,
            Self::Config(_) => StatusCode::BAD_REQUEST,
            Self::NoCredential(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Provider(error) => match error {
                vak_llm::LlmError::Auth(_) | vak_llm::LlmError::InvalidRequest(_) => {
                    StatusCode::BAD_REQUEST
                }
                vak_llm::LlmError::RateLimit { .. } => StatusCode::TOO_MANY_REQUESTS,
                vak_llm::LlmError::Overloaded(_) => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::BAD_GATEWAY,
            },
            Self::Local(VoiceError::InvalidRequest(_)) => StatusCode::BAD_REQUEST,
            Self::Local(VoiceError::Unavailable(_)) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Local(_) => StatusCode::BAD_GATEWAY,
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Disabled => "Voice is disabled in workspace settings".into(),
            Self::Config(message) => message.clone(),
            Self::NoCredential(provider) => format!(
                "No credential is configured for the {provider} voice provider ({})",
                provider.credential_vars().join(" or ")
            ),
            Self::Provider(error) => error.to_string(),
            Self::Local(error) => error.to_string(),
        }
    }

    fn receipt_outcome(&self) -> (vak_llm::FailureDomain, vak_llm::Settlement) {
        match self {
            Self::Provider(error) => vak_llm::work::classify_error(error),
            Self::Local(VoiceError::Cancelled) => (
                vak_llm::FailureDomain::Unknown,
                vak_llm::Settlement::Cancelled,
            ),
            _ => (vak_llm::FailureDomain::Request, vak_llm::Settlement::Failed),
        }
    }
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(serde_json::json!({ "error": self.message() })),
        )
            .into_response()
    }
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn route(settings: &vak_config::VoiceSettings) -> Result<VoiceProvider, RouteError> {
    if !settings.enabled {
        return Err(RouteError::Disabled);
    }
    VoiceProvider::resolve(settings.provider.as_deref()).map_err(RouteError::Config)
}

/// The operator-installed executable for a local engine, if configured.
fn local_engine(var: &str) -> Option<std::path::PathBuf> {
    vak_config::get_var(var)
        .filter(|path| !path.trim().is_empty())
        .map(std::path::PathBuf::from)
}

fn credential(provider: VoiceProvider) -> Result<String, RouteError> {
    provider
        .credential_vars()
        .iter()
        .find_map(|name| vak_config::get_var(name).filter(|value| !value.trim().is_empty()))
        .map(|value| value.trim().to_string())
        .ok_or(RouteError::NoCredential(provider))
}

/// A hosted provider's model for one operation, from the workspace pin.
/// Model ids come from discovery and configuration, never from source
/// (invariant 9), so a missing pin is a configuration gap.
fn pinned_model(pin: Option<&String>, operation: &str) -> Result<String, RouteError> {
    pin.map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            RouteError::Config(format!(
                "Voice {operation} needs a model; choose one in Voice settings"
            ))
        })
}

fn openai_config(api_key: String) -> vak_llm::openai::OpenAiConfig {
    vak_llm::openai::OpenAiConfig {
        api_key,
        base_url: vak_llm::openai::OPENAI_DEFAULT_BASE_URL.into(),
        cache_key: false,
        openrouter: false,
    }
}

/// Transcribe encoded audio through the effective voice route. The one
/// transcription implementation for every surface.
pub(crate) async fn transcribe(
    settings: &vak_config::VoiceSettings,
    audio: &[u8],
    mime: &str,
    cancel: &CancellationToken,
) -> Result<String, RouteError> {
    match route(settings)? {
        VoiceProvider::Local => {
            vak_voice::LocalTranscriber::new(local_engine(vak_voice::TRANSCRIBER_VAR))
                .transcribe(audio, cancel)
                .await
                .map_err(RouteError::Local)
        }
        VoiceProvider::OpenAi => {
            let model = pinned_model(settings.transcription_model.as_ref(), "transcription")?;
            let key = credential(VoiceProvider::OpenAi)?;
            vak_llm::openai::transcribe(&openai_config(key), audio, mime, &model, cancel)
                .await
                .map_err(RouteError::Provider)
        }
        VoiceProvider::Gemini => {
            let model = pinned_model(settings.transcription_model.as_ref(), "transcription")?;
            let key = credential(VoiceProvider::Gemini)?;
            let config = vak_llm::google_live::GoogleLiveConfig::new(key, model);
            vak_llm::google_live::transcribe(&config, audio, mime, cancel)
                .await
                .map_err(RouteError::Provider)
        }
    }
    .map(|text| text.trim().to_string())
}

#[derive(Deserialize)]
pub(crate) struct TranscribeBody {
    audio_base64: String,
    mime: String,
}

/// `POST /voice/transcribe`: bounded batch transcription for the channel
/// bridges. It only transcribes; the bridge's gateway inbound records the
/// resulting turn.
pub(crate) async fn voice_transcribe(
    State(state): State<AppState>,
    Json(body): Json<TranscribeBody>,
) -> Response {
    use base64::Engine as _;
    let settings = state.core.effective_voice();
    if !body.mime.starts_with("audio/") {
        return error_response(StatusCode::BAD_REQUEST, "mime must be an audio type");
    }
    let limit = settings.max_audio_bytes;
    let audio = match base64::engine::general_purpose::STANDARD.decode(body.audio_base64.trim()) {
        Ok(bytes) if !bytes.is_empty() && bytes.len() as u64 <= limit => bytes,
        Ok(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("audio must be 1 byte to {limit} bytes"),
            );
        }
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "audio_base64 is invalid"),
    };
    if let Err(error) = route(&settings) {
        return error.into_response();
    }
    // Only a valid request for an enabled route spends the rolling quota.
    if !state.voice_requests.admit(settings.max_requests_per_minute) {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "voice request rate limit exceeded",
        );
    }
    match transcribe(&settings, &audio, &body.mime, &CancellationToken::new()).await {
        Ok(text) if text.is_empty() => {
            error_response(StatusCode::UNPROCESSABLE_ENTITY, "No speech was recognized")
        }
        Ok(text) => Json(serde_json::json!({ "text": text })).into_response(),
        Err(error) => error.into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct SpeakBody {
    text: String,
    /// `wav` (default), `pcm16`, `ogg_opus` or `mp3`, within what the
    /// provider can produce.
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    bot_id: Option<String>,
    #[serde(default)]
    chat_key: Option<String>,
    #[serde(default)]
    voice_override: Option<vak_config::VoiceConfig>,
    /// The conversation the answer belongs to; its Agent's voice style and
    /// personality shape the persona when nothing narrower does.
    #[serde(default)]
    session_id: Option<String>,
}

/// `POST /voice/speak`: synthesize `text` through the effective voice route
/// and return the encoded audio, with a `x-vak-work-receipt` header recording
/// the dispatch whatever its outcome.
pub(crate) async fn voice_speak(
    State(state): State<AppState>,
    Json(body): Json<SpeakBody>,
) -> Response {
    if body.text.trim().is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "text must not be empty");
    }
    let settings = state.core.effective_voice();
    let provider = match route(&settings) {
        Ok(provider) => provider,
        Err(error) => return error.into_response(),
    };
    if body.text.chars().count() > settings.max_text_chars {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "text exceeds voice.max_text_chars ({})",
                settings.max_text_chars
            ),
        );
    }
    let requested = body.format.as_deref().unwrap_or("wav");
    let Some(format) =
        SpeakFormat::parse(requested).filter(|format| provider.speak_formats().contains(format))
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("the {provider} voice provider cannot produce '{requested}' audio"),
        );
    };
    if !state.voice_requests.admit(settings.max_requests_per_minute) {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "voice request rate limit exceeded",
        );
    }
    let (voice_name, persona) = resolve_voice(&state, &body);
    let model = match provider {
        VoiceProvider::Local => None,
        _ => match pinned_model(settings.synthesis_model.as_ref(), "synthesis") {
            Ok(model) => Some(model),
            Err(error) => return error.into_response(),
        },
    };
    let mut receipt = vak_llm::WorkReceipt::new(
        vak_llm::WorkPurpose::VoiceSynthesis,
        provider.as_str(),
        model.as_deref().unwrap_or("local"),
    );
    let cancel = CancellationToken::new();
    let started = Instant::now();
    let result = synthesize(
        provider,
        model.as_deref(),
        &body.text,
        format,
        voice_name.as_deref(),
        persona.as_deref(),
        &cancel,
    )
    .await;
    let latency_ms = started.elapsed().as_millis() as u64;
    let mut response = match result {
        Ok(audio) => {
            receipt.record(
                vak_llm::AttemptReason::Initial,
                vak_llm::FailureDomain::Unknown,
                vak_llm::Settlement::Ok,
                latency_ms,
                None,
                None,
            );
            ([(header::CONTENT_TYPE, format.mime())], audio).into_response()
        }
        Err(error) => {
            let (domain, settlement) = error.receipt_outcome();
            receipt.record(
                vak_llm::AttemptReason::Initial,
                domain,
                settlement,
                latency_ms,
                None,
                Some(error.message()),
            );
            error.into_response()
        }
    };
    if let Ok(encoded) = serde_json::to_string(&receipt)
        && let Ok(value) = HeaderValue::try_from(encoded)
    {
        response.headers_mut().insert("x-vak-work-receipt", value);
    }
    response
}

async fn synthesize(
    provider: VoiceProvider,
    model: Option<&str>,
    text: &str,
    format: SpeakFormat,
    voice_name: Option<&str>,
    persona: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, RouteError> {
    match provider {
        VoiceProvider::Local => vak_voice::LocalTtsSpeaker::new(local_engine(vak_voice::TTS_VAR))
            .speak(text, format, cancel)
            .await
            .map_err(RouteError::Local),
        VoiceProvider::OpenAi => {
            let key = credential(provider)?;
            let response_format = match format {
                SpeakFormat::Wav => "wav",
                SpeakFormat::Pcm16 => "pcm",
                SpeakFormat::OggOpus => "opus",
                SpeakFormat::Mp3 => "mp3",
            };
            vak_llm::openai::speak(
                &openai_config(key),
                text,
                model.unwrap_or_default(),
                voice_name.or(Some("alloy")),
                response_format,
                cancel,
            )
            .await
            .map_err(RouteError::Provider)
        }
        VoiceProvider::Gemini => {
            let key = credential(provider)?;
            let config =
                vak_llm::google_live::GoogleLiveConfig::new(key, model.unwrap_or_default());
            vak_llm::google_live::speak(&config, text, persona, voice_name, cancel)
                .await
                .map_err(RouteError::Provider)
        }
    }
}

/// Voice name and persona for a synthesis request. Resolution order: the
/// caller's explicit override (the admin console auditioning a value) >
/// the chat's or bot's resolved voice > the conversation's Agent identity.
fn resolve_voice(state: &AppState, body: &SpeakBody) -> (Option<String>, Option<String>) {
    let resolved = body.voice_override.clone().or_else(|| {
        if let Some(chat_key) = body.chat_key.as_deref() {
            state.gateway.resolve_voice(chat_key)
        } else {
            body.bot_id
                .as_deref()
                .and_then(|bot_id| state.gateway.bot_get(bot_id).and_then(|bot| bot.voice))
        }
    });
    let voice_name = resolved
        .as_ref()
        .and_then(|voice| voice.voice_name.clone())
        .filter(|name| !name.trim().is_empty());
    // The persona comes from the bot/chat `identity` prompt block
    // (docs/design/45-prompt-layers.md); an explicit override still wins.
    let persona = body
        .voice_override
        .as_ref()
        .and_then(|voice| voice.persona.clone())
        .filter(|persona| !persona.trim().is_empty())
        .or_else(|| {
            if let Some(chat_key) = body.chat_key.as_deref() {
                state.gateway.resolve_persona(chat_key)
            } else {
                body.bot_id
                    .as_deref()
                    .and_then(|bot_id| state.gateway.resolve_bot_persona(bot_id))
            }
        })
        .or_else(|| {
            let header = crate::read_historical_header(state, body.session_id.as_deref()?, None)?;
            let agent = header.agent?;
            let style = match agent.voice.as_str() {
                "calm" => "Speak calmly, warmly, and at an unhurried pace.",
                "bright" => "Speak with clear, friendly energy.",
                "quiet" => "Speak gently, evenly, and without theatrical emphasis.",
                _ => "Speak naturally and clearly.",
            };
            Some(if agent.personality.trim().is_empty() {
                style.to_string()
            } else {
                format!("{style} {}", agent.personality)
            })
        });
    (voice_name, persona)
}

/// `GET /voice/providers`: the static provider catalogue with credential or
/// engine readiness. Never includes credential values or model ids.
pub(crate) async fn voice_providers() -> Json<serde_json::Value> {
    let providers: Vec<_> = VoiceProvider::ALL
        .into_iter()
        .zip(vak_voice::catalogue())
        .map(|(provider, descriptor)| {
            let (configured, readiness) = if provider == VoiceProvider::Local {
                let listen = vak_voice::engine_readiness(
                    local_engine(vak_voice::TRANSCRIBER_VAR).as_deref(),
                );
                let speak =
                    vak_voice::engine_readiness(local_engine(vak_voice::TTS_VAR).as_deref());
                // Listening is what a conversation needs; without speech the
                // answer stays text.
                let readiness = serde_json::json!({
                    "ready": listen.ready,
                    "detail": format!("transcriber {}; speech {}", listen.detail, speak.detail),
                });
                (listen.configured || speak.configured, Some(readiness))
            } else {
                (credential(provider).is_ok(), None)
            };
            serde_json::json!({
                "name": descriptor.name,
                "formats": descriptor.formats,
                "credential_vars": descriptor.credential_vars,
                "configured": configured,
                "readiness": readiness,
            })
        })
        .collect();
    Json(serde_json::json!({ "providers": providers }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionQuery {
    session_id: String,
}

/// `WS /voice/session`: a governed spoken conversation bound to one Agent
/// session. Final transcripts become ordinary turns on that session.
pub(crate) async fn voice_socket(
    State(state): State<AppState>,
    Query(query): Query<SessionQuery>,
    headers: header::HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    // An upgrade is a GET, so the router's mutation Origin check does not
    // cover it; a microphone relay that spends provider money must check.
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !crate::origin_is_trusted(origin, &state.core.config().server.trusted_hosts) {
        return error_response(StatusCode::FORBIDDEN, "voice session origin is not trusted");
    }
    upgrade.on_upgrade(move |socket| drive(socket, state, query.session_id))
}

async fn send(socket: &mut WebSocket, control: &ServerControl) -> bool {
    match control.encode() {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn send_error(socket: &mut WebSocket, message: impl Into<String>, remedy: &str) -> bool {
    send(
        socket,
        &ServerControl::Error {
            message: message.into(),
            remedy: Some(remedy.into()),
        },
    )
    .await
}

struct Lease(Arc<AtomicUsize>);

impl Lease {
    fn acquire(active: &Arc<AtomicUsize>, limit: usize) -> Option<Self> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < limit).then_some(current + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(active)))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Bound on a spoken answer's text, keeping its frame far below the
/// protocol's control-frame limit whatever the configured synthesis cap.
const MAX_REPLY_CHARS: usize = 8_000;

/// Protocol state of one socket: at most one open utterance, and an
/// utterance id is never reused once it has been closed.
#[derive(Default)]
struct Utterances {
    open: Option<String>,
    audio: Vec<u8>,
    closed: HashSet<String>,
}

impl Utterances {
    fn start(&mut self, id: String) -> Result<(), &'static str> {
        if id.trim().is_empty() || self.closed.contains(&id) {
            return Err("utterance ids must be non-empty and never reused");
        }
        // A new start abandons an unfinished utterance: stopping voice
        // mid-sentence never submits it.
        self.audio.clear();
        self.open = Some(id);
        Ok(())
    }

    fn stop(&mut self, id: &str) -> Result<Vec<u8>, &'static str> {
        if self.open.as_deref() != Some(id) {
            return Err("speech_stopped does not name the open utterance");
        }
        self.open = None;
        self.closed.insert(id.to_string());
        Ok(std::mem::take(&mut self.audio))
    }

    /// Audio outside an open utterance is dropped, never buffered.
    fn push(&mut self, bytes: &[u8]) {
        if self.open.is_some() {
            self.audio.extend_from_slice(bytes);
        }
    }
}

/// What a closed utterance became.
enum Outcome {
    Discarded(DiscardReason),
    Transcript(String),
    Failed(RouteError),
}

async fn close_utterance(state: &AppState, pcm: &[u8]) -> Outcome {
    // Measured here, not trusted from the client: steady room noise that a
    // client detector let through never reaches a provider.
    if !SpeechEvidence::measure(pcm, audio::SOCKET_SAMPLE_RATE_HZ).is_speech() {
        return Outcome::Discarded(DiscardReason::InsufficientSpeech);
    }
    // Re-read per utterance so a settings change applies without reconnecting.
    let settings = state.core.effective_voice();
    if let Err(error) = route(&settings) {
        return Outcome::Failed(error);
    }
    if !state.voice_requests.admit(settings.max_requests_per_minute) {
        return Outcome::Discarded(DiscardReason::RateLimited);
    }
    let wav = match audio::wrap_wav(pcm, audio::PcmSpec::SOCKET) {
        Ok(wav) => wav,
        Err(error) => return Outcome::Failed(RouteError::Local(error)),
    };
    match transcribe(&settings, &wav, "audio/wav", &CancellationToken::new()).await {
        Ok(text) if text.is_empty() => Outcome::Discarded(DiscardReason::EmptyTranscript),
        Ok(text) => Outcome::Transcript(text),
        Err(error) => Outcome::Failed(error),
    }
}

async fn drive(mut socket: WebSocket, state: AppState, session_id: String) {
    let settings = state.core.effective_voice();
    // Refuse before the client asks for the microphone when no utterance
    // could be served: disabled, or no usable provider route.
    if let Err(error) = route(&settings) {
        send_error(&mut socket, error.message(), "Open Voice settings").await;
        return;
    }
    let handle = state
        .sessions
        .lock()
        .ok()
        .and_then(|sessions| sessions.get(&session_id).cloned());
    let Some(handle) = handle else {
        send_error(
            &mut socket,
            "Voice conversation is unavailable",
            "Reopen the Agent conversation and try again",
        )
        .await;
        return;
    };
    let Some(_lease) = Lease::acquire(&state.voice_active, settings.max_concurrent) else {
        send_error(
            &mut socket,
            "Voice session concurrency limit reached",
            "Wait for an active voice session to finish or increase the configured limit",
        )
        .await;
        return;
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(settings.max_session_secs);
    let ready = ServerControl::Ready {
        protocol_version: VOICE_PROTOCOL_VERSION,
        sample_rate_hz: audio::SOCKET_SAMPLE_RATE_HZ,
        channels: audio::PcmSpec::SOCKET.channels,
    };
    if !send(&mut socket, &ready).await {
        return;
    }
    let mut utterances = Utterances::default();
    let mut received_bytes: u64 = 0;
    let (completed_tx, mut completed_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, String)>();
    loop {
        let message = tokio::select! {
            inbound = futures::StreamExt::next(&mut socket) => match inbound {
                Some(Ok(message)) => message,
                _ => break,
            },
            Some((utterance_id, reply)) = completed_rx.recv() => {
                let text: String = reply.chars().take(MAX_REPLY_CHARS).collect();
                if !send(&mut socket, &ServerControl::TurnCompleted { utterance_id, text }).await {
                    break;
                }
                continue;
            }
            () = tokio::time::sleep_until(deadline) => {
                send_error(
                    &mut socket,
                    "This voice session reached its time limit",
                    "Press Voice to start a new session",
                )
                .await;
                break;
            }
        };
        match message {
            Message::Binary(bytes) => {
                received_bytes = received_bytes.saturating_add(bytes.len() as u64);
                if received_bytes > settings.max_audio_bytes {
                    send_error(
                        &mut socket,
                        format!(
                            "Voice audio budget exceeded ({} bytes)",
                            settings.max_audio_bytes
                        ),
                        "Start a new session or increase the configured budget",
                    )
                    .await;
                    break;
                }
                if vak_voice::protocol::validate_audio(&bytes).is_err() {
                    send_error(
                        &mut socket,
                        "Invalid PCM audio frame",
                        "Send mono 16-bit PCM frames",
                    )
                    .await;
                    break;
                }
                utterances.push(&bytes);
            }
            Message::Text(text) => {
                let control = match ClientControl::decode(text.as_str()) {
                    Ok(control) => control,
                    Err(error) => {
                        send_error(&mut socket, error.to_string(), "Update the voice client").await;
                        break;
                    }
                };
                match control {
                    ClientControl::SpeechStarted { utterance_id } => {
                        if let Err(reason) = utterances.start(utterance_id) {
                            send_error(&mut socket, reason, "Update the voice client").await;
                            break;
                        }
                    }
                    ClientControl::SpeechStopped { utterance_id } => {
                        let pcm = match utterances.stop(&utterance_id) {
                            Ok(pcm) => pcm,
                            Err(reason) => {
                                send_error(&mut socket, reason, "Update the voice client").await;
                                break;
                            }
                        };
                        let frame = match close_utterance(&state, &pcm).await {
                            Outcome::Discarded(reason) => ServerControl::Discarded {
                                utterance_id,
                                reason,
                            },
                            Outcome::Failed(error) => ServerControl::Error {
                                message: format!("Voice transcription failed: {}", error.message()),
                                remedy: Some(
                                    "Check Voice settings or use the text composer".into(),
                                ),
                            },
                            Outcome::Transcript(text) => {
                                if let Ok(mut log) = handle.session.lock()
                                    && let Some(log) = log.as_mut()
                                {
                                    let _ = log.append_voice_transcript(
                                        format!("voice:{utterance_id}"),
                                        text.clone(),
                                        true,
                                    );
                                }
                                // A final transcript is an ordinary user turn
                                // through the same governed runner as typed
                                // input: permissions, intent, budgets and
                                // receipts are one contract.
                                let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                crate::gateway::start_turn_chain_with_gateway(
                                    state.gateway.clone(),
                                    &handle.core,
                                    handle.clone(),
                                    crate::gateway::compose_voice_prompt(&text),
                                    Some(reply_tx),
                                );
                                let completed = completed_tx.clone();
                                let answered = utterance_id.clone();
                                tokio::spawn(async move {
                                    if let Ok(reply) = reply_rx.await {
                                        let _ = completed.send((answered, reply));
                                    }
                                });
                                ServerControl::Transcript { utterance_id, text }
                            }
                        };
                        let failed = matches!(frame, ServerControl::Error { .. });
                        if !send(&mut socket, &frame).await || failed {
                            break;
                        }
                    }
                    ClientControl::Playback {
                        utterance_id,
                        emitted_ms,
                        interrupted,
                    } => {
                        if let Ok(mut log) = handle.session.lock()
                            && let Some(log) = log.as_mut()
                        {
                            let _ = log.append_voice_playback(
                                format!("voice:{utterance_id}"),
                                emitted_ms,
                                interrupted,
                            );
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    let _ = socket.close().await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn request_window_enforces_its_limit_and_rolls_over() {
        let window = RequestWindow::new();
        let start = Instant::now();
        assert!(window.admit_at(2, start));
        assert!(window.admit_at(2, start));
        assert!(!window.admit_at(2, start));
        assert!(window.admit_at(2, start + Duration::from_secs(61)));
    }

    #[test]
    fn a_zero_limit_admits_nothing() {
        assert!(!RequestWindow::new().admit(0));
    }

    #[test]
    fn lease_admission_is_atomic_and_released_on_drop() {
        let active = Arc::new(AtomicUsize::new(0));
        let first = Lease::acquire(&active, 1).expect("first lease");
        assert!(Lease::acquire(&active, 1).is_none());
        drop(first);
        assert!(Lease::acquire(&active, 1).is_some());
        assert_eq!(active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn utterance_boundaries_are_strict() {
        let mut utterances = Utterances::default();
        utterances.push(&[1, 0]);
        assert!(utterances.start("u1".into()).is_ok());
        utterances.push(&[2, 0]);
        assert!(utterances.stop("u2").is_err());
        assert_eq!(
            utterances.stop("u1").unwrap(),
            vec![2, 0],
            "audio before the start is dropped"
        );
        assert!(utterances.start("u1".into()).is_err());
        assert!(utterances.start(" ".into()).is_err());
    }

    #[test]
    fn a_restart_abandons_the_unfinished_utterance() {
        let mut utterances = Utterances::default();
        utterances.start("u1".into()).unwrap();
        utterances.push(&[9, 9]);
        utterances.start("u2".into()).unwrap();
        assert!(utterances.stop("u2").unwrap().is_empty());
        assert!(utterances.stop("u1").is_err());
    }

    #[test]
    fn an_unset_or_unknown_provider_is_a_configuration_error() {
        let mut settings = vak_config::VoiceSettings {
            enabled: true,
            ..Default::default()
        };
        assert!(matches!(route(&settings), Err(RouteError::Config(_))));
        settings.provider = Some("google".into());
        assert!(matches!(route(&settings), Err(RouteError::Config(_))));
        settings.provider = Some("openai".into());
        assert_eq!(route(&settings).unwrap(), VoiceProvider::OpenAi);
        settings.enabled = false;
        assert!(matches!(route(&settings), Err(RouteError::Disabled)));
    }

    #[test]
    fn hosted_routes_need_a_pinned_model() {
        assert!(pinned_model(None, "transcription").is_err());
        assert!(pinned_model(Some(&" ".to_string()), "transcription").is_err());
        assert_eq!(
            pinned_model(Some(&" model-x ".to_string()), "transcription").unwrap(),
            "model-x"
        );
    }
}
