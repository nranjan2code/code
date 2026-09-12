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
pub struct CandidateFile {
    pub path: String,
    pub candidate_hash: String,
    pub base_hash: Option<String>,
    pub bytes: u64,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationResult {
    pub path: String,
    pub status: String,
    pub evidence: String,
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
    pub environment_id: String,
    pub candidate: CandidateManifest,
    pub verified: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionRecord {
    pub record_id: String,
    pub candidate_id: String,
    pub receipt: PromotionReceipt,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "record")]
pub enum DurableRecord {
    Environment(EnvironmentRecord),
    Candidate(CandidateRecord),
    Promotion(PromotionRecord),
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

pub fn promote(candidate: &CandidateManifest) -> Result<PromotionReceipt, Error> {
    let mut staged = Vec::new();
    let mut before_hashes = Vec::new();
    for file in &candidate.files {
        let source = confined(&candidate.source_root, &file.path)?;
        let target = confined(&candidate.destination_root, &file.path)?;
        let bytes = fs::read(&source).map_err(|_| Error::Missing(file.path.clone()))?;
        if digest(&bytes) != file.candidate_hash {
            return Err(Error::CandidateChanged(file.path.clone()));
        }
        let before = target
            .is_file()
            .then(|| fs::read(&target).ok())
            .flatten()
            .map(|b| digest(&b));
        if before != file.base_hash {
            return Err(Error::Conflict(file.path.clone()));
        }
        before_hashes.push((file.path.clone(), before));
        staged.push((file.path.clone(), target, bytes));
    }
    let mut applied = Vec::new();
    let mut after_hashes = Vec::new();
    let mut verification = Vec::new();
    for (path, target, bytes) in staged {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = target.with_extension(format!("vak-promotion-{}", candidate.candidate_id));
        fs::write(&temporary, &bytes)?;
        fs::rename(&temporary, &target)?;
        applied.push(path.clone());
        let after = digest(&bytes);
        after_hashes.push((path.clone(), after.clone()));
        let observed = fs::read(&target).ok().map(|written| digest(&written));
        if observed.as_deref() != Some(after.as_str()) {
            return Err(Error::Conflict(format!(
                "post-apply verification failed: {path}"
            )));
        }
        verification.push(VerificationResult {
            path,
            status: "observed".into(),
            evidence: format!("destination hash verified: {after}"),
        });
    }
    Ok(PromotionReceipt {
        candidate_id: candidate.candidate_id.clone(),
        applied,
        before_hashes,
        after_hashes,
        verification,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
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
            }],
        };
        assert!(matches!(promote(&candidate), Err(Error::PathEscape(_))));
        assert!(!target.path().join("candidate.txt").exists());
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
