//! Records derived from a conversation say which conversation and turn
//! they came from (docs/design/73 §4); the field is additive.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::{learning, memory};
use vak_session::trace::DerivedFrom;

#[test]
fn derived_writes_record_provenance() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().join("ws");
    std::fs::create_dir_all(&cwd).unwrap();

    let written =
        memory::append_note_from_turn(&home, &cwd, "fact", "t", "ses-1", Some("trn-9"), "body")
            .unwrap();
    let expected = DerivedFrom {
        conversation: "ses-1".into(),
        turn: Some("trn-9".into()),
    };
    assert_eq!(written.derived_from.as_ref(), Some(&expected));
    let listed = memory::list_notes(&home, &cwd);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].derived_from.as_ref(), Some(&expected));
    assert_eq!(listed[0].id, written.id);
    let plain = memory::append_note(&home, &cwd, "fact", "t", "ses-1", "other").unwrap();
    assert!(plain.derived_from.is_none());

    memory::append_note_from_turn(
        &home,
        &cwd,
        "fact",
        "r",
        "ses-2",
        Some("trn-3"),
        "a distinct note about deploys",
    )
    .unwrap();
    vak_config::spaces::bind(&cwd).unwrap();
    let dir = vak_config::scope::AgentScope::new(&home).skill_proposals(&cwd);
    vak_session::documents::create(
        &dir.join("p1.md"),
        "---\nname: \"ship-it\"\ndescription: \"d\"\n---\n\ndo the thing\n\n<!-- proposed-by: ses-2 at 2026-01-01T00:00:00+00:00; proposal id p1; turn: trn-3 -->\n",
    )
    .unwrap();
    let derived = memory::list_notes(&home, &cwd)
        .into_iter()
        .find(|n| n.tag == "r")
        .unwrap();
    assert_eq!(derived.derived_from.unwrap().turn.as_deref(), Some("trn-3"));
    let queue = learning::list_proposals(&vak_config::scope::AgentScope::new(&home), &cwd);
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue[0].derived_from,
        Some(DerivedFrom {
            conversation: "ses-2".into(),
            turn: Some("trn-3".into())
        })
    );
}
