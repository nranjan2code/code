//! M8 exit tests `share_inherits_and_breaks` and
//! `revoked_grant_hides_from_search` (data-architecture plan M8.2): an
//! artifact inherits its Space's audience until a share breaks
//! inheritance, a grant opens one object to one principal for its role,
//! and a revoked or expired grant hides the object from that principal's
//! search at once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use vak_core::artifacts::{ArtifactKind, Artifacts};
use vak_core::grants::{Grant, GrantError, GrantObject, Grants, Role};
use vak_session::ids::{GrantId, PrincipalId};

fn grant(
    principal: &str,
    object: GrantObject,
    role: Role,
    expires: Option<chrono::DateTime<Utc>>,
) -> Grant {
    Grant {
        id: GrantId::new(),
        principal: principal.into(),
        display_name: String::new(),
        object,
        role,
        audience_id: None,
        capabilities: Vec::new(),
        token_hash: None,
        created_at: Utc::now(),
        expires_at: expires,
        history_from: None,
    }
}

fn setup() -> (
    vak_config::scope::SharedScope,
    Artifacts,
    Grants,
    vak_catalog::Catalog,
) {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap().keep();
    let shared = vak_config::scope::SharedScope::new(&dir);
    let tenant = vak_config::paths::tenant_home_at(&dir, vak_config::paths::LOCAL_TENANT);
    let catalog = vak_catalog::Catalog::open(&dir.join("catalog.db"), &dir).unwrap();
    (
        shared.clone(),
        Artifacts::at(&shared, tenant),
        Grants::at(&shared),
        catalog,
    )
}

fn found(
    catalog: &vak_catalog::Catalog,
    audience: &vak_catalog::Audience,
    query: &str,
) -> Vec<String> {
    catalog.catch_up().unwrap();
    catalog
        .search(query, audience, &Default::default(), 20)
        .unwrap()
        .into_iter()
        .map(|hit| hit.node.id)
        .collect()
}

#[test]
fn share_inherits_and_breaks() {
    let (_shared, artifacts, grants, catalog) = setup();
    let id = artifacts
        .declare(
            "spc_x",
            "vak",
            "atlas.md",
            ArtifactKind::File,
            Some("Atlas of harbours".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let object = GrantObject::Artifact(id);
    let now = Utc::now();
    let guest = PrincipalId::new().to_string();
    let agent = vak_session::trace::local::agent_principal("vak").to_string();

    // Inheriting: the Space's audience may read; an outsider may not.
    assert!(grants.inherits(&object).unwrap());
    assert!(grants.may(&agent, &object, Role::Viewer, true, now));
    assert!(!grants.may(&guest, &object, Role::Viewer, false, now));
    let agent_view = vak_catalog::Audience {
        agents: Some(vec![vak_session::trace::local::agent("vak").to_string()]),
        ..Default::default()
    };
    assert!(found(&catalog, &agent_view, "atlas harbours").contains(&id.to_string()));

    // Sharing gives the guest exactly its role, and breaks inheritance.
    grants
        .grant(
            grant(&guest, object.clone(), Role::Commenter, None),
            None,
            None,
        )
        .unwrap();
    grants.break_inheritance(object.clone(), None).unwrap();
    assert!(grants.may(&guest, &object, Role::Viewer, false, now));
    assert!(grants.may(&guest, &object, Role::Commenter, false, now));
    assert!(!grants.may(&guest, &object, Role::Editor, false, now));
    assert!(
        !grants.may(&agent, &object, Role::Viewer, true, now),
        "a broken object no longer inherits its Space's audience"
    );
    assert!(!found(&catalog, &agent_view, "atlas harbours").contains(&id.to_string()));
    // An explicit grant to the Agent still opens it.
    grants
        .grant(
            grant(&agent, object.clone(), Role::Viewer, None),
            None,
            None,
        )
        .unwrap();
    assert!(grants.may(&agent, &object, Role::Viewer, true, now));

    // Restoring inheritance gives the Space's audience back.
    grants.restore_inheritance(object.clone(), None).unwrap();
    assert!(grants.inherits(&object).unwrap());
    assert!(found(&catalog, &agent_view, "atlas harbours").contains(&id.to_string()));
    assert_eq!(grants.on(&object).unwrap().len(), 2);
}

#[test]
fn revoked_grant_hides_from_search() {
    let (_shared, artifacts, grants, catalog) = setup();
    let shared = artifacts
        .declare(
            "spc_y",
            "vak",
            "shared.md",
            ArtifactKind::File,
            Some("Zanzibar ferry plan".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let private = artifacts
        .declare(
            "spc_y",
            "vak",
            "private.md",
            ArtifactKind::File,
            Some("Zanzibar ferry budget".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let guest = PrincipalId::new().to_string();
    let as_guest = vak_catalog::Audience {
        principal: Some(guest.clone()),
        ..Default::default()
    };
    assert!(
        found(&catalog, &as_guest, "zanzibar ferry").is_empty(),
        "nothing is open to the guest yet"
    );

    let viewer = grant(&guest, GrantObject::Artifact(shared), Role::Viewer, None);
    let viewer_id = viewer.id;
    grants.grant(viewer, None, None).unwrap();
    let hits = found(&catalog, &as_guest, "zanzibar ferry");
    assert_eq!(hits, vec![shared.to_string()], "only what the grant opens");
    assert!(!hits.contains(&private.to_string()));

    grants.revoke(viewer_id, None).unwrap();
    assert!(
        found(&catalog, &as_guest, "zanzibar ferry").is_empty(),
        "revoked at once"
    );

    // An expired grant opens nothing either.
    grants
        .grant(
            grant(
                &guest,
                GrantObject::Artifact(private),
                Role::Viewer,
                Some(Utc::now() - Duration::minutes(1)),
            ),
            None,
            None,
        )
        .unwrap();
    assert!(found(&catalog, &as_guest, "zanzibar ferry").is_empty());

    // The rebuilt catalog says the same.
    let incremental = catalog.dump().unwrap();
    catalog.rebuild().unwrap();
    assert_eq!(catalog.dump().unwrap(), incremental);
}

#[test]
fn an_unreadable_revocation_never_preserves_a_cached_grant() {
    let (_shared, _artifacts, grants, _catalog) = setup();
    let guest = PrincipalId::new().to_string();
    let object = GrantObject::Conversation(uuid::Uuid::now_v7().to_string());
    let viewer = grant(&guest, object.clone(), Role::Viewer, None);
    let id = viewer.id;
    grants.grant(viewer, None, None).unwrap();
    assert!(grants.may(&guest, &object, Role::Viewer, false, Utc::now()));

    let segments = vak_storage::segments::SegmentSet::open(&grants.path()).unwrap();
    let lock = segments.lock().unwrap();
    segments.seal(1, &lock).unwrap();
    drop(lock);
    grants.revoke(id, None).unwrap();
    let revocation = segments.log_path(2);
    let original = std::fs::read(&revocation).unwrap();

    for broken in [
        b"unreadable".to_vec(),
        original[..original.len() - 1].to_vec(),
    ] {
        std::fs::write(&revocation, broken).unwrap();
        assert!(matches!(grants.on(&object), Err(GrantError::Unreadable)));
        assert!(!grants.may(&guest, &object, Role::Viewer, false, Utc::now()));
        // Even inherited access must not turn an unreadable store into yes.
        assert!(!grants.may(&guest, &object, Role::Viewer, true, Utc::now()));
    }
    std::fs::write(&revocation, original).unwrap();
    assert!(!grants.may(&guest, &object, Role::Viewer, false, Utc::now()));
    assert_eq!(
        grants.on(&object).unwrap()[0].status(Utc::now()),
        vak_core::grants::GrantStatus::Revoked
    );
}

#[test]
fn a_malformed_grant_row_and_a_missing_cached_chain_fail_closed() {
    let (_shared, _artifacts, grants, _catalog) = setup();
    let guest = PrincipalId::new().to_string();
    let object = GrantObject::Conversation(uuid::Uuid::now_v7().to_string());
    grants
        .grant(
            grant(&guest, object.clone(), Role::Viewer, None),
            None,
            None,
        )
        .unwrap();
    assert!(grants.may(&guest, &object, Role::Viewer, false, Utc::now()));
    let segment = grants.path().join("seg-00000001.log");
    let original = std::fs::read(&segment).unwrap();
    std::fs::remove_file(&segment).unwrap();
    assert!(matches!(grants.on(&object), Err(GrantError::Unreadable)));
    assert!(!grants.may(&guest, &object, Role::Viewer, true, Utc::now()));
    std::fs::write(&segment, original).unwrap();
    vak_session::chain::RecordChain::at(grants.path())
        .append(&serde_json::json!({"step": "unknown"}))
        .unwrap();
    assert!(matches!(grants.on(&object), Err(GrantError::Unreadable)));
    assert!(!grants.may(&guest, &object, Role::Viewer, false, Utc::now()));
}

#[test]
fn a_missing_revocation_segment_cannot_be_skipped_for_a_later_segment() {
    let (_shared, _artifacts, grants, _catalog) = setup();
    let guest = PrincipalId::new().to_string();
    let object = GrantObject::Conversation(uuid::Uuid::now_v7().to_string());
    let viewer = grant(&guest, object.clone(), Role::Viewer, None);
    let id = viewer.id;
    grants.grant(viewer, None, None).unwrap();
    assert!(grants.may(&guest, &object, Role::Viewer, false, Utc::now()));
    let segments = vak_storage::segments::SegmentSet::open(&grants.path()).unwrap();
    segments.seal(1, &segments.lock().unwrap()).unwrap();
    grants.revoke(id, None).unwrap();
    segments.seal(2, &segments.lock().unwrap()).unwrap();
    grants.restore_inheritance(object.clone(), None).unwrap();
    let path = segments.sealed_path(2);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(grants.on(&object), Err(GrantError::Unreadable)));
    assert!(!grants.may(&guest, &object, Role::Viewer, false, Utc::now()));
    std::fs::write(path, bytes).unwrap();
    assert!(!grants.may(&guest, &object, Role::Viewer, false, Utc::now()));
}

#[test]
fn catalog_queries_refresh_grants_without_a_caller_catch_up() {
    let (_shared, artifacts, grants, catalog) = setup();
    let id = artifacts
        .declare(
            "spc_guard",
            "vak",
            "guard.md",
            ArtifactKind::File,
            Some("Guarded lighthouse".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let guest = PrincipalId::new().to_string();
    let audience = vak_catalog::Audience {
        principal: Some(guest.clone()),
        ..Default::default()
    };
    let viewer = grant(&guest, GrantObject::Artifact(id), Role::Viewer, None);
    let grant_id = viewer.id;
    grants.grant(viewer, None, None).unwrap();
    assert_eq!(
        found(&catalog, &audience, "lighthouse"),
        vec![id.to_string()]
    );
    grants.revoke(grant_id, None).unwrap();
    assert!(
        catalog
            .search("lighthouse", &audience, &Default::default(), 20)
            .unwrap()
            .is_empty()
    );
    assert!(catalog.list("artifact", &audience, 20).unwrap().is_empty());
    let agent_view = vak_catalog::Audience {
        agents: Some(vec![vak_session::trace::local::agent("vak").to_string()]),
        ..Default::default()
    };
    assert_eq!(
        catalog
            .search("lighthouse", &agent_view, &Default::default(), 20)
            .unwrap()
            .len(),
        1
    );
    grants
        .break_inheritance(GrantObject::Artifact(id), None)
        .unwrap();
    assert!(
        catalog
            .list("artifact", &agent_view, 20)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn catalog_queries_reject_unreadable_or_missing_grant_history() {
    let (_shared, artifacts, grants, catalog) = setup();
    let id = artifacts
        .declare(
            "spc_guard",
            "vak",
            "guard.md",
            ArtifactKind::File,
            Some("Guarded harbour".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let guest = PrincipalId::new().to_string();
    let audience = vak_catalog::Audience {
        principal: Some(guest.clone()),
        ..Default::default()
    };
    let viewer = grant(&guest, GrantObject::Artifact(id), Role::Viewer, None);
    let grant_id = viewer.id;
    grants.grant(viewer, None, None).unwrap();
    assert_eq!(found(&catalog, &audience, "harbour"), vec![id.to_string()]);
    let segments = vak_storage::segments::SegmentSet::open(&grants.path()).unwrap();
    segments.seal(1, &segments.lock().unwrap()).unwrap();
    grants.revoke(grant_id, None).unwrap();
    let path = segments.log_path(2);
    let original = std::fs::read(&path).unwrap();
    for broken in [b"broken".to_vec(), original[..original.len() - 1].to_vec()] {
        std::fs::write(&path, broken).unwrap();
        assert!(catalog.catch_up().is_err());
        assert!(
            catalog
                .search("harbour", &audience, &Default::default(), 20)
                .is_err()
        );
        assert!(catalog.list("artifact", &audience, 20).is_err());
        assert!(catalog.calls_of("read", None, &audience).is_err());
    }
    std::fs::write(&path, original).unwrap();
    let backup = grants.path().with_extension("missing-for-test");
    std::fs::rename(grants.path(), &backup).unwrap();
    assert!(catalog.catch_up().is_err());
    assert!(
        catalog
            .search("harbour", &audience, &Default::default(), 20)
            .is_err()
    );
    std::fs::rename(backup, grants.path()).unwrap();
    assert!(
        catalog
            .search("harbour", &audience, &Default::default(), 20)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn malformed_grant_suffix_rolls_back_projection_and_recovers_without_skipping() {
    let (_shared, artifacts, grants, catalog) = setup();
    let id = artifacts
        .declare(
            "spc_atomic",
            "vak",
            "atomic.md",
            ArtifactKind::File,
            Some("Atomic harbour".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let guest = PrincipalId::new().to_string();
    let audience = vak_catalog::Audience {
        principal: Some(guest.clone()),
        ..Default::default()
    };
    let viewer = grant(&guest, GrantObject::Artifact(id), Role::Viewer, None);
    let grant_id = viewer.id;
    grants.grant(viewer, None, None).unwrap();
    assert_eq!(found(&catalog, &audience, "harbour"), vec![id.to_string()]);
    let before = catalog.dump().unwrap();
    grants.revoke(grant_id, None).unwrap();
    let path = grants.path().join("seg-00000001.log");
    let valid = std::fs::read(&path).unwrap();
    vak_session::chain::RecordChain::at(grants.path())
        .append(&serde_json::json!({"at": Utc::now(), "step": "unknown"}))
        .unwrap();
    assert!(catalog.catch_up().is_err());
    assert_eq!(
        catalog.dump().unwrap(),
        before,
        "the whole suffix rolls back"
    );
    assert!(
        catalog
            .search("harbour", &audience, &Default::default(), 20)
            .is_err()
    );
    std::fs::write(path, valid).unwrap();
    assert!(
        catalog
            .search("harbour", &audience, &Default::default(), 20)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn legacy_tolerant_grant_cursors_are_rebuilt_before_authorizing_queries() {
    let (_shared, artifacts, grants, catalog) = setup();
    let id = artifacts
        .declare(
            "spc_legacy",
            "vak",
            "legacy.md",
            ArtifactKind::File,
            Some("Legacy harbour".into()),
            None,
            None,
            None,
        )
        .unwrap();
    let guest = PrincipalId::new().to_string();
    let audience = vak_catalog::Audience {
        principal: Some(guest.clone()),
        ..Default::default()
    };
    grants
        .grant(
            grant(&guest, GrantObject::Artifact(id), Role::Viewer, None),
            None,
            None,
        )
        .unwrap();
    assert_eq!(found(&catalog, &audience, "harbour"), vec![id.to_string()]);
    vak_session::chain::RecordChain::at(grants.path())
        .append(&serde_json::json!({"at": Utc::now(), "step": "unknown"}))
        .unwrap();
    let source = vak_catalog::Source::Grants(grants.path());
    let at = vak_catalog::head_of(&grants.path());
    // Version six could skip this bad row while retaining a positive grant.
    catalog
        .seed(|tx| {
            tx.execute("UPDATE meta SET value='6' WHERE key='schema'", [])?;
            tx.execute(
                "UPDATE cursors SET segment=?1, frames=?2 WHERE source=?3",
                (at.segment as i64, at.frames as i64, source.key()),
            )?;
            Ok(())
        })
        .unwrap();
    let path = catalog.path().to_path_buf();
    let data = path.parent().unwrap();
    drop(catalog);
    let reopened = vak_catalog::Catalog::open(&path, data).unwrap();
    assert!(
        reopened
            .search("harbour", &audience, &Default::default(), 20)
            .is_err()
    );
}
