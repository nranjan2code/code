//! Grants (plan M8, docs/design/73-data-architecture-and-lifecycle.md §10):
//! who may reach an artifact or a conversation beyond its own audience.
//!
//! Permissions work like SharePoint. An object inherits its parent's
//! audience (an artifact its Space's: the owner and the Space's Agents).
//! A grant gives one principal a role (viewer, commenter, editor) on one
//! object. Breaking inheritance makes an object's grants its only audience
//! besides its owner; restoring it gives the parent's audience back.
//! Doc 69's coworking invitations are conversation grants with a token and
//! an expiry, in the same chain.
//!
//! Every step is a row of the `grants/` record chain; a decision reads the
//! rollup over it (`vak_session::rollup`), so a revocation takes effect at
//! the next read, everywhere.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use subtle::ConstantTimeEq;
use vak_config::scope::SharedScope;
use vak_session::chain::RecordChain;
use vak_session::ids::{ArtifactId, GrantId, PrincipalId};
use vak_session::trace::TraceKey;

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

vak_session::impl_traced!(GrantEvent, "grant_event");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum GrantStep {
    Granted { grant: Box<Grant> },
    Revoked { grant: GrantId },
    InheritanceBroken { object: GrantObject },
    InheritanceRestored { object: GrantObject },
}

/// Where a grant stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    Active,
    Expired,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    pub grant: Grant,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Held {
    pub fn status(&self, now: DateTime<Utc>) -> GrantStatus {
        if self.revoked_at.is_some() {
            GrantStatus::Revoked
        } else if self.grant.expires_at.is_some_and(|expires| expires <= now) {
            GrantStatus::Expired
        } else {
            GrantStatus::Active
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    grants: BTreeMap<String, Held>,
    broken: BTreeSet<String>,
    /// A row did not decode. It may have been a revocation, so every
    /// decision fails closed from then on; it is never skipped.
    #[serde(default)]
    unreadable: bool,
}

fn fold(state: &mut State, bytes: &[u8]) {
    let Ok(event) = serde_json::from_slice::<GrantEvent>(bytes) else {
        state.unreadable = true;
        return;
    };
    match event.step {
        GrantStep::Granted { grant } => {
            state.grants.entry(grant.id.to_string()).or_insert(Held {
                grant: *grant,
                revoked_at: None,
            });
        }
        GrantStep::Revoked { grant } => {
            if let Some(held) = state.grants.get_mut(&grant.to_string()) {
                held.revoked_at.get_or_insert(event.at);
            }
        }
        GrantStep::InheritanceBroken { object } => {
            state.broken.insert(object.key());
        }
        GrantStep::InheritanceRestored { object } => {
            state.broken.remove(&object.key());
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GrantError {
    #[error("{0}")]
    Invalid(String),
    #[error("no grant {0}")]
    NotFound(String),
    #[error("grant store: {0}")]
    Store(String),
    /// The chain holds a row that does not decode.
    #[error("the grant records hold a row that cannot be read")]
    Unreadable,
}

/// The hash a grant keeps of an invitation token.
pub fn token_hash(token: &str) -> String {
    use sha2::Digest;
    format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(token.as_bytes()))
    )
}

/// The grants of a data home.
#[derive(Debug, Clone)]
pub struct Grants {
    chain: RecordChain,
    rollup: vak_session::rollup::Rollup,
}

impl Grants {
    pub fn at(shared: &SharedScope) -> Self {
        Self {
            chain: RecordChain::at(shared.grants()),
            rollup: vak_session::rollup::Rollup::new(shared.grants(), shared.grants_rollup()),
        }
    }

    pub fn path(&self) -> PathBuf {
        self.chain.path().to_path_buf()
    }

    fn state(&self) -> Result<State, GrantError> {
        let state = self.rollup.read(fold);
        if state.unreadable {
            return Err(GrantError::Unreadable);
        }
        Ok(state)
    }

    fn append(
        &self,
        step: GrantStep,
        actor: Option<PrincipalId>,
        trace: Option<&TraceKey>,
    ) -> Result<(), GrantError> {
        self.chain
            .append(&GrantEvent {
                at: Utc::now(),
                trace: trace.cloned(),
                actor: actor.or_else(|| trace.and_then(|trace| trace.actor)),
                step,
            })
            .map_err(|error| GrantError::Store(error.to_string()))
    }

    /// Records a new grant. A coworking invitation (a token) must name its
    /// audience, capabilities and expiry.
    pub fn grant(
        &self,
        grant: Grant,
        actor: Option<PrincipalId>,
        trace: Option<&TraceKey>,
    ) -> Result<(), GrantError> {
        if grant.principal.trim().is_empty() {
            return Err(GrantError::Invalid("a grant names its principal".into()));
        }
        if grant.token_hash.is_some()
            && (grant
                .audience_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
                || grant.capabilities.is_empty()
                || grant.expires_at.is_none())
        {
            return Err(GrantError::Invalid(
                "an invitation names its audience, capabilities and expiry".into(),
            ));
        }
        if self.state()?.grants.contains_key(&grant.id.to_string()) {
            return Err(GrantError::Invalid("grant id already exists".into()));
        }
        self.append(
            GrantStep::Granted {
                grant: Box::new(grant),
            },
            actor,
            trace,
        )
    }

    pub fn revoke(&self, grant: GrantId, actor: Option<PrincipalId>) -> Result<(), GrantError> {
        if !self.state()?.grants.contains_key(&grant.to_string()) {
            return Err(GrantError::NotFound(grant.to_string()));
        }
        self.append(GrantStep::Revoked { grant }, actor, None)
    }

    /// Makes `object`'s grants its only audience besides its owner.
    pub fn break_inheritance(
        &self,
        object: GrantObject,
        actor: Option<PrincipalId>,
    ) -> Result<(), GrantError> {
        self.append(GrantStep::InheritanceBroken { object }, actor, None)
    }

    /// Gives `object` its parent's audience again.
    pub fn restore_inheritance(
        &self,
        object: GrantObject,
        actor: Option<PrincipalId>,
    ) -> Result<(), GrantError> {
        self.append(GrantStep::InheritanceRestored { object }, actor, None)
    }

    pub fn inherits(&self, object: &GrantObject) -> Result<bool, GrantError> {
        Ok(!self.state()?.broken.contains(&object.key()))
    }

    /// Every grant on `object`, newest first, with whether it still holds.
    pub fn on(&self, object: &GrantObject) -> Result<Vec<Held>, GrantError> {
        let mut found: Vec<Held> = self
            .state()?
            .grants
            .into_values()
            .filter(|held| &held.grant.object == object)
            .collect();
        found.sort_by_key(|held| std::cmp::Reverse(held.grant.created_at));
        Ok(found)
    }

    /// Whether `principal` may act as `role` on `object` at `now`: an
    /// active grant of that role or a stronger one, or, while the object
    /// inherits, being in its parent's audience (`in_parent`, which the
    /// caller decides: the owner, the Space's Agents). Records that cannot
    /// be read allow nothing.
    pub fn may(
        &self,
        principal: &str,
        object: &GrantObject,
        role: Role,
        in_parent: bool,
        now: DateTime<Utc>,
    ) -> bool {
        let Ok(state) = self.state() else {
            return false;
        };
        let granted = state.grants.values().any(|held| {
            held.grant.principal == principal
                && &held.grant.object == object
                && held.grant.role >= role
                && held.status(now) == GrantStatus::Active
        });
        granted || (in_parent && !state.broken.contains(&object.key()))
    }

    /// The active invitation `token` proves, if any. Hashes compare in
    /// constant time; an expired or revoked one proves nothing.
    pub fn verify_token(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Grant>, GrantError> {
        let supplied = token_hash(token);
        Ok(self.state()?.grants.into_values().find_map(|held| {
            let hash = held.grant.token_hash.as_deref()?;
            let matches: bool = supplied.as_bytes().ct_eq(hash.as_bytes()).into();
            (matches && held.status(now) == GrantStatus::Active).then_some(held.grant)
        }))
    }
}
