#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
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
        stop_reason: vak_llm::types::StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

struct Gateway {
    base: String,
    token: String,
    home: PathBuf,
    _server: tokio::task::JoinHandle<()>,
}

/// Gateway enabled purely through trusted project config. The agent modes
/// (permission/approval) are pinned in the project config by [`config`] so an
/// ambient global `fullaccess` profile cannot silently collapse the Ask gate
/// these forwarded-approval tests assert on, or hang the deny test on a
/// blocking approver. Project overrides global per the layered-config merge;
/// note a runtime `set_approval_mode` would be clobbered by the control-plane
/// refresh, so the modes live in the persisted config instead.
async fn spawn_with_config(provider: Arc<dyn Provider>, gateway_toml: &str) -> Gateway {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let project = cwd.join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(project.join("config.toml"), gateway_toml).unwrap();

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    let home = dir.path().join("home");
    core.set_sessions_home(home.clone());
    core.set_provider_instance(provider);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
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
        home,
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

fn config(toml_body: &str) -> String {
    config_with_modes(toml_body, "workspace-write", "ask")
}

fn config_with_modes(toml_body: &str, permission_mode: &str, approval_mode: &str) -> String {
    format!(
        "permission_mode = \"{permission_mode}\"\n\
         approval_mode = \"{approval_mode}\"\n\
         [gateway]\n\
         enabled = true\n\
         chat_allowlist_open = true\n\
         {toml_body}\n\n\
         [memory]\n\
         reflection = false\n"
    )
}

async fn inbound(
    client: &reqwest::Client,
    base: &str,
    surface: &str,
    chat: &str,
    text: &str,
    wait: bool,
) -> reqwest::Response {
    client
        .post(format!("{base}/gateway/inbound"))
        .json(&serde_json::json!({
            "surface": surface, "chat": chat, "text": text, "wait": wait
        }))
        .send()
        .await
        .unwrap()
}

fn deliveries(home: &Path) -> PathBuf {
    home.join("gateway/deliveries.jsonl")
}

async fn wait_for_delivery(home: &Path, needle: &str, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if let Ok(raw) = std::fs::read_to_string(deliveries(home))
            && raw.contains(needle)
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forwarded_gate_resolves_from_approver_chat() {
    let gw = spawn_with_config(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![
                tool_call(
                    "t1",
                    "bash",
                    serde_json::json!({"command": "echo approved-run"}),
                ),
                text("done after yes"),
            ])),
        }),
        &config("approvals = \"forward\"\napprover = \"log:ops\"\n"),
    )
    .await;
    let client = client_with(&gw.token);

    // Kick off an unattended turn that needs escalation.
    let res = inbound(&client, &gw.base, "webhook", "ci", "run it", false).await;
    assert_eq!(res.status(), 202);

    // The gate lands on the approver surface's delivery transport.
    let announced = wait_for_delivery(&gw.home, "Approval requested", 15).await;
    if !announced {
        let status: serde_json::Value = client
            .get(format!("{}/gateway/status", gw.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        eprintln!("DBG status={status}");
        if let Ok(raw) = std::fs::read_to_string(deliveries(&gw.home)) {
            eprintln!("DBG deliveries={raw}");
        } else {
            eprintln!("DBG deliveries=<missing> home={:?}", gw.home);
        }
    }
    assert!(announced, "gate must be announced on the approver surface");

    // Answering from the approver chat resolves it.
    let res = inbound(&client, &gw.base, "log", "ops", "yes", false).await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "approval_resolved");
    assert_eq!(body["approved"], true);

    // The tool then really runs and the turn completes.
    let sid: String = {
        let status: serde_json::Value = client
            .get(format!("{}/gateway/status", gw.base))
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
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut raw = String::new();
    while std::time::Instant::now() < deadline {
        if let Ok(res) = client
            .get(format!("{}/sessions/{sid}/transcript", gw.base))
            .send()
            .await
            && res.status() == reqwest::StatusCode::OK
        {
            let t: serde_json::Value = res.json().await.unwrap();
            if t.get("error").is_none() {
                raw = serde_json::to_string(&t).unwrap();
                if raw.contains("approved-run") && raw.contains("done after yes") {
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(
        raw.contains("approved-run"),
        "bash ran after approval: {raw}"
    );
    assert!(raw.contains("done after yes"), "turn completed: {raw}");

    // Pending count back to zero.
    let status: serde_json::Value = client
        .get(format!("{}/gateway/status", gw.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["approvals"]["pending"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unanswered_gate_times_out_and_fails_closed() {
    let gw = spawn_with_config(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![
                tool_call(
                    "t1",
                    "bash",
                    serde_json::json!({"command": "echo never-runs"}),
                ),
                text("bash denied: moving on without it"),
            ])),
        }),
        &config("approvals = \"forward\"\napprover = \"log:ops\"\napproval_timeout_secs = 5\n"),
    )
    .await;
    let client = client_with(&gw.token);

    let res = inbound(&client, &gw.base, "webhook", "ci", "go", false).await;
    assert_eq!(res.status(), 202);
    assert!(
        wait_for_delivery(&gw.home, "Approval requested", 15).await,
        "gate announced"
    );

    // Let the 5s window lapse; the late reply must resolve nothing.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let res = inbound(&client, &gw.base, "log", "ops", "yes", false).await;
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "no_pending_approvals");

    // The turn finishes anyway; the tool never executed.
    let sid: String = {
        let status: serde_json::Value = client
            .get(format!("{}/gateway/status", gw.base))
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
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut raw = String::new();
    while std::time::Instant::now() < deadline {
        if let Ok(res) = client
            .get(format!("{}/sessions/{sid}/transcript", gw.base))
            .send()
            .await
            && res.status() == reqwest::StatusCode::OK
        {
            let t: serde_json::Value = res.json().await.unwrap();
            if t.get("error").is_none() {
                raw = serde_json::to_string(&t).unwrap();
                if raw.contains("moving on without it") {
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(raw.contains("moving on without it"), "run finished: {raw}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn addressed_yes_resolves_only_that_gate_and_reports_it() {
    let gw = spawn_with_config(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![
                tool_call(
                    "t1",
                    "bash",
                    serde_json::json!({"command": "echo addressed-run"}),
                ),
                text("done after addressed yes"),
            ])),
        }),
        &config("approvals = \"forward\"\napprover = \"log:ops\"\n"),
    )
    .await;
    let client = client_with(&gw.token);

    let res = inbound(&client, &gw.base, "webhook", "ci", "run it", false).await;
    assert_eq!(res.status(), 202);
    assert!(
        wait_for_delivery(&gw.home, "Approval requested", 15).await,
        "gate announced"
    );
    let raw = std::fs::read_to_string(deliveries(&gw.home)).unwrap();
    let announcement = raw
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find_map(|line| line["text"].as_str().map(str::to_string))
        .unwrap();
    let start = announcement.find('[').unwrap() + 1;
    let short = announcement[start..start + 8].to_string();

    // A verdict addressed to a nonexistent gate must leave the live gate
    // untouched.
    let res = inbound(&client, &gw.base, "log", "ops", "yes 00000000", false).await;
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "no_pending_approvals");
    let status: serde_json::Value = client
        .get(format!("{}/gateway/status", gw.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        status["approvals"]["pending"], 1,
        "live gate survives a mis-addressed yes"
    );

    // Addressing the real gate resolves exactly it, and the reply says so.
    let res = inbound(
        &client,
        &gw.base,
        "log",
        "ops",
        &format!("yes {short}"),
        false,
    )
    .await;
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "approval_resolved");
    assert_eq!(body["approved"], true);
    assert!(
        body["gate"].as_str().unwrap().starts_with(&short),
        "resolved gate id reported: {body}"
    );
    assert_eq!(body["remaining"], 0);
    assert!(
        body["session_id"].as_str().is_some_and(|s| !s.is_empty()),
        "session attribution present"
    );

    // The tool then really runs.
    let sid = body["session_id"].as_str().unwrap().to_string();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut transcript = String::new();
    while std::time::Instant::now() < deadline {
        if let Ok(res) = client
            .get(format!("{}/sessions/{sid}/transcript", gw.base))
            .send()
            .await
            && res.status() == reqwest::StatusCode::OK
        {
            let t: serde_json::Value = res.json().await.unwrap();
            if t.get("error").is_none()
                && let Ok(raw) = serde_json::to_string(&t)
                && raw.contains("addressed-run")
            {
                transcript = raw;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(
        transcript.contains("addressed-run"),
        "bash ran: {transcript}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn default_policy_denies_without_forwarding() {
    let gw = spawn_with_config(
        Arc::new(Scripted {
            // Denial feeds an error result back; the completion guard may
            // ask for one more pass, so keep a spare response queued.
            responses: Mutex::new(VecDeque::from(vec![
                tool_call(
                    "t1",
                    "bash",
                    serde_json::json!({"command": "echo denied-run"}),
                ),
                text("skipping that step"),
                text("skipping that step"),
            ])),
        }),
        &config_with_modes("", "read-only", "ask"), // approvals unset => deny
    )
    .await;
    let client = client_with(&gw.token);

    // No approver configured: forward_mode() must be false even if someone
    // sends verdict-shaped text to any chat.
    let res = inbound(&client, &gw.base, "log", "ops", "yes", false).await;
    let body: serde_json::Value = res.json().await.unwrap();
    assert_ne!(body["state"], "approval_resolved");

    // The unattended turn completes via auto-deny; nothing was forwarded.
    let res = inbound(&client, &gw.base, "webhook", "ci", "go", true).await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["text"], "skipping that step");
    assert!(
        !deliveries(&gw.home).exists(),
        "deny mode must not announce anything"
    );
}
