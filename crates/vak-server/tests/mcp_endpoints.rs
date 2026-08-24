#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_servers_get_put_roundtrip_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let project = cwd.join(".vakcoder");
    std::fs::create_dir_all(&project).unwrap();
    // Pre-existing config with a comment-bearing key we must not destroy.
    std::fs::write(
        project.join("config.toml"),
        "model = \"claude-sonnet-4-5\"\nmax_turns = 12\n",
    )
    .unwrap();

    let core = Core::new_with_trust(cwd.clone(), true).expect("core");
    core.set_sessions_home(dir.path().join("home"));
    core.set_provider_instance(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![text("ok")])),
    }));
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = vak_server::router(core);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Empty at first (this core has no servers of its own; a developer
    // machine's global config may inject some, so only assert shape).
    let res = client
        .get(format!("{base}/config/mcp"))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["servers"].is_object());

    // PUT replaces the whole RUNNING table (global + project) and persists
    // the project-layer file.

    // PUT two servers (one with env + network).
    let res = client
        .put(format!("{base}/config/mcp"))
        .json(&serde_json::json!({
            "servers": {
                "context7": {"command": "npx", "args": ["-y", "@context7/mcp"]},
                "fs": {"command": "./bin/fsd", "env": {"TOKEN": "${FS_TOKEN}"}, "network": true}
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200, "{res:?}");

    // GET reflects the hot-applied table immediately — exactly what was
    // PUT, nothing inherited.
    let res = client
        .get(format!("{base}/config/mcp"))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(
        body["servers"],
        serde_json::json!({
            "context7": {"command": "npx", "args": ["-y", "@context7/mcp"], "env": {}, "network": false},
            "fs": {"command": "./bin/fsd", "args": [], "env": {"TOKEN": "${FS_TOKEN}"}, "network": true},
        })
    );

    // Persisted into the project config without destroying other keys.
    let raw = std::fs::read_to_string(cwd.join(".vakcoder/config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&raw).unwrap();
    assert_eq!(
        parsed["model"],
        toml::Value::from("claude-sonnet-4-5"),
        "existing keys survive"
    );
    assert_eq!(parsed["max_turns"], toml::Value::Integer(12));
    assert_eq!(
        parsed["mcp"]["servers"]["context7"]["command"],
        toml::Value::from("npx")
    );

    // Invalid names are rejected and change nothing.
    let res = client
        .put(format!("{base}/config/mcp"))
        .json(&serde_json::json!({
            "servers": {"bad name!": {"command": "x"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
    let res = client
        .put(format!("{base}/config/mcp"))
        .json(&serde_json::json!({"servers": {"empty-cmd": {"command": "  "}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 400);
}
