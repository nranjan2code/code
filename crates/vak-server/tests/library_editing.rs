//! Editing and Put back (plan M8.4b, docs/design/82-library.md §7): an edit
//! made from the current version is a version credited to the person and
//! becomes the file the Agent reads; a file put back after the artifact
//! moved on is a sibling of what changed since, never an overwrite.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use base64::Engine as _;
use serde_json::{Value, json};
use vak_core::artifacts::{ArtifactKind, NewVersion, VersionSource};

#[tokio::test]
async fn edits_become_the_file_and_late_put_backs_become_siblings() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(dir.path()).unwrap();
    std::fs::write(dir.path().join("report.md"), "one").unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let space = vak_session::trace::local::space(core.cwd()).to_string();
    let artifacts = core.artifacts();
    let id = artifacts
        .declare(
            &space,
            "vak",
            "report.md",
            ArtifactKind::File,
            Some("Report".into()),
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
                source: VersionSource::Call {
                    session: String::new(),
                    call: "c".into(),
                },
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
    let post = |body: Value| {
        client
            .post(format!("http://{addr}/library/{id}/versions"))
            .bearer_auth(&token)
            .json(&body)
            .send()
    };

    // The person downloads version 1 to work on it outside.
    let res = client
        .get(format!("http://{addr}/library/{id}/versions/{first}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    // Meanwhile they edit in the app, from the current version.
    let edited: Value = post(json!({"text": "two", "parent": first.to_string()}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (edited["sibling"].clone(), edited["written"].clone()),
        (json!(false), json!(true))
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("report.md")).unwrap(),
        "two"
    );

    // The file they downloaded comes back: it was made from version 1, so
    // it is a sibling of the edit, and the workspace keeps the edit.
    let data = base64::engine::general_purpose::STANDARD.encode("put back");
    let put: Value = post(json!({"data": data}))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        (put["sibling"].clone(), put["written"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("report.md")).unwrap(),
        "two"
    );
    let detail: Value = client
        .get(format!("http://{addr}/library/{id}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let history = detail["history"].as_array().unwrap();
    assert_eq!(history.len(), 3);
    assert!(
        history[1..]
            .iter()
            .all(|v| v["parent"] == first.to_string() && v["from"] == "person")
    );
    assert_eq!(detail["siblings"], 2);

    let bad = post(json!({"text": "x", "parent": "ver_nope"}))
        .await
        .unwrap();
    assert_eq!(bad.status(), 422);
}

#[tokio::test]
async fn a_saved_card_is_kept_in_the_library() {
    vak_config::paths::isolate_home_for_tests();
    let data = vak_config::paths::data_home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    let id = uuid::Uuid::now_v7().to_string();
    let agent_home = vak_config::paths::agent_home_at(&data, "vak");
    let path = vak_session::SessionPath::new_session_file(&agent_home, cwd.path(), &id);
    let header: vak_session::SessionHeader = serde_json::from_value(json!({
        "session_id": id, "created_at": chrono::Utc::now(), "cwd": cwd.path(),
        "agent": vak_core::vak_agent_identity(),
        "contract": {"app_version": "t", "provider": "p", "model": "m", "route_ladder": [],
            "route_objective": "", "route_annotations": [], "system_prompt": "",
            "permission_mode": "read-only", "capabilities": [], "prompt_layers": []}
    }))
    .unwrap();
    let mut log = vak_session::SessionLog::create(path, header).unwrap();
    let payload = json!({"columns": ["Harbour", "Country"], "rows": [["Sydney", "Australia"]]});
    let presented = log
        .append_presentation(vak_session::types::PresentationRecord {
            turn_id: "t".into(),
            source: vak_session::types::PresentationSource::ToolCall {
                tool_use_id: "toolu_1".into(),
            },
            semantic_type: "data_table".into(),
            skill_id: "s".into(),
            skill_version: "1".into(),
            schema_version: 2,
            payload_digest: vak_session::types::payload_digest(&payload),
            payload,
            derived_from: Vec::new(),
            title: "Famous harbours".into(),
            identity_digest: "d".into(),
        })
        .unwrap();
    drop(log);

    let core = vak_core::Core::new(cwd.path().to_path_buf()).unwrap();
    let (router, token) = vak_server::secured_router(core.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let res = reqwest::Client::new()
        .post(format!("http://{addr}/library/cards"))
        .bearer_auth(&token)
        .json(&json!({"session": id, "presentation": presented.id}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 201);
    let saved: Value = res.json().await.unwrap();
    let artifact = core.artifacts().get(saved["id"].as_str().unwrap()).unwrap();
    assert_eq!(artifact.kind, ArtifactKind::Card);
    assert_eq!(artifact.name(), "Famous harbours");
    let head = artifact.head().unwrap();
    assert!(head.saved, "a saved card's version is kept");
    let bytes = core.artifacts().bytes(&artifact, &head.id).unwrap();
    assert!(String::from_utf8(bytes).unwrap().contains("Sydney"));
}

/// A draft goes to the trash and comes back through the real router (plan
/// M7a-e part 4): in the trash it leaves the Library list, and a saved
/// version does not go.
#[tokio::test]
async fn a_draft_is_trashed_and_restored_and_a_saved_version_is_not() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(dir.path()).unwrap();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    let space = vak_session::trace::local::space(core.cwd()).to_string();
    let artifacts = core.artifacts();
    let id = artifacts
        .declare(
            &space,
            "vak",
            "trash-me.md",
            ArtifactKind::File,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let draft = artifacts
        .version(
            id,
            NewVersion {
                parent: None,
                bytes: b"a draft",
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
    let trash = |on: bool| {
        client
            .put(format!("http://{addr}/library/{id}/versions/{draft}/trash"))
            .bearer_auth(&token)
            .json(&json!({ "on": on }))
            .send()
    };
    let listed = |trash: bool| {
        let request = client
            .get(format!("http://{addr}/library?trash={trash}"))
            .bearer_auth(&token)
            .send();
        async move {
            let body: Value = request.await.unwrap().json().await.unwrap();
            body["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|artifact| artifact["id"] == id.to_string())
        }
    };

    let unsigned = client
        .put(format!("http://{addr}/library/{id}/versions/{draft}/trash"))
        .json(&json!({ "on": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsigned.status(), 401);

    assert!(listed(false).await && !listed(true).await);
    assert_eq!(trash(true).await.unwrap().status(), 204);
    assert!(!listed(false).await && listed(true).await);
    // Still whole while it is there.
    let bytes = client
        .get(format!("http://{addr}/library/{id}/versions/{draft}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(bytes.status(), 200);

    assert_eq!(trash(false).await.unwrap().status(), 204);
    assert!(listed(false).await && !listed(true).await);

    // Saved, it is kept: the trash refuses it.
    let saved = client
        .post(format!("http://{addr}/library/{id}/versions/{draft}/save"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 204);
    assert_eq!(trash(true).await.unwrap().status(), 409);
    assert!(listed(false).await);
}
