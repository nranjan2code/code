#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use vak_llm::{
    EventStream, LlmError, Provider, stream,
    types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage},
};
use vak_session::{Entry, EntryPayload, SessionPath};

#[derive(Default)]
struct Capture(Mutex<Vec<ChatRequest>>);
#[async_trait::async_trait]
impl Provider for Capture {
    fn name(&self) -> &str {
        "capture"
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.0.lock().unwrap().push(request);
        let (mut sink, rx) = stream::channel(8);
        let message = AssistantMessage {
            content: vec![ContentBlock::text("Your request is complete.")],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: "test-model".into(),
        };
        sink.push(stream::StreamEvent::Start {
            partial: message.clone(),
        });
        sink.close_message(message).await;
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
    json!({"id":id,"revision":1,"name":name,"character":"orb","personality":"Use the phrase identity-marker.","behaviour":"Answer concisely.","responsibilities":"Research news", "animation":"off","voice":"default"})
}

async fn run(app: &Router, sid: &str, prompt: &str) {
    let (status, body) = call(
        app,
        "POST",
        &format!("/sessions/{sid}/run"),
        json!({"prompt":prompt,"request_id":uuid::Uuid::now_v7().to_string()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    for _ in 0..200 {
        let (_, sessions) = call(app, "GET", "/sessions", json!({})).await;
        if sessions["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["session_id"] == sid && s["running"] == false)
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("agent did not settle");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_identity_survives_clients_restart_and_followups_without_cross_talk() {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(temp.path().join("sessions-home"));
    let capture = Arc::new(Capture::default());
    core.set_provider_instance(capture.clone());
    let app = vak_server::router(core.clone());
    let profiles =
        json!({"agents":[profile("newsy","Newsy"),profile("other","Other")],"scope":"workspace"});
    assert_eq!(
        call(&app, "PUT", "/config/agents", profiles).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, "POST", "/agents/missing/open", json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&app, "POST", "/agents/vak/open", json!({})).await.0,
        StatusCode::OK
    );
    let (_, a) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    let sid = a["session_id"].as_str().unwrap().to_owned();
    let newsy_home = vak_config::paths::agent_home_at(&core.shared_data_home(), "newsy");
    let ledger = SessionPath::new_session_file(&newsy_home, &cwd, &sid);
    assert!(
        !SessionPath::new_session_file(&core.sessions_home(), &cwd, &sid).exists(),
        "Newsy session must not be in Vak workspace"
    );
    let first = std::fs::read_to_string(ledger)
        .unwrap()
        .lines()
        .next()
        .map(|line| serde_json::from_str::<Entry>(line).unwrap())
        .unwrap();
    let EntryPayload::Header(header) = first.payload else {
        panic!("agent admission ledger must begin with a header");
    };
    let context = header
        .conversation
        .expect("Agent admission must stamp conversation context");
    assert_eq!(context.conversation_id, "agent:newsy:local");
    assert_eq!(context.audience_id, "local");
    assert_eq!(
        context
            .origin
            .as_ref()
            .map(|origin| origin.surface.as_str()),
        Some("desktop")
    );
    let (_, again) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(
        again["session_id"], sid,
        "even header-only conversations must reopen"
    );
    let (_, b) = call(&app, "POST", "/agents/other/open", json!({})).await;
    assert_ne!(b["session_id"], sid);
    run(&app, &sid, "Remember private-newsy-marker").await;
    run(
        &app,
        b["session_id"].as_str().unwrap(),
        "A different request",
    )
    .await;
    {
        let requests = capture.0.lock().unwrap();
        assert!(
            requests
                .iter()
                .any(|r| r.system.as_deref().unwrap_or("").contains("You are Newsy."))
        );
        let other = requests
            .iter()
            .find(|r| r.system.as_deref().unwrap_or("").contains("You are Other."))
            .unwrap();
        assert!(
            !other
                .messages
                .iter()
                .any(|m| m.text_content().contains("private-newsy-marker"))
        );
    }
    drop(app);
    let app = vak_server::router(core.clone());
    let (_, reopened) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(reopened["session_id"], sid);
    run(&app, &sid, "What did I ask you to remember?").await;
    {
        let requests = capture.0.lock().unwrap();
        let last = requests.last().unwrap();
        assert!(
            last.system
                .as_deref()
                .unwrap_or("")
                .contains("identity-marker")
        );
        assert!(
            last.messages
                .iter()
                .any(|m| m.text_content().contains("private-newsy-marker"))
        );
        assert!(
            !last
                .messages
                .iter()
                .any(|m| m.text_content().contains("Use my Agent"))
        );
    }
    let (_, transcript) = call(
        &app,
        "GET",
        &format!("/sessions/{sid}/transcript"),
        json!({}),
    )
    .await;
    assert!(transcript.to_string().contains("private-newsy-marker"));
    // Editing the catalogue never rewrites an admitted conversation's identity.
    call(
        &app,
        "PUT",
        "/config/agents",
        json!({"agents":[profile("newsy","Renamed"),profile("other","Other")]}),
    )
    .await;
    let (_, frozen) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(frozen["agent"]["name"], "Newsy");
    assert_eq!(frozen["agent"]["revision"], 1);
    // Shared writes are not copied into the project layer; effective reads merge.
    call(
        &app,
        "PUT",
        "/config/agents",
        json!({"scope":"user","agents":[profile("shared-helper","Shared helper")]}),
    )
    .await;
    let (_, project_layer) = call(&app, "GET", "/config/agents?scope=workspace", json!({})).await;
    assert!(
        project_layer
            .get("agents")
            .and_then(|v| v.as_array())
            .is_some()
    );
    assert!(!project_layer.to_string().contains("shared-helper"));
    let (_, effective) = call(&app, "GET", "/agents", json!({})).await;
    assert!(effective.get("agents").and_then(|v| v.as_array()).is_some());
    assert!(effective.to_string().contains("shared-helper"));
    // Same identity in another workspace has a separate ledger.
    let other_dir = temp.path().join("other-workspace");
    std::fs::create_dir_all(&other_dir).unwrap();
    let other_core = vak_core::Core::new_with_trust(other_dir, true).unwrap();
    other_core.set_sessions_home(core.shared_data_home());
    other_core.set_provider_instance(capture);
    let other_app = vak_server::router(other_core);
    call(
        &other_app,
        "PUT",
        "/config/agents",
        json!({"agents":[profile("newsy","Newsy")]}),
    )
    .await;
    let (_, isolated) = call(&other_app, "POST", "/agents/newsy/open", json!({})).await;
    assert_ne!(isolated["session_id"], sid);

    // Concurrent process simulation: a second server instance sharing the same workspace
    // and sessions home (e.g. gateway service when desktop application holds the writer lock).
    let concurrent_core = vak_core::Core::new_with_trust(cwd.clone(), true).unwrap();
    concurrent_core.set_sessions_home(core.shared_data_home());
    concurrent_core.set_provider_instance(Arc::new(Capture::default()));
    let concurrent_app = vak_server::router(concurrent_core);

    let (status, opened) = call(&concurrent_app, "POST", "/agents/newsy/open", json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "concurrent open must succeed read-only when locked by desktop"
    );
    assert_eq!(
        opened["session_id"], sid,
        "must match the canonical session id"
    );

    let (transcript_status, transcript) = call(
        &concurrent_app,
        "GET",
        &format!("/sessions/{sid}/transcript"),
        json!({}),
    )
    .await;
    assert_eq!(
        transcript_status,
        StatusCode::OK,
        "transcript must be readable when session is locked"
    );
    assert!(transcript["messages"].as_array().is_some());
}

/// Isolated, credential-free browser fixture. Never reads the operator's home.
/// Run with: cargo test -p vak-server --test agent_chats browser_fixture -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual browser verification server; stops after ten minutes"]
async fn browser_fixture() {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(temp.path().join("sessions"));
    core.set_provider_instance(Arc::new(Capture::default()));
    let app = vak_server::router(core);
    call(
        &app,
        "PUT",
        "/config/agents",
        json!({"agents":[profile("newsy","Newsy"),profile("other","Other")]}),
    )
    .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    println!(
        "AGENT_BROWSER_URL=http://{}/app",
        listener.local_addr().unwrap()
    );
    let server = axum::serve(listener, app).with_graceful_shutdown(async {
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
    });
    server.await.unwrap();
}
