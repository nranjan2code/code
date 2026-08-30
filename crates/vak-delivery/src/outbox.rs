//! Append-preserving delivery outbox. Jobs are persisted before transport.

use crate::{DeliveryJob, DeliveryPacket};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxRecord {
    pub schema_version: u16,
    pub job: DeliveryJob,
    pub state: OutboxState,
    pub attempts: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub packet: Option<DeliveryPacket>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    Pending,
    Delivered,
    DeadLetter,
}

#[derive(Debug)]
pub enum OutboxError {
    Io(String),
    InvalidRecord(String),
    Conflict(String),
}

impl std::fmt::Display for OutboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) => write!(formatter, "delivery outbox I/O failed: {message}"),
            Self::InvalidRecord(message) => write!(formatter, "invalid outbox record: {message}"),
            Self::Conflict(message) => write!(formatter, "delivery outbox conflict: {message}"),
        }
    }
}

impl std::error::Error for OutboxError {}

#[derive(Debug, Clone)]
pub struct Outbox {
    root: PathBuf,
    // Guards `update`'s read-modify-append against concurrent callers within
    // this process. `update` has no file-level lock of its own, so without
    // this, correctness would rest entirely on every caller happening to
    // serialize through some *other* shared mutex (as `DeliveryRuntime`'s
    // `serial` field does today) — an invariant invisible from this module
    // and easy for a future call site to violate, reintroducing a classic
    // lost-update race (two updates read the same `attempts`/state, the
    // second overwrites the first). `Arc` so every `Outbox` clone sharing
    // this `root` also shares the lock, not just the original instance.
    // This still does not protect against a second *process* writing the
    // same `root` concurrently — that would need a real file lock (e.g.
    // flock), which nothing in this workspace does today because there is
    // exactly one `Outbox` per `root` per process in practice.
    lock: Arc<Mutex<()>>,
}

impl Outbox {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn enqueue(&self, job: DeliveryJob) -> Result<OutboxRecord, OutboxError> {
        fs::create_dir_all(&self.root).map_err(io_error)?;
        let path = self.record_path(&job.job_id);
        if path.exists() {
            let existing = read_record(&path)?;
            if existing.job == job {
                return Ok(existing);
            }
            return Err(OutboxError::Conflict(format!(
                "job id {} already exists with different content",
                job.job_id
            )));
        }
        let now = epoch_millis();
        let record = OutboxRecord {
            schema_version: 1,
            job,
            state: OutboxState::Pending,
            attempts: 0,
            created_at_ms: now,
            updated_at_ms: now,
            packet: None,
            last_error: None,
        };
        create_record(&path, &record)?;
        Ok(record)
    }

    pub fn pending(&self) -> Result<Vec<OutboxRecord>, OutboxError> {
        let mut records = self.list_filtered(|record| record.state == OutboxState::Pending)?;
        // Replay is oldest-first so a busy outbox cannot starve its earliest
        // durable job behind a stream of newer alerts.
        records.sort_by_key(|record| (record.created_at_ms, record.job.job_id.clone()));
        Ok(records)
    }

    /// Read every persisted delivery record, newest updates first. The
    /// operations console uses this for evidence and replay; the source of
    /// truth remains the append-preserving JSON files.
    pub fn list(&self) -> Result<Vec<OutboxRecord>, OutboxError> {
        self.list_filtered(|_| true)
    }

    pub fn get(&self, job_id: &str) -> Result<OutboxRecord, OutboxError> {
        read_record(&self.record_path(job_id))
    }

    fn list_filtered(
        &self,
        include: impl Fn(&OutboxRecord) -> bool,
    ) -> Result<Vec<OutboxRecord>, OutboxError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record = read_record(&path)?;
            if include(&record) {
                records.push(record);
            }
        }
        records.sort_by_key(|record| (record.updated_at_ms, record.job.job_id.clone()));
        records.reverse();
        Ok(records)
    }

    pub fn mark_delivered(
        &self,
        job_id: &str,
        packet: DeliveryPacket,
    ) -> Result<OutboxRecord, OutboxError> {
        self.update(job_id, |record| {
            record.state = OutboxState::Delivered;
            record.attempts = record.attempts.saturating_add(1);
            record.packet = Some(packet);
            record.last_error = None;
        })
    }

    pub fn mark_failed(
        &self,
        job_id: &str,
        error: impl Into<String>,
    ) -> Result<OutboxRecord, OutboxError> {
        let error = error.into();
        self.update(job_id, |record| {
            record.attempts = record.attempts.saturating_add(1);
            record.last_error = Some(error);
        })
    }

    pub fn mark_dead_letter(
        &self,
        job_id: &str,
        error: impl Into<String>,
    ) -> Result<OutboxRecord, OutboxError> {
        let error = error.into();
        self.update(job_id, |record| {
            record.state = OutboxState::DeadLetter;
            record.attempts = record.attempts.saturating_add(1);
            record.last_error = Some(error);
        })
    }

    fn update(
        &self,
        job_id: &str,
        mutate: impl FnOnce(&mut OutboxRecord),
    ) -> Result<OutboxRecord, OutboxError> {
        // Serialize the whole read-modify-append so two concurrent updates
        // (e.g. a replay tick and a direct `deliver` call racing on the
        // same job) can't both read the pre-mutation record and have the
        // second overwrite the first's change. See the `lock` field doc.
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let path = self.record_path(job_id);
        let mut record = read_record(&path)?;
        mutate(&mut record);
        record.updated_at_ms = epoch_millis();
        append_record(&path, &record)?;
        Ok(record)
    }

    fn record_path(&self, job_id: &str) -> PathBuf {
        self.root.join(format!("{}.json", hex_name(job_id)))
    }
}

fn read_record(path: &Path) -> Result<OutboxRecord, OutboxError> {
    let bytes = fs::read(path).map_err(io_error)?;
    let record: OutboxRecord = match serde_json::from_slice(&bytes) {
        Ok(record) => record,
        Err(full_error) => bytes
            .split(|byte| *byte == b'\n')
            .rev()
            .filter(|line| !line.is_empty())
            .find_map(|line| serde_json::from_slice(line).ok())
            .ok_or_else(|| OutboxError::InvalidRecord(full_error.to_string()))?,
    };
    if record.schema_version != 1 {
        return Err(OutboxError::InvalidRecord(format!(
            "unsupported schema version {}",
            record.schema_version
        )));
    }
    Ok(record)
}

fn create_record(path: &Path, record: &OutboxRecord) -> Result<(), OutboxError> {
    let bytes = record_line(record)?;
    let parent = path
        .parent()
        .ok_or_else(|| OutboxError::Io("record path has no parent".into()))?;
    fs::create_dir_all(parent).map_err(io_error)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    sync_directory(parent)
}

fn append_record(path: &Path, record: &OutboxRecord) -> Result<(), OutboxError> {
    let bytes = record_line(record)?;
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)
}

fn record_line(record: &OutboxRecord) -> Result<Vec<u8>, OutboxError> {
    let mut bytes = serde_json::to_vec(record)
        .map_err(|error| OutboxError::InvalidRecord(error.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), OutboxError> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error)
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), OutboxError> {
    Ok(())
}

fn hex_name(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn io_error(error: std::io::Error) -> OutboxError {
    OutboxError::Io(error.to_string())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{AnswerDraft, DeliveryContent, DeliveryKind, DeliveryProfile};

    fn test_job() -> DeliveryJob {
        DeliveryJob {
            job_id: "job/one".into(),
            target: "log:test".into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown("exact **answer**")),
            profile: DeliveryProfile::plain("log"),
        }
    }

    #[test]
    fn preserves_job_until_delivered() {
        let root = std::env::temp_dir().join(format!(
            "vak-delivery-outbox-{}-{}",
            std::process::id(),
            epoch_millis()
        ));
        let outbox = Outbox::new(&root);
        let job = test_job();
        outbox.enqueue(job.clone()).expect("enqueue");
        assert_eq!(outbox.pending().expect("pending")[0].job, job);
        let packet = crate::render(&job).expect("render");
        outbox.mark_delivered(&job.job_id, packet).expect("mark");
        assert!(outbox.pending().expect("pending").is_empty());
        let record_path = std::fs::read_dir(&root)
            .expect("records")
            .next()
            .expect("record entry")
            .expect("record path")
            .path();
        let history = std::fs::read_to_string(&record_path).expect("history");
        assert_eq!(history.lines().count(), 2);
        assert!(history.contains("exact **answer**"));
        std::fs::OpenOptions::new()
            .append(true)
            .open(record_path)
            .expect("open partial")
            .write_all(b"{\"partial\":")
            .expect("write partial");
        assert!(
            outbox.pending().expect("pending after partial").is_empty(),
            "a torn final append must not hide the last valid state"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
