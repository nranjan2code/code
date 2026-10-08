//! A committing pass removes what the plan said was due, records each
//! transition, and leaves everything else (plan M7a-d).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, SystemTime};
use vak_core::Core;
use vak_core::lifecycle::TransitionState;
use vak_lifecycle::DataClass;

fn aged(path: &Path, days: u64) {
    let when = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
    std::fs::File::open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn a_committing_pass_removes_what_was_due_and_records_it() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let data = vak_config::paths::data_home();
    let logs = vak_config::paths::logs_dir();
    let environments = vak_config::paths::tenant_home_at(&data, vak_config::paths::LOCAL_TENANT)
        .join("environments");
    let (old_env, new_env) = (environments.join("env-old"), environments.join("env-new"));
    for env in [&old_env, &new_env] {
        std::fs::create_dir_all(env.join("src")).unwrap();
        std::fs::write(env.join("src/main.txt"), b"twelve bytes").unwrap();
    }
    aged(&old_env, 40);
    std::fs::create_dir_all(&logs).unwrap();
    let (old_log, live_log) = (
        logs.join("vak-server.jsonl.2"),
        logs.join("vak-server.jsonl"),
    );
    for log in [&old_log, &live_log] {
        std::fs::write(log, b"lines").unwrap();
        aged(log, 30);
    }

    // Observing: the plan names both, and both are still there.
    let observed = core.lifecycle_tick(false);
    assert_eq!(observed.mode, "observe");
    assert_eq!(observed.plan.actions.len(), 2);
    assert_eq!(observed.left, 2);
    assert!(observed.committed.is_empty());
    assert!(old_env.exists() && old_log.exists());
    assert!(
        core.lifecycle_transitions(10).is_empty(),
        "observing records nothing"
    );

    // Committing: what was due is gone, and nothing else.
    let done = core.lifecycle_tick(true);
    assert_eq!(done.mode, "commit");
    let gone: Vec<(&str, DataClass)> = done
        .committed
        .iter()
        .map(|row| (row.item.as_str(), row.class))
        .collect();
    assert_eq!(
        gone,
        [
            ("env-old", DataClass::Environment),
            ("vak-server.jsonl.2", DataClass::Telemetry),
        ]
    );
    assert!(done.failed.is_empty());
    assert_eq!(done.reclaimed_bytes, 12 + 5);
    assert!(!old_env.exists() && !old_log.exists());
    assert!(
        new_env.join("src/main.txt").exists(),
        "an environment in its keep time stays"
    );
    assert!(
        live_log.exists(),
        "the log being written stays, however old"
    );

    // Each transition was written before it was made and again after.
    let rows = core.lifecycle_transitions(10);
    assert_eq!(rows.len(), 4);
    for key in ["remove:env-old", "remove:vak-server.jsonl.2"] {
        let states: Vec<TransitionState> = rows
            .iter()
            .rev()
            .filter(|row| row.key == key)
            .map(|row| row.state)
            .collect();
        assert_eq!(
            states,
            [TransitionState::Started, TransitionState::Committed],
            "{key}"
        );
    }

    // A second pass finds nothing to do and records nothing.
    let again = core.lifecycle_tick(true);
    assert!(again.plan.actions.is_empty() && again.committed.is_empty());
    assert_eq!(core.lifecycle_transitions(10).len(), 4);
}
