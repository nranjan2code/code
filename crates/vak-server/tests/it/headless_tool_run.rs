//! Regression (live-found): gateway turns with NO SSE subscriber must not
//! be aborted mid-stream when the first event fires. The old broadcast pump
//! treated zero subscribers as fatal, dropped its mpsc receiver, and the
//! agent self-cancelled on the next send — surfacing to chat users as
//! "(aborted)" exactly when the model started a tool call.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

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
        applied: None,
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
        applied: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn headless_tool_turn_survives_without_subscribers() {
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            tool_call("t1", "glob", serde_json::json!({"pattern": "*.md"})),
            text("found some markdown files"),
        ])),
    });

    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    // Hermetic against the developer's global config (e.g. reflection=true):
    // pin learning flags off for deterministic scripted flows.
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let _ = std::fs::write(
        cwd.join(".vak/config.toml"),
        "permission_mode = \"full-access\"\n[gateway]\nchat_allowlist_open = true\n",
    );
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    crate::support::set_provider_without_automatic_topics(&core, provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::gateway_router(core))
            .await
            .unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Deliberately NO /events subscriber: wait:true over inbound only.
    let res = client
        .post(format!("{base}/gateway/inbound"))
        .json(&serde_json::json!({
            "surface": "probe",
            "chat": "headless",
            "text": "What files are in this project?",
            "wait": true,
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["state"], "completed");
    assert_ne!(
        body["text"], "(aborted)",
        "tool event must not kill a subscriber-less run"
    );
    assert_eq!(body["text"], "found some markdown files");

    // The tool really executed: glob result is on the ledger.
    let sid = body["session_id"].as_str().unwrap().to_string();
    let t: serde_json::Value = client
        .get(format!("{base}/sessions/{sid}/transcript"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let raw = serde_json::to_string(&t).unwrap();
    assert!(raw.contains("tool_use") || raw.contains("result"), "{raw}");
}

/// `GET /sessions/{id}/interface` returns what the last turn was bound to
/// (every admitted tool's definition and the deferred names) beside the
/// published capability set with each capability's declared domains: what
/// scripts/capability_coverage.py derives its cases from.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interface_reports_the_bound_tools_and_the_capability_set() {
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![text("hello")])),
    });
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let _ = std::fs::write(
        cwd.join(".vak/config.toml"),
        "[gateway]\nchat_allowlist_open = true\n",
    );
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    crate::support::set_provider_without_automatic_topics(&core, provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::gateway_router(core))
            .await
            .unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .post(format!("{base}/gateway/inbound"))
        .json(&serde_json::json!({
            "surface": "probe",
            "chat": "interface",
            "text": "hi",
            "wait": true,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let sid = body["session_id"].as_str().unwrap().to_string();
    let interface: serde_json::Value = client
        .get(format!("{base}/sessions/{sid}/interface"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bound = &interface["bound"];
    let schemas = bound["interface"]["tool_schemas"].as_array().unwrap();
    let read = schemas
        .iter()
        .find(|tool| tool["name"] == "read")
        .expect("read is bound");
    assert!(read["input_schema"]["properties"].is_object(), "{read}");
    let named: Vec<&str> = bound["core_tool_names"]
        .as_array()
        .unwrap()
        .iter()
        .chain(bound["deferred_tool_names"].as_array().unwrap())
        .filter_map(|name| name.as_str())
        .collect();
    assert_eq!(named.len(), schemas.len());
    let capability = interface["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|capability| capability["id"]["kind"] == "tool" && capability["id"]["name"] == "read")
        .expect("read is in the capability set");
    assert!(capability.get("serves").is_some(), "{capability}");

    let missing = client
        .get(format!("{base}/sessions/no-such-session/interface"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}
