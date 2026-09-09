//! Provider-neutral voice contracts and deterministic audio primitives.
//! Provider clients live behind these traits so surfaces never own audio
//! transport, credentials, or conversational policy.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub mod audio;
pub mod conversation;
pub mod intent;
pub mod local;
pub mod protocol;
pub mod session;
pub mod transcribe;
pub mod vad;

pub use local::{
    LocalSpeaker, LocalTranscriber, LocalTtsReadiness, LocalTtsSpeaker, local_tts_readiness,
};

/// Runtime catalogue of voice capabilities. Descriptors are immutable
/// snapshots; provider implementations can refresh models and voices by
/// replacing their descriptor without changing surface code.
#[derive(Debug, Default)]
pub struct VoiceRegistry {
    descriptors: BTreeMap<String, VoiceDescriptor>,
}

impl VoiceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, descriptor: VoiceDescriptor) {
        self.descriptors.insert(descriptor.name.clone(), descriptor);
    }

    /// Replace only runtime-discovered models and voices while preserving
    /// the provider's static capability and credential metadata.
    pub fn update_discovery(
        &mut self,
        name: &str,
        models: Vec<String>,
        voices: Vec<String>,
    ) -> bool {
        let Some(descriptor) = self.descriptors.get_mut(name) else {
            return false;
        };
        descriptor.models = models;
        descriptor.voices = voices;
        descriptor.model_provenance = Some("provider_discovery".into());
        descriptor.voice_provenance = Some("provider_discovery".into());
        true
    }

    pub fn remove(&mut self, name: &str) -> Option<VoiceDescriptor> {
        self.descriptors.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&VoiceDescriptor> {
        self.descriptors.get(name)
    }

    pub fn list(&self) -> impl Iterator<Item = &VoiceDescriptor> {
        self.descriptors.values()
    }

    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
    }
}

/// Built-in capability descriptors. Models and voices are intentionally
/// empty: providers must populate them through discovery for the active key.
pub fn default_registry() -> VoiceRegistry {
    let mut registry = VoiceRegistry::new();
    for (name, endpointing, formats) in [
        (
            "gemini-live",
            Endpointing::Server,
            vec![SpeakFormat::Pcm16, SpeakFormat::Wav],
        ),
        (
            "openai-realtime",
            Endpointing::Server,
            vec![SpeakFormat::Pcm16, SpeakFormat::OggOpus, SpeakFormat::Mp3],
        ),
        (
            "local",
            Endpointing::Client,
            // LocalSpeaker emits canonical PCM/WAV only.  Container conversion
            // belongs to the channel adapter (for example Telegram's Ogg/Opus
            // requirement), so do not advertise a format it cannot produce.
            vec![SpeakFormat::Pcm16, SpeakFormat::Wav],
        ),
    ] {
        let env_var = match name {
            "gemini-live" => Some("GEMINI_API_KEY".into()),
            "openai-realtime" => Some("OPENAI_API_KEY".into()),
            _ => None,
        };
        let default_base_url = (name == "local").then(|| "http://127.0.0.1:8080/v1".into());
        registry.register(VoiceDescriptor {
            name: name.into(),
            env_var,
            default_base_url,
            endpointing,
            formats,
            input_formats: match name {
                "gemini-live" | "openai-realtime" => vec![
                    ListenFormat::Pcm16,
                    ListenFormat::Wav,
                    ListenFormat::OggOpus,
                ],
                _ => vec![
                    ListenFormat::Pcm16,
                    ListenFormat::Wav,
                    ListenFormat::OggOpus,
                ],
            },
            voices: Vec::new(),
            models: Vec::new(),
            model_provenance: None,
            voice_provenance: None,
        });
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_descriptor_matches_local_speaker_formats() {
        let registry = default_registry();
        let descriptor = registry.get("local").expect("local descriptor");
        let speaker = LocalSpeaker;
        assert!(
            descriptor
                .formats
                .iter()
                .all(|format| speaker.supports(*format))
        );
        assert!(!speaker.supports(SpeakFormat::OggOpus));
    }

    #[test]
    fn default_catalogue_is_capability_only() {
        let registry = default_registry();
        let names: Vec<_> = registry.list().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["gemini-live", "local", "openai-realtime"]);
        assert!(
            registry
                .list()
                .all(|d| d.models.is_empty() && d.voices.is_empty())
        );
        assert_eq!(
            registry
                .get("gemini-live")
                .and_then(|d| d.env_var.as_deref()),
            Some("GEMINI_API_KEY")
        );
        assert_eq!(
            registry
                .get("local")
                .and_then(|d| d.default_base_url.as_deref()),
            Some("http://127.0.0.1:8080/v1")
        );
    }

    #[test]
    fn registration_replaces_descriptor_atomically() {
        let mut registry = VoiceRegistry::new();
        registry.register(VoiceDescriptor {
            name: "local".into(),
            env_var: None,
            default_base_url: None,
            endpointing: Endpointing::Client,
            formats: vec![SpeakFormat::Wav],
            input_formats: vec![ListenFormat::Wav],
            voices: vec![],
            models: vec!["whisper".into()],
            model_provenance: None,
            voice_provenance: None,
        });
        registry.register(VoiceDescriptor {
            name: "local".into(),
            env_var: None,
            default_base_url: None,
            endpointing: Endpointing::Server,
            formats: vec![SpeakFormat::Pcm16],
            input_formats: vec![ListenFormat::Pcm16],
            voices: vec![],
            models: vec!["new".into()],
            model_provenance: None,
            voice_provenance: None,
        });
        assert_eq!(
            registry.get("local").map(|d| d.models.as_slice()),
            Some(["new".to_string()].as_slice())
        );
    }

    #[test]
    fn discovery_update_preserves_provider_metadata() {
        let mut registry = default_registry();
        assert!(registry.update_discovery(
            "gemini-live",
            vec!["live-model".into()],
            vec!["voice-a".into()]
        ));
        let descriptor = registry.get("gemini-live").unwrap();
        assert_eq!(descriptor.models, vec!["live-model"]);
        assert_eq!(descriptor.voices, vec!["voice-a"]);
        assert_eq!(descriptor.env_var.as_deref(), Some("GEMINI_API_KEY"));
        assert_eq!(
            descriptor.model_provenance.as_deref(),
            Some("provider_discovery")
        );
    }

    #[test]
    fn descriptor_serialization_is_safe_for_admin_surfaces() {
        let registry = default_registry();
        let descriptor = registry.get("gemini-live").unwrap();
        let value = serde_json::to_value(descriptor).unwrap();
        assert!(value.get("env_var").is_some());
        assert!(value.get("models").unwrap().as_array().unwrap().is_empty());
        assert!(value.to_string().find("KEY=").is_none());
    }
}

#[derive(Debug, Error)]
pub enum VoiceError {
    #[error("voice provider unavailable: {0}")]
    Unavailable(String),
    #[error("voice request rejected: {0}")]
    InvalidRequest(String),
    #[error("voice format unsupported: {0}")]
    UnsupportedFormat(String),
    #[error("voice operation cancelled")]
    Cancelled,
    #[error("voice transport failed: {0}")]
    Transport(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpeakFormat {
    Pcm16,
    OggOpus,
    Mp3,
    Wav,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenFormat {
    Pcm16,
    OggOpus,
    Mp3,
    Wav,
    WebmOpus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpointing {
    Server,
    Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceDescriptor {
    pub name: String,
    pub env_var: Option<String>,
    pub default_base_url: Option<String>,
    pub endpointing: Endpointing,
    pub formats: Vec<SpeakFormat>,
    #[serde(default)]
    pub input_formats: Vec<ListenFormat>,
    #[serde(default)]
    pub voices: Vec<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_provenance: Option<String>,
    #[serde(default)]
    pub voice_provenance: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ListenSpec {
    pub model: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpeakSpec {
    pub text: String,
    pub model: Option<String>,
    pub voice: Option<String>,
    pub format: SpeakFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioChunk {
    pub sequence: u64,
    pub data: Vec<u8>,
    pub duration_ms: u32,
}

#[derive(Debug, Default)]
pub struct AudioAssembler {
    next_sequence: u64,
    emitted_ms: u64,
    data: Vec<u8>,
    max_bytes: Option<usize>,
}

impl AudioAssembler {
    pub fn with_limit(max_bytes: usize) -> Result<Self, VoiceError> {
        if max_bytes == 0 {
            return Err(VoiceError::InvalidRequest(
                "audio assembler limit must be positive".into(),
            ));
        }
        Ok(Self {
            max_bytes: Some(max_bytes),
            ..Self::default()
        })
    }
    pub fn push(&mut self, chunk: AudioChunk) -> Result<(), VoiceError> {
        if chunk.sequence != self.next_sequence {
            return Err(VoiceError::Transport(format!(
                "unexpected audio chunk sequence {}; expected {}",
                chunk.sequence, self.next_sequence
            )));
        }
        if chunk.data.is_empty() || chunk.duration_ms == 0 {
            return Err(VoiceError::InvalidRequest("empty audio chunk".into()));
        }
        if self
            .max_bytes
            .is_some_and(|limit| self.data.len().saturating_add(chunk.data.len()) > limit)
        {
            return Err(VoiceError::InvalidRequest(
                "audio assembler byte budget exceeded".into(),
            ));
        }
        self.data.extend_from_slice(&chunk.data);
        self.emitted_ms = self.emitted_ms.saturating_add(u64::from(chunk.duration_ms));
        self.next_sequence += 1;
        Ok(())
    }
    pub fn emitted_ms(&self) -> u64 {
        self.emitted_ms
    }
    pub fn bytes_len(&self) -> usize {
        self.data.len()
    }
    pub fn remaining_bytes(&self) -> Option<usize> {
        self.max_bytes
            .map(|limit| limit.saturating_sub(self.data.len()))
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
    pub fn reset(&mut self) {
        self.next_sequence = 0;
        self.emitted_ms = 0;
        self.data.clear();
    }
}

#[cfg(test)]
mod assembler_tests {
    use super::*;
    #[test]
    fn assembler_requires_monotonic_chunks() {
        let mut a = AudioAssembler::default();
        assert!(
            a.push(AudioChunk {
                sequence: 1,
                data: vec![1],
                duration_ms: 20
            })
            .is_err()
        );
        a.push(AudioChunk {
            sequence: 0,
            data: vec![1, 2],
            duration_ms: 20,
        })
        .unwrap();
        a.push(AudioChunk {
            sequence: 1,
            data: vec![3],
            duration_ms: 10,
        })
        .unwrap();
        assert_eq!(a.emitted_ms(), 30);
        assert_eq!(a.into_bytes(), vec![1, 2, 3]);
    }

    #[test]
    fn assembler_rejects_chunks_over_explicit_budget() {
        let mut a = AudioAssembler::with_limit(2).unwrap();
        assert_eq!(a.remaining_bytes(), Some(2));
        assert!(
            a.push(AudioChunk {
                sequence: 0,
                data: vec![1, 2],
                duration_ms: 1
            })
            .is_ok()
        );
        assert_eq!(a.remaining_bytes(), Some(0));
        assert!(
            a.push(AudioChunk {
                sequence: 1,
                data: vec![3],
                duration_ms: 1
            })
            .is_err()
        );
        assert_eq!(a.into_bytes(), vec![1, 2]);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListenEvent {
    SpeechStarted,
    Partial {
        delta: String,
        text: String,
    },
    Final {
        text: String,
        confidence: Option<f32>,
    },
    SpeechStopped,
    Failed(String),
}

#[async_trait]
pub trait ListenStream: Send {
    async fn push(&mut self, frame: &[i16]) -> Result<(), VoiceError>;
    async fn flush(&mut self) -> Result<(), VoiceError>;
    async fn next(&mut self) -> Option<ListenEvent>;
    async fn close(self: Box<Self>) -> Option<String>;
}

#[async_trait]
pub trait Listener: Send + Sync {
    fn name(&self) -> &str;
    fn endpointing(&self) -> Endpointing;
    async fn open(
        &self,
        spec: ListenSpec,
        cancel: CancellationToken,
    ) -> Result<Box<dyn ListenStream>, VoiceError>;
}

#[async_trait]
pub trait SpeakStream: Send {
    async fn next(&mut self) -> Option<Result<AudioChunk, VoiceError>>;
    fn emitted_ms(&self) -> u64;
}

#[async_trait]
pub trait Speaker: Send + Sync {
    fn name(&self) -> &str;
    fn supports(&self, format: SpeakFormat) -> bool;
    async fn speak(
        &self,
        spec: SpeakSpec,
        cancel: CancellationToken,
    ) -> Result<Box<dyn SpeakStream>, VoiceError>;
}

#[async_trait]
pub trait Transcriber: Send + Sync {
    fn name(&self) -> &str;
    async fn transcribe(
        &self,
        audio: audio::AudioBlob,
        spec: ListenSpec,
        cancel: &CancellationToken,
    ) -> Result<String, VoiceError>;
}
