#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Writes to the Shared configuration layer (`PATCH /config/global`).
//!
//! That layer is `vak_config::global_path()`, resolved from the process-wide
//! home, and every `Core::new` in the process reads it. These tests write it,
//! so they cannot share a binary with tests that only read it. They used to:
//! a reader there ran under whatever permission mode a writer had just
//! persisted, and one caught between `load_with_trust`'s existence check and
//! its read of the file a writer was removing failed outright. Here the
//! tests take turns, each on a private, empty home.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use vak_core::Core;

/// A private, empty home pinned for one test. The home is process state, so
/// holding one also holds the turn: two tests here never overlap.
struct PrivateHome {
    _root: tempfile::TempDir,
    _turn: std::sync::MutexGuard<'static, ()>,
}

fn private_home() -> PrivateHome {
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let turn = TURN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = tempfile::tempdir().unwrap();
    vak_config::paths::set_home_override(root.path());
    PrivateHome {
        _root: root,
        _turn: turn,
    }
}

async fn patch_global(app: &Router, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/config/global")
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

/// The gateway's own workspace IS the default workspace, so one file
/// serves as both layers and `load_with_trust` skips the project pass.
/// Reporting it as "shadowed" told an operator their change would not
/// take effect when it would — observed live, on a real install, right
/// after the shadow check shipped.
#[tokio::test]
async fn a_global_write_on_the_default_workspace_is_not_its_own_shadow() {
    let _home = private_home();
    let global = vak_config::global_path().unwrap();
    let workspace = vak_config::paths::default_workspace();
    std::fs::create_dir_all(global.parent().expect("parent")).unwrap();
    std::fs::write(&global, "permission_mode = \"read-only\"\n").unwrap();
    vak_core::trust::record(&workspace).unwrap();

    let core = Core::new_with_trust(workspace.clone(), true).unwrap();
    core.set_sessions_home(workspace.join(".sessions"));
    assert_eq!(
        vak_config::project_path(core.cwd()),
        global,
        "this test is only meaningful when the two layers are one file"
    );
    let app = vak_server::router(core.clone());

    let (status, json) = patch_global(&app, json!({ "permission_mode": "workspace-write" })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        json["shadowed_by_project"],
        json!([]),
        "one file cannot shadow itself: {json}"
    );
    assert_eq!(
        core.effective_permission_mode(),
        vak_config::PermissionMode::WorkspaceWrite,
        "and the change must actually be in force"
    );
}

/// The project layer merges last, so a project pin wins. Applying the
/// global value anyway made the running process disagree with what the
/// files resolve to — until a restart put it back, which read as the
/// operator's change being forgotten.
#[tokio::test]
async fn a_global_write_under_a_project_pin_persists_without_taking_effect() {
    let _home = private_home();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
    std::fs::write(
        dir.path().join(".vak/config.toml"),
        "permission_mode = \"read-only\"\n",
    )
    .unwrap();
    vak_core::trust::record(dir.path()).unwrap();
    let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    assert_eq!(
        core.effective_permission_mode(),
        vak_config::PermissionMode::ReadOnly
    );
    let app = vak_server::router(core.clone());

    let (status, _) = patch_global(&app, json!({ "permission_mode": "full-access" })).await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(
        core.effective_permission_mode(),
        vak_config::PermissionMode::ReadOnly,
        "the project pin still decides what this process runs at"
    );
    let global = vak_config::load_with_trust(dir.path(), true).unwrap();
    assert_eq!(
        global.permission_mode,
        vak_config::PermissionMode::ReadOnly,
        "and what the files resolve to agrees"
    );
}

fn install_retired_plugin(root: &std::path::Path, name: &str, scope: vak_plugin::InstallScope) {
    let staging = tempfile::tempdir().unwrap();
    let package = staging.path().join(name);
    std::fs::create_dir_all(package.join("skills/legacy")).unwrap();
    std::fs::write(
        package.join("vak-plugin.json"),
        format!(
            r#"{{"schema":1,"name":"{name}","version":"1.0.0","description":"Old.","license":"MIT","components":{{"skills":["skills"]}}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        package.join("skills/legacy/SKILL.md"),
        "---\nname: legacy\ndescription: Old.\n---\n\nCall `python_eval`.\n",
    )
    .unwrap();
    vak_plugin::PluginStore::new(root)
        .install_local(
            &package,
            vak_plugin::InstallOptions {
                scope,
                allow_unlicensed: false,
            },
        )
        .unwrap();
}

fn network_allow(path: &std::path::Path) -> Value {
    let config: toml::Value = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    serde_json::to_value(&config["plugins"]["network_allow"]).unwrap()
}

/// A user-scope plugin's grant lives in the Shared layer and a workspace
/// plugin's in the project layer; removing either prunes its own layer.
#[tokio::test]
async fn removing_retired_plugins_prunes_the_layer_of_each_scope() {
    let _home = private_home();
    let dir = tempfile::tempdir().unwrap();
    let global = vak_config::global_path().unwrap();
    let project = vak_config::project_path(dir.path());
    install_retired_plugin(
        global.parent().unwrap(),
        "user-eval",
        vak_plugin::InstallScope::User,
    );
    install_retired_plugin(
        project.parent().unwrap(),
        "project-eval",
        vak_plugin::InstallScope::Workspace,
    );
    let grants = "[plugins]\nnetwork_allow = [\"user-eval\", \"project-eval\", \"kept\"]\n";
    std::fs::write(&global, grants).unwrap();
    std::fs::write(&project, grants).unwrap();
    vak_core::trust::record(dir.path()).unwrap();
    let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    let app = vak_server::router(core);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/plugins/retired")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(network_allow(&global), json!(["project-eval", "kept"]));
    assert_eq!(network_allow(&project), json!(["user-eval", "kept"]));
}
