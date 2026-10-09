//! The second copy over HTTP (data-architecture plan M9-e), through the
//! real router. It changes where this machine stands for the whole home,
//! so it has a binary of its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};

#[tokio::test]
async fn the_owner_sets_up_copies_hands_over_and_takes_back() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let (work, remote) = (dir.path().join("work"), dir.path().join("remote"));
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&remote).unwrap();
    let core = vak_core::Core::new(work).unwrap();
    let mut log = core.start_session().await.unwrap();
    let session = log.header().unwrap().session_id.clone();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("the marigold budget"),
        meta: None,
    })
    .unwrap();
    drop(log);
    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let url = |path: &str| format!("http://{addr}{path}");
    let post = |path: &str, body: Value| {
        let (client, token, url) = (client.clone(), token.clone(), url(path));
        async move {
            let response = client
                .post(url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.unwrap_or_default())
        }
    };
    let status = || async {
        client
            .get(url("/sync"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    };

    // Every route is the owner's.
    assert_eq!(reqwest::get(url("/sync")).await.unwrap().status(), 401);
    for path in [
        "/sync/setup",
        "/sync/now",
        "/sync/handover",
        "/sync/takeover",
        "/sync/forget",
        "/sync/key/export",
        "/sync/key/import",
    ] {
        let unsigned = client.post(url(path)).json(&json!({})).send().await;
        assert_eq!(unsigned.unwrap().status(), 401, "{path}");
    }

    assert_eq!(status().await["configured"], false);
    let (code, refused) = post("/sync/now", json!({})).await;
    assert_eq!(
        (code, refused["reason"].as_str()),
        (404, Some("not_set_up"))
    );
    let missing = dir.path().join("nowhere").to_string_lossy().into_owned();
    let (code, refused) = post("/sync/setup", json!({ "folder": missing })).await;
    assert_eq!(
        (code, refused["reason"].as_str()),
        (409, Some("unreachable"))
    );

    let folder = remote.to_string_lossy().into_owned();
    assert_eq!(
        post("/sync/setup", json!({ "folder": folder })).await.0,
        200
    );
    let (code, pushed) = post("/sync/now", json!({})).await;
    assert_eq!(code, 200, "{pushed}");
    assert!(pushed["copied"].as_u64().unwrap() > 0);
    let now = status().await;
    assert_eq!(
        (
            now["configured"].clone(),
            now["role"].clone(),
            now["unpushed"].clone()
        ),
        (json!(true), json!("holder"), json!(0))
    );

    // The key file comes back to the owner and is sealed.
    let (code, refused) = post("/sync/key/export", json!({ "passphrase": "short" })).await;
    assert_eq!((code, refused["reason"].as_str()), (400, Some("key_file")));
    let (code, made) = post(
        "/sync/key/export",
        json!({ "passphrase": "four quiet lanterns" }),
    )
    .await;
    assert_eq!(code, 200);
    assert!(
        made["file"]
            .as_str()
            .unwrap()
            .starts_with("vakyartha-key-file-1")
    );
    assert!(
        std::fs::read_dir(&remote)
            .unwrap()
            .flatten()
            .all(|entry| { !entry.file_name().to_string_lossy().contains("key") })
    );

    // Handed over, this machine begins no run; taking back needs no pull.
    assert_eq!(post("/sync/handover", json!({})).await.0, 200);
    assert_eq!(status().await["role"], "standing_by");
    let (code, refused) = post(
        &format!("/sessions/{session}/run"),
        json!({ "prompt": "hello" }),
    )
    .await;
    assert_eq!(
        (code, refused["reason"].as_str()),
        (409, Some("standing_by"))
    );
    let (code, refused) = post("/sync/now", json!({})).await;
    assert_eq!(
        (code, refused["reason"].as_str()),
        (409, Some("standing_by"))
    );
    let (code, taken) = post("/sync/takeover", json!({})).await;
    assert_eq!(
        (code, taken["restart_required"].clone()),
        (200, json!(false)),
        "{taken}"
    );
    assert_eq!(status().await["role"], "holder");

    assert_eq!(post("/sync/forget", json!({})).await.0, 200);
    assert_eq!(status().await["configured"], false);
    assert!(
        remote.join("index.json").exists(),
        "the folder is not touched"
    );
}
