//! The read-only Library API (plan M8.1): `/library` lists artifacts,
//! `/library/{id}` gives one with its history, and a version downloads as
//! its bytes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::artifacts::{ArtifactKind, NewVersion, VersionSource};

#[tokio::test]
async fn library_lists_artifacts_and_their_versions() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let artifacts = core.artifacts();
    let id = artifacts
        .declare(
            "spc_lib",
            "vak",
            "notes/brief.md",
            ArtifactKind::File,
            Some("Brief".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let first = artifacts
        .version(
            id,
            NewVersion {
                parent: None,
                bytes: b"one",
                source: VersionSource::Person,
            },
            None,
            None,
        )
        .unwrap();
    artifacts
        .version(
            id,
            NewVersion {
                parent: Some(first),
                bytes: b"two",
                source: VersionSource::Person,
            },
            None,
            None,
        )
        .unwrap();

    let (router, token) = vak_server::secured_router(core);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let get = |path: String| {
        client
            .get(format!("http://{addr}{path}"))
            .bearer_auth(&token)
            .send()
    };

    let listed: serde_json::Value = get("/library".into()).await.unwrap().json().await.unwrap();
    let entry = listed["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id.to_string())
        .expect("the artifact is listed")
        .clone();
    assert_eq!(entry["name"], "Brief");
    assert_eq!(entry["versions"], 2);
    assert_eq!(entry["siblings"], 1);

    let one: serde_json::Value = get(format!("/library/{id}"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["history"].as_array().unwrap().len(), 2);
    let res = get(format!("/library/{id}/versions/{first}"))
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.bytes().await.unwrap().as_ref(), b"one");
    assert_eq!(get("/library/art_nope".into()).await.unwrap().status(), 404);
    assert_eq!(
        get(format!("/library/{id}/versions/ver_nope"))
            .await
            .unwrap()
            .status(),
        404
    );
}
