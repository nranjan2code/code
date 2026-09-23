//! Offline voice engines. Each is an operator-installed executable reached by
//! path, never a shell command, run with an empty environment so provider
//! credentials cannot leak into it (invariant 12). Neither engine is bundled:
//! an unconfigured engine fails closed rather than pretending silence is
//! speech or a transcript. Callers resolve the paths ([`TRANSCRIBER_VAR`],
//! [`TTS_VAR`]) through the configuration chain; this crate never reads the
//! environment.
use crate::{SpeakFormat, VoiceError};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const MAX_ENGINE_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

/// Names the local speech-to-text executable.
pub const TRANSCRIBER_VAR: &str = "VAK_LOCAL_TRANSCRIBER";
/// Names the local text-to-speech executable.
pub const TTS_VAR: &str = "VAK_LOCAL_TTS";

/// Read-only readiness of one local engine for administration and doctor
/// surfaces. It inspects filesystem metadata only; it never executes the
/// program and never exposes inherited environment values.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineReadiness {
    pub configured: bool,
    pub ready: bool,
    pub executable: Option<String>,
    pub detail: String,
}

pub fn engine_readiness(executable: Option<&std::path::Path>) -> EngineReadiness {
    let Some(path) = executable else {
        return EngineReadiness {
            configured: false,
            ready: false,
            executable: None,
            detail: "not configured".into(),
        };
    };
    let (ready, detail) = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => (true, "executable is present".to_string()),
        Ok(_) => (false, "configured path is not a regular file".to_string()),
        Err(error) => (
            false,
            format!("configured executable is unavailable: {error}"),
        ),
    };
    EngineReadiness {
        configured: true,
        ready,
        executable: Some(path.display().to_string()),
        detail,
    }
}

/// Local speech-to-text. The executable ([`TRANSCRIBER_VAR`]) receives
/// encoded audio on stdin — WAV from the voice socket, the channel's own
/// container otherwise — and writes the transcript to stdout.
pub struct LocalTranscriber {
    executable: Option<PathBuf>,
}

impl LocalTranscriber {
    pub fn new(executable: Option<PathBuf>) -> Self {
        Self { executable }
    }

    /// The transcript, trimmed. An engine that hears nothing may print
    /// nothing; that is an empty transcript, not an engine failure.
    pub async fn transcribe(
        &self,
        audio: &[u8],
        cancel: &CancellationToken,
    ) -> Result<String, VoiceError> {
        if audio.is_empty() {
            return Err(VoiceError::InvalidRequest("audio must not be empty".into()));
        }
        let Some(executable) = &self.executable else {
            return Err(VoiceError::Unavailable(
                "local transcription is unavailable: configure VAK_LOCAL_TRANSCRIBER to an executable that reads audio on stdin and writes text on stdout".into(),
            ));
        };
        let output = run_engine(executable, &[], audio, cancel, "local transcription").await?;
        let text = String::from_utf8(output).map_err(|_| {
            VoiceError::Unavailable("local transcription returned non UTF-8 output".into())
        })?;
        Ok(text.trim().to_owned())
    }
}

/// Local text-to-speech. The executable ([`TTS_VAR`]) is invoked with the
/// requested encoding as its only argument (`wav`, `pcm16`, `ogg_opus` or
/// `mp3`), receives UTF-8 text on stdin and writes that encoding to stdout;
/// the bytes are returned unchanged.
pub struct LocalTtsSpeaker {
    executable: Option<PathBuf>,
}

impl LocalTtsSpeaker {
    pub fn new(executable: Option<PathBuf>) -> Self {
        Self { executable }
    }

    pub async fn speak(
        &self,
        text: &str,
        format: SpeakFormat,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, VoiceError> {
        if text.trim().is_empty() {
            return Err(VoiceError::InvalidRequest(
                "speech text must not be empty".into(),
            ));
        }
        let Some(executable) = &self.executable else {
            return Err(VoiceError::Unavailable("local TTS is unavailable: configure VAK_LOCAL_TTS to an executable that reads UTF-8 text on stdin and writes audio on stdout".into()));
        };
        let audio = run_engine(
            executable,
            &[format.as_arg()],
            text.as_bytes(),
            cancel,
            "local TTS",
        )
        .await?;
        if audio.is_empty() {
            return Err(VoiceError::Unavailable(
                "local TTS returned empty audio".into(),
            ));
        }
        Ok(audio)
    }
}

async fn run_engine(
    executable: &std::path::Path,
    args: &[&str],
    input: &[u8],
    cancel: &CancellationToken,
    label: &str,
) -> Result<Vec<u8>, VoiceError> {
    if cancel.is_cancelled() {
        return Err(VoiceError::Cancelled);
    }
    let mut child = Command::new(executable)
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            VoiceError::Unavailable(format!("{label} engine could not start: {error}"))
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input)
            .await
            .map_err(|error| VoiceError::Unavailable(format!("{label} input failed: {error}")))?;
    }
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(VoiceError::Unavailable(format!(
            "{label} did not provide output pipes"
        )));
    };
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let mut stdout = stdout.take(MAX_ENGINE_OUTPUT_BYTES + 1);
    let mut stderr = stderr.take(4096);
    // Drain both pipes together: an engine that fills stderr while stdout is
    // unread would otherwise block forever.
    let read = async {
        tokio::try_join!(
            stdout.read_to_end(&mut output),
            stderr.read_to_end(&mut diagnostics)
        )
    };
    tokio::select! {
        result = read => {
            result.map_err(|error| VoiceError::Unavailable(format!("{label} output failed: {error}")))?;
        }
        () = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(VoiceError::Cancelled);
        }
    }
    if output.len() as u64 > MAX_ENGINE_OUTPUT_BYTES {
        let _ = child.kill().await;
        return Err(VoiceError::InvalidRequest(format!(
            "{label} output exceeds 16 MiB"
        )));
    }
    let status = child
        .wait()
        .await
        .map_err(|error| VoiceError::Unavailable(format!("{label} failed: {error}")))?;
    if !status.success() {
        let detail = String::from_utf8_lossy(&diagnostics).trim().to_owned();
        return Err(VoiceError::Unavailable(format!(
            "{label} engine exited unsuccessfully{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )));
    }
    Ok(output)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn script(name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "vak-voice-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock is after epoch")
                .as_nanos()
        ));
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[tokio::test]
    async fn transcription_fails_closed_without_an_installed_engine() {
        let result = LocalTranscriber::new(None)
            .transcribe(&[0, 0], &CancellationToken::new())
            .await;
        assert!(matches!(result, Err(VoiceError::Unavailable(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_engine_receives_audio_without_provider_secrets() {
        let path = script(
            "stt",
            "#!/bin/sh\nif [ -n \"$GEMINI_API_KEY\" ] || [ -n \"$OPENAI_API_KEY\" ]; then exit 9; fi\ncat >/dev/null\nprintf 'offline transcript\\n'\n",
        );
        let result = LocalTranscriber::new(Some(path.clone()))
            .transcribe(&[1, 2, 3], &CancellationToken::new())
            .await;
        let _ = std::fs::remove_file(&path);
        assert_eq!(result.unwrap(), "offline transcript");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_engine_that_hears_nothing_returns_an_empty_transcript() {
        let path = script("stt-empty", "#!/bin/sh\ncat >/dev/null\n");
        let result = LocalTranscriber::new(Some(path.clone()))
            .transcribe(&[1, 2], &CancellationToken::new())
            .await;
        let _ = std::fs::remove_file(&path);
        assert_eq!(result.unwrap(), "");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failing_engine_reports_its_stderr() {
        let path = script(
            "stt-fail",
            "#!/bin/sh\ncat >/dev/null\necho 'model missing' >&2\nexit 3\n",
        );
        let result = LocalTranscriber::new(Some(path.clone()))
            .transcribe(&[1, 2], &CancellationToken::new())
            .await;
        let _ = std::fs::remove_file(&path);
        let error = result.unwrap_err().to_string();
        assert!(error.contains("model missing"), "{error}");
    }

    #[tokio::test]
    async fn local_tts_fails_closed_without_an_installed_engine() {
        let result = LocalTtsSpeaker::new(Some("/definitely/missing/vak-tts".into()))
            .speak("hello", SpeakFormat::Wav, &CancellationToken::new())
            .await;
        assert!(matches!(result, Err(VoiceError::Unavailable(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_tts_receives_text_and_returns_audio() {
        let path = script(
            "tts",
            "#!/bin/sh\n[ \"$1\" = mp3 ] || exit 4\ncat >/dev/null\nprintf 'audio-bytes'\n",
        );
        let audio = LocalTtsSpeaker::new(Some(path.clone()))
            .speak("hello", SpeakFormat::Mp3, &CancellationToken::new())
            .await;
        let _ = std::fs::remove_file(&path);
        assert_eq!(audio.unwrap(), b"audio-bytes");
    }
}
