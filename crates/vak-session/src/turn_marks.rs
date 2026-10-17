//! A person setting a turn aside, and restoring it
//! (docs/design/99-turns-that-do-not-poison.md §4 F). Each is an entry
//! (`TurnSetAside`, `TurnRestored`) naming the turn; nothing is removed.
//! A turn set aside stays in the ledger and in turn numbering, but no later
//! turn sees it: the planner, recall, citation evidence and what is learned
//! from turns skip it, as they skip a turn behind a reset.

use crate::ids::PrincipalId;
use crate::trace::TraceKey;
use crate::types::{Entry, EntryPayload};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnMarkRecord {
    /// The turn, by its directive's entry id.
    pub turn: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
}

crate::impl_traced!(TurnMarkRecord, "turn_mark");

/// The turns set aside on the chain `entries`, in order: the last mark of
/// each turn decides.
pub fn set_aside_turns<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> HashSet<String> {
    let mut aside = HashSet::new();
    for entry in entries {
        match &entry.payload {
            EntryPayload::TurnSetAside(record) => {
                aside.insert(record.turn.clone());
            }
            EntryPayload::TurnRestored(record) => {
                aside.remove(&record.turn);
            }
            _ => {}
        }
    }
    aside
}
