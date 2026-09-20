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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
enum GrantEvent {
    Invited {
        grant: AudienceGrant,
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("grant store error: {0}")]
    Io(#[from] std::io::Error),
    #[error("grant record is invalid: {0}")]
    Invalid(String),
}

pub fn store_path(sessions_home: &Path) -> PathBuf {
    sessions_home.join("coworking").join("grants.jsonl")
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
    append(path, &GrantEvent::Invited { grant })
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

#[cfg(test)]
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
        }
    }

    #[test]
    fn verifies_scope_without_storing_raw_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(dir.path());
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
        let path = store_path(dir.path());
        invite(&path, grant("expired", "2026-09-20T00:00:00Z")).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-21T00:00:00Z")
            .unwrap()
            .into();
        assert!(verify(&path, "expired", now).unwrap().is_none());

        invite(&path, grant("revoked", "2026-09-22T00:00:00Z")).unwrap();
        revoke(&path, "grant-1", "operator").unwrap();
        assert!(verify(&path, "revoked", now).unwrap().is_none());
    }
}
