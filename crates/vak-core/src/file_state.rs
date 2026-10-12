//! The state of one workspace path, read from the artifact record (doc 90
//! §3.1). A pure function of the record and one observation of the
//! workspace file, so nothing here reads a folder or a store.

use crate::artifacts::{Artifact, Version, VersionSource};
use chrono::{DateTime, Utc};
use std::cmp::Reverse;
use vak_session::ids::VersionId;

/// What the workspace holds at the path, observed when the state is made
/// (doc 90 §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Workspace {
    Absent,
    /// `digest` is lowercase hex SHA-256 of the file's bytes, the form
    /// `Version::digest` stores, and is compared with it exactly.
    Present {
        digest: String,
    },
}

/// How the workspace file relates to the record (doc 90 §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceIs {
    /// No artifact is recorded for this path.
    Unrecorded,
    /// An artifact exists, but the workspace file is absent and no version
    /// was ever accepted into the workspace. "Never accepted into the
    /// workspace": the record and the folder have not met yet.
    NotYet,
    /// The workspace file's bytes are those of this recorded version.
    Version(VersionId),
    /// The file differs from every version: edited outside Vak since
    /// `since`, the latest accepted version (or the latest version when
    /// none was accepted).
    Changed { since: VersionId },
    /// An accepted version's file is no longer in the workspace.
    Missing { accepted: VersionId },
}

/// The state of one version that is not in the workspace (doc 90 §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftState {
    /// Proposed for Review and not accepted.
    Waiting,
    /// Accepted once, then put back by an undo.
    Undone,
    /// Kept by a person, not in the workspace.
    Saved,
    /// In the trash since `since` (the record holds when it went there, not
    /// when it ends; the retention label decides that). Restorable until
    /// erased.
    Trashed { since: DateTime<Utc> },
    /// Its bytes were erased.
    Erased,
}

/// A version that is not the workspace file and has no newer version made
/// from it (doc 90 §3.1, D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub version: VersionId,
    /// `.vak/scratch/<agent>/<execution>/<path>` when a tool call made the
    /// version and its bytes are kept; `None` for a Review candidate, a
    /// person's edit, or an erased version.
    pub draft_name: Option<String>,
    /// The turn the version was made in.
    pub turn: Option<String>,
    /// The version it was made from.
    pub parent: Option<VersionId>,
    pub state: DraftState,
    /// The draft deletes the file: accepting it removes the workspace file.
    pub removes: bool,
}

/// The state of one workspace path (doc 90 §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub path: String,
    pub workspace: Workspace,
    pub workspace_is: WorkspaceIs,
    /// Newest first.
    pub drafts: Vec<Draft>,
    /// The Agent that authored the artifact; `None` with no record.
    pub owner: Option<String>,
}

/// The state of `path` given its artifact (if recorded) and what the
/// workspace holds at it. Stage 1 of doc 90 §5.
pub fn file_state(artifact: Option<&Artifact>, path: &str, workspace: Workspace) -> FileState {
    let Some(artifact) = artifact else {
        return FileState {
            path: path.to_string(),
            workspace,
            workspace_is: WorkspaceIs::Unrecorded,
            drafts: Vec::new(),
            owner: None,
        };
    };
    let workspace_is = workspace_is(artifact, &workspace);
    let current = match workspace_is {
        WorkspaceIs::Version(id) => Some(id),
        _ => None,
    };

    // A version is a draft when no live version was made from it. Erased
    // and trashed children do not supersede it, as in `Artifact::heads`.
    // Accepted versions are not drafts: the workspace file's history is
    // `workspace_is`.
    let mut candidates: Vec<&Version> = artifact
        .versions
        .iter()
        .filter(|version| {
            // A proposed deletion is a draft like any other until accepted.
            (!version.removed || version.proposal().is_some())
                && !version.promoted
                && Some(version.id) != current
                && !artifact
                    .versions
                    .iter()
                    .any(|child| child.is_present() && child.parent == Some(version.id))
        })
        .collect();
    candidates.sort_by_key(|version| Reverse(version.at));

    let drafts = candidates
        .into_iter()
        .map(|version| Draft {
            version: version.id,
            draft_name: draft_name(artifact, version),
            turn: version.turn.clone(),
            parent: version.parent,
            state: draft_state(version),
            removes: version.removed,
        })
        .collect();

    FileState {
        path: path.to_string(),
        workspace,
        workspace_is,
        drafts,
        owner: Some(artifact.agent.clone()),
    }
}

fn workspace_is(artifact: &Artifact, workspace: &Workspace) -> WorkspaceIs {
    if artifact.versions.is_empty() {
        return WorkspaceIs::NotYet;
    }
    let accepted = artifact
        .versions
        .iter()
        .filter(|version| version.promoted)
        .max_by_key(|version| version.at);
    match workspace {
        Workspace::Present { digest } => {
            let matching = artifact
                .versions
                .iter()
                .filter(|version| !version.removed && !version.digest.is_empty())
                .filter(|version| version.digest == *digest)
                .max_by_key(|version| (version.promoted, version.at));
            if let Some(version) = matching {
                return WorkspaceIs::Version(version.id);
            }
            let since =
                accepted.or_else(|| artifact.versions.iter().max_by_key(|version| version.at));
            since.map_or(WorkspaceIs::NotYet, |version| WorkspaceIs::Changed {
                since: version.id,
            })
        }
        Workspace::Absent => match accepted {
            // An accepted deletion: the file is absent as the version says.
            Some(version) if version.removed => WorkspaceIs::Version(version.id),
            Some(version) => WorkspaceIs::Missing {
                accepted: version.id,
            },
            None => WorkspaceIs::NotYet,
        },
    }
}

fn draft_state(version: &Version) -> DraftState {
    if version.erased {
        DraftState::Erased
    } else if let Some(since) = version.trashed_at {
        DraftState::Trashed { since }
    } else if version.undone {
        DraftState::Undone
    } else if version.saved {
        DraftState::Saved
    } else {
        DraftState::Waiting
    }
}

fn draft_name(artifact: &Artifact, version: &Version) -> Option<String> {
    match &version.source {
        VersionSource::Call { call, .. } if !version.erased => Some(format!(
            "{}/{}/{}/{}",
            vak_config::scope::SCRATCH_DIR,
            artifact.agent,
            call,
            artifact.path
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifacts::{ArtifactEvent, ArtifactKind, ArtifactStep, artifact_id, replay};
    use sha2::Digest;
    use vak_session::ids::{ArtifactId, TurnId};

    const PATH: &str = "porto-notes.docx";

    fn digest(bytes: &[u8]) -> String {
        hex::encode(sha2::Sha256::digest(bytes))
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).expect("a valid timestamp")
    }

    struct Record {
        artifact: ArtifactId,
        events: Vec<ArtifactEvent>,
    }

    impl Record {
        fn new(agent: &str) -> Self {
            let artifact = artifact_id("space", PATH);
            let events = vec![ArtifactEvent {
                artifact,
                at: at(1),
                trace: None,
                actor: None,
                step: ArtifactStep::Declared {
                    space: "space".into(),
                    agent: agent.into(),
                    path: PATH.into(),
                    kind: ArtifactKind::Document,
                    title: None,
                    summary: None,
                },
            }];
            Self { artifact, events }
        }

        fn push(&mut self, secs: i64, turn: Option<TurnId>, step: ArtifactStep) {
            let trace = turn.map(|turn| {
                let mut key = vak_session::trace::TraceKey::root(
                    vak_session::ids::TenantId::new(),
                    vak_session::ids::SpaceId::new(),
                    vak_session::ids::AgentId::new(),
                    vak_session::trace::Cause::Heartbeat,
                );
                key.turn = Some(turn);
                key
            });
            self.events.push(ArtifactEvent {
                artifact: self.artifact,
                at: at(secs),
                trace,
                actor: None,
                step,
            });
        }

        /// Adds a version of `bytes` made from `parent` by `source`.
        fn version(
            &mut self,
            secs: i64,
            turn: Option<TurnId>,
            parent: Option<VersionId>,
            bytes: &[u8],
            source: VersionSource,
        ) -> VersionId {
            let version = VersionId::new();
            self.push(
                secs,
                turn,
                ArtifactStep::Versioned {
                    version,
                    parent,
                    object: None,
                    digest: digest(bytes),
                    size: bytes.len() as u64,
                    source,
                },
            );
            version
        }

        fn call(call: &str) -> VersionSource {
            VersionSource::Call {
                session: "ses".into(),
                call: call.into(),
            }
        }

        fn artifact(&self) -> Option<Artifact> {
            replay(&self.events).into_iter().next()
        }
    }

    fn present(bytes: &[u8]) -> Workspace {
        Workspace::Present {
            digest: digest(bytes),
        }
    }

    #[test]
    fn no_record_is_unrecorded() {
        let found = file_state(None, PATH, Workspace::Absent);
        assert_eq!(found.workspace_is, WorkspaceIs::Unrecorded);
        assert!(found.drafts.is_empty());
        assert_eq!(found.owner, None);
    }

    #[test]
    fn absent_file_with_waiting_drafts_is_not_yet_accepted() {
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", Record::call("call_a"));
        let turn = TurnId::new();
        let second = record.version(3, Some(turn), Some(first), b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        assert_eq!(found.workspace_is, WorkspaceIs::NotYet);
        assert_eq!(found.drafts.len(), 1, "the parent is superseded");
        let draft = &found.drafts[0];
        assert_eq!(draft.version, second);
        assert_eq!(draft.state, DraftState::Waiting);
        assert_eq!(draft.parent, Some(first));
        assert_eq!(draft.turn, Some(turn.to_string()));
        assert_eq!(
            draft.draft_name.as_deref(),
            Some(".vak/scratch/vak/call_b/porto-notes.docx")
        );
    }

    #[test]
    fn present_file_with_newer_draft_waiting() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let draft = record.version(4, None, Some(accepted), b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        assert_eq!(found.workspace_is, WorkspaceIs::Version(accepted));
        assert_eq!(found.drafts.len(), 1);
        assert_eq!(found.drafts[0].version, draft);
        assert_eq!(found.drafts[0].state, DraftState::Waiting);
    }

    #[test]
    fn file_changed_outside_vak_since_the_accepted_version() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let draft = record.version(4, None, Some(accepted), b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, present(b"edited by hand"));
        assert_eq!(found.workspace_is, WorkspaceIs::Changed { since: accepted });
        assert_eq!(found.drafts.len(), 1);
        assert_eq!(found.drafts[0].version, draft);
    }

    #[test]
    fn accepted_file_missing_from_the_workspace() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        assert_eq!(found.workspace_is, WorkspaceIs::Missing { accepted });
        assert!(found.drafts.is_empty());
    }

    #[test]
    fn undone_draft_is_listed_as_undone() {
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: first });
        let second = record.version(4, None, Some(first), b"two", Record::call("call_b"));
        record.push(5, None, ArtifactStep::Promoted { version: second });
        record.push(6, None, ArtifactStep::PromotionUndone { version: second });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        assert_eq!(found.workspace_is, WorkspaceIs::Version(first));
        assert_eq!(found.drafts.len(), 1);
        assert_eq!(found.drafts[0].version, second);
        assert_eq!(found.drafts[0].state, DraftState::Undone);
    }

    #[test]
    fn a_deletion_waiting_for_review_is_a_draft_that_removes_the_file() {
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: first });
        let removal = VersionId::new();
        record.push(
            4,
            None,
            ArtifactStep::Removed {
                version: removal,
                parent: Some(first),
                source: VersionSource::Candidate {
                    session: "ses".into(),
                    candidate: "cand".into(),
                },
            },
        );
        record.push(
            5,
            None,
            ArtifactStep::Proposed {
                version: removal,
                session: "ses".into(),
                candidate: "cand".into(),
            },
        );
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        assert_eq!(found.workspace_is, WorkspaceIs::Version(first));
        assert_eq!(found.drafts.len(), 1);
        assert_eq!(found.drafts[0].version, removal);
        assert!(found.drafts[0].removes);
        assert_eq!(found.drafts[0].state, DraftState::Waiting);
    }

    #[test]
    fn erased_draft_has_no_name() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let draft = record.version(4, None, Some(accepted), b"two", Record::call("call_b"));
        record.push(5, None, ArtifactStep::Erased { version: draft });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        assert_eq!(found.drafts.len(), 1);
        assert_eq!(found.drafts[0].state, DraftState::Erased);
        assert_eq!(found.drafts[0].draft_name, None);
    }

    #[test]
    fn trashed_draft_keeps_when_it_went_there() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let draft = record.version(4, None, Some(accepted), b"two", Record::call("call_b"));
        record.push(
            5,
            None,
            ArtifactStep::Trashed {
                version: draft,
                on: true,
            },
        );
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        assert_eq!(found.drafts[0].state, DraftState::Trashed { since: at(5) });
    }

    #[test]
    fn owner_is_the_artifacts_agent() {
        let mut record = Record::new("ops");
        record.version(2, None, None, b"one", Record::call("call_a"));
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        assert_eq!(found.owner.as_deref(), Some("ops"));
    }

    #[test]
    fn workspace_file_equal_to_the_only_version_has_no_drafts() {
        let mut record = Record::new("vak");
        let only = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: only });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        assert_eq!(found.workspace_is, WorkspaceIs::Version(only));
        assert!(found.drafts.is_empty());
    }

    #[test]
    fn sibling_drafts_are_listed_newest_first_with_their_turns() {
        let mut record = Record::new("vak");
        let base = record.version(2, None, None, b"base", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: base });
        let (older_turn, newer_turn) = (TurnId::new(), TurnId::new());
        let older = record.version(
            4,
            Some(older_turn),
            Some(base),
            b"x",
            Record::call("call_b"),
        );
        let newer = record.version(
            5,
            Some(newer_turn),
            Some(base),
            b"y",
            Record::call("call_c"),
        );
        let found = file_state(record.artifact().as_ref(), PATH, present(b"base"));
        let ids: Vec<VersionId> = found.drafts.iter().map(|draft| draft.version).collect();
        assert_eq!(ids, vec![newer, older]);
        assert_eq!(found.drafts[0].turn, Some(newer_turn.to_string()));
        assert_eq!(found.drafts[1].turn, Some(older_turn.to_string()));
    }

    #[test]
    fn superseded_version_is_not_listed() {
        let mut record = Record::new("vak");
        let parent = record.version(2, None, None, b"one", Record::call("call_a"));
        let child = record.version(3, None, Some(parent), b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        let ids: Vec<VersionId> = found.drafts.iter().map(|draft| draft.version).collect();
        assert_eq!(ids, vec![child]);
    }
}
