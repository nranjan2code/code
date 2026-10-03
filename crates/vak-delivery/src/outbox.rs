//! Append-preserving delivery outbox. Jobs are persisted before transport.

use crate::{DeliveryJob, DeliveryPacket};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};
use vak_session::chain::RecordChain;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(OutboxRecord, "outbox_record");

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
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = self.latest().remove(&job.job_id) {
            if existing.job == job {
                return Ok(existing);
            }
            return Err(OutboxError::Conflict(format!(
                "job id {} already exists with different content",
                job.job_id
            )));
        }
        let now = epoch_millis();
        let (trace, actor) = (job.trace.clone(), job.actor);
        let record = OutboxRecord {
            schema_version: 1,
            job,
            state: OutboxState::Pending,
            attempts: 0,
            created_at_ms: now,
            updated_at_ms: now,
            packet: None,
            last_error: None,
            trace,
            actor,
        };
        self.append(&record)?;
        Ok(record)
    }

    pub fn pending(&self) -> Result<Vec<OutboxRecord>, OutboxError> {
        let mut records = self.list_filtered(|record| record.state == OutboxState::Pending)?;
        // Replay is oldest-first so a busy outbox cannot starve its earliest
        // durable job behind a stream of newer alerts.
        records.sort_by_key(|record| (record.created_at_ms, record.job.job_id.clone()));
        Ok(records)
    }

    /// Every job's current record, newest updates first. The operations
    /// console uses this for evidence and replay; the source of truth is the
    /// outbox chain, where each state change is a row.
    pub fn list(&self) -> Result<Vec<OutboxRecord>, OutboxError> {
        self.list_filtered(|_| true)
    }

    pub fn get(&self, job_id: &str) -> Result<OutboxRecord, OutboxError> {
        self.latest()
            .remove(job_id)
            .ok_or_else(|| OutboxError::Io(format!("no outbox job {job_id}")))
    }

    fn list_filtered(
        &self,
        include: impl Fn(&OutboxRecord) -> bool,
    ) -> Result<Vec<OutboxRecord>, OutboxError> {
        let mut records: Vec<OutboxRecord> = self
            .latest()
            .into_values()
            .filter(|record| include(record))
            .collect();
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
        let mut record = self.get(job_id)?;
        mutate(&mut record);
        record.updated_at_ms = epoch_millis();
        self.append(&record)?;
        Ok(record)
    }

    /// The newest record of every job. A row that is not a record of the
    /// supported schema is skipped, so it never hides a job's last state.
    fn latest(&self) -> HashMap<String, OutboxRecord> {
        let mut latest = HashMap::new();
        RecordChain::at(&self.root).scan(|record: OutboxRecord| {
            if record.schema_version == 1 {
                latest.insert(record.job.job_id.clone(), record);
            }
            true
        });
        latest
    }

    fn append(&self, record: &OutboxRecord) -> Result<(), OutboxError> {
        RecordChain::at(&self.root)
            .append(record)
            .map_err(|error| OutboxError::Io(error.to_string()))
    }
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
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
            skill_registry: None,
            trace: None,
            actor: None,
        }
    }

    #[test]
    fn a_job_written_under_a_run_names_it_on_its_record() {
        use vak_session::ids::{AgentId, PrincipalId, SpaceId, TenantId};
        use vak_session::trace::{Cause, TraceKey};
        let actor = PrincipalId::new();
        let key = TraceKey::root(
            TenantId::new(),
            SpaceId::new(),
            AgentId::new(),
            Cause::Heartbeat,
        )
        .acting(actor, None);
        let root = std::env::temp_dir().join(format!(
            "vak-delivery-outbox-trace-{}-{}",
            std::process::id(),
            epoch_millis()
        ));
        let outbox = Outbox::new(&root);
        let mut job = test_job();
        job.trace = Some(key.clone());
        job.actor = Some(actor);
        let record = outbox.enqueue(job.clone()).expect("enqueue");
        assert_eq!(record.trace.as_ref().map(|t| t.run), Some(key.run));
        assert_eq!(record.actor, Some(actor));
        let packet = crate::render(&job).expect("render");
        assert_eq!(packet.trace.as_ref().map(|t| t.run), Some(key.run));
        assert_eq!(packet.actor, Some(actor));
        let _ = std::fs::remove_dir_all(&root);
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
        let history = RecordChain::at(&root).text();
        assert_eq!(history.lines().count(), 2, "each state change is a row");
        assert!(history.contains("exact **answer**"));
        RecordChain::at(&root)
            .append(&serde_json::json!({ "partial": true }))
            .expect("append a foreign row");
        assert!(
            outbox
                .pending()
                .expect("pending after foreign row")
                .is_empty(),
            "a row that is not a record must not hide the last valid state"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
