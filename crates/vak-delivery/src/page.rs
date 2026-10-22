//! A page: an answer kept out of its conversation as its own items, so a
//! client draws it with the conversation's own views and its cards, controls
//! and shared values work as they did there (docs/design/98-answers-beyond-
//! one-card.md §12). A living page gains a version from each run of the
//! automation that names it.

use serde::{Deserialize, Serialize};

use crate::presentation::OutputItem;

/// What a page's bytes say they are.
pub const FORMAT: &str = "vak-page";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub format: String,
    /// The request the answer was given to.
    pub title: String,
    /// The answer's turn as the conversation drew it, its request first.
    pub items: Vec<OutputItem>,
    /// Its text form (the cards' text, then the reply): what search, an
    /// export and a reader that draws no cards get.
    pub text: String,
}

impl Page {
    pub fn new(title: String, items: Vec<OutputItem>, text: String) -> Self {
        Self {
            format: FORMAT.into(),
            title,
            items,
            text,
        }
    }

    /// The page these bytes hold; `None` for anything else.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice::<Self>(bytes)
            .ok()
            .filter(|page| page.format == FORMAT)
    }

    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }
}
