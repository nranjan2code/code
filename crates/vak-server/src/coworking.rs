//! Durable, append-only audience grants for shared Agent conversations.
//!
//! This module owns capability data only. HTTP admission is wired separately
//! so an invitation cannot become usable before every shared route enforces
//! the same audience decision.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use subtle::ConstantTimeEq;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudienceGrant {
    pub grant_id: String,
    pub principal_id: String,
    pub display_name: String,
    pub conversation_id: String,
    pub audience_id: String,
    pub capabilities: Vec<String>,
    pub token_hash: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(AudienceGrant, "audience_grant");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
enum GrantEvent {
    Invited {
        grant: Box<AudienceGrant>,
    },
    Revoked {
        grant_id: String,
        revoked_at: String,
        actor_id: String,
    },
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    Active,
    Expired,
    Revoked,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("grant store error: {0}")]
    Io(#[from] std::io::Error),
    #[error("grant record is invalid: {0}")]
    Invalid(String),
}

/// The coworking grant store of `session_agent`'s home (D25, as
/// [`vak_config::scope::AgentScope::coworking_grants`]).
pub fn store_path(scope: &vak_config::scope::AgentScope, session_agent: &str) -> PathBuf {
    scope.coworking_grants(session_agent)
}

pub fn generate_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::now_v7().simple(),
        uuid::Uuid::now_v7().simple()
    )
}

pub fn token_hash(token: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(token.as_bytes()))
}

fn append(path: &Path, event: &GrantEvent) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(event).map_err(|error| Error::Invalid(error.to_string()))?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

fn load(path: &Path) -> Result<Vec<GrantEvent>, Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    std::fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|error| Error::Invalid(error.to_string())))
        .collect()
}

pub fn invite(path: &Path, grant: AudienceGrant) -> Result<(), Error> {
    if grant.principal_id.trim().is_empty()
        || grant.conversation_id.trim().is_empty()
        || grant.audience_id.trim().is_empty()
        || grant.token_hash.trim().is_empty()
        || grant.capabilities.is_empty()
    {
        return Err(Error::Invalid("grant fields must be explicit".into()));
    }
    if load(path)?.iter().any(
        |event| matches!(event, GrantEvent::Invited { grant: existing } if existing.grant_id == grant.grant_id),
    ) {
        return Err(Error::Invalid("grant id already exists".into()));
    }
    append(
        path,
        &GrantEvent::Invited {
            grant: Box::new(grant),
        },
    )
}

pub fn revoke(path: &Path, grant_id: &str, actor_id: &str) -> Result<(), Error> {
    if grant_id.trim().is_empty() || actor_id.trim().is_empty() {
        return Err(Error::Invalid("revocation identity is required".into()));
    }
    append(
        path,
        &GrantEvent::Revoked {
            grant_id: grant_id.into(),
            revoked_at: chrono::Utc::now().to_rfc3339(),
            actor_id: actor_id.into(),
        },
    )
}

pub fn verify(
    path: &Path,
    token: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<VerifiedPrincipal>, Error> {
    let events = load(path)?;
    let revoked: std::collections::HashSet<&str> = events
        .iter()
        .filter_map(|event| match event {
            GrantEvent::Revoked { grant_id, .. } => Some(grant_id.as_str()),
            _ => None,
        })
        .collect();
    let supplied = token_hash(token);
    for event in events.iter().rev() {
        let GrantEvent::Invited { grant } = event else {
            continue;
        };
        if revoked.contains(grant.grant_id.as_str()) {
            continue;
        }
        let matches: bool = supplied
            .as_bytes()
            .ct_eq(grant.token_hash.as_bytes())
            .into();
        if !matches {
            continue;
        }
        let expires = chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
            .map_err(|error| Error::Invalid(error.to_string()))?
            .with_timezone(&chrono::Utc);
        if expires <= now {
            return Ok(None);
        }
        return Ok(Some(VerifiedPrincipal {
            grant_id: grant.grant_id.clone(),
            principal_id: grant.principal_id.clone(),
            display_name: grant.display_name.clone(),
            conversation_id: grant.conversation_id.clone(),
            audience_id: grant.audience_id.clone(),
            capabilities: grant.capabilities.clone(),
        }));
    }
    Ok(None)
}

pub fn list(
    path: &Path,
    conversation_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<GrantSummary>, Error> {
    let events = load(path)?;
    let revoked: std::collections::HashMap<&str, &str> = events
        .iter()
        .filter_map(|event| match event {
            GrantEvent::Revoked {
                grant_id,
                revoked_at,
                ..
            } => Some((grant_id.as_str(), revoked_at.as_str())),
            _ => None,
        })
        .collect();
    let mut summaries = Vec::new();
    for event in &events {
        let GrantEvent::Invited { grant } = event else {
            continue;
        };
        if grant.conversation_id != conversation_id {
            continue;
        }
        let expires = chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
            .map_err(|error| Error::Invalid(error.to_string()))?
            .with_timezone(&chrono::Utc);
        let revoked_at = revoked.get(grant.grant_id.as_str()).copied();
        summaries.push(GrantSummary {
            grant_id: grant.grant_id.clone(),
            principal_id: grant.principal_id.clone(),
            display_name: grant.display_name.clone(),
            conversation_id: grant.conversation_id.clone(),
            audience_id: grant.audience_id.clone(),
            capabilities: grant.capabilities.clone(),
            created_at: grant.created_at.clone(),
            expires_at: grant.expires_at.clone(),
            status: if revoked_at.is_some() {
                GrantStatus::Revoked
            } else if expires <= now {
                GrantStatus::Expired
            } else {
                GrantStatus::Active
            },
            revoked_at: revoked_at.map(ToOwned::to_owned),
        });
    }
    summaries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    Ok(summaries)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn grant(token: &str, expires_at: &str) -> AudienceGrant {
        AudienceGrant {
            grant_id: "grant-1".into(),
            principal_id: "person-2".into(),
            display_name: "Asha".into(),
            conversation_id: "session-1".into(),
            audience_id: "conversation:session-1".into(),
            capabilities: vec!["read".into(), "comment".into()],
            token_hash: token_hash(token),
            created_at: "2026-09-20T00:00:00Z".into(),
            expires_at: expires_at.into(),
            trace: None,
            actor: None,
        }
    }

    #[test]
    fn verifies_scope_without_storing_raw_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&vak_config::scope::AgentScope::new(dir.path()), "vak");
        let token = generate_token();
        invite(&path, grant(&token, "2026-09-22T00:00:00Z")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&token));
        let principal = verify(
            &path,
            &token,
            chrono::DateTime::parse_from_rfc3339("2026-09-21T00:00:00Z")
                .unwrap()
                .into(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(principal.principal_id, "person-2");
        assert_eq!(principal.capabilities, vec!["read", "comment"]);
    }

    #[test]
    fn expiry_and_revocation_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&vak_config::scope::AgentScope::new(dir.path()), "vak");
        invite(&path, grant("expired", "2026-09-20T00:00:00Z")).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-21T00:00:00Z")
            .unwrap()
            .into();
        assert!(verify(&path, "expired", now).unwrap().is_none());

        let mut revoked = grant("revoked", "2026-09-22T00:00:00Z");
        revoked.grant_id = "grant-2".into();
        invite(&path, revoked).unwrap();
        revoke(&path, "grant-2", "operator").unwrap();
        assert!(verify(&path, "revoked", now).unwrap().is_none());
    }

    #[test]
    fn listing_omits_tokens_and_reports_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(&vak_config::scope::AgentScope::new(dir.path()), "vak");
        invite(&path, grant("secret-token", "2026-09-22T00:00:00Z")).unwrap();
        let duplicate = invite(&path, grant("replacement", "2026-09-23T00:00:00Z"));
        assert!(duplicate.is_err());
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-21T00:00:00Z")
            .unwrap()
            .into();
        let listed = list(&path, "session-1", now).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, GrantStatus::Active);
        let serialized = serde_json::to_string(&listed).unwrap();
        assert!(!serialized.contains("token_hash"));
        assert!(!serialized.contains("secret-token"));

        revoke(&path, "grant-1", "operator").unwrap();
        let listed = list(&path, "session-1", now).unwrap();
        assert_eq!(listed[0].status, GrantStatus::Revoked);
        assert!(listed[0].revoked_at.is_some());
    }
}
