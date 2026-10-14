//! M8 exit tests `concurrent_edit_creates_sibling_versions` and
//! `saved_version_survives_origin_erasure` (data-architecture plan M8.1):
//! two edits made from one version are siblings, never a lost write; a
//! version's bytes belong to the artifact, so they outlive the conversation
//! that made them; and only a declared deliverable becomes an artifact.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_core::artifacts::{
    ArtifactKind, ArtifactStep, Artifacts, CallSink, NewVersion, VersionSource,
};
use vak_session::objects::{Objects, TenantObjects};
use vak_tools::ArtifactSink;

fn home() -> (vak_config::scope::SharedScope, std::path::PathBuf) {
    vak_config::paths::isolate_home_for_tests();
    let data = vak_config::paths::data_home();
    (
        vak_config::scope::SharedScope::new(&data),
        vak_config::paths::tenant_home_at(&data, vak_config::paths::LOCAL_TENANT),
    )
}

fn by_person(bytes: &[u8], parent: Option<vak_session::ids::VersionId>) -> NewVersion<'_> {
    NewVersion {
        parent,
        bytes,
        source: VersionSource::Person,
    }
}

#[test]
fn concurrent_edit_creates_sibling_versions() {
    let (shared, tenant) = home();
    let artifacts = Artifacts::at(&shared, &tenant);
    let id = artifacts
        .declare(
            "spc_a",
            "vak",
            "report.md",
            ArtifactKind::File,
            Some("Report".into()),
            None,
            None,
            None,
        )
        .unwrap();
    // Declaring the same file again is the same artifact.
    let again = artifacts
        .declare(
            "spc_a",
            "vak",
            "./report.md",
            ArtifactKind::File,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(id, again);
    let first = artifacts
        .version(id, by_person(b"v1", None), None, None)
        .unwrap();
    // The same bytes again are no new version.
    assert_eq!(
        artifacts
            .version(id, by_person(b"v1", None), None, None)
            .unwrap(),
        first
    );

    // Two people edit from the first version at once.
    let mine = artifacts
        .version(id, by_person(b"mine", Some(first)), None, None)
        .unwrap();
    let yours = artifacts
        .version(id, by_person(b"yours", Some(first)), None, None)
        .unwrap();
    let artifact = artifacts.get(&id.to_string()).unwrap();
    assert_eq!(artifact.versions.len(), 3);
    let mut heads: Vec<_> = artifact
        .heads()
        .into_iter()
        .map(|version| version.id)
        .collect();
    heads.sort();
    let mut expected = vec![mine, yours];
    expected.sort();
    assert_eq!(heads, expected, "neither edit is lost");
    assert!(
        artifact
            .versions
            .iter()
            .filter(|v| v.parent == Some(first))
            .count()
            == 2
    );
    assert_eq!(artifacts.bytes(&artifact, &mine).unwrap(), b"mine");
    assert_eq!(artifacts.bytes(&artifact, &yours).unwrap(), b"yours");

    // A version made from a sibling reconciles them: it has one parent and
    // the other sibling stays a head until a version is made from it too.
    let merged = artifacts
        .version(id, by_person(b"merged", Some(yours)), None, None)
        .unwrap();
    let artifact = artifacts.get(&id.to_string()).unwrap();
    assert_eq!(artifact.head().map(|v| v.id), Some(merged));
    assert_eq!(artifact.heads().len(), 2);
    assert_eq!(artifact.name(), "Report");
}

#[test]
fn saved_version_survives_origin_erasure() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    std::fs::write(cwd.path().join("plan.md"), "the plan").unwrap();
    let artifacts = Artifacts::at(&shared, &tenant);
    let sink = CallSink {
        artifacts: artifacts.clone(),
        cwd: cwd.path().to_path_buf(),
        executions: vak_config::scope::executions_root(cwd.path()),
        agent: "vak".into(),
    };
    // A call that wrote a file with a title declared it a deliverable.
    let claim = vak_tools::ArtifactClaim {
        path: "plan.md".into(),
        declared: true,
        title: Some("The plan".into()),
        summary: Some("What we will do".into()),
        drafted: false,
        based_on: None,
        ..Default::default()
    };
    sink.declared(
        &claim,
        Some("01920000-0000-7000-8000-000000000001"),
        "toolu_1",
        None,
    );
    let space = vak_session::trace::local::space(cwd.path()).to_string();
    let listed: Vec<_> = artifacts
        .list()
        .into_iter()
        .filter(|a| a.space == space)
        .collect();
    assert_eq!(listed.len(), 1);
    let artifact = &listed[0];
    assert_eq!(
        (artifact.path.as_str(), artifact.name()),
        ("plan.md", "The plan".to_string())
    );
    let version = artifact.head().unwrap().clone();

    // A supporting file written without a title is no entry; a later
    // write of the deliverable without one is a new version of it.
    std::fs::write(cwd.path().join("helper.py"), "print(1)").unwrap();
    let helper = vak_tools::ArtifactClaim {
        path: "helper.py".into(),
        declared: false,
        title: None,
        summary: None,
        drafted: false,
        based_on: None,
        ..Default::default()
    };
    sink.declared(&helper, None, "toolu_2", None);
    assert!(artifacts.list().iter().all(|a| a.path != "helper.py"));
    std::fs::write(cwd.path().join("plan.md"), "the plan, revised").unwrap();
    let revision = vak_tools::ArtifactClaim {
        path: "plan.md".into(),
        declared: false,
        title: None,
        summary: None,
        drafted: false,
        based_on: None,
        ..Default::default()
    };
    sink.declared(&revision, None, "toolu_3", None);
    let revised = artifacts.get(&artifact.id.to_string()).unwrap();
    assert_eq!(revised.versions.len(), 2);
    assert_eq!(revised.versions[1].parent, Some(version.id));
    assert!(matches!(&version.source, VersionSource::Call { call, .. } if call == "toolu_1"));

    // The conversation that made it keeps its own copy under its own
    // scope; erasing that conversation releases its grant.
    let objects = TenantObjects::for_tenant(&tenant).unwrap();
    let origin_scope = "session:01920000-0000-7000-8000-000000000001";
    let origin = objects.put(b"the plan", origin_scope).unwrap();
    artifacts
        .record(
            artifact.id,
            ArtifactStep::Saved {
                version: version.id,
            },
            None,
            None,
        )
        .unwrap();
    objects.release(&origin, origin_scope).unwrap();
    assert!(
        objects.get(&origin, origin_scope).is_err(),
        "the origin's grant is gone"
    );

    // The saved version is the artifact's own: it still reads.
    let artifact = artifacts.get(&artifact.id.to_string()).unwrap();
    assert!(
        artifact
            .versions
            .iter()
            .find(|v| v.id == version.id)
            .unwrap()
            .saved
    );
    assert_eq!(
        artifacts.bytes(&artifact, &version.id).unwrap(),
        b"the plan"
    );
}

/// Plan M8.4c-c: a draft that deletes a file proposes a version in which
/// the file is gone; it has no bytes, and the same deletion again is the
/// same version.
#[test]
fn a_removed_file_is_a_version_with_no_bytes() {
    let (shared, tenant) = home();
    let artifacts = Artifacts::at(&shared, &tenant);
    let id = artifacts
        .declare(
            "spc_removed",
            "vak",
            "old-notes.md",
            ArtifactKind::File,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let first = artifacts
        .version(id, by_person(b"kept so far", None), None, None)
        .unwrap();
    let from_draft = || VersionSource::Candidate {
        session: "session-1".into(),
        candidate: "candidate-1".into(),
    };
    let gone = artifacts
        .removal(id, None, from_draft(), None, None)
        .unwrap();
    assert_eq!(
        artifacts
            .removal(id, None, from_draft(), None, None)
            .unwrap(),
        gone
    );
    artifacts
        .record(
            id,
            ArtifactStep::Proposed {
                version: gone,
                session: "session-1".into(),
                candidate: "candidate-1".into(),
            },
            None,
            None,
        )
        .unwrap();
    let artifact = artifacts.get(&id.to_string()).unwrap();
    assert_eq!(artifact.versions.len(), 2);
    let head = artifact.head().unwrap();
    assert!(head.removed && head.proposed_by("candidate-1"));
    assert_eq!(head.parent, Some(first));
    assert!(artifacts.bytes(&artifact, &gone).is_err());
    assert_eq!(artifacts.bytes(&artifact, &first).unwrap(), b"kept so far");
}

/// What an artifact's rows say (its path, title and summary) is an object
/// of the artifact's own scope (plan M7a-b): the chain keeps ids and steps,
/// and destroying the artifact's key takes the artifact and leaves others.
#[test]
fn an_artifacts_rows_keep_no_content_and_go_with_its_key() {
    let (shared, tenant) = home();
    let artifacts = Artifacts::at(&shared, &tenant);
    let declare = |path: &str, title: &str| {
        artifacts
            .declare(
                "spc_sealed",
                "vak",
                path,
                ArtifactKind::File,
                Some(title.into()),
                Some("figures for the board".into()),
                None,
                None,
            )
            .unwrap()
    };
    let kept = declare("kept-plan.md", "Hiring plan");
    let gone = declare("gone-offer.md", "Offer letter");

    let stored = vak_session::chain::RecordChain::at(shared.artifacts()).text();
    for content in [
        "Offer letter",
        "gone-offer.md",
        "figures for the board",
        "Hiring plan",
    ] {
        assert!(!stored.contains(content), "{content} is in the chain");
    }
    assert!(stored.contains(&gone.to_string()) && stored.contains("\"step\":\"declared\""));
    assert_eq!(
        artifacts.get(&gone.to_string()).unwrap().title.as_deref(),
        Some("Offer letter")
    );

    TenantObjects::for_tenant(&tenant)
        .unwrap()
        .destroy_scope_key(&vak_core::artifacts::object_scope(&gone))
        .unwrap();

    assert!(artifacts.get(&gone.to_string()).is_none());
    assert_eq!(
        artifacts.get(&kept.to_string()).unwrap().title.as_deref(),
        Some("Hiring plan")
    );
}

/// A comment is kept under its author's own key (plan M7a-b): removing
/// one person's contributions leaves the artifact and other comments.
#[test]
fn a_guests_comments_go_with_their_key_and_the_artifact_stays() {
    let (shared, tenant) = home();
    let artifacts = Artifacts::at(&shared, &tenant);
    let id = artifacts
        .declare(
            "spc_comments",
            "vak",
            "brief.md",
            ArtifactKind::File,
            Some("Brief".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let version = artifacts
        .version(id, by_person(b"draft", None), None, None)
        .unwrap();
    for (author, name, text) in [
        ("guest:asha", "Asha", "tighten the second paragraph"),
        ("owner", "You", "agreed, will do"),
    ] {
        artifacts
            .record(
                id,
                ArtifactStep::Commented {
                    version,
                    author: author.into(),
                    author_name: name.into(),
                    text: text.into(),
                    id: None,
                    at_place: Default::default(),
                },
                None,
                None,
            )
            .unwrap();
    }
    let stored = vak_session::chain::RecordChain::at(shared.artifacts()).text();
    assert!(!stored.contains("tighten") && !stored.contains("Asha"));
    assert_eq!(artifacts.get(&id.to_string()).unwrap().comments.len(), 2);

    TenantObjects::for_tenant(&tenant)
        .unwrap()
        .destroy_scope_key(&vak_core::artifacts::comment_scope(&id, "guest:asha"))
        .unwrap();

    let left = artifacts.get(&id.to_string()).unwrap();
    assert_eq!(left.title.as_deref(), Some("Brief"));
    assert_eq!(left.comments.len(), 1);
    assert_eq!(left.comments[0].text, "agreed, will do");
}

/// A draft for review is recorded from the call's own draft, not the
/// workspace file it is for; a revision of it names it as its parent; and a
/// draft that cannot be found leaves no entry with no version (doc 90).
#[test]
fn a_draft_is_a_version_and_its_revision_is_its_child() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    let executions = vak_config::scope::executions_root(cwd.path());
    let write_draft = |call: &str, bytes: &[u8]| {
        let dir = executions.join("vak").join(call);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.docx"), bytes).unwrap();
    };
    let artifacts = Artifacts::at(&shared, &tenant);
    let sink = CallSink {
        artifacts: artifacts.clone(),
        cwd: cwd.path().to_path_buf(),
        executions: executions.clone(),
        agent: "vak".into(),
    };
    let drafted = |based_on: Option<&str>| vak_tools::ArtifactClaim {
        path: "notes.docx".into(),
        declared: true,
        title: None,
        summary: None,
        drafted: true,
        based_on: based_on.map(str::to_string),
        ..Default::default()
    };
    let space = vak_session::trace::local::space(cwd.path()).to_string();
    let notes = |artifacts: &Artifacts| {
        artifacts
            .list()
            .into_iter()
            .find(|a| a.space == space && a.path == "notes.docx")
    };

    // No draft on disk: nothing is declared.
    sink.declared(&drafted(None), Some("ses"), "call_none", None);
    assert!(notes(&artifacts).is_none());

    write_draft("call_a", b"first draft");
    sink.declared(&drafted(None), Some("ses"), "call_a", None);
    let first = notes(&artifacts).unwrap().head().unwrap().id;

    write_draft("call_b", b"second draft");
    sink.declared(
        &drafted(Some(".vak/scratch/vak/call_a/notes.docx")),
        Some("ses"),
        "call_b",
        None,
    );
    let artifact = notes(&artifacts).unwrap();
    assert_eq!(artifact.versions.len(), 2);
    let second = artifact.head().unwrap();
    assert_eq!(second.parent, Some(first));
    assert_eq!(
        artifacts.bytes(&artifact, &second.id).unwrap(),
        b"second draft"
    );
}

/// A listing by pattern is told of recorded files it cannot see because
/// they are drafts not yet in the workspace, and only of those (doc 90 §3.3).
#[test]
fn a_listing_is_told_of_matching_drafts_not_in_the_workspace() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    let executions = vak_config::scope::executions_root(cwd.path());
    let dir = executions.join("vak").join("call_m");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("madrid-notes.docx"), b"draft").unwrap();
    let artifacts = Artifacts::at(&shared, &tenant);
    let sink = CallSink {
        artifacts: artifacts.clone(),
        cwd: cwd.path().to_path_buf(),
        executions,
        agent: "vak".into(),
    };
    let claim = vak_tools::ArtifactClaim {
        path: "madrid-notes.docx".into(),
        declared: true,
        title: None,
        summary: None,
        drafted: true,
        based_on: None,
        ..Default::default()
    };
    sink.declared(&claim, Some("ses"), "call_m", None);
    let space = vak_session::trace::local::space(cwd.path()).to_string();

    let told = vak_core::file_state::matching_note(
        &artifacts,
        &space,
        cwd.path(),
        "**/*madrid*.docx",
        None,
    )
    .expect("the draft is listed");
    assert!(told.contains("`madrid-notes.docx`"), "{told}");
    assert!(
        told.contains(".vak/scratch/vak/call_m/madrid-notes.docx"),
        "{told}"
    );
    assert!(
        vak_core::file_state::matching_note(&artifacts, &space, cwd.path(), "*.xlsx", None)
            .is_none()
    );

    // Accepted into the folder, the listing sees it itself.
    std::fs::write(cwd.path().join("madrid-notes.docx"), b"draft").unwrap();
    assert!(
        vak_core::file_state::matching_note(&artifacts, &space, cwd.path(), "*.docx", None)
            .is_none()
    );
}

/// A search hit on a deliverable names its file and the record's state of
/// it: a draft until accepted, then in the workspace as recorded (doc 90 §3.3).
#[test]
fn a_search_hit_names_its_file_and_state() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    let executions = vak_config::scope::executions_root(cwd.path());
    let dir = executions.join("vak").join("call_r");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rome-notes.docx"), b"draft").unwrap();
    let artifacts = Artifacts::at(&shared, &tenant);
    let sink = CallSink {
        artifacts: artifacts.clone(),
        cwd: cwd.path().to_path_buf(),
        executions,
        agent: "vak".into(),
    };
    sink.declared(
        &vak_tools::ArtifactClaim {
            path: "rome-notes.docx".into(),
            declared: true,
            title: None,
            summary: None,
            drafted: true,
            based_on: None,
            ..Default::default()
        },
        Some("ses"),
        "call_r",
        None,
    );
    let space = vak_session::trace::local::space(cwd.path()).to_string();
    let id = artifacts
        .list()
        .into_iter()
        .find(|a| a.space == space && a.path == "rome-notes.docx")
        .unwrap()
        .id
        .to_string();

    let told = vak_core::file_state::describe(
        &artifacts,
        cwd.path(),
        &id,
        vak_core::file_state::Reader {
            agent: "vak",
            session: None,
        },
    )
    .unwrap();
    assert!(told.starts_with("file `rome-notes.docx`"), "{told}");
    assert!(
        told.contains(".vak/scratch/vak/call_r/rome-notes.docx"),
        "{told}"
    );

    std::fs::write(cwd.path().join("rome-notes.docx"), b"draft").unwrap();
    let told = vak_core::file_state::describe(
        &artifacts,
        cwd.path(),
        &id,
        vak_core::file_state::Reader {
            agent: "vak",
            session: None,
        },
    )
    .unwrap();
    assert!(told.contains("in the workspace"), "{told}");
}

/// An edit made outside Vak is recorded as a version of what it changed when
/// Vak next reads the file, and a draft made before it is told it does not
/// include it (doc 90 D2, §3.2).
#[test]
fn an_outside_edit_is_recorded_and_a_draft_made_before_it_is_behind() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    let space = vak_session::trace::local::space(cwd.path()).to_string();
    let artifacts = Artifacts::at(&shared, &tenant);
    let id = artifacts
        .declare(
            &space,
            "vak",
            "notes.md",
            ArtifactKind::Document,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let person = |bytes: &'static [u8], parent| NewVersion {
        parent,
        bytes,
        source: VersionSource::Person,
    };
    // The accepted file, then a draft made from it.
    let accepted = artifacts
        .version(id, person(b"one", None), None, None)
        .unwrap();
    artifacts
        .record(id, ArtifactStep::Promoted { version: accepted }, None, None)
        .unwrap();
    std::fs::write(cwd.path().join("notes.md"), b"one").unwrap();
    let draft = artifacts
        .version(id, person(b"one, and a draft", Some(accepted)), None, None)
        .unwrap();

    let before = vak_core::file_state::observe(&artifacts, &space, cwd.path(), "notes.md").unwrap();
    assert!(!before.changed_outside);
    assert!(before.drafts.iter().all(|d| !d.behind), "{before:?}");

    // Someone edits the file by hand.
    std::fs::write(cwd.path().join("notes.md"), b"one, edited by hand").unwrap();
    let after = vak_core::file_state::observe(&artifacts, &space, cwd.path(), "notes.md").unwrap();
    assert!(after.changed_outside, "{after:?}");
    let artifact = artifacts.get(&id.to_string()).unwrap();
    let observed = artifact
        .versions
        .iter()
        .find(|v| matches!(v.source, VersionSource::Workspace))
        .expect("the edit is a version");
    assert_eq!(observed.parent, Some(accepted));
    let waiting = after.drafts.iter().find(|d| d.version == draft).unwrap();
    assert!(waiting.behind, "{after:?}");
    let told = vak_core::file_state::note(
        &after,
        vak_core::file_state::Reader {
            agent: "vak",
            session: None,
        },
    )
    .unwrap();
    assert!(told.contains("edit made outside Vak"), "{told}");
    assert!(told.contains("which it does not include"), "{told}");

    // Reading again records nothing more.
    vak_core::file_state::observe(&artifacts, &space, cwd.path(), "notes.md").unwrap();
    let again = artifacts.get(&id.to_string()).unwrap();
    assert_eq!(again.versions.len(), artifact.versions.len());
}

/// Readers with handles of their own, as the CLI and the server have on one
/// data home, all see one hand edit at once: it is recorded once, because
/// each decision is appended only if the chain did not move since it read.
#[test]
fn an_outside_edit_seen_by_many_writers_at_once_is_one_version() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    let space = vak_session::trace::local::space(cwd.path()).to_string();
    let artifacts = Artifacts::at(&shared, &tenant);
    let id = artifacts
        .declare(
            &space,
            "vak",
            "race.md",
            ArtifactKind::Document,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let accepted = artifacts
        .version(id, by_person(b"one", None), None, None)
        .unwrap();
    artifacts
        .record(id, ArtifactStep::Promoted { version: accepted }, None, None)
        .unwrap();
    std::fs::write(cwd.path().join("race.md"), b"one, edited by hand").unwrap();

    let start = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let own = Artifacts::at(&shared, &tenant);
                start.wait();
                let state = vak_core::file_state::observe(&own, &space, cwd.path(), "race.md");
                assert!(state.unwrap().changed_outside);
            });
        }
    });
    let found = artifacts
        .get(&id.to_string())
        .unwrap()
        .versions
        .into_iter()
        .filter(|v| matches!(v.source, VersionSource::Workspace))
        .count();
    assert_eq!(found, 1);
}

/// A draft of a file that was in the folder before Vak recorded anything of
/// it is a version of that file: the file is the record's first version,
/// the draft its child, and reading the file later does not make the draft
/// look as if it missed an edit (doc 90 §3.5). A hand edit after it does.
#[test]
fn a_draft_of_a_file_vak_never_recorded_is_made_from_that_file() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    std::fs::write(cwd.path().join("plan.docx"), b"as the person wrote it").unwrap();
    let executions = vak_config::scope::executions_root(cwd.path());
    let dir = executions.join("vak").join("call_p");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plan.docx"), b"with the Agent's change").unwrap();
    let artifacts = Artifacts::at(&shared, &tenant);
    let sink = CallSink {
        artifacts: artifacts.clone(),
        cwd: cwd.path().to_path_buf(),
        executions,
        agent: "vak".into(),
    };
    sink.declared(
        &vak_tools::ArtifactClaim {
            path: "plan.docx".into(),
            declared: true,
            title: None,
            summary: None,
            drafted: true,
            based_on: None,
            ..Default::default()
        },
        Some("ses"),
        "call_p",
        None,
    );
    let space = vak_session::trace::local::space(cwd.path()).to_string();
    let artifact = artifacts
        .list()
        .into_iter()
        .find(|a| a.space == space && a.path == "plan.docx")
        .unwrap();
    let found = artifact
        .versions
        .iter()
        .find(|v| matches!(v.source, VersionSource::Workspace))
        .expect("the file in the folder is a version");
    assert_eq!(found.parent, None);
    let draft = artifact
        .versions
        .iter()
        .find(|v| matches!(v.source, VersionSource::Call { .. }))
        .expect("the draft is a version");
    assert_eq!(draft.parent, Some(found.id));

    let state = vak_core::file_state::observe(&artifacts, &space, cwd.path(), "plan.docx").unwrap();
    assert!(!state.changed_outside, "{state:?}");
    assert!(state.drafts.iter().all(|d| !d.behind), "{state:?}");
    assert!(!vak_core::file_state::behind(&artifact, &state, draft.id));

    std::fs::write(cwd.path().join("plan.docx"), b"edited by hand since").unwrap();
    let state = vak_core::file_state::observe(&artifacts, &space, cwd.path(), "plan.docx").unwrap();
    let artifact = artifacts.get(&artifact.id.to_string()).unwrap();
    assert!(state.changed_outside, "{state:?}");
    assert!(vak_core::file_state::behind(&artifact, &state, draft.id));
}

/// A version keeps the few words its call named it with and what its turn
/// read first (doc 90 §5, §5.1): sealed in the chain like a path, read back
/// on the version, and told in the note.
#[test]
fn a_version_keeps_its_name_and_what_its_turn_read_first() {
    let (shared, tenant) = home();
    let cwd = tempfile::tempdir().unwrap();
    vak_config::spaces::bind(cwd.path()).unwrap();
    std::fs::write(cwd.path().join("lisbon.md"), "Tram 28 rides the hills.").unwrap();
    let artifacts = Artifacts::at(&shared, &tenant);
    let sink = CallSink {
        artifacts: artifacts.clone(),
        cwd: cwd.path().to_path_buf(),
        executions: vak_config::scope::executions_root(cwd.path()),
        agent: "vak".into(),
    };
    let read_first = vec![
        vak_tools::ReadInput::File {
            path: "notes/belem-sources.md".into(),
            digest: Some("abc".into()),
        },
        vak_tools::ReadInput::Page {
            url: "https://example.com/tram-28".into(),
        },
    ];
    let claim = vak_tools::ArtifactClaim {
        path: "lisbon.md".into(),
        declared: true,
        title: Some("Lisbon".into()),
        change: Some("Added tram 28".into()),
        read_first: read_first.clone(),
        ..Default::default()
    };
    sink.declared(
        &claim,
        Some("01920000-0000-7000-8000-000000000002"),
        "toolu_tram",
        None,
    );
    let space = vak_session::trace::local::space(cwd.path()).to_string();
    // Read back by a fresh reader, from the chain alone.
    let artifact = Artifacts::at(&shared, &tenant)
        .list()
        .into_iter()
        .find(|a| a.space == space && a.path == "lisbon.md")
        .unwrap();
    let version = artifact.head().unwrap();
    assert_eq!(version.title.as_deref(), Some("Added tram 28"));
    assert_eq!(version.derived_from, read_first);

    let stored = vak_session::chain::RecordChain::at(shared.artifacts()).text();
    for content in ["Added tram 28", "belem-sources", "example.com/tram-28"] {
        assert!(!stored.contains(content), "{content} is in the chain");
    }
}
