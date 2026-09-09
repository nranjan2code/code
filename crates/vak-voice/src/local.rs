//! Offline voice backends used by local hosts and tests.
use crate::{
    AudioChunk, ListenSpec, SpeakFormat, SpeakSpec, SpeakStream, Speaker, Transcriber, VoiceError,
};
use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub struct LocalSpeaker;

/// Explicit offline transcription boundary. A local speech engine can be
/// installed behind this trait without changing the session or transport
/// contracts; the bundled backend fails closed rather than pretending that
/// silence is a transcript.
pub struct LocalTranscriber {
    /// Executable receives the encoded audio on stdin and must write the
    /// transcript to stdout. This is deliberately an executable path rather
    /// than a shell command, so configuration cannot inject shell syntax.
    executable: Option<std::path::PathBuf>,
}

impl LocalTranscriber {
    pub fn from_env() -> Self {
        Self {
            executable: std::env::var_os("VAK_LOCAL_TRANSCRIBER").map(std::path::PathBuf::from),
        }
    }
    pub fn new(executable: impl Into<std::path::PathBuf>) -> Self {
        Self {
            executable: Some(executable.into()),
        }
    }
}

#[async_trait]
impl Transcriber for LocalTranscriber {
    fn name(&self) -> &str {
        "local"
    }

    async fn transcribe(
        &self,
        audio: crate::audio::AudioBlob,
        _spec: ListenSpec,
        cancel: &CancellationToken,
    ) -> Result<String, VoiceError> {
        if cancel.is_cancelled() {
            return Err(VoiceError::Cancelled);
        }
        if audio.data.is_empty() {
            return Err(VoiceError::InvalidRequest("audio must not be empty".into()));
        }
        let Some(executable) = &self.executable else {
            return Err(VoiceError::Unavailable(
                "local transcription is unavailable: configure VAK_LOCAL_TRANSCRIBER to an executable that reads audio on stdin and writes text on stdout".into(),
            ));
        };
        let mut child = Command::new(executable)
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| {
                VoiceError::Unavailable(format!(
                    "local transcription engine could not start: {error}"
                ))
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&audio.data).await.map_err(|error| {
                VoiceError::Unavailable(format!("local transcription input failed: {error}"))
            })?;
        }
        // `wait_with_output` owns the child; check cancellation before entering
        // it so a cancelled request never starts a local subprocess operation.
        if cancel.is_cancelled() {
            let _ = child.kill().await;
            return Err(VoiceError::Cancelled);
        }
        let output = child.wait_with_output().await.map_err(|error| {
            VoiceError::Unavailable(format!("local transcription failed: {error}"))
        })?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(VoiceError::Unavailable(format!(
                "local transcription engine exited unsuccessfully{}",
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            )));
        }
        let text = String::from_utf8(output.stdout).map_err(|_| {
            VoiceError::Unavailable("local transcription returned non UTF-8 output".into())
        })?;
        if text.trim().is_empty() {
            return Err(VoiceError::Unavailable(
                "local transcription returned empty text".into(),
            ));
        }
        Ok(text.trim().to_owned())
    }
}

struct SilenceStream {
    chunks: u64,
    emitted: u64,
}

#[async_trait]
impl SpeakStream for SilenceStream {
    async fn next(&mut self) -> Option<Result<AudioChunk, VoiceError>> {
        if self.chunks >= 1 {
            return None;
        }
        self.chunks += 1;
        let bytes_per_sample = 2;
        let data = vec![0u8; 320 * bytes_per_sample];
        self.emitted = 20;
        Some(Ok(AudioChunk {
            sequence: 0,
            data,
            duration_ms: 20,
        }))
    }
    fn emitted_ms(&self) -> u64 {
        self.emitted
    }
}

#[async_trait]
impl Speaker for LocalSpeaker {
    fn name(&self) -> &str {
        "local"
    }
    fn supports(&self, format: SpeakFormat) -> bool {
        matches!(format, SpeakFormat::Pcm16 | SpeakFormat::Wav)
    }
    async fn speak(
        &self,
        spec: SpeakSpec,
        cancel: CancellationToken,
    ) -> Result<Box<dyn SpeakStream>, VoiceError> {
        if cancel.is_cancelled() {
            return Err(VoiceError::Cancelled);
        }
        if spec.text.trim().is_empty() {
            return Err(VoiceError::InvalidRequest(
                "speech text must not be empty".into(),
            ));
        }
        Ok(Box::new(SilenceStream {
            chunks: 0,
            emitted: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpeakFormat;

    #[tokio::test]
    async fn emits_bounded_pcm_for_offline_hosts() {
        let speaker = LocalSpeaker;
        let token = CancellationToken::new();
        let mut stream = speaker
            .speak(
                SpeakSpec {
                    text: "hello".into(),
                    model: None,
                    voice: None,
                    format: SpeakFormat::Pcm16,
                },
                token,
            )
            .await
            .unwrap();
        let chunk = stream.next().await.unwrap().unwrap();
        assert_eq!(chunk.duration_ms, 20);
        assert_eq!(chunk.data.len(), 640);
        assert!(stream.next().await.is_none());
        assert_eq!(stream.emitted_ms(), 20);
    }

    #[tokio::test]
    async fn transcription_fails_closed_without_an_installed_engine() {
        use crate::audio::AudioBlob;
        let transcriber = LocalTranscriber::from_env();
        let result = transcriber
            .transcribe(
                AudioBlob {
                    data: vec![0, 0],
                    mime: "audio/pcm".into(),
                },
                ListenSpec {
                    model: None,
                    language: None,
                },
                &CancellationToken::new(),
            )
            .await;
        assert!(matches!(result, Err(VoiceError::Unavailable(_))));
    }
}
