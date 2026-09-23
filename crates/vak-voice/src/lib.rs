//! Provider-neutral voice contracts and deterministic audio primitives
//! (docs/design/49-live-voice.md). Surfaces never own audio transport,
//! credentials, or conversational policy; this crate owns the vocabulary
//! they share: which providers exist, the socket wire protocol, the speech
//! evidence an utterance must carry before it may cost a provider call, and
//! the offline engines.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod audio;
pub mod local;
pub mod protocol;
pub mod provider;
pub mod vad;

pub use local::{
    EngineReadiness, LocalTranscriber, LocalTtsSpeaker, TRANSCRIBER_VAR, TTS_VAR, engine_readiness,
};
pub use provider::{VoiceDescriptor, VoiceProvider, catalogue};

#[derive(Debug, Error)]
pub enum VoiceError {
    #[error("voice provider unavailable: {0}")]
    Unavailable(String),
    #[error("voice request rejected: {0}")]
    InvalidRequest(String),
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

impl SpeakFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "wav" => Some(Self::Wav),
            "pcm16" => Some(Self::Pcm16),
            "ogg_opus" => Some(Self::OggOpus),
            "mp3" => Some(Self::Mp3),
            _ => None,
        }
    }

    /// The canonical name, as accepted by [`Self::parse`] and passed to a
    /// local engine.
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Pcm16 => "pcm16",
            Self::OggOpus => "ogg_opus",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Pcm16 => "audio/pcm",
            Self::OggOpus => "audio/ogg",
            Self::Mp3 => "audio/mpeg",
            Self::Wav => "audio/wav",
        }
    }
}
