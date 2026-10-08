//! Thirty days of use under committing retention (plan M7a-i): what a day
//! leaves behind is bounded by the retention rules, not by how long the
//! install has run, and nothing that is kept is touched.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use std::path::Path;
use vak_core::Core;
use vak_lifecycle::{DataClass, Label};

fn at(path: &Path, when: chrono::DateTime<Utc>) {
    std::fs::File::open(path)
        .unwrap()
        .set_modified(when.into())
        .unwrap();
}

fn days_kept(class: DataClass) -> i64 {
    Label::default_tenant()
        .rule(class)
        .and_then(|rule| rule.delete_after_secs)
        .unwrap()
        / 86_400
}

fn children(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, Iterator::count)
}

#[tokio::test]
async fn thirty_day_soak_stays_within_budget() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let data = vak_config::paths::data_home();
    let logs = vak_config::paths::logs_dir();
    let environments = vak_config::paths::tenant_home_at(&data, vak_config::paths::LOCAL_TENANT)
        .join("environments");
    let executions = vak_config::paths::runtime_dir()
        .join("executions")
        .join("spc_soak")
        .join("vak");
    std::fs::create_dir_all(&logs).unwrap();

    // A conversation made on the first day: retention has no rule for it.
    let mut log = core.start_session().await.unwrap();
    let kept = log.path().to_path_buf();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("the marigold budget"),
        meta: None,
    })
    .unwrap();
    drop(log);

    const DAYS: i64 = 30;
    const EACH: usize = 1024;
    let start = Utc::now() - Duration::days(DAYS - 1);
    let mut removed = 0;
    for day in 0..DAYS {
        let today = start + Duration::days(day);
        // What a day of work leaves: a run's environment, an execution's
        // scratch files, and a rotated log.
        let env = environments.join(format!("env-day-{day:02}"));
        let execution = executions.join(format!("exec-day-{day:02}"));
        for dir in [&env, &execution] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join("work.bin"), vec![0u8; EACH]).unwrap();
            at(&dir.join("work.bin"), today);
            at(dir, today);
        }
        let rotated = logs.join(format!("vak-server.jsonl.{}", day + 1));
        std::fs::write(&rotated, vec![b'x'; EACH]).unwrap();
        at(&rotated, today);

        let tick = core.lifecycle_tick_at(true, today + Duration::hours(1));
        assert!(tick.failed.is_empty(), "day {day}: {:?}", tick.failed);
        removed += tick.committed.len();
    }

    // What is left is what the rules keep, whatever the install's age.
    let end = start + Duration::days(DAYS - 1) + Duration::hours(1);
    let within = |class| (days_kept(class) + 1) as usize;
    assert!(children(&environments) <= within(DataClass::Environment));
    assert!(children(&executions) <= within(DataClass::Execution));
    let rotated_left = std::fs::read_dir(&logs)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains(".jsonl."))
        .count();
    assert!(
        rotated_left <= within(DataClass::Telemetry),
        "{rotated_left}"
    );
    let made = 3 * DAYS as usize;
    let left = children(&environments) + children(&executions) + rotated_left;
    assert_eq!(
        removed,
        made - left,
        "every removal was a recorded transition"
    );
    assert!(
        left * EACH <= made * EACH / 2,
        "thirty days of scratch stay under half of what was written: {left} of {made}"
    );

    // Nothing past its time is waiting, and a second pass does nothing.
    let again = core.lifecycle_tick_at(true, end);
    assert!(again.committed.is_empty() && again.failed.is_empty());
    assert!(
        again
            .plan
            .actions
            .iter()
            .all(|action| !vak_core::lifecycle::COMMITTED.contains(&action.class)),
        "{:?}",
        again.plan.actions
    );

    // What is kept was not touched, and the home verifies.
    let read = vak_session::SessionLog::open_read_only(kept).unwrap();
    assert!(
        read.derive_transcript()
            .iter()
            .any(|message| message.message.text_content().contains("marigold"))
    );
    let integrity = core.data_integrity();
    assert!(integrity.sound(), "{integrity:?}");
}
