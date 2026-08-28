#![allow(clippy::unwrap_used, clippy::expect_used)]

use vak_config::load_with_trust;

#[test]
fn project_layer_retry_keys_reach_effective_config() {
    // Regression: merge_into dropped run_retry_* when layering project over
    // user config, so the file value silently fell back to the default.
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vak");
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
    let project = dir.path().join(".vak");
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
    std::fs::create_dir_all(dir.path().join(".vak")).unwrap();

    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.theme, "dark");
    assert!(cfg.ui.bell);
    assert!(cfg.ui.keymap.is_empty());
}

#[test]
fn ui_keymap_overrides_merge_project_over_user() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[ui.keymap]\n\"Ctrl-P\" = \"exit\"\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(
        cfg.ui.keymap.get("Ctrl-P").map(String::as_str),
        Some("exit")
    );
}

#[test]
fn ui_layer_overrides_and_unknown_theme_normalizes_to_dark() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vak");
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
    for theme in [
        "neo",
        "rich",
        "teenage",
        "plain",
        "midnight",
        "synthwave",
        "forest",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join(".vak");
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
fn custom_theme_names_resolve_without_warning() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[ui]\ntheme = \"sunset\"\n[ui.themes.sunset]\naccent = \"#ff5500\"\ndim = \"grey\"\nbogus = 1\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.theme, "sunset");
    assert_eq!(
        cfg.ui.themes.get("sunset").and_then(|t| t.get("accent")),
        Some(&"#ff5500".to_string())
    );
    assert!(
        cfg.warnings
            .iter()
            .any(|w| w.contains("themes.sunset.bogus")),
        "unknown theme color must warn: {:?}",
        cfg.warnings
    );
}

#[test]
fn composer_osc52_and_accessibility_layers_apply() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vak");
    std::fs::create_dir_all(&project).unwrap();

    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.composer, "emacs");
    assert!(!cfg.ui.osc52);
    assert!(!cfg.ui.accessibility.plain);

    std::fs::write(
        project.join("config.toml"),
        "[ui]\ncomposer = \"vim\"\nosc52 = true\n[ui.accessibility]\nplain = true\nreduced_motion = true\nbogus = 1\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.composer, "vim");
    assert!(cfg.ui.osc52);
    assert!(cfg.ui.accessibility.plain);
    assert!(cfg.ui.accessibility.reduced_motion);
    assert!(!cfg.ui.accessibility.screen_reader);
    assert!(
        cfg.warnings
            .iter()
            .any(|w| w.contains("accessibility.bogus")),
        "unknown accessibility key must warn: {:?}",
        cfg.warnings
    );

    std::fs::write(project.join("config.toml"), "[ui]\ncomposer = \"dvorak\"\n").unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.ui.composer, "emacs");
    assert!(cfg.warnings.iter().any(|w| w.contains("ui.composer")));
}

#[test]
fn stop_policy_defaults_on_and_layer_overrides_apply() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_with_trust(dir.path(), false).unwrap();
    assert!(cfg.stop_policy.enabled);
    assert_eq!(cfg.stop_policy.max_blocks, 2);

    let project = dir.path().join(".vak");
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

#[test]
fn finops_caps_and_overrides_layer_with_unknown_key_warning() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = load_with_trust(dir.path(), false).unwrap();
    assert!(cfg.finops.max_run_usd.is_none());
    assert!(cfg.finops.price_overrides.is_empty());

    let project = dir.path().join(".vak");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[finops]\nmax_run_usd = 5.0\nmax_day_usd = 20.0\nbogus = 1\n\n[finops.price_overrides.custom-model]\ninput = 1.25\noutput = 6.0\n",
    )
    .unwrap();
    let cfg = load_with_trust(dir.path(), true).unwrap();
    assert_eq!(cfg.finops.max_run_usd, Some(5.0));
    assert_eq!(cfg.finops.max_day_usd, Some(20.0));
    let entry = cfg
        .finops
        .price_overrides
        .get("custom-model")
        .expect("override");
    assert_eq!((entry.input, entry.output), (1.25, 6.0));
    assert!(
        cfg.warnings.iter().any(|w| w.contains("finops.bogus")),
        "unknown finops key must warn: {:?}",
        cfg.warnings
    );

    // Pricing resolution honors the override over the heuristic table.
    assert_eq!(
        vak_config::resolve_usd_per_mtok("custom-model", &cfg.finops.price_overrides),
        Some((1.25, 6.0))
    );
}
