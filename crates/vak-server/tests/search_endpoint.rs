#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::ChatRequest;
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};

struct Empty;

#[async_trait::async_trait]
impl Provider for Empty {
    fn name(&self) -> &str {
        "empty"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(8);
        sink.close_error(LlmError::Parse("unused".into())).await;
        Ok(rx)
    }
}

fn header_for(id: &str, cwd: &Path) -> SessionHeader {
    SessionHeader {
        session_id: id.to_string(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        contract: FrozenContract {
            app_version: "test".into(),
            provider: "scripted".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: String::new(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_endpoint_returns_ranked_hits() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().to_path_buf();

    let path = SessionPath::new_session_file(&home, &cwd, "11111111-past");
    let mut log = SessionLog::create(path, header_for("11111111-past", &cwd)).unwrap();
    log.append_message(vak_session::types::MessageRecord {
        message: vak_llm::Message {
            role: vak_llm::Role::User,
            content: vec![vak_llm::types::ContentBlock::text(
                "the rollout checklist lives in ops/handbook.md",
            )],
        },
        meta: None,
    })
    .unwrap();
    drop(log);

    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(home.clone());
    core.set_provider_instance(Arc::new(Empty));
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::router(core))
            .await
            .unwrap();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let res = client
        .get(format!("{base}/search"))
        .query(&[("q", "rollout checklist")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["session_id"], "11111111-past");
    assert!(
        hits[0]["snippet"].as_str().unwrap().contains("rollout"),
        "snippet: {}",
        hits[0]["snippet"]
    );

    // Garbage queries answer empty, not error.
    let res = client
        .get(format!("{base}/search"))
        .query(&[("q", "")])
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
}
