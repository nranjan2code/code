//! What a person did on a card (docs/design/96-answer-projection-gaps.md,
//! Q3): chose one of its options or ticked an item. Each action is a
//! `CardAction` session entry. A choice starts a turn whose directive names
//! the entry (`MessageMeta::card_action`); a tick starts none and reaches the
//! model in the next turn's tail (`<card_marks>`).

use crate::ids::PrincipalId;
use crate::trace::TraceKey;
use crate::types::{Entry, EntryPayload};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// The closed set of actions a card takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardActionKind {
    /// One of the card's options was chosen; a turn follows.
    Choose,
    /// One of the card's items was ticked or unticked; no turn follows.
    Mark,
}

/// What the action was about, typed by the card's form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardActionValues {
    /// An option of a comparison, by its label.
    Option { label: String },
    /// An item of a checkable list, by its place and its label.
    Item {
        index: usize,
        label: String,
        checked: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardActionRecord {
    /// The `Presentation` entry the action was taken on.
    pub presentation_id: String,
    pub action: CardActionKind,
    pub values: CardActionValues,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
}

crate::impl_traced!(CardActionRecord, "card_action");

/// A card's state as its actions left it: the items ticked now and the
/// option chosen last.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marked: Vec<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chosen: Option<String>,
}

/// Each card's state, folded from the actions in `entries` in order.
pub fn card_states<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> HashMap<String, CardState> {
    let mut marks: HashMap<String, BTreeMap<usize, bool>> = HashMap::new();
    let mut chosen: HashMap<String, String> = HashMap::new();
    for entry in entries {
        let EntryPayload::CardAction(record) = &entry.payload else {
            continue;
        };
        match &record.values {
            CardActionValues::Item { index, checked, .. } => {
                marks
                    .entry(record.presentation_id.clone())
                    .or_default()
                    .insert(*index, *checked);
            }
            CardActionValues::Option { label } => {
                chosen.insert(record.presentation_id.clone(), label.clone());
            }
        }
    }
    let mut states: HashMap<String, CardState> = HashMap::new();
    for (id, items) in marks {
        states.entry(id).or_default().marked = items
            .into_iter()
            .filter_map(|(index, checked)| checked.then_some(index))
            .collect();
    }
    for (id, label) in chosen {
        states.entry(id).or_default().chosen = Some(label);
    }
    states
}

/// The directive a choice starts its turn with: the person's choice, naming
/// the card it was made on.
pub fn choice_text(card_title: &str, label: &str) -> String {
    if card_title.trim().is_empty() {
        format!("I choose “{label}”.")
    } else {
        format!("I choose “{label}” from “{}”.", card_title.trim())
    }
}

/// One card's items as last marked: label and whether ticked, by place.
type ItemMarks = BTreeMap<usize, (String, bool)>;

/// The marks made between two directives, newest per item, as the tail
/// block the next turn carries; `None` when there were none.
pub fn marks_block<'a>(
    entries: impl IntoIterator<Item = &'a Entry>,
    title_of: impl Fn(&str) -> String,
) -> Option<String> {
    let mut by_card: Vec<(String, ItemMarks)> = Vec::new();
    for entry in entries {
        let EntryPayload::CardAction(record) = &entry.payload else {
            continue;
        };
        let CardActionValues::Item {
            index,
            label,
            checked,
        } = &record.values
        else {
            continue;
        };
        let position = match by_card
            .iter()
            .position(|(id, _)| *id == record.presentation_id)
        {
            Some(position) => position,
            None => {
                by_card.push((record.presentation_id.clone(), BTreeMap::new()));
                by_card.len() - 1
            }
        };
        by_card[position]
            .1
            .insert(*index, (label.clone(), *checked));
    }
    if by_card.is_empty() {
        return None;
    }
    let lines: Vec<String> = by_card
        .into_iter()
        .map(|(id, items)| {
            let changes: Vec<String> = items
                .into_values()
                .map(|(label, checked)| {
                    format!("{} “{label}”", if checked { "ticked" } else { "unticked" })
                })
                .collect();
            let title = title_of(&id);
            if title.trim().is_empty() {
                format!("On a list card the person {}.", changes.join("; "))
            } else {
                format!("On “{}” the person {}.", title.trim(), changes.join("; "))
            }
        })
        .collect();
    Some(format!("<card_marks>\n{}\n</card_marks>", lines.join("\n")))
}
