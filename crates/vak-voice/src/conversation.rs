//! Provider-neutral conversation turn contract.
//! Surfaces submit committed transcripts here; adapters attach governed
//! transcription/synthesis implementations without inventing a second flow.
use crate::{SpeakSpec, VoiceError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceTurn {
    pub utterance_id: String,
    pub transcript: String,
}

impl VoiceTurn {
    pub fn new(
        utterance_id: impl Into<String>,
        transcript: impl Into<String>,
    ) -> Result<Self, VoiceError> {
        let utterance_id = utterance_id.into();
        let transcript = transcript.into();
        if utterance_id.trim().is_empty() || transcript.trim().is_empty() {
            return Err(VoiceError::InvalidRequest(
                "voice turns require an utterance id and transcript".into(),
            ));
        }
        Ok(Self {
            utterance_id,
            transcript,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SynthesisPlan {
    pub turn: VoiceTurn,
    pub speech: SpeakSpec,
}

#[derive(Debug, Default)]
pub struct ConversationLedger {
    turns: Vec<VoiceTurn>,
}

impl ConversationLedger {
    pub fn record(&mut self, turn: VoiceTurn) -> Result<(), VoiceError> {
        if self
            .turns
            .iter()
            .any(|prior| prior.utterance_id == turn.utterance_id)
        {
            return Err(VoiceError::InvalidRequest(
                "duplicate voice utterance id".into(),
            ));
        }
        self.turns.push(turn);
        Ok(())
    }
    pub fn turns(&self) -> &[VoiceTurn] {
        &self.turns
    }
}

impl SynthesisPlan {
    pub fn validate(&self) -> Result<(), VoiceError> {
        if self.speech.text.trim().is_empty() {
            return Err(VoiceError::InvalidRequest(
                "synthesis text must not be empty".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpeakFormat;

    #[test]
    fn turn_rejects_missing_identity_or_text() {
        assert!(VoiceTurn::new("", "hello").is_err());
        assert!(VoiceTurn::new("u1", " ").is_err());
    }

    #[test]
    fn synthesis_plan_keeps_turn_identity() {
        let turn = VoiceTurn::new("u1", "hello").unwrap();
        let plan = SynthesisPlan {
            turn: turn.clone(),
            speech: SpeakSpec {
                text: "hello".into(),
                model: None,
                voice: None,
                format: SpeakFormat::Pcm16,
            },
        };
        assert_eq!(plan.turn, turn);
        assert!(plan.validate().is_ok());
    }

    #[test]
    fn ledger_rejects_duplicate_utterances() {
        let mut ledger = ConversationLedger::default();
        ledger.record(VoiceTurn::new("u1", "one").unwrap()).unwrap();
        assert!(
            ledger
                .record(VoiceTurn::new("u1", "again").unwrap())
                .is_err()
        );
        assert_eq!(ledger.turns().len(), 1);
    }

    #[test]
    fn turn_round_trips_for_append_only_storage() {
        let turn = VoiceTurn::new("u1", "inspect the build").unwrap();
        let encoded = serde_json::to_vec(&turn).unwrap();
        let decoded: VoiceTurn = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, turn);
    }
}
