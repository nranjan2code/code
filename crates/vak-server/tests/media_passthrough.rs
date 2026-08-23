#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};

/// Records every request it receives so tests can assert exactly what the
/// model would have seen.
struct Recording {
    seen: Arc<Mutex<Vec<ChatRequest>>>,
    reply: String,
}

#[async_trait::async_trait]
impl Provider for Recording {
    fn name(&self) -> &str {
        "recording"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.seen.lock().unwrap().push(request);
        let (mut sink, rx) = stream::channel(16);
        sink.push(stream::StreamEvent::Start {
            partial: AssistantMessage {
                content: vec![ContentBlock::text(self.reply.clone())],
                stop_reason: vak_llm::types::StopReason::EndTurn,
                usage: Usage::default(),
                model: "test-model".into(),
            },
        });
        sink.close_message(AssistantMessage {
            content: vec![ContentBlock::text(self.reply.clone())],
            stop_reason: vak_llm::types::StopReason::EndTurn,
            usage: Usage::default(),
            model: "test-model".into(),
        })
        .await;
        Ok(rx)
    }
}

const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABh6FO1AAAAABJRU5ErkJggg==";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gateway_inbound_carries_images_to_the_model() {
    let seen: Arc<Mutex<Vec<ChatRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let core_provider = Arc::new(Recording {
        seen: seen.clone(),
        reply: "I see a tiny red pixel.".into(),
    });

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    // Hermetic against the developer's global config (reflection=true):
    let _ = std::fs::create_dir_all(cwd.join(".vakcoder"));
    let _ = std::fs::write(
        cwd.join(".vakcoder/config.toml"),
        "[memory]\nreflection = false\n",
    );
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_provider_instance(core_provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::gateway_router(core))
            .await
            .unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{base}/gateway/inbound"))
        .json(&serde_json::json!({
            "surface": "telegram",
            "chat": "42",
            "text": "what is this?",
            "wait": true,
            "attachments": [
                {"mime": "image/png", "data": PNG_B64}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["text"], "I see a tiny red pixel.");

    // The MODEL saw the image block...
    {
        let requests = seen.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let user_msg = requests[0]
            .messages
            .iter()
            .find(|m| m.role == vak_llm::Role::User)
            .unwrap();
        assert!(
            user_msg.content.iter().any(|b| matches!(
                b,
                ContentBlock::Image { source }
                    if source.data == PNG_B64 && source.media_type == "image/png"
            )),
            "image block reached the request"
        );
    }

    // ...and the LEDGER stored it verbatim (invariant 1).
    let sid: String = {
        // wait:true response carries session_id only on completed; re-read
        // status for the binding.
        let st: serde_json::Value = client
            .get(format!("{base}/gateway/status"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        st["bindings"][0]["session_id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let t: serde_json::Value = client
        .get(format!("{base}/sessions/{sid}/transcript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let raw = serde_json::to_string(&t).unwrap();
    assert!(
        raw.contains("iVBORw0KGgo") && raw.contains("\"type\":\"image\""),
        "image persisted on the ledger"
    );
}
