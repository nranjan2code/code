//! Artifacts and their versions (plan M8, docs/design/82-library.md,
//! docs/design/73-data-architecture-and-lifecycle.md §10).
//!
//! An artifact is a deliverable a call declared (`Tool::artifact`) or a
//! person saved: one per (space, path), so its id is derived from them and
//! two writers of one file name one artifact. Everything that happens to it
//! is a row of the `artifacts/` record chain naming the run or person
//! behind it. A version names its parent, so two edits made from one
//! version are siblings, never a lost write. A version's bytes are a tenant
//! object granted to the artifact itself, so a saved version outlives the
//! conversation that made it.
//!
//! What a decision needs (an artifact's head, its siblings, its title) is
//! read through a rollup Document over the chain (`vak_session::rollup`),
//! never by replaying it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vak_config::scope::SharedScope;
use vak_session::chain::RecordChain;
use vak_session::ids::{ArtifactId, CommentId, PrincipalId, VersionId};
use vak_session::objects::{ObjectRef, Objects, TenantObjects};
use vak_session::trace::TraceKey;

/// The most bytes one version keeps. A larger deliverable is declared and
/// gets no version: the Library lists it, and Workbench still has the file.
pub const MAX_VERSION_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    #[default]
    File,
    /// An Office file or PDF, edited through drafts and Review.
    Document,
    /// Code changes that promote into git as a commit.
    Changeset,
    /// A card a person saved out of a conversation.
    Card,
}

/// Where a version came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "from", rename_all = "snake_case")]
pub enum VersionSource {
    /// A tool call that wrote the deliverable.
    Call { session: String, call: String },
    /// A Review candidate of the file (its draft before promotion).
    Candidate { session: String, candidate: String },
    /// A person's own edit or upload.
    Person,
}

/// One row of the `artifacts/` chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactEvent {
    pub artifact: ArtifactId,
    pub at: DateTime<Utc>,
    /// The run behind the step; absent for a person's own step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
    #[serde(flatten)]
    pub step: ArtifactStep,
}

vak_session::impl_traced!(ArtifactEvent, "artifact_event");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum ArtifactStep {
    /// The artifact exists: its space, its authoring Agent and its path.
    Declared {
        space: String,
        agent: String,
        path: String,
        kind: ArtifactKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    /// A version, made from `parent` (none for the first).
    Versioned {
        version: VersionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<VersionId>,
        /// Its bytes, when they were kept.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        object: Option<ObjectRef>,
        digest: String,
        size: u64,
        #[serde(flatten)]
        source: VersionSource,
    },
    /// A version in which the file is gone: what a draft that deletes the
    /// file proposes, so a deletion is reviewed and accepted like any other
    /// version (plan M8.4c-c). It has no bytes.
    Removed {
        version: VersionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<VersionId>,
        #[serde(flatten)]
        source: VersionSource,
    },
    /// A Review candidate proposes this version for the file: how Review
    /// reads and decides a candidate by artifact and version (plan
    /// M8.4c-a). The same bytes proposed again name the newer candidate.
    Proposed {
        version: VersionId,
        session: String,
        candidate: String,
    },
    /// A candidate version was promoted into the space's working tree.
    Promoted {
        version: VersionId,
    },
    /// The promotion of this version was undone: the working tree holds
    /// what it held before.
    PromotionUndone {
        version: VersionId,
    },
    Renamed {
        title: String,
    },
    Starred {
        on: bool,
    },
    Archived {
        on: bool,
    },
    /// A draft went into the trash (`on`) or was restored from it. In the
    /// trash it is still whole and can be read; it is erased when its time
    /// there ends (doc 74 §2.7).
    Trashed {
        version: VersionId,
        on: bool,
    },
    /// A version's bytes were erased. The row that named it stays.
    Erased {
        version: VersionId,
    },
    /// A person kept this version: it outlives the conversation it came
    /// from (doc 74 §4).
    Saved {
        version: VersionId,
    },
    /// A person downloaded this version: what a later Put back was made
    /// from (docs/design/82-library.md §7).
    Downloaded {
        version: VersionId,
    },
    /// A comment on a version, by the owner, a guest of its conversation or
    /// through a share: the one thread a version has (plan M8.4c-b).
    Commented {
        version: VersionId,
        /// Who wrote it: a principal id.
        author: String,
        author_name: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<CommentId>,
        #[serde(flatten)]
        at_place: Place,
    },
}

/// Where in the file a comment points: lines of a text file, or an anchor
/// in an Office file or PDF (`Budget!B4`, `page:2/line:5`). Empty for a
/// comment on the whole version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

/// A comment on one version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// Absent on a comment recorded before comments had ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<CommentId>,
    pub version: VersionId,
    pub author: String,
    pub author_name: String,
    pub text: String,
    #[serde(flatten)]
    pub at_place: Place,
    pub at: DateTime<Utc>,
}

/// The Review candidate that proposes a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub session: String,
    pub candidate: String,
}

/// One version as the rollup holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub id: VersionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<VersionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<ObjectRef>,
    pub digest: String,
    pub size: u64,
    #[serde(flatten)]
    pub source: VersionSource,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
    #[serde(default)]
    pub promoted: bool,
    #[serde(default)]
    pub saved: bool,
    /// The file is gone in this version; it has no bytes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    /// The Review candidates that propose this version, oldest first: more
    /// than one when the same bytes were put up for Review again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposed: Vec<Proposal>,
    /// When it went into the trash; absent while it is not there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed_at: Option<DateTime<Utc>>,
    /// When it was last restored from the trash: a draft's age starts
    /// again from here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_at: Option<DateTime<Utc>>,
    /// Its bytes were erased.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub erased: bool,
}

impl Version {
    /// A version nobody accepted or saved, whose bytes are kept.
    pub fn is_draft(&self) -> bool {
        !self.promoted && !self.saved && !self.removed && !self.erased && self.object.is_some()
    }

    /// Neither in the trash nor erased.
    pub fn is_present(&self) -> bool {
        self.trashed_at.is_none() && !self.erased
    }

    /// The newest candidate that proposes this version: the one Review
    /// acts on.
    pub fn proposal(&self) -> Option<&Proposal> {
        self.proposed.last()
    }

    pub fn proposed_by(&self, candidate: &str) -> bool {
        self.proposed
            .iter()
            .any(|proposal| proposal.candidate == candidate)
    }
}

/// An artifact's current state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: ArtifactId,
    pub space: String,
    pub agent: String,
    pub path: String,
    pub kind: ArtifactKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub versions: Vec<Version>,
    #[serde(default)]
    pub starred: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    /// The version a person last downloaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_download: Option<VersionId>,
}

impl Artifact {
    /// The versions no other version was made from: one, unless edits made
    /// from the same version are waiting to be reconciled. A version in the
    /// trash or erased is not counted, so the version it was made from is
    /// current again.
    pub fn heads(&self) -> Vec<&Version> {
        self.versions
            .iter()
            .filter(|version| {
                version.is_present()
                    && !self.versions.iter().any(|child| {
                        child.is_present() && child.parent.as_ref() == Some(&version.id)
                    })
            })
            .collect()
    }

    /// Whether every version it has is in the trash or erased: it is
    /// listed in the trash, not the Library.
    pub fn in_trash(&self) -> bool {
        !self.versions.is_empty() && !self.versions.iter().any(Version::is_present)
    }

    /// The newest head: what "the current version" means.
    pub fn head(&self) -> Option<&Version> {
        self.heads().into_iter().max_by_key(|version| version.at)
    }

    /// How it is named to a person: its title, else its file name.
    pub fn name(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            Path::new(&self.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.clone())
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("no artifact {0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("artifact store: {0}")]
    Store(String),
}

fn store_error(error: impl std::fmt::Display) -> ArtifactError {
    ArtifactError::Store(error.to_string())
}

/// The id of the artifact at `path` in `space`: the same file is always
/// the same artifact.
pub fn artifact_id(space: &str, path: &str) -> ArtifactId {
    ArtifactId::derived(&format!("{space}\n{}", normalize(path)))
}

/// A path as an artifact names it: relative, `/`-separated, with a draft's
/// `.vak/scratch/<agent>/<execution>/` prefix removed so a draft names the
/// file it will become.
pub fn normalize(path: &str) -> String {
    let path = path.trim().trim_start_matches("./").replace('\\', "/");
    let scratch = format!("{}/", vak_config::scope::SCRATCH_DIR.trim_end_matches('/'));
    match path.strip_prefix(&scratch) {
        Some(rest) => rest.splitn(3, '/').nth(2).unwrap_or(rest).to_string(),
        None => path,
    }
}

/// The grant scope of an artifact's version objects.
pub fn object_scope(artifact: &ArtifactId) -> String {
    format!("artifact:{artifact}")
}

/// The scope one person's comments on an artifact are granted to, so
/// their contributions can be removed and the artifact kept (plan M7a-b).
pub fn comment_scope(artifact: &ArtifactId, author: &str) -> String {
    vak_session::objects::contributor_scope(&artifact.to_string(), author)
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    artifacts: BTreeMap<String, Artifact>,
}

/// What an artifact row says, as against what it is: kept as an object of
/// the artifact's own scope (plan M7a-b), so it goes when the artifact is
/// erased and stays while a saved artifact outlives its conversation.
const ROW_CONTENT: &[&str] = &["path", "title", "summary", "author_name", "text"];

fn fold(state: &mut State, bytes: &[u8]) {
    let Ok(mut row) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return;
    };
    if !matches!(
        vak_session::content::restore(&mut row),
        Ok(vak_session::content::Restored::Whole)
    ) {
        return;
    }
    let Ok(event) = serde_json::from_value::<ArtifactEvent>(row) else {
        return;
    };

    let key = event.artifact.to_string();
    if let ArtifactStep::Declared {
        space,
        agent,
        path,
        kind,
        title,
        summary,
    } = &event.step
    {
        state
            .artifacts
            .entry(key.clone())
            .or_insert_with(|| Artifact {
                id: event.artifact,
                space: space.clone(),
                agent: agent.clone(),
                path: path.clone(),
                kind: *kind,
                title: title.clone(),
                summary: summary.clone(),
                created_at: event.at,
                updated_at: event.at,
                versions: Vec::new(),
                starred: false,
                archived: false,
                comments: Vec::new(),
                last_download: None,
            });
    }
    let Some(artifact) = state.artifacts.get_mut(&key) else {
        return;
    };
    artifact.updated_at = event.at;
    match event.step {
        ArtifactStep::Declared { title, summary, .. } => {
            // A later declaration may name what an earlier one left out.
            artifact.title = artifact.title.take().or(title);
            artifact.summary = artifact.summary.take().or(summary);
        }
        ArtifactStep::Versioned {
            version,
            parent,
            object,
            digest,
            size,
            source,
        } => {
            if !artifact.versions.iter().any(|known| known.id == version) {
                artifact.versions.push(Version {
                    id: version,
                    parent,
                    object,
                    digest,
                    size,
                    source,
                    at: event.at,
                    actor: event.actor,
                    promoted: false,
                    saved: false,
                    proposed: Vec::new(),
                    removed: false,
                    trashed_at: None,
                    restored_at: None,
                    erased: false,
                });
            }
        }
        ArtifactStep::Removed {
            version,
            parent,
            source,
        } => {
            if !artifact.versions.iter().any(|known| known.id == version) {
                artifact.versions.push(Version {
                    id: version,
                    parent,
                    object: None,
                    digest: String::new(),
                    size: 0,
                    source,
                    at: event.at,
                    actor: event.actor,
                    promoted: false,
                    saved: false,
                    proposed: Vec::new(),
                    removed: true,
                    trashed_at: None,
                    restored_at: None,
                    erased: false,
                });
            }
        }
        ArtifactStep::Promoted { version } => {
            if let Some(found) = artifact
                .versions
                .iter_mut()
                .find(|known| known.id == version)
            {
                found.promoted = true;
                found.trashed_at = None;
            }
        }
        ArtifactStep::Proposed {
            version,
            session,
            candidate,
        } => {
            if let Some(found) = artifact
                .versions
                .iter_mut()
                .find(|known| known.id == version)
            {
                found.proposed.retain(|known| known.candidate != candidate);
                found.proposed.push(Proposal { session, candidate });
            }
        }
        ArtifactStep::PromotionUndone { version } => {
            if let Some(found) = artifact
                .versions
                .iter_mut()
                .find(|known| known.id == version)
            {
                found.promoted = false;
            }
        }
        ArtifactStep::Saved { version } => {
            if let Some(found) = artifact
                .versions
                .iter_mut()
                .find(|known| known.id == version)
            {
                found.saved = true;
                found.trashed_at = None;
            }
        }
        ArtifactStep::Downloaded { version } => artifact.last_download = Some(version),
        ArtifactStep::Commented {
            version,
            author,
            author_name,
            text,
            id,
            at_place,
        } => artifact.comments.push(Comment {
            id,
            version,
            author,
            author_name,
            text,
            at_place,
            at: event.at,
        }),
        ArtifactStep::Renamed { title } => artifact.title = Some(title),
        ArtifactStep::Starred { on } => artifact.starred = on,
        ArtifactStep::Archived { on } => artifact.archived = on,
        ArtifactStep::Trashed { version, on } => {
            if let Some(found) = artifact
                .versions
                .iter_mut()
                .find(|known| known.id == version)
            {
                if on {
                    // Trashing again does not restart its time there.
                    found.trashed_at.get_or_insert(event.at);
                } else if found.trashed_at.take().is_some() {
                    found.restored_at = Some(event.at);
                }
            }
        }
        ArtifactStep::Erased { version } => {
            if let Some(found) = artifact
                .versions
                .iter_mut()
                .find(|known| known.id == version)
            {
                found.erased = true;
                found.object = None;
            }
        }
    }
}

/// The artifacts of a data home.
#[derive(Debug, Clone)]
pub struct Artifacts {
    chain: RecordChain,
    rollup: vak_session::rollup::Rollup,
    tenant_home: PathBuf,
}

/// What a new version is.
pub struct NewVersion<'a> {
    /// The version it was made from; `None` makes it from the current head.
    pub parent: Option<VersionId>,
    pub bytes: &'a [u8],
    pub source: VersionSource,
}

impl Artifacts {
    pub fn at(shared: &SharedScope, tenant_home: impl Into<PathBuf>) -> Self {
        Self {
            chain: RecordChain::at(shared.artifacts()),
            rollup: vak_session::rollup::Rollup::new(shared.artifacts(), shared.artifacts_rollup()),
            tenant_home: tenant_home.into(),
        }
    }

    pub fn path(&self) -> &Path {
        self.chain.path()
    }

    /// The rollup's state, less every artifact whose key was destroyed: the
    /// rollup may have folded its rows before the erasure.
    fn state(&self) -> State {
        let mut state: State = self.rollup.read(fold);
        state
            .artifacts
            .retain(|id, _| !vak_session::content::scope_destroyed(&format!("artifact:{id}")));
        // So may a comment whose author has since been removed.
        let mut removed: BTreeMap<String, bool> = BTreeMap::new();
        for (id, artifact) in &mut state.artifacts {
            artifact.comments.retain(|comment| {
                let scope = vak_session::objects::contributor_scope(id, &comment.author);
                !*removed
                    .entry(scope)
                    .or_insert_with_key(|scope| vak_session::content::scope_destroyed(scope))
            });
        }
        state
    }

    /// Every artifact, newest change first.
    pub fn list(&self) -> Vec<Artifact> {
        let mut all: Vec<Artifact> = self.state().artifacts.into_values().collect();
        all.sort_by_key(|artifact| std::cmp::Reverse(artifact.updated_at));
        all
    }

    pub fn get(&self, id: &str) -> Option<Artifact> {
        self.state().artifacts.remove(id)
    }

    fn append(&self, event: &ArtifactEvent) -> Result<(), ArtifactError> {
        let mut row = serde_json::to_value(event).map_err(|error| {
            store_error(vak_session::types::SessionError::Objects(error.to_string()))
        })?;
        // A comment belongs to who wrote it; everything else an artifact's
        // rows say belongs to the artifact.
        let scope = match &event.step {
            ArtifactStep::Commented { author, .. } => comment_scope(&event.artifact, author),
            _ => object_scope(&event.artifact),
        };
        vak_session::content::seal_fields_under(&mut row, &scope, ROW_CONTENT)
            .map_err(store_error)?;
        self.chain.append(&row).map_err(store_error)
    }

    /// Declares the artifact at `path` in `space` (once; a later
    /// declaration adds a title or summary the first left out) and returns
    /// its id.
    #[allow(clippy::too_many_arguments)]
    pub fn declare(
        &self,
        space: &str,
        agent: &str,
        path: &str,
        kind: ArtifactKind,
        title: Option<String>,
        summary: Option<String>,
        trace: Option<&TraceKey>,
        actor: Option<PrincipalId>,
    ) -> Result<ArtifactId, ArtifactError> {
        let path = normalize(path);
        if path.is_empty() || path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(ArtifactError::Invalid(
                "an artifact's path is relative to its space".into(),
            ));
        }
        let id = artifact_id(space, &path);
        let known = self.get(&id.to_string());
        let adds_name = known.as_ref().is_some_and(|artifact| {
            (artifact.title.is_none() && title.is_some())
                || (artifact.summary.is_none() && summary.is_some())
        });
        if known.is_none() || adds_name {
            self.append(&ArtifactEvent {
                artifact: id,
                at: Utc::now(),
                trace: trace.cloned(),
                actor: actor.or_else(|| trace.and_then(|trace| trace.actor)),
                step: ArtifactStep::Declared {
                    space: space.to_string(),
                    agent: agent.to_string(),
                    path,
                    kind,
                    title,
                    summary,
                },
            })?;
        }
        Ok(id)
    }

    /// Adds a version of `artifact`. The same bytes as its parent are no
    /// new version: the parent is returned.
    pub fn version(
        &self,
        artifact: ArtifactId,
        new: NewVersion<'_>,
        trace: Option<&TraceKey>,
        actor: Option<PrincipalId>,
    ) -> Result<VersionId, ArtifactError> {
        let current = self
            .get(&artifact.to_string())
            .ok_or_else(|| ArtifactError::NotFound(artifact.to_string()))?;
        let parent = new.parent.or_else(|| current.head().map(|head| head.id));
        let digest = {
            use sha2::Digest;
            hex::encode(sha2::Sha256::digest(new.bytes))
        };
        if let Some(parent) = parent
            && let Some(same) = current.versions.iter().find(|version| version.id == parent)
            && same.digest == digest
        {
            return Ok(parent);
        }
        if new.bytes.len() as u64 > MAX_VERSION_BYTES {
            return Err(ArtifactError::Invalid(
                "the file is larger than a version keeps".into(),
            ));
        }
        let objects = TenantObjects::for_tenant(&self.tenant_home).map_err(store_error)?;
        let object = Some(
            objects
                .put(new.bytes, &object_scope(&artifact))
                .map_err(store_error)?,
        );
        let version = VersionId::new();
        self.append(&ArtifactEvent {
            artifact,
            at: Utc::now(),
            trace: trace.cloned(),
            actor: actor.or_else(|| trace.and_then(|trace| trace.actor)),
            step: ArtifactStep::Versioned {
                version,
                parent,
                object,
                digest,
                size: new.bytes.len() as u64,
                source: new.source,
            },
        })?;
        Ok(version)
    }

    /// Adds a version of `artifact` in which the file is gone, made from
    /// `parent` or the current head. A head that is already such a version
    /// is returned as it is.
    pub fn removal(
        &self,
        artifact: ArtifactId,
        parent: Option<VersionId>,
        source: VersionSource,
        trace: Option<&TraceKey>,
        actor: Option<PrincipalId>,
    ) -> Result<VersionId, ArtifactError> {
        let current = self
            .get(&artifact.to_string())
            .ok_or_else(|| ArtifactError::NotFound(artifact.to_string()))?;
        let parent = parent.or_else(|| current.head().map(|head| head.id));
        if let Some(parent) = parent
            && let Some(same) = current.versions.iter().find(|known| known.id == parent)
            && same.removed
        {
            return Ok(parent);
        }
        let version = VersionId::new();
        self.append(&ArtifactEvent {
            artifact,
            at: Utc::now(),
            trace: trace.cloned(),
            actor: actor.or_else(|| trace.and_then(|trace| trace.actor)),
            step: ArtifactStep::Removed {
                version,
                parent,
                source,
            },
        })?;
        Ok(version)
    }

    /// Records a person's or a run's step on an artifact.
    pub fn record(
        &self,
        artifact: ArtifactId,
        step: ArtifactStep,
        trace: Option<&TraceKey>,
        actor: Option<PrincipalId>,
    ) -> Result<(), ArtifactError> {
        if matches!(
            step,
            ArtifactStep::Declared { .. }
                | ArtifactStep::Versioned { .. }
                | ArtifactStep::Removed { .. }
        ) {
            return Err(ArtifactError::Invalid(
                "declare and version through their own calls".into(),
            ));
        }
        if self.get(&artifact.to_string()).is_none() {
            return Err(ArtifactError::NotFound(artifact.to_string()));
        }
        self.append(&ArtifactEvent {
            artifact,
            at: Utc::now(),
            trace: trace.cloned(),
            actor: actor.or_else(|| trace.and_then(|trace| trace.actor)),
            step,
        })
    }

    /// Moves a draft into the trash or restores it. Only a draft goes in:
    /// a version someone accepted or saved is kept.
    pub fn trash_version(
        &self,
        artifact: ArtifactId,
        version: VersionId,
        on: bool,
        actor: Option<PrincipalId>,
    ) -> Result<(), ArtifactError> {
        let current = self
            .get(&artifact.to_string())
            .ok_or_else(|| ArtifactError::NotFound(artifact.to_string()))?;
        let found = current
            .versions
            .iter()
            .find(|known| known.id == version)
            .ok_or_else(|| ArtifactError::NotFound(version.to_string()))?;
        if found.erased {
            return Err(ArtifactError::Invalid("this version was erased".into()));
        }
        if on && !found.is_draft() {
            return Err(ArtifactError::Invalid(
                "only a draft nobody accepted or saved goes to the trash".into(),
            ));
        }
        if on == found.trashed_at.is_some() {
            return Ok(());
        }
        self.record(artifact, ArtifactStep::Trashed { version, on }, None, actor)
    }

    /// Erases a version from the trash: its bytes are released, unless
    /// another version of the artifact holds the same ones, and the record
    /// says so. Returns whether it erased anything.
    pub fn erase_version(
        &self,
        artifact: ArtifactId,
        version: VersionId,
        actor: Option<PrincipalId>,
    ) -> Result<bool, ArtifactError> {
        let current = self
            .get(&artifact.to_string())
            .ok_or_else(|| ArtifactError::NotFound(artifact.to_string()))?;
        let found = current
            .versions
            .iter()
            .find(|known| known.id == version)
            .ok_or_else(|| ArtifactError::NotFound(version.to_string()))?;
        if found.erased {
            return Ok(false);
        }
        if found.trashed_at.is_none() {
            return Err(ArtifactError::Invalid(
                "a version is erased from the trash".into(),
            ));
        }
        // The record first: a version that says it is whole must be.
        self.record(artifact, ArtifactStep::Erased { version }, None, actor)?;
        if let Some(object) = &found.object {
            let shared = current
                .versions
                .iter()
                .any(|other| other.id != version && other.object.as_ref() == Some(object));
            if !shared {
                TenantObjects::for_tenant(&self.tenant_home)
                    .and_then(|objects| objects.release(object, &object_scope(&artifact)))
                    .map_err(store_error)?;
            }
        }
        Ok(true)
    }

    /// A version's bytes, when they were kept.
    pub fn bytes(
        &self,
        artifact: &Artifact,
        version: &VersionId,
    ) -> Result<Vec<u8>, ArtifactError> {
        let found = artifact
            .versions
            .iter()
            .find(|known| &known.id == version)
            .ok_or_else(|| ArtifactError::NotFound(version.to_string()))?;
        if found.removed {
            return Err(ArtifactError::Invalid(
                "the file is gone in this version".into(),
            ));
        }
        if found.erased {
            return Err(ArtifactError::Invalid("this version was erased".into()));
        }
        let object = found
            .object
            .as_ref()
            .ok_or_else(|| ArtifactError::Invalid("this version was too large to keep".into()))?;
        TenantObjects::for_tenant(&self.tenant_home)
            .and_then(|objects| objects.get(object, &object_scope(&artifact.id)))
            .map_err(store_error)
    }
}

/// An Office file or PDF is a document; anything else a file.
fn kind_of(path: &str) -> ArtifactKind {
    let extension = Path::new(path)
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let office = matches!(
        extension.as_str(),
        "docx"
            | "docm"
            | "dotx"
            | "dotm"
            | "xlsx"
            | "xlsm"
            | "xltx"
            | "xltm"
            | "pptx"
            | "pptm"
            | "potx"
            | "potm"
            | "ppsx"
            | "ppsm"
            | "vsdx"
            | "vsdm"
            | "vstx"
            | "vstm"
            | "pdf"
    );
    if office {
        ArtifactKind::Document
    } else {
        ArtifactKind::File
    }
}

/// Records what a model's tool calls declare (`vak_tools::ArtifactSink`):
/// the artifact, and the file it wrote as a version of it.
pub struct CallSink {
    pub artifacts: Artifacts,
    /// The workspace the calls ran in.
    pub cwd: PathBuf,
    /// Where the space's drafts live (`vak_config::scope::executions_root`).
    pub executions: PathBuf,
    pub agent: String,
}

impl CallSink {
    fn file(&self, path: &str) -> Option<PathBuf> {
        let named = Path::new(path);
        let resolved = vak_config::scope::draft_location(&self.executions, named)
            .unwrap_or_else(|| self.cwd.join(named));
        let canonical = resolved.canonicalize().ok()?;
        let inside = canonical.starts_with(self.cwd.canonicalize().ok()?)
            || self
                .executions
                .canonicalize()
                .is_ok_and(|root| canonical.starts_with(root));
        inside.then_some(canonical)
    }
}

impl vak_tools::ArtifactSink for CallSink {
    fn declared(
        &self,
        declaration: &vak_tools::ArtifactClaim,
        session: Option<&str>,
        call: &str,
        trace: Option<&TraceKey>,
    ) {
        let space = vak_session::trace::local::space(&self.cwd).to_string();
        let kind = kind_of(&declaration.path);
        // An absolute path inside the workspace names the same artifact as
        // its relative form.
        let named = Path::new(&declaration.path);
        let path = if named.is_absolute() {
            match self.cwd.canonicalize().ok().and_then(|root| {
                named
                    .canonicalize()
                    .ok()?
                    .strip_prefix(&root)
                    .ok()
                    .map(|rest| rest.to_string_lossy().into_owned())
            }) {
                Some(relative) => relative,
                None => declaration.path.clone(),
            }
        } else {
            declaration.path.clone()
        };
        // An undeclared claim versions only a file that is already an
        // artifact: continuing one is a new version of it (doc 82 §6), and
        // a supporting file never becomes an entry (§3).
        let known = artifact_id(&space, &path);
        if !declaration.declared && self.artifacts.get(&known.to_string()).is_none() {
            return;
        }
        let artifact = if declaration.declared {
            self.artifacts.declare(
                &space,
                &self.agent,
                &path,
                kind,
                declaration.title.clone(),
                declaration.summary.clone(),
                trace,
                None,
            )
        } else {
            Ok(known)
        };
        let recorded = artifact.and_then(|artifact| {
            let file = self.file(&declaration.path).ok_or_else(|| {
                ArtifactError::Invalid("the declared file is not in the workspace".into())
            })?;
            if std::fs::metadata(&file).map_err(store_error)?.len() > MAX_VERSION_BYTES {
                return Err(ArtifactError::Invalid(
                    "the file is larger than a version keeps".into(),
                ));
            }
            let bytes = std::fs::read(&file).map_err(store_error)?;
            self.artifacts.version(
                artifact,
                NewVersion {
                    parent: None,
                    bytes: &bytes,
                    source: VersionSource::Call {
                        session: session.unwrap_or_default().to_string(),
                        call: call.to_string(),
                    },
                },
                trace,
                None,
            )
        });
        if let Err(error) = recorded {
            tracing::warn!(
                kind = "artifact",
                error_kind = %vak_telemetry::error_kind(&error),
                "a declared deliverable was not recorded"
            );
        }
    }
}
