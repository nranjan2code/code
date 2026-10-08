//! Holds (plan M7b-b, docs/design/74 §3.2): while something is on hold,
//! nothing destroys it, whoever asks and whichever rule says its time is
//! up; released, each of those goes ahead.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use vak_core::Core;
use vak_core::artifacts::{ArtifactKind, NewVersion, VersionSource};
use vak_core::erasure::{Cause, ErasureError};
use vak_lifecycle::Guard;

async fn conversation(core: &Core, said: &str) -> String {
    let mut log = core.start_session().await.unwrap();
    let id = log.header().unwrap().session_id.clone();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text(said),
        meta: None,
    })
    .unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("Asha: I will bring the lanterns"),
        meta: Some(vak_session::MessageMeta {
            author_id: Some("guest:asha".into()),
            author_name: Some("Asha".into()),
            ..Default::default()
        }),
    })
    .unwrap();
    id
}

fn draft(
    core: &Core,
    path: &str,
    session: &str,
) -> (vak_session::ids::ArtifactId, vak_session::ids::VersionId) {
    let artifacts = core.artifacts();
    let id = artifacts
        .declare(
            "spc_holds",
            "vak",
            path,
            ArtifactKind::File,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let version = artifacts
        .version(
            id,
            NewVersion {
                parent: None,
                bytes: path.as_bytes(),
                source: VersionSource::Call {
                    session: session.to_string(),
                    call: "call-1".into(),
                },
            },
            None,
            None,
        )
        .unwrap();
    // Put up for Review in that conversation, as a tool's draft is.
    artifacts
        .record(
            id,
            vak_core::artifacts::ArtifactStep::Proposed {
                version,
                session: session.to_string(),
                candidate: "cand-1".into(),
            },
            None,
            None,
        )
        .unwrap();
    (id, version)
}

#[tokio::test]
async fn hold_blocks_every_destructive_transition() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let shared = core.shared_scope();
    let artifacts = core.artifacts();
    let kept_back = |core: &Core, item: &str, at| {
        core.lifecycle_plan_at(at)
            .guarded
            .iter()
            .any(|kept| kept.item == item && kept.guard == Guard::Held)
    };

    // A conversation on hold, in the trash.
    let held = conversation(&core, "the marigold budget").await;
    vak_core::trash::set(&shared, std::slice::from_ref(&held), true).unwrap();
    core.hold_conversation(&held, true).unwrap();
    // The owner asking, the end of its time in the trash, and a guest's
    // erasure are all refused.
    assert!(matches!(
        core.erase_conversation(&held, None, Cause::Person, None),
        Err(ErasureError::Held)
    ));
    let trash_ended = Utc::now() + Duration::days(31);
    assert!(kept_back(&core, &held, trash_ended));
    let tick = core.lifecycle_tick_at(true, trash_ended);
    assert!(tick.failed.is_empty(), "{:?}", tick.failed);
    assert!(
        vak_core::trash::state(&shared, &held)
            .unwrap()
            .erased_at
            .is_none()
    );
    assert!(matches!(
        core.erase_guest(&held, "guest:asha", None, Cause::Person, None),
        Err(ErasureError::Held)
    ));

    // A file on hold: its draft does not go to the trash when its time is
    // up, and is not erased from the trash either.
    let (file, version) = draft(&core, "notes/held.md", "nobody");
    core.hold_artifact(&file.to_string(), true).unwrap();
    assert!(core.artifact_held(&file.to_string()));
    let draft_ended = Utc::now() + Duration::days(61);
    assert!(kept_back(&core, &format!("{file}/{version}"), draft_ended));
    core.lifecycle_tick_at(true, draft_ended);
    assert!(!artifacts.get(&file.to_string()).unwrap().in_trash());
    artifacts.trash_version(file, version, true, None).unwrap();
    assert!(matches!(
        core.erase_draft(&file.to_string(), &version.to_string(), Cause::Policy, None),
        Err(ErasureError::Held)
    ));
    core.lifecycle_tick_at(true, Utc::now() + Duration::days(31));
    assert!(
        artifacts
            .bytes(&artifacts.get(&file.to_string()).unwrap(), &version)
            .is_ok()
    );

    // A conversation whose erasure would take a held file is refused too.
    let maker = conversation(&core, "the walnut table").await;
    let (made, _) = draft(&core, "notes/made-there.md", &maker);
    core.hold_artifact(&made.to_string(), true).unwrap();
    vak_core::trash::set(&shared, std::slice::from_ref(&maker), true).unwrap();
    let preview = core.erasure_preview(&maker).unwrap();
    assert!(preview.held && preview.artifacts.contains(&made.to_string()));
    assert!(matches!(
        core.erase_conversation(&maker, Some(&preview.digest), Cause::Person, None),
        Err(ErasureError::Held)
    ));

    // One place lists them all, by what the owner knows them as.
    let holds = core.holds();
    let listed: Vec<(&str, &str)> = holds
        .iter()
        .map(|hold| (hold.kind, hold.id.as_str()))
        .collect();
    assert_eq!(listed.len(), 3, "{holds:?}");
    assert!(listed.contains(&("conversation", held.as_str())));
    assert!(listed.contains(&("artifact", file.to_string().as_str())));
    assert!(
        holds
            .iter()
            .any(|hold| hold.name.as_deref() == Some("the marigold budget"))
    );
    assert!(
        holds
            .iter()
            .any(|hold| hold.name.as_deref() == Some("held.md"))
    );
    assert!(matches!(
        core.hold_artifact("art_nope", true),
        Err(ErasureError::NotFound(_))
    ));

    // Released, each goes ahead.
    core.hold_conversation(&held, false).unwrap();
    core.hold_artifact(&file.to_string(), false).unwrap();
    core.hold_artifact(&made.to_string(), false).unwrap();
    assert!(core.holds().is_empty());
    core.erase_guest(&held, "guest:asha", None, Cause::Person, None)
        .unwrap();
    core.erase_conversation(&held, None, Cause::Person, None)
        .unwrap();
    core.erase_draft(&file.to_string(), &version.to_string(), Cause::Policy, None)
        .unwrap();
    core.erase_conversation(&maker, None, Cause::Person, None)
        .unwrap();
}
