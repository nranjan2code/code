//! A Document's earlier versions are history the reconciler prunes; its
//! current version is never history (plan M7a-d part 3b).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_lifecycle::DataClass;
use vak_session::documents;

#[test]
fn a_documents_history_is_observed_and_pruned_and_its_current_version_stays() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let path = vak_config::paths::data_home()
        .join("agents")
        .join("vak")
        .join("notes")
        .join("standup.md");
    documents::create(&path, "first").unwrap();
    for text in ["second", "third"] {
        documents::update(&path, |_| Ok(Some((text.to_string(), ())))).unwrap();
    }
    assert_eq!(documents::version_count(&path), 3);
    let name = documents::names()
        .into_iter()
        .find(|name| name.ends_with("notes/standup.md"))
        .expect("the document is listed");
    let times = documents::history_times(&name);
    assert_eq!(times.len(), 2, "two earlier versions, each with its time");
    assert!(times.iter().all(Option::is_some));

    // The reconciler sees the history, by a digest and never by its name,
    // and it is not due: the versions were saved today.
    let observed: Vec<_> = core
        .lifecycle_items()
        .into_iter()
        .filter(|item| item.class == DataClass::DocumentHistory && item.files == 2)
        .collect();
    assert_eq!(observed.len(), 1);
    assert!(!observed[0].id.contains("standup"));
    let tick = core.lifecycle_tick(true);
    assert!(
        !tick
            .committed
            .iter()
            .any(|row| row.class == DataClass::DocumentHistory),
        "history inside its keep time stays"
    );
    assert_eq!(documents::version_count(&path), 3);

    // Past the rule: the earlier versions go and the current one stays.
    let tomorrow = chrono::Utc::now().timestamp() + 86_400;
    assert_eq!(documents::prune_history(&name, tomorrow).unwrap(), 2);
    assert_eq!(documents::version_count(&path), 1);
    assert_eq!(documents::read(&path).unwrap().as_deref(), Some("third"));
    assert_eq!(documents::prune_history(&name, tomorrow).unwrap(), 0);
    // And the Document can still be saved.
    documents::update(&path, |_| Ok(Some(("fourth".to_string(), ())))).unwrap();
    assert_eq!(documents::version_count(&path), 2);
}
