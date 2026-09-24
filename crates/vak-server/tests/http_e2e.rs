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
        response_id: None,
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
        response_id: None,
    }
}

/// Pin `VAK_HOME` to an empty, process-stable tempdir so `spawn_server`'s
/// `Core::new` does not inherit the operator's real user-global config — which
/// here is `permission_mode = fullaccess` with `approval_mode` != `ask` and
/// would, via `refresh_persisted_preferences`, clobber the test's runtime
/// `set_permission_mode` pin and auto-approve the `bash` -> `Ask` arm so no
/// `ApprovalRequested` is ever published. With the global layer absent,
/// `Config::default()` applies (`approval_mode = Ask`, vak-config lib.rs:891)
/// and the per-test `mode` pin plus `WorkspaceWrite` default restore the
/// intended human-in-the-loop path. Installed once via `Once`; harmless to the
/// sibling tests because the temp never holds a global config.
fn isolate_global_config() {
    vak_config::paths::isolate_home_for_tests();
}

async fn spawn_server(
    provider: Arc<dyn Provider>,
    mode: vak_config::PermissionMode,
) -> (String, tokio::task::JoinHandle<()>) {
    isolate_global_config();
    let dir = tempfile::tempdir().unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(dir.path().to_path_buf()).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(mode);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
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
    let body_health: serde_json::Value = health.json().await.unwrap();
    assert_eq!(body_health["status"], "ok");

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

    // Phase R forensics: receipts must be served over HTTP with leg
    // attribution (the desktop drill-down panel reads exactly this).
    let receipts: serde_json::Value = client
        .get(format!("{base}/sessions/{session_id}/receipts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let list = receipts.as_array().expect("receipts array");
    assert!(!list.is_empty(), "a settled run must leave receipts");
    let r = &list[list.len() - 1];
    assert_eq!(r["purpose"], "execute");
    assert_eq!(r["provider"], "scripted");
    // The receipt stamps the CONTRACT leg (what was dispatched), which is
    // the core's effective model -- not the mock's self-reported id.
    let expected_model = body_health["model"].as_str().unwrap().to_string();
    assert_eq!(r["model"], expected_model);
    assert_eq!(r["winning_attempt"], 0);
    let attempts = r["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["settlement"], "ok");
    assert_eq!(attempts[0]["reason"], "initial");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn operations_center_is_a_real_evidence_projection() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::ReadOnly).await;
    let body: serde_json::Value = reqwest::get(format!("{base}/ops/center"))
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(body["server"]["pid"].as_u64().is_some());
    assert!(body["server"]["uptime_secs"].as_u64().is_some());
    assert!(body["health"]["checks"].is_array());
    assert!(body["pool"]["entries"].is_array());
    assert!(body["runs"].is_array());
    assert!(body["outbox"]["records"].is_array());
    assert!(body["incidents"].is_array());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn secured_operations_center_uses_the_bound_port() {
    let dir = tempfile::tempdir().unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(dir.path().to_path_buf()).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with_port(core, false, addr.port());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::new();

    let unauthenticated = client
        .get(format!("http://{addr}/ops/center"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), reqwest::StatusCode::UNAUTHORIZED);

    let body: serde_json::Value = client
        .get(format!("http://{addr}/ops/center"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["ops_port"], addr.port());
    assert_eq!(body["server"]["pid"], std::process::id());
    assert!(body["server"]["uptime_secs"].as_u64().is_some());
    assert_eq!(body["services"]["gateway_healthy"], true);

    let outbox: serde_json::Value = client
        .get(format!("http://{addr}/ops/outbox"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(outbox["records"].is_array());

    let replay_missing = client
        .post(format!("http://{addr}/ops/outbox/not-a-real-job/replay"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(replay_missing.status(), reqwest::StatusCode::CONFLICT);

    handle.abort();
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
async fn config_endpoint_exposes_route_policy() {
    // Phase R: the desktop Settings panel reads routing policy from
    // /config; the section must exist with resolved defaults.
    struct Empty;
    #[async_trait::async_trait]
    impl Provider for Empty {
        fn name(&self) -> &str {
            "empty"
        }
        async fn stream(
            &self,
            _r: ChatRequest,
            _c: CancellationToken,
        ) -> Result<EventStream, LlmError> {
            Err(LlmError::Network("unused".into()))
        }
    }
    let (base, _server) =
        spawn_server(Arc::new(Empty), vak_config::PermissionMode::FullAccess).await;
    let client = reqwest::Client::new();
    let cfg: serde_json::Value = client
        .get(format!("{base}/config"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let route = cfg
        .get("route")
        .expect("/config must carry the route section");
    assert_eq!(route["objective"], "auto");
    assert_eq!(route["max_fallbacks"], 4);
    assert!(route["fallback_models"].is_array());
    assert!(route["quality_hints"].is_array());
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
            let (mut sink, rx) = stream::channel(8);
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
                // `cancel_run` no longer synthesizes its own `RunFinished`
                // (it raced the real one); the terminal event now comes
                // from the run itself unwinding as `TurnOutcome::Aborted`,
                // which `run_prompt`/`http_settle` summarize as the internal
                // sentinel "aborted". The wire's `RunFinished` carries a
                // typed `outcome` and a human `message`, never that raw
                // internal summary string (`vak-server/src/
                // client_events.rs`) -- a cancelled run's outcome is
                // `"Stopped"`.
                if text.contains("RunFinished") && text.contains("\"Stopped\"") {
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
    assert!(saw, "RunFinished(aborted) must be observed after /cancel");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_endpoints_scope_and_wire() {
    let (base, _server) = spawn_server(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![text("no children")])),
        }),
        vak_config::PermissionMode::WorkspaceWrite,
    )
    .await;

    // Mint a session.
    let res = reqwest::Client::new()
        .post(format!("{base}/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let sid: serde_json::Value = res.json().await.unwrap();
    let sid = sid["session_id"].as_str().unwrap().to_string();

    // No live children.
    let res = reqwest::Client::new()
        .get(format!("{base}/sessions/{sid}/workers"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["workers"], serde_json::json!([]));

    // Steering/stopping a child this session does not own is 404 — never a
    // cross-session capability leak, and never a silent no-op.
    let client = reqwest::Client::new();
    for path in [
        format!("/sessions/{sid}/workers/child-nope/steer"),
        format!("/sessions/{sid}/workers/child-nope/stop"),
    ] {
        let res = client
            .post(format!("{base}{path}"))
            .json(&serde_json::json!({"text": "hi"}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 404, "{path}");
    }
}

/// Responds after a fixed delay, so a caller has a window to observe the
/// session as busy before the leg settles.
struct DelayedThenScripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
    delay: Duration,
}

#[async_trait::async_trait]
impl Provider for DelayedThenScripted {
    fn name(&self) -> &str {
        "delayed"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        tokio::time::sleep(self.delay).await;
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

async fn poll_transcript_contains(
    client: &reqwest::Client,
    base: &str,
    sid: &str,
    needle: &str,
    deadline: std::time::Instant,
) -> String {
    let mut raw = String::new();
    while std::time::Instant::now() < deadline {
        if let Ok(res) = client
            .get(format!("{base}/sessions/{sid}/transcript"))
            .send()
            .await
        {
            let t: serde_json::Value = res.json().await.unwrap_or_default();
            raw = serde_json::to_string(&t).unwrap_or_default();
            if raw.contains(needle) {
                return raw;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    raw
}

/// Finding 1: `/run` on a busy session used to return a bare 202 with the
/// prompt silently dropped (invariant 30 / docs/design/64, "Request
/// durability and delivery" — busy input is queued durably, never
/// discarded). It must now be queued and actually run as the chain's next
/// leg once the first run settles.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn busy_run_is_queued_and_runs_as_a_continuation_leg() {
    let provider = Arc::new(DelayedThenScripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("first done"),
            text("second done"),
        ])),
        delay: Duration::from_millis(400),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::FullAccess).await;
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

    let first = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "first", "request_id": "run-first"}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 202);
    let first_body: serde_json::Value = first.json().await.unwrap();
    assert_eq!(first_body["state"], "started");

    // The provider is still asleep, so this lands while busy.
    let second = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "second", "request_id": "run-second"}))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 202);
    let second_body: serde_json::Value = second.json().await.unwrap();
    assert_eq!(
        second_body["state"], "queued",
        "a busy /run must be queued, not silently dropped: {second_body}"
    );
    assert_eq!(second_body["request_id"], "run-second");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let raw = poll_transcript_contains(&client, &base, &session_id, "second done", deadline).await;
    assert!(raw.contains("first done"), "first leg missing: {raw}");
    assert!(
        raw.contains("second"),
        "queued prompt must reach the model as the next leg: {raw}"
    );
    assert!(
        raw.contains("second done"),
        "queued run must actually execute as a continuation leg: {raw}"
    );
}

/// Same fix, the `/steering` admission path: a steer that arrives while
/// busy must be queued and drained into a continuation leg — never
/// acknowledged and left in the queue with nothing left to drain it
/// (finding 1b).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn busy_steering_is_queued_and_runs_as_a_continuation_leg() {
    let provider = Arc::new(DelayedThenScripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("first done"),
            text("steered done"),
        ])),
        delay: Duration::from_millis(400),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::FullAccess).await;
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

    let run = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "first"}))
        .send()
        .await
        .unwrap();
    assert_eq!(run.status(), 202);

    let steer = client
        .post(format!("{base}/sessions/{session_id}/steering"))
        .json(&serde_json::json!({"text": "please continue with this"}))
        .send()
        .await
        .unwrap();
    assert_eq!(steer.status(), 202);
    let steer_body: serde_json::Value = steer.json().await.unwrap();
    assert_eq!(steer_body["state"], "steering_queued");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let raw = poll_transcript_contains(&client, &base, &session_id, "steered done", deadline).await;
    assert!(raw.contains("first done"), "first leg missing: {raw}");
    assert!(
        raw.contains("please continue with this"),
        "queued steering text must reach the model: {raw}"
    );
    assert!(
        raw.contains("steered done"),
        "queued steering must actually execute as a continuation leg: {raw}"
    );
}

/// Hangs on its first call (until cancelled), then answers normally on any
/// later call — models a run that gets stopped and immediately resent.
struct HungOnceThenScripted {
    hung_once: std::sync::atomic::AtomicBool,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for HungOnceThenScripted {
    fn name(&self) -> &str {
        "hung-once"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        if !self
            .hung_once
            .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let (mut sink, rx) = stream::channel(8);
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
            return Ok(rx);
        }
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

/// Finding 1c: `cancel_run` no longer synthesizes its own `RunFinished`, and
/// input arriving after the stop but before the run unwinds is queued and
/// runs as the next leg of the chain — a stop must never eat the resend
/// that follows it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_then_resend_runs_the_resend() {
    let provider = Arc::new(HungOnceThenScripted {
        hung_once: std::sync::atomic::AtomicBool::new(false),
        responses: Mutex::new(VecDeque::from(vec![text("resend done")])),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::FullAccess).await;
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

    client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "hang forever"}))
        .send()
        .await
        .unwrap();

    // Give the hung leg a moment to actually take the ledger.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let cancelled = client
        .post(format!("{base}/sessions/{session_id}/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled.status(), 202);

    // Resend immediately — this may land while the cancelled leg is still
    // unwinding (queued) or just after (started); both must eventually run.
    let resend = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "resend", "request_id": "resend-1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resend.status(), 202);
    let resend_body: serde_json::Value = resend.json().await.unwrap();
    assert!(
        matches!(
            resend_body["state"].as_str(),
            Some("started") | Some("queued")
        ),
        "resend must be admitted, not rejected: {resend_body}"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let raw = poll_transcript_contains(&client, &base, &session_id, "resend done", deadline).await;
    assert!(
        raw.contains("resend done"),
        "a resend right after stop must actually run: {raw}"
    );
}

/// Finding 2: `run_prompt` used to wait up to 2s on `handle.subscribed`
/// unconditionally, but `Notify`'s single permit only ever satisfies the
/// FIRST run — every later run paid the full timeout even with a client
/// already attached. With an SSE consumer already connected, `/run` must
/// return promptly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_wait_when_an_sse_subscriber_is_already_attached() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("fast reply")])),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::FullAccess).await;
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

    // Attach and confirm the stream is open before timing the run.
    let sse_url = format!("{base}/sessions/{session_id}/events");
    let (opened_tx, opened_rx) = tokio::sync::oneshot::channel::<()>();
    let mut opened_tx = Some(opened_tx);
    let _sse_task = tokio::spawn(async move {
        let res = reqwest::get(&sse_url).await.unwrap();
        use futures::StreamExt;
        let mut stream = res.bytes_stream();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(Ok(chunk)) = stream.next().await {
                let text = String::from_utf8_lossy(&chunk).into_owned();
                if text.contains("StreamOpened") {
                    if let Some(t) = opened_tx.take() {
                        let _ = t.send(());
                    }
                    break;
                }
            }
        }
    });
    let _ = tokio::time::timeout(Duration::from_secs(5), opened_rx).await;

    let start = std::time::Instant::now();
    let res = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "go"}))
        .send()
        .await
        .unwrap();
    let elapsed = start.elapsed();
    assert_eq!(res.status(), 202);
    assert!(
        elapsed < Duration::from_millis(1500),
        "an already-attached subscriber must skip the 2s attach wait entirely, took {elapsed:?}"
    );
}

/// Finding 4: validation must happen BEFORE any side effect — a rejected
/// request must never write a durable admission activity, insert into the
/// admissions set, or broadcast a synthesized `RunFinished` for a run that
/// never started.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_request_writes_no_durable_admission() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    });
    let (base, _server) = spawn_server(provider, vak_config::PermissionMode::FullAccess).await;
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

    // Goal mode explicitly refuses attachments; this must be rejected
    // before any admission is ever recorded.
    let res = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({
            "prompt": "do a thing",
            "request_id": "rejected-1",
            "goal": "finish the thing",
            "attachments": [{"mime": "image/png", "data": "AAAA"}],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    let transcript: serde_json::Value = client
        .get(format!("{base}/sessions/{session_id}/transcript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        transcript["count"].as_u64(),
        Some(0),
        "a rejected request must leave no durable entry behind: {transcript}"
    );

    // The same request_id must be admittable afterward — nothing was
    // parked in the in-memory admissions guard either.
    let retry = client
        .post(format!("{base}/sessions/{session_id}/run"))
        .json(&serde_json::json!({"prompt": "do a thing", "request_id": "rejected-1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(retry.status(), 202);
    let retry_body: serde_json::Value = retry.json().await.unwrap();
    assert_eq!(retry_body["state"], "started");
}
