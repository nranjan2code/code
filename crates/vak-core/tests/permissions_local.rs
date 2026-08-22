#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

#[test]
fn learned_rules_persist_reload_and_cannot_shadow_denies() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".vakcoder")).expect("mkdir");

    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).expect("core");
    core.learn_allow_rule("bash(cargo *)").expect("learn");
    core.learn_allow_rule("bash(cargo *)").expect("dedupe ok");

    let file = dir.path().join(".vakcoder/permissions.local.toml");
    let text = fs::read_to_string(&file).expect("file written");
    assert!(text.contains(r#""bash(cargo *)""#));

    // Reload path: a fresh Core picks up the persisted rule.
    let core2 = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).expect("core2");
    assert_eq!(
        core2.extra_allow_snapshot(),
        vec!["bash(cargo *)".to_string()]
    );

    // Engine behavior: learned allow approves matching calls…
    let cfg = vak_core::vak_config::Config::default();
    let engine = vak_core::build_engine_with(&cfg, &core2.extra_allow_snapshot()).expect("engine");
    let d = engine.evaluate(
        "bash",
        &serde_json::json!({"command": "cargo test"}),
        vak_permission::Mode::WorkspaceWrite,
        dir.path(),
    );
    assert!(matches!(d, vak_permission::Decision::Allow));

    // …but an explicit deny from config still wins by severity.
    let mut cfg_deny = cfg.clone();
    cfg_deny.deny = vec!["bash(cargo publish *)".to_string()];
    let engine2 =
        vak_core::build_engine_with(&cfg_deny, &["bash(cargo *)".to_string()]).expect("engine2");
    let d2 = engine2.evaluate(
        "bash",
        &serde_json::json!({"command": "cargo publish --dry-run"}),
        vak_permission::Mode::WorkspaceWrite,
        dir.path(),
    );
    assert!(matches!(d2, vak_permission::Decision::Deny { .. }));
    // …and the learned allow still approves other cargo invocations under
    // that same deny config.
    let d3 = engine2.evaluate(
        "bash",
        &serde_json::json!({"command": "cargo test"}),
        vak_permission::Mode::WorkspaceWrite,
        dir.path(),
    );
    assert!(matches!(d3, vak_permission::Decision::Allow));

    // Untrusted workspaces refuse to persist grants.
    let untrusted_dir = tempfile::tempdir().expect("tempdir2");
    let untrusted =
        vak_core::Core::new_with_trust(untrusted_dir.path().to_path_buf(), false).expect("u");
    assert!(untrusted.learn_allow_rule("bash(cargo *)").is_err());
}
