use super::VoiceError;
use serde::{Deserialize, Serialize};

pub const MAX_CONTROL_BYTES: usize = 64 * 1024;
pub const MAX_AUDIO_FRAME_BYTES: usize = 64 * 1024;
/// Wire contract version for the WebSocket voice session protocol.
/// Additive fields remain compatible within a major contract.
pub const VOICE_PROTOCOL_VERSION: u16 = 1;

pub fn supports_protocol_version(version: u16) -> bool {
    version == VOICE_PROTOCOL_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Control {
    Ready {
        /// Optional for backward compatibility with pre-versioned clients.
        #[serde(default)]
        protocol_version: Option<u16>,
        sample_rate_hz: u32,
        channels: u16,
    },
    Transcript {
        utterance_id: String,
        text: String,
        #[serde(rename = "final")]
        final_: bool,
    },
    SpeechStarted {
        utterance_id: String,
    },
    SpeechStopped {
        utterance_id: String,
    },
    Playback {
        utterance_id: String,
        emitted_ms: u64,
        interrupted: bool,
    },
    Receipt {
        utterance_id: String,
        receipt: serde_json::Value,
    },
    Error {
        message: String,
        remedy: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Audio(Vec<u8>),
    Control(Control),
}

impl Frame {
    pub fn encode_control(control: &Control) -> Result<Vec<u8>, VoiceError> {
        let bytes =
            serde_json::to_vec(control).map_err(|e| VoiceError::Transport(e.to_string()))?;
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(VoiceError::InvalidRequest(
                "voice control frame too large".into(),
            ));
        }
        Ok(bytes)
    }

    pub fn decode_control(bytes: &[u8]) -> Result<Control, VoiceError> {
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(VoiceError::InvalidRequest(
                "voice control frame too large".into(),
            ));
        }
        serde_json::from_slice(bytes)
            .map_err(|e| VoiceError::Transport(format!("invalid voice control frame: {e}")))
    }

    pub fn validate_audio(bytes: &[u8]) -> Result<(), VoiceError> {
        if bytes.is_empty() || bytes.len() > MAX_AUDIO_FRAME_BYTES || !bytes.len().is_multiple_of(2)
        {
            return Err(VoiceError::InvalidRequest("invalid PCM audio frame".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_round_trip() {
        let c = Control::Transcript {
            utterance_id: "u1".into(),
            text: "hello".into(),
            final_: true,
        };
        assert_eq!(
            Frame::decode_control(&Frame::encode_control(&c).unwrap()).unwrap(),
            c
        );
        let json = String::from_utf8(Frame::encode_control(&c).unwrap()).unwrap();
        assert!(json.contains("\"final\":true"));
        let receipt = Control::Receipt {
            utterance_id: "u1".into(),
            receipt: serde_json::json!({"purpose":"voice_synthesis"}),
        };
        assert_eq!(
            Frame::decode_control(&Frame::encode_control(&receipt).unwrap()).unwrap(),
            receipt
        );
    }
    #[test]
    fn audio_validation_rejects_odd_pcm() {
        assert!(Frame::validate_audio(&[1]).is_err());
        assert!(Frame::validate_audio(&[0, 0]).is_ok());
    }

    #[test]
    fn ready_advertises_version_and_accepts_legacy_payloads() {
        let ready = Control::Ready {
            protocol_version: Some(VOICE_PROTOCOL_VERSION),
            sample_rate_hz: 16_000,
            channels: 1,
        };
        let bytes = Frame::encode_control(&ready).unwrap();
        let decoded = Frame::decode_control(&bytes).unwrap();
        assert_eq!(decoded, ready);
        let legacy = br#"{"t":"ready","sample_rate_hz":16000,"channels":1}"#;
        let decoded = Frame::decode_control(legacy).unwrap();
        assert!(matches!(
            decoded,
            Control::Ready {
                protocol_version: None,
                ..
            }
        ));
    }

    #[test]
    fn unsupported_protocol_versions_fail_the_capability_check() {
        assert!(supports_protocol_version(VOICE_PROTOCOL_VERSION));
        assert!(!supports_protocol_version(VOICE_PROTOCOL_VERSION + 1));
    }
}
