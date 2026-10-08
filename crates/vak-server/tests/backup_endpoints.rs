//! Backup and restore over HTTP (plan M7a-h). A restore moves the store's
//! writer epoch, which fences the whole process, so this test has a
//! binary of its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_roundtrip_reapplies_erasures_and_rejects_self_backup() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("ws");
    std::fs::create_dir_all(&cwd).unwrap();
    let core = vak_core::Core::new(cwd).unwrap();
    let home = core.shared_scope().into_root();
    // Two conversations and a record chain, all under the data home a
    // backup covers whole.
    let mut ids = Vec::new();
    for said in ["keep this", "the zephyrine launch"] {
        let mut log = core.start_session().await.unwrap();
        ids.push(log.header().unwrap().session_id.clone());
        log.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(said),
            meta: None,
        })
        .unwrap();
    }
    let erased = ids[1].clone();
    let shared = core.shared_scope();

    let (router, token) = vak_server::secured_router(core.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let post = |path: &str, body: Value| {
        let request = client
            .post(format!("http://{addr}{path}"))
            .bearer_auth(&token)
            .json(&body)
            .send();
        async move {
            let response = request.await.unwrap();
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.unwrap_or_default())
        }
    };

    // Self-backup is refused both ways, and so is an unknown conflict rule.
    for (path, field) in [
        ("/data/backups", "dest_dir"),
        ("/data/backups/restore", "src_dir"),
    ] {
        let (status, body) = post(path, json!({ field: home.display().to_string() })).await;
        assert_eq!(status, 400, "{path}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("outside the vak home"),
            "{body}"
        );
    }
    assert_eq!(
        reqwest::Client::new()
            .post(format!("http://{addr}/data/backups"))
            .json(&json!({ "dest_dir": "/tmp/nowhere" }))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    let dest = tempfile::tempdir().unwrap();
    let at = dest.path().display().to_string();
    let (status, made) = post(
        "/data/backups",
        json!({ "dest_dir": at, "include_secrets": false }),
    )
    .await;
    assert_eq!(status, 200, "{made}");
    assert!(
        made["manifest"]["file_count"].as_u64().unwrap() >= 2,
        "{made}"
    );
    assert_eq!(made["manifest"]["version"], 2);
    assert_eq!(made["included_secrets"], false);

    // One conversation is erased after the backup was taken.
    vak_core::trash::set(&shared, std::slice::from_ref(&erased), true).unwrap();
    let receipt = core
        .erase_conversation(&erased, None, vak_core::erasure::Cause::Person, None)
        .unwrap();

    let (status, looked) = post("/data/backups/restore/preview", json!({ "src_dir": at })).await;
    assert_eq!(status, 200, "{looked}");
    assert_eq!(
        looked["preview"]["erasures_to_reapply"],
        json!([receipt.id])
    );
    let (status, _) = post(
        "/data/backups/restore",
        json!({ "src_dir": at, "conflict": "overwrite" }),
    )
    .await;
    assert_eq!(status, 400);

    let (status, done) = post(
        "/data/backups/restore",
        json!({ "src_dir": at, "conflict": "skip" }),
    )
    .await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["restart_required"], true);
    assert_eq!(done["report"]["erasures_reapplied"], 1);
    assert!(
        done["report"]["keys_removed"].as_u64().unwrap() >= 1,
        "{done}"
    );

    // What was erased stays erased, and the server says it is fenced.
    assert!(
        vak_core::trash::state(&shared, &erased)
            .unwrap()
            .erased_at
            .is_some()
    );
    let health: Value = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(health.to_string().contains("fenced"), "{health}");
}
