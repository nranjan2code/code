//! Sharing an artifact (plan M8.4a, docs/design/82-library.md §8): a share
//! link opens one artifact, and only it, in one role; it shows the current
//! version, and earlier ones only from the version the owner chose; a
//! commenter's comments reach the owner; sharing breaks inheritance; and a
//! revoked link stops working at once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use reqwest::Method;
use serde_json::{Value, json};
use vak_core::artifacts::{ArtifactKind, NewVersion, VersionSource};

#[tokio::test]
async fn a_share_opens_one_artifact_in_its_role_until_revoked() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let shared_scope = core.shared_scope();
    let artifacts = core.artifacts();
    let id = artifacts
        .declare(
            "spc_share",
            "vak",
            "deck.md",
            ArtifactKind::File,
            Some("Board deck".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let draft = |bytes: &'static [u8], parent| NewVersion {
        parent,
        bytes,
        source: VersionSource::Person,
    };
    let first = artifacts
        .version(
            id,
            draft(b"draft one, with a removed secret", None),
            None,
            None,
        )
        .unwrap();
    let second = artifacts
        .version(id, draft(b"final", Some(first)), None, None)
        .unwrap();

    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let call = |method: Method, path: String, bearer: String, body: Option<Value>| {
        let mut request = client
            .request(method, format!("http://{addr}{path}"))
            .bearer_auth(bearer);
        if let Some(body) = body {
            request = request.json(&body);
        }
        async move {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let bytes = response.bytes().await.unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
                bytes,
            )
        }
    };
    let owner = token.clone();

    // A viewer sees only the current version.
    let (status, made, _) = call(
        Method::POST,
        format!("/library/{id}/shares"),
        owner.clone(),
        Some(json!({"name": "Asha", "role": "viewer", "expires_in_hours": 24})),
    )
    .await;
    assert_eq!(status, 201, "{made}");
    let viewer = made["token"].as_str().unwrap().to_string();
    let (status, view, _) =
        call(Method::GET, "/shared/artifact".into(), viewer.clone(), None).await;
    assert_eq!(status, 200, "{view}");
    assert_eq!(view["name"], "Board deck");
    assert_eq!(view["versions"].as_array().unwrap().len(), 1);
    assert_eq!(view["versions"][0]["number"], 2);
    let text = view.to_string();
    assert!(
        !text.contains("session") && !text.contains("conversation"),
        "{text}"
    );
    let (status, _, bytes) = call(
        Method::GET,
        format!("/shared/artifact/versions/{second}"),
        viewer.clone(),
        None,
    )
    .await;
    assert_eq!((status, bytes.as_ref()), (200, b"final".as_ref()));
    let (status, _, _) = call(
        Method::GET,
        format!("/shared/artifact/versions/{first}"),
        viewer.clone(),
        None,
    )
    .await;
    assert_eq!(
        status, 404,
        "an earlier version is not shown without history"
    );
    let (status, _, _) = call(
        Method::POST,
        "/shared/artifact/comments".into(),
        viewer.clone(),
        Some(json!({"version": second.to_string(), "text": "nice"})),
    )
    .await;
    assert_eq!(status, 403, "a viewer cannot comment");
    // The link opens nothing else.
    for path in ["/library", "/sessions", "/search?q=deck"] {
        let (status, _, _) = call(Method::GET, path.into(), viewer.clone(), None).await;
        assert_eq!(status, 403, "{path}");
    }

    // Sharing broke inheritance.
    let grants = vak_core::grants::Grants::at(&shared_scope);
    assert!(
        !grants
            .inherits(&vak_core::grants::GrantObject::Artifact(id))
            .unwrap()
    );

    // A commenter with history from the first version sees both and comments.
    let (_, made, _) = call(
        Method::POST,
        format!("/library/{id}/shares"),
        owner.clone(),
        Some(json!({"name": "Ravi", "role": "commenter", "history_from": first.to_string()})),
    )
    .await;
    let commenter = made["token"].as_str().unwrap().to_string();
    let (_, view, _) = call(
        Method::GET,
        "/shared/artifact".into(),
        commenter.clone(),
        None,
    )
    .await;
    assert_eq!(view["versions"].as_array().unwrap().len(), 2);
    let (status, _, _) = call(
        Method::POST,
        "/shared/artifact/comments".into(),
        commenter.clone(),
        Some(json!({"version": second.to_string(), "text": "Tighten slide two."})),
    )
    .await;
    assert_eq!(status, 204);
    let (_, detail, _) = call(Method::GET, format!("/library/{id}"), owner.clone(), None).await;
    assert_eq!(detail["comments"][0]["author_name"], "Ravi");
    assert_eq!(detail["comments"][0]["text"], "Tighten slide two.");

    // Listing never shows tokens; revoking stops the link at once.
    let (_, listed, _) = call(
        Method::GET,
        format!("/library/{id}/shares"),
        owner.clone(),
        None,
    )
    .await;
    let listed_text = listed.to_string();
    assert_eq!(listed["shares"].as_array().unwrap().len(), 2);
    assert!(!listed_text.contains(&viewer) && !listed_text.contains("token_hash"));
    let ravi = listed["shares"]
        .as_array()
        .unwrap()
        .iter()
        .find(|share| share["name"] == "Ravi")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _, _) = call(
        Method::DELETE,
        format!("/library/{id}/shares/{ravi}"),
        owner.clone(),
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _, _) = call(Method::GET, "/shared/artifact".into(), commenter, None).await;
    assert_eq!(status, 401, "a revoked link opens nothing");
}
