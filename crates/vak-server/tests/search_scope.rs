#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `/search` reads exactly one Agent's memory (AGENTS.md invariant 37),
//! resolved the way `/memory` resolves it: `?agent=`, defaulting to `vak`.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

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

#[tokio::test]
async fn search_reads_the_resolved_agents_own_memory() {
    vak_config::paths::isolate_home_for_tests();
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let core = vak_core::Core::new_with_trust(cwd, true).unwrap();
    core.set_sessions_home(temp.path().join("sessions-home"));
    let app = vak_server::router(core);

    let (status, body) = call(
        &app,
        "POST",
        "/memory",
        json!({"text": "the rollback window opens at noon", "kind": "fact", "tag": "rollback"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, found) = call(&app, "GET", "/search?q=rollback%20window", json!({})).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert!(
        found.to_string().contains("rollback window opens at noon"),
        "the default Agent finds its own note: {found}"
    );

    let (status, scoped) = call(
        &app,
        "GET",
        "/search?q=rollback%20window&agent=vak",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found, scoped, "no agent means `vak`, exactly like /memory");
}
