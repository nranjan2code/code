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
    /// Whether a person stated this as the goal (`/goal fix`, `/goal
    /// replace`) rather than the runtime taking a message as one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
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
    /// Whether a person stated the objective as the goal. Otherwise it is
    /// only the conversation's first message, and it is never presented to
    /// the model as the primary objective — "hi" is not one.
    #[serde(default)]
    pub explicit: bool,
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
                explicit: false,
            });
            current.revision = update.revision;
            match update.relation {
                GoalRelation::New | GoalRelation::Replaces => {
                    current.objective = update.request;
                    current.additions.clear();
                    current.control = GoalControlState::Active;
                    current.explicit = update.explicit;
                }
                // A correction amends the objective and keeps what was added
                // to it; only a replacement discards the additions.
                GoalRelation::Corrects => {
                    current.objective = update.request;
                    current.control = GoalControlState::Active;
                    current.explicit = update.explicit;
                }
                // Adding to a paused goal is working on it again: the turn
                // that carries the addition runs.
                GoalRelation::AddsTo => {
                    current.additions.push(update.request);
                    current.control = GoalControlState::Active;
                }
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

    /// Whether there is a goal to add to or amend: one that has not been
    /// called off.
    pub fn is_live(&self) -> bool {
        self.control != GoalControlState::Cancelled
    }
}

/// The relationship a request has to the goal in progress.
///
/// Only an explicit [`Command`] can correct or replace the goal, or change
/// its control state; every other request adds to it, or opens one when
/// there is none — including when the last one was called off, because a
/// message after `/stop` starts over rather than adding to cancelled work.
/// There is no text classifier here on purpose: the earlier one read
/// "replace the deprecated API call" as a replacement of the goal and
/// cleared everything the user had added to it.
pub fn goal_relation(command: Option<&Command>, goal: Option<&GoalState>) -> GoalRelation {
    let live = goal.is_some_and(GoalState::is_live);
    let if_live = |relation| if live { relation } else { GoalRelation::New };
    match command {
        Some(Command::Status) => GoalRelation::Status,
        Some(Command::Pause) => GoalRelation::Pauses,
        Some(Command::Resume) => GoalRelation::Resumes,
        Some(Command::Cancel) => GoalRelation::Cancels,
        Some(Command::GoalFix { .. }) => if_live(GoalRelation::Corrects),
        Some(Command::GoalReplace { .. }) => if_live(GoalRelation::Replaces),
        Some(Command::Replan { .. })
        | Some(Command::AddRequirement { .. })
        | Some(Command::RemoveRequirement { .. })
        | Some(Command::Reprioritize { .. })
        | Some(Command::Approve { .. })
        | Some(Command::Reject { .. })
        | Some(Command::UntilDone { .. })
        | None => if_live(GoalRelation::AddsTo),
    }
}

/// The next goal update for a message, given the goal so far and the last
/// update's revision. The one place that decides it — for the message that
/// starts a turn and for one steered into a running turn — so both number
/// revisions the same way and read commands the same way.
pub fn next_goal_update(
    text: &str,
    goal: Option<&GoalState>,
    latest_revision: Option<u64>,
) -> GoalUpdate {
    let command = crate::outcome::parse_command(text);
    let relation = goal_relation(command.as_ref(), goal);
    let explicit = matches!(
        command,
        Some(Command::GoalFix { .. } | Command::GoalReplace { .. })
    );
    GoalUpdate {
        revision: latest_revision.unwrap_or(0).saturating_add(1),
        relation,
        request: command
            .as_ref()
            .and_then(Command::text)
            .map(str::to_string)
            .unwrap_or_else(|| text.to_string()),
        supersedes_revision: matches!(relation, GoalRelation::Corrects | GoalRelation::Replaces)
            .then(|| goal.map(|goal| goal.revision))
            .flatten(),
        explicit,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn update(relation: GoalRelation, request: &str, revision: u64) -> GoalUpdate {
        GoalUpdate {
            revision,
            relation,
            request: request.into(),
            supersedes_revision: None,
            explicit: false,
        }
    }

    fn state(updates: &[GoalUpdate]) -> GoalState {
        GoalState::from_updates(updates.iter().cloned()).unwrap()
    }

    #[test]
    fn relation_follows_the_explicit_command_only() {
        let active = state(&[update(GoalRelation::New, "prepare a briefing", 1)]);
        assert_eq!(
            goal_relation(Some(&Command::Status), Some(&active)),
            GoalRelation::Status
        );
        assert_eq!(goal_relation(None, Some(&active)), GoalRelation::AddsTo);
        assert_eq!(goal_relation(None, None), GoalRelation::New);
        assert_eq!(
            goal_relation(
                Some(&Command::GoalFix {
                    text: "use the web version".into()
                }),
                Some(&active)
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
                Some(&active)
            ),
            GoalRelation::AddsTo
        );
    }

    /// A message after `/stop` starts over; it is not an addition to work
    /// that was called off.
    #[test]
    fn a_cancelled_goal_is_not_added_to() {
        let cancelled = state(&[
            update(GoalRelation::New, "prepare a briefing", 1),
            update(GoalRelation::Cancels, "stop", 2),
        ]);
        assert_eq!(goal_relation(None, Some(&cancelled)), GoalRelation::New);
        let next = next_goal_update("book a table for two", Some(&cancelled), Some(2));
        assert_eq!(next.relation, GoalRelation::New);
        assert_eq!(next.revision, 3);
        let after = state(&[
            update(GoalRelation::New, "prepare a briefing", 1),
            update(GoalRelation::Cancels, "stop", 2),
            next,
        ]);
        assert_eq!(after.objective, "book a table for two");
        assert_eq!(after.control, GoalControlState::Active);
    }

    /// Adding to a paused goal works on it again.
    #[test]
    fn an_addition_resumes_a_paused_goal() {
        let resumed = state(&[
            update(GoalRelation::New, "prepare a briefing", 1),
            update(GoalRelation::Pauses, "pause", 2),
            update(GoalRelation::AddsTo, "include sources", 3),
        ]);
        assert_eq!(resumed.control, GoalControlState::Active);
    }

    /// Only a goal a person stated is one; the first message of a
    /// conversation is not its objective by default.
    #[test]
    fn only_a_stated_goal_is_explicit() {
        let inferred = next_goal_update("hi", None, None);
        assert!(!inferred.explicit);
        assert!(!state(&[inferred]).explicit);
        let stated = next_goal_update("/goal replace ship the release notes", None, None);
        assert!(stated.explicit);
        assert_eq!(stated.request, "ship the release notes");
        assert!(state(&[stated]).explicit);
    }

    /// Revisions count every update, so two call sites can never reuse one.
    #[test]
    fn revisions_follow_the_latest_update_of_any_kind() {
        let paused = state(&[
            update(GoalRelation::New, "prepare a briefing", 1),
            update(GoalRelation::Pauses, "pause", 2),
        ]);
        let next = next_goal_update("also include sources", Some(&paused), Some(2));
        assert_eq!(next.revision, 3);
        assert_eq!(next.relation, GoalRelation::AddsTo);
    }

    #[test]
    fn a_correction_keeps_additions_and_a_replacement_drops_them() {
        let base = |relation, request: &str, revision| update(relation, request, revision);
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
            update(GoalRelation::New, "prepare a briefing", 1),
            update(GoalRelation::AddsTo, "include sources", 2),
            update(GoalRelation::Pauses, "pause", 3),
        ]);
        assert!(state.is_some());
        if let Some(state) = state {
            assert_eq!(state.objective, "prepare a briefing");
            assert_eq!(state.additions, vec!["include sources"]);
            assert_eq!(state.control, GoalControlState::Paused);
        }
    }
}
