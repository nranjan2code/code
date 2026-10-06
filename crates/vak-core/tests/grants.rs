//! M8 exit tests `share_inherits_and_breaks` and
//! `revoked_grant_hides_from_search` (data-architecture plan M8.2): an
//! artifact inherits its Space's audience until a share breaks
//! inheritance, a grant opens one object to one principal for its role,
//! and a revoked or expired grant hides the object from that principal's
//! search at once.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use vak_core::artifacts::{ArtifactKind, Artifacts};
use vak_core::grants::{Grant, GrantObject, Grants, Role};
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
