//! Offline voice backends used by local hosts and tests.
use crate::{
    AudioChunk, ListenSpec, SpeakFormat, SpeakSpec, SpeakStream, Speaker, Transcriber, VoiceError,
};
use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

pub struct LocalSpeaker;

/// Configurable local TTS boundary. The executable receives UTF-8 text on
/// stdin and writes the requested encoded audio to stdout. It is deliberately
/// an executable path, never a shell command.
pub struct LocalTtsSpeaker {
    executable: Option<std::path::PathBuf>,
}

/// Read-only readiness information for administration and doctor surfaces.
/// This deliberately inspects filesystem metadata only; it never executes the
/// configured program and never exposes inherited environment values.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalTtsReadiness {
    pub configured: bool,
    pub ready: bool,
    pub executable: Option<String>,
    pub detail: String,
}

pub fn local_tts_readiness() -> LocalTtsReadiness {
    let executable = std::env::var_os("VAK_LOCAL_TTS").map(std::path::PathBuf::from);
    let Some(path) = executable else {
        return LocalTtsReadiness {
            configured: false,
            ready: false,
            executable: None,
            detail: "not configured (optional local TTS)".into(),
        };
    };
    let display = path.display().to_string();
    match std::fs::metadata(&path) {
        Ok(meta) if meta.is_file() => LocalTtsReadiness {
            configured: true,
            ready: true,
            executable: Some(display),
            detail: "executable is present".into(),
        },
        Ok(_) => LocalTtsReadiness {
            configured: true,
            ready: false,
            executable: Some(display),
            detail: "configured path is not a regular file".into(),
        },
        Err(error) => LocalTtsReadiness {
            configured: true,
            ready: false,
            executable: Some(display),
            detail: format!("configured executable is unavailable: {error}"),
        },
    }
}

impl LocalTtsSpeaker {
    pub fn from_env() -> Self {
        Self {
            executable: std::env::var_os("VAK_LOCAL_TTS").map(std::path::PathBuf::from),
        }
    }

    pub fn new(executable: impl Into<std::path::PathBuf>) -> Self {
        Self {
            executable: Some(executable.into()),
        }
    }
}

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

struct LocalAudioStream {
    chunks: std::vec::IntoIter<Vec<u8>>,
    emitted_ms: u64,
}

#[async_trait]
impl SpeakStream for LocalAudioStream {
    async fn next(&mut self) -> Option<Result<AudioChunk, VoiceError>> {
        self.chunks.next().map(|data| {
            Ok(AudioChunk {
                sequence: 0,
                duration_ms: self.emitted_ms.min(u32::MAX as u64) as u32,
                data,
            })
        })
    }

    fn emitted_ms(&self) -> u64 {
        self.emitted_ms
    }
}

#[async_trait]
impl Speaker for LocalTtsSpeaker {
    fn name(&self) -> &str {
        "local-tts"
    }

    fn supports(&self, format: SpeakFormat) -> bool {
        matches!(
            format,
            SpeakFormat::Pcm16 | SpeakFormat::Wav | SpeakFormat::OggOpus | SpeakFormat::Mp3
        )
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
        if !self.supports(spec.format) {
            return Err(VoiceError::InvalidRequest(format!(
                "local TTS does not support {:?}",
                spec.format
            )));
        }
        let Some(executable) = &self.executable else {
            return Err(VoiceError::Unavailable("local TTS is unavailable: configure VAK_LOCAL_TTS to an executable that reads UTF-8 text on stdin and writes audio on stdout".into()));
        };
        let mut child = Command::new(executable)
            .env_clear()
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| {
                VoiceError::Unavailable(format!("local TTS engine could not start: {error}"))
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(spec.text.as_bytes())
                .await
                .map_err(|error| {
                    VoiceError::Unavailable(format!("local TTS input failed: {error}"))
                })?;
        }
        if cancel.is_cancelled() {
            let _ = child.kill().await;
            return Err(VoiceError::Cancelled);
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| VoiceError::Unavailable("local TTS did not provide stdout".into()))?;
        let mut audio = Vec::new();
        stdout
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut audio)
            .await
            .map_err(|error| {
                VoiceError::Unavailable(format!("local TTS output failed: {error}"))
            })?;
        let result = child
            .wait()
            .await
            .map_err(|error| VoiceError::Unavailable(format!("local TTS failed: {error}")))?;
        if cancel.is_cancelled() {
            return Err(VoiceError::Cancelled);
        }
        if !result.success() {
            return Err(VoiceError::Unavailable(
                "local TTS engine exited unsuccessfully".into(),
            ));
        }
        if audio.is_empty() {
            return Err(VoiceError::Unavailable(
                "local TTS returned empty audio".into(),
            ));
        }
        if audio.len() > 16 * 1024 * 1024 {
            return Err(VoiceError::InvalidRequest(
                "local TTS output exceeds 16 MiB".into(),
            ));
        }
        Ok(Box::new(LocalAudioStream {
            chunks: vec![audio].into_iter(),
            emitted_ms: 0,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn local_engine_receives_audio_without_provider_secrets() {
        use crate::audio::AudioBlob;
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "vak-voice-transcriber-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after epoch")
                .as_nanos()
        ));
        std::fs::write(
            &path,
            b"#!/bin/sh\nif [ -n \"$GEMINI_API_KEY\" ] || [ -n \"$OPENAI_API_KEY\" ]; then exit 9; fi\ncat >/dev/null\nprintf 'offline transcript\\n'\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&path, perms).unwrap();

        let result = LocalTranscriber::new(&path)
            .transcribe(
                AudioBlob {
                    data: vec![1, 2, 3],
                    mime: "audio/ogg".into(),
                },
                ListenSpec {
                    model: None,
                    language: None,
                },
                &CancellationToken::new(),
            )
            .await;
        let _ = std::fs::remove_file(&path);
        assert_eq!(result.unwrap(), "offline transcript");
    }

    #[tokio::test]
    async fn local_tts_fails_closed_without_an_installed_engine() {
        let result = LocalTtsSpeaker::new("/definitely/missing/vak-tts")
            .speak(
                SpeakSpec {
                    text: "hello".into(),
                    model: None,
                    voice: None,
                    format: SpeakFormat::Wav,
                },
                CancellationToken::new(),
            )
            .await;
        assert!(matches!(result, Err(VoiceError::Unavailable(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_tts_receives_text_and_returns_audio() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("vak-voice-tts-{}", std::process::id()));
        std::fs::write(&path, b"#!/bin/sh\ncat >/dev/null\nprintf 'audio-bytes'\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&path, perms).unwrap();
        let mut stream = LocalTtsSpeaker::new(&path)
            .speak(
                SpeakSpec {
                    text: "hello".into(),
                    model: None,
                    voice: None,
                    format: SpeakFormat::Wav,
                },
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(stream.next().await.unwrap().unwrap().data, b"audio-bytes");
        let _ = std::fs::remove_file(path);
    }
}
