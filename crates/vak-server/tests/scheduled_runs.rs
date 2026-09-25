//! Scheduled runs that cannot start say so, and runs that do are findable
//! (docs/plans/data-architecture-plan.md, M0): a refused routine leaves a
//! `routine_failed` inbox entry with the reason and remedy instead of a log
//! line; a folder that is not a git repository is refused loudly; two tasks
//! due in one tick both run; and a run recorded on a task opens after a
//! restart, because its handle is its ledger id.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Counting(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Provider for Counting {
    fn name(&self) -> &str {
        "counting"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let (mut sink, rx) = stream::channel(8);
        let done = AssistantMessage {
            content: vec![ContentBlock::text("routine done")],
            stop_reason: vak_llm::types::StopReason::EndTurn,
            usage: Usage::default(),
            model: "counted-model".into(),
            response_id: None,
        };
        sink.push(stream::StreamEvent::Start {
            partial: done.clone(),
        });
        sink.close_message(done).await;
        Ok(rx)
    }
}

struct Server {
    base: String,
    token: String,
    dispatches: Arc<AtomicUsize>,
}

impl Server {
    fn client(&self) -> reqwest::Client {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", self.token).parse().unwrap(),
        );
        reqwest::ClientBuilder::new()
            .default_headers(headers)
            .build()
            .unwrap()
    }

    async fn get(&self, path: &str) -> (reqwest::StatusCode, serde_json::Value) {
        let response = self
            .client()
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap();
        let status = response.status();
        (status, response.json().await.unwrap_or_default())
    }

    async fn task(&self, id: &str) -> serde_json::Value {
        let (_, body) = self.get("/tasks").await;
        body["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|task| task["id"] == id)
            .cloned()
            .unwrap_or_default()
    }

    async fn run_now(&self, id: &str) -> reqwest::StatusCode {
        self.client()
            .post(format!("{}/tasks/{id}/run-now", self.base))
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn inbox(&self) -> Vec<serde_json::Value> {
        let (_, body) = self.get("/inbox").await;
        body["entries"].as_array().cloned().unwrap_or_default()
    }
}

fn git_seed(cwd: &Path) {
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

/// A never-run prompt task in `ws`, due at the first tick.
fn prompt_task(id: &str, ws: &Path, agent_id: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": id,
        "prompt": "summarise the day",
        "interval_secs": 3600,
        "enabled": true,
        "cwd": ws.display().to_string(),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "last_run_at": null,
        "last_session_id": null,
        "last_summary": null,
        "last_wt": null,
        "agent_id": agent_id,
    })
}

/// A workspace (a git repository when `git`) and a home seeded with
/// `tasks`, returned before any server starts.
fn space(
    git: bool,
    tasks: impl FnOnce(&Path) -> serde_json::Value,
) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("ws");
    std::fs::create_dir_all(ws.join(".vak")).unwrap();
    std::fs::write(
        ws.join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    if git {
        git_seed(&ws);
    }
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        vak_core::tasks::tasks_file(&home),
        serde_json::to_string_pretty(&tasks(&ws)).unwrap(),
    )
    .unwrap();
    (dir, ws, home)
}

async fn serve(ws: &Path, home: &Path) -> Server {
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(ws.to_path_buf(), true).unwrap();
    core.set_sessions_home(home.to_path_buf());
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_tool_worker_exe(PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")));
    let dispatches = Arc::new(AtomicUsize::new(0));
    core.set_provider_instance(Arc::new(Counting(dispatches.clone())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with(core, false);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        base: format!("http://{addr}"),
        token,
        dispatches,
    }
}

async fn eventually<F, Fut>(secs: u64, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if check().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    check().await
}

fn routine_failures(inbox: &[serde_json::Value], task: &str) -> Vec<serde_json::Value> {
    inbox
        .iter()
        .filter(|entry| entry["kind"] == "routine_failed" && entry["task_id"] == task)
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fire_task_records_refusal() {
    let (_dir, ws, home) = space(true, |ws| {
        serde_json::json!([prompt_task("for-nobody", ws, Some("no-such-agent"))])
    });
    let server = serve(&ws, &home).await;
    assert_eq!(
        server.run_now("for-nobody").await,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let failures = routine_failures(&server.inbox().await, "for-nobody");
    assert_eq!(
        failures.len(),
        1,
        "one entry for the missed slot, however often it is retried"
    );
    assert!(
        failures[0]["body"]
            .as_str()
            .unwrap()
            .contains("no-such-agent"),
        "{failures:?}"
    );
    assert_eq!(
        server.task("for-nobody").await["last_run_status"],
        "refused"
    );
    assert_eq!(server.dispatches.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_git_space_routine_is_refused_loudly() {
    let (_dir, ws, home) = space(false, |ws| {
        serde_json::json!([prompt_task("plain-folder", ws, None)])
    });
    let server = serve(&ws, &home).await;
    assert!(
        eventually(10, || async {
            !routine_failures(&server.inbox().await, "plain-folder").is_empty()
        })
        .await,
        "the scheduler's refusal reached the inbox"
    );
    let failures = routine_failures(&server.inbox().await, "plain-folder");
    let body = failures[0]["body"].as_str().unwrap();
    assert!(body.contains("is not a git repository"), "{body}");
    assert!(body.contains("git init"), "the remedy is named: {body}");
    assert_eq!(
        server.run_now("plain-folder").await,
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        routine_failures(&server.inbox().await, "plain-folder").len(),
        1,
        "retries of the same slot do not repeat the entry"
    );
    assert_eq!(server.dispatches.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_tasks_due_same_tick_both_fire() {
    let (_dir, ws, home) = space(true, |ws| {
        serde_json::json!([
            prompt_task("first", ws, None),
            prompt_task("second", ws, None)
        ])
    });
    let server = serve(&ws, &home).await;
    assert!(
        eventually(20, || async {
            server.task("first").await["last_run_status"] == "complete"
                && server.task("second").await["last_run_status"] == "complete"
        })
        .await,
        "both routines ran: {} / {}",
        server.task("first").await,
        server.task("second").await
    );
    let first = server.task("first").await["last_session_id"].clone();
    let second = server.task("second").await["last_session_id"].clone();
    assert_ne!(first, second);
    assert!(server.dispatches.load(Ordering::SeqCst) >= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduled_run_resolves_after_restart() {
    let (_dir, ws, home) = space(true, |ws| {
        serde_json::json!([prompt_task("nightly", ws, None)])
    });
    let server = serve(&ws, &home).await;
    assert!(
        eventually(20, || async {
            server.task("nightly").await["last_run_status"] == "complete"
        })
        .await,
        "the routine ran"
    );
    let session = server.task("nightly").await["last_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let restarted = serve(&ws, &home).await;
    let (status, transcript) = restarted
        .get(&format!("/sessions/{session}/transcript"))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{transcript}");
    assert!(
        transcript.to_string().contains("routine done"),
        "{transcript}"
    );
}
