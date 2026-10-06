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

/// Records what a Review record means for the artifacts it touches: each
/// file of a candidate that is already a declared deliverable gets the
/// candidate's bytes as a version, and a promotion marks the versions of
/// its candidate promoted. A file nobody declared stays out (doc 82 §3).
pub(crate) fn note_review(state: &AppState, record: &vak_sandbox::DurableRecord) {
    let artifacts = state.core.artifacts();
    match record {
        vak_sandbox::DurableRecord::Candidate(candidate) => {
            let manifest = &candidate.candidate;
            let space = vak_session::trace::local::space(&manifest.destination_root).to_string();
            for file in &manifest.files {
                if file.operation == vak_sandbox::CandidateOperation::Delete {
                    continue;
                }
                let id = vak_core::artifacts::artifact_id(&space, &file.path);
                if artifacts.get(&id.to_string()).is_none() {
                    continue;
                }
                let Ok(bytes) = std::fs::read(manifest.source_root.join(&file.path)) else {
                    continue;
                };
                if let Err(error) = artifacts.version(
                    id,
                    NewVersion {
                        parent: None,
                        bytes: &bytes,
                        source: VersionSource::Candidate {
                            session: candidate.session_id.clone(),
                            candidate: manifest.candidate_id.clone(),
                        },
                    },
                    candidate.trace.as_ref(),
                    candidate.actor,
                ) {
                    tracing::warn!(kind = "artifact", error_kind = %vak_telemetry::error_kind(&error), "a candidate was not recorded as a version");
                }
            }
        }
        vak_sandbox::DurableRecord::Promotion(promotion) => {
            for artifact in artifacts.list() {
                for version in &artifact.versions {
                    let of_candidate = matches!(&version.source,
                        VersionSource::Candidate { candidate, .. } if *candidate == promotion.candidate_id);
                    if of_candidate
                        && !version.promoted
                        && let Err(error) = artifacts.record(
                            artifact.id,
                            ArtifactStep::Promoted {
                                version: version.id,
                            },
                            promotion.trace.as_ref(),
                            promotion.actor,
                        )
                    {
                        tracing::warn!(kind = "artifact", error_kind = %vak_telemetry::error_kind(&error), "a promotion was not recorded on its version");
                    }
                }
            }
        }
        _ => {}
    }
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
    let read = tokio::task::spawn_blocking(move || {
        let artifact = artifacts.get(&id)?;
        let bytes = artifacts.bytes(&artifact, &version).ok()?;
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

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/library", get(list))
        .route("/library/{id}", get(get_one))
        .route("/library/{id}/versions/{version}", get(version_bytes))
        .route("/library/{id}/versions/{version}/save", post(save))
        .route("/library/{id}/{action}", post(change))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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
