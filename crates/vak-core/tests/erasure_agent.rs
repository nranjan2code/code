//! Erasing an Agent's data (plan M7b-c): everything a revoked or archived
//! Agent holds goes, and nothing of another Agent's or of what a person
//! kept.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::Core;
use vak_core::artifacts::{ArtifactKind, ArtifactStep, NewVersion, VersionSource};
use vak_core::erasure::{Cause, ErasureError};
use vak_session::SessionLog;

fn definitions(work: &std::path::Path, scout: &str) {
    let agent = |id: &str, lifecycle: &str| {
        serde_json::json!({
            "id": id, "revision": 1, "lifecycle": lifecycle, "name": id,
            "character": "vak", "personality": "Calm.", "behaviour": "Brief.",
            "animation": "off", "voice": "default"
        })
    };
    let file = vak_core::agent_definitions::path(work);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        file,
        serde_json::json!([agent("scout", scout), agent("other", "active")]).to_string(),
    )
    .unwrap();
}

/// A conversation in `agent`'s own home that says `said`.
fn conversation_of(
    core: &Core,
    template: &std::path::Path,
    agent: &str,
    said: &str,
) -> std::path::PathBuf {
    let mut header = SessionLog::read_header(template).unwrap();
    header.session_id = format!("{}-{agent}", header.session_id);
    let ledger = vak_config::scope::AgentScope::new(core.shared_scope().agents_dir().join(agent))
        .session_file(core.cwd(), &header.session_id);
    std::fs::create_dir_all(ledger.parent().unwrap()).unwrap();
    let mut log = SessionLog::create(ledger.clone(), header).unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text(said),
        meta: None,
    })
    .unwrap();
    ledger
}

fn file_by(
    core: &Core,
    agent: &str,
    path: &str,
) -> (vak_session::ids::ArtifactId, vak_session::ids::VersionId) {
    let artifacts = core.artifacts();
    let id = artifacts
        .declare(
            "spc_agent",
            agent,
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
    (id, version)
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

fn says(ledger: &std::path::Path, needle: &str) -> bool {
    SessionLog::open_read_only(ledger.to_path_buf()).is_ok_and(|log| {
        log.derive_transcript()
            .iter()
            .any(|message| message.message.text_content().contains(needle))
    })
}

#[tokio::test]
async fn agent_erasure_takes_what_it_owns_and_nothing_else() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    definitions(work.path(), "active");
    let core = Core::new_with_trust(work.path().to_path_buf(), true).unwrap();
    let agents = core.shared_scope().agents_dir();

    // The built-in Agent's conversation, and one each of two saved Agents.
    let mut log = core.start_session().await.unwrap();
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("the marigold budget"),
        meta: None,
    })
    .unwrap();
    let own = log.path().to_path_buf();
    let own_id = log.header().unwrap().session_id.clone();
    drop(log);
    let scout = conversation_of(&core, &own, "scout", "the zephyrine plan");
    let other = conversation_of(&core, &own, "other", "the walnut table");
    let scout_id = format!("{own_id}-scout");

    // What Scout holds besides: a memory note, a draft, a file a person
    // saved, and a file in the workspace Vakyartha keeps for it.
    vak_core::memory::append_note(
        &agents.join("scout"),
        work.path(),
        "fact",
        "",
        &scout_id,
        "scout knows zephyrine",
    )
    .unwrap();
    vak_core::memory::append_note(
        &agents.join("vak"),
        work.path(),
        "fact",
        "",
        &own_id,
        "budget is marigold",
    )
    .unwrap();
    let (draft, _) = file_by(&core, "scout", "notes/scout-draft.md");
    let (saved, saved_version) = file_by(&core, "scout", "notes/scout-saved.md");
    core.artifacts()
        .record(
            saved,
            ArtifactStep::Saved {
                version: saved_version,
            },
            None,
            None,
        )
        .unwrap();
    let (others_draft, _) = file_by(&core, "other", "notes/other-draft.md");
    let workspace = vak_config::paths::ensure_agent_workspace(work.path(), "scout").unwrap();
    std::fs::write(workspace.join("scratch.txt"), "scout's own file").unwrap();
    std::fs::write(work.path().join("mine.txt"), "the owner's file").unwrap();
    assert!(indexed(&core, "zephyrine"));

    // An Agent in use is not erased, nor the built-in one, nor an unknown.
    assert!(matches!(
        core.agent_erasure_preview("scout"),
        Err(ErasureError::AgentInUse)
    ));
    assert!(matches!(
        core.erase_agent("vak", None, Cause::Person, None),
        Err(ErasureError::NotFound(_))
    ));
    assert!(matches!(
        core.agent_erasure_preview("ghost"),
        Err(ErasureError::NotFound(_))
    ));

    definitions(work.path(), "archived");
    let preview = core.agent_erasure_preview("scout").unwrap();
    assert_eq!(
        (
            preview.conversations,
            preview.artifacts,
            preview.workspace_files
        ),
        (1, 1, 1),
        "{preview:?}"
    );
    assert!(preview.documents >= 1 && !preview.held);
    // A hold on one of its conversations blocks the whole erasure.
    core.hold_conversation(&scout_id, true).unwrap();
    assert!(matches!(
        core.erase_agent("scout", Some(&preview.digest), Cause::Person, None),
        Err(ErasureError::Held)
    ));
    core.hold_conversation(&scout_id, false).unwrap();
    assert!(matches!(
        core.erase_agent("scout", Some("another"), Cause::Person, None),
        Err(ErasureError::StalePreview)
    ));

    let receipt = core
        .erase_agent("scout", Some(&preview.digest), Cause::Person, None)
        .unwrap();
    assert_eq!(
        (receipt.scope.as_str(), receipt.subject.as_str()),
        ("agent", "scout")
    );
    assert_eq!((receipt.conversations, receipt.artifacts), (1, 1));
    assert!(receipt.keys_destroyed >= 2 && receipt.verifies());

    // Everything of Scout's is gone.
    assert!(SessionLog::open_read_only(scout.clone()).is_err());
    assert!(!indexed(&core, "zephyrine"));
    assert!(vak_core::memory::list_notes(&agents.join("scout"), work.path()).is_empty());
    assert!(core.artifacts().get(&draft.to_string()).is_none());
    assert!(!workspace.exists());
    assert!(vak_core::trash::is_trashed(&core.shared_scope(), &scout_id));

    // What a person kept of its work, the other Agents and the owner's
    // folder are untouched.
    let kept = core.artifacts().get(&saved.to_string()).unwrap();
    assert_eq!(
        core.artifacts().bytes(&kept, &saved_version).unwrap(),
        b"notes/scout-saved.md"
    );
    assert!(core.artifacts().get(&others_draft.to_string()).is_some());
    assert!(says(&own, "marigold") && says(&other, "walnut"));
    assert!(indexed(&core, "marigold") && indexed(&core, "walnut"));
    assert_eq!(
        vak_core::memory::list_notes(&agents.join("vak"), work.path()).len(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(work.path().join("mine.txt")).unwrap(),
        "the owner's file"
    );
}
