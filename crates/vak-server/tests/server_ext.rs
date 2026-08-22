#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, Usage};
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

/// Spawn `secured_router` exactly like the desktop shell will: bind an
/// ephemeral loopback port and keep the token for authenticated calls.
async fn spawn_secured(
    provider: Arc<dyn Provider>,
) -> (
    String,
    String,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_provider_instance(provider);
    // keep tempdir alive for the process lifetime of the test
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), token, cwd, handle)
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

fn text(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![vak_llm::types::ContentBlock::text(t)],
        stop_reason: vak_llm::types::StopReason::EndTurn,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
    }
}

async fn wait_transcript(client: &reqwest::Client, base: &str, id: &str) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        assert!(std::time::Instant::now() < deadline, "transcript timeout");
        if let Ok(res) = client
            .get(format!("{base}/sessions/{id}/transcript"))
            .send()
            .await
            && res.status() == reqwest::StatusCode::OK
        {
            let body: serde_json::Value = res.json().await.unwrap();
            // While a run is live the endpoint answers 200 with an error
            // payload; keep polling until the real transcript lands.
            if body.get("error").is_none() {
                return body;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn secured_router_enforces_token_and_cors() {
    let (base, token, _cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;

    // /health is open by design.
    let health = reqwest::get(format!("{base}/health")).await.unwrap();
    assert_eq!(health.status(), 200);

    // Missing or wrong bearer tokens are rejected.
    let anon = reqwest::get(format!("{base}/sessions")).await.unwrap();
    assert_eq!(anon.status(), 401);
    let wrong = reqwest::Client::new()
        .get(format!("{base}/sessions"))
        .bearer_auth("vk_wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);

    // The webview origin must be allowed through CORS preflight.
    let preflight = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, format!("{base}/sessions"))
        .header(reqwest::header::ORIGIN, "tauri://localhost")
        .header(reqwest::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .send()
        .await
        .unwrap();
    assert_eq!(
        preflight
            .headers()
            .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&reqwest::header::HeaderValue::from_static(
            "tauri://localhost"
        )),
    );

    // The real token grants access.
    let ok = client_with(&token)
        .get(format!("{base}/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_list_attach_and_title_roundtrip() {
    // Build the persisted ledger directly, then drop it so the lock is
    // released — mirroring "TUI wrote this session earlier and exited".
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let core_a = Core::new(cwd.clone()).unwrap();
    core_a.set_sessions_home(cwd.join("home"));
    std::mem::forget(dir);

    let mut log = core_a.start_session().await.unwrap();
    let session_id = log.header().unwrap().session_id.clone();
    use vak_llm::types::{ContentBlock as CB, Role};
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message {
            role: Role::User,
            content: vec![CB::text("title probe here")],
        },
        meta: None,
    })
    .unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message {
            role: Role::Assistant,
            content: vec![CB::text("all done")],
        },
        meta: None,
    })
    .unwrap();
    drop(log);

    // A fresh server over the same store: lists the session, resumes it.
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(cwd.join("home"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = client_with(&token);

    // The sidebar projection must see the persisted file with a usable title.
    let listed = client.get(format!("{base}/sessions")).send().await.unwrap();
    let body: serde_json::Value = listed.json().await.unwrap();
    let entry = body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["session_id"] == session_id.as_str())
        .expect("persisted session must be listed");
    assert_eq!(entry["title"], "title probe here");
    assert!(entry["updated_at"].is_string());

    let attached = client
        .post(format!("{base}/sessions/{session_id}/attach"))
        .json(&serde_json::json!({"session_id": session_id}))
        .send()
        .await
        .unwrap();
    assert_eq!(attached.status(), 200);

    let resumed: serde_json::Value = client
        .get(format!("{base}/sessions/{session_id}/transcript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resumed["count"].as_u64(), Some(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fs_endpoints_are_confined_to_workspace() {
    let (base, token, cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);

    // Write inside the workspace.
    let put = client
        .put(format!("{base}/fs/file"))
        .json(&serde_json::json!({"path": "notes/hello.txt", "content": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 200);
    assert_eq!(
        tokio::fs::read_to_string(cwd.join("notes/hello.txt"))
            .await
            .unwrap(),
        "hi"
    );

    let got: serde_json::Value = client
        .get(format!("{base}/fs/file?path=notes/hello.txt"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(got["content"], "hi");

    // Traversal escape is rejected.
    let escape_put = client
        .put(format!("{base}/fs/file"))
        .json(&serde_json::json!({"path": "../evil.txt", "content": "nope"}))
        .send()
        .await
        .unwrap();
    assert_eq!(escape_put.status(), 403);

    let escape_get = client
        .get(format!("{base}/fs/file?path=/etc/hostname"))
        .send()
        .await
        .unwrap();
    assert_eq!(escape_get.status(), 403);

    let missing = client
        .get(format!("{base}/fs/file?path=no/such.txt"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mode_switch_and_diff_endpoint() {
    let (base, token, _cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);

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

    // Unknown mode is a bad request.
    let bad = client
        .post(format!("{base}/config/mode"))
        .json(&serde_json::json!({"mode": "yolo"}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    let switched = client
        .post(format!("{base}/config/mode"))
        .json(&serde_json::json!({"mode": "read-only"}))
        .send()
        .await
        .unwrap();
    assert_eq!(switched.status(), 200);
    let health: serde_json::Value = reqwest::get(format!("{base}/health"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["permission_mode"], "ReadOnly");

    // Tempdir is not a git repo: diff endpoint reports that as a value.
    let diff: serde_json::Value = client
        .get(format!("{base}/sessions/{session_id}/diff"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(diff["error"], "not a git repository");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fs_tree_lists_and_skips_vendored_dirs() {
    let (base, token, cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);

    tokio::fs::create_dir_all(cwd.join("src/deep"))
        .await
        .unwrap();
    tokio::fs::write(cwd.join("src/main.rs"), "fn main() {}")
        .await
        .unwrap();
    tokio::fs::write(cwd.join("src/deep/util.rs"), "pub fn u() {}")
        .await
        .unwrap();
    tokio::fs::create_dir_all(cwd.join("node_modules/left-pad"))
        .await
        .unwrap();
    tokio::fs::write(cwd.join("node_modules/left-pad/index.js"), "x")
        .await
        .unwrap();

    let body: serde_json::Value = client
        .get(format!("{base}/fs/tree"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let files: Vec<String> = body["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert!(
        files.contains(&"src/main.rs".to_string()),
        "files: {files:?}"
    );
    assert!(files.contains(&"src/deep/util.rs".to_string()));
    assert!(
        !files.iter().any(|f| f.starts_with("node_modules")),
        "vendored dirs must be skipped"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn side_chat_branches_off_and_restores_main_chain() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("main answer"),
            text("side answer"),
        ])),
    });
    let (base, token, cwd, _server) = spawn_secured(provider).await;
    let client = client_with(&token);

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
        .json(&serde_json::json!({"prompt": "say main answer"}))
        .send()
        .await
        .unwrap();
    let before = wait_transcript(&client, &base, &session_id).await;
    assert_eq!(before["count"].as_u64(), Some(2));

    let started = client
        .post(format!("{base}/sessions/{session_id}/side"))
        .json(&serde_json::json!({"question": "quick aside?"}))
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), 202);

    // Once the side turn completes, the main view is exactly what it was.
    let after = wait_transcript(&client, &base, &session_id).await;
    assert_eq!(after["count"].as_u64(), Some(2), "main chain untouched");
    assert!(
        !serde_json::to_string(&after)
            .unwrap()
            .contains("side answer"),
        "side answer must not leak into derive_messages()"
    );
    assert!(
        serde_json::to_string(&after)
            .unwrap()
            .contains("main answer"),
        "main context must remain intact"
    );

    // ...but the sibling branch stays reconstructable in the JSONL.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut dir = std::fs::read_dir(vak_session::SessionPath::sessions_dir(
        &cwd.join("home"),
        &cwd,
    ))
    .unwrap();
    let path = dir.next().unwrap().unwrap().path();
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(raw.contains("quick aside?"), "question persisted");
    assert!(
        raw.contains("side answer"),
        "answer persisted as branch entry"
    );
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bestofn_fans_out_keep_and_discard() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("candidate A"),
            text("candidate B"),
        ])),
    });
    let (base, token, cwd, _server) = spawn_secured(provider).await;
    let client = client_with(&token);
    git_seed(&cwd);

    let anchor: String = client
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

    // Not a repo guard would have fired before seed; now it must succeed.
    let res = client
        .post(format!("{base}/sessions/{anchor}/bestofn"))
        .json(&serde_json::json!({"prompt": "solve it", "n": 2}))
        .send()
        .await
        .unwrap();
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(status, 200, "start failed: {body}");
    let runs = body["runs"].as_array().unwrap().clone();
    assert_eq!(runs.len(), 2);

    let wt_root = cwd.join(".vakcoder/worktrees");
    let wait_child = |cid: String| {
        let client = client.clone();
        let base = base.clone();
        async move { wait_transcript(&client, &base, &cid).await }
    };

    let mut child_ids = Vec::new();
    for r in &runs {
        let cid = r["session_id"].as_str().unwrap().to_string();
        let t = wait_child(cid.clone()).await;
        assert_eq!(t["count"].as_u64(), Some(2));
        child_ids.push(cid);
    }

    // Both worktrees exist on disk.
    assert_eq!(std::fs::read_dir(&wt_root).unwrap().count(), 2);

    // Discard one: worktree + branch gone.
    let disc = client
        .post(format!("{base}/sessions/{}/discard", child_ids[1]))
        .send()
        .await
        .unwrap();
    assert_eq!(disc.status(), 200);
    assert_eq!(std::fs::read_dir(&wt_root).unwrap().count(), 1);

    // Keep the other: endpoint merges (up-to-date is fine for text-only
    // candidates) and always cleans up the worktree + branch.
    let keep = client
        .post(format!("{base}/sessions/{}/keep", child_ids[0]))
        .send()
        .await
        .unwrap();
    assert_eq!(
        keep.status(),
        200,
        "keep failed: {}",
        keep.text().await.unwrap_or_default()
    );

    assert_eq!(
        std::fs::read_dir(&wt_root).unwrap().count(),
        0,
        "worktrees all cleaned"
    );
    let branches = {
        let o = std::process::Command::new("git")
            .args(["branch", "--list", "vakcoder/*"])
            .current_dir(&cwd)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    assert!(
        branches.is_empty(),
        "candidate branches must be gone: {branches}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pr_endpoints_surface_structured_results() {
    let (base, token, cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);
    git_seed_main(&cwd);

    let anchor: String = client
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

    // Status is always structured JSON: either a PR view, a no_pr/gh reason,
    // or an error — never a hang.
    let body: serde_json::Value = client
        .get(format!("{base}/sessions/{anchor}/pr"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["branch"], "main");
    assert!(body.get("pr").is_some(), "must carry pr slot: {body}");
    if body["pr"].is_null() {
        assert!(body.get("reason").is_some() || body.get("error").is_some());
    }

    // Merge endpoint surfaces gh failures as values (409), not hangs.
    let merged = client
        .post(format!("{base}/sessions/{anchor}/pr/merge"))
        .json(&serde_json::json!({"number": 999}))
        .send()
        .await
        .unwrap();
    assert!(
        merged.status() == 200 || merged.status() == 409,
        "merge must resolve deterministically"
    );

    // Unknown session → 404 on merge.
    let missing = client
        .post(format!("{base}/sessions/nope/pr/merge"))
        .json(&serde_json::json!({"number": 1}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

fn git_seed_main(cwd: &std::path::Path) {
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
    run(&["init", "-q", "-b", "main"]);
    std::fs::write(cwd.join("README.md"), "seed\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "seed"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scheduled_tasks_crud_runnow_and_worktree_churn() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("task ran"),
            text("task ran again"),
        ])),
    });
    let (base, token, cwd, _server) = spawn_secured(provider).await;
    let client = client_with(&token);
    git_seed_main(&cwd);

    // Interval validation is a value, not a panic.
    let bad = client
        .post(format!("{base}/tasks"))
        .json(&serde_json::json!({"name":"too hot","prompt":"x","interval_secs":10}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    let created = client
        .post(format!("{base}/tasks"))
        .json(&serde_json::json!({"name":"nightly sweep","prompt":"sweep the repo","interval_secs":3600}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200);

    let listed: serde_json::Value = client
        .get(format!("{base}/tasks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tasks = listed["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["name"], "nightly sweep");
    assert_eq!(tasks[0]["enabled"], true);
    let tid = tasks[0]["id"].as_str().unwrap().to_string();

    // Run now: child runs in a worktree; transcript completes; task records it.
    let fired = client
        .post(format!("{base}/tasks/{tid}/run-now"))
        .send()
        .await
        .unwrap();
    assert_eq!(fired.status(), 202);

    #[allow(unused_assignments)]
    let mut child_id = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        assert!(std::time::Instant::now() < deadline, "task run timeout");
        let t: serde_json::Value = client
            .get(format!("{base}/tasks"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let t0 = &t["tasks"][0];
        if let Some(cid) = t0["last_session_id"].as_str() {
            child_id = cid.to_string();
            let _ = &child_id;
            if t0["last_summary"] == "completed" {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let ct = wait_transcript(&client, &base, &child_id).await;
    assert_eq!(ct["count"].as_u64(), Some(2));
    let wt_root = cwd.join(".vakcoder/worktrees");
    assert_eq!(std::fs::read_dir(&wt_root).unwrap().count(), 1);

    // Second run replaces the worktree (latest-only retention).
    client
        .post(format!("{base}/tasks/{tid}/run-now"))
        .send()
        .await
        .unwrap();
    let deadline2 = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        assert!(std::time::Instant::now() < deadline2, "second run timeout");
        let t: serde_json::Value = client
            .get(format!("{base}/tasks"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if t["tasks"][0]["last_session_id"].as_str() != Some(child_id.as_str())
            && t["tasks"][0]["last_summary"] == "completed"
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    assert_eq!(
        std::fs::read_dir(&wt_root).unwrap().count(),
        1,
        "old worktree replaced"
    );

    // Delete cleans up the remaining worktree.
    let del = client
        .delete(format!("{base}/tasks/{tid}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);
    assert_eq!(std::fs::read_dir(&wt_root).unwrap().count(), 0);
}
