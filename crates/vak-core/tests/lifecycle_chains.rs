//! Rows past their retention leave an append-only chain a sealed segment
//! at a time, and the chain still verifies (plan M7a-d part 3).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vak_core::Core;
use vak_lifecycle::DataClass;
use vak_session::chain::RecordChain;

fn days_ago(days: i64) -> String {
    (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
}

fn rows(chain: &RecordChain) -> Vec<String> {
    chain
        .read::<serde_json::Value>()
        .iter()
        .map(|row| row["n"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn expired_rows_leave_a_chain_by_whole_segments_and_it_still_verifies() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let agent = vak_config::scope::AgentScope::new(
        vak_config::paths::data_home().join("agents").join("vak"),
    );
    let inbox = RecordChain::at(agent.inbox());
    let cost = RecordChain::at(agent.cost_log());

    // Two old rows, in an open segment a quiet inbox never filled.
    inbox
        .append(&json!({"ts": days_ago(200), "n": "a"}))
        .unwrap();
    inbox
        .append(&json!({"ts": days_ago(190), "n": "b"}))
        .unwrap();
    // Observing seals nothing and plans nothing for an open segment.
    assert!(core.lifecycle_tick(false).plan.actions.is_empty());
    assert!(inbox.segments()[0].open);

    // Committing seals the aged segment. It is the chain's newest sealed
    // one, which numbers the next, so it stays.
    let first = core.lifecycle_tick(true);
    assert!(first.committed.is_empty());
    assert!(!inbox.segments()[0].open);
    assert_eq!(rows(&inbox), ["a", "b"]);

    // A later, still old, row; the next pass seals it too, and now the
    // first segment is wholly past the inbox's 90 days.
    inbox
        .append(&json!({"ts": days_ago(120), "n": "c"}))
        .unwrap();
    let second = core.lifecycle_tick(true);
    let gone: Vec<(&str, DataClass)> = second
        .committed
        .iter()
        .map(|row| (row.item.as_str(), row.class))
        .collect();
    assert_eq!(gone, [("inbox:vak/1", DataClass::InboxEntry)]);
    assert_eq!(rows(&inbox), ["c"], "the expired rows went together");

    // The chain carries on and verifies from the seal records alone.
    inbox.append(&json!({"ts": days_ago(0), "n": "d"})).unwrap();
    assert_eq!(rows(&inbox), ["c", "d"]);
    let set = vak_storage::segments::SegmentSet::open(inbox.path()).unwrap();
    assert_eq!(
        set.seals().unwrap().len(),
        2,
        "a dropped segment keeps its seal"
    );
    assert!(
        set.read(2, None).is_ok(),
        "segment 2 chains from segment 1's sealed head"
    );
    assert!(set.read(3, None).is_ok());
    let numbers: Vec<u64> = inbox
        .segments()
        .iter()
        .map(|segment| segment.number)
        .collect();
    assert_eq!(numbers, [2, 3]);

    // A segment with one row still inside its keep time stays whole.
    cost.append(&json!({"ts": days_ago(500), "n": "old"}))
        .unwrap();
    cost.append(&json!({"ts": days_ago(100), "n": "kept"}))
        .unwrap();
    core.lifecycle_tick(true);
    cost.append(&json!({"ts": days_ago(40), "n": "later"}))
        .unwrap();
    let third = core.lifecycle_tick(true);
    assert!(third.committed.is_empty(), "{:?}", third.committed);
    assert_eq!(rows(&cost), ["old", "kept", "later"]);
    assert!(core.lifecycle_tick(true).committed.is_empty());
}
