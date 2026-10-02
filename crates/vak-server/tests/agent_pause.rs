//! Pausing an Agent (docs/design/84-worker-questions-and-control.md §7):
//! by default the next turn is refused and a running one finishes; with
//! `stop_running` the Agent's live runs are cancelled now, and no other
//! Agent's are touched.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use vak_llm::{
    EventStream, LlmError, Provider, stream,
    types::{AssistantMessage, ChatRequest},
};

/// Hangs until the run is cancelled, counting how many requests it was sent.
#[derive(Default)]
struct Hung(AtomicUsize, crate::support::CapacityKey);

#[async_trait::async_trait]
impl Provider for Hung {
    fn name(&self) -> &str {
        "hung"
    }

    fn rate_limit_key(&self) -> String {
        self.1.0.clone()
    }
    async fn stream(
        &self,
        _: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let (mut sink, rx) = stream::channel(8);
        sink.push(stream::StreamEvent::Start {
            partial: AssistantMessage::empty("m"),
        });
        tokio::spawn(async move {
            let _keep = sink;
            cancel.cancelled().await;
        });
        Ok(rx)
    }
}

async fn call(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn profile(id: &str, name: &str) -> Value {
    json!({"id":id,"revision":1,"name":name,"character":"vak","personality":"Calm.","behaviour":"Answer concisely.","responsibilities":"Research", "animation":"off","voice":"default"})
}

async fn running(app: &Router, sid: &str) -> bool {
    let (_, sessions) = call(app, "GET", "/sessions", json!({})).await;
    sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["session_id"] == sid && s["running"] == true)
}

async fn wait_until(app: &Router, sid: &str, want_running: bool) -> bool {
    for _ in 0..200 {
        if running(app, sid).await == want_running {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    false
}

async fn start(app: &Router, sid: &str) {
    let (status, body) = call(
        app,
        "POST",
        &format!("/sessions/{sid}/run"),
        json!({"prompt":"work","request_id":uuid::Uuid::now_v7().to_string()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

async fn setup() -> (Router, Arc<Hung>, tempfile::TempDir) {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(temp.path().join("sessions-home"));
    let provider = Arc::new(Hung::default());
    core.set_provider_instance(provider.clone());
    let app = vak_server::router(core);
    let profiles =
        json!({"agents":[profile("newsy","Newsy"),profile("other","Other")],"scope":"workspace"});
    assert_eq!(
        call(&app, "PUT", "/config/agents", profiles).await.0,
        StatusCode::OK
    );
    (app, provider, temp)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_with_stop_running_cancels_only_that_agents_runs() {
    let (app, _provider, _temp) = setup().await;
    let (_, newsy) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    let (_, other) = call(&app, "POST", "/agents/other/open", json!({})).await;
    let newsy = newsy["session_id"].as_str().unwrap().to_owned();
    let other = other["session_id"].as_str().unwrap().to_owned();
    start(&app, &newsy).await;
    start(&app, &other).await;
    assert!(wait_until(&app, &newsy, true).await);
    assert!(wait_until(&app, &other, true).await);

    let (status, body) = call(
        &app,
        "POST",
        "/agents/newsy/pause",
        json!({"stop_running": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["lifecycle"], "paused");
    assert_eq!(body["stopped_runs"], 1);

    assert!(
        wait_until(&app, &newsy, false).await,
        "the paused Agent's run must stop"
    );
    assert!(
        running(&app, &other).await,
        "another Agent's run is untouched"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pause_without_stop_running_lets_a_running_turn_finish_and_refuses_the_next() {
    let (app, provider, _temp) = setup().await;
    let (_, newsy) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    let newsy = newsy["session_id"].as_str().unwrap().to_owned();
    start(&app, &newsy).await;
    assert!(wait_until(&app, &newsy, true).await);

    let (status, body) = call(&app, "POST", "/agents/newsy/pause", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["stopped_runs"], 0);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        running(&app, &newsy).await,
        "a plain pause never stops a running turn"
    );

    // Release the hung turn the supported way, then ask for another.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/sessions/{newsy}/cancel"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(wait_until(&app, &newsy, false).await);
    let sent = provider.0.load(Ordering::SeqCst);
    start(&app, &newsy).await;
    assert!(wait_until(&app, &newsy, false).await);
    assert_eq!(
        provider.0.load(Ordering::SeqCst),
        sent,
        "a paused Agent's next turn makes no model call"
    );

    let (status, body) = call(&app, "POST", "/agents/newsy/resume", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    start(&app, &newsy).await;
    assert!(wait_until(&app, &newsy, true).await);
    let mut resumed = false;
    for _ in 0..200 {
        if provider.0.load(Ordering::SeqCst) > sent {
            resumed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(resumed, "a resumed Agent's turn reaches the model again");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_built_in_agent_and_unknown_agents_cannot_be_paused() {
    let (app, _provider, _temp) = setup().await;
    assert_eq!(
        call(&app, "POST", "/agents/vak/pause", json!({})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, "POST", "/agents/ghost/pause", json!({})).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            "POST",
            "/agents/newsy/pause",
            json!({"state": "deleted"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
