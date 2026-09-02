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
        stop_reason: vak_llm::types::StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

/// Spawn the secured stack exactly like `serve --gateway`: bearer token +
/// forced gateway enable, independent of project config.
async fn spawn_gateway(
    provider: Arc<dyn Provider>,
) -> (
    String,
    String,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    // Hermetic against the developer's global config (e.g. reflection=true):
    // pin learning flags off for deterministic scripted flows.
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let _ = std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n[gateway]\nchat_allowlist_open = true\n",
    );
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    // Gateway turns run unattended with AutoDeny; give the fixture bash
    // execution so scripted tool flows behave like an interactive session.
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    // Pin the REAL brokered-tool worker instead of `current_exe()`: under
    // `cargo test` the latter is the test harness itself, which cannot speak
    // the broker protocol and makes brokered bash flake (or hard-fail) under
    // CPU contention. Mirrors the established fixture pattern in
    // scheduler_personal_os.rs / inbox_endpoints.rs / vak-tool-worker.rs.
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    core.set_provider_instance(provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with(core, true);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), token, cwd, handle)
}

/// Scheduler-free variant (gateway_router): dropping it releases session
/// locks immediately, like a real process exit.
async fn spawn_gateway_bare(
    provider: Arc<dyn Provider>,
) -> (String, std::path::PathBuf, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    // Hermetic against the developer's global config (e.g. reflection=true):
    // pin learning flags off for deterministic scripted flows.
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let _ = std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n[gateway]\nchat_allowlist_open = true\n",
    );
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_provider_instance(provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::gateway_router(core);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), cwd, handle)
}

async fn spawn_plain(provider: Arc<dyn Provider>) -> (String, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(dir.path().join("home"));
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

fn client_with(token: &str) -> reqwest::Client {
    reqwest::ClientBuilder::new()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
            h
        })
        .build()
        .unwrap()
}

async fn inbound(
    client: &reqwest::Client,
    base: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    client
        .post(format!("{base}/gateway/inbound"))
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbound_wait_roundtrip_reuses_binding() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("gateway hello"),
            text("second reply"),
        ])),
    });
    let (base, token, home, _server) = spawn_gateway(provider).await;
    let client = client_with(&token);
    let msg = |text: &str| {
        serde_json::json!({
            "surface": "webhook",
            "chat": "ci",
            "sender": "bot",
            "text": text,
            "wait": true
        })
    };

    // Auth middleware covers gateway routes too.
    let anon = reqwest::Client::new();
    let res = anon
        .post(format!("{base}/gateway/inbound"))
        .json(&msg("hi"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    let res = inbound(&client, &base, msg("hello from chat")).await;
    assert_eq!(res.status(), 200, "wait roundtrip must complete");
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "completed");
    assert_eq!(body["text"], "gateway hello");
    let sid = body["session_id"].as_str().unwrap().to_string();
    assert!(!sid.is_empty());

    // Binding table knows the route; ledger holds the exchange.
    let status: serde_json::Value = client
        .get(format!("{base}/gateway/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["enabled"], true);
    let bindings = status["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["target"], "webhook:ci");
    assert_eq!(bindings[0]["session_id"], sid.as_str());

    let raw = std::fs::read_to_string(home.join("home/gateway/bindings.json")).unwrap();
    assert!(raw.contains("webhook:ci") && raw.contains(&sid));

    // Second message resumes the SAME session.
    let res = inbound(&client, &base, msg("again")).await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["text"], "second reply");
    assert_eq!(body["session_id"], sid.as_str());

    let t: serde_json::Value = client
        .get(format!("{base}/sessions/{sid}/transcript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(t["count"].as_u64(), Some(4));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn telegram_reply_is_an_ordered_multi_message_packet() {
    let answer = format!("# Long result\n\n{}", "🧪 result line\n".repeat(700));
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text(&answer)])),
    });
    let (base, token, _home, _server) = spawn_gateway(provider).await;
    let client = client_with(&token);
    let response = inbound(
        &client,
        &base,
        serde_json::json!({
            "surface": "telegram",
            "chat": "42",
            "sender": "tester",
            "text": "give me the full result",
            "wait": true
        }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["delivery"]["fallback_markdown"], answer);
    let chunks = body["delivery"]["chunks"].as_array().unwrap();
    assert!(
        chunks.len() >= 3,
        "long answer must become multiple messages"
    );
    assert!(chunks.iter().all(|chunk| {
        chunk
            .as_str()
            .is_some_and(|text| text.chars().count() <= 4000)
    }));
    assert!(chunks.iter().all(|chunk| {
        let text = chunk.as_str().unwrap_or_default();
        text.matches('<').count() == text.matches('>').count()
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unbind_removes_route_and_404s_after() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("only")])),
    });
    let (base, token, _home, _server) = spawn_gateway(provider).await;
    let client = client_with(&token);

    let res = inbound(
        &client,
        &base,
        serde_json::json!({"surface":"log","chat":"ops","text":"hi","wait":true}),
    )
    .await;
    assert_eq!(res.status(), 200);

    let res = client
        .delete(format!(
            "{base}/gateway/bindings/{}",
            urlencoding_escape("log:ops")
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let status: serde_json::Value = client
        .get(format!("{base}/gateway/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["bindings"].as_array().unwrap().len(), 0);

    let res = client
        .delete(format!(
            "{base}/gateway/bindings/{}",
            urlencoding_escape("log:ops")
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bindings_survive_process_restart() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("first life")])),
    });
    let (base, cwd, server) = spawn_gateway_bare(provider).await;
    let client = reqwest::Client::new();

    let res = inbound(
        &client,
        &base,
        serde_json::json!({"surface":"telegram","chat":"48211","text":"hi","wait":true}),
    )
    .await;
    assert_eq!(res.status(), 200);
    let sid: String = res.json::<serde_json::Value>().await.unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Simulate a process restart: kill the first runtime, boot a fresh
    // router over the SAME sessions home. The persisted binding must route
    // to the same ledger, reattached from disk.
    server.abort();
    let _ = server.await;

    vak_config::paths::isolate_home_for_tests();
    let core2 = Core::new(cwd.clone()).unwrap();
    core2.set_sessions_home(cwd.join("home"));
    core2.set_provider_instance(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("reborn")])),
    }));
    let listener2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let (app2, token2) = vak_server::secured_router_with(core2, true);
    tokio::spawn(async move {
        axum::serve(listener2, app2).await.unwrap();
    });
    let base2 = format!("http://{addr2}");
    let client2 = client_with(&token2);

    // No handles live in process #2: the binding must reopen the ledger.
    let status: serde_json::Value = client2
        .get(format!("{base2}/gateway/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["bindings"][0]["session_id"], sid.as_str());

    let res = inbound(
        &client2,
        &base2,
        serde_json::json!({"surface":"telegram","chat":"48211","text":"still there?","wait":true}),
    )
    .await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["session_id"],
        sid.as_str(),
        "same session after restart"
    );
    assert_eq!(body["text"], "reborn");

    let t: serde_json::Value = client2
        .get(format!("{base2}/sessions/{sid}/transcript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(t["count"].as_u64(), Some(4), "history continued");
}

fn urlencoding_escape(s: &str) -> String {
    s.replace(':', "%3A")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn busy_message_is_steered_not_dropped() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            tool_call("t1", "bash", serde_json::json!({"command": "sleep 8"})),
            text("done two"),
        ])),
    });
    let (base, token, _home, _server) = spawn_gateway(provider).await;
    let client = client_with(&token);

    // Start a run that occupies the session for a while.
    let res = inbound(
        &client,
        &base,
        serde_json::json!({"surface":"webhook","chat":"ci","text":"first msg"}),
    )
    .await;
    assert_eq!(res.status(), 202);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "started");
    let sid = {
        let status: serde_json::Value = client
            .get(format!("{base}/gateway/status"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        status["bindings"][0]["session_id"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // Wait until the turn actually owns the ledger (transcript answers
    // "run in progress" while busy), then send a follow-up: it must be
    // queued as logged steering, never rejected.
    let deadline_busy = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            std::time::Instant::now() < deadline_busy,
            "never became busy"
        );
        if let Ok(res) = client
            .get(format!("{base}/sessions/{sid}/transcript"))
            .send()
            .await
            && res.status() == reqwest::StatusCode::OK
        {
            let t: serde_json::Value = res.json().await.unwrap();
            let raw = serde_json::to_string(&t).unwrap_or_default();
            if raw.contains("run in progress") {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let res = inbound(
        &client,
        &base,
        serde_json::json!({
            "surface": "webhook",
            "chat": "ci",
            "sender": "@alice",
            "text": "second msg",
            "attachments": [{"mime": "image/png", "data": "TUVPT1c="}],
        }),
    )
    .await;
    assert_eq!(res.status(), 202);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "steering_queued");

    // Eventually every message is visible on the chain and the run ends:
    // user, assistant(tool), user(result), user(steered), assistant(final).
    // The queued message keeps its image blocks AND its sender attribution
    // — busy queueing must never degrade the payload to bare text.
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut raw = String::new();
    while std::time::Instant::now() < deadline {
        if let Ok(res) = client
            .get(format!("{base}/sessions/{sid}/transcript"))
            .send()
            .await
            && res.status() == reqwest::StatusCode::OK
        {
            let t: serde_json::Value = res.json().await.unwrap();
            if t.get("error").is_none() {
                raw = serde_json::to_string(&t).unwrap();
                if raw.contains("second msg") && raw.contains("done two") {
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(raw.contains("first msg"), "prompt missing: {raw}");
    assert!(
        raw.contains("[from @alice] second msg"),
        "sender attributed on queued turn: {raw}"
    );
    assert!(
        raw.contains("TUVPT1c="),
        "image attachment survives busy queueing: {raw}"
    );
    assert!(raw.contains("done two"), "run must complete after steering");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gateway_disabled_by_default_returns_conflict() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("nope")])),
    });
    let (base, _server) = spawn_plain(provider).await;
    let client = reqwest::Client::new(); // plain router has no auth gate

    let res = inbound(
        &client,
        &base,
        serde_json::json!({"surface":"webhook","chat":"x","text":"hi"}),
    )
    .await;
    assert_eq!(res.status(), 409);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("gateway disabled")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn empty_chat_allowlist_denies_by_default() {
    // 0c-02: an empty chat_allowlist must fail closed, not open — no
    // `chat_allowlist_open = true` here, unlike the shared test helpers.
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("should never run")])),
    });
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let _ = std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    );
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_provider_instance(provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::gateway_router(core);
    let _server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let res = inbound(
        &client,
        &base,
        serde_json::json!({"surface":"telegram","chat":"999","sender":"999","text":"hi"}),
    )
    .await;
    assert_eq!(res.status(), 403);
    let body: serde_json::Value = res.json().await.unwrap();
    // docs/design/34: an unknown chat becomes a reviewable pending entry
    // instead of a flat rejection with a config-editing hint.
    assert_eq!(body["state"].as_str(), Some("pending"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cron_task_delivers_summary_to_log_surface() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("task finished cleanly")])),
    });
    let (base, token, cwd, _server) = spawn_gateway(provider).await;
    let client = client_with(&token);
    git_seed(&cwd);

    // Create a routine with a delivery target and fire it immediately.
    let res = client
        .post(format!("{base}/tasks"))
        .json(&serde_json::json!({
            "name": "nightly",
            "prompt": "check the build",
            "interval_secs": 3600,
            "deliver_to": "log:ops"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "task create failed");

    // Malformed targets are rejected at creation time.
    let res = client
        .post(format!("{base}/tasks"))
        .json(&serde_json::json!({
            "name": "bad",
            "prompt": "x",
            "interval_secs": 3600,
            "deliver_to": "nologseparator"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);

    let tasks: serde_json::Value = client
        .get(format!("{base}/tasks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = tasks["tasks"][0]["id"].as_str().unwrap().to_string();

    let res = client
        .post(format!("{base}/tasks/{tid}/run-now"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);

    // The summary lands in the log surface's delivery journal.
    let path = cwd.join("home/gateway/deliveries.jsonl");
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut found = false;
    while std::time::Instant::now() < deadline {
        if let Ok(raw) = std::fs::read_to_string(&path)
            && raw.contains("routine 'nightly' finished")
            && raw.contains("task finished cleanly")
        {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(found, "delivery journal never received the routine summary");
}

fn git_seed(cwd: &std::path::Path) {
    let run = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-q"]);
    std::fs::write(cwd.join("README.md"), "seed\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "seed"]);
}
