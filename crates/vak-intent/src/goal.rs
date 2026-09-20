use serde::{Deserialize, Serialize};

use crate::outcome::Command;

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
                GoalRelation::New | GoalRelation::Replaces => {
                    current.objective = update.request;
                    current.additions.clear();
                    current.control = GoalControlState::Active;
                }
                // A correction amends the objective and keeps what was added
                // to it; only a replacement discards the additions.
                GoalRelation::Corrects => {
                    current.objective = update.request;
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

/// The relationship a request has to the active goal.
///
/// Only an explicit [`Command`] can correct or replace the active goal, or
/// change its control state; every other request adds to it (or opens one
/// when none is active). There is no text classifier here on purpose: the
/// earlier one read "replace the deprecated API call" as a replacement of
/// the goal and cleared everything the user had added to it.
pub fn goal_relation(command: Option<&Command>, active_revision: Option<u64>) -> GoalRelation {
    match command {
        Some(Command::Status) => GoalRelation::Status,
        Some(Command::Pause) => GoalRelation::Pauses,
        Some(Command::Resume) => GoalRelation::Resumes,
        Some(Command::Cancel) => GoalRelation::Cancels,
        Some(Command::GoalFix { .. }) => {
            GoalRelation::if_active(active_revision, GoalRelation::Corrects)
        }
        Some(Command::GoalReplace { .. }) => {
            GoalRelation::if_active(active_revision, GoalRelation::Replaces)
        }
        Some(Command::Replan { .. })
        | Some(Command::AddRequirement { .. })
        | Some(Command::RemoveRequirement { .. })
        | Some(Command::Reprioritize { .. })
        | Some(Command::Approve { .. })
        | Some(Command::Reject { .. })
        | None => {
            if active_revision.is_some() {
                GoalRelation::AddsTo
            } else {
                GoalRelation::New
            }
        }
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn relation_follows_the_explicit_command_only() {
        assert_eq!(
            goal_relation(Some(&Command::Status), Some(1)),
            GoalRelation::Status
        );
        assert_eq!(goal_relation(None, Some(1)), GoalRelation::AddsTo);
        assert_eq!(goal_relation(None, None), GoalRelation::New);
        assert_eq!(
            goal_relation(
                Some(&Command::GoalFix {
                    text: "use the web version".into()
                }),
                Some(1)
            ),
            GoalRelation::Corrects
        );
        assert_eq!(
            goal_relation(
                Some(&Command::GoalReplace {
                    text: "just add the index".into()
                }),
                None
            ),
            GoalRelation::New
        );
        // Text that used to be read as a replacement is an addition now.
        assert_eq!(
            goal_relation(
                crate::outcome::parse_command("replace the deprecated API call").as_ref(),
                Some(1)
            ),
            GoalRelation::AddsTo
        );
    }

    #[test]
    fn a_correction_keeps_additions_and_a_replacement_drops_them() {
        let base = |relation, request: &str, revision| GoalUpdate {
            revision,
            relation,
            request: request.into(),
            supersedes_revision: None,
        };
        let corrected = GoalState::from_updates([
            base(GoalRelation::New, "prepare a briefing", 1),
            base(GoalRelation::AddsTo, "include sources", 2),
            base(GoalRelation::Corrects, "prepare a two-page briefing", 3),
        ])
        .unwrap();
        assert_eq!(corrected.objective, "prepare a two-page briefing");
        assert_eq!(corrected.additions, vec!["include sources"]);
        let replaced = GoalState::from_updates([
            base(GoalRelation::New, "prepare a briefing", 1),
            base(GoalRelation::AddsTo, "include sources", 2),
            base(GoalRelation::Replaces, "just send the summary", 3),
        ])
        .unwrap();
        assert!(replaced.additions.is_empty());
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
