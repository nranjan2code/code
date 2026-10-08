//! Executions and checkpoints under the reconciler (plan M7a-d part 2).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, SystemTime};
use vak_core::Core;
use vak_core::checkpoints;
use vak_lifecycle::{DataClass, Reason};

fn aged(path: &Path, days: u64) {
    let when = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
    std::fs::File::open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn due(core: &Core, class: DataClass) -> Vec<(String, Reason)> {
    let mut found: Vec<(String, Reason)> = core
        .lifecycle_plan()
        .actions
        .into_iter()
        .filter(|action| action.class == class)
        .map(|action| (action.item, action.reason))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[test]
fn settled_execution_leaves_nothing() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();

    // Two executions of one Agent in one space, and that Agent's tool cache.
    let agent = vak_config::paths::runtime_dir()
        .join("executions")
        .join("spc_test")
        .join("vak");
    let (old, recent, cache) = (
        agent.join("exec-old"),
        agent.join("exec-recent"),
        agent.join("cache"),
    );
    for dir in [&old, &recent, &cache] {
        std::fs::create_dir_all(dir.join("tmp")).unwrap();
        std::fs::write(dir.join("tmp/scratch.bin"), b"scratch bytes").unwrap();
        std::fs::write(dir.join("draft.md"), b"a draft").unwrap();
    }
    for path in [
        old.join("tmp/scratch.bin"),
        old.join("draft.md"),
        old.join("tmp"),
        old.clone(),
    ] {
        aged(&path, 9);
    }
    aged(&cache, 90);

    assert_eq!(
        due(&core, DataClass::Execution),
        [("spc_test/vak/exec-old".to_string(), Reason::Age)],
        "an execution is due a week after its last write; a cache is not an execution"
    );
    let tick = core.lifecycle_tick(true);
    assert!(tick.failed.is_empty(), "{:?}", tick.failed);
    assert!(!old.exists(), "a settled execution leaves nothing behind");
    assert!(recent.join("draft.md").exists() && cache.join("draft.md").exists());

    // Checkpoints: a session keeps its first and its newest 20.
    let scope = vak_config::scope::AgentScope::new(
        vak_config::paths::data_home().join("agents").join("vak"),
    );
    let objects = core.objects().unwrap();
    for seq in 0..24u32 {
        std::fs::write(work.path().join("f.txt"), format!("v{seq}")).unwrap();
        let (manifest, _) =
            checkpoints::capture(work.path(), &scope, objects.as_ref(), "ses-cp", seq, "turn")
                .unwrap();
        checkpoints::store(&scope, objects.as_ref(), &manifest).unwrap();
    }
    assert_eq!(
        due(&core, DataClass::Checkpoint),
        [
            ("vak/ses-cp/1".to_string(), Reason::Count),
            ("vak/ses-cp/2".to_string(), Reason::Count),
            ("vak/ses-cp/3".to_string(), Reason::Count),
        ]
    );
    let tick = core.lifecycle_tick(true);
    assert!(tick.failed.is_empty(), "{:?}", tick.failed);
    let left: Vec<u32> = checkpoints::list(&scope, "ses-cp")
        .unwrap()
        .iter()
        .map(|manifest| manifest.seq)
        .collect();
    assert_eq!(left.len(), 21);
    assert_eq!(
        &left[..2],
        [0, 4],
        "the first stays, then the newest twenty"
    );
    assert!(
        core.lifecycle_tick(true).committed.is_empty(),
        "nothing more is due"
    );
}
