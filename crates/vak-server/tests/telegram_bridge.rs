#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_server::telegram::TelegramBridge;

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
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
    }
}

/// Minimal Bot API double: serves one scripted update, then empty polls;
/// records every sendMessage payload.
async fn spawn_mock_telegram() -> (String, Arc<Mutex<Vec<serde_json::Value>>>, Arc<AtomicUsize>) {
    let updates_left = Arc::new(Mutex::new(1u32));
    let sent: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    let get_calls = Arc::new(AtomicUsize::new(0));

    let ul = updates_left.clone();
    let gc = get_calls.clone();
    async fn ok_json() -> serde_json::Value {
        serde_json::json!({ "ok": true })
    }
    let app = axum::Router::new()
        .route(
            "/botbottok/getUpdates",
            axum::routing::get(move || async move {
                gc.fetch_add(1, Ordering::SeqCst);
                let mut left = ul.lock().unwrap();
                if *left > 0 {
                    *left -= 1;
                    // First scripted update is a PHOTO message.
                    return axum::Json(serde_json::json!({
                        "ok": true,
                        "result": [{
                            "update_id": 777,
                            "message": {
                                "chat": {"id": 4242},
                                "photo": [
                                    {"file_id": "small", "width": 90, "height": 90},
                                    {"file_id": "large", "width": 640, "height": 640}
                                ]
                            }
                        }]
                    }));
                }
                axum::Json(serde_json::json!({ "ok": true, "result": [] }))
            }),
        )
        .route(
            "/botbottok/getFile",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "ok": true,
                    "result": {"file_path": "photos/img.jpg"}
                }))
            }),
        )
        .route(
            "/file/botbottok/photos/img.jpg",
            axum::routing::get(|| async { [0x89u8, b'P', b'N', b'G'].to_vec() }),
        )
        .route(
            "/botbottok/sendMessage",
            axum::routing::post({
                let sent = sent.clone();
                move |axum::Json(body): axum::Json<serde_json::Value>| async move {
                    sent.lock().unwrap().push(body);
                    axum::Json(ok_json().await)
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), sent, get_calls)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telegram_bridge_routes_message_and_delivers_reply() {
    let (tg_base, sent, get_calls) = spawn_mock_telegram().await;

    // The gateway side of the contract.
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    // Hermetic against the developer's global config (e.g. reflection=true):
    // pin learning flags off for deterministic scripted flows.
    let _ = std::fs::create_dir_all(cwd.join(".vakcoder"));
    let _ = std::fs::write(
        cwd.join(".vakcoder/config.toml"),
        "[memory]\nreflection = false\n",
    );
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_provider_instance(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("pong from agent")])),
    }));
    std::mem::forget(dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::gateway_router(core))
            .await
            .unwrap();
    });

    let bridge = TelegramBridge {
        locks_dir: None,
        api_base: tg_base,
        bot_token: "bottok".into(),
        gateway_url: format!("http://{gw_addr}"),
        gateway_token: "vk_test".into(),
    };

    let next = bridge.tick(0).await.unwrap();
    assert_eq!(next, 778, "offset advances past the handled update");

    {
        let delivered = sent.lock().unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0]["chat_id"], 4242);
        assert_eq!(delivered[0]["text"], "pong from agent");
    }

    // A quiet poll delivers nothing new.
    bridge.tick(next).await.unwrap();
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert!(
        get_calls.load(Ordering::SeqCst) >= 2,
        "long-poll must keep polling"
    );
}
