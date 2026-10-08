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
    };
    sink.declared(&helper, None, "toolu_2", None);
    assert!(artifacts.list().iter().all(|a| a.path != "helper.py"));
    std::fs::write(cwd.path().join("plan.md"), "the plan, revised").unwrap();
    let revision = vak_tools::ArtifactClaim {
        path: "plan.md".into(),
        declared: false,
        title: None,
        summary: None,
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
