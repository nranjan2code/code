//! Drafts under the reconciler (plan M7a-e part 4): an old draft goes to
//! the trash whole, and is erased when its time there ends.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use vak_core::Core;
use vak_core::artifacts::{ArtifactKind, ArtifactStep, NewVersion, VersionSource};
use vak_lifecycle::{DataClass, Guard};
use vak_session::ids::{ArtifactId, VersionId};

fn artifact(core: &Core, path: &str) -> ArtifactId {
    core.artifacts()
        .declare(
            "spc_test",
            "vak",
            path,
            ArtifactKind::File,
            None,
            None,
            None,
            None,
        )
        .unwrap()
}

fn version(core: &Core, artifact: ArtifactId, parent: Option<VersionId>, text: &str) -> VersionId {
    core.artifacts()
        .version(
            artifact,
            NewVersion {
                parent,
                bytes: text.as_bytes(),
                source: VersionSource::Person,
            },
            None,
            None,
        )
        .unwrap()
}

fn read(core: &Core, artifact: ArtifactId, version: VersionId) -> Result<Vec<u8>, String> {
    let artifacts = core.artifacts();
    let found = artifacts
        .get(&artifact.to_string())
        .ok_or("no artifact".to_string())?;
    artifacts
        .bytes(&found, &version)
        .map_err(|error| error.to_string())
}

#[test]
fn an_old_draft_goes_to_the_trash_and_is_erased_when_its_time_there_ends() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let artifacts = core.artifacts();

    // A file with an accepted version and three drafts made after it. The
    // last draft has the accepted version's bytes.
    let plan = artifact(&core, "notes/plan.md");
    let accepted = version(&core, plan, None, "the accepted plan");
    artifacts
        .record(
            plan,
            ArtifactStep::Promoted { version: accepted },
            None,
            None,
        )
        .unwrap();
    let draft = version(&core, plan, Some(accepted), "a draft nobody took");
    let restored = version(&core, plan, Some(draft), "a draft someone wants back");
    let same_bytes = version(&core, plan, Some(restored), "the accepted plan");
    // A file that is only a draft, one that is starred, and one on hold.
    let only = artifact(&core, "notes/idea.md");
    let only_version = version(&core, only, None, "an idea nobody kept");
    let starred = artifact(&core, "notes/starred.md");
    let starred_version = version(&core, starred, None, "a starred draft");
    artifacts
        .record(starred, ArtifactStep::Starred { on: true }, None, None)
        .unwrap();
    let held = artifact(&core, "notes/held.md");
    let held_version = version(&core, held, None, "a draft on hold");

    // Nothing is due while the drafts are young.
    let now = Utc::now();
    assert!(
        !core
            .lifecycle_plan()
            .actions
            .iter()
            .any(|action| action.class == DataClass::DraftVersion)
    );

    // After 60 days the drafts go to the trash; the accepted version and
    // the starred draft stay.
    let tick = core.lifecycle_tick_at(true, now + Duration::days(61));
    assert!(tick.failed.is_empty(), "{:?}", tick.failed);
    let after = artifacts.get(&plan.to_string()).unwrap();
    let state = |id: VersionId| {
        after
            .versions
            .iter()
            .find(|known| known.id == id)
            .unwrap()
            .trashed_at
            .is_some()
    };
    assert!(!state(accepted) && state(draft) && state(restored) && state(same_bytes));
    assert_eq!(
        after.head().map(|head| head.id),
        Some(accepted),
        "with its drafts in the trash the accepted version is current again"
    );
    assert!(artifacts.get(&only.to_string()).unwrap().in_trash());
    assert!(!artifacts.get(&starred.to_string()).unwrap().in_trash());
    assert_eq!(
        read(&core, plan, draft).unwrap(),
        b"a draft nobody took",
        "a draft in the trash is whole"
    );
    assert!(
        core.erasure_receipts().is_empty(),
        "going to the trash erases nothing"
    );
    assert!(
        core.lifecycle_transitions(50).iter().any(
            |row| row.class == DataClass::DraftVersion && row.item == format!("{plan}/{draft}")
        ),
        "the move is a recorded transition"
    );

    // One is restored: its age starts again, and it is nobody's to erase.
    artifacts
        .trash_version(plan, restored, false, None)
        .unwrap();
    let since = core
        .lifecycle_items()
        .into_iter()
        .find(|item| item.id == format!("{plan}/{restored}"))
        .expect("a restored draft is a draft again")
        .since;
    assert!(since >= now, "a restored draft ages from its restore");
    // An accepted version cannot be trashed.
    assert!(artifacts.trash_version(plan, accepted, true, None).is_err());

    // One is put on hold.
    let tenant =
        vak_session::objects::TenantObjects::for_tenant(&vak_config::paths::tenant_home_at(
            &vak_config::paths::data_home(),
            vak_config::paths::LOCAL_TENANT,
        ))
        .unwrap();
    tenant
        .hold_scope(&vak_core::artifacts::object_scope(&held))
        .unwrap();

    // 30 days in the trash, and they are erased.
    let later = Utc::now() + Duration::days(31);
    let plan_then = core.lifecycle_plan_at(later);
    assert!(
        plan_then
            .guarded
            .iter()
            .any(|kept| kept.item == format!("draft/{held}/{held_version}")
                && kept.guard == Guard::Held),
        "a held draft is kept back"
    );
    let tick = core.lifecycle_tick_at(true, later);
    assert!(tick.failed.is_empty(), "{:?}", tick.failed);

    assert!(read(&core, plan, draft).is_err(), "an erased draft is gone");
    assert!(read(&core, plan, same_bytes).is_err());
    assert_eq!(
        read(&core, plan, accepted).unwrap(),
        b"the accepted plan",
        "the accepted version keeps the bytes an erased draft shared"
    );
    assert_eq!(
        read(&core, plan, restored).unwrap(),
        b"a draft someone wants back"
    );
    assert!(
        artifacts.get(&only.to_string()).is_none(),
        "a file that was only a draft goes whole"
    );
    assert_eq!(
        read(&core, starred, starred_version).unwrap(),
        b"a starred draft"
    );
    assert_eq!(
        read(&core, held, held_version).unwrap(),
        b"a draft on hold",
        "a hold blocks the erasure"
    );

    let receipts = core.erasure_receipts();
    assert_eq!(receipts.len(), 3, "{receipts:?}");
    assert!(receipts.iter().all(|receipt| receipt.scope == "draft"
        && receipt.verifies()
        && receipt.cause == vak_core::erasure::Cause::Policy));
    let whole = receipts
        .iter()
        .find(|receipt| receipt.subject == format!("{only}/{only_version}"))
        .unwrap();
    assert_eq!((whole.artifacts, whole.keys_destroyed), (1, 1));
    assert!(whole.objects_deleted >= 1);
    let part = receipts
        .iter()
        .find(|receipt| receipt.subject == format!("{plan}/{draft}"))
        .unwrap();
    assert_eq!((part.artifacts, part.keys_destroyed), (0, 0));

    // Erasing is final, and a second pass finds nothing to do.
    assert!(artifacts.trash_version(plan, draft, false, None).is_err());
    let again = core.lifecycle_tick_at(true, later);
    assert!(again.committed.is_empty(), "{:?}", again.committed);
}
