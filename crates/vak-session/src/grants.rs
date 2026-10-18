//! Canonical grant records shared by the authority and its catalog projection.
//! This module owns the wire types, not authorization decisions or storage.

use crate::ids::{ArtifactId, GrantId, PrincipalId};
use crate::trace::TraceKey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a grant opens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum GrantObject {
    Artifact(ArtifactId),
    /// A conversation, by its session id.
    Conversation(String),
}

impl GrantObject {
    /// The object as one string, for indexes.
    pub fn key(&self) -> String {
        match self {
            Self::Artifact(id) => format!("artifact:{id}"),
            Self::Conversation(id) => format!("conversation:{id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Commenter,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub id: GrantId,
    /// Who it is for: a principal id (`prn_…`), as a string so a principal
    /// minted elsewhere (an invitation) needs no parsing here.
    pub principal: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    pub object: GrantObject,
    pub role: Role,
    /// A coworking invitation's audience and capabilities (doc 69).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// The hash of the token that proves an invited person holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// An artifact share's history: earlier versions are shown only from
    /// this one on (docs/design/82-library.md §8); `None` shows only the
    /// current version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_from: Option<String>,
}

/// One row of the `grants/` chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantEvent {
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    /// Who granted, revoked, broke or restored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
    #[serde(flatten)]
    pub step: GrantStep,
}

crate::impl_traced!(GrantEvent, "grant_event");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum GrantStep {
    Granted { grant: Box<Grant> },
    Revoked { grant: GrantId },
    InheritanceBroken { object: GrantObject },
    InheritanceRestored { object: GrantObject },
}
