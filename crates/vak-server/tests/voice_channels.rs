//! Channel voice notes end to end (docs/design/49-live-voice.md): a bridge
//! only attaches the audio; the gateway transcribes it after allowlist
//! admission, through the chat's voice route, and the model receives the
//! words as the user's message. A chat awaiting approval never spends a
//! provider call.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;
use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

/// Answers every turn and keeps the last request it was sent.
#[derive(Default)]
struct Recording {
    capacity_key: crate::support::CapacityKey,
    last_request: Mutex<String>,
}

#[async_trait::async_trait]
impl Provider for Recording {
    fn name(&self) -> &str {
        "recording"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        *self.last_request.lock().unwrap() = format!("{:?}", request.messages.last());
        let message = AssistantMessage {
            content: vec![ContentBlock::text("Four.")],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "test-model".into(),
            response_id: None,
        };
        let (mut sink, rx) = stream::channel(64);
        sink.push(stream::StreamEvent::Start {
            partial: message.clone(),
        });
        sink.close_message(message).await;
        Ok(rx)
    }
}

fn transcriber(dir: &Path) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let calls = dir.join("transcriber-calls");
    let path = dir.join("transcriber.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n/bin/cat >/dev/null\necho call >> '{}'\nprintf 'what is two plus two'\n",
            calls.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    (path, calls)
}

fn calls(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

async fn spawn_gateway(allowlist_open: bool, provider: Arc<Recording>) -> String {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        format!(
            "[memory]\nreflection = false\n[gateway]\nchat_allowlist_open = {allowlist_open}\n"
        ),
    )
    .unwrap();
    let core = Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_provider_instance(provider);
    core.apply_persisted_voice(vak_config::VoiceSettings {
        enabled: true,
        provider: Some("local".into()),
        ..Default::default()
    });
    std::mem::forget(dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::gateway_router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn voice_note(chat: &str) -> serde_json::Value {
    use base64::Engine as _;
    serde_json::json!({
        "surface": "telegram",
        "chat": chat,
        "sender": "alice",
        "text": "",
        "wait": true,
        "attachments": [{
            "kind": "audio",
            "mime": "audio/ogg",
            "filename": "voice.ogg",
            "data": base64::engine::general_purpose::STANDARD.encode(b"OggS fake opus"),
        }],
    })
}

/// One test, because the transcriber override is process-global.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn channel_voice_notes_are_transcribed_only_after_admission() {
    let scratch = tempfile::tempdir().unwrap();
    let (script, calls_path) = transcriber(scratch.path());
    vak_config::set_override(vak_voice::TRANSCRIBER_VAR, script.display().to_string());
    let client = reqwest::Client::new();

    // An unknown chat lands as a pending review; its voice note is never
    // transcribed, so a stranger cannot spend the operator's provider.
    let closed = spawn_gateway(false, Arc::new(Recording::default())).await;
    let pending = client
        .post(format!("{closed}/gateway/inbound"))
        .json(&voice_note("1001"))
        .send()
        .await
        .unwrap();
    assert_eq!(pending.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        calls(&calls_path),
        0,
        "a pending chat must not be transcribed"
    );

    // An admitted chat's note becomes the user's words.
    let provider = Arc::new(Recording::default());
    let open = spawn_gateway(true, provider.clone()).await;
    let admitted = client
        .post(format!("{open}/gateway/inbound"))
        .json(&voice_note("2002"))
        .send()
        .await
        .unwrap();
    let status = admitted.status();
    let body: serde_json::Value = admitted.json().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");
    assert_eq!(calls(&calls_path), 1);
    let request = provider.last_request.lock().unwrap().clone();
    assert!(
        request.contains("what is two plus two"),
        "the model must receive the transcript: {request}"
    );
    assert!(
        !request.contains("not configured"),
        "no false statement about the transcription provider: {request}"
    );
}

/// A bot's voice tier is checked where it is written: a provider that does
/// not exist is refused, not stored to fail later on a voice note.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bot_voice_tier_with_an_unknown_provider_is_refused() {
    let base = spawn_gateway(true, Arc::new(Recording::default())).await;
    let client = reqwest::Client::new();
    let created = client
        .post(format!("{base}/gateway/bots"))
        .json(&serde_json::json!({"id": "support", "surface": "telegram", "label": "Support"}))
        .send()
        .await
        .unwrap();
    assert!(created.status().is_success(), "{}", created.status());
    let patch = |provider: &str| {
        client
            .patch(format!("{base}/gateway/bots/support"))
            .json(&serde_json::json!({"voice": {"provider": provider}}))
            .send()
    };
    assert_eq!(
        patch("google").await.unwrap().status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert!(patch("openai").await.unwrap().status().is_success());
}
