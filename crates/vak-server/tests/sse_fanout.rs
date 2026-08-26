//! SSE fan-out (docs/design/34-base-addons.md M4.3): one base, many
//! concurrent subscribers on the same session stream — every client
//! receives every event, none starve, and slow readers do not block the
//! run or each other.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
            input_tokens: 5,
            output_tokens: 2,
            ..Default::default()
        },
        model: "test-model".into(),
    }
}

const SUBSCRIBERS: usize = 5;
const MARKER: &str = "fanout-sentinel-42";

async fn spawn_server(provider: Arc<dyn Provider>) -> (String, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(dir.path().to_path_buf()).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_provider_instance(provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::router(core);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), handle)
}

/// Collects `data:` lines until the run-finished marker or deadline.
async fn subscribe(sse_url: String, opened: tokio::sync::oneshot::Sender<()>) -> Vec<String> {
    let res = reqwest::get(&sse_url).await.unwrap();
    assert_eq!(res.status(), 200);
    let mut opened = Some(opened);
    let mut collected = Vec::new();
    let mut buf = String::new();
    use futures::StreamExt;
    let mut stream = res.bytes_stream();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if let Some(Ok(chunk)) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            // SSE frames are newline-delimited; a chunk boundary can split
            // a frame, so only consume complete lines.
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].to_string();
                buf.drain(..pos + 1);
                if let Some(data) = line.strip_prefix("data:") {
                    collected.push(data.trim().to_string());
                }
            }
        }
        if opened.is_some()
            && !collected.is_empty()
            && let Some(tx) = opened.take()
        {
            let _ = tx.send(());
        }
        if collected.iter().any(|c| c.contains("RunFinished")) {
            break;
        }
    }
    collected
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn all_subscribers_receive_the_full_turn() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text(MARKER)])),
    });
    let (base, _server) = spawn_server(provider).await;
    let http = reqwest::Client::new();

    let session_id: String = http
        .post(format!("{base}/sessions"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // All subscribers attach BEFORE the turn fires.
    let mut subscribers = Vec::new();
    let mut opened_rxs = Vec::new();
    for _ in 0..SUBSCRIBERS {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        opened_rxs.push(rx);
        let url = format!("{base}/sessions/{session_id}/events");
        subscribers.push(tokio::spawn(subscribe(url, tx)));
    }
    for rx in opened_rxs {
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("subscriber never opened")
            .expect("open signal dropped");
    }

    let res = http
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({ "prompt": "go" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);

    // Fan-out contract: EVERY subscriber independently observes the whole
    // turn, through its own stream, up to the RunFinished frame.
    for (i, sub) in subscribers.into_iter().enumerate() {
        let collected = tokio::time::timeout(Duration::from_secs(15), sub)
            .await
            .expect("subscriber did not finish in time")
            .expect("subscriber task panicked");
        assert!(
            collected.iter().any(|c| c.contains(MARKER)),
            "subscriber {i} missed the assistant payload; got {:?}",
            &collected[..collected.len().min(12)]
        );
        assert!(
            collected.iter().any(|c| c.contains("RunFinished")),
            "subscriber {i} never saw the terminal frame"
        );
    }
}
