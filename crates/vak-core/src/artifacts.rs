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
use vak_session::ids::{ArtifactId, PrincipalId, VersionId};
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
    /// A candidate version was promoted into the space's working tree.
    Promoted {
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
    /// A person kept this version: it outlives the conversation it came
    /// from (doc 74 §4).
    Saved {
        version: VersionId,
    },
    /// A comment on a version, by the owner or through a share.
    Commented {
        version: VersionId,
        /// Who wrote it: a principal id.
        author: String,
        author_name: String,
        text: String,
    },
}

/// A comment on one version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub version: VersionId,
    pub author: String,
    pub author_name: String,
    pub text: String,
    pub at: DateTime<Utc>,
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
}

impl Artifact {
    /// The versions no other version was made from: one, unless edits made
    /// from the same version are waiting to be reconciled.
    pub fn heads(&self) -> Vec<&Version> {
        self.versions
            .iter()
            .filter(|version| {
                !self
                    .versions
                    .iter()
                    .any(|child| child.parent.as_ref() == Some(&version.id))
            })
            .collect()
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

#[derive(Default, Serialize, Deserialize)]
struct State {
    artifacts: BTreeMap<String, Artifact>,
}

fn fold(state: &mut State, bytes: &[u8]) {
    let Ok(event) = serde_json::from_slice::<ArtifactEvent>(bytes) else {
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
            }
        }
        ArtifactStep::Saved { version } => {
            if let Some(found) = artifact
                .versions
                .iter_mut()
                .find(|known| known.id == version)
            {
                found.saved = true;
            }
        }
        ArtifactStep::Commented {
            version,
            author,
            author_name,
            text,
        } => artifact.comments.push(Comment {
            version,
            author,
            author_name,
            text,
            at: event.at,
        }),
        ArtifactStep::Renamed { title } => artifact.title = Some(title),
        ArtifactStep::Starred { on } => artifact.starred = on,
        ArtifactStep::Archived { on } => artifact.archived = on,
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

    fn state(&self) -> State {
        self.rollup.read(fold)
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
        self.chain.append(event).map_err(store_error)
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
            ArtifactStep::Declared { .. } | ArtifactStep::Versioned { .. }
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
