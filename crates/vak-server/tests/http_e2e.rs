#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
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
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
    }
}

fn tool_call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

async fn spawn_server(
    provider: Arc<dyn Provider>,
    mode: vak_config::PermissionMode,
) -> (String, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(dir.path().to_path_buf()).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(mode);
    core.set_provider_instance(provider);
    // keep tempdir alive for the process lifetime of the test
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::router(core);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), handle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_lifecycle_run_events_transcript() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            tool_call("t1", "bash", serde_json::json!({"command": "echo served"})),
            text("all served"),
        ])),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::FullAccess).await;
    let client = reqwest::Client::new();

    // health
    let health = client.get(format!("{base}/health")).send().await.unwrap();
    assert_eq!(health.status(), 200);
    let body: serde_json::Value = health.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    // create session
    let res = client
        .post(format!("{base}/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let session_id: String = res.json::<serde_json::Value>().await.unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // subscribe to SSE BEFORE running so no events are missed
    let sse_url = format!("{base}/sessions/{session_id}/events");
    let (opened_tx, opened_rx) = tokio::sync::oneshot::channel::<()>();
    let mut opened_tx = Some(opened_tx);
    let sse_task = tokio::spawn(async move {
        let res = reqwest::get(&sse_url).await.unwrap();
        assert_eq!(res.status(), 200);
        let mut collected = Vec::new();
        let mut opened_done = false;
        use futures::StreamExt;
        let mut stream = res.bytes_stream();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(Ok(chunk)) = stream.next().await {
                let text = String::from_utf8_lossy(&chunk).into_owned();
                for line in text.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        collected.push(data.trim().to_string());
                    }
                }
            }
            if !opened_done && collected.iter().any(|c| c.contains("StreamOpened")) {
                opened_done = true;
                if let Some(t) = opened_tx.take() {
                    let _ = t.send(());
                }
                #[allow(unused_assignments)]
                {
                    // oneshot send consumes; guard against loop re-entry
                }
            }
            if collected
                .iter()
                .any(|c| c.contains("__done__") || c.contains("RunFinished"))
            {
                break;
            }
        }
        collected
    });

    // fire the run only once the event stream is confirmed open
    let _ = tokio::time::timeout(Duration::from_secs(5), opened_rx).await;

    // run a prompt
    let res = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "serve it"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);

    let events = tokio::time::timeout(Duration::from_secs(10), sse_task)
        .await
        .expect("sse timed out")
        .unwrap();

    // The transcript must show the full loop.
    let transcript: serde_json::Value = client
        .get(format!("{base}/sessions/{session_id}/transcript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(transcript["count"].as_u64(), Some(4)); // user, assistant(tool), user(result), assistant(final)
    assert!(
        serde_json::to_string(&transcript)
            .unwrap()
            .contains("served"),
        "transcript must contain the run"
    );

    // Events must include streamed text and the terminal marker.
    let joined = events.join("\n");
    assert!(joined.contains("TurnStart"), "events: {joined}");
    assert!(
        joined.contains("RunFinished") || joined.contains("__done__"),
        "terminal event missing: {joined}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_flow_resolves_over_http() {
    // First turn requests bash (workspace-write => ask); we approve over HTTP.
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            tool_call(
                "t1",
                "bash",
                serde_json::json!({"command": "echo approved-run"}),
            ),
            text("done after approval"),
        ])),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::WorkspaceWrite).await;
    let client = reqwest::Client::new();

    let session_id: String = client
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

    // SSE collector answers approvals inline so the run can proceed.
    let sse_session = session_id.clone();
    let sse_url = format!("{base}/sessions/{sse_session}/events");
    let answer_url_base = base.clone();
    let (opened_tx, opened_rx) = tokio::sync::oneshot::channel::<()>();
    let mut opened_tx = Some(opened_tx);
    let sse_task = tokio::spawn(async move {
        let res = reqwest::get(&sse_url).await.unwrap();
        use futures::StreamExt;
        let mut stream = res.bytes_stream();
        let mut approval_answered = false;
        let mut saw_finish = false;
        let mut opened_done = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(Ok(chunk)) = stream.next().await {
                let text = String::from_utf8_lossy(&chunk).into_owned();
                for line in text.lines() {
                    if let Some(data) = line.strip_prefix("data:")
                        && let Ok(v) = serde_json::from_str::<serde_json::Value>(data.trim())
                    {
                        if !opened_done && v["StreamOpened"].is_object() {
                            opened_done = true;
                            if let Some(t) = opened_tx.take() {
                                let _ = t.send(());
                            }
                        }
                        if !approval_answered && v["ApprovalRequested"]["id"].is_string() {
                            let rid = v["ApprovalRequested"]["id"].as_str().unwrap().to_string();
                            let _ = reqwest::Client::new()
                                .post(format!(
                                    "{answer_url_base}/sessions/{sse_session}/approvals/{rid}"
                                ))
                                .json(&serde_json::json!({"approve": true}))
                                .send()
                                .await;
                            approval_answered = true;
                        }
                        if v["RunFinished"].is_object() {
                            saw_finish = true;
                        }
                    }
                }
            }
            if saw_finish {
                break;
            }
        }
        (approval_answered, saw_finish)
    });

    // fire the run only once the event stream is confirmed open
    let _ = tokio::time::timeout(Duration::from_secs(5), opened_rx).await;
    client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "needs approval"}))
        .send()
        .await
        .unwrap();

    let (answered, saw_finish) = tokio::time::timeout(Duration::from_secs(10), sse_task)
        .await
        .expect("sse timed out")
        .unwrap();
    assert!(answered, "an approval request must have been published");
    assert!(saw_finish, "run must finish after inline approval");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mode_change_revokes_run_waiting_for_approval() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![tool_call(
            "t1",
            "bash",
            serde_json::json!({"command": "echo must-not-run"}),
        )])),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::WorkspaceWrite).await;
    let client = reqwest::Client::new();
    let session_id: String = client
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

    let _events = client
        .get(format!("{base}/sessions/{session_id}/events"))
        .send()
        .await
        .unwrap();
    let run = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "request approval"}))
        .send()
        .await
        .unwrap();
    assert_eq!(run.status(), 202);
    tokio::time::sleep(Duration::from_millis(250)).await;

    let switched = client
        .post(format!("{base}/config/mode"))
        .json(&serde_json::json!({"mode": "read-only"}))
        .send()
        .await
        .unwrap();
    assert_eq!(switched.status(), 200);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(std::time::Instant::now() < deadline, "run was not revoked");
        let transcript: serde_json::Value = client
            .get(format!("{base}/sessions/{session_id}/transcript"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if transcript.get("error").is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_endpoint_stops_a_running_session() {
    // A provider that hangs until cancelled — mirrors a stalled stream.
    struct Hung;
    #[async_trait::async_trait]
    impl Provider for Hung {
        fn name(&self) -> &str {
            "hung"
        }
        async fn stream(
            &self,
            _r: ChatRequest,
            cancel: CancellationToken,
        ) -> Result<EventStream, LlmError> {
            let (sink, rx) = stream::channel(8);
            sink.push(stream::StreamEvent::Start {
                partial: AssistantMessage::empty("m"),
            });
            tokio::spawn(async move {
                let _keep = sink;
                tokio::select! {
                    _ = cancel.cancelled() => {}
                    _ = std::future::pending::<()>() => {}
                }
            });
            Ok(rx)
        }
    }

    let (base, _server) =
        spawn_server(Arc::new(Hung), vak_config::PermissionMode::FullAccess).await;
    let client = reqwest::Client::new();

    let session_id: String = client
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

    // Wait for the stream-open marker, then start the run.
    let sse_url = format!("{base}/sessions/{session_id}/events");
    let opened = reqwest::get(&sse_url).await.unwrap();
    use futures::StreamExt;
    let mut events = opened.bytes_stream();
    let mut saw_cancelled = false;

    let reader = tokio::spawn(async move {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while std::time::Instant::now() < deadline {
            if let Some(Ok(chunk)) = events.next().await {
                let text = String::from_utf8_lossy(&chunk);
                if text.contains("RunFinished") && text.contains("cancelled") {
                    saw_cancelled = true;
                    break;
                }
            }
        }
        saw_cancelled
    });

    tokio::time::sleep(Duration::from_millis(150)).await;
    client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "hang forever"}))
        .send()
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;
    let res = client
        .post(format!("{base}/sessions/{session_id}/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);

    let saw = tokio::time::timeout(Duration::from_secs(9), reader)
        .await
        .expect("reader timed out")
        .unwrap();
    assert!(saw, "RunFinished(cancelled) must be observed after /cancel");
}
