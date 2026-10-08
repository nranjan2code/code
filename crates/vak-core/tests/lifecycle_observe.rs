//! The reconciler observes and plans; until a class's commit is enabled it
//! commits nothing (plan M7a-c).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use vak_core::Core;
use vak_lifecycle::{DataClass, Guard, OnExpiry};

const DAY: u64 = 24 * 60 * 60;

fn aged(path: &Path, days: u64) {
    let when = SystemTime::now() - Duration::from_secs(days * DAY);
    // A directory opens for reading only; its owner may still set its time.
    let file = std::fs::File::open(path).unwrap();
    file.set_modified(when).unwrap();
}

fn tree(roots: &[PathBuf]) -> BTreeMap<PathBuf, (u64, SystemTime)> {
    fn walk(path: &Path, out: &mut BTreeMap<PathBuf, (u64, SystemTime)>) {
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return;
        };
        if meta.is_dir() {
            for entry in std::fs::read_dir(path).unwrap().flatten() {
                walk(&entry.path(), out);
            }
        } else {
            out.insert(path.to_path_buf(), (meta.len(), meta.modified().unwrap()));
        }
    }
    let mut out = BTreeMap::new();
    for root in roots {
        walk(root, &mut out);
    }
    out
}

#[test]
fn reconciler_observe_only_commits_nothing() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let data = vak_config::paths::data_home();
    let logs = vak_config::paths::logs_dir();
    let environments = vak_config::paths::tenant_home_at(&data, vak_config::paths::LOCAL_TENANT)
        .join("environments");

    // An environment left by a run that never recorded itself, long ago;
    // one written today; a rotated log past its age; the log being written.
    let (old_env, new_env) = (
        environments.join("orphan-old"),
        environments.join("orphan-new"),
    );
    for env in [&old_env, &new_env] {
        std::fs::create_dir_all(env).unwrap();
        std::fs::write(env.join("notes.txt"), b"twelve bytes").unwrap();
    }
    aged(&old_env.join("notes.txt"), 40);
    aged(&old_env, 40);
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("vak-server.jsonl.3"), b"old lines").unwrap();
    aged(&logs.join("vak-server.jsonl.3"), 30);
    std::fs::write(logs.join("vak-server.jsonl"), b"live lines").unwrap();
    aged(&logs.join("vak-server.jsonl"), 30);

    let roots = [data.clone(), logs.clone(), vak_config::paths::runtime_dir()];
    // The first read of a data home opens its stores (keys, refs); that is
    // opening, not a transition, so it happens before the snapshot.
    let _ = core.data_status();
    let before = tree(&roots);

    let plan = core.lifecycle_plan();
    let status = core.data_status();
    let usage = core.data_usage();
    let again = core.lifecycle_plan();

    assert_eq!(tree(&roots), before, "observing changes no file");

    let due: Vec<(&str, DataClass, OnExpiry)> = plan
        .actions
        .iter()
        .map(|action| (action.item.as_str(), action.class, action.does))
        .collect();
    assert_eq!(
        due,
        [
            ("orphan-old", DataClass::Environment, OnExpiry::Remove),
            ("vak-server.jsonl.3", DataClass::Telemetry, OnExpiry::Remove),
        ]
    );
    assert_eq!(plan.actions[0].bytes, 12);
    assert!(
        plan.guarded
            .iter()
            .any(|kept| kept.item == "vak-server.jsonl" && kept.guard == Guard::Live),
        "the log a service is writing is never planned away"
    );
    assert_eq!(
        again.actions.iter().map(|a| &a.key).collect::<Vec<_>>(),
        plan.actions.iter().map(|a| &a.key).collect::<Vec<_>>(),
        "a second look plans the same transitions"
    );
    // What nobody observed is said, not counted as nothing due.
    assert!(plan.unobserved.contains(&DataClass::Trash));
    assert!(!plan.unobserved.contains(&DataClass::Environment));

    assert_eq!(status.mode, "observe");
    assert_eq!(status.due, 2);
    assert!(usage.bytes > 0 && usage.files > 0);
    assert_eq!(
        usage.rows.iter().map(|row| row.files).sum::<u64>(),
        usage.files
    );
    assert!(
        usage
            .rows
            .iter()
            .any(|row| row.root == "logs" && row.class == "telemetry"),
        "{:?}",
        usage.rows
    );
}
