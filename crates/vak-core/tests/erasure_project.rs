//! Erasing a project's data (plan M7b-d): everything Vakyartha stored for
//! one folder goes, another project's is untouched, and the folder itself
//! is never touched.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::artifacts::{ArtifactKind, ArtifactStep, NewVersion, VersionSource};
use vak_core::erasure::{Cause, ErasureError};
use vak_session::SessionLog;

async fn conversation(core: &Core, said: &str) -> (String, std::path::PathBuf) {
    let mut log = core.start_session().await.unwrap();
    let id = log.header().unwrap().session_id.clone();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text(said),
        meta: None,
    })
    .unwrap();
    (id, log.path().to_path_buf())
}

fn saved_file(core: &Core, space: &str, path: &str) -> vak_session::ids::ArtifactId {
    let artifacts = core.artifacts();
    let id = artifacts
        .declare(
            space,
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
                source: VersionSource::Person,
            },
            None,
            None,
        )
        .unwrap();
    artifacts
        .record(id, ArtifactStep::Saved { version }, None, None)
        .unwrap();
    id
}

fn says(ledger: &std::path::Path, needle: &str) -> bool {
    SessionLog::open_read_only(ledger.to_path_buf()).is_ok_and(|log| {
        log.derive_transcript()
            .iter()
            .any(|message| message.message.text_content().contains(needle))
    })
}

fn indexed(core: &Core, needle: &str) -> bool {
    let catalog = core.catalog().unwrap();
    catalog.catch_up().unwrap();
    catalog
        .dump()
        .unwrap()
        .iter()
        .any(|row| row.contains(needle))
}

#[tokio::test]
async fn project_erasure_leaves_the_folder() {
    vak_config::paths::isolate_home_for_tests();
    let (gone_dir, kept_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    // The owner's own files and settings in the project's folder.
    std::fs::create_dir_all(gone_dir.path().join(".vak")).unwrap();
    std::fs::write(
        gone_dir.path().join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    std::fs::write(gone_dir.path().join("report.md"), "the owner's report").unwrap();
    let gone = Core::new_with_trust(gone_dir.path().to_path_buf(), true).unwrap();
    let kept = Core::new_with_trust(kept_dir.path().to_path_buf(), true).unwrap();
    let home = gone.shared_scope().agents_dir().join("vak");

    let (gone_id, gone_ledger) = conversation(&gone, "the zephyrine plan").await;
    let (kept_id, kept_ledger) = conversation(&kept, "the marigold budget").await;
    let gone_space = vak_config::spaces::bind(gone_dir.path()).unwrap();
    let kept_space = vak_config::spaces::bind(kept_dir.path()).unwrap();
    vak_core::memory::append_note(
        &home,
        gone_dir.path(),
        "fact",
        "",
        &gone_id,
        "plan is zephyrine",
    )
    .unwrap();
    vak_core::memory::append_note(
        &home,
        kept_dir.path(),
        "fact",
        "",
        &kept_id,
        "budget is marigold",
    )
    .unwrap();
    let gone_file = saved_file(&gone, &gone_space, "report.md");
    let kept_file = saved_file(&kept, &kept_space, "budget.md");
    let scratch = vak_config::paths::runtime_dir()
        .join("executions")
        .join(&gone_space)
        .join("vak")
        .join("exec-1");
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("draft.md"), "scratch").unwrap();
    assert!(indexed(&gone, "zephyrine"));

    assert!(matches!(
        gone.project_erasure_preview("spc_unknown"),
        Err(ErasureError::NotFound(_))
    ));
    let preview = gone.project_erasure_preview(&gone_space).unwrap();
    assert_eq!(
        (
            preview.conversations,
            preview.artifacts,
            preview.workspace_files
        ),
        (1, 1, 1),
        "{preview:?}"
    );
    assert!(preview.documents >= 1);
    // A hold on one of its files blocks the whole erasure.
    gone.hold_artifact(&gone_file.to_string(), true).unwrap();
    assert!(matches!(
        gone.erase_project(&gone_space, Some(&preview.digest), Cause::Person, None),
        Err(ErasureError::Held)
    ));
    gone.hold_artifact(&gone_file.to_string(), false).unwrap();
    assert!(matches!(
        gone.erase_project(&gone_space, Some("another"), Cause::Person, None),
        Err(ErasureError::StalePreview)
    ));

    let receipt = gone
        .erase_project(&gone_space, Some(&preview.digest), Cause::Person, None)
        .unwrap();
    assert_eq!(
        (receipt.scope.as_str(), receipt.subject.as_str()),
        ("project", gone_space.as_str())
    );
    assert_eq!((receipt.conversations, receipt.artifacts), (1, 1));
    assert!(receipt.verifies());

    // What Vakyartha stored for it is gone.
    assert!(SessionLog::open_read_only(gone_ledger).is_err());
    assert!(!indexed(&gone, "zephyrine"));
    assert!(vak_core::memory::list_notes(&home, gone_dir.path()).is_empty());
    assert!(gone.artifacts().get(&gone_file.to_string()).is_none());
    assert!(!scratch.exists());
    let record = vak_config::spaces::all()
        .into_iter()
        .find(|record| record.id == gone_space)
        .expect("the project stays known");
    assert!(record.forgotten, "and is hidden until it is opened again");

    // The folder and the other project are untouched.
    assert_eq!(
        std::fs::read_to_string(gone_dir.path().join("report.md")).unwrap(),
        "the owner's report"
    );
    assert!(gone_dir.path().join(".vak/config.toml").is_file());
    assert!(says(&kept_ledger, "marigold") && indexed(&kept, "marigold"));
    assert_eq!(
        vak_core::memory::list_notes(&home, kept_dir.path()).len(),
        1
    );
    assert!(kept.artifacts().get(&kept_file.to_string()).is_some());
}
