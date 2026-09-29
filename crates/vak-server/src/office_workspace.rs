//! Shared, versioned editing rooms for one Office file or PDF in a saved
//! candidate.
//!
//! Room metadata is a replaceable snapshot for fast reads; its revisions are
//! append-only entries, each pointing at an immutable saved candidate. Office
//! bytes are always produced and checked by the document worker.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
};

use crate::{AppState, AuthenticatedPrincipal};

const MAX_OPS: usize = 32;
const MAX_OPERATION_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OfficeRoom {
    schema: u32,
    room_id: String,
    session_id: String,
    path: String,
    created_at: String,
    branches: Vec<OfficeBranch>,
    revisions: Vec<OfficeRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OfficeBranch {
    branch_id: String,
    name: String,
    base_candidate_id: String,
    head_candidate_id: String,
    shared: bool,
    archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OfficeRevision {
    candidate_id: String,
    parent_candidate_id: String,
    #[serde(default)]
    merge_parent_candidate_id: Option<String>,
    branch_id: String,
    author_id: String,
    author_name: String,
    created_at: String,
    ops: Vec<vak_ooxml::edit::OfficeOp>,
    /// A PDF room's ops, one step per call: every anchor in a step names the
    /// file that step started from (docs/design/77).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pdf_steps: Vec<Vec<vak_pdf::edit::PdfOp>>,
}

/// One edit's ops, typed by the kind of file the room holds.
enum Edits {
    Office(Vec<vak_ooxml::edit::OfficeOp>),
    Pdf(Vec<Vec<vak_pdf::edit::PdfOp>>),
}

impl Edits {
    fn parse(path: &str, ops: Vec<serde_json::Value>) -> Option<Edits> {
        let ops = serde_json::Value::Array(ops);
        if vak_pdf::is_pdf_path(path) {
            serde_json::from_value(ops)
                .ok()
                .map(|ops| Edits::Pdf(vec![ops]))
        } else {
            serde_json::from_value(ops).ok().map(Edits::Office)
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateBody {
    candidate_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Action {
    Branch {
        name: String,
        expected_head: String,
    },
    Edit {
        branch_id: String,
        expected_head: String,
        /// Typed by the room's file: Office ops, or PDF ops.
        ops: Vec<serde_json::Value>,
    },
    Merge {
        branch_id: String,
        expected_shared_head: String,
    },
    Import {
        branch_id: String,
        expected_head: String,
        candidate_id: String,
    },
}

#[derive(Debug, Deserialize)]
pub(super) struct FocusBody {
    anchor: Option<String>,
    #[serde(default = "default_focus_active")]
    active: bool,
}

fn default_focus_active() -> bool {
    true
}

fn id_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn room_path(state: &AppState, session_id: &str, room_id: &str) -> Option<PathBuf> {
    if !id_ok(session_id) || !id_ok(room_id) {
        return None;
    }
    Some(
        vak_config::paths::office_workspaces_at(&state.core.sessions_home(), session_id)
            .join(format!("{room_id}.json")),
    )
}

fn load(path: &FsPath) -> Result<OfficeRoom, ()> {
    let bytes = fs::read(path).map_err(|_| ())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(());
    }
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn save(path: &FsPath, room: &OfficeRoom) -> Result<(), ()> {
    let parent = path.parent().ok_or(())?;
    fs::create_dir_all(parent).map_err(|_| ())?;
    let temp = parent.join(format!(".{}.{}.tmp", room.room_id, uuid::Uuid::now_v7()));
    let bytes = serde_json::to_vec(room).map_err(|_| ())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(());
    }
    let result = (|| {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok::<(), std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.map_err(|_| ())
}

fn authority(
    state: &AppState,
    session_id: &str,
    principal: &AuthenticatedPrincipal,
    edit: bool,
) -> Result<(String, String, String), StatusCode> {
    match principal {
        AuthenticatedPrincipal::Operator => Ok(("operator".into(), "You".into(), "owner".into())),
        AuthenticatedPrincipal::Participant(p) => {
            let audience =
                super::conversation_audience(state, session_id).ok_or(StatusCode::NOT_FOUND)?;
            if p.conversation_id != session_id
                || p.audience_id != audience
                || !p.capabilities.iter().any(|c| c == "read")
            {
                return Err(StatusCode::FORBIDDEN);
            }
            if edit && !p.capabilities.iter().any(|c| c == "edit") {
                return Err(StatusCode::FORBIDDEN);
            }
            Ok((
                p.principal_id.clone(),
                p.display_name.clone(),
                p.grant_id.clone(),
            ))
        }
    }
}

pub(super) async fn create(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Json(body): Json<CreateBody>,
) -> axum::response::Response {
    if !matches!(principal, AuthenticatedPrincipal::Operator) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !super::is_document_path(&body.path) {
        return (
            StatusCode::BAD_REQUEST,
            "Choose a Word, Excel, PowerPoint, Visio or PDF file.",
        )
            .into_response();
    }
    let saved = match super::saved_candidate(&state, &session_id, &body.candidate_id) {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    if !saved
        .candidate
        .files
        .iter()
        .any(|f| f.path == body.path && f.operation == vak_sandbox::CandidateOperation::Upsert)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(status) =
        super::sandbox_candidate_file_bytes(&state, &session_id, &body.candidate_id, &body.path)
            .await
    {
        return status.into_response();
    }
    let room_id = uuid::Uuid::now_v7().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let room = OfficeRoom {
        schema: 1,
        room_id: room_id.clone(),
        session_id: session_id.clone(),
        path: body.path,
        created_at: now.clone(),
        branches: vec![OfficeBranch {
            branch_id: "shared".into(),
            name: "Shared draft".into(),
            base_candidate_id: body.candidate_id.clone(),
            head_candidate_id: body.candidate_id.clone(),
            shared: true,
            archived: false,
        }],
        revisions: vec![OfficeRevision {
            candidate_id: body.candidate_id,
            parent_candidate_id: String::new(),
            merge_parent_candidate_id: None,
            branch_id: "shared".into(),
            author_id: "operator".into(),
            author_name: "You".into(),
            created_at: now,
            ops: Vec::new(),
            pdf_steps: Vec::new(),
        }],
    };
    let Some(path) = room_path(&state, &session_id, &room_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if save(&path, &room).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Json(room).into_response()
}

pub(super) async fn list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
) -> axum::response::Response {
    if !id_ok(&session_id) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if let Err(status) = authority(&state, &session_id, &principal, false) {
        return status.into_response();
    }
    let root = vak_config::paths::office_workspaces_at(&state.core.sessions_home(), &session_id);
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Json(serde_json::json!({"workspaces": []})).into_response();
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let mut rooms = Vec::new();
    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|e| e == "json") {
            match load(&entry.path()) {
                Ok(room) if room.session_id == session_id => rooms.push(room),
                Ok(_) => return StatusCode::FORBIDDEN.into_response(),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
    }
    rooms.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Json(serde_json::json!({"workspaces": rooms})).into_response()
}

pub(super) async fn focus(
    State(state): State<AppState>,
    Path((session_id, room_id)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Json(body): Json<FocusBody>,
) -> axum::response::Response {
    let (principal_id, display_name, _) = match authority(&state, &session_id, &principal, false) {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    let Some(path) = room_path(&state, &session_id, &room_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let room = match load(&path) {
        Ok(room) if room.session_id == session_id => room,
        Ok(_) => return StatusCode::FORBIDDEN.into_response(),
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if body
        .anchor
        .as_deref()
        .is_some_and(|anchor| anchor.len() > 256 || !super::is_document_anchor(&room.path, anchor))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    super::touch_coworking_presence(&state, &session_id, &principal_id, &display_name);
    if let Ok(mut all) = state.coworking_presence.lock()
        && let Some(presence) = all
            .get_mut(&session_id)
            .and_then(|people| people.get_mut(&principal_id))
    {
        presence.office_room_id = body.active.then(|| room_id.clone());
        presence.office_anchor = body.active.then_some(body.anchor).flatten();
        presence.seen_at = std::time::Instant::now();
    }
    if let Some(handle) = state.get(&session_id) {
        let _ = handle.coworking_comments_tx.send(());
    }
    Json(serde_json::json!({"ok":true,"room_id":room.room_id})).into_response()
}

pub(super) async fn mutate(
    State(state): State<AppState>,
    Path((session_id, room_id)): Path<(String, String)>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Json(action): Json<Action>,
) -> axum::response::Response {
    let (actor_id, actor_name, _grant) = match authority(&state, &session_id, &principal, true) {
        Ok(v) => v,
        Err(s) => return s.into_response(),
    };
    let Some(path) = room_path(&state, &session_id, &room_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let lock_path = path.with_extension("lock");
    if let Some(parent) = lock_path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let lock = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if lock.try_lock().is_err() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":"This draft is being updated; try again shortly."})),
        )
            .into_response();
    }
    let mut room = match load(&path) {
        Ok(r) if r.session_id == session_id && r.room_id == room_id => r,
        Ok(_) => return StatusCode::FORBIDDEN.into_response(),
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let (branch_id, current_head) = match &action {
        Action::Branch { expected_head, .. } => ("shared", expected_head),
        Action::Edit {
            branch_id,
            expected_head,
            ..
        }
        | Action::Import {
            branch_id,
            expected_head,
            ..
        } => (branch_id.as_str(), expected_head),
        Action::Merge {
            branch_id: _,
            expected_shared_head,
        } => ("shared", expected_shared_head),
    };
    let Some(branch_pos) = room
        .branches
        .iter()
        .position(|b| b.branch_id == branch_id && !b.archived)
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if room.branches[branch_pos].head_candidate_id != *current_head {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"This version changed. Reload the draft before editing."}))).into_response();
    }
    match action {
        Action::Branch { name, .. } => {
            let name = name.trim();
            if name.is_empty()
                || name.chars().count() > 80
                || name.chars().any(char::is_control)
                || room.branches.len() >= 24
            {
                return StatusCode::BAD_REQUEST.into_response();
            }
            let branch_id = uuid::Uuid::now_v7().to_string();
            let base = room.branches[branch_pos].head_candidate_id.clone();
            room.branches.push(OfficeBranch {
                branch_id: branch_id.clone(),
                name: name.into(),
                base_candidate_id: base.clone(),
                head_candidate_id: base,
                shared: false,
                archived: false,
            });
            if save(&path, &room).is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            Json(serde_json::json!({"workspace":room})).into_response()
        }
        Action::Edit { branch_id, ops, .. } => {
            if ops.is_empty()
                || ops.len() > MAX_OPS
                || serde_json::to_vec(&ops).map_or(true, |b| b.len() > MAX_OPERATION_BYTES)
            {
                return StatusCode::BAD_REQUEST.into_response();
            }
            let Some(edits) = Edits::parse(&room.path, ops) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            return create_revision(
                &state, &path, room, branch_pos, branch_id, actor_id, actor_name, edits, None,
            )
            .await;
        }
        Action::Import {
            branch_id,
            candidate_id,
            ..
        } => {
            let imported = match super::saved_candidate(&state, &session_id, &candidate_id) {
                Ok(c) => c,
                Err(s) => return s.into_response(),
            };
            if !imported.candidate.files.iter().any(|f| {
                f.path == room.path && f.operation == vak_sandbox::CandidateOperation::Upsert
            }) {
                return StatusCode::BAD_REQUEST.into_response();
            }
            let parent_id = room.branches[branch_pos].head_candidate_id.clone();
            let imported_parent_id = match imported.parent_candidate_id.as_deref() {
                Some(id) => id.to_string(),
                None => {
                    return (
                        StatusCode::CONFLICT,
                        "This saved version has no recorded base to compare against.",
                    )
                        .into_response();
                }
            };
            let base_bytes = match super::sandbox_candidate_file_bytes(
                &state,
                &session_id,
                &imported_parent_id,
                &room.path,
            )
            .await
            {
                Ok(b) => b,
                Err(s) => return s.into_response(),
            };
            let head_bytes = match super::sandbox_candidate_file_bytes(
                &state,
                &session_id,
                &parent_id,
                &room.path,
            )
            .await
            {
                Ok(b) => b,
                Err(s) => return s.into_response(),
            };
            let base_hash = vak_sandbox::digest(&base_bytes);
            let head_hash = vak_sandbox::digest(&head_bytes);
            if base_hash != head_hash {
                return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"The Agent version starts from another document version. Review it separately or start a branch from that version."}))).into_response();
            }
            let lineage = match super::office_lineage(
                &state,
                &session_id,
                &imported.execution_id,
                &room.path,
            ) {
                Ok(lineage) => lineage,
                Err(reason) => {
                    return (
                        StatusCode::CONFLICT,
                        Json(serde_json::json!({"error":reason})),
                    )
                        .into_response();
                }
            };
            if let Err(status) =
                super::sandbox_candidate_file_bytes(&state, &session_id, &candidate_id, &room.path)
                    .await
            {
                return status.into_response();
            }
            let branch = &mut room.branches[branch_pos];
            branch.head_candidate_id = candidate_id.clone();
            room.revisions.push(OfficeRevision {
                candidate_id,
                parent_candidate_id: parent_id,
                merge_parent_candidate_id: None,
                branch_id,
                author_id: actor_id,
                author_name: actor_name,
                created_at: chrono::Utc::now().to_rfc3339(),
                ops: lineage.ops,
                pdf_steps: lineage.pdf,
            });
            if save(&path, &room).is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            Json(serde_json::json!({"workspace":room})).into_response()
        }
        Action::Merge { branch_id, .. } => {
            let shared_head = room.branches[branch_pos].head_candidate_id.clone();
            let Some(other_pos) = room
                .branches
                .iter()
                .position(|b| b.branch_id == branch_id && !b.shared && !b.archived)
            else {
                return StatusCode::NOT_FOUND.into_response();
            };
            let source_head = room.branches[other_pos].head_candidate_id.clone();
            if source_head == shared_head {
                return Json(room).into_response();
            }
            let base = room.branches[other_pos].base_candidate_id.clone();
            if vak_pdf::is_pdf_path(&room.path) {
                let shared_steps = pdf_steps_since(&room, &shared_head, &base);
                let branch_steps = pdf_steps_since(&room, &source_head, &base);
                let (Some(shared_steps), Some(branch_steps)) = (shared_steps, branch_steps) else {
                    return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"These versions no longer share a mergeable base."}))).into_response();
                };
                if pdf_conflict(&shared_steps, &branch_steps) {
                    return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"Both versions changed the same pages, or one moved pages the other edited. Review them separately; this merge needs a manual edit."}))).into_response();
                }
                return create_revision(
                    &state,
                    &path,
                    room,
                    branch_pos,
                    "shared".into(),
                    actor_id,
                    actor_name,
                    Edits::Pdf(branch_steps),
                    Some(source_head),
                )
                .await;
            }
            let shared_ops = operations_since(&room, &shared_head, &base);
            let branch_ops = operations_since(&room, &source_head, &base);
            let (Some(shared_ops), Some(branch_ops)) = (shared_ops, branch_ops) else {
                return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"These versions no longer share a mergeable base."}))).into_response();
            };
            if has_conflict(&shared_ops, &branch_ops) {
                return (StatusCode::CONFLICT, Json(serde_json::json!({"error":"Both versions changed the same document area. Review them separately; this merge needs a manual edit."}))).into_response();
            }
            return create_revision(
                &state,
                &path,
                room,
                branch_pos,
                "shared".into(),
                actor_id,
                actor_name,
                Edits::Office(branch_ops),
                Some(source_head),
            )
            .await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn create_revision(
    state: &AppState,
    path: &FsPath,
    mut room: OfficeRoom,
    branch_pos: usize,
    branch_id: String,
    actor_id: String,
    actor_name: String,
    edits: Edits,
    merge_parent: Option<String>,
) -> axum::response::Response {
    let parent_id = room.branches[branch_pos].head_candidate_id.clone();
    let session_id = room.session_id.clone();
    let parent = match super::saved_candidate(state, &session_id, &parent_id) {
        Ok(c) => c,
        Err(s) => return s.into_response(),
    };
    let Some(parent_file) =
        parent.candidate.files.iter().find(|f| {
            f.path == room.path && f.operation == vak_sandbox::CandidateOperation::Upsert
        })
    else {
        return StatusCode::CONFLICT.into_response();
    };
    let id = uuid::Uuid::now_v7().to_string();
    let staging_root = state
        .core
        .sessions_home()
        .join("sandbox")
        .join("staging")
        .join(&id);
    if vak_sandbox::prepare_revision_copy(&parent.candidate, &staging_root).is_err() {
        return StatusCode::CONFLICT.into_response();
    }
    let Some(draft_path) = super::confined_path(&staging_root, &room.path) else {
        let _ = fs::remove_dir_all(&staging_root);
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(parent_source) = super::confined_path(&parent.candidate.source_root, &room.path)
    else {
        let _ = fs::remove_dir_all(&staging_root);
        return StatusCode::FORBIDDEN.into_response();
    };
    let Some(extension) = FsPath::new(&room.path).extension().and_then(|e| e.to_str()) else {
        let _ = fs::remove_dir_all(&staging_root);
        return StatusCode::BAD_REQUEST.into_response();
    };
    let output_path = draft_path.with_file_name(format!("vak-{id}.{extension}"));
    let lineage = vak_tools::broker::OfficeLineage {
        origin: vak_tools::broker::OfficeOrigin::File {
            path: parent_source,
            base_digest: parent_file.candidate_hash.clone(),
        },
        ops: match &edits {
            Edits::Office(ops) => ops.clone(),
            Edits::Pdf(_) => Vec::new(),
        },
        planned_ops: match &edits {
            Edits::Office(ops) => {
                vak_ooxml::edit::plan_chart_locations(ops).unwrap_or_else(|_| ops.clone())
            }
            Edits::Pdf(_) => Vec::new(),
        },
        author: actor_name.clone(),
        new_file: false,
        pdf: match &edits {
            Edits::Pdf(steps) => steps.clone(),
            Edits::Office(_) => Vec::new(),
        },
    };
    let applied = match vak_tools::broker::office_apply_to(
        &state.core.tool_worker_exe(),
        &lineage,
        &output_path,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging_root);
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error":e})),
            )
                .into_response();
        }
    };
    if fs::remove_file(&draft_path).is_err() || fs::rename(&output_path, &draft_path).is_err() {
        let _ = fs::remove_dir_all(&staging_root);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let frozen_root = super::sandbox_candidates_root(state).join(&id);
    let candidate = match vak_sandbox::freeze_revision_candidate(
        &id,
        &staging_root,
        &parent.candidate,
        &frozen_root,
    ) {
        Ok(c) => c,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging_root);
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error":e.to_string()})),
            )
                .into_response();
        }
    };
    let _ = fs::remove_dir_all(&staging_root);
    let mut candidate = candidate;
    candidate.target_checks = vak_sandbox::default_target_verifiers().plan(&candidate);
    candidate.workspace_checks = super::planned_workspace_checks(&candidate);
    let draft_checks = vak_tools::broker::verify_targets(
        &state.core.tool_worker_exe(),
        &candidate.source_root,
        &candidate.target_checks,
    )
    .await;
    let digest = match vak_sandbox::candidate_digest(&candidate) {
        Ok(d) => d,
        Err(_) => {
            let _ = vak_sandbox::remove_frozen_candidate(&frozen_root);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let saved = vak_sandbox::CandidateRecord {
        record_id: format!("candidate-{id}"),
        session_id: parent.session_id.clone(),
        turn_id: parent.turn_id.clone(),
        result_id: parent.result_id.clone(),
        execution_id: parent.execution_id.clone(),
        environment_id: "office-workspace".into(),
        candidate_digest: digest,
        candidate,
        verified: true,
        draft_checks,
        updated_at: chrono::Utc::now().to_rfc3339(),
        parent_candidate_id: Some(parent_id.clone()),
        revision_session_id: None,
        narrowed: None,
    };
    if vak_sandbox::append_record(
        &super::sandbox_records_path(state),
        &vak_sandbox::DurableRecord::Candidate(saved.clone()),
    )
    .is_err()
    {
        let _ = vak_sandbox::remove_frozen_candidate(&frozen_root);
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    room.branches[branch_pos].head_candidate_id = id.clone();
    room.revisions.push(OfficeRevision {
        candidate_id: id,
        parent_candidate_id: parent_id,
        merge_parent_candidate_id: merge_parent,
        branch_id,
        author_id: actor_id,
        author_name: actor_name,
        created_at: chrono::Utc::now().to_rfc3339(),
        ops: match &edits {
            Edits::Office(ops) => ops.clone(),
            Edits::Pdf(_) => Vec::new(),
        },
        pdf_steps: match edits {
            Edits::Pdf(steps) => steps,
            Edits::Office(_) => Vec::new(),
        },
    });
    if save(path, &room).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Some(handle) = state.get(&session_id) {
        let _ = handle.coworking_comments_tx.send(());
    }
    Json(serde_json::json!({"workspace":room,"candidate":saved,"worker_result":applied}))
        .into_response()
}

fn operations_since(
    room: &OfficeRoom,
    head: &str,
    base: &str,
) -> Option<Vec<vak_ooxml::edit::OfficeOp>> {
    let mut cursor = head;
    let mut result = Vec::new();
    for _ in 0..room.revisions.len() {
        if cursor == base {
            result.reverse();
            return Some(result.into_iter().flatten().collect());
        }
        let revision = room
            .revisions
            .iter()
            .rev()
            .find(|r| r.candidate_id == cursor)?;
        result.push(revision.ops.clone());
        cursor = &revision.parent_candidate_id;
    }
    None
}

/// A PDF branch's steps since `base`, oldest first.
fn pdf_steps_since(
    room: &OfficeRoom,
    head: &str,
    base: &str,
) -> Option<Vec<Vec<vak_pdf::edit::PdfOp>>> {
    let mut cursor = head;
    let mut result: Vec<Vec<Vec<vak_pdf::edit::PdfOp>>> = Vec::new();
    for _ in 0..room.revisions.len() {
        if cursor == base {
            result.reverse();
            return Some(result.into_iter().flatten().collect());
        }
        let revision = room
            .revisions
            .iter()
            .rev()
            .find(|r| r.candidate_id == cursor)?;
        result.push(revision.pdf_steps.clone());
        cursor = &revision.parent_candidate_id;
    }
    None
}

/// Whether two PDF branches overlap. PDF anchors are positions, so the
/// rule is conservative, as Word's positional merge is: a change to the
/// page order conflicts with any edit, deleting a line conflicts with any
/// line edit on that page, and two edits of one place conflict.
fn pdf_conflict(
    shared: &[Vec<vak_pdf::edit::PdfOp>],
    branch: &[Vec<vak_pdf::edit::PdfOp>],
) -> bool {
    use vak_pdf::edit::PdfOp as P;
    let ops = |steps: &[Vec<P>]| steps.iter().flatten().cloned().collect::<Vec<P>>();
    let (left, right) = (ops(shared), ops(branch));
    let reorders = |ops: &[P]| {
        ops.iter().any(|op| {
            matches!(
                op,
                P::DeletePage { .. }
                    | P::MovePage { .. }
                    | P::AddParagraph { .. }
                    | P::AddTable { .. }
                    | P::AddImage { .. }
                    | P::AddChart { .. }
                    | P::AddPageBreak { .. }
            )
        })
    };
    if (reorders(&left) && !right.is_empty()) || (reorders(&right) && !left.is_empty()) {
        return true;
    }
    let page = |anchor: &str| anchor.split('/').next().unwrap_or(anchor).to_string();
    let deleted_on = |ops: &[P]| {
        ops.iter()
            .filter_map(|op| match op {
                P::DeleteParagraph { anchor } => Some(page(anchor)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let lines_on = |ops: &[P]| {
        ops.iter()
            .filter_map(|op| match op {
                P::ReplaceParagraphText { anchor, .. }
                | P::DeleteParagraph { anchor }
                | P::AddComment { anchor, .. }
                | P::Highlight { anchor, .. } => Some(page(anchor)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    if deleted_on(&left)
        .iter()
        .any(|page| lines_on(&right).contains(page))
        || deleted_on(&right)
            .iter()
            .any(|page| lines_on(&left).contains(page))
    {
        return true;
    }
    let keys = |ops: &[P]| ops.iter().flat_map(P::keys).collect::<Vec<_>>();
    let right_keys = keys(&right);
    keys(&left).iter().any(|key| right_keys.contains(key))
}

fn op_keys(ops: &[vak_ooxml::edit::OfficeOp]) -> Vec<String> {
    use vak_ooxml::edit::OfficeOp as O;
    let mut keys = Vec::new();
    for op in ops {
        match op {
            O::ReplaceParagraphText { anchor, .. }
            | O::DeleteParagraph { anchor }
            | O::SetPlaceholderText { anchor, .. }
            | O::SetNotes { anchor, .. }
            | O::DeleteSlide { anchor }
            | O::MoveSlide { anchor, .. } => keys.push(format!("a:{anchor}")),
            O::AddParagraph { after, .. }
            | O::AddTable { after, .. }
            | O::AddImage { after, .. } => keys.push(match after {
                Some(anchor) => format!("a:{anchor}"),
                None => "doc:end".into(),
            }),
            O::SetCells { sheet, cells } => {
                keys.extend(cells.keys().map(|cell| format!("c:{sheet}!{cell}")));
                keys.push(format!("s:{sheet}:table-range"));
            }
            O::AppendRows { sheet, .. } => keys.push(format!("s:{sheet}:rows")),
            O::AddSheet { name: sheet } => keys.push(format!("s:{sheet}:create")),
            // A rename touches everything on the sheet under either name.
            O::RenameSheet { sheet, name } => {
                keys.push(format!("s:{sheet}:create"));
                keys.push(format!("s:{name}:create"));
                keys.push(format!("s:{sheet}:charts"));
                keys.push(format!("s:{sheet}:table-range"));
                keys.push(format!("s:{name}:charts"));
                keys.push(format!("s:{name}:table-range"));
            }
            O::FormatCells { sheet, .. } => keys.push(format!("s:{sheet}:format")),
            O::SetColumnWidths { sheet, .. } => keys.push(format!("s:{sheet}:columns")),
            // A chart reads a source range and adds drawing/package parts. Treat every
            // chart mutation on this sheet as conflicting with cell edits there.
            O::AddChart { sheet, .. } => keys.push(format!("s:{sheet}:charts")),
            O::AddExcelTable { sheet, .. } => keys.push(format!("s:{sheet}:table-range")),
            O::AddExcelImage { sheet, .. } => keys.push(format!("s:{sheet}:charts")),
            O::AddSlideFromLayout { .. } => keys.push("deck:slides".into()),
            O::SetTitle { .. } => keys.push("meta:title".into()),
        }
    }
    keys
}

fn has_conflict(
    shared: &[vak_ooxml::edit::OfficeOp],
    branch: &[vak_ooxml::edit::OfficeOp],
) -> bool {
    let a = op_keys(shared);
    let b = op_keys(branch);
    use vak_ooxml::edit::OfficeOp as O;
    let changes_paragraph_order = |ops: &[O]| {
        ops.iter().any(|op| {
            matches!(
                op,
                O::AddParagraph { .. } | O::AddTable { .. } | O::DeleteParagraph { .. }
            )
        })
    };
    let word = |ops: &[O]| {
        ops.iter().any(|op| {
            matches!(
                op,
                O::ReplaceParagraphText { .. }
                    | O::AddParagraph { .. }
                    | O::AddTable { .. }
                    | O::DeleteParagraph { .. }
            )
        })
    };
    let (shared_word, branch_word) = (word(shared), word(branch));
    (changes_paragraph_order(shared) && branch_word
        || changes_paragraph_order(branch) && shared_word)
        || a.iter().any(|left| {
            b.iter().any(|right| {
                left == right
                    || (left == "deck:slides" && right.starts_with("a:slide:"))
                    || (right == "deck:slides" && left.starts_with("a:slide:"))
                    || (left.starts_with("s:")
                        && right.starts_with("s:")
                        && left.split(':').nth(1) == right.split(':').nth(1)
                        && (left.ends_with(":create") || right.ends_with(":create")))
                    || (left.starts_with("c:")
                        && right.starts_with("s:")
                        && left
                            .strip_prefix("c:")
                            .and_then(|cell| cell.split_once('!'))
                            .is_some_and(|(sheet, _)| {
                                right
                                    .strip_prefix("s:")
                                    .is_some_and(|rows| rows.starts_with(&format!("{sheet}:")))
                            }))
                    || (right.starts_with("c:")
                        && left.starts_with("s:")
                        && right
                            .strip_prefix("c:")
                            .and_then(|cell| cell.split_once('!'))
                            .is_some_and(|(sheet, _)| {
                                left.strip_prefix("s:")
                                    .is_some_and(|rows| rows.starts_with(&format!("{sheet}:")))
                            }))
            })
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_ooxml::edit::OfficeOp as O;

    #[test]
    fn overlapping_cells_and_sheet_creation_conflict() {
        let left = vec![O::SetCells {
            sheet: "Sheet 1".into(),
            cells: [("A1".into(), vak_ooxml::edit::CellValue::Text("one".into()))]
                .into_iter()
                .collect(),
        }];
        let same = vec![O::SetCells {
            sheet: "Sheet 1".into(),
            cells: [("A1".into(), vak_ooxml::edit::CellValue::Text("two".into()))]
                .into_iter()
                .collect(),
        }];
        assert!(has_conflict(&left, &same));
        let create = vec![O::AddSheet {
            name: "Sheet 1".into(),
        }];
        assert!(has_conflict(&left, &create));
    }

    #[test]
    fn inserting_a_paragraph_conflicts_with_other_word_edits() {
        let insert = vec![O::AddParagraph {
            text: "new".into(),
            style: None,
            after: Some("p@1".into()),
        }];
        let edit = vec![O::ReplaceParagraphText {
            anchor: "p@8".into(),
            text: "changed".into(),
        }];
        assert!(has_conflict(&insert, &edit));
    }

    #[test]
    fn pdf_merges_refuse_overlapping_or_shifting_edits() {
        let steps =
            |json: &str| vec![serde_json::from_str::<Vec<vak_pdf::edit::PdfOp>>(json).unwrap()];
        let comment = steps(r#"[{"op": "add_comment", "anchor": "page:1/line:2", "text": "a"}]"#);
        let other_line =
            steps(r#"[{"op": "replace_paragraph_text", "anchor": "page:1/line:5", "text": "b"}]"#);
        let same_line =
            steps(r#"[{"op": "replace_paragraph_text", "anchor": "page:1/line:2", "text": "c"}]"#);
        let delete = steps(r#"[{"op": "delete_paragraph", "anchor": "page:1/line:9"}]"#);
        let move_page = steps(r#"[{"op": "move_page", "anchor": "page:3"}]"#);
        let title = steps(r#"[{"op": "set_title", "title": "x"}]"#);
        assert!(!pdf_conflict(&comment, &other_line));
        assert!(pdf_conflict(&comment, &same_line));
        assert!(
            pdf_conflict(&delete, &other_line),
            "a deleted line shifts the page's anchors"
        );
        assert!(
            pdf_conflict(&move_page, &title),
            "reordering pages conflicts with any edit"
        );
        assert!(!pdf_conflict(&title, &comment));
    }
}
