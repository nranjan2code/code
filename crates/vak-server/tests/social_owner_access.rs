#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;

use vak_core::Core;

async fn spawn() -> (SocketAddr, String) {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path()).unwrap();
    let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    core.set_sessions_home(dir.path().join("sessions"));
    std::mem::forget(dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with_port(core, false, addr.port());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, token)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn social_credential_routes_require_server_authentication() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();

    let denied = client
        .get(format!("http://{addr}/social/connectors?agent=vak"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);

    let accepted = client
        .get(format!("http://{addr}/social/connectors?agent=vak"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_social_addon_registers_its_own_inactive_presentation_pack() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    for (id, adapter) in [
        ("social-reddit", "reddit-data-api-v1"),
        ("social-youtube", "youtube-data-api-v3"),
        ("social-x", "x-api-v2"),
        ("social-linkedin", "linkedin-api-v2"),
    ] {
        let response = client
            .post(format!("http://{addr}/social/connectors/{id}/install"))
            .bearer_auth(&token)
            .json(&serde_json::json!({"scope":"workspace","agent":"vak"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{id}");
        let installed: serde_json::Value = response.json().await.unwrap();
        assert_eq!(installed["native_adapter"], adapter, "{id}");

        let response = client
            .post(format!(
                "http://{addr}/social/connectors/{id}/presentations/install?agent=vak"
            ))
            .bearer_auth(&token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let response_text = response.text().await.unwrap();
        assert_eq!(status, reqwest::StatusCode::OK, "{id}: {response_text}");
        let body: serde_json::Value = serde_json::from_str(&response_text).unwrap();
        assert_eq!(body["registered"], 3, "{id}");
        assert_eq!(body["enabled"], false, "{id}");
    }

    let response = client
        .get(format!("http://{addr}/social/connectors?agent=vak"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    let connectors = body["connectors"].as_array().unwrap();
    assert_eq!(connectors.len(), 4);
    for id in [
        "social-reddit",
        "social-youtube",
        "social-x",
        "social-linkedin",
    ] {
        let connector = connectors
            .iter()
            .find(|connector| connector["id"] == id)
            .unwrap();
        if id == "social-linkedin" {
            assert_eq!(connector["readiness"], "identity_link", "{id}");
        } else {
            assert_eq!(connector["readiness"], "blocked", "{id}");
        }
    }
    let youtube = connectors
        .iter()
        .find(|connector| connector["id"] == "social-youtube")
        .unwrap();
    assert!(youtube["reason"].as_str().unwrap().contains("enable"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn addon_enable_state_gates_youtube_preview_independently() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    let auth = |request: reqwest::RequestBuilder| request.bearer_auth(&token);

    let installed = auth(
        client
            .post(format!(
                "http://{addr}/social/connectors/social-youtube/install"
            ))
            .json(&serde_json::json!({"scope":"workspace","agent":"vak"})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(installed.status(), reqwest::StatusCode::OK);

    let disabled_search = auth(
        client
            .post(format!("http://{addr}/social/youtube/search?agent=vak"))
            .json(&serde_json::json!({"query":"rust","max_results":1})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(disabled_search.status(), reqwest::StatusCode::FORBIDDEN);

    let enabled = auth(client.post(format!(
        "http://{addr}/plugins/social-youtube/enable?scope=workspace&agent=vak"
    )))
    .send()
    .await
    .unwrap();
    assert_eq!(enabled.status(), reqwest::StatusCode::OK);

    let connectors: serde_json::Value =
        auth(client.get(format!("http://{addr}/social/connectors?agent=vak")))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(
        connectors["connectors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|connector| connector["id"] == "social-youtube")
            .unwrap()["readiness"],
        "owner_preview"
    );

    let no_key_search = auth(
        client
            .post(format!("http://{addr}/social/youtube/search?agent=vak"))
            .json(&serde_json::json!({"query":"rust","max_results":1})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(
        no_key_search.status(),
        reqwest::StatusCode::PRECONDITION_FAILED
    );

    let disabled = auth(client.post(format!(
        "http://{addr}/plugins/social-youtube/disable?scope=workspace&agent=vak"
    )))
    .send()
    .await
    .unwrap();
    assert_eq!(disabled.status(), reqwest::StatusCode::OK);

    let disabled_search = auth(
        client
            .post(format!("http://{addr}/social/youtube/search?agent=vak"))
            .json(&serde_json::json!({"query":"rust","max_results":1})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(disabled_search.status(), reqwest::StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linkedin_identity_link_uses_native_pkce_and_requires_an_enabled_local_addon() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    let auth = |request: reqwest::RequestBuilder| request.bearer_auth(&token);

    let unauthorized = client
        .get(format!("http://{addr}/social/linkedin/account?agent=vak"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), reqwest::StatusCode::UNAUTHORIZED);

    let disabled = auth(
        client
            .post(format!("http://{addr}/social/linkedin/connect?agent=vak"))
            .json(&serde_json::json!({})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(disabled.status(), reqwest::StatusCode::FORBIDDEN);

    let installed = auth(
        client
            .post(format!(
                "http://{addr}/social/connectors/social-linkedin/install"
            ))
            .json(&serde_json::json!({"scope":"workspace","agent":"vak"})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(installed.status(), reqwest::StatusCode::OK);
    let enabled = auth(client.post(format!(
        "http://{addr}/plugins/social-linkedin/enable?scope=workspace&agent=vak"
    )))
    .send()
    .await
    .unwrap();
    assert_eq!(enabled.status(), reqwest::StatusCode::OK);

    let no_client_id = auth(
        client
            .post(format!("http://{addr}/social/linkedin/connect?agent=vak"))
            .json(&serde_json::json!({})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(
        no_client_id.status(),
        reqwest::StatusCode::PRECONDITION_FAILED
    );

    let saved = auth(
        client
            .put(format!("http://{addr}/social/linkedin/client-id?agent=vak"))
            .json(&serde_json::json!({"client_id":"vak-local-client_123"})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(saved.status(), reqwest::StatusCode::OK);

    let connected = auth(client.get(format!("http://{addr}/social/linkedin/account?agent=vak")))
        .send()
        .await
        .unwrap();
    assert_eq!(connected.status(), reqwest::StatusCode::OK);
    let status: serde_json::Value = connected.json().await.unwrap();
    assert_eq!(status["connected"], false);

    let started = auth(
        client
            .post(format!("http://{addr}/social/linkedin/connect?agent=vak"))
            .json(&serde_json::json!({})),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(started.status(), reqwest::StatusCode::OK);
    let response: serde_json::Value = started.json().await.unwrap();
    let authorization = url::Url::parse(response["authorization_url"].as_str().unwrap()).unwrap();
    assert_eq!(authorization.host_str(), Some("www.linkedin.com"));
    assert_eq!(authorization.path(), "/oauth/native-pkce/authorization");
    let query = authorization
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        query.get("response_type").map(|value| value.as_ref()),
        Some("code")
    );
    assert_eq!(
        query.get("scope").map(|value| value.as_ref()),
        Some("openid profile")
    );
    assert_eq!(
        query
            .get("code_challenge_method")
            .map(|value| value.as_ref()),
        Some("S256")
    );
    assert_eq!(
        query.get("client_id").map(|value| value.as_ref()),
        Some("vak-local-client_123")
    );
    assert_eq!(query.get("state").map(|value| value.len()), Some(43));
    assert_eq!(
        query.get("code_challenge").map(|value| value.len()),
        Some(43)
    );
    let redirect = url::Url::parse(query.get("redirect_uri").unwrap()).unwrap();
    assert_eq!(redirect.scheme(), "http");
    assert_eq!(redirect.host_str(), Some("127.0.0.1"));
    assert!(redirect.port().is_some());
    assert_eq!(redirect.path(), "/social/linkedin/oauth/callback");

    // Removing app config cancels the pending loopback listener and its
    // authorization state; the test does not leave a background port open.
    let removed = auth(client.delete(format!("http://{addr}/social/linkedin/client-id?agent=vak")))
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), reqwest::StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_social_key_is_inherited_and_an_agent_key_overrides_it() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    let url = |query: &str| format!("http://{addr}/social/youtube/key?{query}");
    let status = |query: String| {
        let client = client.clone();
        let token = token.clone();
        async move {
            client
                .get(query)
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }
    };

    let put = client
        .put(url("agent=vak&scope=user"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "key": "shared-key" }))
        .send()
        .await
        .unwrap();
    assert!(put.status().is_success());

    let shared = status(url("agent=vak&scope=user")).await;
    assert_eq!(shared["configured"], true);
    let agent = status(url("agent=vak")).await;
    assert_eq!(
        (agent["configured"].as_bool(), agent["inherited"].as_bool()),
        (Some(false), Some(true))
    );

    client
        .put(url("agent=vak"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "key": "own-key" }))
        .send()
        .await
        .unwrap();
    let agent = status(url("agent=vak")).await;
    assert_eq!(
        (agent["configured"].as_bool(), agent["inherited"].as_bool()),
        (Some(true), Some(false))
    );

    client
        .delete(url("agent=vak"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    client
        .delete(url("agent=vak&scope=user"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    let agent = status(url("agent=vak")).await;
    assert_eq!(
        (agent["configured"].as_bool(), agent["inherited"].as_bool()),
        (Some(false), Some(false))
    );
}
