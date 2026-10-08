//! The Library over HTTP (plan M8, docs/design/82-library.md): every
//! artifact and its versions, read through the artifacts rollup. Review
//! records reach it here too: a candidate of a declared deliverable is a
//! version of it, and its promotion is recorded on that version.

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use vak_core::artifacts::{ArtifactStep, NewVersion, VersionSource};

use crate::AppState;

/// Records what a Review record means for the artifacts it touches. Each
/// file of a candidate is an artifact (declared here when nothing declared
/// it before) and gets the candidate's bytes as a version, made from the version its parent
/// candidate proposed, and that version is recorded as what the candidate
/// proposes (the same bytes as the current version propose that one). A
/// promotion marks the versions of the files it applied, and its undo
/// unmarks them.
pub(crate) fn note_review(state: &AppState, record: &vak_sandbox::DurableRecord) {
    let artifacts = state.core.artifacts();
    let warn = |error: vak_core::artifacts::ArtifactError| {
        tracing::warn!(kind = "artifact", error_kind = %vak_telemetry::error_kind(&error), "a review record was not recorded on its artifact");
    };
    // The versions `candidate` proposes, limited to `paths` when given.
    let proposed_by = |candidate: &str, paths: Option<&[String]>| {
        let mut found = Vec::new();
        for artifact in artifacts.list() {
            if let Some(paths) = paths
                && !paths
                    .iter()
                    .any(|path| vak_core::artifacts::normalize(path) == artifact.path)
            {
                continue;
            }
            for version in &artifact.versions {
                if version.proposed_by(candidate) {
                    found.push((artifact.id, version.id, version.promoted));
                }
            }
        }
        found
    };
    match record {
        vak_sandbox::DurableRecord::Candidate(candidate) => {
            let manifest = &candidate.candidate;
            let space = vak_session::trace::local::space(&manifest.destination_root).to_string();
            for file in &manifest.files {
                let id = vak_core::artifacts::artifact_id(&space, &file.path);
                let deletes = file.operation == vak_sandbox::CandidateOperation::Delete;
                let bytes = if deletes {
                    Vec::new()
                } else {
                    match std::fs::read(manifest.source_root.join(&file.path)) {
                        Ok(bytes) => bytes,
                        Err(_) => continue,
                    }
                };
                // Putting a file up for Review declares it: a person is
                // asked to decide it, and its thread and versions need an
                // artifact to belong to (plan M8.4c-b).
                if artifacts.get(&id.to_string()).is_none()
                    && let Err(error) = artifacts.declare(
                        &space,
                        &crate::session_agent_name(state, &candidate.session_id),
                        &file.path,
                        if crate::is_document_path(&file.path) {
                            vak_core::artifacts::ArtifactKind::Document
                        } else {
                            vak_core::artifacts::ArtifactKind::File
                        },
                        None,
                        None,
                        candidate.trace.as_ref(),
                        candidate.actor,
                    )
                {
                    warn(error);
                    continue;
                }
                let Some(artifact) = artifacts.get(&id.to_string()) else {
                    continue;
                };
                let parent = candidate.parent_candidate_id.as_ref().and_then(|parent| {
                    artifact
                        .versions
                        .iter()
                        .rev()
                        .find(|version| version.proposed_by(parent))
                        .map(|version| version.id)
                });
                let source = VersionSource::Candidate {
                    session: candidate.session_id.clone(),
                    candidate: manifest.candidate_id.clone(),
                };
                // A draft that deletes the file proposes a version in which
                // it is gone.
                let made = if deletes {
                    artifacts.removal(
                        id,
                        parent,
                        source,
                        candidate.trace.as_ref(),
                        candidate.actor,
                    )
                } else {
                    artifacts.version(
                        id,
                        NewVersion {
                            parent,
                            bytes: &bytes,
                            source,
                        },
                        candidate.trace.as_ref(),
                        candidate.actor,
                    )
                };
                let version = match made {
                    Ok(version) => version,
                    Err(error) => {
                        warn(error);
                        continue;
                    }
                };
                if let Err(error) = artifacts.record(
                    id,
                    ArtifactStep::Proposed {
                        version,
                        session: candidate.session_id.clone(),
                        candidate: manifest.candidate_id.clone(),
                    },
                    candidate.trace.as_ref(),
                    candidate.actor,
                ) {
                    warn(error);
                }
            }
        }
        vak_sandbox::DurableRecord::Promotion(promotion) => {
            for (artifact, version, promoted) in
                proposed_by(&promotion.candidate_id, Some(&promotion.receipt.applied))
            {
                if !promoted
                    && let Err(error) = artifacts.record(
                        artifact,
                        ArtifactStep::Promoted { version },
                        promotion.trace.as_ref(),
                        promotion.actor,
                    )
                {
                    warn(error);
                }
            }
        }
        vak_sandbox::DurableRecord::PromotionUndo(undo) => {
            for (artifact, version, promoted) in proposed_by(&undo.candidate_id, None) {
                if promoted
                    && let Err(error) = artifacts.record(
                        artifact,
                        ArtifactStep::PromotionUndone { version },
                        None,
                        Some(crate::request_actor(state)),
                    )
                {
                    warn(error);
                }
            }
        }
        _ => {}
    }
}

/// The artifacts and versions a conversation's Review and declared files
/// are: what its sandbox records name them by (plan M8.4c-a).
pub(crate) fn bindings(
    state: &AppState,
    session: &str,
    principal: &crate::AuthenticatedPrincipal,
) -> Vec<serde_json::Value> {
    let mut found = Vec::new();
    for artifact in state.core.artifacts().list() {
        for version in &artifact.versions {
            // A guest is told only of versions their grant reaches.
            if discusser(
                state,
                principal,
                &artifact,
                version,
                vak_core::grants::Role::Viewer,
            )
            .is_err()
            {
                continue;
            }
            let entry = |candidate: Option<&str>| {
                serde_json::json!({
                    "artifact": artifact.id,
                    "version": version.id,
                    "path": artifact.path,
                    "candidate": candidate,
                    "promoted": version.promoted,
                    "removed": version.removed,
                })
            };
            let here: Vec<_> = version
                .proposed
                .iter()
                .filter(|proposal| proposal.session == session)
                .collect();
            let declared_here = matches!(&version.source,
                VersionSource::Call { session: from, .. } if from == session);
            if here.is_empty() && declared_here {
                found.push(entry(None));
            }
            for proposal in here {
                found.push(entry(Some(&proposal.candidate)));
            }
        }
    }
    found
}

/// The Review candidate a version is, and the file in it.
struct InReview {
    session: String,
    candidate: String,
    path: String,
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// Resolves an artifact version to the candidate that proposes it. An
/// unknown artifact or version is not found; a version nobody proposed for
/// review is a conflict; a version whose conversation is in the trash is
/// not found, as everything of a trashed conversation is.
fn in_review(state: &AppState, id: &str, version: &str) -> Result<InReview, Box<Response>> {
    let missing = || Box::new(refuse(StatusCode::NOT_FOUND, "no such version"));
    let version = vak_session::ids::VersionId::parse(version).map_err(|_| missing())?;
    let artifact = state.core.artifacts().get(id).ok_or_else(missing)?;
    let found = artifact
        .versions
        .iter()
        .find(|known| known.id == version)
        .ok_or_else(missing)?;
    let Some(proposal) = found.proposal() else {
        return Err(Box::new(refuse(
            StatusCode::CONFLICT,
            "this version is not waiting for review",
        )));
    };
    if vak_core::trash::is_trashed(&state.core.shared_scope(), &proposal.session) {
        return Err(Box::new(refuse(
            StatusCode::NOT_FOUND,
            "its conversation is in the trash",
        )));
    }
    let saved = crate::saved_candidate(state, &proposal.session, &proposal.candidate)
        .map_err(|status| Box::new(status.into_response()))?;
    let manifest = &saved.candidate;
    let space = vak_session::trace::local::space(&manifest.destination_root).to_string();
    let path = manifest
        .files
        .iter()
        .find(|file| vak_core::artifacts::artifact_id(&space, &file.path) == artifact.id)
        .map(|file| file.path.clone())
        .ok_or_else(missing)?;
    Ok(InReview {
        session: proposal.session.clone(),
        candidate: proposal.candidate.clone(),
        path,
    })
}

/// Refuses a read of a version by anyone but the owner or a guest of the
/// conversation it is reviewed in, and gives back its bytes and path.
async fn readable(
    state: &AppState,
    principal: &crate::AuthenticatedPrincipal,
    id: String,
    version: &str,
) -> Result<(String, Vec<u8>), StatusCode> {
    let artifact = state
        .core
        .artifacts()
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let found = version_of(&artifact, version)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    discusser(
        state,
        principal,
        &artifact,
        &found,
        vak_core::grants::Role::Viewer,
    )?;
    let artifacts = state.core.artifacts();
    let path = artifact.path.clone();
    tokio::task::spawn_blocking(move || artifacts.bytes(&artifact, &found.id))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(|bytes| (path, bytes))
        .map_err(|_| StatusCode::NOT_FOUND)
}

/// `GET /library/{id}/versions/{version}/text`: a version as text, or its
/// size when it is not text. Never a download, and never recorded as one.
pub(crate) async fn version_text(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
) -> Response {
    match readable(&state, &principal, id, &version).await {
        Ok((path, bytes)) => {
            let size = bytes.len();
            let content = String::from_utf8(bytes).ok();
            Json(serde_json::json!({
                "path": path,
                "kind": if content.is_some() { "text" } else { "binary" },
                "bytes": size,
                "content": content,
                "editable": false,
            }))
            .into_response()
        }
        Err(status) => status.into_response(),
    }
}

/// `GET /library/{id}/versions/{version}/raw`: a version's bytes for a
/// viewer, with its media type and no script allowed to run from it. Not a
/// download, and never recorded as one.
pub(crate) async fn version_raw(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
) -> Response {
    match readable(&state, &principal, id, &version).await {
        Ok((path, bytes)) => (
            [
                (
                    header::CONTENT_TYPE,
                    crate::raw_mime_for(std::path::Path::new(&path)),
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CACHE_CONTROL, "no-store"),
                (header::CONTENT_DISPOSITION, "attachment"),
                (
                    header::CONTENT_SECURITY_POLICY,
                    "sandbox; default-src 'none'",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(status) => status.into_response(),
    }
}

/// Refuses a Review read by a guest of another conversation.
fn may_review(
    state: &AppState,
    principal: &crate::AuthenticatedPrincipal,
    id: &str,
    version: &str,
) -> Result<(), Box<Response>> {
    let missing = || Box::new(refuse(StatusCode::NOT_FOUND, "no such version"));
    let artifact = state.core.artifacts().get(id).ok_or_else(missing)?;
    let found = version_of(&artifact, version).ok_or_else(missing)?;
    discusser(
        state,
        principal,
        &artifact,
        found,
        vak_core::grants::Role::Viewer,
    )
    .map(|_| ())
    .map_err(|status| Box::new(status.into_response()))
}

/// `GET /library/{id}/versions/{version}/document`: the Office or PDF
/// projection of a version in Review, parsed in the worker.
async fn version_document(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
    axum::extract::Query(mut query): axum::extract::Query<crate::OfficeProjectionQuery>,
) -> Response {
    if let Err(response) = may_review(&state, &principal, &id, &version) {
        return *response;
    }
    let review = match in_review(&state, &id, &version) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    query.path = review.path;
    crate::read_sandbox_candidate_office_projection(
        State(state),
        Path((review.session, review.candidate)),
        axum::extract::Query(query),
    )
    .await
}

/// `GET /library/{id}/versions/{version}/review`: what a version in
/// Review changes in the file as the workspace holds it.
pub(crate) async fn version_review(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
) -> Response {
    if let Err(response) = may_review(&state, &principal, &id, &version) {
        return *response;
    }
    let review = match in_review(&state, &id, &version) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    crate::read_sandbox_candidate_office_review(
        State(state),
        Path((review.session, review.candidate)),
        axum::extract::Query(crate::FileQuery {
            path: review.path,
            session: None,
        }),
    )
    .await
}

#[derive(serde::Deserialize)]
struct Narrow {
    keep: Vec<String>,
}

/// `POST /library/{id}/versions/{version}/narrow`: a new version that
/// keeps only the chosen changes of this one.
async fn version_narrow(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    Json(body): Json<Narrow>,
) -> Response {
    let review = match in_review(&state, &id, &version) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    crate::narrow_sandbox_candidate_office(
        State(state),
        Path((review.session, review.candidate)),
        Json(crate::OfficeNarrowBody {
            path: review.path,
            keep: body.keep,
        }),
    )
    .await
}

#[derive(Default, serde::Deserialize)]
pub(crate) struct Accept {
    /// Other versions the same draft proposes, accepted with this one in
    /// the one promotion a draft gets.
    #[serde(default)]
    pub also: Vec<VersionRef>,
}

#[derive(serde::Deserialize)]
pub(crate) struct VersionRef {
    pub artifact: String,
    pub version: String,
}

/// `POST /library/{id}/versions/{version}/accept`: the version becomes the
/// file in the workspace, through the one promotion path. A draft of
/// several files is accepted once: the others it proposes go in `also`.
pub(crate) async fn version_accept(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    body: Option<Json<Accept>>,
) -> Response {
    let review = match in_review(&state, &id, &version) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    let mut files = vec![review.path];
    for other in body.map(|Json(body)| body.also).unwrap_or_default() {
        match in_review(&state, &other.artifact, &other.version) {
            Ok(also) if also.session == review.session && also.candidate == review.candidate => {
                if !files.contains(&also.path) {
                    files.push(also.path);
                }
            }
            Ok(_) => {
                return refuse(
                    StatusCode::CONFLICT,
                    "these versions are not proposed by the same draft",
                );
            }
            Err(response) => return *response,
        }
    }
    crate::promote_sandbox_candidate(
        State(state),
        Path(review.session),
        Json(crate::SandboxPromotionBody {
            candidate_id: review.candidate,
            files,
        }),
    )
    .await
}

/// `POST /library/{id}/versions/{version}/undo`: the workspace gets back
/// what it held before this version was accepted.
pub(crate) async fn version_undo(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
) -> Response {
    let review = match in_review(&state, &id, &version) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    crate::undo_sandbox_promotion(State(state), Path((review.session, review.candidate))).await
}

/// `POST /library/{id}/versions/{version}/checks`: run one planned
/// workspace check of an accepted version.
async fn version_check(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    Json(body): Json<crate::WorkspaceCheckBody>,
) -> Response {
    let review = match in_review(&state, &id, &version) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    crate::run_sandbox_workspace_check(
        State(state),
        Path((review.session, review.candidate)),
        Json(body),
    )
    .await
}

/// What a turn request names to attach (plan M8.3b).
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct ArtifactRef {
    pub id: String,
    #[serde(default)]
    pub mode: vak_session::ArtifactMode,
}

/// The block that tells the model about an attached artifact, written from
/// the artifact's own records, and the typed attachment that names it
/// (docs/design/82-library.md §6). It carries the artifact's metadata and
/// where its versions came from, never conversation text. Refused when the
/// artifact is not this conversation's Agent's or not in its workspace:
/// Continue working continues in the authoring Agent's own conversation.
pub(crate) fn attach(
    core: &vak_core::Core,
    reference: &ArtifactRef,
) -> Result<(String, vak_session::AttachedArtifact), String> {
    let artifact = core
        .artifacts()
        .get(&reference.id)
        .ok_or_else(|| format!("no artifact {}", reference.id))?;
    let agent = core
        .agent_identity()
        .map(|agent| agent.id.clone())
        .unwrap_or_else(|| "vak".into());
    if artifact.agent != agent {
        return Err(format!(
            "{} was made by another Agent; continue it in that Agent's conversation",
            artifact.name()
        ));
    }
    let space = vak_session::trace::local::space(core.cwd()).to_string();
    if artifact.space != space {
        return Err(format!("{} belongs to another workspace", artifact.name()));
    }
    let head = artifact
        .head()
        .ok_or_else(|| format!("{} has no version yet", artifact.name()))?;
    let mut conversations: Vec<String> = Vec::new();
    let mut lines = Vec::new();
    for (index, version) in artifact.versions.iter().enumerate() {
        let (who, session) = match &version.source {
            VersionSource::Call { session, .. } => ("Vakyartha", Some(session)),
            VersionSource::Candidate { session, .. } => ("Vakyartha, for review", Some(session)),
            VersionSource::Person => ("the person", None),
        };
        let from = session
            .filter(|session| !session.is_empty())
            .map(|session| {
                if !conversations.contains(session) {
                    conversations.push(session.clone());
                }
                format!(", in conversation {session}")
            })
            .unwrap_or_default();
        lines.push(format!(
            "- version {}: {who}, {}{from}",
            index + 1,
            version.at.format("%Y-%m-%d %H:%M UTC")
        ));
    }
    let number = artifact
        .versions
        .iter()
        .position(|version| version.id == head.id)
        .map_or(0, |index| index + 1);
    // Name the tool for its kind: a small model told only "change it"
    // reached for office_apply on a text file (measured live).
    let how = match artifact.kind {
        vak_core::artifacts::ArtifactKind::Document => {
            "with `office_apply` (the change is a draft the person reviews)"
        }
        _ => "with `edit`, or `write` the whole file",
    };
    let ask = match reference.mode {
        vak_session::ArtifactMode::Continue => format!(
            "The person wants to keep working on it: change {} {how}; each change becomes a new version.",
            artifact.path
        ),
        vak_session::ArtifactMode::Another => format!(
            "The person wants a new one like it: write a new file, and do not change {}.",
            artifact.path
        ),
    };
    let reach = if conversations.is_empty() {
        String::new()
    } else {
        "\nThe conversations that made it can be read with `recall` and their id.".to_string()
    };
    let kind = match artifact.kind {
        vak_core::artifacts::ArtifactKind::Document => "a document",
        vak_core::artifacts::ArtifactKind::Changeset => "code changes",
        vak_core::artifacts::ArtifactKind::Card => "a saved card",
        vak_core::artifacts::ArtifactKind::File => "a file",
    };
    // The digest stays in the typed attachment: shown to the model, it read
    // as an `office_apply` base digest.
    let block = format!(
        "[Library artifact] \"{}\": {kind} at {}, now at version {number} of {}.\nVersions:\n{}\n{ask}{reach}",
        artifact.name(),
        artifact.path,
        artifact.versions.len(),
        lines.join("\n"),
    );
    Ok((
        block,
        vak_session::AttachedArtifact {
            block: 0,
            artifact: artifact.id.to_string(),
            name: artifact.name(),
            path: artifact.path.clone(),
            version: head.id.to_string(),
            digest: head.digest.clone(),
            mode: reference.mode,
            conversations,
        },
    ))
}

/// An artifact as a list shows it: its name and current version, never
/// its bytes.
fn summary(artifact: &vak_core::artifacts::Artifact) -> serde_json::Value {
    let head = artifact.head();
    serde_json::json!({
        "id": artifact.id,
        "name": artifact.name(),
        "path": artifact.path,
        "kind": artifact.kind,
        "agent": artifact.agent,
        "space": artifact.space,
        "summary": artifact.summary,
        "starred": artifact.starred,
        "archived": artifact.archived,
        "created_at": artifact.created_at,
        "updated_at": artifact.updated_at,
        "versions": artifact.versions.len(),
        "siblings": artifact.heads().len(),
        "head": head.map(|version| version.id),
        "size": head.map(|version| version.size),
    })
}

/// `GET /library`: every artifact, newest change first.
async fn list(State(state): State<AppState>) -> Response {
    let artifacts = state.core.artifacts();
    let listed = tokio::task::spawn_blocking(move || artifacts.list()).await;
    match listed {
        Ok(all) => Json(serde_json::json!({
            "artifacts": all.iter().map(summary).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// `GET /library/{id}`: one artifact with every version.
async fn get_one(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let artifacts = state.core.artifacts();
    match tokio::task::spawn_blocking(move || artifacts.get(&id)).await {
        Ok(Some(artifact)) => {
            let mut value = summary(&artifact);
            value["history"] = serde_json::json!(artifact.versions);
            value["comments"] = serde_json::json!(artifact.comments);
            Json(value).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// `GET /library/{id}/versions/{version}`: a version's bytes, as a
/// download.
async fn version_bytes(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
) -> Response {
    let Ok(version) = vak_session::ids::VersionId::parse(&version) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let artifacts = state.core.artifacts();
    let actor = crate::request_actor(&state);
    let read = tokio::task::spawn_blocking(move || {
        let artifact = artifacts.get(&id)?;
        let bytes = artifacts.bytes(&artifact, &version).ok()?;
        // What a later Put back was made from (doc 82 §7).
        if let Err(error) = artifacts.record(artifact.id, ArtifactStep::Downloaded { version }, None, Some(actor)) {
            tracing::warn!(kind = "artifact", error_kind = %vak_telemetry::error_kind(&error), "a download was not recorded");
        }
        Some((artifact.name(), bytes))
    })
    .await;
    match read {
        Ok(Some((name, bytes))) => {
            let safe: String = name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            (
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{safe}\""),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(serde::Deserialize)]
struct Change {
    /// For star and archive.
    #[serde(default)]
    on: Option<bool>,
    /// For rename.
    #[serde(default)]
    title: Option<String>,
}

/// `POST /library/{id}/{action}`: a person stars, renames or archives an
/// artifact, as a record credited to them.
async fn change(
    State(state): State<AppState>,
    Path((id, action)): Path<(String, String)>,
    Json(body): Json<Change>,
) -> Response {
    let step = match action.as_str() {
        "star" => ArtifactStep::Starred {
            on: body.on.unwrap_or(true),
        },
        "archive" => ArtifactStep::Archived {
            on: body.on.unwrap_or(true),
        },
        "rename" => match body.title.as_deref().map(str::trim) {
            Some(title) if !title.is_empty() && title.chars().count() <= 200 => {
                ArtifactStep::Renamed {
                    title: title.to_string(),
                }
            }
            _ => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": "a name is 1 to 200 characters" })),
                )
                    .into_response();
            }
        },
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    record(&state, &id, step)
}

/// `POST /library/{id}/versions/{version}/save`: a person keeps a version,
/// so it outlives the conversation that made it.
async fn save(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
) -> Response {
    let Ok(version) = vak_session::ids::VersionId::parse(&version) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let known = state
        .core
        .artifacts()
        .get(&id)
        .is_some_and(|artifact| artifact.versions.iter().any(|known| known.id == version));
    if !known {
        return StatusCode::NOT_FOUND.into_response();
    }
    record(&state, &id, ArtifactStep::Saved { version })
}

fn record(state: &AppState, id: &str, step: ArtifactStep) -> Response {
    let Ok(artifact) = vak_session::ids::ArtifactId::parse(id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state
        .core
        .artifacts()
        .record(artifact, step, None, Some(crate::request_actor(state)))
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(vak_core::artifacts::ArtifactError::NotFound(_)) => {
            StatusCode::NOT_FOUND.into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

// ---- Editing and Put back (plan M8.4b, docs/design/82-library.md §7) --------

#[derive(serde::Deserialize)]
struct PersonVersion {
    /// The version it was made from: absent, the one this person last
    /// downloaded, else the current one.
    #[serde(default)]
    parent: Option<String>,
    /// The new content as text (an edit in the app) ...
    #[serde(default)]
    text: Option<String>,
    /// ... or as base64 bytes (a file put back).
    #[serde(default)]
    data: Option<String>,
}

/// `POST /library/{id}/versions`: a version made by the person, from the
/// version they edited or downloaded. Made from an older version, it is a
/// sibling of the versions made since, never an overwrite; made from the
/// current one, it also becomes the file in the workspace, so the Agent
/// builds on it.
async fn person_version(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<PersonVersion>,
) -> Response {
    use base64::Engine as _;
    let Some(artifact) = state.core.artifacts().get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bytes = match (body.text, body.data) {
        (Some(text), None) => text.into_bytes(),
        (None, Some(data)) => match base64::engine::general_purpose::STANDARD.decode(data.trim()) {
            Ok(bytes) => bytes,
            Err(_) => {
                return share_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "the file could not be read",
                );
            }
        },
        _ => {
            return share_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "send the new content as text or as a file",
            );
        }
    };
    let parent = match body.parent {
        Some(parent) => match artifact
            .versions
            .iter()
            .find(|version| version.id.to_string() == parent)
        {
            Some(version) => Some(version.id),
            None => {
                return share_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "that version is not one of this artifact's",
                );
            }
        },
        None => artifact
            .last_download
            .or_else(|| artifact.head().map(|head| head.id)),
    };
    let artifacts = state.core.artifacts();
    let version = match artifacts.version(
        artifact.id,
        NewVersion {
            parent,
            bytes: &bytes,
            source: VersionSource::Person,
        },
        None,
        Some(crate::request_actor(&state)),
    ) {
        Ok(version) => version,
        Err(error) => return share_error(StatusCode::UNPROCESSABLE_ENTITY, &error.to_string()),
    };
    let Some(after) = artifacts.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let sibling = after.heads().len() > 1;
    let written = !sibling
        && after.head().is_some_and(|head| head.id == version)
        && write_back(&state, &after, &bytes);
    Json(serde_json::json!({ "version": version, "sibling": sibling, "written": written }))
        .into_response()
}

/// Writes the current version to the artifact's file, when the artifact is
/// a plain file of this workspace. Documents change only through Review.
fn write_back(state: &AppState, artifact: &vak_core::artifacts::Artifact, bytes: &[u8]) -> bool {
    if artifact.kind != vak_core::artifacts::ArtifactKind::File
        || artifact.space != vak_session::trace::local::space(state.core.cwd()).to_string()
        || artifact
            .path
            .split('/')
            .any(|part| part == ".." || part.is_empty())
    {
        return false;
    }
    let Ok(root) = state.core.cwd().canonicalize() else {
        return false;
    };
    let target = root.join(&artifact.path);
    let inside = target
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .is_some_and(|parent| parent.starts_with(&root));
    let not_a_link =
        std::fs::symlink_metadata(&target).map_or(true, |meta| !meta.file_type().is_symlink());
    inside && not_a_link && std::fs::write(&target, bytes).is_ok()
}

#[derive(serde::Deserialize)]
struct CardSave {
    session: String,
    presentation: String,
}

/// `POST /library/cards`: a card a person keeps out of its conversation
/// (doc 82 §3: a card stays in its chat unless saved). Its payload becomes
/// the artifact's first version.
async fn save_card(State(state): State<AppState>, Json(body): Json<CardSave>) -> Response {
    let Some(log) = crate::open_historical_session(&state, &body.session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some((_, record)) = log
        .presentations()
        .into_iter()
        .find(|(id, _)| *id == body.presentation)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // A card with no title is named from what it shows (its columns),
    // never just its type: "table" names nothing a person would recall.
    let columns: Vec<String> = record
        .payload
        .get("columns")
        .and_then(serde_json::Value::as_array)
        .map(|columns| {
            columns
                .iter()
                .filter_map(|column| {
                    column
                        .get("label")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| column.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let title = if !record.title.trim().is_empty() {
        record.title.clone()
    } else if !columns.is_empty() {
        format!("{} table", columns.join(", "))
    } else {
        format!(
            "Saved {}",
            record.semantic_type.replace(['_', '-', '.'], " ")
        )
    };
    let bytes = serde_json::to_vec_pretty(&record.payload).unwrap_or_default();
    let space = vak_session::trace::local::space(state.core.cwd()).to_string();
    let agent = state
        .core
        .agent_identity()
        .map(|agent| agent.id.clone())
        .unwrap_or_else(|| "vak".into());
    let artifacts = state.core.artifacts();
    let actor = Some(crate::request_actor(&state));
    let saved = artifacts
        .declare(
            &space,
            &agent,
            &format!("cards/{}.json", body.presentation),
            vak_core::artifacts::ArtifactKind::Card,
            Some(title),
            None,
            None,
            actor,
        )
        .and_then(|id| {
            let version = artifacts.version(
                id,
                NewVersion {
                    parent: None,
                    bytes: &bytes,
                    source: VersionSource::Call {
                        session: body.session.clone(),
                        call: body.presentation.clone(),
                    },
                },
                None,
                actor,
            )?;
            artifacts.record(id, ArtifactStep::Saved { version }, None, actor)?;
            Ok(id)
        });
    match saved {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(error) => share_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

// ---- Sharing (plan M8.4a, docs/design/82-library.md §8) --------------------

#[derive(serde::Deserialize)]
struct ShareDraft {
    name: String,
    role: vak_core::grants::Role,
    #[serde(default = "default_share_hours")]
    expires_in_hours: u32,
    /// Show earlier versions from this one on; absent shows only the
    /// current version.
    #[serde(default)]
    history_from: Option<String>,
}

fn default_share_hours() -> u32 {
    7 * 24
}

fn share_error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// `POST /library/{id}/shares`: a link that opens this artifact, and only
/// it, to one person in one role. Sharing breaks inheritance, so the
/// artifact's grants are then its whole audience besides its owner.
async fn share(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(draft): Json<ShareDraft>,
) -> Response {
    let Some(artifact) = state.core.artifacts().get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let name = draft.name.trim();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return share_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "name the person, in 1 to 80 characters",
        );
    }
    if !(1..=30 * 24).contains(&draft.expires_in_hours) {
        return share_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "a share lasts 1 hour to 30 days",
        );
    }
    if let Some(from) = &draft.history_from
        && !artifact
            .versions
            .iter()
            .any(|version| version.id.to_string() == *from)
    {
        return share_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "that version is not one of this artifact's",
        );
    }
    let token = crate::coworking::generate_token();
    let now = chrono::Utc::now();
    let mut capabilities = vec!["read".to_string()];
    if draft.role >= vak_core::grants::Role::Commenter {
        capabilities.push("comment".into());
    }
    if draft.role >= vak_core::grants::Role::Editor {
        capabilities.push("edit".into());
    }
    let grant = vak_core::grants::Grant {
        id: vak_session::ids::GrantId::new(),
        principal: vak_session::ids::PrincipalId::new().to_string(),
        display_name: name.to_string(),
        object: vak_core::grants::GrantObject::Artifact(artifact.id),
        role: draft.role,
        audience_id: Some(format!("artifact:{}", artifact.id)),
        capabilities,
        token_hash: Some(vak_core::grants::token_hash(&token)),
        created_at: now,
        expires_at: Some(now + chrono::Duration::hours(i64::from(draft.expires_in_hours))),
        history_from: draft.history_from,
    };
    let grants = crate::coworking::grants(&state);
    let actor = Some(crate::request_actor(&state));
    let object = grant.object.clone();
    if let Err(error) = grants.grant(grant.clone(), actor, None) {
        return share_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    if grants.inherits(&object).unwrap_or(true)
        && let Err(error) = grants.break_inheritance(object, actor)
    {
        return share_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string());
    }
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "share": share_summary(&grant, None, now), "token": token })),
    )
        .into_response()
}

fn share_summary(
    grant: &vak_core::grants::Grant,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> serde_json::Value {
    let held = vak_core::grants::Held {
        grant: grant.clone(),
        revoked_at,
    };
    serde_json::json!({
        "id": grant.id,
        "name": grant.display_name,
        "role": grant.role,
        "created_at": grant.created_at,
        "expires_at": grant.expires_at,
        "history_from": grant.history_from,
        "status": held.status(now),
    })
}

/// `GET /library/{id}/shares`: who it is shared with, never their tokens.
async fn shares(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Ok(artifact) = vak_session::ids::ArtifactId::parse(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let now = chrono::Utc::now();
    match crate::coworking::grants(&state).on(&vak_core::grants::GrantObject::Artifact(artifact)) {
        Ok(held) => Json(serde_json::json!({
            "shares": held
                .iter()
                .filter(|held| held.grant.token_hash.is_some())
                .map(|held| share_summary(&held.grant, held.revoked_at, now))
                .collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => share_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

/// `DELETE /library/{id}/shares/{grant}`: the link stops working at once.
async fn unshare(
    State(state): State<AppState>,
    Path((id, grant)): Path<(String, String)>,
) -> Response {
    let (Ok(artifact), Ok(grant)) = (
        vak_session::ids::ArtifactId::parse(&id),
        vak_session::ids::GrantId::parse(&grant),
    ) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let grants = crate::coworking::grants(&state);
    let ours = grants
        .on(&vak_core::grants::GrantObject::Artifact(artifact))
        .is_ok_and(|held| held.iter().any(|held| held.grant.id == grant));
    if !ours {
        return StatusCode::NOT_FOUND.into_response();
    }
    match grants.revoke(grant, Some(crate::request_actor(&state))) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => share_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

/// A comment as a person writes it: its text and, when it points
/// somewhere, lines of a text file or an anchor in an Office file or PDF.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct NewComment {
    pub text: String,
    #[serde(default)]
    pub line_start: Option<u32>,
    #[serde(default)]
    pub line_end: Option<u32>,
    #[serde(default)]
    pub anchor: Option<String>,
}

/// Records one comment in the version's thread: the one thread a version
/// has, whoever writes in it (plan M8.4c-b).
fn comment(
    state: &AppState,
    artifact: &vak_core::artifacts::Artifact,
    version: &str,
    draft: &NewComment,
    author: String,
    author_name: String,
) -> Response {
    let text = draft.text.trim();
    if text.is_empty() || text.chars().count() > 4000 {
        return share_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "a comment is 1 to 4000 characters",
        );
    }
    let Some(version) = artifact
        .versions
        .iter()
        .find(|known| known.id.to_string() == version)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let anchor = draft
        .anchor
        .as_deref()
        .map(str::trim)
        .filter(|anchor| !anchor.is_empty());
    let document = crate::is_document_path(&artifact.path);
    let refusal = if draft.line_start == Some(0)
        || draft.line_end == Some(0)
        || (draft.line_end.is_some() && draft.line_start.is_none())
        || matches!((draft.line_start, draft.line_end), (Some(start), Some(end)) if end < start)
    {
        Some("lines are counted from 1, and a range ends at or after its start")
    } else if document && draft.line_start.is_some() {
        Some(
            "line numbers mean nothing in an Office file or PDF; point at a cell, paragraph, slide or PDF line with anchor",
        )
    } else if anchor.is_some() && !document {
        Some("an anchor points into an Office file or PDF; use line numbers for a text file")
    } else if anchor.is_some_and(|anchor| !crate::is_document_anchor(&artifact.path, anchor)) {
        Some("anchor is not a cell, paragraph, slide, shape or PDF page or line anchor")
    } else {
        None
    };
    if let Some(message) = refusal {
        return share_error(StatusCode::BAD_REQUEST, message);
    }
    let id = vak_session::ids::CommentId::new();
    match state.core.artifacts().record(
        artifact.id,
        ArtifactStep::Commented {
            version: version.id,
            author,
            author_name,
            text: text.to_string(),
            id: Some(id),
            at_place: vak_core::artifacts::Place {
                line_start: draft.line_start,
                line_end: draft.line_end,
                anchor: anchor.map(str::to_string),
            },
        },
        None,
        None,
    ) {
        Ok(()) => {
            // People watching the conversation this version is reviewed in
            // see the thread change.
            if let Some(proposal) = version.proposal()
                && let Some(handle) = state.get(&proposal.session)
            {
                let _ = handle.coworking_comments_tx.send(());
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!({ "comment_id": id })),
            )
                .into_response()
        }
        Err(error) => share_error(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()),
    }
}

/// Who a request speaks as on a version, in the role it needs: the owner,
/// or a guest of a conversation the version is reviewed in. The guest's
/// conversation grant reaches the artifact by inheritance, decided by the
/// grants (`Grants::may`): an artifact that broke inheritance, as sharing
/// it by itself does, is no longer reached through the conversation, and a
/// grant that ended or is too weak reaches nothing. An artifact share's
/// guest uses `/shared/artifact`.
fn discusser(
    state: &AppState,
    principal: &crate::AuthenticatedPrincipal,
    artifact: &vak_core::artifacts::Artifact,
    version: &vak_core::artifacts::Version,
    role: vak_core::grants::Role,
) -> Result<(String, String), StatusCode> {
    match principal {
        crate::AuthenticatedPrincipal::Operator => {
            Ok((crate::request_actor(state).to_string(), "You".into()))
        }
        crate::AuthenticatedPrincipal::Participant(guest) => {
            use vak_core::grants::GrantObject;
            let grants = crate::coworking::grants(state);
            let now = chrono::Utc::now();
            let reviewed_there = version.proposed.iter().any(|proposal| {
                proposal.session == guest.conversation_id
                    && crate::conversation_audience(state, &proposal.session).as_deref()
                        == Some(guest.audience_id.as_str())
            });
            let in_conversation = reviewed_there
                && grants.may(
                    &guest.principal_id,
                    &GrantObject::Conversation(guest.conversation_id.clone()),
                    role,
                    false,
                    now,
                );
            if grants.may(
                &guest.principal_id,
                &GrantObject::Artifact(artifact.id),
                role,
                in_conversation,
                now,
            ) {
                Ok((guest.principal_id.clone(), guest.display_name.clone()))
            } else {
                Err(StatusCode::FORBIDDEN)
            }
        }
        crate::AuthenticatedPrincipal::ArtifactGuest(_) => Err(StatusCode::FORBIDDEN),
    }
}

fn thread_entry(
    artifact: &vak_core::artifacts::Artifact,
    comment: &vak_core::artifacts::Comment,
) -> serde_json::Value {
    serde_json::json!({
        "comment_id": comment.id,
        "version": comment.version,
        "actor_id": comment.author,
        "actor_name": comment.author_name,
        "text": comment.text,
        "path": artifact.path,
        "line_start": comment.at_place.line_start,
        "line_end": comment.at_place.line_end,
        "anchor": comment.at_place.anchor,
        "created_at": comment.at,
    })
}

fn version_of<'a>(
    artifact: &'a vak_core::artifacts::Artifact,
    version: &str,
) -> Option<&'a vak_core::artifacts::Version> {
    artifact
        .versions
        .iter()
        .find(|known| known.id.to_string() == version)
}

/// `GET /library/{id}/versions/{version}/comments`: the version's thread.
pub(crate) async fn thread(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
) -> Response {
    let Some(artifact) = state.core.artifacts().get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(found) = version_of(&artifact, &version) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Err(status) = discusser(
        &state,
        &principal,
        &artifact,
        found,
        vak_core::grants::Role::Viewer,
    ) {
        return status.into_response();
    }
    let comments: Vec<_> = artifact
        .comments
        .iter()
        .filter(|comment| comment.version == found.id)
        .map(|comment| thread_entry(&artifact, comment))
        .collect();
    Json(serde_json::json!({ "comments": comments })).into_response()
}

/// `POST /library/{id}/versions/{version}/comments`: a comment in the
/// version's thread, by the owner or a guest of its conversation who may
/// comment.
pub(crate) async fn add_comment(
    State(state): State<AppState>,
    Path((id, version)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
    Json(draft): Json<NewComment>,
) -> Response {
    let Some(artifact) = state.core.artifacts().get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(found) = version_of(&artifact, &version) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (author, author_name) = match discusser(
        &state,
        &principal,
        &artifact,
        found,
        vak_core::grants::Role::Commenter,
    ) {
        Ok(who) => who,
        Err(status) => return status.into_response(),
    };
    comment(&state, &artifact, &version, &draft, author, author_name)
}

/// Where a comment points, as a revision request states it: an Office
/// anchor (`Budget!B4`) or a line range, after the file.
fn comment_location(path: &str, place: &vak_core::artifacts::Place) -> String {
    match (&place.anchor, place.line_start, place.line_end) {
        (Some(anchor), _, _) => format!(" file {path}, at {anchor}"),
        (None, Some(start), Some(end)) => format!(" file {path}, lines {start}-{end}"),
        (None, Some(start), None) => format!(" file {path}, line {start}"),
        (None, None, _) => format!(" file {path}"),
    }
}

/// `POST /library/{id}/comments/{comment}/revise`: the owner asks Vak to
/// revise the version a comment is on. The revision is a new version made
/// from that one, and it waits for Review like any other.
pub(crate) async fn revise(
    State(state): State<AppState>,
    Path((id, comment)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
) -> Response {
    if !matches!(principal, crate::AuthenticatedPrincipal::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(artifact) = state.core.artifacts().get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(found) = artifact
        .comments
        .iter()
        .find(|known| known.id.is_some_and(|known| known.to_string() == comment))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let review = match in_review(&state, &id, &found.version.to_string()) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    let saved = match crate::saved_candidate(&state, &review.session, &review.candidate) {
        Ok(saved) => saved,
        Err(status) => return status.into_response(),
    };
    let prompt = format!(
        "Revise candidate {} for result {}{}. Owner selected comment {comment} by {} as feedback: {}",
        review.candidate,
        saved.result_id,
        comment_location(&review.path, &found.at_place),
        found.author_name,
        found.text,
    );
    crate::dispatch_candidate_revision(state, saved, comment, prompt).await
}

#[derive(serde::Deserialize)]
struct SharedComment {
    version: String,
    text: String,
}

/// The grant a guest request carries, and its artifact while the grant
/// holds.
fn guest(
    state: &AppState,
    principal: &crate::AuthenticatedPrincipal,
) -> Result<(vak_core::grants::Grant, vak_core::artifacts::Artifact), StatusCode> {
    let crate::AuthenticatedPrincipal::ArtifactGuest(grant) = principal else {
        return Err(StatusCode::FORBIDDEN);
    };
    let vak_core::grants::GrantObject::Artifact(id) = &grant.object else {
        return Err(StatusCode::FORBIDDEN);
    };
    let artifact = state
        .core
        .artifacts()
        .get(&id.to_string())
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((*grant.clone(), artifact))
}

/// The versions a share shows: the current one, and earlier ones only
/// from the version the owner chose.
fn shown(
    grant: &vak_core::grants::Grant,
    artifact: &vak_core::artifacts::Artifact,
) -> Vec<(usize, vak_core::artifacts::Version)> {
    let head = artifact.head().map(|version| version.id);
    let from = grant.history_from.as_ref().and_then(|from| {
        artifact
            .versions
            .iter()
            .position(|version| version.id.to_string() == *from)
    });
    artifact
        .versions
        .iter()
        .enumerate()
        .filter(|(index, version)| {
            Some(version.id) == head || from.is_some_and(|from| *index >= from)
        })
        .map(|(index, version)| (index + 1, version.clone()))
        .collect()
}

/// `GET /shared/artifact`: what the share shows. Never conversation text,
/// conversation ids or the owner's comments.
async fn shared_view(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
) -> Response {
    let (grant, artifact) = match guest(&state, &principal) {
        Ok(found) => found,
        Err(status) => return status.into_response(),
    };
    let versions = shown(&grant, &artifact);
    let ids: Vec<_> = versions.iter().map(|(_, version)| version.id).collect();
    Json(serde_json::json!({
        "name": artifact.name(),
        "kind": artifact.kind,
        "file_name": std::path::Path::new(&artifact.path).file_name().map(|name| name.to_string_lossy().into_owned()),
        "role": grant.role,
        "you": grant.display_name,
        "versions": versions.iter().map(|(number, version)| serde_json::json!({
            "id": version.id,
            "number": number,
            "by": if matches!(version.source, VersionSource::Person) { "the owner" } else { "Vakyartha" },
            "at": version.at,
            "size": version.size,
        })).collect::<Vec<_>>(),
        "comments": artifact.comments.iter()
            .filter(|comment| ids.contains(&comment.version) && comment.author != crate::request_actor(&state).to_string())
            .map(|comment| serde_json::json!({
                "version": comment.version, "name": comment.author_name, "text": comment.text, "at": comment.at,
            }))
            .collect::<Vec<_>>(),
    }))
    .into_response()
}

/// `GET /shared/artifact/versions/{version}`: a shown version's bytes.
async fn shared_bytes(
    State(state): State<AppState>,
    Path(version): Path<String>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
) -> Response {
    let (grant, artifact) = match guest(&state, &principal) {
        Ok(found) => found,
        Err(status) => return status.into_response(),
    };
    let Some((_, found)) = shown(&grant, &artifact)
        .into_iter()
        .find(|(_, shown)| shown.id.to_string() == version)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state.core.artifacts().bytes(&artifact, &found.id) {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::CONTENT_DISPOSITION, "attachment".to_string()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `POST /shared/artifact/comments`: a commenter or editor comments on a
/// shown version.
async fn shared_comment(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<crate::AuthenticatedPrincipal>,
    Json(draft): Json<SharedComment>,
) -> Response {
    let (grant, artifact) = match guest(&state, &principal) {
        Ok(found) => found,
        Err(status) => return status.into_response(),
    };
    if grant.role < vak_core::grants::Role::Commenter {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !shown(&grant, &artifact)
        .iter()
        .any(|(_, version)| version.id.to_string() == draft.version)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    comment(
        &state,
        &artifact,
        &draft.version,
        &NewComment {
            text: draft.text,
            ..NewComment::default()
        },
        grant.principal.clone(),
        grant.display_name.clone(),
    )
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/library", get(list))
        .route("/library/{id}", get(get_one))
        .route("/library/{id}/versions/{version}", get(version_bytes))
        .route("/library/{id}/versions/{version}/save", post(save))
        .route("/library/{id}/versions/{version}/text", get(version_text))
        .route("/library/{id}/versions/{version}/raw", get(version_raw))
        .route(
            "/library/{id}/versions/{version}/document",
            get(version_document),
        )
        .route(
            "/library/{id}/versions/{version}/review",
            get(version_review),
        )
        .route(
            "/library/{id}/versions/{version}/narrow",
            post(version_narrow),
        )
        .route(
            "/library/{id}/versions/{version}/accept",
            post(version_accept),
        )
        .route("/library/{id}/versions/{version}/undo", post(version_undo))
        .route(
            "/library/{id}/versions/{version}/checks",
            post(version_check),
        )
        .route("/library/cards", post(save_card))
        .route("/library/{id}/versions", post(person_version))
        .route("/library/{id}/shares", get(shares).post(share))
        .route(
            "/library/{id}/shares/{grant}",
            axum::routing::delete(unshare),
        )
        .route(
            "/library/{id}/versions/{version}/comments",
            get(thread).post(add_comment),
        )
        .route("/library/{id}/comments/{comment}/revise", post(revise))
        .route("/library/{id}/{action}", post(change))
        .route("/shared/artifact", get(shared_view))
        .route("/shared/artifact/versions/{version}", get(shared_bytes))
        .route("/shared/artifact/comments", post(shared_comment))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_revision_request_names_the_cell_or_lines_a_comment_points_at() {
        let place = |line_start, line_end, anchor: Option<&str>| vak_core::artifacts::Place {
            line_start,
            line_end,
            anchor: anchor.map(str::to_string),
        };
        assert_eq!(
            comment_location("budget.xlsx", &place(None, None, Some("'Q4 plan'!B4"))),
            " file budget.xlsx, at 'Q4 plan'!B4"
        );
        assert_eq!(
            comment_location("a.txt", &place(Some(3), Some(5), None)),
            " file a.txt, lines 3-5"
        );
        assert_eq!(
            comment_location("a.txt", &place(Some(3), None, None)),
            " file a.txt, line 3"
        );
        assert_eq!(
            comment_location("a.txt", &place(None, None, None)),
            " file a.txt"
        );
    }
    use vak_core::artifacts::ArtifactKind;

    #[test]
    fn an_attached_artifact_is_told_from_its_records_and_only_in_its_place() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        vak_config::spaces::bind(dir.path()).unwrap();
        let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
        let space = vak_session::trace::local::space(core.cwd()).to_string();
        let artifacts = core.artifacts();
        let id = artifacts
            .declare(
                &space,
                "vak",
                "notes/plan.md",
                ArtifactKind::File,
                Some("The plan".into()),
                None,
                None,
                None,
            )
            .unwrap();
        artifacts
            .version(
                id,
                NewVersion {
                    parent: None,
                    bytes: b"secret conversation words stay out",
                    source: VersionSource::Call {
                        session: "01920000-0000-7000-8000-00000000aaaa".into(),
                        call: "toolu_1".into(),
                    },
                },
                None,
                None,
            )
            .unwrap();

        let (block, attached) = attach(
            &core,
            &ArtifactRef {
                id: id.to_string(),
                mode: vak_session::ArtifactMode::Continue,
            },
        )
        .unwrap();
        assert!(block.contains("\"The plan\""), "{block}");
        assert!(block.contains("notes/plan.md"));
        assert!(block.contains("version 1 of 1"));
        assert!(block.contains("conversation 01920000-0000-7000-8000-00000000aaaa"));
        assert!(
            !block.contains("secret conversation words"),
            "no content, only records"
        );
        assert_eq!(
            attached.conversations,
            vec!["01920000-0000-7000-8000-00000000aaaa".to_string()]
        );
        assert_eq!(attached.path, "notes/plan.md");

        let (another, _) = attach(
            &core,
            &ArtifactRef {
                id: id.to_string(),
                mode: vak_session::ArtifactMode::Another,
            },
        )
        .unwrap();
        assert!(another.contains("do not change notes/plan.md"), "{another}");

        // Another Agent's artifact, or another workspace's, is refused.
        let theirs = artifacts
            .declare(
                &space,
                "scout",
                "theirs.md",
                ArtifactKind::File,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let elsewhere = artifacts
            .declare(
                "spc_elsewhere",
                "vak",
                "far.md",
                ArtifactKind::File,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        for refused in [theirs, elsewhere] {
            assert!(
                attach(
                    &core,
                    &ArtifactRef {
                        id: refused.to_string(),
                        mode: Default::default(),
                    },
                )
                .is_err()
            );
        }
    }
}
