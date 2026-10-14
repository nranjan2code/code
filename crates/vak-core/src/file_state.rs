//! The state of one workspace path, read from the artifact record (doc 90
//! §3.1). A pure function of the record and one observation of the
//! workspace file, so nothing here reads a folder or a store.

use crate::artifacts::{Artifact, Artifacts, Version, VersionSource, artifact_id, normalize};
use chrono::{DateTime, Utc};
use sha2::Digest;
use std::cmp::Reverse;
use std::path::Path;
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
    /// An artifact exists, but the record and the folder have not met yet:
    /// no version was ever accepted into the workspace or found there, and
    /// the file is absent or holds bytes no version has (a file that was
    /// there before Vak recorded anything of it).
    NotYet,
    /// The workspace file's bytes are those of this recorded version.
    Version(VersionId),
    /// The file differs from every version: edited outside Vak since
    /// `since`, the version the workspace last held.
    Changed { since: VersionId },
    /// The version the workspace last held (accepted, or found there) is
    /// not there now.
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
    /// The conversation the version was made in, from its source.
    pub session: Option<String>,
    /// The version it was made from.
    pub parent: Option<VersionId>,
    pub state: DraftState,
    /// What made the version; an unnamed draft is described by it, never
    /// by an author Vak did not record.
    pub source: VersionSource,
    /// The draft deletes the file: accepting it removes the workspace file.
    pub removes: bool,
    /// The workspace's current version is not among the draft's ancestors:
    /// the draft was made before it and does not include it (doc 90 §3.2).
    pub behind: bool,
    /// The few words its call named it with.
    pub title: Option<String>,
    /// What its turn read before the call that made it (doc 90 §5.1).
    pub read_first: Vec<vak_tools::ReadInput>,
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
    /// The workspace file is an edit Vak found made outside it (D2).
    pub changed_outside: bool,
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
            changed_outside: false,
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
            // A copy the workspace held (found there, or written there by a
            // call) was never proposed: once the workspace moves on it is
            // history, not a draft.
            !version.held_by_workspace()
                && (!version.removed || version.proposal().is_some())
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
            behind: current.is_some_and(|current| !ancestors(artifact, version).contains(&current)),
            version: version.id,
            draft_name: draft_name(artifact, version),
            turn: version.turn.clone(),
            session: match &version.source {
                VersionSource::Call { session, .. } | VersionSource::Candidate { session, .. } => {
                    Some(session.clone())
                }
                VersionSource::Person | VersionSource::Workspace => None,
            },
            parent: version.parent,
            state: draft_state(version),
            source: version.source.clone(),
            removes: version.removed,
            title: version.title.clone(),
            read_first: version.derived_from.clone(),
        })
        .collect();

    FileState {
        path: path.to_string(),
        workspace,
        workspace_is,
        drafts,
        owner: Some(artifact.agent.clone()),
        changed_outside: current.is_some_and(|current| {
            artifact.versions.iter().any(|v| {
                v.id == current
                    && v.parent.is_some()
                    && matches!(v.source, VersionSource::Workspace)
            })
        }),
    }
}

fn ancestors(artifact: &Artifact, version: &Version) -> Vec<VersionId> {
    let mut seen = Vec::new();
    let mut next = version.parent;
    while let Some(id) = next.filter(|id| !seen.contains(id)) {
        seen.push(id);
        next = artifact
            .versions
            .iter()
            .find(|v| v.id == id)
            .and_then(|v| v.parent);
    }
    seen
}

/// Whether `version` was made before the workspace's current copy and does
/// not include it (`Draft::behind`), for any version, a draft or not: what
/// Review asks before a version replaces the workspace file (doc 90 §3.4).
/// `state` is a fresh observation of the version's own path.
pub fn behind(artifact: &Artifact, state: &FileState, version: VersionId) -> bool {
    let WorkspaceIs::Version(current) = state.workspace_is else {
        return false;
    };
    current != version
        && artifact
            .versions
            .iter()
            .find(|known| known.id == version)
            .is_some_and(|known| !ancestors(artifact, known).contains(&current))
}

/// What one version is now, as the person's view shows it (doc 90 §5): a
/// word and a mark for each, read from the record and one observation of
/// the workspace file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum VersionIs {
    /// The workspace file holds these bytes now.
    InFolder,
    /// Accepted into the workspace once; the folder has moved on since.
    Accepted,
    /// Written into the workspace by a call; the folder has moved on since.
    Written,
    /// An edit made outside Vak that Vak found; the folder has moved on.
    FoundOutside,
    /// Proposed and not accepted, with no newer version made from it.
    /// `behind`: made before the workspace's current copy, which it does
    /// not include.
    Waiting {
        behind: bool,
    },
    /// A newer version was made from it before anyone accepted it.
    Revised,
    /// Accepted, then put back by an undo.
    Undone,
    /// Kept by the person, not in the workspace.
    Saved,
    InTrash {
        since: DateTime<Utc>,
    },
    Erased,
}

/// Each version of `artifact` with what it is now, in record order.
pub fn version_states(artifact: &Artifact, state: &FileState) -> Vec<(VersionId, VersionIs)> {
    let current = match state.workspace_is {
        WorkspaceIs::Version(id) => Some(id),
        _ => None,
    };
    artifact
        .versions
        .iter()
        .map(|version| {
            let has_child = artifact
                .versions
                .iter()
                .any(|child| child.is_present() && child.parent == Some(version.id));
            let is = if version.erased {
                VersionIs::Erased
            } else if let Some(since) = version.trashed_at {
                VersionIs::InTrash { since }
            } else if Some(version.id) == current {
                VersionIs::InFolder
            } else if version.undone {
                VersionIs::Undone
            } else if version.promoted {
                VersionIs::Accepted
            } else if matches!(version.source, VersionSource::Workspace) {
                VersionIs::FoundOutside
            } else if version.held_by_workspace() {
                VersionIs::Written
            } else if version.saved {
                VersionIs::Saved
            } else if has_child {
                VersionIs::Revised
            } else {
                VersionIs::Waiting {
                    behind: behind(artifact, state, version.id),
                }
            };
            (version.id, is)
        })
        .collect()
}

/// What about one file waits on the person (doc 90 §5): the strip and the
/// Library's filters read this.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Needs {
    /// Drafts waiting for review.
    pub waiting: usize,
    /// Of those, made before the workspace's current copy.
    pub behind: usize,
    /// The workspace file is an edit made outside Vak, or differs from
    /// every version.
    pub changed_outside: bool,
    /// The version the workspace last held is not there now.
    pub missing: bool,
}

impl Needs {
    pub fn any(&self) -> bool {
        self.waiting > 0 || self.missing
    }
}

/// What `artifact` needs of the person, from its version states.
pub fn needs(state: &FileState, states: &[(VersionId, VersionIs)]) -> Needs {
    let waiting = states
        .iter()
        .filter(|(_, is)| matches!(is, VersionIs::Waiting { .. }))
        .count();
    let behind = states
        .iter()
        .filter(|(_, is)| matches!(is, VersionIs::Waiting { behind: true }))
        .count();
    Needs {
        waiting,
        behind,
        changed_outside: state.changed_outside
            || matches!(state.workspace_is, WorkspaceIs::Changed { .. }),
        missing: matches!(state.workspace_is, WorkspaceIs::Missing { .. }),
    }
}

/// The newest version the workspace held as far as the record knows: one
/// accepted into it, found in it, or written to it. A call that writes the
/// workspace file makes its version a child of this one.
pub fn last_held(artifact: &Artifact) -> Option<VersionId> {
    artifact
        .versions
        .iter()
        .filter(|version| version.promoted || version.held_by_workspace())
        .max_by_key(|version| version.at)
        .map(|version| version.id)
}

fn workspace_is(artifact: &Artifact, workspace: &Workspace) -> WorkspaceIs {
    if artifact.versions.is_empty() {
        return WorkspaceIs::NotYet;
    }
    // What the workspace last held, as far as the record knows: a version
    // accepted into it, or an edit found in it.
    let accepted = artifact
        .versions
        .iter()
        .filter(|version| version.promoted || version.held_by_workspace())
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
            accepted.map_or(WorkspaceIs::NotYet, |version| WorkspaceIs::Changed {
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

/// A workspace path as the record names it: relative to `cwd`, normalised
/// by `artifacts::normalize`, so a draft's name and the file it becomes are
/// one artifact.
pub fn workspace_path(cwd: &Path, path: &str) -> String {
    let named = Path::new(path.trim());
    let relative = if named.is_absolute() {
        match named.strip_prefix(cwd) {
            Ok(rest) => rest.to_string_lossy().into_owned(),
            Err(_) => cwd
                .canonicalize()
                .ok()
                .and_then(|root| {
                    vak_tools::context::resolved(named)
                        .strip_prefix(&root)
                        .ok()
                        .map(Path::to_path_buf)
                })
                .map_or_else(
                    || path.to_string(),
                    |rest| rest.to_string_lossy().into_owned(),
                ),
        }
    } else {
        path.to_string()
    };
    normalize(&relative)
}

/// The state of `path` in `space` as the Core sees it now: the record, and
/// the digest of the workspace file's bytes (doc 90 §3.1). Reads the file;
/// hashing is not parsing. `None` when the file cannot be read.
/// A found version is recorded in one [`Artifacts::transaction`]; when it
/// cannot be recorded, the state is as read.
pub fn observe(artifacts: &Artifacts, space: &str, cwd: &Path, path: &str) -> Option<FileState> {
    match artifacts.transaction(|tx| Ok(observe_in(tx, space, cwd, path, true))) {
        Ok(state) => state,
        Err(_) => artifacts
            .transaction(|tx| Ok(observe_in(tx, space, cwd, path, false)))
            .ok()
            .flatten(),
    }
}

/// [`observe`] inside a caller's transaction: what the folder holds is
/// compared with the record and a found version pushed in the same
/// decision, so a call's own write recorded at the same moment is never
/// also found as an edit made outside Vak (found by a test, 2026-10-13: a
/// Library read beside a write recorded the call's bytes twice). With
/// `record` false nothing is pushed.
pub(crate) fn observe_in(
    tx: &mut crate::artifacts::Tx<'_>,
    space: &str,
    cwd: &Path,
    path: &str,
    record: bool,
) -> Option<FileState> {
    let normal = workspace_path(cwd, path);
    // A path Vak never recorded has nothing to say beyond the folder: the
    // file is not read, so a model reading large files pays nothing.
    let id = artifact_id(space, &normal);
    let artifact = tx.get(&id.to_string())?;
    let bytes = match std::fs::read(cwd.join(&normal)) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return None,
    };
    let workspace = |bytes: &Option<Vec<u8>>| match bytes {
        Some(bytes) => Workspace::Present {
            digest: hex::encode(sha2::Sha256::digest(bytes)),
        },
        None => Workspace::Absent,
    };
    let state = file_state(Some(&artifact), &normal, workspace(&bytes));
    // Bytes no version holds are an edit made outside Vak: recorded as a
    // version of what they changed (D2), so the history shows it and a
    // draft made before it says so. A failed write leaves the state as read.
    // A file that was there before the record knew it is its first
    // workspace version, with no parent: a draft made from it has a base.
    let parent = match (&state.workspace_is, &state.workspace) {
        (WorkspaceIs::Changed { since }, _) => Some(Some(*since)),
        (WorkspaceIs::NotYet, Workspace::Present { .. }) => Some(None),
        _ => None,
    };
    if record && let (Some(parent), Some(found)) = (parent, &bytes) {
        let recorded = tx.found_in_workspace(id, parent, found);
        if recorded.is_ok()
            && let Some(artifact) = tx.get(&id.to_string())
        {
            return Some(file_state(Some(&artifact), &normal, workspace(&bytes)));
        }
    }
    Some(state)
}

/// What a search hit on the artifact `id` says of its file (doc 90 §3.3):
/// its workspace path, then the note about its state, or that it is in the
/// workspace as recorded.
pub fn describe(artifacts: &Artifacts, cwd: &Path, id: &str, reader: Reader<'_>) -> Option<String> {
    let artifact = artifacts.get(id)?;
    let state = observe(artifacts, &artifact.space, cwd, &artifact.path)?;
    Some(match note(&state, reader) {
        Some(note) => format!("file `{}`\n{note}", artifact.path),
        None => format!("file `{}`, in the workspace as recorded", artifact.path),
    })
}

/// The recorded files a listing by `pattern` cannot see, because they are
/// not in the workspace yet (doc 90 §3.3): each with its newest draft.
/// `None` when every match is in the folder.
pub fn matching_note(
    artifacts: &Artifacts,
    space: &str,
    cwd: &Path,
    pattern: &str,
    session: Option<&str>,
) -> Option<String> {
    let matcher = globset::Glob::new(pattern).ok()?.compile_matcher();
    let lines: Vec<String> = artifacts
        .list()
        .into_iter()
        .filter(|artifact| artifact.space == space && matcher.is_match(&artifact.path))
        .filter(|artifact| !cwd.join(&artifact.path).exists())
        .filter_map(|artifact| {
            let state = file_state(Some(&artifact), &artifact.path, Workspace::Absent);
            let newest = state.drafts.first()?;
            Some(format!(
                "- `{}`: {}",
                artifact.path,
                draft_line(newest, session)
            ))
        })
        .collect();
    (!lines.is_empty()).then(|| {
        format!(
            "[file] Matching `{pattern}` and not in the workspace yet:\n{}",
            lines.join("\n")
        )
    })
}

/// Who the note is for: the Agent reading and the conversation it reads
/// in, so a version's origin is named from where the reader stands
/// (doc 90 §5.1).
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    pub agent: &'a str,
    pub session: Option<&'a str>,
}

/// Where a version was made, by the ids `recall` takes: `turn_id` for a turn
/// of this conversation, with `conversation` for another one (doc 90 §5.1).
fn origin(draft: &Draft, session: Option<&str>) -> Option<String> {
    let here = draft.session.is_some() && draft.session.as_deref() == session;
    match (&draft.turn, &draft.session) {
        (Some(turn), _) if here => Some(format!("made in turn {turn} of this conversation")),
        (Some(turn), Some(other)) => Some(format!("made in turn {turn} of conversation {other}")),
        (Some(turn), None) => Some(format!("made in turn {turn}")),
        (None, Some(_)) if here => Some("made in this conversation".to_string()),
        (None, Some(other)) => Some(format!("made in conversation {other}")),
        (None, None) => None,
    }
}

/// The note the loop attaches to a call that touched `state.path` (doc 90
/// §3.2), or `None` when the folder alone already says everything. Facts
/// only: it never chooses between a draft and the workspace copy (D1).
pub fn note(state: &FileState, reader: Reader<'_>) -> Option<String> {
    let other_owner = state
        .owner
        .as_deref()
        .is_some_and(|owner| owner != reader.agent);
    let status = match (&state.workspace_is, &state.workspace) {
        (WorkspaceIs::Changed { since }, _) => Some(format!(
            "`{}` changed in the workspace outside Vak since version {since}.",
            state.path
        )),
        (WorkspaceIs::Missing { accepted }, _) => Some(format!(
            "`{}` was in the workspace as version {accepted} and is not there now.",
            state.path
        )),
        (WorkspaceIs::NotYet, Workspace::Absent) if !state.drafts.is_empty() => Some(format!(
            "`{}` exists only as a draft; it is not in the workspace yet.",
            state.path
        )),
        (WorkspaceIs::NotYet, Workspace::Absent) => {
            Some(format!("`{}` is not in the workspace.", state.path))
        }
        (WorkspaceIs::Version(id), _) if state.changed_outside => Some(format!(
            "`{}` in the workspace is an edit made outside Vak (version {id}).",
            state.path
        )),
        (WorkspaceIs::Version(id), _) if !state.drafts.is_empty() => {
            Some(format!("`{}` matches version {id}.", state.path))
        }
        _ => None,
    };
    if status.is_none() && state.drafts.is_empty() && !other_owner {
        return None;
    }
    let mut lines: Vec<String> = status.into_iter().collect();
    lines.extend(
        state
            .drafts
            .iter()
            .map(|draft| format!("- {}", draft_line(draft, reader.session))),
    );
    if let Some(newest) = state
        .drafts
        .iter()
        .find_map(|draft| draft.draft_name.as_deref())
    {
        lines.push(format!(
            "To read it, read `{newest}`. To change it, pass a draft as `source`."
        ));
    }
    if other_owner {
        lines.push(format!(
            "`{}` belongs to another Agent; this Agent cannot continue it.",
            state.path
        ));
    }
    Some(format!("[file] {}", lines.join("\n")))
}

fn draft_line(draft: &Draft, session: Option<&str>) -> String {
    let mut parts = Vec::new();
    let name = match (&draft.draft_name, &draft.state) {
        (Some(name), _) => {
            parts.push(state_words(&draft.state));
            format!("`{name}`")
        }
        (None, DraftState::Erased) => "erased, contents gone".to_string(),
        (None, _) => {
            parts.push(state_words(&draft.state));
            match draft.source {
                VersionSource::Person => "no name: made by the person",
                VersionSource::Candidate { .. } => "no name: a Review candidate",
                VersionSource::Call { .. } | VersionSource::Workspace => "no name",
            }
            .to_string()
        }
    };
    if let Some(title) = &draft.title {
        parts.push(format!("named \"{title}\""));
    }
    if draft.removes {
        parts.push("deletes the file".to_string());
    }
    if draft.behind {
        parts.push(
            "made before the workspace's current copy, which it does not include".to_string(),
        );
    }
    if let Some(origin) = origin(draft, session) {
        parts.push(origin);
    }
    if !draft.read_first.is_empty() {
        let read: Vec<String> = draft
            .read_first
            .iter()
            .map(|input| match input {
                vak_tools::ReadInput::File { path, .. } => format!("`{path}`"),
                vak_tools::ReadInput::Page { url } => url.clone(),
            })
            .collect();
        parts.push(format!("after reading {}", read.join(", ")));
    }
    if let Some(parent) = draft.parent {
        parts.push(format!("made from version {parent}"));
    }
    if parts.is_empty() {
        name
    } else {
        format!("{name}: {}", parts.join(", "))
    }
}

fn state_words(state: &DraftState) -> String {
    match state {
        DraftState::Waiting => "waiting for review".to_string(),
        DraftState::Undone => "undone by the person".to_string(),
        DraftState::Saved => "saved by the person".to_string(),
        DraftState::Trashed { since } => {
            format!("in the trash since {}", since.format("%Y-%m-%d"))
        }
        DraftState::Erased => "erased".to_string(),
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

        /// The conversation `Record::call` versions are made in.
        fn reader(&self) -> String {
            "ses".to_string()
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
                    title: None,
                    derived_from: Vec::new(),
                },
            );
            version
        }

        fn call(call: &str) -> VersionSource {
            VersionSource::Call {
                session: "ses".into(),
                call: call.into(),
                in_workspace: false,
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

    /// Found live 2026-10-13: each `write` straight to the workspace was
    /// listed as a draft "waiting for review, made before the workspace's
    /// current copy". A call's own workspace write is what the workspace
    /// held, and the next one is its child.
    #[test]
    fn a_call_that_wrote_the_workspace_left_no_draft() {
        let written = |call: &str| VersionSource::Call {
            session: "ses".into(),
            call: call.into(),
            in_workspace: true,
        };
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", written("call_a"));
        let artifact = record.artifact().unwrap();
        assert_eq!(last_held(&artifact), Some(first));
        let second = record.version(3, None, Some(first), b"two", written("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, present(b"two"));
        assert!(found.drafts.is_empty(), "{:?}", found.drafts);
        assert_eq!(found.workspace_is, WorkspaceIs::Version(second));
        let gone = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        assert_eq!(gone.workspace_is, WorkspaceIs::Missing { accepted: second });
    }

    /// Found live 2026-10-13: after a file was deleted, the copy Vak had
    /// found in the workspace was listed as "made by a person: waiting for
    /// review". Nothing proposed it and nothing recorded who made it.
    #[test]
    fn a_copy_found_in_the_workspace_is_never_a_draft() {
        let mut record = Record::new("vak");
        record.version(2, None, None, b"one", VersionSource::Workspace);
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        assert!(found.drafts.is_empty(), "{:?}", found.drafts);
        assert!(
            !note(
                &found,
                Reader {
                    agent: "vak",
                    session: None
                }
            )
            .unwrap_or_default()
            .contains("made by"),
            "{found:?}"
        );
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
        assert_eq!(draft.turn, Some(turn.uuid().to_string()));
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
        assert_eq!(found.drafts[0].turn, Some(newer_turn.uuid().to_string()));
        assert_eq!(found.drafts[1].turn, Some(older_turn.uuid().to_string()));
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

    #[test]
    fn nothing_is_noted_for_an_unrecorded_path_or_the_workspace_version() {
        let found = file_state(None, PATH, Workspace::Absent);
        assert_eq!(
            note(
                &found,
                Reader {
                    agent: "vak",
                    session: None
                }
            ),
            None
        );
        let mut record = Record::new("vak");
        let only = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: only });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        assert_eq!(
            note(
                &found,
                Reader {
                    agent: "vak",
                    session: None
                }
            ),
            None
        );
    }

    /// A draft's name and what its turn read first are in the note
    /// (doc 90 §5.1), from the record alone.
    #[test]
    fn a_drafts_name_and_what_its_turn_read_are_told() {
        let mut record = Record::new("vak");
        let turn = TurnId::new();
        record.push(
            2,
            Some(turn),
            ArtifactStep::Versioned {
                version: VersionId::new(),
                parent: None,
                object: None,
                digest: digest(b"one"),
                size: 3,
                source: Record::call("call_a"),
                title: Some("Shorter version".into()),
                derived_from: vec![
                    vak_tools::ReadInput::File {
                        path: "budget.xlsx".into(),
                        digest: None,
                    },
                    vak_tools::ReadInput::Page {
                        url: "https://example.com/rates".into(),
                    },
                ],
            },
        );
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        let here = record.reader();
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: Some(&here),
            },
        )
        .expect("a note");
        assert!(text.contains("named \"Shorter version\""), "{text}");
        assert!(
            text.contains("after reading `budget.xlsx`, https://example.com/rates"),
            "{text}"
        );
    }

    #[test]
    fn absent_file_with_two_waiting_drafts_lists_both_newest_first() {
        let mut record = Record::new("vak");
        let older = record.version(2, Some(TurnId::new()), None, b"one", Record::call("call_a"));
        let newer_turn = TurnId::new();
        let newer = record.version(3, Some(newer_turn), None, b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        assert_eq!(found.drafts[0].version, newer);
        assert_eq!(found.drafts[1].version, older);
        let here = record.reader();
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: Some(&here),
            },
        )
        .expect("a note");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert_eq!(
            lines[0],
            "[file] `porto-notes.docx` exists only as a draft; it is not in the workspace yet."
        );
        assert_eq!(
            lines[1],
            format!(
                "- `.vak/scratch/vak/call_b/porto-notes.docx`: waiting for review, made in turn {} of this conversation",
                newer_turn.uuid()
            )
        );
        assert!(lines[2].starts_with(
            "- `.vak/scratch/vak/call_a/porto-notes.docx`: waiting for review, made in turn "
        ));
        assert_eq!(
            lines[3],
            "To read it, read `.vak/scratch/vak/call_b/porto-notes.docx`. To change it, pass a draft as `source`."
        );
        assert!(!text.contains("workspace copy"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn a_new_file_named_through_a_link_is_the_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let named = real.canonicalize().unwrap().join("notes/trip.docx");
        assert_eq!(
            workspace_path(&link, &named.to_string_lossy()),
            "notes/trip.docx"
        );
        let through_link = link.join("trip.docx");
        assert_eq!(
            workspace_path(&real, &through_link.to_string_lossy()),
            "trip.docx"
        );
        let outside = dir.path().join("elsewhere/trip.docx");
        assert_eq!(
            workspace_path(&real, &outside.to_string_lossy()),
            outside.to_string_lossy()
        );
    }

    #[test]
    fn a_copy_the_workspace_held_is_never_a_draft_to_trash() {
        let mut record = Record::new("vak");
        let written = record.version(
            2,
            None,
            None,
            b"one",
            VersionSource::Call {
                session: "ses".into(),
                call: "call_a".into(),
                in_workspace: true,
            },
        );
        let found = record.version(3, None, Some(written), b"two", VersionSource::Workspace);
        let drafted = record.version(4, None, Some(found), b"three", Record::call("call_b"));
        let artifact = record.artifact().unwrap();
        let held = |id| {
            artifact
                .versions
                .iter()
                .find(|v| v.id == id)
                .unwrap()
                .held_by_workspace()
        };
        assert!(held(written) && held(found) && !held(drafted));
    }

    #[test]
    fn each_version_says_what_it_is_now() {
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: first });
        let revised = record.version(4, None, Some(first), b"two", Record::call("call_b"));
        let waiting = record.version(5, None, Some(revised), b"three", Record::call("call_c"));
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        let states = version_states(&record.artifact().unwrap(), &found);
        assert_eq!(
            states,
            vec![
                (first, VersionIs::InFolder),
                (revised, VersionIs::Revised),
                (waiting, VersionIs::Waiting { behind: false }),
            ]
        );
        let need = needs(&found, &states);
        assert_eq!(need.waiting, 1);
        assert!(need.any() && !need.changed_outside);

        let edited = file_state(record.artifact().as_ref(), PATH, present(b"mine"));
        let states = version_states(&record.artifact().unwrap(), &edited);
        assert_eq!(states[0].1, VersionIs::Accepted);
        assert!(needs(&edited, &states).changed_outside);
    }

    #[test]
    fn a_draft_names_its_conversation_turn_and_parent_by_the_ids_recall_takes() {
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", Record::call("call_a"));
        let turn = TurnId::new();
        record.version(3, Some(turn), Some(first), b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        let conversation = record.reader();
        let here = note(
            &found,
            Reader {
                agent: "vak",
                session: Some(&conversation),
            },
        )
        .expect("a note");
        assert!(
            here.contains(&format!(
                "made in turn {} of this conversation, made from version {first}",
                turn.uuid()
            )),
            "{here}"
        );
        let elsewhere = note(
            &found,
            Reader {
                agent: "vak",
                session: Some("another"),
            },
        )
        .expect("a note");
        assert!(
            elsewhere.contains(&format!(
                "made in turn {} of conversation {conversation}",
                turn.uuid()
            )),
            "{elsewhere}"
        );
    }

    #[test]
    fn present_file_with_a_newer_draft_names_the_version_it_matches() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let draft = record.version(4, None, Some(accepted), b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(
            text.starts_with(&format!(
                "[file] `porto-notes.docx` matches version {accepted}.\n"
            )),
            "{text}"
        );
        assert!(
            text.contains("- `.vak/scratch/vak/call_b/porto-notes.docx`: waiting for review"),
            "{text}"
        );
        assert_eq!(found.drafts[0].version, draft);
    }

    #[test]
    fn changed_outside_vak_is_stated_with_the_version_it_differs_from() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"edited"));
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert_eq!(
            text,
            format!(
                "[file] `porto-notes.docx` changed in the workspace outside Vak since version {accepted}."
            )
        );
    }

    #[test]
    fn accepted_then_missing_is_stated() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        assert_eq!(
            note(
                &found,
                Reader {
                    agent: "vak",
                    session: None
                }
            )
            .expect("a note"),
            format!(
                "[file] `porto-notes.docx` was in the workspace as version {accepted} and is not there now."
            )
        );
    }

    #[test]
    fn undone_saved_and_unnamed_drafts_say_what_happened_to_them() {
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: first });
        let second = record.version(4, None, Some(first), b"two", Record::call("call_b"));
        record.push(5, None, ArtifactStep::Promoted { version: second });
        record.push(6, None, ArtifactStep::PromotionUndone { version: second });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(
            text.contains("`.vak/scratch/vak/call_b/porto-notes.docx`: undone by the person"),
            "{text}"
        );

        let mut record = Record::new("vak");
        let base = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: base });
        let candidate = record.version(
            4,
            None,
            Some(base),
            b"two",
            VersionSource::Candidate {
                session: "ses".into(),
                candidate: "cand".into(),
            },
        );
        record.push(5, None, ArtifactStep::Saved { version: candidate });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(
            text.contains("- no name: a Review candidate: saved by the person"),
            "{text}"
        );
        assert!(!text.contains("pass that draft"), "{text}");
    }

    #[test]
    fn trashed_erased_and_deleting_drafts_are_stated() {
        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let trashed = record.version(4, None, Some(accepted), b"two", Record::call("call_b"));
        record.push(
            5,
            None,
            ArtifactStep::Trashed {
                version: trashed,
                on: true,
            },
        );
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(text.contains("in the trash since 1970-01-01"), "{text}");

        let mut record = Record::new("vak");
        let accepted = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: accepted });
        let draft = record.version(4, None, Some(accepted), b"two", Record::call("call_b"));
        record.push(5, None, ArtifactStep::Erased { version: draft });
        let found = file_state(record.artifact().as_ref(), PATH, present(b"one"));
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(text.contains("- erased, contents gone"), "{text}");
        assert!(!text.contains("pass that draft"), "{text}");

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
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(
            text.contains("- no name: a Review candidate: waiting for review, deletes the file"),
            "{text}"
        );
    }

    #[test]
    fn a_file_owned_by_another_agent_is_refused_in_words() {
        let mut record = Record::new("ops");
        record.version(2, None, None, b"one", Record::call("call_a"));
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(
            text.ends_with(
                "`porto-notes.docx` belongs to another Agent; this Agent cannot continue it."
            ),
            "{text}"
        );
        assert!(
            !note(
                &found,
                Reader {
                    agent: "ops",
                    session: None
                }
            )
            .unwrap_or_default()
            .contains("another Agent")
        );
    }

    #[test]
    fn the_draft_is_not_recommended_over_the_workspace_copy() {
        let mut record = Record::new("vak");
        let first = record.version(2, None, None, b"one", Record::call("call_a"));
        record.push(3, None, ArtifactStep::Promoted { version: first });
        record.version(4, None, Some(first), b"two", Record::call("call_b"));
        let found = file_state(record.artifact().as_ref(), PATH, Workspace::Absent);
        let text = note(
            &found,
            Reader {
                agent: "vak",
                session: None,
            },
        )
        .expect("a note");
        assert!(!text.to_lowercase().contains("prefer"), "{text}");
        assert!(!text.contains("instead"), "{text}");
    }

    #[test]
    fn workspace_path_names_the_file_the_draft_will_become() {
        let cwd = Path::new("/work/space");
        assert_eq!(
            workspace_path(cwd, "./notes/porto.docx"),
            "notes/porto.docx"
        );
        assert_eq!(
            workspace_path(cwd, ".vak/scratch/vak/call_a/porto-notes.docx"),
            "porto-notes.docx"
        );
        assert_eq!(workspace_path(cwd, "/work/space/porto.docx"), "porto.docx");
    }
}
