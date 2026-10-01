//! Records derived from a conversation say which conversation and turn
//! they came from (docs/design/73 §4); the field is additive.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::{entities, learning, memory, reflection};
use vak_session::trace::DerivedFrom;

#[test]
fn derived_writes_record_provenance() {
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

    let proposals = reflection::Proposals {
        notes: vec![reflection::NoteProposal {
            note: "reflected distinct note about deploys".into(),
            kind: "fact".into(),
            tag: "r".into(),
        }],
        skill: Some(reflection::SkillDraft {
            name: "ship-it".into(),
            description: "d".into(),
            instructions: "do the thing".into(),
        }),
    };
    reflection::apply(&home, &cwd, "ses-2", Some("trn-3"), &proposals).unwrap();
    let reflected = memory::list_notes(&home, &cwd)
        .into_iter()
        .find(|n| n.tag == "r")
        .unwrap();
    assert_eq!(
        reflected.derived_from.unwrap().turn.as_deref(),
        Some("trn-3")
    );
    let queue = learning::list_proposals(&home, &cwd);
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue[0].derived_from,
        Some(DerivedFrom {
            conversation: "ses-2".into(),
            turn: Some("trn-3".into())
        })
    );

    let record = entities::EntityRecord {
        id: "e1".into(),
        name: "E".into(),
        entity_type: "system".into(),
        summary: "s".into(),
        attributes: Default::default(),
        relations: vec![],
        updated_at: chrono::Utc::now(),
        derived_from: Some(expected.clone()),
    };
    entities::upsert_entity(&home, Some(&cwd), record).unwrap();
    let got = entities::get_entity(&home, Some(&cwd), "e1").unwrap();
    assert_eq!(got.derived_from, Some(expected));
}
