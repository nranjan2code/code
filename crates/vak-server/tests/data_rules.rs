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
