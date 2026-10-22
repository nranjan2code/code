//! A person retries a turn: it is answered again on a new branch from the
//! turn before it, and the next turn's request, recall and the transcript
//! follow that branch, never the turn retried
//! (docs/design/99-turns-that-do-not-poison.md §4 F).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

/// Answers in order, and keeps every request it was sent.
struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<String>>,
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
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests
            .lock()
            .unwrap()
            .push(format!("{:?}", request.messages));
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

fn message(content: Vec<ContentBlock>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason,
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

fn text(t: &str) -> AssistantMessage {
    message(vec![ContentBlock::text(t)], StopReason::EndTurn)
}

struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
    session: String,
}

impl Client {
    async fn transcript(&self) -> serde_json::Value {
        self.http
            .get(format!(
                "{}/sessions/{}/transcript",
                self.base, self.session
            ))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn turns(&self) -> Vec<serde_json::Value> {
        let turns: serde_json::Value = self
            .http
            .get(format!("{}/sessions/{}/context", self.base, self.session))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap_or_default();
        turns["turns"].as_array().cloned().unwrap_or_default()
    }

    /// Waits until the transcript shows `words` and the session is idle
    /// again (a retry of an unknown turn answers 404 only when it is).
    async fn settled(&self, words: &str) -> serde_json::Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let transcript = self.transcript().await;
            if transcript.to_string().contains(words) {
                let idle = self
                    .http
                    .post(format!(
                        "{}/sessions/{}/turns/no-such-turn/retry",
                        self.base, self.session
                    ))
                    .bearer_auth(&self.token)
                    .json(&serde_json::json!({}))
                    .send()
                    .await
                    .unwrap()
                    .status();
                if idle == 404 {
                    return transcript;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "never settled on {words}: {transcript}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn run(&self, prompt: &str) {
        let status = self
            .http
            .post(format!("{}/sessions/{}/run", self.base, self.session))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "prompt": prompt }))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 202);
    }

    async fn retry(&self, turn: &str) -> reqwest::StatusCode {
        self.http
            .post(format!(
                "{}/sessions/{}/turns/{turn}/retry",
                self.base, self.session
            ))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({ "can_show_questions": true }))
            .send()
            .await
            .unwrap()
            .status()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_retried_turn_is_answered_again_and_the_next_turn_follows_it() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    vak_config::spaces::bind(&cwd).unwrap();
    let core = vak_core::Core::new(cwd).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            text("The codename is Heron."),
            text("The route goes over the quarry ridge."),
            // The retry.
            text("The route follows the river path."),
            // The next turn searches for what was said.
            message(
                vec![ContentBlock::ToolUse {
                    id: "c1".into(),
                    name: "recall".into(),
                    input: serde_json::json!({ "query": "ridge" }),
                }],
                StopReason::ToolUse,
            ),
            text("Done."),
        ])),
        requests: Mutex::new(Vec::new()),
    });
    crate::support::set_provider_without_automatic_topics(&core, provider.clone());
    std::mem::forget(dir);

    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let http = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session = http
        .post(format!("{base}/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let client = Client {
        http,
        base,
        token,
        session,
    };

    client.run("what is the codename?").await;
    client.settled("Heron").await;
    client.run("where does the route go?").await;
    client.settled("quarry ridge").await;
    let turns = client.turns().await;
    assert_eq!(turns.len(), 2, "{turns:?}");
    let retried = turns[1]["turn_id"].as_str().unwrap().to_string();

    assert_eq!(client.retry(&retried).await, 202);
    let transcript = client.settled("river path").await;
    assert!(
        !transcript.to_string().contains("quarry"),
        "the retried turn left the transcript: {transcript}"
    );
    let turns = client.turns().await;
    assert_eq!(turns.len(), 2, "{turns:?}");
    assert_ne!(turns[1]["turn_id"].as_str(), Some(retried.as_str()));
    // The retry was asked the same thing, after the turn before it.
    let retry_request = provider.requests.lock().unwrap()[2].clone();
    assert!(retry_request.contains("where does the route go?"));
    assert!(!retry_request.contains("quarry"), "{retry_request}");

    // The turn retried cannot be retried again: it is off the chain.
    assert_eq!(client.retry(&retried).await, 404);

    client.run("what did you say about the route?").await;
    client.settled("Done.").await;
    let requests = provider.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 5, "{requests:#?}");
    // The next turn's request carries the turn retried, not the original.
    assert!(requests[3].contains("river path"), "{}", requests[3]);
    assert!(!requests[3].contains("quarry"), "{}", requests[3]);
    // Recall searched and found nothing of the original answer.
    assert!(!requests[4].contains("quarry"), "{}", requests[4]);
}

/// A conversation at rest holds no writer lock (docs/design/02-sessions.md).
/// After a turn settles another holder may write its ledger; while one does,
/// a turn here is refused and nothing is lost; once it lets go, a turn and a
/// write between turns (setting a turn aside) each take the lock and give it
/// up again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_settled_conversation_holds_no_writer_lock() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(&cwd).unwrap();
    vak_config::spaces::bind(&cwd).unwrap();
    let core = vak_core::Core::new(cwd).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            text("The first answer."),
            text("The second answer."),
        ])),
        requests: Mutex::new(Vec::new()),
    });
    crate::support::set_provider_without_automatic_topics(&core, provider.clone());
    let scope = core.scope();
    let ledger_cwd = core.cwd().clone();
    std::mem::forget(dir);

    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    let http = reqwest::Client::new();
    let base = format!("http://{addr}");
    let session = http
        .post(format!("{base}/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let ledger = scope.session_file(&ledger_cwd, &session);
    let client = Client {
        http,
        base,
        token,
        session,
    };

    client.run("first question").await;
    client.settled("first answer").await;
    let other = vak_session::SessionLog::open(ledger.clone())
        .expect("a settled conversation holds no writer lock");

    let refused = client
        .http
        .post(format!("{}/sessions/{}/run", client.base, client.session))
        .bearer_auth(&client.token)
        .json(&serde_json::json!({ "prompt": "second question" }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(refused, 409, "a turn waits for the other writer");
    drop(other);

    client.run("second question").await;
    client.settled("second answer").await;
    drop(
        vak_session::SessionLog::open(ledger.clone())
            .expect("the lock is given up after the second turn too"),
    );

    let turns = client.turns().await;
    let first = turns[0]["turn_id"].as_str().unwrap().to_string();
    let aside = client
        .http
        .put(format!(
            "{}/sessions/{}/turns/{first}/aside",
            client.base, client.session
        ))
        .bearer_auth(&client.token)
        .json(&serde_json::json!({ "aside": true }))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(aside, 200, "a write between turns takes the lock");
    drop(vak_session::SessionLog::open(ledger).expect("and gives it up when the write is done"));
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
}
