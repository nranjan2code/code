#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Retired-plugin cleanup during seeding. It writes the Shared layer, which
//! every `Core::new` reads, so it runs in a binary of its own.

use vak_plugin::{InstallOptions, InstallScope, PluginStore};

#[test]
fn a_retired_shared_plugin_is_removed_and_its_shared_grant_pruned() {
    let _home = vak_config::paths::isolate_home_for_tests();
    let root = vak_config::paths::default_workspace().join(".vak");
    let staging = tempfile::tempdir().unwrap();
    let package = staging.path().join("legacy-eval");
    std::fs::create_dir_all(package.join("skills/legacy")).unwrap();
    std::fs::write(
        package.join("vak-plugin.json"),
        r#"{"schema":1,"name":"legacy-eval","version":"1.0.0","description":"Old.","license":"MIT","components":{"skills":["skills"]}}"#,
    )
    .unwrap();
    std::fs::write(
        package.join("skills/legacy/SKILL.md"),
        "---\nname: legacy\ndescription: Old.\n---\n\nCall `python_eval`.\n",
    )
    .unwrap();
    PluginStore::new(&root)
        .install_local(
            &package,
            InstallOptions {
                scope: InstallScope::User,
                allow_unlicensed: false,
            },
        )
        .unwrap();
    let global = vak_config::global_path().unwrap();
    std::fs::write(
        &global,
        "[plugins]\nnetwork_allow = [\"legacy-eval\", \"kept\"]\n",
    )
    .unwrap();

    vak_core::seed::seed_shared_capabilities().expect("cleanup succeeds");

    assert!(
        !PluginStore::new(&root)
            .list()
            .unwrap()
            .iter()
            .any(|plugin| plugin.name == "legacy-eval")
    );
    let config: toml::Value = toml::from_str(&std::fs::read_to_string(&global).unwrap()).unwrap();
    assert_eq!(
        config["plugins"]["network_allow"],
        toml::Value::Array(vec!["kept".into()])
    );
}
