//! A worker's question carried to the gateway's approver chat
//! (docs/design/84-worker-questions-and-control.md §4.5): announced there,
//! answered there by `answer …`, refused from any other chat, and never
//! forwarded when the gateway is not in forward mode.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

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
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
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
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.clone()));
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
    home.join("gateway/deliveries")
}

async fn wait_for_delivery(home: &Path, needle: &str, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if vak_session::chain::RecordChain::at(deliveries(home))
            .text()
            .contains(needle)
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    false
}

fn script(extra_between: Option<AssistantMessage>) -> Arc<Scripted> {
    let mut queue = vec![
        tool_call(
            "t1",
            "task",
            serde_json::json!({"prompt": "total", "label": "Totals"}),
        ),
        tool_call(
            "a1",
            "ask_parent",
            serde_json::json!({"question": "Which fiscal year?", "options": ["2025", "2026"]}),
        ),
    ];
    queue.extend(extra_between);
    queue.push(text("worker done"));
    queue.push(text("parent done"));
    Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(queue)),
    })
}

async fn transcript_contains(
    client: &reqwest::Client,
    gw: &Gateway,
    needle: &str,
    secs: u64,
) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        let status: serde_json::Value = client
            .get(format!("{}/gateway/status", gw.base))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(sid) = status["bindings"][0]["session_id"].as_str()
            && let Ok(res) = client
                .get(format!("{}/sessions/{sid}/transcript", gw.base))
                .send()
                .await
            && res.status() == reqwest::StatusCode::OK
        {
            let raw = res.text().await.unwrap_or_default();
            if raw.contains(needle) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    false
}

async fn question_pending(client: &reqwest::Client, gw: &Gateway) -> u64 {
    let status: serde_json::Value = client
        .get(format!("{}/gateway/status", gw.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    status["questions"]["pending"].as_u64().unwrap()
}

fn worker_ledgers(home: &Path) -> String {
    let mut out = String::new();
    fn walk(dir: &Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let child = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("child-"));
            if child && vak_config::scope::ledger_session_id(&path).is_some() {
                out.push_str(&vak_session::SessionLog::text(&path));
            } else if path.is_dir() {
                walk(&path, out);
            }
        }
    }
    // A worker's ledger sits in the shared sessions root, which a test pins
    // to its isolated data home rather than the gateway's own sessions home.
    walk(home, &mut out);
    walk(&vak_config::paths::data_home(), &mut out);
    out
}

async fn wait_for_ledger(home: &Path, needle: &str, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if worker_ledgers(home).contains(needle) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_forwarded_question_is_answered_from_the_approver_chat() {
    let gw = spawn_with_config(
        script(None),
        &config_with_modes(
            "approvals = \"forward\"\napprover = \"log:ops\"\n",
            "full-access",
            "ask",
        ),
    )
    .await;
    let client = client_with(&gw.token);

    let res = inbound(&client, &gw.base, "webhook", "ci", "total the year", false).await;
    assert_eq!(res.status(), 202);
    assert!(
        wait_for_delivery(&gw.home, "Question from Totals", 15).await,
        "the question is announced on the approver surface"
    );
    let raw = vak_session::chain::RecordChain::at(deliveries(&gw.home)).text();
    assert!(
        raw.contains("Which fiscal year?") && raw.contains("2025 | 2026"),
        "{raw}"
    );
    assert_eq!(question_pending(&client, &gw).await, 1);

    // A bare reply answers the only question waiting.
    let res = inbound(&client, &gw.base, "log", "ops", "answer fiscal 2026", false).await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "question_answered", "{body}");
    assert_eq!(body["worker"], "Totals");

    assert!(
        wait_for_ledger(&gw.home, "Answer from the approver chat: fiscal 2026", 20).await,
        "the answer reached the worker"
    );
    assert_eq!(question_pending(&client, &gw).await, 0);
    // A second reply has nothing to answer.
    let res = inbound(&client, &gw.base, "log", "ops", "answer again", false).await;
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "no_pending_questions");
    assert!(transcript_contains(&client, &gw, "parent done", 20).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chat_that_is_not_the_approver_cannot_answer() {
    // The stranger's own message is ordinary input: it gets a turn, which
    // takes the next scripted reply, then the worker and parent finish.
    let gw = spawn_with_config(
        script(Some(text("noted"))),
        &config_with_modes(
            "approvals = \"forward\"\napprover = \"log:ops\"\n",
            "full-access",
            "ask",
        ),
    )
    .await;
    let client = client_with(&gw.token);

    let res = inbound(&client, &gw.base, "webhook", "ci", "total the year", false).await;
    assert_eq!(res.status(), 202);
    assert!(wait_for_delivery(&gw.home, "Question from Totals", 15).await);

    // From any other chat, "answer ..." is conversation, not an answer.
    let res = inbound(
        &client,
        &gw.base,
        "webhook",
        "stranger",
        "answer 2025",
        false,
    )
    .await;
    assert_ne!(
        res.json::<serde_json::Value>().await.unwrap()["state"],
        "question_answered"
    );
    assert_eq!(
        question_pending(&client, &gw).await,
        1,
        "a non-approver chat resolves nothing"
    );
    assert!(
        !worker_ledgers(&gw.home).contains("Answer from the approver chat: 2025"),
        "the stranger's text never reached the worker"
    );

    let res = inbound(&client, &gw.base, "log", "ops", "answer 2026", false).await;
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "question_answered", "{body}");
    assert!(wait_for_ledger(&gw.home, "Answer from the approver chat: 2026", 20).await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_forward_mode_the_worker_is_told_nobody_is_available() {
    let gw = spawn_with_config(
        script(None),
        &config_with_modes("approvals = \"deny\"\n", "full-access", "ask"),
    )
    .await;
    let client = client_with(&gw.token);

    let res = inbound(&client, &gw.base, "webhook", "ci", "total the year", false).await;
    assert_eq!(res.status(), 202);
    assert!(
        wait_for_ledger(&gw.home, "Nobody is available to answer", 20).await,
        "{}",
        worker_ledgers(&gw.home)
    );
    assert!(
        !vak_session::chain::RecordChain::at(deliveries(&gw.home))
            .text()
            .contains("Question from"),
        "nothing was forwarded"
    );
    assert_eq!(question_pending(&client, &gw).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unanswered_question_expires_and_a_late_answer_resolves_nothing() {
    let gw = spawn_with_config(
        script(None),
        &config_with_modes(
            "approvals = \"forward\"\napprover = \"log:ops\"\napproval_timeout_secs = 3\n",
            "full-access",
            "ask",
        ),
    )
    .await;
    let client = client_with(&gw.token);

    let res = inbound(&client, &gw.base, "webhook", "ci", "total the year", false).await;
    assert_eq!(res.status(), 202);
    assert!(wait_for_delivery(&gw.home, "Question from Totals", 15).await);
    assert!(
        wait_for_ledger(&gw.home, "No answer arrived", 20).await,
        "the worker is told when the window passes"
    );

    let res = inbound(&client, &gw.base, "log", "ops", "answer too late", false).await;
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "no_pending_questions", "{body}");
    assert_eq!(question_pending(&client, &gw).await, 0);
    assert!(!worker_ledgers(&gw.home).contains("approver chat: too late"));
}
