//! The telemetry readers (plan M5b): `/telemetry/services`,
//! `/telemetry/logs` and `/runs/{id}/spans` sit behind the same
//! authentication as every other route, filter by service, level and run,
//! and lay one run's spans out in the order they started.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;

const RUN: &str = "run_0199b3e0-0000-7000-8000-000000000001";
const OTHER: &str = "run_0199b3e0-0000-7000-8000-000000000002";

fn seed_logs() {
    let server = vak_telemetry::log_path("server");
    std::fs::create_dir_all(server.parent().unwrap()).unwrap();
    let lines = [
        r#"{"ts":"2026-10-06T10:00:00.000+00:00","level":"INFO","service":"server","message":"started"}"#.to_string(),
        format!(
            r#"{{"ts":"2026-10-06T10:00:01.000+00:00","level":"WARN","service":"server","message":"an effect was not sent","trace_id":"{RUN}"}}"#
        ),
        format!(
            r#"{{"ts":"2026-10-06T10:00:02.000+00:00","level":"TRACE","service":"server","event":"span.close","span":"step","duration_ms":400,"trace_id":"{RUN}"}}"#
        ),
        format!(
            r#"{{"ts":"2026-10-06T10:00:03.000+00:00","level":"TRACE","service":"server","event":"span.close","span":"run","duration_ms":3000,"trace_id":"{RUN}"}}"#
        ),
        format!(
            r#"{{"ts":"2026-10-06T10:00:03.000+00:00","level":"TRACE","service":"server","event":"span.close","span":"run","duration_ms":10,"trace_id":"{OTHER}"}}"#
        ),
    ];
    std::fs::write(&server, lines.join("\n") + "\n").unwrap();
    let telegram = vak_telemetry::log_path("telegram");
    std::fs::write(
        telegram,
        r#"{"ts":"2026-10-06T10:00:04.000+00:00","level":"ERROR","service":"telegram","message":"a poll failed"}"#.to_string() + "\n",
    )
    .unwrap();
}

#[tokio::test]
async fn telemetry_readers_filter_and_order() {
    vak_config::paths::isolate_home_for_tests();
    seed_logs();
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(dir.path().to_path_buf()).unwrap();
    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let get = |path: &str| {
        client
            .get(format!("{base}{path}"))
            .bearer_auth(&token)
            .send()
    };

    let refused = client
        .get(format!("{base}/telemetry/logs"))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 401);

    let services: serde_json::Value = get("/telemetry/services")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let services: Vec<&str> = services["services"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    assert!(
        services.contains(&"server") && services.contains(&"telegram"),
        "{services:?}"
    );

    // Warnings and worse, every service, newest first; timings left out.
    let logs: serde_json::Value = get("/telemetry/logs?level=warn")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages: Vec<&str> = logs["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["message"].as_str().unwrap())
        .collect();
    assert_eq!(messages, ["a poll failed", "an effect was not sent"]);

    // One run's lines, with its timings.
    let logs: serde_json::Value = get(&format!(
        "/telemetry/logs?service=server&trace={RUN}&spans=true"
    ))
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(logs["lines"].as_array().unwrap().len(), 3, "{logs}");
    assert_eq!(
        get("/telemetry/logs?service=nope").await.unwrap().status(),
        404
    );

    // The waterfall: this run's spans only, in the order they started.
    let spans: serde_json::Value = get(&format!("/runs/{RUN}/spans"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let spans = spans["spans"].as_array().unwrap();
    let names: Vec<&str> = spans
        .iter()
        .map(|span| span["span"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["run", "step"]);
    assert_eq!(spans[0]["started_at"], "2026-10-06T10:00:00.000Z");
    assert_eq!(spans[1]["started_at"], "2026-10-06T10:00:01.600Z");
    assert_eq!(get("/runs/not-a-run/spans").await.unwrap().status(), 400);
    drop(dir);
}
