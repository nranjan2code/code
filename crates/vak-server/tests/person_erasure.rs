//! Erasing one person across chats (plan M7b-e). It rebuilds search and
//! writes the gateway's list of erased people, so it has a binary of its
//! own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

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
    }
}

async fn spawn_gateway(
    provider: Arc<dyn Provider>,
) -> (
    String,
    String,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    // Hermetic against the developer's global config (e.g. reflection=true):
    // pin learning flags off for deterministic scripted flows.
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    let _ = std::fs::write(
        cwd.join(".vak/config.toml"),
        "permission_mode = \"full-access\"\n[memory]\nreflection = false\n[gateway]\nchat_allowlist_open = true\n",
    );
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    // Gateway turns run unattended with AutoDeny; give the fixture bash
    // execution so scripted tool flows behave like an interactive session.
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    // Pin the REAL brokered-tool worker instead of `current_exe()`: under
    // `cargo test` the latter is the test harness itself, which cannot speak
    // the broker protocol and makes brokered bash flake (or hard-fail) under
    // CPU contention. Mirrors the established fixture pattern in
    // scheduler_personal_os.rs / inbox_endpoints.rs / vak-tool-worker.rs.
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    core.set_provider_instance(provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with(core, true);
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

async fn inbound(
    client: &reqwest::Client,
    base: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    client
        .post(format!("{base}/gateway/inbound"))
        .json(&body)
        .send()
        .await
        .unwrap()
}

/// The owner erases one person who wrote to their bots (plan M7b-e): the
/// conversations that are theirs alone go, across chats; a group chat
/// they share with someone else stays and the receipt says so; and they
/// are refused from then on, by a fingerprint that does not name them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn person_erasure_spans_agents_and_chats() {
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            text("one"),
            text("two"),
            text("three"),
            text("four"),
            text("five"),
            text("six"),
        ])),
    });
    let (base, token, cwd, _server) = spawn_gateway(provider).await;
    let client = client_with(&token);
    let say = |chat: &str, sender: &str, text: &str| {
        serde_json::json!({
            "surface": "webhook", "chat": chat, "sender": sender, "text": text, "wait": true
        })
    };
    let mut sessions = std::collections::HashMap::new();
    for (chat, sender, said) in [
        ("dana-dm", "dana", "the zephyrine plan"),
        ("dana-second", "dana", "the quince order"),
        ("team", "dana", "the walnut table"),
        ("team", "omar", "the hazel chairs"),
        ("omar-dm", "omar", "the marigold budget"),
    ] {
        let res = inbound(&client, &base, say(chat, sender, said)).await;
        assert_eq!(res.status(), 200, "{chat} {sender}");
        let body: serde_json::Value = res.json().await.unwrap();
        sessions.insert(chat, body["session_id"].as_str().unwrap().to_string());
    }
    let post = |path: &str, body: serde_json::Value| {
        let request = client.post(format!("{base}{path}")).json(&body).send();
        async move {
            let response = request.await.unwrap();
            let status = response.status().as_u16();
            (
                status,
                response
                    .json::<serde_json::Value>()
                    .await
                    .unwrap_or_default(),
            )
        }
    };
    let dana = serde_json::json!({ "surface": "webhook", "sender": "dana" });

    let (status, looked) = post("/data/erasure/people/preview", dana.clone()).await;
    assert_eq!(status, 200, "{looked}");
    assert_eq!(
        (
            looked["preview"]["conversations"].clone(),
            looked["preview"]["shared_conversations"].clone()
        ),
        (serde_json::json!(2), serde_json::json!(1)),
        "her two own chats, and the one she shares: {looked}"
    );
    // Nothing happens without the preview, or for someone who wrote nothing.
    let (status, refused) = post("/data/erasure/people", dana.clone()).await;
    assert_eq!(
        (status, refused["reason"].as_str()),
        (400, Some("confirmation"))
    );
    let (status, _) = post(
        "/data/erasure/people",
        serde_json::json!({ "surface": "webhook", "sender": "nobody", "digest": "x" }),
    )
    .await;
    assert_eq!(status, 404);

    let mut confirmed = dana.clone();
    confirmed["digest"] = looked["preview"]["digest"].clone();
    let (status, done) = post("/data/erasure/people", confirmed).await;
    assert_eq!(status, 200, "{done}");
    let receipt = &done["receipt"];
    assert_eq!(receipt["scope"], "person");
    assert_eq!(receipt["conversations"], 2);
    assert!(
        receipt["not_reached"][0]
            .as_str()
            .unwrap()
            .starts_with("1 conversation"),
        "{receipt}"
    );
    assert!(
        !done.to_string().contains("dana"),
        "the receipt does not name her: {done}"
    );
    assert_eq!(done["refused_from_now_on"], true);

    // Her own conversations are erased; the shared one and Omar's are not.
    let gone = |chat: &str| {
        let request = client
            .get(format!("{base}/conversations/{}/gone", sessions[chat]))
            .send();
        async move { request.await.unwrap().status().as_u16() }
    };
    assert_eq!(
        (gone("dana-dm").await, gone("dana-second").await),
        (200, 200)
    );
    assert_eq!((gone("team").await, gone("omar-dm").await), (404, 404));
    let search = |word: &str| {
        let request = client.get(format!("{base}/search?q={word}")).send();
        let word = word.to_string();
        async move {
            let body: serde_json::Value = request.await.unwrap().json().await.unwrap();
            !body.to_string().contains("\"results\":[]") && body.to_string().contains(&word)
        }
    };
    assert!(!search("zephyrine").await && !search("quince").await);
    assert!(search("walnut").await && search("marigold").await);

    // She is refused from now on, in any chat; Omar is not.
    let res = inbound(&client, &base, say("dana-dm", "dana", "hello again")).await;
    assert_eq!(res.status(), 403);
    let res = inbound(&client, &base, say("brand-new", "dana", "a new chat")).await;
    assert_eq!(res.status(), 403);
    let res = inbound(&client, &base, say("team", "omar", "still here")).await;
    assert_eq!(res.status(), 200);

    // What is kept to refuse her is a fingerprint, not her id.
    let kept = std::fs::read_to_string(cwd.join("home/gateway/erased-people.json")).unwrap();
    assert!(kept.len() > 60 && !kept.contains("dana"), "{kept}");
}
