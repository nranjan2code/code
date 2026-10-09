//! The install's retention rules over HTTP (plan M7b-a). They are one
//! Document for the whole home, so this test has a binary of its own.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};

#[tokio::test]
async fn the_owner_changes_a_keep_time_and_a_shorter_one_is_confirmed() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let send = |request: reqwest::RequestBuilder| {
        let token = token.clone();
        async move {
            let response = request.bearer_auth(token).send().await.unwrap();
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.unwrap_or_default())
        }
    };
    let url = |path: &str| format!("http://{addr}{path}");
    assert_eq!(
        reqwest::get(url("/data/rules")).await.unwrap().status(),
        401
    );

    let days = |rules: &Value, kind: &str| {
        rules["label"]["rules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|rule| rule["class"] == kind)
            .unwrap()["delete_after_secs"]
            .as_i64()
            .unwrap()
            / 86_400
    };
    let (status, rules) = send(client.get(url("/data/rules"))).await;
    assert_eq!(status, 200, "{rules}");
    assert_eq!(rules["label"]["id"], "default");
    assert_eq!(days(&rules, "trash"), 30);

    // Longer needs nothing.
    let (status, saved) = send(
        client
            .put(url("/data/rules"))
            .json(&json!({ "keep_days": { "trash": 45 } })),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    assert_eq!(saved["label"]["id"], "install");

    // Shorter is refused until the preview is confirmed.
    let shorter = json!({ "keep_days": { "trash": 10 } });
    let (status, refused) = send(client.put(url("/data/rules")).json(&shorter)).await;
    assert_eq!((status, refused["reason"].as_str()), (409, Some("confirm")));
    let (status, looked) = send(client.post(url("/data/rules/preview")).json(&shorter)).await;
    assert_eq!(status, 200, "{looked}");
    assert_eq!(looked["preview"]["shortened"], json!(["trash"]));
    let digest = looked["preview"]["digest"].as_str().unwrap();
    let (status, saved) = send(
        client
            .put(url("/data/rules"))
            .json(&json!({ "keep_days": { "trash": 10 }, "digest": digest })),
    )
    .await;
    assert_eq!(status, 200, "{saved}");
    let (_, rules) = send(client.get(url("/data/rules"))).await;
    assert_eq!(days(&rules, "trash"), 10);
    assert_eq!(rules["defaults"]["id"], "default");

    // A keep time out of range, or for a kind there is none, is refused.
    let (status, refused) = send(
        client
            .put(url("/data/rules"))
            .json(&json!({ "keep_days": { "trash": 0 } })),
    )
    .await;
    assert_eq!((status, refused["reason"].as_str()), (400, Some("invalid")));
}

/// The owner erases what is kept for a project, through the real router
/// (plan M7b-d): previewed, its name typed, and its folder left alone.
#[tokio::test]
async fn a_projects_data_is_erased_and_its_folder_is_left() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("orchard");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("notes.md"), "the owner's notes").unwrap();
    let core = vak_core::Core::new(folder.clone()).unwrap();
    let mut log = core.start_session().await.unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("plan the orchard walk"),
        meta: None,
    })
    .unwrap();
    drop(log);
    let space = vak_config::spaces::bind(&folder).unwrap();

    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let send = |request: reqwest::RequestBuilder| {
        let token = token.clone();
        async move {
            let response = request.bearer_auth(token).send().await.unwrap();
            let status = response.status().as_u16();
            (status, response.json::<Value>().await.unwrap_or_default())
        }
    };
    let url = format!("http://{addr}/data/erasure/projects/{space}");
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 401);
    let (status, _) =
        send(client.get(format!("http://{addr}/data/erasure/projects/spc_unknown"))).await;
    assert_eq!(status, 404);

    let (status, looked) = send(client.get(&url)).await;
    assert_eq!(status, 200, "{looked}");
    assert_eq!(looked["confirm"], "orchard");
    assert_eq!(looked["preview"]["conversations"], 1);
    let digest = looked["preview"]["digest"].as_str().unwrap();
    let (status, refused) = send(
        client
            .post(&url)
            .json(&json!({ "digest": digest, "confirm": "yes" })),
    )
    .await;
    assert_eq!(
        (status, refused["reason"].as_str()),
        (400, Some("confirmation"))
    );
    let (status, done) = send(
        client
            .post(&url)
            .json(&json!({ "digest": digest, "confirm": "orchard" })),
    )
    .await;
    assert_eq!(status, 200, "{done}");
    assert_eq!(done["receipt"]["scope"], "project");
    assert_eq!(
        std::fs::read_to_string(folder.join("notes.md")).unwrap(),
        "the owner's notes"
    );
    let (_, again) = send(client.get(&url)).await;
    assert_eq!(
        again["preview"]["conversations"], 0,
        "what was erased is not offered again: {again}"
    );
}

/// Erasing everything is the owner's alone, previewed, and refused until
/// the words are typed (plan M7b-f). The erasure itself stops the
/// process, so it is exercised in vak-core's `erasure_install`.
#[tokio::test]
async fn erasing_everything_is_previewed_and_needs_the_words() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/data/erasure/install");
    assert_eq!(reqwest::get(&url).await.unwrap().status(), 401);
    assert_eq!(
        client
            .post(&url)
            .json(&json!({ "digest": "", "confirm": "erase everything" }))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let looked: Value = client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(looked["confirm"], "erase everything");
    assert!(looked["preview"]["digest"].is_string(), "{looked}");
    let refused = client
        .post(&url)
        .bearer_auth(&token)
        .json(&json!({ "digest": looked["preview"]["digest"], "confirm": "yes" }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 400);
    let refused: Value = refused.json().await.unwrap();
    assert_eq!(refused["reason"], "confirmation");
}

/// The keys are the owner's to see and rotate, and no key material is
/// ever returned (plan M7b-g).
#[tokio::test]
async fn the_owner_sees_the_keys_and_rotates_them() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let url = |path: &str| format!("http://{addr}{path}");
    assert_eq!(reqwest::get(url("/data/keys")).await.unwrap().status(), 401);
    assert_eq!(
        client
            .post(url("/data/keys/rotate"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let read = || async {
        client
            .get(url("/data/keys"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    };
    let before = read().await;
    let version = before["version"].as_u64().unwrap();
    let rotated = client
        .post(url("/data/keys/rotate"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(rotated.status(), 200);
    let rotated: Value = rotated.json().await.unwrap();
    assert_eq!(rotated["rotation"]["version"], version + 1);
    let after = read().await;
    assert_eq!(after["version"], version + 1);
    assert_eq!(after["oldest_in_use"], version + 1);
    assert_eq!(
        after["rotations"].as_array().unwrap().len(),
        before["rotations"].as_array().unwrap().len() + 1
    );
    let text = after.to_string();
    assert!(!text.contains("pkcs") && !text.contains("kek"), "{text}");

    // Retiring the earlier keys needs the owner's confirmation.
    let retire = |body: Value| {
        client
            .post(url("/data/keys/retire"))
            .bearer_auth(&token)
            .json(&body)
            .send()
    };
    assert_eq!(retire(serde_json::json!({})).await.unwrap().status(), 400);
    let retired = retire(serde_json::json!({ "confirmed": true }))
        .await
        .unwrap();
    assert_eq!(retired.status(), 200);
    let retired: Value = retired.json().await.unwrap();
    assert_eq!(retired["retirement"]["retired"], version + 1);
    assert_eq!(read().await["retired"], version + 1);
    let again = retire(serde_json::json!({ "confirmed": true }))
        .await
        .unwrap();
    assert_eq!(again.status(), 409);
}
