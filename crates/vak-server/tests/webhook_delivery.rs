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

type Captured = Arc<Mutex<Vec<(Option<String>, serde_json::Value)>>>;

/// Records (authorization header, body) for every POST. With `fail_first`,
/// the first `n` requests answer 503 before succeeding — exercising the
/// delivery retry/backoff path.
async fn spawn_receiver_failing(fail_first: u32) -> (String, Captured, Arc<Mutex<u32>>) {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(0u32));
    let cap = captured.clone();
    let count = seen.clone();
    let app = axum::Router::new().route(
        "/hook",
        axum::routing::post(
            move |headers: axum::http::HeaderMap,
                  axum::Json(body): axum::Json<serde_json::Value>| async move {
                let auth = headers
                    .get(axum::http::header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .map(String::from);
                cap.lock().unwrap().push((auth, body));
                let mut n = count.lock().unwrap();
                *n += 1;
                if *n <= fail_first {
                    axum::http::StatusCode::BAD_GATEWAY
                } else {
                    axum::http::StatusCode::OK
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), captured, seen)
}

async fn spawn_receiver() -> (String, Captured) {
    let (base, captured, _seen) = spawn_receiver_failing(0).await;
    (base, captured)
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
        assert!(out.status.success(), "git {args:?} failed");
    };
    run(&["init", "-q"]);
    std::fs::write(cwd.join("README.md"), "seed\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "seed"]);
}

struct Gateway {
    base: String,
    token: String,
    cwd: std::path::PathBuf,
    _server: tokio::task::JoinHandle<()>,
}

/// Gateway enabled purely through trusted project config — exercises the
/// `[gateway]` config path end to end.
async fn spawn_with_config(provider: Arc<dyn Provider>, gateway_toml: &str) -> Gateway {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let project = cwd.join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("config.toml"), gateway_toml).unwrap();

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_provider_instance(provider);
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with(core, false);
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Gateway {
        base: format!("http://{addr}"),
        token,
        cwd,
        _server: server,
    }
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

async fn run_nightly(gw: &Gateway) -> () {
    let client = client_with(&gw.token);
    git_seed(&gw.cwd);

    client
        .post(format!("{}/tasks", gw.base))
        .json(&serde_json::json!({
            "name": "nightly",
            "prompt": "check the build",
            "interval_secs": 3600,
            "deliver_to": "webhook:ci"
        }))
        .send()
        .await
        .unwrap();
    let tasks: serde_json::Value = client
        .get(format!("{}/tasks", gw.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = tasks["tasks"][0]["id"].as_str().unwrap().to_string();
    client
        .post(format!("{}/tasks/{tid}/run-now", gw.base))
        .send()
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webhook_delivery_posts_run_output() {
    let (rx_base, captured) = spawn_receiver().await;
    let toml = format!(
        "[gateway]\nenabled = true\n[gateway.outbound.webhooks.ci]\nurl = \"{rx_base}/hook\"\n"
    );
    let gw = spawn_with_config(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![text("built ok")])),
        }),
        &toml,
    )
    .await;
    run_nightly(&gw).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if !captured.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let cap = captured.lock().unwrap();
    assert_eq!(cap.len(), 1, "exactly one delivery expected");
    let (auth, body) = &cap[0];
    assert!(auth.is_none(), "no token_env configured: must post bare");
    let text = body["text"].as_str().unwrap_or_default();
    assert!(text.contains("routine 'nightly' finished"), "{body}");
    assert!(text.contains("built ok"), "real answer delivered: {body}");
    assert_eq!(body["delivery"]["kind"], "task_summary");
    assert_eq!(body["delivery"]["fallback_markdown"], body["text"]);
    assert_eq!(body["job_id"], body["delivery"]["job_id"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webhook_missing_token_fails_closed() {
    let (rx_base, captured) = spawn_receiver().await;
    let toml = format!(
        "[gateway]\nenabled = true\n[gateway.outbound.webhooks.ci]\nurl = \"{rx_base}/hook\"\ntoken_env = \"GATEWAY_TEST_UNSET_TOKEN\"\n"
    );
    let gw = spawn_with_config(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![text("secret output")])),
        }),
        &toml,
    )
    .await;

    // Sanity: the credential really is absent from this environment.
    assert!(std::env::var("GATEWAY_TEST_UNSET_TOKEN").is_err());

    run_nightly(&gw).await;
    let client = client_with(&gw.token);

    // The run itself still completes and records its answer; only the
    // delivery is withheld.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "task never recorded a summary"
        );
        let t: serde_json::Value = client
            .get(format!("{}/tasks", gw.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if t["tasks"][0]["last_summary"] == "secret output" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        captured.lock().unwrap().is_empty(),
        "missing credential must fail closed: nothing posted"
    );
    let outbox = gw.cwd.join("home/delivery/jobs");
    let pending = std::fs::read_dir(&outbox)
        .unwrap()
        .filter_map(Result::ok)
        .find_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .expect("failed delivery remains in the durable outbox");
    assert!(pending.contains("\"state\":\"pending\""), "{pending}");
    assert!(
        pending.contains("secret output"),
        "exact output survives: {pending}"
    );
    let inbox = std::fs::read_to_string(gw.cwd.join("home/inbox.jsonl")).unwrap();
    assert!(
        inbox.contains("secret output"),
        "inbox is recorded before push"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn webhook_retries_transient_5xx_and_succeeds() {
    // One 503, then success: delivery must survive the blip.
    let (rx_base, captured, seen) = spawn_receiver_failing(1).await;
    let toml = format!(
        "[gateway]\nenabled = true\n[gateway.outbound.webhooks.ci]\nurl = \"{rx_base}/hook\"\n"
    );
    let gw = spawn_with_config(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![text("retry built ok")])),
        }),
        &toml,
    )
    .await;
    run_nightly(&gw).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "delivery never landed despite retries"
        );
        let n = *seen.lock().unwrap();
        if n >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    {
        let cap = captured.lock().unwrap();
        // This receiver records EVERY post including the one that answered
        // 503, so two captures = attempt(503) + retry(200). Exactly one
        // retry happened and it carried the same payload.
        assert_eq!(cap.len(), 2, "one failed attempt then one successful retry");
        assert_eq!(cap[0].1, cap[1].1, "retry reuses the identical payload");
    }

    // A permanently failing receiver (all attempts 5xx) reports failure
    // instead of hanging or pretending success.
    let (rx_base2, _captured2, seen2) = spawn_receiver_failing(u32::MAX).await;
    let toml2 = format!(
        "[gateway]\nenabled = true\n[gateway.outbound.webhooks.ci]\nurl = \"{rx_base2}/hook\"\n"
    );
    let gw2 = spawn_with_config(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![text("never delivered")])),
        }),
        &toml2,
    )
    .await;
    run_nightly(&gw2).await;
    let deadline2 = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        assert!(
            std::time::Instant::now() < deadline2,
            "webhook exhausted retries without giving up"
        );
        if *seen2.lock().unwrap() >= 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
}
