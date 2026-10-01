//! The voice surface (docs/design/49-live-voice.md): `/voice/speak`,
//! `/voice/providers`, `WS /voice/session`, and the transcription of voice
//! notes the gateway admits from channels.
//!
//! Every route resolves its provider through [`vak_voice::VoiceProvider`] and
//! its credentials through the canonical secret chain, and every paid call
//! draws on one shared per-minute [`RequestWindow`]. Transcription has one
//! implementation, [`transcribe`], used by the socket and by channel voice
//! notes alike. A channel note is transcribed only after allowlist admission,
//! through its chat's bot → chat voice tiers ([`narrowed`]).

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

/// Persist a voice dispatch against the Agent conversation. The hosted
/// speech endpoints used here do not return rated usage, so FinOps records
/// the call as unknown spend instead of inventing token counts or a price.
fn record_voice_dispatch(
    state: &AppState,
    core: &vak_core::Core,
    session_id: &str,
    receipt: &vak_llm::WorkReceipt,
) {
    if let Some(handle) = state.get(session_id)
        && let Ok(mut session) = handle.session.lock()
        && let Some(session) = session.as_mut()
    {
        let _ = session.append_receipt(receipt.clone());
    }
    vak_core::routing::EvidenceLedger::new(&core.sessions_home())
        .record_receipts(std::slice::from_ref(receipt));
    if receipt.provider != "local" {
        let source = match receipt.purpose {
            vak_llm::WorkPurpose::SpeechRecognition => "voice_recognition_usage_unavailable",
            vak_llm::WorkPurpose::VoiceSynthesis => "voice_synthesis_usage_unavailable",
            _ => "voice_usage_unavailable",
        };
        let _ = vak_core::finops::FinOpsLedger::new(&core.shared_data_home()).append(
            &vak_core::finops::CostRow {
                ts: chrono::Utc::now(),
                model: receipt.model.clone(),
                provider: receipt.provider.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                usd: None,
                source: source.into(),
                session_id: session_id.to_string(),
                trace: None,
                actor: None,
            },
        );
    }
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

fn credential(core: &vak_core::Core, provider: VoiceProvider) -> Result<String, RouteError> {
    let provider_id = match provider {
        VoiceProvider::Gemini => "google",
        VoiceProvider::OpenAi => "openai",
        VoiceProvider::Local => return Err(RouteError::NoCredential(provider)),
    };
    core.provider_api_key(provider_id)
        .map(|key| key.trim().to_string())
        .map_err(|_| RouteError::NoCredential(provider))
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
    core: &vak_core::Core,
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
            let key = credential(core, VoiceProvider::OpenAi)?;
            vak_llm::openai::transcribe(&openai_config(key), audio, mime, &model, cancel)
                .await
                .map_err(RouteError::Provider)
        }
        VoiceProvider::Gemini => {
            let model = pinned_model(settings.transcription_model.as_ref(), "transcription")?;
            let key = credential(core, VoiceProvider::Gemini)?;
            let config = vak_llm::google_live::GoogleLiveConfig::new(key, model);
            vak_llm::google_live::transcribe(&config, audio, mime, cancel)
                .await
                .map_err(RouteError::Provider)
        }
    }
    .map(|text| text.trim().to_string())
}

#[derive(Deserialize)]
pub(crate) struct SpeakBody {
    text: String,
    /// `wav` (default), `pcm16`, `ogg_opus` or `mp3`, within what the
    /// provider can produce.
    #[serde(default)]
    format: Option<String>,
    /// A voice being auditioned (the admin console's Preview): the narrowest
    /// tier, above everything the conversation resolves to.
    #[serde(default)]
    voice_override: Option<vak_config::VoiceConfig>,
    /// The conversation the answer belongs to. A gateway chat's session
    /// brings that chat's bot → chat voice tiers; any session brings its
    /// Agent's voice style and personality.
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
    let voice = match resolve_speaker(&state, &body) {
        Ok(voice) => voice,
        Err(response) => return response,
    };
    let settings = voice.settings;
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
    let (voice_name, persona) = (voice.voice_name, voice.persona);
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
        &voice.core,
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
    if let Some(session_id) = body.session_id.as_deref() {
        record_voice_dispatch(&state, &voice.core, session_id, &receipt);
    }
    if let Ok(encoded) = serde_json::to_string(&receipt)
        && let Ok(value) = HeaderValue::try_from(encoded)
    {
        response.headers_mut().insert("x-vak-work-receipt", value);
    }
    response
}

#[allow(clippy::too_many_arguments)]
async fn synthesize(
    core: &vak_core::Core,
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
            let key = credential(core, provider)?;
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
            let key = credential(core, provider)?;
            let config =
                vak_llm::google_live::GoogleLiveConfig::new(key, model.unwrap_or_default());
            vak_llm::google_live::speak(&config, text, persona, voice_name, cancel)
                .await
                .map_err(RouteError::Provider)
        }
    }
}

/// Workspace voice settings narrowed by a voice tier: its provider and
/// model pins win where set, like every other route tier (invariant 23).
pub(crate) fn narrowed(
    mut settings: vak_config::VoiceSettings,
    tier: Option<&vak_config::VoiceConfig>,
) -> vak_config::VoiceSettings {
    let Some(tier) = tier else {
        return settings;
    };
    let pin = |value: &Option<String>| value.clone().filter(|v| !v.trim().is_empty());
    if let Some(provider) = pin(&tier.provider) {
        if settings.provider.as_deref() != Some(provider.as_str()) {
            // A model id belongs to one provider catalogue. A Channel that
            // changes provider must choose its own model pair instead of
            // accidentally inheriting ids from the Agent's provider.
            settings.transcription_model = None;
            settings.synthesis_model = None;
        }
        settings.provider = Some(provider);
    }
    if let Some(model) = pin(&tier.transcription_model) {
        settings.transcription_model = Some(model);
    }
    if let Some(model) = pin(&tier.synthesis_model) {
        settings.synthesis_model = Some(model);
    }
    settings
}

/// Whether a bot or chat voice tier can be stored: a pinned provider must be
/// one that exists, and every pin must be a plausible identifier. Checked at
/// write time so a typo is refused where it is made, not discovered when a
/// voice note fails.
pub(crate) fn check_tier(tier: &vak_config::VoiceConfig) -> Result<(), String> {
    if let Some(provider) = tier.provider.as_deref().filter(|p| !p.trim().is_empty()) {
        provider.trim().parse::<VoiceProvider>()?;
    }
    for (name, value) in [
        ("voice_name", &tier.voice_name),
        ("transcription_model", &tier.transcription_model),
        ("synthesis_model", &tier.synthesis_model),
    ] {
        if value.as_deref().is_some_and(|v| v.chars().count() > 256) {
            return Err(format!("{name} must be at most 256 characters"));
        }
    }
    Ok(())
}

/// The gateway chat a session is bound to, if any.
fn chat_for_session(state: &AppState, session_id: &str) -> Option<String> {
    state
        .gateway
        .bindings_snapshot()
        .into_iter()
        .find(|(_, binding)| binding.session_id.as_deref() == Some(session_id))
        .map(|(key, _)| key)
}

struct Speaker {
    core: vak_core::Core,
    settings: vak_config::VoiceSettings,
    voice_name: Option<String>,
    persona: Option<String>,
}

/// Resolve from the Agent Core attached to the live session, then apply
/// endpoint and audition overrides. The UI's currently selected Agent is
/// never used to route an already admitted conversation.
#[allow(clippy::result_large_err)]
fn resolve_speaker(state: &AppState, body: &SpeakBody) -> Result<Speaker, Response> {
    let chat = body
        .session_id
        .as_deref()
        .and_then(|id| chat_for_session(state, id));
    let core = if let Some(session_id) = body.session_id.as_deref() {
        crate::resolve_scoped_core(state, Some(session_id), None)?
    } else if let Some(key) = chat.as_deref() {
        state
            .gateway
            .core_for_entry(&state.core, key)
            .unwrap_or_else(|_| state.core.clone())
    } else {
        state.core.clone()
    };
    let chat_voice = chat
        .as_deref()
        .and_then(|key| state.gateway.resolve_voice(key));
    let settings = narrowed(
        narrowed(core.effective_voice(), chat_voice.as_ref()),
        body.voice_override.as_ref(),
    );
    let non_empty = |value: Option<String>| value.filter(|v| !v.trim().is_empty());
    let voice_name = non_empty(
        body.voice_override
            .as_ref()
            .and_then(|voice| voice.voice_name.clone()),
    )
    .or_else(|| {
        non_empty(
            chat_voice
                .as_ref()
                .and_then(|voice| voice.voice_name.clone()),
        )
    });
    // The persona comes from the bot/chat `identity` prompt block
    // (docs/design/45-prompt-layers.md); an explicit override still wins.
    let persona = non_empty(
        body.voice_override
            .as_ref()
            .and_then(|voice| voice.persona.clone()),
    )
    .or_else(|| {
        chat.as_deref()
            .and_then(|key| state.gateway.resolve_persona(key))
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
    Ok(Speaker {
        core,
        settings,
        voice_name,
        persona,
    })
}

/// What the model is told about one channel voice note.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VoiceNote {
    Heard(String),
    Unheard(String),
}

impl VoiceNote {
    /// The note's place in the user message: its words, or an honest account
    /// of why there are none.
    pub(crate) fn prompt_line(&self) -> String {
        match self {
            Self::Heard(text) => text.clone(),
            Self::Unheard(reason) => format!("[voice note not transcribed: {reason}]"),
        }
    }
}

/// Transcribe one voice note from an admitted gateway chat through that
/// chat's resolved route and the shared per-minute budget. Called only after
/// allowlist admission, so an unknown or pending chat never spends a provider
/// call.
pub(crate) async fn transcribe_voice_note(
    state: &AppState,
    core: &vak_core::Core,
    key: &str,
    mime: &str,
    data_base64: &str,
    rejected: Option<&str>,
) -> VoiceNote {
    use base64::Engine as _;
    if let Some(reason) = rejected {
        return VoiceNote::Unheard(reason.to_string());
    }
    let Ok(audio) = base64::engine::general_purpose::STANDARD.decode(data_base64.trim()) else {
        return VoiceNote::Unheard("the audio could not be decoded".into());
    };
    let settings = narrowed(
        core.effective_voice(),
        state.gateway.resolve_voice(key).as_ref(),
    );
    if audio.is_empty() || audio.len() as u64 > settings.max_audio_bytes {
        return VoiceNote::Unheard(format!(
            "the audio must be 1 to {} bytes",
            settings.max_audio_bytes
        ));
    }
    let provider = match route(&settings) {
        Ok(provider) => provider,
        Err(error) => return VoiceNote::Unheard(error.message()),
    };
    let model = match provider {
        VoiceProvider::Local => "local".to_string(),
        _ => match pinned_model(settings.transcription_model.as_ref(), "transcription") {
            Ok(model) => model,
            Err(error) => return VoiceNote::Unheard(error.message()),
        },
    };
    if !state.voice_requests.admit(settings.max_requests_per_minute) {
        return VoiceNote::Unheard("this minute's voice request budget is spent".into());
    }
    let started = Instant::now();
    let result = transcribe(core, &settings, &audio, mime, &CancellationToken::new()).await;
    let mut receipt = vak_llm::WorkReceipt::new(
        vak_llm::WorkPurpose::SpeechRecognition,
        provider.as_str(),
        &model,
    );
    let note = match result {
        Ok(text) if text.is_empty() => {
            receipt.record(
                vak_llm::AttemptReason::Initial,
                vak_llm::FailureDomain::Model,
                vak_llm::Settlement::Failed,
                started.elapsed().as_millis() as u64,
                None,
                Some("provider returned an empty transcript".into()),
            );
            VoiceNote::Unheard("no speech was recognized".into())
        }
        Ok(text) => {
            receipt.record(
                vak_llm::AttemptReason::Initial,
                vak_llm::FailureDomain::Unknown,
                vak_llm::Settlement::Ok,
                started.elapsed().as_millis() as u64,
                None,
                None,
            );
            VoiceNote::Heard(text)
        }
        Err(error) => {
            let (domain, settlement) = error.receipt_outcome();
            receipt.record(
                vak_llm::AttemptReason::Initial,
                domain,
                settlement,
                started.elapsed().as_millis() as u64,
                None,
                Some(error.message()),
            );
            VoiceNote::Unheard(error.message())
        }
    };
    let session_id = state
        .gateway
        .bindings_snapshot()
        .into_iter()
        .find(|(binding_key, _)| binding_key == key)
        .and_then(|(_, binding)| binding.session_id)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    record_voice_dispatch(state, core, &session_id, &receipt);
    note
}

/// `GET /voice/providers`: the static provider catalogue with credential or
/// engine readiness. Never includes credential values or model ids.
pub(crate) async fn voice_providers(
    State(state): State<AppState>,
    Query(query): Query<crate::AgentScopeQuery>,
) -> Response {
    let core = match crate::resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
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
                let provider_id = match provider {
                    VoiceProvider::Gemini => "google",
                    VoiceProvider::OpenAi => "openai",
                    VoiceProvider::Local => "",
                };
                let configured = core.provider_configured(provider_id);
                (configured, None)
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
    Json(serde_json::json!({ "providers": providers })).into_response()
}

/// Operation subsets derived from the live provider catalogue. Model ids
/// remain account-discovered; this only classifies the provider's declared
/// naming conventions for the two voice operations.
pub(crate) fn voice_model_capabilities(provider: &str, models: &[String]) -> serde_json::Value {
    let transcription: Vec<_> = models
        .iter()
        .filter(|model| {
            if provider == "openai" {
                let id = model.to_ascii_lowercase();
                id.contains("transcri") || id.contains("whisper")
            } else {
                !model.to_ascii_lowercase().contains("tts")
                    && !model.to_ascii_lowercase().contains("image")
                    && !model.to_ascii_lowercase().contains("embed")
            }
        })
        .cloned()
        .collect();
    let synthesis: Vec<_> = models
        .iter()
        .filter(|model| model.to_ascii_lowercase().contains("tts"))
        .cloned()
        .collect();
    serde_json::json!({ "transcription": transcription, "synthesis": synthesis })
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
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if !crate::origin_is_trusted(
        origin,
        host,
        state.core.config().server.public_url.as_deref(),
    ) {
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

async fn close_utterance(
    state: &AppState,
    core: &vak_core::Core,
    session_id: &str,
    pcm: &[u8],
) -> Outcome {
    // Measured here, not trusted from the client: steady room noise that a
    // client detector let through never reaches a provider.
    if !SpeechEvidence::measure(pcm, audio::SOCKET_SAMPLE_RATE_HZ).is_speech() {
        return Outcome::Discarded(DiscardReason::InsufficientSpeech);
    }
    // Re-read per utterance so a settings change applies without reconnecting.
    let settings = core.effective_voice();
    let provider = match route(&settings) {
        Ok(provider) => provider,
        Err(error) => return Outcome::Failed(error),
    };
    let model = match provider {
        VoiceProvider::Local => "local".to_string(),
        _ => match pinned_model(settings.transcription_model.as_ref(), "transcription") {
            Ok(model) => model,
            Err(error) => return Outcome::Failed(error),
        },
    };
    if !state.voice_requests.admit(settings.max_requests_per_minute) {
        return Outcome::Discarded(DiscardReason::RateLimited);
    }
    let wav = match audio::wrap_wav(pcm, audio::PcmSpec::SOCKET) {
        Ok(wav) => wav,
        Err(error) => return Outcome::Failed(RouteError::Local(error)),
    };
    let started = Instant::now();
    let result = transcribe(
        core,
        &settings,
        &wav,
        "audio/wav",
        &CancellationToken::new(),
    )
    .await;
    let mut receipt = vak_llm::WorkReceipt::new(
        vak_llm::WorkPurpose::SpeechRecognition,
        provider.as_str(),
        &model,
    );
    match result {
        Ok(text) if text.is_empty() => {
            receipt.record(
                vak_llm::AttemptReason::Initial,
                vak_llm::FailureDomain::Model,
                vak_llm::Settlement::Failed,
                started.elapsed().as_millis() as u64,
                None,
                Some("provider returned an empty transcript".into()),
            );
            record_voice_dispatch(state, core, session_id, &receipt);
            Outcome::Discarded(DiscardReason::EmptyTranscript)
        }
        Ok(text) => {
            receipt.record(
                vak_llm::AttemptReason::Initial,
                vak_llm::FailureDomain::Unknown,
                vak_llm::Settlement::Ok,
                started.elapsed().as_millis() as u64,
                None,
                None,
            );
            record_voice_dispatch(state, core, session_id, &receipt);
            Outcome::Transcript(text)
        }
        Err(error) => {
            let (domain, settlement) = error.receipt_outcome();
            receipt.record(
                vak_llm::AttemptReason::Initial,
                domain,
                settlement,
                started.elapsed().as_millis() as u64,
                None,
                Some(error.message()),
            );
            record_voice_dispatch(state, core, session_id, &receipt);
            Outcome::Failed(error)
        }
    }
}

async fn drive(mut socket: WebSocket, state: AppState, session_id: String) {
    let Some(handle) = state.get(&session_id) else {
        send_error(
            &mut socket,
            "Voice conversation is unavailable",
            "Reopen the Agent conversation and try again",
        )
        .await;
        return;
    };
    let core = handle.core.clone();
    let settings = core.effective_voice();
    // Refuse before the client asks for the microphone when no utterance
    // could be served: disabled, or no usable provider route.
    if let Err(error) = route(&settings) {
        send_error(&mut socket, error.message(), "Open Voice settings").await;
        return;
    }
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
                        let frame = match close_utterance(&state, &core, &session_id, &pcm).await {
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
                                        let _ = completed.send((answered, reply.text));
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
    fn a_channel_provider_change_does_not_inherit_foreign_model_ids() {
        let workspace = vak_config::VoiceSettings {
            enabled: true,
            provider: Some("gemini".into()),
            transcription_model: Some("stt-workspace".into()),
            synthesis_model: Some("tts-workspace".into()),
            ..Default::default()
        };
        let chat = vak_config::VoiceConfig {
            provider: Some("openai".into()),
            synthesis_model: Some("tts-chat".into()),
            transcription_model: Some("  ".into()),
            ..Default::default()
        };
        let effective = narrowed(workspace.clone(), Some(&chat));
        assert_eq!(effective.provider.as_deref(), Some("openai"));
        assert_eq!(effective.synthesis_model.as_deref(), Some("tts-chat"));
        assert_eq!(effective.transcription_model, None);
        assert!(effective.enabled, "a tier never changes the enable switch");
        assert_eq!(narrowed(workspace.clone(), None), workspace);

        let same_provider = vak_config::VoiceConfig {
            provider: Some("gemini".into()),
            ..Default::default()
        };
        let inherited = narrowed(workspace, Some(&same_provider));
        assert_eq!(
            inherited.transcription_model.as_deref(),
            Some("stt-workspace")
        );
        assert_eq!(inherited.synthesis_model.as_deref(), Some("tts-workspace"));
    }

    #[test]
    fn a_voice_tier_with_an_unknown_provider_is_refused_at_write() {
        let mut tier = vak_config::VoiceConfig {
            provider: Some("google".into()),
            ..Default::default()
        };
        assert!(
            check_tier(&tier)
                .unwrap_err()
                .contains("unknown voice provider")
        );
        tier.provider = Some("openai".into());
        assert!(check_tier(&tier).is_ok());
        tier.provider = None;
        tier.synthesis_model = Some("x".repeat(257));
        assert!(check_tier(&tier).is_err());
        assert!(check_tier(&vak_config::VoiceConfig::default()).is_ok());
    }

    #[test]
    fn a_voice_note_tells_the_model_the_truth() {
        assert_eq!(VoiceNote::Heard("hi".into()).prompt_line(), "hi");
        assert_eq!(
            VoiceNote::Unheard("no speech was recognized".into()).prompt_line(),
            "[voice note not transcribed: no speech was recognized]"
        );
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
