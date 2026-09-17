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
    // Isolate from the developer's real global config: layered config
    // loads $HOME/.config/vak/config.toml, and a personal
    // `[memory] reflection = true` would make the post-turn reflection
    // seam consume scripted provider responses mid-test.
    let project = cwd.join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
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

    // Both Tauri webview origins must be allowed through CORS preflight.
    for origin in ["tauri://localhost", "http://tauri.localhost"] {
        let preflight = reqwest::Client::new()
            .request(reqwest::Method::OPTIONS, format!("{base}/sessions"))
            .header(reqwest::header::ORIGIN, origin)
            .header(reqwest::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .send()
            .await
            .unwrap();
        assert_eq!(
            preflight
                .headers()
                .get(reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&reqwest::header::HeaderValue::from_bytes(origin.as_bytes()).unwrap()),
        );
    }

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
    vak_config::paths::isolate_home_for_tests();
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
    vak_config::paths::isolate_home_for_tests();
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
    assert_eq!(entry["running"], false);

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
async fn sessions_list_hides_abandoned_header_only_drafts() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(cwd.join("home"));
    let draft = core.start_session().await.unwrap();
    let draft_id = draft.header().unwrap().session_id.clone();
    drop(draft);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let body: serde_json::Value = client_with(&token)
        .get(format!("http://{addr}/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|session| session["session_id"] != draft_id)
    );
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
    let (base, token, cwd, _server) = spawn_secured(Arc::new(Scripted {
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

    let config: serde_json::Value = client
        .get(format!("{base}/config"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(config["permission_mode"], "ReadOnly");
    assert!(config["paths"]["project_config"].is_string());
    assert!(config["integrations"]["skills"].is_array());

    let patched = client
        .patch(format!("{base}/config"))
        .json(&serde_json::json!({
            "provider": "google",
            "model": "gemini-test",
            "max_turns": 17,
            "permission_mode": "workspace-write"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patched.status(), 200);
    let updated: serde_json::Value = client
        .get(format!("{base}/config"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["provider"], "google");
    assert_eq!(updated["model"], "gemini-test");
    assert_eq!(updated["max_turns"], 17);
    assert_eq!(updated["permission_mode"], "WorkspaceWrite");
    assert_eq!(updated["provider_source"], "project_config");
    assert_eq!(updated["model_source"], "project_config");
    assert!(
        updated["route_revision"]
            .as_str()
            .is_some_and(|r| r.starts_with('r'))
    );
    let persisted = tokio::fs::read_to_string(cwd.join(".vak/config.toml"))
        .await
        .unwrap();
    assert!(persisted.contains("provider = \"google\""));
    assert!(persisted.contains("model = \"gemini-test\""));
    vak_config::paths::isolate_home_for_tests();
    let restarted = Core::new_with_trust(cwd, true).unwrap();
    assert_eq!(restarted.effective_provider(), "google");
    assert_eq!(restarted.effective_model(), "gemini-test");

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
async fn failed_side_turn_does_not_wedge_the_session() {
    // Regression (v0.6 deployment gate): whatever way a side turn ends
    // without a restorable log — provider exhausted into endless endurance
    // retries, or an explicit cancel — the session handle must become
    // usable again instead of answering "run in progress" forever.
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("main answer"),
            text("side answer"),
        ])),
    });
    let (base, token, _cwd, _server) = spawn_secured(provider).await;
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

    // Start a side turn and cancel it mid-flight.
    let started = client
        .post(format!("{base}/sessions/{session_id}/side"))
        .json(&serde_json::json!({"question": "long aside?"}))
        .send()
        .await
        .unwrap();
    assert_eq!(started.status(), 202);
    let cancelled = client
        .post(format!("{base}/sessions/{session_id}/side/cancel"))
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled.status(), 202);

    // The handle must come back: transcript answers with real content and
    // a follow-up side turn is accepted rather than 409-conflicted.
    let after = wait_transcript(&client, &base, &session_id).await;
    assert_eq!(
        after["count"].as_u64(),
        Some(2),
        "cancelled side turn must leave the main chain intact"
    );
    let again = client
        .post(format!("{base}/sessions/{session_id}/side"))
        .json(&serde_json::json!({"question": "still here?"}))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 202, "session handle must be restorable");
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
    let agent_home = cwd.join("home").join("agents").join("vak");
    let home = if agent_home.exists() {
        agent_home
    } else {
        cwd.join("home")
    };
    let mut dir = std::fs::read_dir(vak_session::SessionPath::sessions_dir(
        &home,
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

    let wt_root = cwd.join(".vak/worktrees");
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
            .args(["branch", "--list", "vak/*"])
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
            // The watcher records the run's real final answer, not a
            // status word (docs/design/22-gateway.md).
            if t0["last_summary"] == "task ran" {
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let ct = wait_transcript(&client, &base, &child_id).await;
    assert_eq!(ct["count"].as_u64(), Some(2));
    let wt_root = cwd.join(".vak/worktrees");
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
            && t["tasks"][0]["last_summary"] == "task ran again"
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn launch_config_and_process_lifecycle() {
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    });
    let (base, token, cwd, _server) = spawn_secured(provider).await;
    let client = client_with(&token);

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

    // No config yet → empty list (auto-detect finds nothing in a tempdir).
    let empty: serde_json::Value = client
        .get(format!("{base}/sessions/{anchor}/launch"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty["servers"].as_array().unwrap().len(), 0);

    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/launch.toml"),
        "[[server]]\nname = \"static\"\ncmd = \"python3\"\nargs = [\"-m\", \"http.server\", \"4519\"]\nport = 4519\n",
    )
    .unwrap();

    let cfg: serde_json::Value = client
        .get(format!("{base}/sessions/{anchor}/launch"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let servers = cfg["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["port"], 4519);
    assert_eq!(servers[0]["running"], false);

    let py = std::process::Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !py {
        eprintln!("python3 unavailable; skipping process assertions");
        return;
    }

    let start = client
        .post(format!("{base}/sessions/{anchor}/launch/start"))
        .json(&serde_json::json!({"name": "static"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        start.status(),
        200,
        "{}",
        start.text().await.unwrap_or_default()
    );
    let body: serde_json::Value = start.json().await.unwrap();
    assert_eq!(body["started"], true);
    assert_eq!(body["listening"], true, "http.server should bind quickly");

    // Double-start conflicts.
    let again = client
        .post(format!("{base}/sessions/{anchor}/launch/start"))
        .json(&serde_json::json!({"name": "static"}))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), 409);

    // Logs are flowing.
    let logs: serde_json::Value = client
        .get(format!("{base}/sessions/{anchor}/launch/logs?name=static"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(logs["lines"].as_array().is_some());

    let stop = client
        .post(format!("{base}/sessions/{anchor}/launch/stop"))
        .json(&serde_json::json!({"name": "static"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stop.status(), 200);

    // Port actually released.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        tokio::net::TcpStream::connect("127.0.0.1:4519")
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checkpoints_list_and_restore_roundtrip() {
    let (base, token, cwd, _srv) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);
    let home = cwd.join("home"); // mirrors spawn_secured's sessions_home

    let created: serde_json::Value = client
        .post(format!("{base}/sessions"))
        .body("{}")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["session_id"].as_str().unwrap().to_string();

    // Capture a checkpoint directly (the primitive the loop uses at turn
    // start), then mutate the workspace "like an agent would".
    std::fs::write(cwd.join("notes.txt"), "original").unwrap();
    let cp = vak_core::checkpoints::capture(&cwd, &id, 1, "turn 1").unwrap();
    vak_core::checkpoints::store(&home, &cp).unwrap();
    std::fs::write(cwd.join("notes.txt"), "mutated by agent").unwrap();
    std::fs::write(cwd.join("stray.txt"), "extra").unwrap();

    let listed: serde_json::Value = client
        .get(format!("{base}/sessions/{id}/checkpoints"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let items = listed["checkpoints"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["seq"], 1);
    assert_eq!(items[0]["label"], "turn 1");
    // At least two files live here: notes.txt plus the session ledger
    // itself, because this test nests sessions_home inside the workspace.
    // The gateway may also seed a live-reloadable `gateway/allowlist.json`
    // from `gateway.chat_allowlist` on startup (docs/design/34), which
    // adds one more file whenever a config layer sets that key — not
    // asserted as an exact count so this stays independent of the
    // environment's own config layering.
    let expected_files = items[0]["files"].as_u64().unwrap();
    assert!(
        expected_files >= 2,
        "expected at least notes.txt + session ledger, got {expected_files}"
    );

    let restored: serde_json::Value = client
        .post(format!("{base}/sessions/{id}/checkpoints/1/restore"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["restored"].as_u64(), Some(expected_files));
    assert!(
        restored["deleted"].as_u64().unwrap() >= 1,
        "the stray file must be removed"
    );
    assert_eq!(
        std::fs::read_to_string(cwd.join("notes.txt")).unwrap(),
        "original"
    );
    assert!(!cwd.join("stray.txt").exists());
    // Observed-at-capture files are rewritten, never removed: the ledger
    // must survive the rewind untouched.
    let agent_home = home.join("agents").join("vak");
    let ledger_home = if agent_home.exists() {
        agent_home
    } else {
        home.clone()
    };
    let ledger =
        vak_session::SessionPath::sessions_dir(&ledger_home, &cwd).join(format!("{id}.jsonl"));
    assert!(
        ledger.is_file(),
        "rewind must never delete the session ledger"
    );

    let missing = client
        .post(format!("{base}/sessions/{id}/checkpoints/99/restore"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn archive_toggle_is_reflected_in_session_list() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    vak_config::paths::isolate_home_for_tests();
    let core_a = Core::new(cwd.clone()).unwrap();
    core_a.set_sessions_home(cwd.join("home"));
    std::mem::forget(dir);

    let mut log = core_a.start_session().await.unwrap();
    let session_id = log.header().unwrap().session_id.clone();
    use vak_llm::types::{ContentBlock as CB, Role};
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message {
            role: Role::User,
            content: vec![CB::text("archivable task")],
        },
        meta: None,
    })
    .unwrap();
    drop(log);

    vak_config::paths::isolate_home_for_tests();
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

    let archived_flag = |body: serde_json::Value| {
        body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["session_id"] == session_id.as_str())
            .expect("session must be listed")["archived"]
            .as_bool()
            .unwrap_or(false)
    };

    let listed: serde_json::Value = client
        .get(format!("{base}/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!archived_flag(listed));

    let on = client
        .post(format!("{base}/sessions/{session_id}/archive"))
        .json(&serde_json::json!({ "archived": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(on.status(), 200);
    let listed: serde_json::Value = client
        .get(format!("{base}/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(archived_flag(listed));

    // The sidecar must persist independently of the running server.
    let raw = std::fs::read_to_string(cwd.join("home/archive.json")).unwrap();
    assert!(raw.contains(&session_id));

    let off = client
        .post(format!("{base}/sessions/{session_id}/archive"))
        .json(&serde_json::json!({ "archived": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(off.status(), 200);
    let listed: serde_json::Value = client
        .get(format!("{base}/sessions"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!archived_flag(listed));

    let unknown = client
        .post(format!("{base}/sessions/not-a-session/archive"))
        .json(&serde_json::json!({ "archived": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);
}

/// `archive.json`/`deleted.json` are one shared map per `sessions_home`,
/// keyed by session id with no workspace scoping of their own — a ledger
/// file only ever lives under *this* process's own
/// `sessions_dir(sessions_home, cwd)`. Single-session delete already checks
/// that file exists before acting on it; `DELETE /sessions/archived`
/// (bulk) iterated every archived id in the shared map with no such check,
/// so running it from a server bound to workspace B could soft-delete an
/// archived session that only ever belonged to workspace A.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delete_all_archived_never_touches_a_different_workspaces_session() {
    use vak_llm::types::{ContentBlock as CB, Role};
    let home_dir = tempfile::tempdir().unwrap();
    let home = home_dir.path().to_path_buf();
    std::mem::forget(home_dir);

    async fn make_archived_session(cwd: &std::path::Path, home: &std::path::Path) -> String {
        vak_config::paths::isolate_home_for_tests();
        let core = Core::new(cwd.to_path_buf()).unwrap();
        core.set_sessions_home(home.to_path_buf());
        let mut log = core.start_session().await.unwrap();
        let id = log.header().unwrap().session_id.clone();
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message {
                role: Role::User,
                content: vec![CB::text("task")],
            },
            meta: None,
        })
        .unwrap();
        drop(log);
        id
    }

    let dir_a = tempfile::tempdir().unwrap();
    let cwd_a = dir_a.path().to_path_buf();
    std::mem::forget(dir_a);
    let dir_b = tempfile::tempdir().unwrap();
    let cwd_b = dir_b.path().to_path_buf();
    std::mem::forget(dir_b);

    let id_a = make_archived_session(&cwd_a, &home).await;
    let id_b = make_archived_session(&cwd_b, &home).await;

    // Archive both directly in the shared sidecar, exactly as the running
    // server would after two `POST .../archive` calls from two different
    // workspaces.
    let archive_path = home.join("archive.json");
    std::fs::write(
        &archive_path,
        serde_json::to_string(&serde_json::json!({ &id_a: true, &id_b: true })).unwrap(),
    )
    .unwrap();

    // A server bound to workspace B only.
    vak_config::paths::isolate_home_for_tests();
    let core_b = Core::new(cwd_b.clone()).unwrap();
    core_b.set_sessions_home(home.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core_b);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = client_with(&token);

    let res = client
        .delete(format!("{base}/sessions/archived"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["deleted"], 1, "only workspace B's own session, {body}");

    let deleted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("deleted.json")).unwrap()).unwrap();
    assert_eq!(
        deleted[&id_b], true,
        "workspace B's archived session must be deleted"
    );
    assert!(
        deleted.get(&id_a).is_none() || deleted[&id_a] != true,
        "workspace A's archived session must survive a bulk delete run from workspace B"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skills_listing_and_pascalcase_mode() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let skill_dir = cwd.join(".vak/skills/tdd");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: tdd\ndescription: red green refactor loop\n---\nbody",
    )
    .unwrap();
    std::mem::forget(dir);

    vak_config::paths::isolate_home_for_tests();
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

    let skills: serde_json::Value = client
        .get(format!("{base}/skills"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = skills["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"tdd"));
    let tdd = skills["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "tdd")
        .unwrap();
    assert_eq!(tdd["description"], "red green refactor loop");

    // The status bar sends the Debug spelling surfaced by /health.
    for mode in [
        "ReadOnly",
        "WorkspaceWrite",
        "FullAccess",
        "workspace-write",
    ] {
        let res = client
            .post(format!("{base}/config/mode"))
            .json(&serde_json::json!({ "mode": mode }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200, "mode {mode} must parse");
    }
    let bad = client
        .post(format!("{base}/config/mode"))
        .json(&serde_json::json!({ "mode": "yolo" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn providers_listing_and_key_storage_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::mem::forget(dir);

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(cwd.join("home"));
    // Hermetic secret store: never touch the developer's real ~/.vak.
    let user_env = cwd.join("user-home/.vak/.env");
    core.set_user_env_path(user_env.clone());
    // Pin the current provider so the assertion reflects THIS fixture, not
    // whatever default the developer's global config selects.
    core.set_provider("anthropic".to_string());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = client_with(&token);

    // Listing: names, env var mapping, configured flags, curated models.
    let listed: serde_json::Value = client
        .get(format!("{base}/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["current"], "anthropic");
    let providers = listed["providers"].as_array().unwrap();
    let anthropic = providers
        .iter()
        .find(|p| p["name"] == "anthropic")
        .expect("anthropic must be listed");
    assert_eq!(anthropic["env_var"], "ANTHROPIC_API_KEY");
    assert_eq!(anthropic["requires_key"], true);
    // Nothing is configured in this hermetic environment.
    assert_eq!(anthropic["configured"], false);
    let ollama = providers.iter().find(|p| p["name"] == "ollama").unwrap();
    assert_eq!(ollama["requires_key"], false);
    assert_eq!(ollama["configured"], true);
    // Model lists are discovered per provider, never shipped in this
    // payload — a static catalogue would drift from what the key reaches.
    assert!(
        listed.get("models").is_none(),
        "/providers must not ship a hardcoded model catalogue"
    );
    assert_eq!(listed["current_configured"], false);

    // Saving a key: effective immediately, persisted to the store, never
    // echoed back.
    let saved: serde_json::Value = client
        .put(format!("{base}/config/key"))
        .json(&serde_json::json!({ "provider": "opencode-zen", "key": "  sk-test-123  " }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(saved["provider"], "opencode-zen");
    assert_eq!(saved["env_var"], "OPENCODE_API_KEY");
    assert_eq!(saved["configured"], true);
    let body_text = serde_json::to_string(&saved).unwrap();
    assert!(!body_text.contains("sk-test-123"), "key must not echo back");

    let file = std::fs::read_to_string(&user_env).unwrap();
    assert!(file.contains("OPENCODE_API_KEY=sk-test-123"));

    // The runtime override makes it visible to auth lookups instantly.
    let after: serde_json::Value = client
        .get(format!("{base}/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let zen = after["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "opencode-zen")
        .unwrap();
    assert_eq!(zen["configured"], true);

    // Re-saving replaces the line instead of appending duplicates.
    client
        .put(format!("{base}/config/key"))
        .json(&serde_json::json!({ "provider": "opencode-zen", "key": "sk-test-456" }))
        .send()
        .await
        .unwrap();
    let file = std::fs::read_to_string(&user_env).unwrap();
    assert_eq!(
        file.matches("OPENCODE_API_KEY=").count(),
        1,
        "upsert must replace, not duplicate"
    );
    assert!(file.contains("OPENCODE_API_KEY=sk-test-456"));

    // Revoking strips the key, leaves the rest of the file intact, and
    // flips the provider back to unconfigured.
    std::fs::write(
        &user_env,
        format!(
            "{}\nUNRELATED_VALUE=keep-me\n",
            std::fs::read_to_string(&user_env).unwrap().trim_end()
        ),
    )
    .unwrap();
    let removed: serde_json::Value = client
        .delete(format!("{base}/config/key"))
        .json(&serde_json::json!({ "provider": "opencode-zen" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(removed["env_var"], "OPENCODE_API_KEY");
    assert_eq!(removed["configured"], false);
    assert_eq!(removed["shadowed_by_env"], false);
    let file = std::fs::read_to_string(&user_env).unwrap();
    assert!(!file.contains("OPENCODE_API_KEY"), "key must be gone");
    assert!(
        file.contains("UNRELATED_VALUE=keep-me"),
        "revoking one key must not disturb the rest of the file"
    );
    let after_remove: serde_json::Value = client
        .get(format!("{base}/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let zen = after_remove["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "opencode-zen")
        .unwrap();
    assert_eq!(zen["configured"], false);

    // Revoking a provider we do not know is a value, not a panic.
    let bogus = client
        .delete(format!("{base}/config/key"))
        .json(&serde_json::json!({ "provider": "nope" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bogus.status(), 400);

    // Discovery for an unknown provider is a 404, and for a configured-but
    // -unreachable one it reports the reason rather than a fallback list.
    let unknown_models = client
        .get(format!("{base}/providers/nope/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown_models.status(), 404);

    // Unknown and keyless providers are rejected as values, not panics.
    let unknown = client
        .put(format!("{base}/config/key"))
        .json(&serde_json::json!({ "provider": "nope", "key": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 400);
    let ollama_key = client
        .put(format!("{base}/config/key"))
        .json(&serde_json::json!({ "provider": "ollama", "key": "x" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ollama_key.status(), 400);
}

/// The guided first task is read-only **even when the workspace is
/// full-access** (doc 46 security invariant 5).
///
/// This is the assertion that makes the guarantee real rather than
/// documented: the workspace below is configured `full-access` and
/// trusted, so an ordinary session there would be unsandboxed — and the
/// starter session still comes back ReadOnly, because the cap is applied
/// by `CorePool`'s `capped_by` (a `min`) and carried on the handle the run
/// path uses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_first_task_is_read_only_even_in_a_full_access_workspace() {
    vak_config::paths::isolate_home_for_tests();
    let (base, token, cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);

    // Make the workspace as permissive as it can be.
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "permission_mode = \"full-access\"\n[memory]\nreflection = false\n",
    )
    .unwrap();
    client
        .post(format!("{base}/config/mode"))
        .json(&serde_json::json!({ "mode": "full-access" }))
        .send()
        .await
        .unwrap();
    let config: serde_json::Value = client
        .get(format!("{base}/config"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        config["permission_mode"], "FullAccess",
        "precondition: the workspace really is unrestricted"
    );

    let started: serde_json::Value = client
        .post(format!("{base}/onboarding/first-task"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        started["permission_mode"], "ReadOnly",
        "the guided first task is capped regardless of the workspace mode"
    );
    assert!(
        started["session_id"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "and it produced a real session to run"
    );
    assert!(
        started["prompt"]
            .as_str()
            .is_some_and(|p| p.contains("Do not modify files")),
        "the prompt is ours, not the caller's"
    );
}

/// The trust review describes a workspace **without loading it**.
///
/// Describing a project's privileged config by parsing it through the
/// normal loader would activate the very thing the operator is being
/// asked to decide about (doc 46, Step 2). This asserts the review
/// reports what the file asks for while the Core serving it stays
/// untrusted — so nothing in that file took effect.
#[tokio::test]
async fn a_workspace_review_reports_privileges_without_granting_them() {
    vak_config::paths::isolate_home_for_tests();
    let (base, token, cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);

    // A workspace that asks for execution power.
    let project = cwd.join("hostile");
    std::fs::create_dir_all(project.join(".vak")).unwrap();
    std::fs::write(
        project.join(".vak/config.toml"),
        "permission_mode = \"full-access\"\n[mcp.servers.thing]\ncommand = \"sh\"\n",
    )
    .unwrap();

    let review: serde_json::Value = client
        .post(format!("{base}/onboarding/workspace-review"))
        .json(&serde_json::json!({ "path": project }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(review["requests_privilege"], true);
    assert_eq!(review["trusted"], false, "reviewing is not trusting");
    let named = review["privileges"].as_array().unwrap();
    assert!(
        named.iter().any(|p| p == "a permission mode"),
        "the review names what the project asks for: {named:?}"
    );
    assert!(
        named.iter().any(|p| p == "external tool servers"),
        "{named:?}"
    );

    // The reviewed mode is not in force anywhere: the server's own Core
    // never loaded that file.
    let config: serde_json::Value = client
        .get(format!("{base}/config"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(
        config["permission_mode"], "FullAccess",
        "a reviewed workspace must not have granted itself anything"
    );
}

/// A bot's token round-trips through its own id-addressed route, and is
/// never returned once stored.
///
/// This replaces a test that exercised `PUT/DELETE /config/telegram-token`
/// — a per-surface credential slot that could describe only one bot per
/// transport. A surface is a transport, not a credential slot (AGENTS.md
/// invariant 23), so the slot and its route are gone.
#[tokio::test]
async fn bot_token_storage_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::mem::forget(dir);

    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(cwd.join("home"));
    // Hermetic secret store: never touch the developer's real home.
    let user_env = cwd.join("user-home/.vak/.env");
    std::fs::create_dir_all(user_env.parent().unwrap()).unwrap();
    core.set_user_env_path(user_env.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = client_with(&token);

    let created = client
        .post(format!("{base}/gateway/bots"))
        .json(&serde_json::json!({
            "id": "support",
            "surface": "telegram",
            "label": "Support",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200, "a bot is created explicitly");

    let listed: serde_json::Value = client
        .get(format!("{base}/gateway/bots"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["bots"][0]["token_configured"], false);

    let saved = client
        .put(format!("{base}/gateway/bots/support/token"))
        .json(&serde_json::json!({ "token": "123:abc" }))
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let saved: serde_json::Value = saved.json().await.unwrap();
    assert!(
        !saved.to_string().contains("123:abc"),
        "the token is never returned once stored"
    );

    let after: serde_json::Value = client
        .get(format!("{base}/gateway/bots"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after["bots"][0]["token_configured"], true);

    let removed = client
        .delete(format!("{base}/gateway/bots/support/token"))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), 200);

    let after_remove: serde_json::Value = client
        .get(format!("{base}/gateway/bots"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after_remove["bots"][0]["token_configured"], false);
}

/// `PUT /config/hooks` used to drop a disabled hook from `config.toml`
/// entirely instead of recording it as off (there was nowhere in
/// `[[hooks]]` to say "off" before `HookConfig::enabled` existed), and `GET
/// /config/hooks` always reported `enabled: true` regardless. A console
/// toggle unchecking "enabled" therefore deleted the hook's definition
/// outright rather than pausing it. This locks in both the round-trip and
/// that `get_hooks` reports the *project* file's own hooks, not the
/// runtime-merged effective set (see the vak-config `merge_into` tests for
/// why the merged set must never be resubmitted here).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hooks_roundtrip_preserves_disabled_hooks_instead_of_deleting_them() {
    let (base, token, cwd, _server) = spawn_secured(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::new()),
    }))
    .await;
    let client = client_with(&token);

    let put = client
        .put(format!("{base}/config/hooks"))
        .json(&serde_json::json!({
            "hooks": [
                {"event": "pre_tool_use", "matcher": null, "command": "echo on", "timeout_ms": 5000, "enabled": true},
                {"event": "stop", "matcher": null, "command": "echo off", "timeout_ms": 5000, "enabled": false},
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 200);
    // The success payload counts only the *enabled* hooks written — one of
    // the two, here.
    let put_body: serde_json::Value = put.json().await.unwrap();
    assert_eq!(put_body["count"], 1);

    let got: serde_json::Value = client
        .get(format!("{base}/config/hooks"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let hooks = got["hooks"].as_array().unwrap();
    assert_eq!(
        hooks.len(),
        2,
        "the disabled hook must still be listed: {hooks:?}"
    );
    let off = hooks
        .iter()
        .find(|h| h["command"] == "echo off")
        .expect("disabled hook must survive the round trip");
    assert_eq!(off["enabled"], false);
    let on = hooks
        .iter()
        .find(|h| h["command"] == "echo on")
        .expect("enabled hook must still be present");
    assert_eq!(on["enabled"], true);

    // And it is genuinely on disk, not just echoed back from memory.
    let raw = std::fs::read_to_string(cwd.join(".vak/config.toml")).unwrap();
    assert!(
        raw.contains("echo off"),
        "disabled hook missing from config.toml:\n{raw}"
    );
    assert!(raw.contains("enabled = false"), "config.toml:\n{raw}");

    // A fresh Core loading that same file must not run the disabled hook.
    vak_config::paths::isolate_home_for_tests();
    let restarted = Core::new_with_trust(cwd, true).unwrap();
    let built = vak_core::build_hooks(restarted.config()).unwrap();
    assert_eq!(
        built.len(),
        1,
        "only the enabled hook should become live: {built:?}"
    );
}
