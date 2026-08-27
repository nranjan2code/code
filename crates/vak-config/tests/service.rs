use std::{fs, sync::Arc, thread};
use tempfile::tempdir;
use vak_config::{ConfigService, PermissionMode, SandboxBackend, SecretService};

#[test]
fn project_overrides_global_without_erasing_unset_values() {
    let root = tempdir().unwrap();
    let project = tempdir().unwrap();
    fs::write(
        root.path().join("config.toml"),
        "[provider]\nname='global'\nendpoint='https://example'\n[limits]\nmax_turns=10\n",
    )
    .unwrap();
    fs::create_dir(project.path().join(".vakcoder")).unwrap();
    fs::write(
        project.path().join(".vakcoder/project.toml"),
        "[provider]\nname='project'\n[permission]\nmode='workspace_write'\n",
    )
    .unwrap();
    let snapshot = ConfigService::new(root.path(), project.path())
        .load()
        .unwrap();
    assert_eq!(snapshot.config.provider.name.as_deref(), Some("project"));
    assert_eq!(
        snapshot.config.provider.endpoint.as_deref(),
        Some("https://example")
    );
    assert_eq!(snapshot.config.limits.max_turns, Some(10));
    assert_eq!(
        snapshot.config.permission.mode,
        PermissionMode::WorkspaceWrite
    );
}

#[test]
fn unknown_keys_are_warnings_not_failures() {
    let root = tempdir().unwrap();
    fs::write(
        root.path().join("config.toml"),
        "mystery=true\n[provider]\nfuture='x'\n",
    )
    .unwrap();
    let snapshot = ConfigService::new(root.path(), root.path()).load().unwrap();
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|warning| warning.contains("mystery"))
    );
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|warning| warning.contains("provider.future"))
    );
}

#[test]
fn updates_are_atomic_and_revisioned() {
    let root = tempdir().unwrap();
    let service = ConfigService::new(root.path(), root.path());
    let first = service
        .update(|config| config.model.name = Some("one".into()))
        .unwrap();
    let second = service
        .update(|config| config.model.name = Some("two".into()))
        .unwrap();
    assert_eq!((first.revision, second.revision), (1, 2));
    assert_eq!(
        service.load().unwrap().config.model.name.as_deref(),
        Some("two")
    );
    assert!(!root.path().join(".config.toml.tmp").exists());
}

#[test]
fn concurrent_updates_do_not_lose_valid_files() {
    let root = tempdir().unwrap();
    let service = Arc::new(ConfigService::new(root.path(), root.path()));
    let mut threads = Vec::new();
    for index in 0..8 {
        let service = Arc::clone(&service);
        threads.push(thread::spawn(move || {
            loop {
                let revision = service.load().unwrap().revision;
                match service
                    .update_global(revision, |config| config.limits.max_turns = Some(index))
                {
                    Ok(snapshot) => break snapshot.revision,
                    Err(vak_config::ConfigError::RevisionConflict { .. }) => continue,
                    Err(error) => panic!("unexpected config error: {error}"),
                }
            }
        }));
    }
    let revisions: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let snapshot = service.load().unwrap();
    assert_eq!(snapshot.revision, 8);
    assert!(revisions.iter().all(|revision| *revision > 0));
    assert!(snapshot.config.limits.max_turns.is_some());
}

#[test]
fn secrets_are_locked_and_replaced_atomically() {
    let root = tempdir().unwrap();
    let secrets = SecretService::new(root.path());
    secrets.set("API_KEY", "first").unwrap();
    assert_eq!(secrets.get("API_KEY").unwrap().as_deref(), Some("first"));
    secrets.set("API_KEY", "second").unwrap();
    assert_eq!(secrets.get("API_KEY").unwrap().as_deref(), Some("second"));
    assert!(secrets.remove("API_KEY").unwrap());
    assert_eq!(secrets.get("API_KEY").unwrap(), None);
}

#[test]
fn typed_defaults_are_safe() {
    let root = tempdir().unwrap();
    let config = ConfigService::new(root.path(), root.path())
        .load()
        .unwrap()
        .config;
    assert_eq!(config.permission.mode, PermissionMode::ReadOnly);
    assert_eq!(config.sandbox.backend, SandboxBackend::Seatbelt);
}

#[test]
fn dotenv_overrides_toml_and_process_env_wins() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("config.toml"), "[provider]\nname='toml'\n").unwrap();
    fs::write(root.path().join(".env"), "VAKCODER_PROVIDER=\"dotenv\"\n").unwrap();
    let service = ConfigService::new(root.path(), root.path());
    assert_eq!(
        service.load().unwrap().config.provider.name.as_deref(),
        Some("dotenv")
    );
}

#[test]
fn dotenv_round_trips_special_values_and_is_private() {
    let root = tempdir().unwrap();
    let secrets = SecretService::new(root.path());
    let value = "a=b # c\\n\"quoted\"";
    secrets.set("TOKEN", value).unwrap();
    assert_eq!(secrets.get("TOKEN").unwrap().as_deref(), Some(value));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(secrets.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn revision_conflict_and_project_scope_are_explicit() {
    let root = tempdir().unwrap();
    let project = tempdir().unwrap();
    let service = ConfigService::new(root.path(), project.path());
    assert!(service.update_global(1, |_| {}).is_err());
    let snapshot = service
        .update_project(project.path(), 0, |config| {
            config.model.name = Some("project".into())
        })
        .unwrap();
    assert_eq!(snapshot.config.model.name.as_deref(), Some("project"));
    assert!(
        service
            .update_project(root.path(), snapshot.revision, |_| {})
            .is_err()
    );
}
