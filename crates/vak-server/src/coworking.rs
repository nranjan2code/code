//! Coworking invitations for shared Agent conversations (doc 69), as
//! conversation grants in the one `grants/` chain (`vak_core::grants`,
//! plan M8.2). An invitation carries the hash of its token, never the
//! token, an audience, capabilities and an expiry.
//!
//! This module owns the invitation shape only. HTTP admission is wired
//! separately so an invitation cannot become usable before every shared
//! route enforces the same audience decision.

use chrono::{DateTime, Utc};
use serde::Serialize;
use vak_core::grants::{Grant, GrantError, GrantObject, Grants, Role};
use vak_session::ids::GrantId;

pub use vak_core::grants::{GrantStatus, token_hash};

/// What an invitation is made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudienceGrant {
    pub grant_id: GrantId,
    pub principal_id: String,
    pub display_name: String,
    pub conversation_id: String,
    pub audience_id: String,
    pub capabilities: Vec<String>,
    pub token_hash: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub trace: Option<vak_session::trace::TraceKey>,
    pub actor: Option<vak_session::ids::PrincipalId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPrincipal {
    pub grant_id: String,
    pub principal_id: String,
    pub display_name: String,
    pub conversation_id: String,
    pub audience_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GrantSummary {
    pub grant_id: String,
    pub principal_id: String,
    pub display_name: String,
    pub conversation_id: String,
    pub audience_id: String,
    pub capabilities: Vec<String>,
    pub created_at: String,
    pub expires_at: String,
    pub status: GrantStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

/// The grants of the server's data home.
pub fn grants(state: &crate::AppState) -> Grants {
    Grants::at(&state.core.shared_scope())
}

pub fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::now_v7().simple(),
        uuid::Uuid::now_v7().simple()
    )
}

/// The role a set of coworking capabilities amounts to.
fn role_of(capabilities: &[String]) -> Role {
    if capabilities.iter().any(|capability| capability == "edit") {
        Role::Editor
    } else if capabilities
        .iter()
        .any(|capability| capability == "comment")
    {
        Role::Commenter
    } else {
        Role::Viewer
    }
}

pub fn invite(grants: &Grants, grant: AudienceGrant) -> Result<(), GrantError> {
    if grant.principal_id.trim().is_empty()
        || grant.conversation_id.trim().is_empty()
        || grant.audience_id.trim().is_empty()
        || grant.token_hash.trim().is_empty()
        || grant.capabilities.is_empty()
    {
        return Err(GrantError::Invalid("grant fields must be explicit".into()));
    }
    let role = role_of(&grant.capabilities);
    grants.grant(
        Grant {
            id: grant.grant_id,
            principal: grant.principal_id,
            display_name: grant.display_name,
            object: GrantObject::Conversation(grant.conversation_id),
            role,
            audience_id: Some(grant.audience_id),
            capabilities: grant.capabilities,
            token_hash: Some(grant.token_hash),
            created_at: grant.created_at,
            expires_at: Some(grant.expires_at),
            history_from: None,
        },
        grant.actor,
        grant.trace.as_ref(),
    )
}

pub fn revoke(
    grants: &Grants,
    grant_id: &str,
    actor: Option<vak_session::ids::PrincipalId>,
) -> Result<(), GrantError> {
    let id = GrantId::parse(grant_id).map_err(|_| GrantError::NotFound(grant_id.into()))?;
    grants.revoke(id, actor)
}

/// The active invitation `token` proves, if any.
pub fn verify(
    grants: &Grants,
    token: &str,
    now: DateTime<Utc>,
) -> Result<Option<VerifiedPrincipal>, GrantError> {
    Ok(grants.verify_token(token, now)?.and_then(|grant| {
        let GrantObject::Conversation(conversation_id) = grant.object else {
            return None;
        };
        Some(VerifiedPrincipal {
            grant_id: grant.id.to_string(),
            principal_id: grant.principal,
            display_name: grant.display_name,
            conversation_id,
            audience_id: grant.audience_id.unwrap_or_default(),
            capabilities: grant.capabilities,
        })
    }))
}

/// The invitations to `conversation_id`, newest first, without tokens.
pub fn list(
    grants: &Grants,
    conversation_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<GrantSummary>, GrantError> {
    Ok(grants
        .on(&GrantObject::Conversation(conversation_id.to_string()))?
        .into_iter()
        .filter(|held| held.grant.token_hash.is_some())
        .map(|held| GrantSummary {
            status: held.status(now),
            grant_id: held.grant.id.to_string(),
            principal_id: held.grant.principal,
            display_name: held.grant.display_name,
            conversation_id: conversation_id.to_string(),
            audience_id: held.grant.audience_id.unwrap_or_default(),
            capabilities: held.grant.capabilities,
            created_at: held.grant.created_at.to_rfc3339(),
            expires_at: held
                .grant
                .expires_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
            revoked_at: held.revoked_at.map(|at| at.to_rfc3339()),
        })
        .collect())
}

/// A stable grant id for a test fixture's name.
#[cfg(test)]
pub(crate) fn test_grant_id(name: &str) -> GrantId {
    GrantId::derived(name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().into()
    }

    fn grant(token: &str, expires_at: &str) -> AudienceGrant {
        AudienceGrant {
            grant_id: GrantId::new(),
            principal_id: "person-2".into(),
            display_name: "Asha".into(),
            conversation_id: "session-1".into(),
            audience_id: "conversation:session-1".into(),
            capabilities: vec!["read".into(), "comment".into()],
            token_hash: token_hash(token),
            created_at: at("2026-09-20T00:00:00Z"),
            expires_at: at(expires_at),
            trace: None,
            actor: None,
        }
    }

    fn store() -> (tempfile::TempDir, Grants) {
        let dir = tempfile::tempdir().unwrap();
        let grants = Grants::at(&vak_config::scope::SharedScope::new(dir.path()));
        (dir, grants)
    }

    #[test]
    fn verifies_scope_without_storing_raw_token() {
        let (_dir, grants) = store();
        let token = generate_token();
        invite(&grants, grant(&token, "2026-09-22T00:00:00Z")).unwrap();
        let text = vak_session::chain::RecordChain::at(grants.path()).text();
        assert!(!text.contains(&token));
        let principal = verify(&grants, &token, at("2026-09-21T00:00:00Z"))
            .unwrap()
            .unwrap();
        assert_eq!(principal.principal_id, "person-2");
        assert_eq!(principal.capabilities, vec!["read", "comment"]);
    }

    #[test]
    fn an_unreadable_grant_row_fails_closed() {
        let (_dir, grants) = store();
        let token = generate_token();
        invite(&grants, grant(&token, "2026-09-22T00:00:00Z")).unwrap();
        vak_session::chain::RecordChain::at(grants.path())
            .append(&serde_json::json!({"not": "a grant event"}))
            .unwrap();
        assert!(
            verify(&grants, &token, at("2026-09-21T00:00:00Z")).is_err(),
            "a row that may be a revocation is never skipped"
        );
    }

    #[test]
    fn expiry_and_revocation_fail_closed() {
        let (_dir, grants) = store();
        invite(&grants, grant("expired", "2026-09-20T00:00:00Z")).unwrap();
        let now = at("2026-09-21T00:00:00Z");
        assert!(verify(&grants, "expired", now).unwrap().is_none());

        let revoked = grant("revoked", "2026-09-22T00:00:00Z");
        let id = revoked.grant_id.to_string();
        invite(&grants, revoked).unwrap();
        revoke(&grants, &id, None).unwrap();
        assert!(verify(&grants, "revoked", now).unwrap().is_none());
    }

    #[test]
    fn listing_omits_tokens_and_reports_lifecycle() {
        let (_dir, grants) = store();
        let first = grant("secret-token", "2026-09-22T00:00:00Z");
        let id = first.grant_id;
        invite(&grants, first.clone()).unwrap();
        let mut duplicate = grant("replacement", "2026-09-23T00:00:00Z");
        duplicate.grant_id = id;
        assert!(invite(&grants, duplicate).is_err());
        let now = at("2026-09-21T00:00:00Z");
        let listed = list(&grants, "session-1", now).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, GrantStatus::Active);
        let serialized = serde_json::to_string(&listed).unwrap();
        assert!(!serialized.contains("token_hash"));
        assert!(!serialized.contains("secret-token"));

        revoke(&grants, &id.to_string(), None).unwrap();
        let listed = list(&grants, "session-1", now).unwrap();
        assert_eq!(listed[0].status, GrantStatus::Revoked);
        assert!(listed[0].revoked_at.is_some());
    }
}
