#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
async fn catalog_package_installs_disabled_then_uses_existing_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let catalog = temp.path().join("catalog");
    let package = catalog.join("brief");
    std::fs::create_dir_all(package.join("skills/brief")).unwrap();
    std::fs::write(
        catalog.join("marketplace.json"),
        r#"{"name":"local","plugins":[{"name":"brief","source":"./brief","license":"MIT"}]}"#,
    )
    .unwrap();
    std::fs::write(package.join("vak-plugin.json"), r#"{"schema":1,"name":"brief","version":"1.0.0","description":"Brief","license":"MIT","components":{"skills":["skills"]}}"#).unwrap();
    std::fs::write(
        package.join("skills/brief/SKILL.md"),
        "---\nname: brief\ndescription: Prepare a brief.\n---\nPrepare it.\n",
    )
    .unwrap();

    let core = vak_core::Core::new_with_trust(workspace, true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(
        temp.path().join("sessions"),
    ));
    let app = vak_server::router(core);
    let (status, source) = call(
        &app,
        "POST",
        "/plugins/sources",
        json!({"path": catalog, "label": "Local", "scope": "workspace"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{source}");
    let source_id = source["id"].as_str().unwrap();
    let install = json!({"source_id": source_id, "source_scope": "workspace", "name": "brief", "scope": "workspace"});
    let (status, _) = call(&app, "POST", "/plugins/catalog/install", install.clone()).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "disabled catalog must not install"
    );
    let (status, _) = call(
        &app,
        "POST",
        &format!("/plugins/sources/{source_id}/enable?scope=workspace"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, installed) = call(&app, "POST", "/plugins/catalog/install", install).await;
    assert_eq!(status, StatusCode::OK, "{installed}");
    assert_eq!(installed["enabled"], false);
    let (status, enabled) = call(
        &app,
        "POST",
        "/plugins/brief/enable?scope=workspace",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{enabled}");
    assert_eq!(enabled["enabled"], true);
    let (status, disabled) = call(
        &app,
        "POST",
        "/plugins/brief/disable?scope=workspace",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{disabled}");
    assert_eq!(disabled["enabled"], false);
    let next_catalog = temp.path().join("catalog-next");
    let next_package = next_catalog.join("brief");
    std::fs::create_dir_all(next_package.join("skills/brief")).unwrap();
    std::fs::write(
        next_catalog.join("marketplace.json"),
        r#"{"name":"local-next","plugins":[{"name":"brief","source":"./brief","license":"MIT"}]}"#,
    )
    .unwrap();
    std::fs::write(
        next_package.join("vak-plugin.json"),
        r#"{"schema":1,"name":"brief","version":"1.1.0","description":"Brief","license":"MIT","components":{"skills":["skills"]}}"#,
    ).unwrap();
    std::fs::write(
        next_package.join("skills/brief/SKILL.md"),
        "---\nname: brief\ndescription: Prepare a better brief.\n---\nPrepare it.\n",
    )
    .unwrap();
    let (status, next_source) = call(
        &app,
        "POST",
        "/plugins/sources",
        json!({"path": next_catalog, "label": "Local next", "scope": "workspace"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{next_source}");
    let next_id = next_source["id"].as_str().unwrap();
    let (status, _) = call(
        &app,
        "POST",
        &format!("/plugins/sources/{next_id}/enable?scope=workspace"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, updated) = call(&app, "POST", "/plugins/catalog/install", json!({"source_id": next_id, "source_scope": "workspace", "name": "brief", "scope": "workspace", "update": true})).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["version"], "1.1.0");
    assert_eq!(updated["enabled"], false);
    let (status, rolled_back) = call(
        &app,
        "POST",
        "/plugins/brief/rollback?scope=workspace",
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rolled_back}");
    assert_eq!(rolled_back["version"], "1.0.0");
    assert_eq!(rolled_back["enabled"], false);
    let (status, _) = call(&app, "DELETE", "/plugins/brief?scope=workspace", json!({})).await;
    assert_eq!(status, StatusCode::OK);
}
