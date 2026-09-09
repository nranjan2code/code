use super::audio::AudioBlob;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptionRequest {
    pub model: Option<String>,
    pub language: Option<String>,
    pub mime: String,
    pub byte_len: usize,
}

impl TranscriptionRequest {
    pub fn from_blob(blob: &AudioBlob, model: Option<String>, language: Option<String>) -> Self {
        Self {
            model,
            language,
            mime: blob.mime.clone(),
            byte_len: blob.data.len(),
        }
    }
    pub fn validate(&self, max_bytes: usize) -> Result<(), super::VoiceError> {
        if self.byte_len == 0 || self.byte_len > max_bytes {
            return Err(super::VoiceError::InvalidRequest(
                "audio recording exceeds configured limit".into(),
            ));
        }
        if !self.mime.starts_with("audio/") {
            return Err(super::VoiceError::InvalidRequest(
                "audio MIME type required".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_audio_blob_metadata_and_size() {
        let blob = AudioBlob {
            mime: "audio/ogg".into(),
            data: vec![1, 2, 3],
        };
        let req = TranscriptionRequest::from_blob(&blob, Some("whisper".into()), None);
        assert!(req.validate(4).is_ok());
        assert!(
            TranscriptionRequest {
                mime: "text/plain".into(),
                ..req
            }
            .validate(4)
            .is_err()
        );
    }
}
