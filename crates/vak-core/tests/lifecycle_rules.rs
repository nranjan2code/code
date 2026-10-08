//! The install's retention rules (plan M7b-a): the owner changes a keep
//! time, a shorter one shows what it would remove and needs confirming,
//! and the rules the owner set are the ones the pass runs under.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};
use vak_core::Core;
use vak_core::lifecycle::{self, RulesError};
use vak_lifecycle::DataClass;

fn environment(name: &str, days_old: u64) -> std::path::PathBuf {
    let dir = vak_config::paths::tenant_home_at(
        &vak_config::paths::data_home(),
        vak_config::paths::LOCAL_TENANT,
    )
    .join("environments")
    .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("work.bin"), vec![0u8; 100]).unwrap();
    let when = SystemTime::now() - Duration::from_secs(days_old * 86_400);
    for path in [dir.join("work.bin"), dir.clone()] {
        std::fs::File::open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }
    dir
}

/// The two tests share one home, so one set of rules: each runs alone
/// and leaves the defaults and no environment behind.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn days(pairs: &[(DataClass, i64)]) -> BTreeMap<DataClass, i64> {
    pairs.iter().copied().collect()
}

#[test]
fn shortened_retention_previews_what_it_removes() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    // Five days old: kept under the default seven days.
    let five = environment("env-five-days", 5);
    let fresh = environment("env-today", 0);
    assert!(core.lifecycle_plan().actions.is_empty());
    assert_eq!(lifecycle::retention_label().id, "default");

    // Three days would remove it, and the preview says so.
    let shorter = days(&[(DataClass::Environment, 3)]);
    let preview = core.retention_preview(&shorter).unwrap();
    assert_eq!(preview.shortened, [DataClass::Environment]);
    assert_eq!(preview.newly_due.len(), 1);
    assert_eq!(
        (
            preview.newly_due[0].class,
            preview.newly_due[0].items,
            preview.newly_due[0].bytes
        ),
        (DataClass::Environment, 1, 100)
    );
    assert!(five.exists(), "a preview removes nothing");

    // Without the preview's digest, or with another, nothing changes.
    assert!(matches!(
        core.set_retention(&shorter, None),
        Err(RulesError::Confirm)
    ));
    assert!(matches!(
        core.set_retention(&shorter, Some("another")),
        Err(RulesError::Confirm)
    ));
    assert_eq!(lifecycle::retention_label().id, "default");
    assert!(matches!(
        core.retention_preview(&days(&[(DataClass::Environment, 0)])),
        Err(RulesError::OutOfRange)
    ));

    // Confirmed, the rules are the install's, and the pass runs under them.
    let label = core.set_retention(&shorter, Some(&preview.digest)).unwrap();
    assert_eq!(label.id, "install");
    assert_eq!(
        lifecycle::retention_label()
            .rule(DataClass::Environment)
            .unwrap()
            .delete_after_secs,
        Some(3 * 86_400)
    );
    assert_eq!(core.data_status().label.id, "install");
    let tick = core.lifecycle_tick(true);
    assert!(tick.failed.is_empty(), "{:?}", tick.failed);
    assert!(!five.exists() && fresh.exists());
    std::fs::remove_dir_all(fresh).unwrap();
    let defaults = BTreeMap::new();
    core.set_retention(&defaults, None).unwrap();
}

#[test]
fn edited_rules_are_the_ones_the_pass_uses() {
    let _serial = SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    // Twenty days old: past the default seven.
    let old = environment("env-twenty-days", 20);
    // A longer keep time needs no confirmation, and it is kept.
    let longer = days(&[(DataClass::Environment, 30), (DataClass::Trash, 45)]);
    let preview = core.retention_preview(&longer).unwrap();
    assert!(preview.shortened.is_empty() && preview.newly_due.is_empty());
    core.set_retention(&longer, None).unwrap();
    assert_eq!(lifecycle::trash_window().num_days(), 45);
    assert_eq!(
        lifecycle::draft_window().num_days(),
        60,
        "the rest keep their defaults"
    );
    let tick = core.lifecycle_tick(true);
    assert!(tick.committed.is_empty() && old.exists());

    // Back to the defaults is shorter again, so it is confirmed; then the
    // pass removes what the default removes.
    let defaults = BTreeMap::new();
    let back = core.retention_preview(&defaults).unwrap();
    assert!(back.shortened.contains(&DataClass::Environment));
    core.set_retention(&defaults, Some(&back.digest)).unwrap();
    assert_eq!(lifecycle::retention_label().id, "default");
    let tick = core.lifecycle_tick(true);
    assert!(tick.failed.is_empty() && !old.exists());
}
