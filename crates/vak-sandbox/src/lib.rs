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

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum CandidateOperation {
    #[default]
    Upsert,
    Delete,
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
    #[serde(default)]
    pub target_checks: Vec<TargetCheckPlan>,
    #[serde(default)]
    pub workspace_checks: Vec<WorkspaceCheckPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCheckPlan {
    pub id: String,
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetCheckPlan {
    pub verifier: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetCheckResult {
    pub verifier: String,
    pub path: String,
    pub status: String,
    pub evidence: String,
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
    #[serde(default)]
    pub target_checks: Vec<TargetCheckResult>,
}

pub trait TargetVerifier: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, path: &str) -> bool;
    fn verify(&self, path: &Path) -> Result<String, String>;
}

#[derive(Default)]
pub struct TargetVerifierRegistry {
    verifiers: Vec<Box<dyn TargetVerifier>>,
}

impl TargetVerifierRegistry {
    pub fn register(&mut self, verifier: impl TargetVerifier + 'static) {
        self.verifiers.push(Box::new(verifier));
    }

    pub fn plan(&self, candidate: &CandidateManifest) -> Vec<TargetCheckPlan> {
        candidate
            .files
            .iter()
            .filter(|file| file.operation == CandidateOperation::Upsert)
            .flat_map(|file| {
                self.verifiers
                    .iter()
                    .filter(|verifier| verifier.supports(&file.path))
                    .map(|verifier| TargetCheckPlan {
                        verifier: verifier.id().to_string(),
                        path: file.path.clone(),
                    })
            })
            .collect()
    }

    pub fn verify(
        &self,
        destination_root: &Path,
        checks: &[TargetCheckPlan],
    ) -> Vec<TargetCheckResult> {
        checks
            .iter()
            .map(|check| {
                let result = self
                    .verifiers
                    .iter()
                    .find(|verifier| verifier.id() == check.verifier)
                    .ok_or_else(|| "registered verifier is unavailable".to_string())
                    .and_then(|verifier| {
                        confined(destination_root, &check.path)
                            .map_err(|error| error.to_string())
                            .and_then(|path| verifier.verify(&path))
                    });
                match result {
                    Ok(evidence) => TargetCheckResult {
                        verifier: check.verifier.clone(),
                        path: check.path.clone(),
                        status: "passed".into(),
                        evidence,
                    },
                    Err(evidence) => TargetCheckResult {
                        verifier: check.verifier.clone(),
                        path: check.path.clone(),
                        status: "failed".into(),
                        evidence,
                    },
                }
            })
            .collect()
    }
}

pub struct JsonSyntaxVerifier;

impl TargetVerifier for JsonSyntaxVerifier {
    fn id(&self) -> &'static str {
        "format.json"
    }

    fn supports(&self, path: &str) -> bool {
        path.to_ascii_lowercase().ends_with(".json")
    }

    fn verify(&self, path: &Path) -> Result<String, String> {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        serde_json::from_slice::<serde_json::Value>(&bytes)
            .map(|_| "JSON parsed".into())
            .map_err(|error| format!("JSON parse failed: {error}"))
    }
}

/// Checks the browser-parsed document shape, not visual quality or script output.
/// In particular, an unclosed raw-text element such as <title> can swallow the
/// intended page while leaving the source file nonempty.
pub struct HtmlBodyVerifier;

impl TargetVerifier for HtmlBodyVerifier {
    fn id(&self) -> &'static str {
        "format.html.body"
    }

    fn supports(&self, path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        path.ends_with(".html") || path.ends_with(".htm")
    }

    fn verify(&self, path: &Path) -> Result<String, String> {
        const MAX_HTML_BYTES: u64 = 16 * 1024 * 1024;
        let size = fs::metadata(path).map_err(|error| error.to_string())?.len();
        if size > MAX_HTML_BYTES {
            return Err("HTML exceeds the 16 MiB structural check limit".into());
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let source = String::from_utf8_lossy(&bytes);
        let document = dom_query::Document::from(source.as_ref());
        let body = document.select("body");
        if body.is_empty() {
            return Err("HTML parser found no body".into());
        }
        let has_body_text = !body.text().trim().is_empty();
        let has_body_element = !body.select("*").is_empty();
        let has_script = !document.select("script").is_empty();
        if !has_body_text && !has_body_element && !has_script {
            return Err("HTML parser found no visible body content or script; check for an unclosed <title> or other raw-text element".into());
        }
        Ok("HTML parsed with body content or a script; visual output was not inspected".into())
    }
}

pub struct ImageDecodeVerifier;

impl TargetVerifier for ImageDecodeVerifier {
    fn id(&self) -> &'static str {
        "format.image-decode"
    }

    fn supports(&self, path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        [".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .any(|extension| path.ends_with(extension))
    }

    fn verify(&self, path: &Path) -> Result<String, String> {
        let format = image::ImageFormat::from_path(path)
            .map_err(|error| format!("image extension is unsupported: {error}"))?;
        let mut reader = image::ImageReader::open(path)
            .map_err(|error| format!("image could not be opened: {error}"))?;
        reader.set_format(format);
        let decoded = reader
            .decode()
            .map_err(|error| format!("image decode failed: {error}"))?;
        let width = decoded.width();
        let height = decoded.height();
        if width == 0 || height == 0 {
            return Err("decoded image has zero width or height".into());
        }
        Ok(format!("decoded {width}×{height} image"))
    }
}

/// Opens a PDF candidate through `vak-pdf`: bounded parsing of the header,
/// cross-reference data, catalog and page tree, and a full read of every
/// page's text. A file named `.pdf` that is not a PDF fails.
pub struct PdfStructureVerifier;

impl TargetVerifier for PdfStructureVerifier {
    fn id(&self) -> &'static str {
        "format.pdf-structure"
    }

    fn supports(&self, path: &str) -> bool {
        vak_pdf::is_pdf_path(path)
    }

    fn verify(&self, path: &Path) -> Result<String, String> {
        let limits = vak_pdf::Limits::default();
        let size = fs::metadata(path).map_err(|error| error.to_string())?.len();
        if size > limits.max_file_bytes {
            return Err(vak_pdf::Error::TooLarge {
                bytes: size,
                limit: limits.max_file_bytes,
            }
            .to_string());
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let document = vak_pdf::read(&bytes, limits).map_err(|error| error.to_string())?;
        let stats = document
            .stats()
            .iter()
            .map(|(name, count)| format!("{count} {name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut evidence = format!("PDF {} parsed; {stats}", document.version);
        let flags = document.inspection.flags();
        if !flags.is_empty() {
            evidence.push_str("; flags: ");
            evidence.push_str(&flags.join("; "));
        }
        if !document.not_read.is_empty() {
            evidence.push_str("; not read: ");
            evidence.push_str(&document.not_read.join("; "));
        }
        evidence.push_str("; signatures and rendering were not checked");
        Ok(evidence)
    }
}

/// Opens a candidate of the Open XML family through `vak-ooxml`: bounded
/// package reading, detection by main-part content type, the main-part
/// root, and a full read projection. A package whose content type names a
/// different format than its extension (a macro package renamed `.docx`)
/// fails, because the name is what a person trusts before opening it.
pub struct OpenXmlPackageVerifier;

impl TargetVerifier for OpenXmlPackageVerifier {
    fn id(&self) -> &'static str {
        "format.openxml"
    }

    fn supports(&self, path: &str) -> bool {
        vak_ooxml::is_openxml_path(path)
    }

    fn verify(&self, path: &Path) -> Result<String, String> {
        let file = fs::File::open(path).map_err(|error| error.to_string())?;
        let mut package = vak_ooxml::Package::open(file, vak_ooxml::Limits::default())
            .map_err(|error| error.to_string())?;
        let format = package.format();
        let named = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if named != format.extension() {
            return Err(format!(
                "package is a {} (.{}) but is named .{named}",
                format.vocabulary.label(),
                format.extension()
            ));
        }
        let document = vak_ooxml::read::project(&mut package).map_err(|error| error.to_string())?;
        if !document.inspection.untyped_parts.is_empty() {
            return Err(format!(
                "package has part(s) without a content type: {}",
                document.inspection.untyped_parts.join(", ")
            ));
        }
        let stats = document
            .stats
            .iter()
            .map(|(name, count)| format!("{count} {name}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut evidence = format!(
            "Open XML {} (.{}, {:?}) opened with {} parts; main part {} parsed; {stats}",
            format.vocabulary.label(),
            format.extension(),
            document.inspection.conformance,
            document.inspection.part_count,
            document.inspection.main_part,
        );
        let flags = document.inspection.flags();
        if !flags.is_empty() {
            evidence.push_str("; flags: ");
            evidence.push_str(&flags.join("; "));
        }
        evidence.push_str("; schema conformance and rendering were not checked");
        Ok(evidence)
    }
}

pub struct DelimitedDataVerifier;

impl TargetVerifier for DelimitedDataVerifier {
    fn id(&self) -> &'static str {
        "data.delimited"
    }

    fn supports(&self, path: &str) -> bool {
        let path = path.to_ascii_lowercase();
        path.ends_with(".csv") || path.ends_with(".tsv")
    }

    fn verify(&self, path: &Path) -> Result<String, String> {
        let delimiter = if path
            .to_string_lossy()
            .to_ascii_lowercase()
            .ends_with(".tsv")
        {
            b'\t'
        } else {
            b','
        };
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .flexible(false)
            .from_path(path)
            .map_err(|error| format!("delimited data could not be opened: {error}"))?;
        let columns = reader
            .headers()
            .map_err(|error| format!("header parse failed: {error}"))?
            .len();
        if columns == 0 {
            return Err("delimited data has no columns".into());
        }
        let mut rows = 0_u64;
        for record in reader.records() {
            record.map_err(|error| format!("record parse failed: {error}"))?;
            rows += 1;
        }
        Ok(format!(
            "parsed {rows} data row(s) with {columns} consistent column(s)"
        ))
    }
}

pub struct SvgStructureVerifier;

impl TargetVerifier for SvgStructureVerifier {
    fn id(&self) -> &'static str {
        "format.svg"
    }

    fn supports(&self, path: &str) -> bool {
        path.to_ascii_lowercase().ends_with(".svg")
    }

    fn verify(&self, path: &Path) -> Result<String, String> {
        let mut reader = quick_xml::Reader::from_file(path)
            .map_err(|error| format!("SVG could not be opened: {error}"))?;
        reader.config_mut().trim_text(true);
        let mut buffer = Vec::new();
        loop {
            match reader.read_event_into(&mut buffer) {
                Ok(quick_xml::events::Event::Start(element))
                | Ok(quick_xml::events::Event::Empty(element)) => {
                    let name = element.local_name();
                    return if name.as_ref() == b"svg" {
                        Ok("parsed SVG root".into())
                    } else {
                        Err(format!(
                            "XML root is {}, expected svg",
                            String::from_utf8_lossy(name.as_ref())
                        ))
                    };
                }
                Ok(quick_xml::events::Event::Eof) => return Err("SVG has no root element".into()),
                Ok(_) => {}
                Err(error) => return Err(format!("SVG parse failed: {error}")),
            }
            buffer.clear();
        }
    }
}

pub fn default_target_verifiers() -> TargetVerifierRegistry {
    let mut registry = TargetVerifierRegistry::default();
    registry.register(JsonSyntaxVerifier);
    registry.register(HtmlBodyVerifier);
    registry.register(ImageDecodeVerifier);
    registry.register(PdfStructureVerifier);
    registry.register(OpenXmlPackageVerifier);
    registry.register(DelimitedDataVerifier);
    registry.register(SvgStructureVerifier);
    registry
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
pub struct PreviewPreparationRecord {
    pub record_id: String,
    pub session_id: String,
    pub result_id: String,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub environment_id: String,
    pub state: EnvironmentState,
    pub command: String,
    pub evidence: String,
    pub updated_at: String,
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
    /// Format evidence observed from the frozen draft bytes. Acceptance runs
    /// the same planned checks again against the applied workspace state.
    #[serde(default)]
    pub draft_checks: Vec<TargetCheckResult>,
    pub updated_at: String,
    /// The saved version used as input for a human-requested revision.
    #[serde(default)]
    pub parent_candidate_id: Option<String>,
    /// Durable child session whose tool receipts produced this version.
    #[serde(default)]
    pub revision_session_id: Option<String>,
    /// Set when a person kept only some of an Office draft's changes: this
    /// version replays those, and `parent_candidate_id` is the full draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrowed: Option<NarrowedDraft>,
}

/// Which of an Office draft's changes a narrowed version keeps
/// (docs/design/72-openxml-documents.md, P3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NarrowedDraft {
    pub path: String,
    pub keep: Vec<String>,
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
    #[serde(default)]
    pub workspace_checks: Vec<WorkspaceCheckPlan>,
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
pub struct WorkspaceCheckRecord {
    pub record_id: String,
    pub session_id: String,
    pub candidate_id: String,
    pub applied_state_digest: String,
    pub check: WorkspaceCheckPlan,
    pub status: String,
    pub evidence: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "record")]
pub enum DurableRecord {
    Environment(EnvironmentRecord),
    PreviewPreparation(PreviewPreparationRecord),
    Candidate(CandidateRecord),
    Promotion(PromotionRecord),
    PromotionUndo(PromotionUndoRecord),
    WorkspaceCheck(WorkspaceCheckRecord),
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
        target_checks: Vec::new(),
        workspace_checks: Vec::new(),
    })
}

fn protect_frozen_tree(root: &Path) -> Result<(), Error> {
    let mut directories = Vec::new();
    for item in WalkDir::new(root).follow_links(false) {
        let item = item.map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
        let path = item.path();
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() {
            return Err(Error::PathEscape(path.display().to_string()));
        }
        if metadata.is_dir() {
            directories.push(path.to_path_buf());
            continue;
        }
        let mut permissions = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() & 0o555);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)?;
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in directories {
        let mut permissions = fs::symlink_metadata(&path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() & 0o555);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

pub fn remove_frozen_candidate(root: &Path) -> Result<(), Error> {
    if !root.exists() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|entry| {
            entry
                .map(|entry| entry.path().to_path_buf())
                .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))
        })
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|path| path.components().count());
    for path in entries {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let mut permissions = metadata.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o700);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    fs::remove_dir_all(root)?;
    Ok(())
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
        protect_frozen_tree(frozen_root)
    })();
    if let Err(error) = copy {
        let _ = remove_frozen_candidate(frozen_root);
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
        let _ = remove_frozen_candidate(frozen_root);
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

/// An Office draft a revision turn delivered: `draft`, under the task copy's
/// `.vak/scratch/`, is the next version of the task file `path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionDraft {
    pub path: String,
    pub draft: String,
}

/// Put each delivered draft in place of the task file it is a draft for, so
/// the revision's candidate is frozen from the task copy like any other
/// change. `office_apply` never writes the file it edits (it writes a draft
/// under `.vak/scratch/`, which a candidate never includes); in a revision the
/// task copy stands where the workspace stands in a conversation, and this
/// is its acceptance of the draft, made before the person reviews the new
/// version.
pub fn adopt_revision_drafts(task_root: &Path, drafts: &[RevisionDraft]) -> Result<(), Error> {
    for draft in drafts {
        if Path::new(&draft.path).starts_with(".vak") {
            return Err(Error::PathEscape(draft.path.clone()));
        }
        if !Path::new(&draft.draft).starts_with(".vak/scratch") {
            return Err(Error::PathEscape(draft.draft.clone()));
        }
        let source = confined(task_root, &draft.draft)?;
        let bytes = fs::read(&source).map_err(|_| Error::Missing(draft.draft.clone()))?;
        let target = confined(task_root, &draft.path)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&target)?;
        use std::io::Write;
        output.write_all(&bytes)?;
        output.sync_all()?;
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
            target_checks: Vec::new(),
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
        .truncate(false)
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
        .truncate(false)
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
    fn registered_target_verifiers_plan_and_check_applied_files() {
        let target = tempfile::tempdir().unwrap();
        fs::write(target.path().join("result.json"), br#"{"ready":true}"#).unwrap();
        fs::write(target.path().join("preview.png"), b"not a png").unwrap();
        let candidate = CandidateManifest {
            candidate_id: "formats".into(),
            source_root: target.path().into(),
            destination_root: target.path().into(),
            files: vec![
                CandidateFile {
                    path: "preview.png".into(),
                    candidate_hash: digest(b"not a png"),
                    base_hash: None,
                    bytes: 9,
                    operation: CandidateOperation::Upsert,
                },
                CandidateFile {
                    path: "result.json".into(),
                    candidate_hash: digest(br#"{"ready":true}"#),
                    base_hash: None,
                    bytes: 14,
                    operation: CandidateOperation::Upsert,
                },
            ],
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
        };
        let registry = default_target_verifiers();
        let plan = registry.plan(&candidate);
        assert_eq!(plan.len(), 2);
        let results = registry.verify(target.path(), &plan);
        assert!(
            results
                .iter()
                .any(|result| { result.path == "result.json" && result.status == "passed" })
        );
        assert!(
            results
                .iter()
                .any(|result| { result.path == "preview.png" && result.status == "failed" })
        );
    }

    #[test]
    fn openxml_verifier_covers_the_family_and_refuses_disguises() {
        use vak_ooxml::fixtures;

        let target = tempfile::tempdir().unwrap();
        let verifier = OpenXmlPackageVerifier;
        for (name, bytes, expected) in [
            ("report.docx", fixtures::docx(), "Word document (.docx"),
            ("book.xlsx", fixtures::xlsx(), "2 sheets"),
            ("deck.pptx", fixtures::pptx(), "2 slides"),
            ("flow.vsdx", fixtures::vsdx(), "1 pages"),
        ] {
            let path = target.path().join(name);
            fs::write(&path, bytes).unwrap();
            assert!(verifier.supports(name));
            let evidence = verifier.verify(&path).unwrap();
            assert!(evidence.contains(expected), "{evidence}");
            assert!(
                evidence.contains("rendering were not checked"),
                "{evidence}"
            );
        }
        let flagged = verifier.verify(&target.path().join("report.docx")).unwrap();
        assert!(flagged.contains("1 risky fields"), "{flagged}");

        let renamed = target.path().join("invoice.docx");
        fs::write(
            &renamed,
            fixtures::word_with(
                "application/vnd.ms-word.document.macroEnabled.main+xml",
                fixtures::MINIMAL_WORD_BODY,
                &[],
                &[],
                &[],
                &[],
            ),
        )
        .unwrap();
        let error = verifier.verify(&renamed).unwrap_err();
        assert!(error.contains("(.docm) but is named .docx"), "{error}");

        let wrong_root = target.path().join("wrong-root.docx");
        fs::write(
            &wrong_root,
            fixtures::word_with(fixtures::WORD_MAIN, "<workbook/>", &[], &[], &[], &[]),
        )
        .unwrap();
        assert!(verifier.verify(&wrong_root).is_err());

        let broken = target.path().join("broken.docx");
        fs::write(&broken, b"not a package").unwrap();
        assert!(verifier.verify(&broken).is_err());
        assert!(verifier.supports("macro.xlsm") && verifier.supports("stencil.VSSX"));
        assert!(!verifier.supports("legacy.doc") && !verifier.supports("binary.xlsb"));
    }

    #[test]
    fn image_verifier_decodes_pixels_instead_of_trusting_the_header() {
        let target = tempfile::tempdir().unwrap();
        let valid_path = target.path().join("preview.png");
        let corrupt_path = target.path().join("corrupt.png");
        let image = image::RgbImage::from_pixel(3, 2, image::Rgb([12, 34, 56]));
        image.save(&valid_path).unwrap();
        fs::write(&corrupt_path, b"\x89PNG\r\n\x1a\ncorrupt body").unwrap();

        let evidence = ImageDecodeVerifier.verify(&valid_path).unwrap();
        assert!(evidence.contains("3×2"));
        assert!(ImageDecodeVerifier.verify(&corrupt_path).is_err());
    }

    #[test]
    fn pdf_structure_verifier_requires_a_parseable_page_tree() {
        let target = tempfile::tempdir().unwrap();
        let complete = target.path().join("complete.pdf");
        let truncated = target.path().join("truncated.pdf");
        let renamed = target.path().join("renamed.pdf");
        fs::write(&complete, vak_pdf::fixtures::report()).unwrap();
        fs::write(&truncated, b"%PDF-1.7\nnot a document\n%%EOF\n").unwrap();
        fs::write(&renamed, vak_ooxml::fixtures::docx()).unwrap();

        assert!(PdfStructureVerifier.supports("Report.PDF"));
        let evidence = PdfStructureVerifier.verify(&complete).unwrap();
        assert!(evidence.contains("PDF 1.7 parsed; 2 pages"), "{evidence}");
        assert!(
            evidence.contains("1 JavaScript action (never run)"),
            "{evidence}"
        );
        assert!(
            evidence.contains("rendering were not checked"),
            "{evidence}"
        );
        assert!(PdfStructureVerifier.verify(&truncated).is_err());
        let refused = PdfStructureVerifier.verify(&renamed).unwrap_err();
        assert!(refused.contains("not a PDF"), "{refused}");
    }

    #[test]
    fn data_and_svg_verifiers_reject_structural_errors() {
        let target = tempfile::tempdir().unwrap();
        let csv = target.path().join("table.csv");
        let broken_csv = target.path().join("broken.csv");
        let svg = target.path().join("figure.svg");
        let broken_svg = target.path().join("broken.svg");
        fs::write(&csv, "name,value\nalpha,1\nbeta,2\n").unwrap();
        fs::write(&broken_csv, "name,value\nalpha\n").unwrap();
        fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><circle r="2"/></svg>"#,
        )
        .unwrap();
        fs::write(&broken_svg, "<html></html>").unwrap();

        assert!(DelimitedDataVerifier.verify(&csv).is_ok());
        assert!(DelimitedDataVerifier.verify(&broken_csv).is_err());
        assert!(SvgStructureVerifier.verify(&svg).is_ok());
        assert!(SvgStructureVerifier.verify(&broken_svg).is_err());
    }

    #[test]
    fn html_target_check_catches_a_page_swallowed_by_unclosed_title() {
        let target = tempfile::tempdir().unwrap();
        let valid = target.path().join("working.html");
        let script_root = target.path().join("script-root.htm");
        let swallowed = target.path().join("blank.html");
        fs::write(&valid, "<!doctype html><html><head><title>Draft</title></head><body><main>Ready</main></body></html>").unwrap();
        fs::write(&script_root, "<!doctype html><html><body><script>document.body.textContent = 'Ready'</script></body></html>").unwrap();
        fs::write(&swallowed, "<!doctype html><html><head><title>Draft</head><body><main>Missing</main></body></html>").unwrap();

        let candidate = CandidateManifest {
            candidate_id: "html-check".into(),
            source_root: target.path().into(),
            destination_root: target.path().into(),
            files: ["working.html", "script-root.htm", "blank.html"]
                .into_iter()
                .map(|path| CandidateFile {
                    path: path.into(),
                    candidate_hash: String::new(),
                    base_hash: None,
                    bytes: 0,
                    operation: CandidateOperation::Upsert,
                })
                .collect(),
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
        };
        let registry = default_target_verifiers();
        let checks = registry.plan(&candidate);
        assert_eq!(checks.len(), 3);
        let results = registry.verify(target.path(), &checks);
        assert_eq!(
            results
                .iter()
                .map(|result| result.status.as_str())
                .collect::<Vec<_>>(),
            vec!["passed", "passed", "failed"]
        );
        assert!(results[2].evidence.contains("unclosed <title>"));
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
    fn server_owned_cleanup_removes_a_protected_candidate_tree() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("nested")).unwrap();
        fs::write(source.path().join("nested/result.txt"), "reviewed").unwrap();
        let frozen = store.path().join("candidate-cleanup");
        freeze_candidate("candidate-cleanup", source.path(), target.path(), &frozen).unwrap();

        assert!(fs::write(frozen.join("nested/extra.txt"), "unreviewed").is_err());
        remove_frozen_candidate(&frozen).unwrap();
        assert!(!frozen.exists());
        remove_frozen_candidate(&frozen).unwrap();
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
        assert!(
            fs::write(
                saved.source_root.join("unlisted.txt"),
                "must not enter copy",
            )
            .is_err()
        );
        let copy = store.path().join("revision-copy");
        prepare_revision_copy(&saved, &copy).unwrap();
        assert_eq!(
            fs::read_to_string(copy.join("pages/index.html")).unwrap(),
            "version one"
        );
        assert!(!copy.join("unlisted.txt").exists());

        let saved_file = saved.source_root.join("pages/index.html");
        let mut permissions = fs::metadata(&saved_file).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(permissions.mode() | 0o200);
        }
        #[cfg(not(unix))]
        permissions.set_readonly(false);
        fs::set_permissions(&saved_file, permissions).unwrap();
        fs::write(&saved_file, "tampered").unwrap();
        let failed_copy = store.path().join("failed-copy");
        assert!(matches!(
            prepare_revision_copy(&saved, &failed_copy),
            Err(Error::CandidateChanged(path)) if path == "pages/index.html"
        ));
        assert!(!failed_copy.exists());
    }

    #[test]
    fn adopted_office_draft_becomes_the_next_version() {
        let source = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("letter.docx"), "original").unwrap();
        fs::write(source.path().join("letter.docx"), "version one").unwrap();
        let first = freeze_candidate(
            "v1",
            source.path(),
            workspace.path(),
            &store.path().join("v1"),
        )
        .unwrap();
        let task = store.path().join("task");
        prepare_revision_copy(&first, &task).unwrap();
        let drafts = task.join(".vak/scratch/vak/call-2");
        fs::create_dir_all(&drafts).unwrap();
        fs::write(drafts.join("letter.docx"), "version two").unwrap();

        // Left in scratch, the draft is not a change: this is the failure.
        assert!(matches!(
            freeze_revision_candidate("unchanged", &task, &first, &store.path().join("x")),
            Err(Error::InvalidPlan(reason)) if reason == "revision did not change candidate files"
        ));

        let adopted = RevisionDraft {
            path: "letter.docx".into(),
            draft: ".vak/scratch/vak/call-2/letter.docx".into(),
        };
        adopt_revision_drafts(&task, std::slice::from_ref(&adopted)).unwrap();
        let second =
            freeze_revision_candidate("v2", &task, &first, &store.path().join("v2")).unwrap();
        assert_eq!(second.files.len(), 1);
        assert_eq!(second.files[0].path, "letter.docx");
        assert_eq!(second.files[0].base_hash, first.files[0].base_hash);
        assert_eq!(
            fs::read_to_string(second.source_root.join("letter.docx")).unwrap(),
            "version two"
        );

        for refused in [
            RevisionDraft {
                path: ".vak/config.toml".into(),
                ..adopted.clone()
            },
            RevisionDraft {
                draft: "letter.docx".into(),
                ..adopted.clone()
            },
            RevisionDraft {
                draft: ".vak/scratch/../../letter.docx".into(),
                ..adopted.clone()
            },
        ] {
            assert!(matches!(
                adopt_revision_drafts(&task, &[refused]),
                Err(Error::PathEscape(_))
            ));
        }
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
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
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
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
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
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
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
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
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
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
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
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
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
            target_checks: Vec::new(),
            workspace_checks: Vec::new(),
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
        let preparation = DurableRecord::PreviewPreparation(PreviewPreparationRecord {
            record_id: "preview-1".into(),
            session_id: "session-1".into(),
            result_id: "result-1".into(),
            candidate_id: "candidate-1".into(),
            candidate_digest: "sha256:digest".into(),
            environment_id: "preview:candidate-1".into(),
            state: EnvironmentState::Ready,
            command: "npm ci --ignore-scripts --no-audit --no-fund".into(),
            evidence: "dependencies prepared".into(),
            updated_at: "2026-09-08T00:01:00Z".into(),
        });
        append_record(&path, &preparation).unwrap();
        assert_eq!(load_records(&path).unwrap(), vec![record, preparation]);
    }
}
