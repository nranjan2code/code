//! A conversation's lifecycle state (plan M7a-e,
//! docs/design/74-lifecycle-and-data-administration.md §2.4): whether it is
//! archived, in the trash and since when, or erased. One ref per session,
//! `lc/ses/<session id>`, moved by CAS under the writer epoch, so two
//! processes never disagree about what is in the trash and the trash
//! window has a start.

use crate::objects::{TenantObjects, objects_error};
use crate::types::SessionError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;

const PREFIX: &str = "lc/ses/";

/// What the ref of one conversation says. A conversation with no ref is
/// active and was never anything else.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationState {
    /// Hidden from the default list; still searchable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// In the trash since then: hidden everywhere, and restorable until
    /// its window ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<DateTime<Utc>>,
    /// Erased then. Nothing reads it again and nothing restores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub erased_at: Option<DateTime<Utc>>,
}

impl ConversationState {
    /// Hidden from every list, search, transcript and export.
    pub fn hidden(&self) -> bool {
        self.trashed_at.is_some() || self.erased_at.is_some()
    }
}

fn decode(bytes: &[u8]) -> Result<ConversationState, SessionError> {
    serde_json::from_slice(bytes)
        .map_err(|error| SessionError::Objects(format!("conversation state: {error}")))
}

/// The state of `session_id`; the default when it has none.
pub fn get(tenant_home: &Path, session_id: &str) -> Result<ConversationState, SessionError> {
    let store = TenantObjects::for_tenant(tenant_home)?.store();
    match store
        .get_ref(&format!("{PREFIX}{session_id}"))
        .map_err(objects_error)?
    {
        Some(value) => decode(&value.target),
        None => Ok(ConversationState::default()),
    }
}

/// Every conversation that has a state, by session id.
pub fn all(tenant_home: &Path) -> Result<Vec<(String, ConversationState)>, SessionError> {
    let store = TenantObjects::for_tenant(tenant_home)?.store();
    let mut out = Vec::new();
    for name in store.ref_names(PREFIX).map_err(objects_error)? {
        let Some(value) = store.get_ref(&name).map_err(objects_error)? else {
            continue;
        };
        if let Some(id) = name.strip_prefix(PREFIX) {
            out.push((id.to_string(), decode(&value.target)?));
        }
    }
    Ok(out)
}

/// Applies `change` to the state of `session_id` and stores the result.
/// An erased conversation's state never changes again.
pub fn update(
    tenant_home: &Path,
    session_id: &str,
    change: impl Fn(&mut ConversationState),
) -> Result<ConversationState, SessionError> {
    let mut stored = ConversationState::default();
    crate::fence::swap_ref(tenant_home, &format!("{PREFIX}{session_id}"), |current| {
        let mut state = match current {
            Some(bytes) => decode(bytes)?,
            None => ConversationState::default(),
        };
        if state.erased_at.is_none() {
            change(&mut state);
        }
        stored = state.clone();
        serde_json::to_vec(&state)
            .map(Some)
            .map_err(|error| SessionError::Objects(error.to_string()))
    })?;
    Ok(stored)
}
