//! The isolated execution contract for vak.
//!
//! This crate deliberately does not know about models, prompts, sessions,
//! approvals, gateways, or presentation. It owns the environment lifecycle
//! and the reviewable boundary between a task candidate and a destination.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub mod backend;
pub mod docker;
#[cfg(target_os = "linux")]
pub mod landlock;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum EnvironmentState {
    Planned,
    Preparing,
    Ready,
    Running,
    Stopped,
    Failed,
    Expired,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum ExecutionEvent {
    Started {
        execution_id: String,
        environment_id: String,
        command: String,
        cwd: String,
    },
    Output {
        execution_id: String,
        stream: String,
        chunk: String,
        sequence: u64,
    },
    Artifact {
        execution_id: String,
        path: String,
        mime_type: String,
        bytes: u64,
    },
    Finished {
        execution_id: String,
        status: String,
        exit_code: Option<i32>,
        duration_ms: u64,
    },
}

/// Backend lifecycle owned by this crate. Policy and approval are deliberately
/// supplied by the caller; a backend cannot widen them or decide promotion.
pub trait EnvironmentBackend: Send + Sync {
    fn name(&self) -> &str;
    fn prepare(&self, plan: &EnvironmentPlan) -> Result<(), Error>;
    fn state(&self, environment_id: &str) -> EnvironmentState;
    fn cancel(&self, execution_id: &str) -> Result<(), Error>;
    fn export_candidate(&self, environment_id: &str) -> Result<CandidateManifest, Error>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct EnvironmentPlan {
    pub id: String,
    pub outcome_revision: u64,
    pub input_root: PathBuf,
    pub task_root: PathBuf,
    pub backend: String,
    pub image: Option<String>,
    pub network_policy: String,
    pub setup_recipe: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum CandidateOperation {
    Upsert,
    Delete,
}

impl Default for CandidateOperation {
    fn default() -> Self {
        Self::Upsert
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CandidateFile {
    pub path: String,
    pub candidate_hash: String,
    pub base_hash: Option<String>,
    pub bytes: u64,
    #[serde(default)]
    pub operation: CandidateOperation,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CandidateManifest {
    pub candidate_id: String,
    pub source_root: PathBuf,
    pub destination_root: PathBuf,
    pub files: Vec<CandidateFile>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PromotionReceipt {
    pub candidate_id: String,
    pub applied: Vec<String>,
    pub before_hashes: Vec<(String, Option<String>)>,
    pub after_hashes: Vec<(String, String)>,
    pub verification: Vec<VerificationResult>,
    #[serde(default)]
    pub deleted: Vec<String>,
    #[serde(default)]
    pub integration: IntegrationVerification,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct IntegrationVerification {
    /// Digest of the exact accepted selection and the state observed after
    /// apply. This lets every later check name the workspace state it tested.
    pub applied_state_digest: String,
    /// `observed` means the accepted bytes/absences still match. Target checks
    /// have their own status and must never be inferred from this value.
    pub workspace_state_status: String,
    /// `unavailable` until a registered target verifier runs against this
    /// applied state. A sandbox run is deliberately not copied into this field.
    pub target_checks_status: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PromotionTransactionState {
    Prepared,
    Applying,
    Completed,
    RolledBack,
    RecoveryRequired,
    Undoing,
    Undone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PromotionFileState {
    Prepared,
    Applying,
    Applied,
    Undoing,
    Undone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionTransactionFile {
    pub path: String,
    pub before_hash: Option<String>,
    pub after_hash: String,
    pub backup_path: Option<PathBuf>,
    pub state: PromotionFileState,
    #[serde(default)]
    pub operation: CandidateOperation,
}

/// Crash-recovery journal for one exact candidate import. It is stored outside
/// the destination workspace, so the Agent cannot edit its own transaction
/// state and a partially-applied import remains recoverable after restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionTransaction {
    pub schema_version: u32,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub destination_root: PathBuf,
    pub state: PromotionTransactionState,
    pub files: Vec<PromotionTransactionFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationResult {
    pub path: String,
    pub status: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UndoReceipt {
    pub candidate_id: String,
    pub restored: Vec<String>,
    pub verification: Vec<VerificationResult>,
}

/// Durable control-plane fact for an environment lifecycle. The session ledger
/// remains the conversational source of truth; this JSONL record is the
/// addressable projection used by Workbench and server operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentRecord {
    pub record_id: String,
    pub environment_id: String,
    pub state: EnvironmentState,
    pub plan: EnvironmentPlan,
    pub updated_at: String,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateRecord {
    pub record_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub result_id: String,
    pub execution_id: String,
    pub environment_id: String,
    pub candidate_digest: String,
    pub candidate: CandidateManifest,
    pub verified: bool,
    pub updated_at: String,
    /// The saved version used as input for a human-requested revision.
    #[serde(default)]
    pub parent_candidate_id: Option<String>,
    /// Durable child session whose tool receipts produced this version.
    #[serde(default)]
    pub revision_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateRevisionStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateRevisionRecord {
    pub record_id: String,
    pub revision_id: String,
    pub session_id: String,
    pub parent_candidate_id: String,
    pub comment_id: String,
    pub child_session_id: String,
    pub task_root: PathBuf,
    pub status: CandidateRevisionStatus,
    pub candidate_id: Option<String>,
    pub detail: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionRecord {
    pub record_id: String,
    pub session_id: String,
    pub result_id: String,
    pub candidate_digest: String,
    pub candidate_id: String,
    pub receipt: PromotionReceipt,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionUndoRecord {
    pub record_id: String,
    pub session_id: String,
    pub candidate_id: String,
    pub receipt: UndoReceipt,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "record")]
pub enum DurableRecord {
    Environment(EnvironmentRecord),
    Candidate(CandidateRecord),
    Promotion(PromotionRecord),
    PromotionUndo(PromotionUndoRecord),
    CandidateRevision(CandidateRevisionRecord),
}

pub fn append_record(path: &Path, record: &DurableRecord) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(record)
        .map_err(|e| Error::InvalidPlan(format!("record serialization failed: {e}")))?;
    line.push(b'\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    use std::io::Write;
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(())
}

pub fn load_records(path: &Path) -> Result<Vec<DurableRecord>, Error> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|e| Error::InvalidPlan(format!("record parse failed: {e}")))
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("path escapes its root: {0}")]
    PathEscape(String),
    #[error("candidate file is missing: {0}")]
    Missing(String),
    #[error("candidate changed after review: {0}")]
    CandidateChanged(String),
    #[error("workspace changed since review: {0}")]
    Conflict(String),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid environment plan: {0}")]
    InvalidPlan(String),
}

pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn candidate_digest(candidate: &CandidateManifest) -> Result<String, Error> {
    let bytes = serde_json::to_vec(candidate)
        .map_err(|error| Error::InvalidPlan(format!("candidate serialization failed: {error}")))?;
    Ok(digest(&bytes))
}

fn confined(root: &Path, relative: &str) -> Result<PathBuf, Error> {
    let rel = Path::new(relative);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::PathEscape(relative.to_string()));
    }
    let root = root
        .canonicalize()
        .map_err(|error| Error::InvalidPlan(format!("root is unavailable: {error}")))?;
    let mut current = root;
    for component in rel.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(Error::PathEscape(relative.to_string()));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Ok(current)
}

pub fn candidate_manifest(
    id: &str,
    source_root: &Path,
    destination_root: &Path,
) -> Result<CandidateManifest, Error> {
    let mut files = Vec::new();
    for item in WalkDir::new(source_root).follow_links(false) {
        let item = item.map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
        // A task Core may maintain local control state. It is never part of
        // the deliverable and must not enter a reviewed candidate.
        if item
            .path()
            .strip_prefix(source_root)
            .ok()
            .is_some_and(|relative| {
                relative
                    .components()
                    .next()
                    .is_some_and(|component| component.as_os_str() == ".vak")
            })
        {
            continue;
        }
        if !item.file_type().is_file() {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(source_root)
            .map_err(|_| Error::PathEscape(item.path().display().to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(item.path())?;
        let target = destination_root.join(&relative);
        let base_hash = target
            .is_file()
            .then(|| fs::read(&target).ok())
            .flatten()
            .map(|b| digest(&b));
        files.push(CandidateFile {
            path: relative,
            candidate_hash: digest(&bytes),
            base_hash,
            bytes: bytes.len() as u64,
            operation: CandidateOperation::Upsert,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(CandidateManifest {
        candidate_id: id.to_string(),
        source_root: source_root.to_path_buf(),
        destination_root: destination_root.to_path_buf(),
        files,
    })
}

/// Capture the reviewed bytes under a new, server-owned directory. The
/// manifest still records the destination baseline observed at export time.
pub fn freeze_candidate(
    id: &str,
    source_root: &Path,
    destination_root: &Path,
    frozen_root: &Path,
) -> Result<CandidateManifest, Error> {
    let mut manifest = candidate_manifest(id, source_root, destination_root)?;
    fs::create_dir(frozen_root)?;
    let copy = (|| -> Result<(), Error> {
        for file in &manifest.files {
            if file.operation == CandidateOperation::Delete {
                continue;
            }
            let source = confined(source_root, &file.path)?;
            let bytes = fs::read(&source).map_err(|_| Error::Missing(file.path.clone()))?;
            if digest(&bytes) != file.candidate_hash {
                return Err(Error::CandidateChanged(file.path.clone()));
            }
            let target = confined(frozen_root, &file.path)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            use std::io::Write;
            output.write_all(&bytes)?;
            output.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = copy {
        let _ = fs::remove_dir_all(frozen_root);
        return Err(error);
    }
    manifest.source_root = frozen_root.to_path_buf();
    Ok(manifest)
}

/// Freeze a later version while retaining the baseline the person originally
/// reviewed. Re-reading the destination here would let intervening workspace
/// edits become an implicitly accepted baseline.
pub fn freeze_revision_candidate(
    id: &str,
    task_root: &Path,
    parent: &CandidateManifest,
    frozen_root: &Path,
) -> Result<CandidateManifest, Error> {
    let mut revision = freeze_candidate(id, task_root, &parent.destination_root, frozen_root)?;
    let result = (|| -> Result<(), Error> {
        for original in &parent.files {
            if !revision.files.iter().any(|file| file.path == original.path)
                && original.base_hash.is_some()
            {
                revision.files.push(CandidateFile {
                    path: original.path.clone(),
                    candidate_hash: original.candidate_hash.clone(),
                    base_hash: original.base_hash.clone(),
                    bytes: 0,
                    operation: CandidateOperation::Delete,
                });
            }
        }
        revision.files.sort_by(|a, b| a.path.cmp(&b.path));
        if revision.files.is_empty() {
            return Err(Error::InvalidPlan(
                "revision has no workspace changes to review".into(),
            ));
        }
        let changed = revision.files.len() != parent.files.len()
            || revision.files.iter().any(|file| {
                parent
                    .files
                    .iter()
                    .find(|old| old.path == file.path)
                    .is_some_and(|old| {
                        old.candidate_hash != file.candidate_hash || old.operation != file.operation
                    })
            });
        if !changed {
            return Err(Error::InvalidPlan(
                "revision did not change candidate files".into(),
            ));
        }
        for file in &mut revision.files {
            file.base_hash = parent
                .files
                .iter()
                .find(|old| old.path == file.path)
                .and_then(|old| old.base_hash.clone());
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(frozen_root);
        return Err(error);
    }
    Ok(revision)
}

/// Seed a fresh revision environment from the exact reviewed version. Only
/// manifest files are copied, and each byte stream is verified against the
/// saved candidate before it becomes writable task input. The destination
/// must not exist, so a prior run can never be silently reused.
pub fn prepare_revision_copy(candidate: &CandidateManifest, task_root: &Path) -> Result<(), Error> {
    fs::create_dir(task_root)?;
    let copy = (|| -> Result<(), Error> {
        for file in &candidate.files {
            if file.operation == CandidateOperation::Delete {
                continue;
            }
            let source = confined(&candidate.source_root, &file.path)?;
            let bytes = fs::read(&source).map_err(|_| Error::Missing(file.path.clone()))?;
            if digest(&bytes) != file.candidate_hash {
                return Err(Error::CandidateChanged(file.path.clone()));
            }
            let target = confined(task_root, &file.path)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            use std::io::Write;
            output.write_all(&bytes)?;
            output.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = copy {
        let _ = fs::remove_dir_all(task_root);
        return Err(error);
    }
    Ok(())
}

fn write_transaction(path: &Path, transaction: &PromotionTransaction) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::InvalidPlan("promotion journal has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join("journal.json.tmp");
    let bytes = serde_json::to_vec_pretty(transaction)
        .map_err(|error| Error::InvalidPlan(format!("journal serialization failed: {error}")))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    use std::io::Write;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn load_transaction(path: &Path) -> Result<PromotionTransaction, Error> {
    serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| Error::InvalidPlan(format!("journal parse failed: {error}")))
}

fn transaction_directory(root: &Path, candidate_id: &str) -> Result<PathBuf, Error> {
    let mut components = Path::new(candidate_id).components();
    let valid = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if !valid {
        return Err(Error::PathEscape(candidate_id.into()));
    }
    Ok(root.join(candidate_id))
}

fn transaction_receipt(transaction: &PromotionTransaction) -> PromotionReceipt {
    let applied_state_digest = promotion_state_digest(transaction);
    PromotionReceipt {
        candidate_id: transaction.candidate_id.clone(),
        applied: transaction
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        before_hashes: transaction
            .files
            .iter()
            .map(|file| (file.path.clone(), file.before_hash.clone()))
            .collect(),
        after_hashes: transaction
            .files
            .iter()
            .filter(|file| file.operation == CandidateOperation::Upsert)
            .map(|file| (file.path.clone(), file.after_hash.clone()))
            .collect(),
        verification: transaction
            .files
            .iter()
            .map(|file| VerificationResult {
                path: file.path.clone(),
                status: "observed".into(),
                evidence: if file.operation == CandidateOperation::Delete {
                    "destination absence verified".into()
                } else {
                    format!("destination hash verified: {}", file.after_hash)
                },
            })
            .collect(),
        deleted: transaction
            .files
            .iter()
            .filter(|file| file.operation == CandidateOperation::Delete)
            .map(|file| file.path.clone())
            .collect(),
        integration: IntegrationVerification {
            applied_state_digest,
            workspace_state_status: "observed".into(),
            target_checks_status: "unavailable".into(),
            evidence: "accepted files and deletions were read back from the target workspace; no registered target verifier ran".into(),
        },
    }
}

fn promotion_state_digest(transaction: &PromotionTransaction) -> String {
    let mut state = format!("candidate:{}\n", transaction.candidate_digest);
    for file in &transaction.files {
        let observed = if file.operation == CandidateOperation::Delete {
            "absent"
        } else {
            file.after_hash.as_str()
        };
        state.push_str(&file.path);
        state.push('\t');
        state.push_str(observed);
        state.push('\n');
    }
    digest(state.as_bytes())
}

fn rollback_transaction(
    transaction: &mut PromotionTransaction,
    journal_path: &Path,
) -> Result<(), Error> {
    transaction.state = PromotionTransactionState::RecoveryRequired;
    write_transaction(journal_path, transaction)?;
    for index in (0..transaction.files.len()).rev() {
        if transaction.files[index].state == PromotionFileState::Prepared {
            continue;
        }
        let file = transaction.files[index].clone();
        let target = confined(&transaction.destination_root, &file.path)?;
        let current = match fs::read(&target) {
            Ok(bytes) => Some(digest(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(Error::Io(error)),
        };
        if current == file.before_hash && file.state == PromotionFileState::Applying {
            transaction.files[index].state = PromotionFileState::Prepared;
            write_transaction(journal_path, transaction)?;
            continue;
        }
        let expected_after =
            (file.operation == CandidateOperation::Upsert).then_some(file.after_hash.as_str());
        if current.as_deref() != expected_after {
            return Err(Error::Conflict(format!(
                "promotion recovery blocked by a later workspace change: {}",
                file.path
            )));
        }
        if let Some(backup) = &file.backup_path {
            let bytes = fs::read(backup)?;
            if digest(&bytes) != file.before_hash.clone().unwrap_or_default() {
                return Err(Error::CandidateChanged(format!(
                    "promotion backup changed: {}",
                    file.path
                )));
            }
            let temporary =
                target.with_extension(format!("vak-recovery-{}", transaction.candidate_id));
            fs::write(&temporary, bytes)?;
            fs::rename(temporary, target)?;
        } else {
            fs::remove_file(target)?;
        }
        transaction.files[index].state = PromotionFileState::Prepared;
        write_transaction(journal_path, transaction)?;
    }
    transaction.state = PromotionTransactionState::RolledBack;
    write_transaction(journal_path, transaction)
}

/// Import a reviewed candidate under a cross-process workspace lock. Before
/// images and per-file progress are persisted before the first destination
/// rename. An interrupted prior attempt is rolled back before a retry, while
/// a completed journal is idempotently returned for durable-record recovery.
pub fn promote_recoverable(
    candidate: &CandidateManifest,
    transaction_root: &Path,
) -> Result<PromotionReceipt, Error> {
    fs::create_dir_all(transaction_root)?;
    let lock_destination = candidate
        .destination_root
        .canonicalize()
        .map_err(Error::Io)?;
    let workspace_key = digest(lock_destination.to_string_lossy().as_bytes()).replace(':', "_");
    let lock_path = transaction_root.join(format!("{workspace_key}.lock"));
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(Error::Io)?;
    lock_file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => {
            Error::Conflict("another workspace acceptance is in progress".into())
        }
        std::fs::TryLockError::Error(error) => Error::Io(error),
    })?;
    let directory = transaction_directory(transaction_root, &candidate.candidate_id)?;
    let journal_path = directory.join("journal.json");
    let selected_digest = candidate_digest(candidate)?;
    if journal_path.exists() {
        let mut previous = load_transaction(&journal_path)?;
        if previous.candidate_id != candidate.candidate_id
            || previous.candidate_digest != selected_digest
            || previous.destination_root != candidate.destination_root
        {
            return Err(Error::InvalidPlan(
                "promotion journal identity mismatch".into(),
            ));
        }
        if previous.state == PromotionTransactionState::Completed {
            for file in &previous.files {
                let target = confined(&previous.destination_root, &file.path)?;
                let observed = fs::read(&target).ok().map(|bytes| digest(&bytes));
                let expected = (file.operation == CandidateOperation::Upsert)
                    .then_some(file.after_hash.as_str());
                if observed.as_deref() != expected {
                    return Err(Error::Conflict(format!(
                        "completed promotion no longer matches the workspace: {}",
                        file.path
                    )));
                }
            }
            return Ok(transaction_receipt(&previous));
        }
        if matches!(
            previous.state,
            PromotionTransactionState::Undoing | PromotionTransactionState::Undone
        ) {
            return Err(Error::Conflict(
                "candidate acceptance has already been undone".into(),
            ));
        }
        if previous
            .files
            .iter()
            .any(|file| file.state != PromotionFileState::Prepared)
        {
            rollback_transaction(&mut previous, &journal_path)?;
        }
        fs::remove_dir_all(&directory)?;
    } else if directory.exists() {
        // A process can stop during preflight before the first journal write.
        // No destination rename is possible at that point, so the orphaned
        // backup staging directory is safe to discard before retry.
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(directory.join("backups"))?;
    let mut staged = Vec::new();
    let mut files = Vec::new();
    for (index, file) in candidate.files.iter().enumerate() {
        let bytes = if file.operation == CandidateOperation::Delete {
            Vec::new()
        } else {
            let source = confined(&candidate.source_root, &file.path)?;
            let bytes = fs::read(&source).map_err(|_| Error::Missing(file.path.clone()))?;
            if digest(&bytes) != file.candidate_hash {
                return Err(Error::CandidateChanged(file.path.clone()));
            }
            bytes
        };
        let proposed_target = candidate.destination_root.join(&file.path);
        let mut parent = proposed_target.parent();
        while let Some(path) = parent {
            if path == candidate.destination_root {
                break;
            }
            match fs::metadata(path) {
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(Error::Conflict(file.path.clone()));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::Io(error)),
            }
            parent = path.parent();
        }
        let target = confined(&candidate.destination_root, &file.path)?;
        let before_bytes = match fs::metadata(&target) {
            Ok(metadata) if metadata.is_file() => Some(fs::read(&target)?),
            Ok(_) => return Err(Error::Conflict(file.path.clone())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(Error::Io(error)),
        };
        let before_hash = before_bytes.as_deref().map(digest);
        if before_hash != file.base_hash {
            return Err(Error::Conflict(file.path.clone()));
        }
        let backup_path = if let Some(before) = before_bytes {
            let path = directory.join("backups").join(index.to_string());
            let mut backup = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            use std::io::Write;
            backup.write_all(&before)?;
            backup.sync_all()?;
            Some(path)
        } else {
            None
        };
        files.push(PromotionTransactionFile {
            path: file.path.clone(),
            before_hash,
            after_hash: file.candidate_hash.clone(),
            backup_path,
            state: PromotionFileState::Prepared,
            operation: file.operation.clone(),
        });
        staged.push((target, bytes));
    }
    let mut transaction = PromotionTransaction {
        schema_version: 1,
        candidate_id: candidate.candidate_id.clone(),
        candidate_digest: selected_digest,
        destination_root: candidate.destination_root.clone(),
        state: PromotionTransactionState::Prepared,
        files,
    };
    write_transaction(&journal_path, &transaction)?;
    transaction.state = PromotionTransactionState::Applying;
    write_transaction(&journal_path, &transaction)?;
    for (index, (target, bytes)) in staged.into_iter().enumerate() {
        let temporary = target.with_extension(format!("vak-promotion-{}", candidate.candidate_id));
        let result = (|| -> Result<(), Error> {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            transaction.files[index].state = PromotionFileState::Applying;
            write_transaction(&journal_path, &transaction)?;
            if transaction.files[index].operation == CandidateOperation::Delete {
                fs::remove_file(&target)?;
            } else {
                let mut output = fs::OpenOptions::new()
                    .create(true)
                    .truncate(true)
                    .write(true)
                    .open(&temporary)?;
                use std::io::Write;
                output.write_all(&bytes)?;
                output.sync_all()?;
                fs::rename(&temporary, &target)?;
            }
            transaction.files[index].state = PromotionFileState::Applied;
            write_transaction(&journal_path, &transaction)?;
            let observed = fs::read(&target).ok().map(|bytes| digest(&bytes));
            let expected = (transaction.files[index].operation == CandidateOperation::Upsert)
                .then_some(transaction.files[index].after_hash.as_str());
            if observed.as_deref() != expected {
                return Err(Error::Conflict(format!(
                    "post-apply verification failed: {}",
                    transaction.files[index].path
                )));
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            rollback_transaction(&mut transaction, &journal_path)?;
            return Err(error);
        }
    }
    transaction.state = PromotionTransactionState::Completed;
    write_transaction(&journal_path, &transaction)?;
    Ok(transaction_receipt(&transaction))
}

/// Reverse one completed promotion while it still owns the destination bytes.
/// Undo is itself journaled and resumes after a crash. Any later workspace
/// edit blocks the operation instead of being erased.
pub fn undo_promotion(candidate_id: &str, transaction_root: &Path) -> Result<UndoReceipt, Error> {
    let directory = transaction_directory(transaction_root, candidate_id)?;
    let journal_path = directory.join("journal.json");
    let mut transaction = load_transaction(&journal_path)?;
    if transaction.candidate_id != candidate_id {
        return Err(Error::InvalidPlan(
            "promotion journal identity mismatch".into(),
        ));
    }
    fs::create_dir_all(transaction_root)?;
    let lock_destination = transaction
        .destination_root
        .canonicalize()
        .map_err(Error::Io)?;
    let workspace_key = digest(lock_destination.to_string_lossy().as_bytes()).replace(':', "_");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(transaction_root.join(format!("{workspace_key}.lock")))?;
    lock_file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => {
            Error::Conflict("another workspace acceptance is in progress".into())
        }
        std::fs::TryLockError::Error(error) => Error::Io(error),
    })?;
    transaction = load_transaction(&journal_path)?;
    if transaction.candidate_id != candidate_id {
        return Err(Error::InvalidPlan(
            "promotion journal identity mismatch".into(),
        ));
    }
    if transaction.state == PromotionTransactionState::Undone {
        let verification = transaction
            .files
            .iter()
            .map(|file| VerificationResult {
                path: file.path.clone(),
                status: "observed".into(),
                evidence: match &file.before_hash {
                    Some(hash) => format!("restored destination hash verified: {hash}"),
                    None => "new destination file removed".into(),
                },
            })
            .collect();
        return Ok(UndoReceipt {
            candidate_id: candidate_id.into(),
            restored: transaction
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect(),
            verification,
        });
    }
    if !matches!(
        transaction.state,
        PromotionTransactionState::Completed | PromotionTransactionState::Undoing
    ) {
        return Err(Error::Conflict(
            "candidate acceptance is not complete and cannot be undone".into(),
        ));
    }
    transaction.state = PromotionTransactionState::Undoing;
    write_transaction(&journal_path, &transaction)?;
    for index in (0..transaction.files.len()).rev() {
        let file = transaction.files[index].clone();
        let target = confined(&transaction.destination_root, &file.path)?;
        let current = match fs::read(&target) {
            Ok(bytes) => Some(digest(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(Error::Io(error)),
        };
        if file.state == PromotionFileState::Undone {
            if current != file.before_hash {
                return Err(Error::Conflict(format!(
                    "undo recovery blocked by a later workspace change: {}",
                    file.path
                )));
            }
            continue;
        }
        if file.state == PromotionFileState::Undoing && current == file.before_hash {
            transaction.files[index].state = PromotionFileState::Undone;
            write_transaction(&journal_path, &transaction)?;
            continue;
        }
        let expected_after =
            (file.operation == CandidateOperation::Upsert).then_some(file.after_hash.as_str());
        if current.as_deref() != expected_after {
            return Err(Error::Conflict(format!(
                "undo blocked by a later workspace change: {}",
                file.path
            )));
        }
        transaction.files[index].state = PromotionFileState::Undoing;
        write_transaction(&journal_path, &transaction)?;
        if let Some(backup) = &file.backup_path {
            let bytes = fs::read(backup)?;
            if Some(digest(&bytes)) != file.before_hash {
                return Err(Error::CandidateChanged(format!(
                    "promotion backup changed: {}",
                    file.path
                )));
            }
            let temporary = target.with_extension(format!("vak-undo-{candidate_id}"));
            fs::write(&temporary, bytes)?;
            fs::rename(temporary, &target)?;
        } else {
            fs::remove_file(&target)?;
        }
        let observed = fs::read(&target).ok().map(|bytes| digest(&bytes));
        if observed != file.before_hash {
            return Err(Error::Conflict(format!(
                "post-undo verification failed: {}",
                file.path
            )));
        }
        transaction.files[index].state = PromotionFileState::Undone;
        write_transaction(&journal_path, &transaction)?;
    }
    transaction.state = PromotionTransactionState::Undone;
    write_transaction(&journal_path, &transaction)?;
    Ok(UndoReceipt {
        candidate_id: candidate_id.into(),
        restored: transaction
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        verification: transaction
            .files
            .iter()
            .map(|file| VerificationResult {
                path: file.path.clone(),
                status: "observed".into(),
                evidence: match &file.before_hash {
                    Some(hash) => format!("restored destination hash verified: {hash}"),
                    None => "new destination file removed".into(),
                },
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn promote(candidate: &CandidateManifest) -> Result<PromotionReceipt, Error> {
        let control = tempfile::tempdir()?;
        promote_recoverable(candidate, control.path())
    }

    #[test]
    fn promotion_is_compare_before_write() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::write(source.path().join("result.txt"), "draft").unwrap();
        let candidate = candidate_manifest("c1", source.path(), target.path()).unwrap();
        fs::write(target.path().join("result.txt"), "user-edit").unwrap();
        assert!(matches!(promote(&candidate), Err(Error::Conflict(path)) if path == "result.txt"));
        assert_eq!(
            fs::read_to_string(target.path().join("result.txt")).unwrap(),
            "user-edit"
        );
    }
    #[test]
    fn nested_artifacts_are_first_class() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::create_dir_all(source.path().join("assets")).unwrap();
        fs::write(source.path().join("assets/chart.csv"), "x,y\n1,2\n").unwrap();
        let candidate = candidate_manifest("c2", source.path(), target.path()).unwrap();
        let receipt = promote(&candidate).unwrap();
        assert_eq!(receipt.applied, vec!["assets/chart.csv"]);
        assert_eq!(receipt.verification[0].status, "observed");
        assert_eq!(receipt.integration.workspace_state_status, "observed");
        assert_eq!(receipt.integration.target_checks_status, "unavailable");
        assert!(
            receipt
                .integration
                .applied_state_digest
                .starts_with("sha256:")
        );
    }

    #[test]
    fn promotion_preflights_later_destination_types_before_writing() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), "ready").unwrap();
        fs::write(source.path().join("z.txt"), "blocked").unwrap();
        let candidate = candidate_manifest("c3", source.path(), target.path()).unwrap();
        fs::create_dir(target.path().join("z.txt")).unwrap();

        assert!(matches!(promote(&candidate), Err(Error::Conflict(path)) if path == "z.txt"));
        assert!(!target.path().join("a.txt").exists());
    }

    #[test]
    fn promotion_preflights_later_parent_collisions_before_writing() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), "ready").unwrap();
        fs::create_dir(source.path().join("nested")).unwrap();
        fs::write(source.path().join("nested/z.txt"), "blocked").unwrap();
        let candidate = candidate_manifest("c4", source.path(), target.path()).unwrap();
        fs::write(target.path().join("nested"), "user file").unwrap();

        assert!(
            matches!(promote(&candidate), Err(Error::Conflict(path)) if path == "nested/z.txt")
        );
        assert!(!target.path().join("a.txt").exists());
    }

    #[test]
    fn frozen_candidate_keeps_reviewed_bytes_after_scratch_changes() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        fs::write(source.path().join("result.txt"), "reviewed").unwrap();
        let frozen = store.path().join("candidate-1");
        let candidate =
            freeze_candidate("candidate-1", source.path(), target.path(), &frozen).unwrap();
        fs::write(source.path().join("result.txt"), "later agent work").unwrap();

        assert_eq!(candidate.source_root, frozen);
        assert_eq!(
            fs::read_to_string(frozen.join("result.txt")).unwrap(),
            "reviewed"
        );
        let receipt = promote(&candidate).unwrap();
        assert_eq!(receipt.applied, vec!["result.txt"]);
        assert_eq!(
            fs::read_to_string(target.path().join("result.txt")).unwrap(),
            "reviewed"
        );
    }

    #[test]
    fn revision_copy_uses_only_verified_saved_version() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("pages")).unwrap();
        fs::write(source.path().join("pages/index.html"), "version one").unwrap();
        let saved = freeze_candidate(
            "version-one",
            source.path(),
            target.path(),
            &store.path().join("saved"),
        )
        .unwrap();
        fs::write(source.path().join("pages/index.html"), "unreviewed change").unwrap();
        fs::write(
            saved.source_root.join("unlisted.txt"),
            "must not enter copy",
        )
        .unwrap();
        let copy = store.path().join("revision-copy");
        prepare_revision_copy(&saved, &copy).unwrap();
        assert_eq!(
            fs::read_to_string(copy.join("pages/index.html")).unwrap(),
            "version one"
        );
        assert!(!copy.join("unlisted.txt").exists());

        fs::write(saved.source_root.join("pages/index.html"), "tampered").unwrap();
        let failed_copy = store.path().join("failed-copy");
        assert!(matches!(
            prepare_revision_copy(&saved, &failed_copy),
            Err(Error::CandidateChanged(path)) if path == "pages/index.html"
        ));
        assert!(!failed_copy.exists());
    }

    #[test]
    fn revised_candidate_keeps_original_baseline_and_excludes_control_files() {
        let source = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("page.html"),
            "workspace before review",
        )
        .unwrap();
        fs::write(source.path().join("page.html"), "version one").unwrap();
        let first = freeze_candidate(
            "v1",
            source.path(),
            workspace.path(),
            &store.path().join("v1"),
        )
        .unwrap();
        let original_base = first.files[0].base_hash.clone();
        fs::write(
            workspace.path().join("page.html"),
            "human changed workspace",
        )
        .unwrap();
        let task = store.path().join("task");
        prepare_revision_copy(&first, &task).unwrap();
        fs::write(task.join("page.html"), "version two").unwrap();
        fs::write(task.join("added.txt"), "new draft file").unwrap();
        fs::create_dir(task.join(".vak")).unwrap();
        fs::write(task.join(".vak/config.toml"), "private control state").unwrap();
        let second =
            freeze_revision_candidate("v2", &task, &first, &store.path().join("v2")).unwrap();
        assert_eq!(second.files.len(), 2);
        assert_eq!(
            second
                .files
                .iter()
                .find(|file| file.path == "page.html")
                .unwrap()
                .base_hash,
            original_base
        );
        assert_eq!(
            second
                .files
                .iter()
                .find(|file| file.path == "added.txt")
                .unwrap()
                .base_hash,
            None
        );
        assert!(!second.source_root.join(".vak").exists());
        assert!(matches!(promote(&second), Err(Error::Conflict(path)) if path == "page.html"));
    }

    #[test]
    fn unchanged_revision_does_not_create_another_version() {
        let source = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        fs::write(source.path().join("draft.txt"), "same").unwrap();
        let first = freeze_candidate(
            "v1",
            source.path(),
            workspace.path(),
            &store.path().join("v1"),
        )
        .unwrap();
        let task = store.path().join("task");
        prepare_revision_copy(&first, &task).unwrap();
        let next = store.path().join("v2");
        assert!(matches!(
            freeze_revision_candidate("v2", &task, &first, &next),
            Err(Error::InvalidPlan(_))
        ));
        assert!(!next.exists());
    }

    #[cfg(unix)]
    #[test]
    fn promotion_rejects_symlinked_candidate_paths() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            source.path().join("candidate.txt"),
        )
        .unwrap();

        let candidate = CandidateManifest {
            candidate_id: "symlink".into(),
            source_root: source.path().into(),
            destination_root: target.path().into(),
            files: vec![CandidateFile {
                path: "candidate.txt".into(),
                candidate_hash: digest(b"secret"),
                base_hash: None,
                bytes: 6,
                operation: CandidateOperation::Upsert,
            }],
        };
        assert!(matches!(promote(&candidate), Err(Error::PathEscape(_))));
        assert!(!target.path().join("candidate.txt").exists());
    }

    #[test]
    fn recoverable_promotion_rolls_back_interrupted_apply_then_retries() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), "new a").unwrap();
        fs::write(source.path().join("b.txt"), "new b").unwrap();
        fs::write(target.path().join("a.txt"), "old a").unwrap();
        let candidate = CandidateManifest {
            candidate_id: "recoverable".into(),
            source_root: source.path().into(),
            destination_root: target.path().into(),
            files: vec![
                CandidateFile {
                    path: "a.txt".into(),
                    candidate_hash: digest(b"new a"),
                    base_hash: Some(digest(b"old a")),
                    bytes: 5,
                    operation: CandidateOperation::Upsert,
                },
                CandidateFile {
                    path: "b.txt".into(),
                    candidate_hash: digest(b"new b"),
                    base_hash: None,
                    bytes: 5,
                    operation: CandidateOperation::Upsert,
                },
            ],
        };
        let directory = control.path().join("recoverable");
        fs::create_dir_all(directory.join("backups")).unwrap();
        let backup = directory.join("backups/0");
        fs::write(&backup, "old a").unwrap();
        fs::write(target.path().join("a.txt"), "new a").unwrap();
        let interrupted = PromotionTransaction {
            schema_version: 1,
            candidate_id: "recoverable".into(),
            candidate_digest: candidate_digest(&candidate).unwrap(),
            destination_root: target.path().into(),
            state: PromotionTransactionState::Applying,
            files: vec![
                PromotionTransactionFile {
                    path: "a.txt".into(),
                    before_hash: Some(digest(b"old a")),
                    after_hash: digest(b"new a"),
                    backup_path: Some(backup),
                    state: PromotionFileState::Applied,
                    operation: CandidateOperation::Upsert,
                },
                PromotionTransactionFile {
                    path: "b.txt".into(),
                    before_hash: None,
                    after_hash: digest(b"new b"),
                    backup_path: None,
                    state: PromotionFileState::Applying,
                    operation: CandidateOperation::Upsert,
                },
            ],
        };
        write_transaction(&directory.join("journal.json"), &interrupted).unwrap();

        let receipt = promote_recoverable(&candidate, control.path()).unwrap();
        assert_eq!(receipt.applied, vec!["a.txt", "b.txt"]);
        assert_eq!(
            fs::read_to_string(target.path().join("a.txt")).unwrap(),
            "new a"
        );
        assert_eq!(
            fs::read_to_string(target.path().join("b.txt")).unwrap(),
            "new b"
        );
        let journal = load_transaction(&directory.join("journal.json")).unwrap();
        assert_eq!(journal.state, PromotionTransactionState::Completed);
    }

    #[test]
    fn recovery_refuses_to_erase_a_later_workspace_edit() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), "candidate").unwrap();
        fs::write(target.path().join("a.txt"), "human edit").unwrap();
        let candidate = CandidateManifest {
            candidate_id: "conflicted-recovery".into(),
            source_root: source.path().into(),
            destination_root: target.path().into(),
            files: vec![CandidateFile {
                path: "a.txt".into(),
                candidate_hash: digest(b"candidate"),
                base_hash: Some(digest(b"original")),
                bytes: 9,
                operation: CandidateOperation::Upsert,
            }],
        };
        let directory = control.path().join("conflicted-recovery");
        fs::create_dir_all(directory.join("backups")).unwrap();
        let backup = directory.join("backups/0");
        fs::write(&backup, "original").unwrap();
        write_transaction(
            &directory.join("journal.json"),
            &PromotionTransaction {
                schema_version: 1,
                candidate_id: candidate.candidate_id.clone(),
                candidate_digest: candidate_digest(&candidate).unwrap(),
                destination_root: target.path().into(),
                state: PromotionTransactionState::Applying,
                files: vec![PromotionTransactionFile {
                    path: "a.txt".into(),
                    before_hash: Some(digest(b"original")),
                    after_hash: digest(b"candidate"),
                    backup_path: Some(backup),
                    state: PromotionFileState::Applied,
                    operation: CandidateOperation::Upsert,
                }],
            },
        )
        .unwrap();

        assert!(matches!(
            promote_recoverable(&candidate, control.path()),
            Err(Error::Conflict(message)) if message.contains("later workspace change")
        ));
        assert_eq!(
            fs::read_to_string(target.path().join("a.txt")).unwrap(),
            "human edit"
        );
    }

    #[test]
    fn completed_transaction_is_bound_to_the_selected_file_set() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.txt"), "a").unwrap();
        fs::write(source.path().join("b.txt"), "b").unwrap();
        let file = |path: &str, bytes: &[u8]| CandidateFile {
            path: path.into(),
            candidate_hash: digest(bytes),
            base_hash: None,
            bytes: bytes.len() as u64,
            operation: CandidateOperation::Upsert,
        };
        let first = CandidateManifest {
            candidate_id: "selection".into(),
            source_root: source.path().into(),
            destination_root: target.path().into(),
            files: vec![file("a.txt", b"a")],
        };
        promote_recoverable(&first, control.path()).unwrap();
        let different_selection = CandidateManifest {
            files: vec![file("b.txt", b"b")],
            ..first
        };
        assert!(matches!(
            promote_recoverable(&different_selection, control.path()),
            Err(Error::InvalidPlan(message)) if message.contains("identity mismatch")
        ));
        assert!(!target.path().join("b.txt").exists());
    }

    #[test]
    fn scoped_undo_restores_changed_files_and_removes_new_files() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        fs::write(source.path().join("changed.txt"), "after").unwrap();
        fs::write(source.path().join("new.txt"), "new").unwrap();
        fs::write(target.path().join("changed.txt"), "before").unwrap();
        let candidate = CandidateManifest {
            candidate_id: "undoable".into(),
            source_root: source.path().into(),
            destination_root: target.path().into(),
            files: vec![
                CandidateFile {
                    path: "changed.txt".into(),
                    candidate_hash: digest(b"after"),
                    base_hash: Some(digest(b"before")),
                    bytes: 5,
                    operation: CandidateOperation::Upsert,
                },
                CandidateFile {
                    path: "new.txt".into(),
                    candidate_hash: digest(b"new"),
                    base_hash: None,
                    bytes: 3,
                    operation: CandidateOperation::Upsert,
                },
            ],
        };
        promote_recoverable(&candidate, control.path()).unwrap();
        let receipt = undo_promotion("undoable", control.path()).unwrap();
        assert_eq!(receipt.restored, vec!["changed.txt", "new.txt"]);
        assert_eq!(
            fs::read_to_string(target.path().join("changed.txt")).unwrap(),
            "before"
        );
        assert!(!target.path().join("new.txt").exists());
        assert_eq!(
            load_transaction(&control.path().join("undoable/journal.json"))
                .unwrap()
                .state,
            PromotionTransactionState::Undone
        );
        assert_eq!(undo_promotion("undoable", control.path()).unwrap(), receipt);
    }

    #[test]
    fn scoped_undo_refuses_to_erase_post_acceptance_edits() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        fs::write(source.path().join("draft.txt"), "accepted").unwrap();
        let candidate = CandidateManifest {
            candidate_id: "undo-conflict".into(),
            source_root: source.path().into(),
            destination_root: target.path().into(),
            files: vec![CandidateFile {
                path: "draft.txt".into(),
                candidate_hash: digest(b"accepted"),
                base_hash: None,
                bytes: 8,
                operation: CandidateOperation::Upsert,
            }],
        };
        promote_recoverable(&candidate, control.path()).unwrap();
        fs::write(target.path().join("draft.txt"), "human changed it").unwrap();
        assert!(matches!(
            undo_promotion("undo-conflict", control.path()),
            Err(Error::Conflict(message)) if message.contains("later workspace change")
        ));
        assert_eq!(
            fs::read_to_string(target.path().join("draft.txt")).unwrap(),
            "human changed it"
        );
    }

    #[test]
    fn scoped_undo_resumes_after_restore_before_progress_was_recorded() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        fs::write(source.path().join("draft.txt"), "after").unwrap();
        fs::write(target.path().join("draft.txt"), "before").unwrap();
        let candidate = CandidateManifest {
            candidate_id: "undo-resume".into(),
            source_root: source.path().into(),
            destination_root: target.path().into(),
            files: vec![CandidateFile {
                path: "draft.txt".into(),
                candidate_hash: digest(b"after"),
                base_hash: Some(digest(b"before")),
                bytes: 5,
                operation: CandidateOperation::Upsert,
            }],
        };
        promote_recoverable(&candidate, control.path()).unwrap();
        let journal_path = control.path().join("undo-resume/journal.json");
        let mut journal = load_transaction(&journal_path).unwrap();
        journal.state = PromotionTransactionState::Undoing;
        journal.files[0].state = PromotionFileState::Undoing;
        write_transaction(&journal_path, &journal).unwrap();
        fs::write(target.path().join("draft.txt"), "before").unwrap();

        undo_promotion("undo-resume", control.path()).unwrap();
        assert_eq!(
            load_transaction(&journal_path).unwrap().state,
            PromotionTransactionState::Undone
        );
        assert_eq!(
            fs::read_to_string(target.path().join("draft.txt")).unwrap(),
            "before"
        );
    }

    #[test]
    fn revised_candidate_can_review_accept_and_undo_a_deletion() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        let control = tempfile::tempdir().unwrap();
        fs::write(target.path().join("obsolete.txt"), "workspace original").unwrap();
        fs::write(source.path().join("obsolete.txt"), "draft version").unwrap();
        let first = freeze_candidate(
            "delete-v1",
            source.path(),
            target.path(),
            &store.path().join("v1"),
        )
        .unwrap();
        let task = store.path().join("task");
        prepare_revision_copy(&first, &task).unwrap();
        fs::remove_file(task.join("obsolete.txt")).unwrap();
        let deletion =
            freeze_revision_candidate("delete-v2", &task, &first, &store.path().join("v2"))
                .unwrap();
        assert_eq!(deletion.files.len(), 1);
        assert_eq!(deletion.files[0].operation, CandidateOperation::Delete);
        let receipt = promote_recoverable(&deletion, control.path()).unwrap();
        assert_eq!(receipt.deleted, vec!["obsolete.txt"]);
        assert!(!target.path().join("obsolete.txt").exists());
        undo_promotion("delete-v2", control.path()).unwrap();
        assert_eq!(
            fs::read_to_string(target.path().join("obsolete.txt")).unwrap(),
            "workspace original"
        );
    }

    #[test]
    fn durable_records_are_append_only_and_replayable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sandbox").join("records.jsonl");
        let plan = EnvironmentPlan {
            id: "env-1".into(),
            outcome_revision: 7,
            input_root: dir.path().into(),
            task_root: dir.path().join("task"),
            backend: "local".into(),
            image: None,
            network_policy: "none".into(),
            setup_recipe: vec!["make".into()],
        };
        let record = DurableRecord::Environment(EnvironmentRecord {
            record_id: "r-1".into(),
            environment_id: "env-1".into(),
            state: EnvironmentState::Ready,
            plan,
            updated_at: "2026-09-08T00:00:00Z".into(),
            detail: None,
        });
        append_record(&path, &record).unwrap();
        append_record(&path, &record).unwrap();
        assert_eq!(load_records(&path).unwrap().len(), 2);
    }
}
