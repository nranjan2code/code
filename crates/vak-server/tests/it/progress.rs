#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use axum::{body::Body, http::Request};
use tower::ServiceExt;
use vak_llm::{ContentBlock, Message};
use vak_session::types::MessageRecord;

#[tokio::test]
async fn the_latest_successful_list_round_trips_through_the_ledger_and_secured_projection() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(work.path().to_path_buf()).unwrap();
    let mut log = core
        .start_session_with_route("ollama".into(), "test-model".into())
        .await
        .unwrap();
    let id = log.header().unwrap().session_id.clone();
    log.append_message(MessageRecord {
        message: Message::user_text("Do the work"),
        meta: None,
    })
    .unwrap();
    for (call, state, reason, failed) in [
        ("p1", "pending", None, false),
        ("p2", "dropped", Some("Source unavailable"), false),
        ("p3", "done", None, true),
    ] {
        let input =
            serde_json::json!({"items":[{"label":"Read source","state":state,"reason":reason}]});
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: call.into(),
                name: "progress".into(),
                input: input.clone(),
            }]),
            meta: None,
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: Message {
                role: vak_llm::Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: call.into(),
                    content: input.to_string(),
                    is_error: failed,
                }],
            },
            meta: None,
        })
        .unwrap();
    }
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text("First useful result.")]),
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord::control(
        vak_intent::control::ControlKind::ProgressReminder,
        "Runtime reminder",
    ))
    .unwrap();
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text("Available answer.")]),
        meta: None,
    })
    .unwrap();
    drop(log);
    core.catalog().unwrap().catch_up().unwrap();
    let (app, token) = vak_server::secured_router(core);
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/sessions/{id}/snapshot"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let lists: Vec<_> = snapshot["presentation"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["content"]["type"] == "work_progress")
        .collect();
    assert_eq!(lists.len(), 1);
    assert_eq!(
        lists[0]["content"]["progress"]["items"][0],
        serde_json::json!({"label":"Read source","state":"dropped","reason":"Source unavailable"})
    );
    assert_eq!(lists[0]["provenance"]["tool_call_id"], "p2");
    let transcript = snapshot["transcript"]["messages"].as_array().unwrap();
    assert!(
        transcript
            .iter()
            .any(|message| message.to_string().contains("First useful result."))
    );
    assert!(
        transcript
            .iter()
            .any(|message| message.to_string().contains("Available answer."))
    );
}
