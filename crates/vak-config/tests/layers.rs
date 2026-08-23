#![allow(clippy::unwrap_used, clippy::expect_used)]

use vak_config::load_with_trust;

#[test]
fn project_layer_retry_keys_reach_effective_config() {
    // Regression: merge_into dropped run_retry_* when layering project over
    // user config, so the file value silently fell back to the default.
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vakcoder");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "provider = \"opencode-zen\"\nmodel = \"x-preview-f-free\"\nrun_retry_attempts = 9\nrun_retry_base_backoff_ms = 1234\n",
    )
    .unwrap();

    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.run_retry_attempts, 9);
    assert_eq!(cfg.run_retry_base_backoff_ms, 1234);
}

#[test]
fn unknown_config_keys_warn_instead_of_failing() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vakcoder");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "provider = \"opencode-zen\"\nmodel = \"m\"\nfuture_key = true\n",
    )
    .unwrap();

    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert!(
        cfg.warnings.iter().any(|w| w.contains("future_key")),
        "typo'd keys must be visible: {:?}",
        cfg.warnings
    );
}

#[test]
fn ui_defaults_apply_when_section_absent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".vakcoder")).unwrap();

    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.theme, "dark");
    assert!(cfg.ui.bell);
}

#[test]
fn ui_layer_overrides_and_unknown_theme_normalizes_to_dark() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vakcoder");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[ui]\ntheme = \"light\"\nbell = false\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.theme, "light");
    assert!(!cfg.ui.bell);

    std::fs::write(
        project.join("config.toml"),
        "[ui]\ntheme = \"neon\"\nbell = true\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.theme, "dark");
    assert!(cfg.ui.bell);
    assert!(
        cfg.warnings.iter().any(|w| w.contains("ui.theme")),
        "unknown theme must warn: {:?}",
        cfg.warnings
    );
}

#[test]
fn designed_ui_themes_are_valid_config_values() {
    for theme in ["neo", "rich", "teenage", "plain"] {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join(".vakcoder");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("config.toml"),
            format!("[ui]\ntheme = \"{theme}\"\n"),
        )
        .unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.ui.theme, theme);
    }
}

#[test]
fn stop_policy_defaults_on_and_layer_overrides_apply() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_with_trust(dir.path(), false).unwrap();
    assert!(cfg.stop_policy.enabled);
    assert_eq!(cfg.stop_policy.max_blocks, 2);

    let project = dir.path().join(".vakcoder");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[stop_policy]\nenabled = true\nmax_blocks = 5\nbogus = 1\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert!(cfg.stop_policy.enabled);
    assert_eq!(cfg.stop_policy.max_blocks, 5);
    assert!(
        cfg.warnings.iter().any(|w| w.contains("stop_policy.bogus")),
        "unknown stop_policy key must warn: {:?}",
        cfg.warnings
    );

    std::fs::write(
        project.join("config.toml"),
        "[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert!(!cfg.stop_policy.enabled);
}
