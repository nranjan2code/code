#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Control vocabulary crosses the client boundary as typed projection, not
//! duplicated TypeScript marker lists. Exercise the secured HTTP boundary:
//! every declared runtime control disappears, authored lookalikes survive,
//! and transcript identities still match their presentation items.

use axum::{body::Body, http::Request};
use tower::ServiceExt;
use vak_session::types::MessageRecord;

#[tokio::test]
async fn every_control_kind_is_filtered_before_the_client_without_stripping_authored_text() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(work.path().to_path_buf()).unwrap();
    let mut log = core
        .start_session_with_route("ollama".into(), "test-model".into())
        .await
        .unwrap();
    let id = log.header().unwrap().session_id.clone();
    let question = "Explain [fence-check] and <intent>an authored example</intent>.";
    let answer = "An authored answer can mention [recovery] and completed.";
    log.append_message(MessageRecord {
        message: vak_llm::Message::user_text(question),
        meta: None,
    })
    .unwrap();
    for kind in vak_intent::control::ControlKind::ALL {
        log.append_message(MessageRecord::control(
            kind,
            "opaque runtime-only directive",
        ))
        .unwrap();
    }
    log.append_message(MessageRecord {
        message: vak_llm::Message::assistant(vec![vak_llm::types::ContentBlock::text(answer)]),
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
    let messages = snapshot["transcript"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert!(messages[0].to_string().contains(question));
    assert!(messages[1].to_string().contains(answer));
    assert!(
        !snapshot
            .to_string()
            .contains("opaque runtime-only directive")
    );
    assert_eq!(snapshot["cursor"], snapshot["presentation"]["cursor"]);
    let items = snapshot["presentation"]["items"].as_array().unwrap();
    for entry in snapshot["transcript"]["entries"].as_array().unwrap() {
        assert!(
            items
                .iter()
                .any(|item| item["provenance"]["entry_id"] == entry["entry_id"])
        );
    }
    assert!(items.iter().any(|item| item["fallback_text"] == question));
    assert!(items.iter().any(|item| item["fallback_text"] == answer));
}
