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
    core.set_shared_scope(vak_config::scope::SharedScope::new(
        temp.path().join("sessions-home"),
    ));
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

/// Revoking an Agent (plan M7a-g, docs/design/74 §2.3) cuts everything it
/// holds that reaches outside, at once: its run stops, its bot's token and
/// its private secrets are gone, its next turn is refused, and it does not
/// resume. Another Agent is untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_cuts_endpoints_within_one_tick() {
    let (app, provider, _temp) = setup().await;
    // What Newsy holds: a bot with a token, and a secret of its own.
    let bot = json!({"id":"newsy-bot","surface":"telegram","label":"Newsy bot","agent_id":"newsy"});
    let (status, made) = call(&app, "POST", "/gateway/bots", bot).await;
    assert!(status.is_success(), "{made}");
    let (status, set) = call(
        &app,
        "PUT",
        "/gateway/bots/newsy-bot/token",
        json!({"token":"1234567890:not-a-real-token"}),
    )
    .await;
    assert!(status.is_success(), "{set}");
    let token_name = set["env_var"].as_str().unwrap().to_owned();
    assert!(vak_config::get_var(&token_name).is_some());
    let secrets = vak_config::paths::agent_home("newsy").join(".env");
    vak_config::credentials::set(&secrets, "NEWSY_FEED_KEY", "not-a-real-key").unwrap();
    let other_secrets = vak_config::paths::agent_home("other").join(".env");
    vak_config::credentials::set(&other_secrets, "OTHER_KEY", "not-a-real-key").unwrap();

    let (_, newsy) = call(&app, "POST", "/agents/newsy/open", json!({})).await;
    let (_, other) = call(&app, "POST", "/agents/other/open", json!({})).await;
    let newsy = newsy["session_id"].as_str().unwrap().to_owned();
    let other = other["session_id"].as_str().unwrap().to_owned();
    start(&app, &newsy).await;
    start(&app, &other).await;
    assert!(wait_until(&app, &newsy, true).await);
    assert!(wait_until(&app, &other, true).await);

    let (status, life) = call(&app, "GET", "/agents/newsy/lifecycle", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{life}");
    assert_eq!(
        (
            life["lifecycle"].clone(),
            life["bots"].clone(),
            life["secrets"].clone()
        ),
        (json!("active"), json!(1), json!(1)),
        "{life}"
    );

    // The Agent's name is typed, or nothing happens.
    let (status, _) = call(
        &app,
        "POST",
        "/agents/newsy/revoke",
        json!({"confirm":"yes"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(running(&app, &newsy).await);
    assert_eq!(
        call(
            &app,
            "POST",
            "/agents/vak/revoke",
            json!({"confirm":"Vakyartha"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    let (status, done) = call(
        &app,
        "POST",
        "/agents/newsy/revoke",
        json!({"confirm":"Newsy"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["lifecycle"], "revoked");
    assert_eq!(
        (
            done["stopped_runs"].clone(),
            done["bot_tokens_removed"].clone(),
            done["secrets_removed"].clone()
        ),
        (json!(1), json!(1), json!(1)),
        "{done}"
    );
    assert!(done["not_removed"].as_array().unwrap().is_empty(), "{done}");
    // By the time the request answers, every credential is gone.
    assert!(
        vak_config::get_var(&token_name).is_none(),
        "the bot's token is removed"
    );
    assert!(vak_config::credentials::list(&secrets).is_empty());
    let kept: Vec<String> = vak_config::credentials::list(&other_secrets)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(kept, ["OTHER_KEY"]);
    assert!(wait_until(&app, &newsy, false).await, "its run stops");
    assert!(
        running(&app, &other).await,
        "another Agent's run is untouched"
    );

    // Its next turn makes no model call, and it does not resume. The other
    // Agent's turn is ended first, so every call counted is Newsy's.
    let (status, _) = call(
        &app,
        "POST",
        &format!("/sessions/{other}/cancel"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(wait_until(&app, &other, false).await);
    let sent = provider.0.load(Ordering::SeqCst);
    start(&app, &newsy).await;
    assert!(wait_until(&app, &newsy, false).await);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(provider.0.load(Ordering::SeqCst), sent);
    let (status, _) = call(&app, "POST", "/agents/newsy/resume", json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Saving its definition as active does not bring it back either.
    let back =
        json!({"agents":[profile("newsy","Newsy"),profile("other","Other")],"scope":"workspace"});
    assert_eq!(
        call(&app, "PUT", "/config/agents", back).await.0,
        StatusCode::CONFLICT
    );
    let (_, life) = call(&app, "GET", "/agents/newsy/lifecycle", json!({})).await;
    assert_eq!(
        (
            life["lifecycle"].clone(),
            life["bots"].clone(),
            life["secrets"].clone()
        ),
        (json!("revoked"), json!(0), json!(0))
    );

    // Revoked, what it holds can be erased: previewed, its name typed.
    let (status, looked) = call(&app, "GET", "/agents/newsy/erasure", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{looked}");
    assert_eq!(looked["confirm"], "Newsy");
    assert_eq!(looked["preview"]["conversations"], 1);
    let digest = looked["preview"]["digest"].as_str().unwrap().to_owned();
    let (status, refused) = call(
        &app,
        "POST",
        "/agents/newsy/erasure",
        json!({"digest": digest, "confirm": "yes"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, refused) = call(&app, "GET", "/agents/other/erasure", json!({})).await;
    assert_eq!(
        (status, refused["reason"].as_str()),
        (StatusCode::CONFLICT, Some("agent_in_use"))
    );
    let (status, done) = call(
        &app,
        "POST",
        "/agents/newsy/erasure",
        json!({"digest": digest, "confirm": "Newsy"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["receipt"]["scope"], "agent");
    assert_eq!(done["receipt"]["conversations"], 1);

    // The server had that conversation open. It no longer serves it, and
    // the other Agent's is untouched.
    let (status, gone) = call(
        &app,
        "GET",
        &format!("/sessions/{newsy}/transcript"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
    let (status, _) = call(
        &app,
        "GET",
        &format!("/sessions/{other}/transcript"),
        json!({}),
    )
    .await;
    assert_ne!(status, StatusCode::NOT_FOUND);
}
