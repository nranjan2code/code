use serde::{Deserialize, Serialize};

/// How a new human message relates to the work already in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalRelation {
    New,
    AddsTo,
    Corrects,
    Replaces,
    Status,
    Pauses,
    Resumes,
    Cancels,
}

/// A durable, append-only interpretation of a change to collaborative work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalUpdate {
    pub revision: u64,
    pub relation: GoalRelation,
    pub request: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalControlState {
    Active,
    Paused,
    Cancelled,
}

/// Deterministic current-state projection over append-only goal updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalState {
    pub revision: u64,
    pub objective: String,
    pub additions: Vec<String>,
    pub superseded_revisions: Vec<u64>,
    pub control: GoalControlState,
}

impl GoalState {
    pub fn from_updates(updates: impl IntoIterator<Item = GoalUpdate>) -> Option<Self> {
        let mut state = None;
        for update in updates {
            let current = state.get_or_insert_with(|| Self {
                revision: 0,
                objective: String::new(),
                additions: Vec::new(),
                superseded_revisions: Vec::new(),
                control: GoalControlState::Active,
            });
            current.revision = update.revision;
            match update.relation {
                GoalRelation::New | GoalRelation::Replaces | GoalRelation::Corrects => {
                    current.objective = update.request;
                    current.additions.clear();
                    current.control = GoalControlState::Active;
                }
                GoalRelation::AddsTo => current.additions.push(update.request),
                GoalRelation::Pauses => current.control = GoalControlState::Paused,
                GoalRelation::Resumes => current.control = GoalControlState::Active,
                GoalRelation::Cancels => current.control = GoalControlState::Cancelled,
                GoalRelation::Status => {}
            }
            if let Some(revision) = update.supersedes_revision
                && !current.superseded_revisions.contains(&revision)
            {
                current.superseded_revisions.push(revision);
            }
        }
        state
    }
}

/// Classifies the control relationship without pretending to understand the
/// domain request. Domain planning still happens after this audit fact.
pub fn classify_goal_update(request: &str, active_revision: Option<u64>) -> GoalRelation {
    let text = request.trim().to_ascii_lowercase();
    let command = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|word| !word.is_empty())
        .unwrap_or("");
    if command == "status"
        || text.starts_with("how is ")
        || text.starts_with("what is the progress")
        || text.starts_with("what is the status")
    {
        GoalRelation::Status
    } else if command == "pause" || command == "hold" {
        GoalRelation::Pauses
    } else if command == "resume" || command == "continue" {
        GoalRelation::Resumes
    } else if command == "cancel" || command == "stop" || command == "abort" {
        GoalRelation::Cancels
    } else if text.starts_with("actually ")
        || text.starts_with("correction")
        || text.starts_with("that's wrong")
        || text.starts_with("fix that")
    {
        GoalRelation::if_active(active_revision, GoalRelation::Corrects)
    } else if text.contains("instead")
        || text.contains("change of mind")
        || text.contains("replace")
        || text.contains("forget that")
    {
        GoalRelation::if_active(active_revision, GoalRelation::Replaces)
    } else if active_revision.is_some() {
        GoalRelation::AddsTo
    } else {
        GoalRelation::New
    }
}

impl GoalRelation {
    fn if_active(active_revision: Option<u64>, active: Self) -> Self {
        if active_revision.is_some() {
            active
        } else {
            Self::New
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_collaborative_updates() {
        assert_eq!(
            classify_goal_update("what is the status?", Some(1)),
            GoalRelation::Status
        );
        assert_eq!(
            classify_goal_update("also include citations", Some(1)),
            GoalRelation::AddsTo
        );
        assert_eq!(
            classify_goal_update("actually use the web version instead", Some(1)),
            GoalRelation::Corrects
        );
        assert_eq!(
            classify_goal_update("start a new thing", None),
            GoalRelation::New
        );
        assert_eq!(
            classify_goal_update("Implement a progress bar", None),
            GoalRelation::New
        );
        assert_eq!(
            classify_goal_update("Fix the cancellation button", Some(1)),
            GoalRelation::AddsTo
        );
    }

    #[test]
    fn projects_objective_additions_and_controls() {
        let state = GoalState::from_updates([
            GoalUpdate {
                revision: 1,
                relation: GoalRelation::New,
                request: "prepare a briefing".into(),
                supersedes_revision: None,
            },
            GoalUpdate {
                revision: 2,
                relation: GoalRelation::AddsTo,
                request: "include sources".into(),
                supersedes_revision: None,
            },
            GoalUpdate {
                revision: 3,
                relation: GoalRelation::Pauses,
                request: "pause".into(),
                supersedes_revision: None,
            },
        ]);
        assert!(state.is_some());
        if let Some(state) = state {
            assert_eq!(state.objective, "prepare a briefing");
            assert_eq!(state.additions, vec!["include sources"]);
            assert_eq!(state.control, GoalControlState::Paused);
        }
    }
}
