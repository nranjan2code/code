#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

#[test]
fn learned_rules_persist_reload_and_cannot_shadow_denies() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".vak")).expect("mkdir");

    vak_config::paths::isolate_home_for_tests();
    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).expect("core");
    core.learn_allow_rule("bash(cargo *)").expect("learn");
    core.learn_allow_rule("bash(cargo *)").expect("dedupe ok");

    let file = dir.path().join(".vak/permissions.local.toml");
    let text = fs::read_to_string(&file).expect("file written");
    assert!(text.contains(r#""bash(cargo *)""#));

    // Reload path: a fresh Core picks up the persisted rule.
    vak_config::paths::isolate_home_for_tests();
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
    vak_config::paths::isolate_home_for_tests();
    let untrusted =
        vak_core::Core::new_with_trust(untrusted_dir.path().to_path_buf(), false).expect("u");
    assert!(untrusted.learn_allow_rule("bash(cargo *)").is_err());
}

/// Approvals learned at the same moment all persist, the file always parses,
/// and a key the learner does not own survives every rewrite.
#[test]
fn concurrent_learned_rules_all_persist() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join(".vak")).expect("mkdir");
    let file = dir.path().join(".vak/permissions.local.toml");
    fs::write(&file, "note = \"kept\"\nallow = []\n").expect("seed");

    vak_config::paths::isolate_home_for_tests();
    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).expect("core");

    const ROUNDS: usize = 20;
    const WRITERS: usize = 8;
    let mut expected = Vec::new();
    for round in 0..ROUNDS {
        let specs: Vec<String> = (0..WRITERS)
            .map(|writer| format!("bash(tool{round}x{writer} *)"))
            .collect();
        let barrier = std::sync::Barrier::new(WRITERS);
        std::thread::scope(|scope| {
            for spec in &specs {
                let (core, barrier) = (&core, &barrier);
                scope.spawn(move || {
                    barrier.wait();
                    core.learn_allow_rule(spec).expect("learn");
                });
            }
        });
        expected.extend(specs);

        let text = fs::read_to_string(&file).expect("read");
        let document: toml::Table = toml::from_str(&text)
            .unwrap_or_else(|error| panic!("round {round}: does not parse: {error}\n{text}"));
        assert_eq!(
            document.get("note").and_then(toml::Value::as_str),
            Some("kept")
        );
        let allow: Vec<String> = document["allow"]
            .as_array()
            .expect("allow array")
            .iter()
            .filter_map(|rule| rule.as_str().map(ToOwned::to_owned))
            .collect();
        let mut sorted = expected.clone();
        sorted.sort();
        let mut got = allow.clone();
        got.sort();
        assert_eq!(got, sorted, "round {round}: a learned rule was lost");
        let mut snapshot = core.extra_allow_snapshot();
        snapshot.sort();
        assert_eq!(snapshot, sorted, "round {round}: engine inputs are stale");
    }
    let strays: Vec<_> = fs::read_dir(dir.path().join(".vak"))
        .expect("list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(strays.is_empty(), "temporary files left behind: {strays:?}");
}
