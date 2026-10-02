//! A worker's question over HTTP (docs/design/84-worker-questions-and-control.md
//! §4.5): it is listed on the parent session, answerable once, scoped to that
//! session, and the answer reaches the worker.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use vak_llm::{
    EventStream, LlmError, Provider, stream,
    types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage},
};

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<ChatRequest>>,
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
        _: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(request);
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

fn message(content: Vec<ContentBlock>, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason: stop,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn call(id: &str, name: &str, input: Value) -> AssistantMessage {
    message(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        StopReason::ToolUse,
    )
}

async fn call_api(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_worker_question_is_listed_answered_once_and_reaches_the_worker() {
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
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            call(
                "t1",
                "task",
                json!({"prompt": "total the year", "label": "totals"}),
            ),
            call(
                "a1",
                "ask_parent",
                json!({"question": "Which fiscal year?", "options": ["2025", "2026"]}),
            ),
            message(
                vec![ContentBlock::text("child used it")],
                StopReason::EndTurn,
            ),
            message(vec![ContentBlock::text("all done")], StopReason::EndTurn),
        ])),
        requests: Mutex::new(Vec::new()),
    });
    core.set_provider_instance(provider.clone());
    let app = vak_server::router(core);

    let (_, created) = call_api(&app, "POST", "/sessions", json!({})).await;
    let sid = created["session_id"].as_str().unwrap().to_owned();
    let (status, _) = call_api(
        &app,
        "POST",
        &format!("/sessions/{sid}/run"),
        json!({"prompt": "delegate", "can_show_questions": true, "request_id": uuid::Uuid::now_v7().to_string()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let mut question = Value::Null;
    for _ in 0..200 {
        let (_, listed) = call_api(
            &app,
            "GET",
            &format!("/sessions/{sid}/questions"),
            json!({}),
        )
        .await;
        if let Some(first) = listed["questions"].as_array().and_then(|q| q.first()) {
            question = first.clone();
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(question["question"], "Which fiscal year?", "{question}");
    assert_eq!(question["options"], json!(["2025", "2026"]));
    let qid = question["id"].as_str().unwrap().to_owned();

    // Another session can neither see nor answer it.
    let (_, other) = call_api(&app, "POST", "/sessions", json!({})).await;
    let other = other["session_id"].as_str().unwrap().to_owned();
    assert_eq!(
        call_api(
            &app,
            "POST",
            &format!("/sessions/{other}/questions/{qid}"),
            json!({"text": "2025"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // An empty answer is refused and keeps the question open.
    assert_eq!(
        call_api(
            &app,
            "POST",
            &format!("/sessions/{sid}/questions/{qid}"),
            json!({"text": "  "})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    let (status, _) = call_api(
        &app,
        "POST",
        &format!("/sessions/{sid}/questions/{qid}"),
        json!({"text": "2026"}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        call_api(
            &app,
            "POST",
            &format!("/sessions/{sid}/questions/{qid}"),
            json!({"text": "2025"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "the first answer wins"
    );

    for _ in 0..200 {
        let seen: String = provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .flat_map(|r| r.messages.iter())
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        if seen.contains("Answer from the person: 2026") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the answer never reached the worker");
}

/// A client that does not say it can show a question (a terminal, a script)
/// must not leave a worker waiting for one nobody will see.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_that_cannot_show_questions_ends_the_question_at_once() {
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
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from(vec![
            call("t1", "task", json!({"prompt": "total", "label": "totals"})),
            call("a1", "ask_parent", json!({"question": "Which year?"})),
            message(
                vec![ContentBlock::text("child went on")],
                StopReason::EndTurn,
            ),
            message(vec![ContentBlock::text("all done")], StopReason::EndTurn),
        ])),
        requests: Mutex::new(Vec::new()),
    });
    core.set_provider_instance(provider.clone());
    let app = vak_server::router(core);
    let (_, created) = call_api(&app, "POST", "/sessions", json!({})).await;
    let sid = created["session_id"].as_str().unwrap().to_owned();
    // No `can_show_questions`: the default.
    call_api(
        &app,
        "POST",
        &format!("/sessions/{sid}/run"),
        json!({"prompt": "delegate", "request_id": uuid::Uuid::now_v7().to_string()}),
    )
    .await;
    for _ in 0..200 {
        let seen: String = provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .flat_map(|r| r.messages.iter())
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect();
        if seen.contains("Nobody is available to answer") {
            let (_, listed) = call_api(
                &app,
                "GET",
                &format!("/sessions/{sid}/questions"),
                json!({}),
            )
            .await;
            assert_eq!(listed["questions"], json!([]), "nothing was left waiting");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the worker was left waiting on a client that cannot show a question");
}
