//! Collection releases only what nothing names (plan M7a-d part 4).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;
use vak_core::Core;
use vak_session::documents;
use vak_session::objects::conversation_scope;

fn update(path: &std::path::Path, text: &str) {
    documents::update(path, |_| Ok(Some((text.to_string(), ())))).unwrap();
}

#[test]
fn gc_keeps_everything_reachable() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let notes = vak_config::paths::data_home()
        .join("agents")
        .join("vak")
        .join("notes");
    let (plan, twin, gone) = (
        notes.join("plan.md"),
        notes.join("twin.md"),
        notes.join("gone.md"),
    );

    // `plan` has a history; `twin` holds the text of one of its old
    // versions; `gone` holds the text `plan` has now, and is forgotten.
    documents::create(&plan, "draft one").unwrap();
    update(&plan, "only in history");
    update(&plan, "the plan as it stands");
    documents::create(&twin, "draft one").unwrap();
    documents::create(&gone, "the plan as it stands").unwrap();
    // An object of a conversation, which no Document names.
    let objects = core.objects().unwrap();
    let scope = conversation_scope("ses-gc-kept");
    let evidence = objects.put(b"what a tool returned", &scope).unwrap();

    let tomorrow = chrono::Utc::now().timestamp() + 86_400;
    let name = |suffix: &str| {
        documents::names()
            .into_iter()
            .find(|name| name.ends_with(suffix))
            .unwrap()
    };
    assert_eq!(
        documents::prune_history(&name("notes/plan.md"), tomorrow).unwrap(),
        2
    );
    assert!(documents::forget(&gone).unwrap());

    // Nothing young is released: a save in flight has objects its
    // Document does not name yet.
    // What pruning and forgetting already released (three version
    // records) has no holder and is deleted either way.
    assert_eq!(
        documents::collect(Duration::from_secs(3600)).unwrap(),
        (0, 3)
    );

    let (released, deleted) = documents::collect(Duration::ZERO).unwrap();
    assert_eq!(released, 1, "only the text nothing names any more");
    assert_eq!(deleted, 1, "and with its last grant gone it is deleted");

    assert_eq!(
        documents::read(&plan).unwrap().as_deref(),
        Some("the plan as it stands")
    );
    assert_eq!(
        documents::read(&twin).unwrap().as_deref(),
        Some("draft one")
    );
    assert_eq!(documents::read(&gone).unwrap(), None);
    assert_eq!(
        objects.get(&evidence, &scope).unwrap(),
        b"what a tool returned"
    );
    // The Documents still take new versions, and a second pass finds nothing.
    update(&plan, "the plan, revised");
    update(&twin, "only in history");
    assert_eq!(documents::collect(Duration::ZERO).unwrap().0, 0);
    assert_eq!(
        documents::read(&twin).unwrap().as_deref(),
        Some("only in history")
    );
}
