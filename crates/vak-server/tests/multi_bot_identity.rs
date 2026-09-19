//! Integration tests for multi-bot-per-channel identity
//! (docs/design/34 Phase 5 follow-up, AGENTS.md invariants 23-24).
//!
//! These tests verify the three-segment key resolution at the HTTP
//! `/gateway/inbound` boundary: a bot-scoped key (`surface:chat:bot_id`)
//! must inherit from an already-allowed legacy key, each bot gets an
//! independent entry, and a legacy (no bot_id) message still works.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &'static str {
        "scripted"
    }
    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn text(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: vak_llm::types::StopReason::EndTurn,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

/// Spawn a gateway server with the given allowlist and one scripted reply.
/// Uses `gateway_router` (no bearer auth) for simplicity.
async fn spawn_gateway(allowlist: &[&str]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let list = allowlist
        .iter()
        .map(|k| format!("\"{k}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!(
        "[memory]\nreflection = false\n[gateway]\nenabled = true\nchat_allowlist = [{list}]\n"
    );
    let _ = std::fs::write(cwd.join(".vak/config.toml"), &config);
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_provider_instance(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("hello")])),
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::gateway_router(core))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

/// POST a message to the gateway inbound endpoint (no auth for gateway_router).
async fn post_inbound(base: &str, body: serde_json::Value) -> reqwest::StatusCode {
    let client = reqwest::Client::new();
    client
        .post(format!("{base}/gateway/inbound"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
}

/// Two bots sharing one physical chat get independent allowlist entries.
/// Bot A's message inherits the legacy approval; Bot B gets its own
/// independent entry (rule 24: one identity per bot).
#[tokio::test]
async fn two_bots_on_same_chat_get_independent_keys() {
    let base = spawn_gateway(&["telegram:12345"]).await;

    // Bot A inherits from the pre-approved legacy key.
    let status_a = post_inbound(
        &base,
        serde_json::json!({
            "surface": "telegram",
            "chat": "12345",
            "text": "hi from Alpha",
            "bot_id": "Alpha",
        }),
    )
    .await;
    assert!(
        status_a.is_success(),
        "bot-scoped key with legacy approval must be allowed (got {status_a})"
    );

    // Bot B — same physical chat, different bot identity. Also inherits
    // from the legacy key, but gets its own entry, session, and policy.
    let status_b = post_inbound(
        &base,
        serde_json::json!({
            "surface": "telegram",
            "chat": "12345",
            "text": "hi from Beta",
            "bot_id": "Beta",
        }),
    )
    .await;
    assert!(
        status_b.is_success(),
        "second bot on same chat must be independently allowed (got {status_b})"
    );

    // A second message from Bot A should still be accepted (idempotent —
    // already-allowed bot-scoped key takes the same Allowed path).
    let status_a2 = post_inbound(
        &base,
        serde_json::json!({
            "surface": "telegram",
            "chat": "12345",
            "text": "second from Alpha",
            "bot_id": "Alpha",
        }),
    )
    .await;
    assert!(
        status_a2.is_success(),
        "repeat bot-scoped message must be allowed (got {status_a2})"
    );
}

/// A bot-scoped key with no pre-approved legacy entry is rejected at the
/// gateway (pending review, AG-34).
#[tokio::test]
async fn bot_scoped_key_without_legacy_approval_is_rejected() {
    let base = spawn_gateway(&["telegram:99999"]).await;

    let status = post_inbound(
        &base,
        serde_json::json!({
            "surface": "telegram",
            "chat": "11111",
            "text": "unapproved chat with bot",
            "bot_id": "SoloBot",
        }),
    )
    .await;
    assert!(
        !status.is_success(),
        "bot-scoped key without legacy approval must be rejected (got {status})"
    );
}

/// A legacy (no bot_id) message to a pre-approved chat is allowed.
/// This confirms the two-segment key path still works after multi-bot
/// support was added (rule 24).
#[tokio::test]
async fn legacy_key_without_bot_id_still_works() {
    let base = spawn_gateway(&["telegram:54321"]).await;

    let status = post_inbound(
        &base,
        serde_json::json!({
            "surface": "telegram",
            "chat": "54321",
            "text": "legacy message",
        }),
    )
    .await;
    assert!(
        status.is_success(),
        "legacy two-segment key should be allowed (got {status})"
    );
}
