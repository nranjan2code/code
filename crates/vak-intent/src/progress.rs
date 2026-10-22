//! Optional, turn-local progress declared by the model, separate from outcomes
//! and presentation layout. These states make no claim of verified completion.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState {
    Pending,
    InProgress,
    Done,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressItem {
    pub label: String,
    pub state: ProgressState,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressList {
    pub items: Vec<ProgressItem>,
}

impl ProgressList {
    pub fn validate(&self) -> Result<(), String> {
        for item in &self.items {
            if item.label.trim().is_empty() {
                return Err("Every progress item needs a label.".into());
            }
            if item.state == ProgressState::Dropped
                && item
                    .reason
                    .as_ref()
                    .is_none_or(|reason| reason.trim().is_empty())
            {
                return Err("A dropped progress item needs a reason.".into());
            }
        }
        Ok(())
    }

    pub fn open_labels(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|item| {
                matches!(
                    item.state,
                    ProgressState::Pending | ProgressState::InProgress
                )
            })
            .map(|item| item.label.clone())
            .collect()
    }
}
