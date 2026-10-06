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
    assert!(artifact.head().unwrap().saved);
    assert_eq!(
        artifacts.bytes(&artifact, &version.id).unwrap(),
        b"the plan"
    );
}
