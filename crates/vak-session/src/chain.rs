//! A side ledger as a record chain (plan M3b slice 3): an append-only
//! directory of `vak-storage` record segments, hash-chained across seals.
//! The one way a side ledger (costs, routing evidence, security events,
//! the inbox and the rest) is written; it is never rewritten or compacted.
//!
//! Appends from any process serialise on the directory's `LOCK`. Each
//! append opens the active segment, verifies it and cuts a torn tail, so
//! its cost is bounded by [`ROTATE_BYTES`], not by the ledger's history:
//! a segment that reaches it is sealed and the next one starts.

use crate::types::SessionError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};

/// A segment this large is sealed and the next one opened.
pub const ROTATE_BYTES: u64 = 256 * 1024;

fn storage_error(error: vak_storage::StorageError) -> SessionError {
    match error {
        vak_storage::StorageError::Io(io) => SessionError::Io(io),
        other => SessionError::Corrupt {
            line: 0,
            message: other.to_string(),
        },
    }
}

/// One segment of a chain, with its rows as JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub number: u64,
    /// Still being appended to.
    pub open: bool,
    pub bytes: u64,
    pub rows: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordChain {
    dir: PathBuf,
    /// The key scope a chain that belongs to one conversation is sealed
    /// under; `None` for a chain many conversations share.
    scope: Option<String>,
}

impl RecordChain {
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            scope: None,
        }
    }

    /// The chain as one conversation's own: every frame is sealed under
    /// that conversation's key, so the chain goes with it (plan M7a-b). A
    /// reader needs no such call; the chain's directory names its scope.
    pub fn of_conversation(mut self, session_id: &str) -> Self {
        self.scope = Some(crate::objects::conversation_scope(session_id));
        self
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Whether anything has been appended.
    pub fn exists(&self) -> bool {
        !crate::log::segment_numbers(&self.dir).is_empty()
    }

    pub fn append<T: Serialize>(&self, row: &T) -> Result<(), SessionError> {
        self.write(std::slice::from_ref(row), true)
    }

    /// Appends `rows` in order with one sync.
    pub fn append_all<T: Serialize>(&self, rows: &[T]) -> Result<(), SessionError> {
        self.write(rows, true)
    }

    /// Appends without syncing: for telemetry, where losing the last rows
    /// to a power cut costs nothing. The rows reach disk with the next
    /// sync of this chain or when the system writes them back.
    pub fn append_relaxed<T: Serialize>(&self, row: &T) -> Result<(), SessionError> {
        self.write(std::slice::from_ref(row), false)
    }

    /// Appends `rows` with one sync only if nothing was appended after
    /// `after`, the position the caller decided them from (a rollup's
    /// [`crate::rollup::Rollup::read_at`]): the check and the append are
    /// one step under the chain's writer lock, which every process takes,
    /// so a decision made from what a chain held is never written over
    /// what another writer added since. `false` when the chain moved: read
    /// it again and decide again.
    pub fn append_all_after<T: Serialize>(
        &self,
        after: crate::tail::Position,
        rows: &[T],
    ) -> Result<bool, SessionError> {
        self.write_if(rows, true, Some(after))
    }

    /// Appends what `decide` returns, deciding and appending as one step
    /// under the chain's writer lock, which every process takes: `decide`
    /// reads what the chain holds (through its rollup) and may do work of
    /// its own that must not race a row another writer would add, such as
    /// saving a credential only while an account is still connected. Its
    /// error appends nothing and is handed back.
    pub fn append_decided<T: Serialize, E>(
        &self,
        decide: impl FnOnce() -> Result<Vec<T>, E>,
    ) -> Result<Result<(), E>, SessionError> {
        let mut refused = None;
        self.write_with(true, None, || match decide() {
            Ok(rows) => rows,
            Err(error) => {
                refused = Some(error);
                Vec::new()
            }
        })?;
        Ok(refused.map_or(Ok(()), Err))
    }

    fn write<T: Serialize>(&self, rows: &[T], sync: bool) -> Result<(), SessionError> {
        self.write_if(rows, sync, None).map(|_| ())
    }

    fn write_if<T: Serialize>(
        &self,
        rows: &[T],
        sync: bool,
        after: Option<crate::tail::Position>,
    ) -> Result<bool, SessionError> {
        if rows.is_empty() {
            return Ok(true);
        }
        self.write_with(sync, after, || rows)
    }

    fn write_with<R: AsRef<[T]>, T: Serialize>(
        &self,
        sync: bool,
        after: Option<crate::tail::Position>,
        rows: impl FnOnce() -> R,
    ) -> Result<bool, SessionError> {
        std::fs::create_dir_all(&self.dir)?;
        let segments = vak_storage::segments::SegmentSet::open(&self.dir).map_err(storage_error)?;
        let lock = segments.lock().map_err(storage_error)?;
        crate::fence::check()?;
        segments.recover(&lock).map_err(storage_error)?;
        if let Some(after) = after {
            let mut moved = false;
            crate::tail::try_tail(&self.dir, after, |_, _| {
                moved = true;
                Ok(false)
            })?;
            if moved {
                return Ok(false);
            }
        }
        let rows = rows();
        let rows = rows.as_ref();
        if rows.is_empty() {
            return Ok(true);
        }
        let key = match (crate::keys::of(&self.dir)?, &self.scope) {
            (Some(key), _) => Some(key),
            (None, None) => None,
            (None, Some(scope)) if !self.exists() => Some(crate::keys::declare(&self.dir, scope)?),
            (None, Some(_)) => return Err(SessionError::Unencrypted(self.dir.clone())),
        };
        let active = match crate::log::segment_numbers(&self.dir).last() {
            Some((number, true)) => *number,
            Some((number, false)) => number + 1,
            None => 1,
        };
        let log_path = segments.log_path(active);
        if log_path.exists() {
            let prev = segments.prev_head(active).map_err(storage_error)?;
            let report =
                vak_storage::records::verify_chain_from(&log_path, prev).map_err(storage_error)?;
            if report.torn_tail.is_some() {
                vak_storage::records::truncate_torn_tail_from(&log_path, prev)
                    .map_err(storage_error)?;
            }
        }
        let mut writer = segments.writer(active, &lock).map_err(storage_error)?;
        for row in rows {
            let bytes = serde_json::to_vec(row).map_err(|error| SessionError::Corrupt {
                line: 0,
                message: error.to_string(),
            })?;
            writer
                .append_unsynced(&bytes, key.as_ref())
                .map_err(storage_error)?;
        }
        if sync {
            writer.sync().map_err(storage_error)?;
            drop(writer);
        } else {
            writer.close_unsynced();
        }
        let full = std::fs::metadata(&log_path).map(|meta| meta.len())? >= ROTATE_BYTES;
        if full {
            segments.seal(active, &lock).map_err(storage_error)?;
        }
        drop(lock);
        Ok(true)
    }

    /// The chain's segments, oldest first, each with its rows: what the
    /// lifecycle reconciler reads to tell which sealed segment holds only
    /// rows past their retention (plan M7a-d).
    pub fn segments(&self) -> Vec<Segment> {
        let Ok(key) = crate::keys::of(&self.dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let files = crate::log::SessionLog::segment_files(&self.dir);
        for ((number, open), path) in crate::log::segment_numbers(&self.dir)
            .into_iter()
            .zip(files)
        {
            let Ok(bytes) = vak_storage::segments::frame_bytes(&path) else {
                continue;
            };
            let Ok(frames) = vak_storage::records::located_entries(&bytes, 0, key.as_ref()) else {
                continue;
            };
            out.push(Segment {
                number,
                open,
                bytes: std::fs::metadata(&path).map_or(0, |meta| meta.len()),
                rows: frames
                    .iter()
                    .filter_map(|frame| serde_json::from_slice(&frame.entry).ok())
                    .collect(),
            });
        }
        out
    }

    /// Seals the open segment so its rows can age out together; the next
    /// append starts a new one. `false` when nothing is open.
    pub fn seal_open(&self) -> Result<bool, SessionError> {
        let Some((number, true)) = crate::log::segment_numbers(&self.dir).last().copied() else {
            return Ok(false);
        };
        let segments = vak_storage::segments::SegmentSet::open(&self.dir).map_err(storage_error)?;
        let lock = segments.lock().map_err(storage_error)?;
        crate::fence::check()?;
        segments.recover(&lock).map_err(storage_error)?;
        segments.seal(number, &lock).map_err(storage_error)?;
        Ok(true)
    }

    /// Removes sealed segment `number` whole; later segments still verify
    /// (`SegmentSet::drop_sealed`). Never an open segment or the newest
    /// sealed one.
    pub fn drop_segment(&self, number: u64) -> Result<bool, SessionError> {
        let segments = vak_storage::segments::SegmentSet::open(&self.dir).map_err(storage_error)?;
        let lock = segments.lock().map_err(storage_error)?;
        crate::fence::check()?;
        segments.recover(&lock).map_err(storage_error)?;
        segments.drop_sealed(number, &lock).map_err(storage_error)
    }

    /// Every row in order. A row that does not decode as `T` is skipped,
    /// as is a segment that cannot be read.
    pub fn read<T: DeserializeOwned>(&self) -> Vec<T> {
        let mut rows = Vec::new();
        self.scan(|row| {
            rows.push(row);
            true
        });
        rows
    }

    /// Every row for a decision: unreadable frames and malformed rows are
    /// errors. Use [`Self::read`] only for explicitly best-effort inspection.
    pub fn try_read<T: DeserializeOwned>(&self) -> Result<Vec<T>, SessionError> {
        let mut rows = Vec::new();
        crate::tail::try_tail(&self.dir, Default::default(), |_, bytes| {
            let row = serde_json::from_slice(bytes).map_err(|error| SessionError::Corrupt {
                line: 0,
                message: error.to_string(),
            })?;
            rows.push(row);
            Ok(true)
        })?;
        Ok(rows)
    }

    /// The rows as JSON, one per line, in order: a plain-text rendering for
    /// inspection and tests. Not a format anything parses back.
    pub fn text(&self) -> String {
        self.read::<serde_json::Value>()
            .iter()
            .map(|row| format!("{row}\n"))
            .collect()
    }

    /// Visits rows in order until `visit` returns `false`.
    pub fn scan<T: DeserializeOwned>(&self, mut visit: impl FnMut(T) -> bool) {
        // A chain whose conversation was erased reads as empty.
        let Ok(key) = crate::keys::of(&self.dir) else {
            return;
        };
        for segment in crate::log::SessionLog::segment_files(&self.dir) {
            let Ok(bytes) = vak_storage::segments::frame_bytes(&segment) else {
                continue;
            };
            let Ok(frames) = vak_storage::records::located_entries(&bytes, 0, key.as_ref()) else {
                continue;
            };
            for frame in frames {
                if let Ok(row) = serde_json::from_slice::<T>(&frame.entry)
                    && !visit(row)
                {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_reads_reject_bad_rows_torn_frames_and_broken_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let chain = RecordChain::at(dir.path().join("strict"));
        chain.append(&serde_json::json!({"n": 1})).unwrap();
        assert_eq!(chain.try_read::<serde_json::Value>().unwrap().len(), 1);
        assert!(chain.try_read::<u64>().is_err());
        let segment = chain.path().join("seg-00000001.log");
        let original = std::fs::read(&segment).unwrap();
        std::fs::write(&segment, &original[..original.len() - 1]).unwrap();
        assert!(chain.try_read::<serde_json::Value>().is_err());
        let mut changed_hash = original;
        let last = changed_hash.len() - 1;
        changed_hash[last] ^= 1;
        std::fs::write(&segment, changed_hash).unwrap();
        assert!(chain.try_read::<serde_json::Value>().is_err());
    }

    #[test]
    fn authoritative_reads_report_an_erased_scope_instead_of_an_empty_chain() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let session = uuid::Uuid::now_v7().to_string();
        let chain = RecordChain::at(dir.path().join("private")).of_conversation(&session);
        chain.append(&serde_json::json!({"n": 1})).unwrap();
        assert_eq!(chain.try_read::<serde_json::Value>().unwrap().len(), 1);
        crate::objects::TenantObjects::for_tenant(&vak_config::paths::local_tenant_home())
            .unwrap()
            .destroy_scope_key(&crate::objects::conversation_scope(&session))
            .unwrap();
        assert!(matches!(
            chain.try_read::<serde_json::Value>(),
            Err(SessionError::Erased(_))
        ));
    }

    #[test]
    fn authoritative_reads_reject_ambiguous_segment_names() {
        let dir = tempfile::tempdir().unwrap();
        let chain = RecordChain::at(dir.path().join("rows"));
        chain.append(&serde_json::json!({"n": 1})).unwrap();
        let original = chain.path().join("seg-00000001.log");
        let malformed = chain.path().join("seg-bad.log");
        std::fs::rename(&original, &malformed).unwrap();
        assert!(chain.try_read::<serde_json::Value>().is_err());
        std::fs::rename(&malformed, &original).unwrap();
        let segments = vak_storage::segments::SegmentSet::open(chain.path()).unwrap();
        let lock = segments.lock().unwrap();
        let bytes = std::fs::read(&original).unwrap();
        segments.seal(1, &lock).unwrap();
        drop(lock);
        std::fs::write(&original, bytes).unwrap();
        assert!(chain.try_read::<serde_json::Value>().is_err());
        std::fs::remove_file(&original).unwrap();
        assert_eq!(chain.try_read::<serde_json::Value>().unwrap().len(), 1);
    }

    #[test]
    fn rows_read_back_in_order_across_seals_and_handles() {
        let dir = tempfile::tempdir().unwrap();
        let chain = RecordChain::at(dir.path().join("costs"));
        assert!(!chain.exists());
        for n in 0..200u32 {
            // Hash output, so compression cannot shrink it away.
            let pad: String = (0..64u32)
                .map(|i| {
                    use sha2::{Digest, Sha256};
                    Sha256::digest(format!("{n}-{i}"))
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                })
                .collect();
            RecordChain::at(chain.path())
                .append(&serde_json::json!({ "n": n, "pad": pad }))
                .unwrap();
        }
        let rows: Vec<serde_json::Value> = chain.read();
        assert_eq!(rows.len(), 200);
        assert!(rows.iter().enumerate().all(|(i, row)| row["n"] == i as u64));
        assert!(
            crate::log::segment_numbers(chain.path()).len() > 1,
            "the chain rotated into several segments"
        );
        let report = vak_storage::segments::SegmentSet::open(chain.path())
            .unwrap()
            .seals()
            .unwrap();
        assert!(!report.is_empty(), "rotation sealed segments");
    }

    #[test]
    fn an_append_decided_from_a_moved_chain_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let chain = RecordChain::at(dir.path().join("decided"));
        let empty = crate::tail::head(chain.path());
        assert!(
            chain
                .append_all_after(empty, &[serde_json::json!({ "n": 1 })])
                .unwrap()
        );
        // A second writer decided from the empty chain too.
        assert!(
            !chain
                .append_all_after(empty, &[serde_json::json!({ "n": 2 })])
                .unwrap()
        );
        let now = crate::tail::head(chain.path());
        assert!(
            chain
                .append_all_after(now, &[serde_json::json!({ "n": 3 })])
                .unwrap()
        );
        let rows: Vec<serde_json::Value> = chain.read();
        assert_eq!(
            rows,
            vec![serde_json::json!({ "n": 1 }), serde_json::json!({ "n": 3 })]
        );
    }

    #[test]
    fn an_append_decided_sees_every_row_written_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("decided-under-lock");
        let writers: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let chain = RecordChain::at(&path);
                    for _ in 0..10 {
                        chain
                            .append_decided(|| {
                                let seen: Vec<serde_json::Value> = RecordChain::at(&path).read();
                                Ok::<_, ()>(vec![serde_json::json!({ "n": seen.len() })])
                            })
                            .unwrap()
                            .unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let chain = RecordChain::at(&path);
        let refused = chain
            .append_decided(|| Err::<Vec<serde_json::Value>, _>("refused"))
            .unwrap();
        assert_eq!(refused, Err("refused"));
        let rows: Vec<serde_json::Value> = chain.read();
        assert_eq!(
            rows,
            (0..80)
                .map(|n| serde_json::json!({ "n": n }))
                .collect::<Vec<_>>(),
            "every decision saw the rows before it, and a refusal wrote nothing"
        );
    }

    #[test]
    fn concurrent_appenders_never_interleave_a_chain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events");
        let writers: Vec<_> = (0..8)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for n in 0..25 {
                        RecordChain::at(&path)
                            .append(&serde_json::json!({ "w": w, "n": n }))
                            .unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let rows: Vec<serde_json::Value> = RecordChain::at(&path).read();
        assert_eq!(rows.len(), 200);
        for segment in crate::log::SessionLog::segment_files(&path) {
            let bytes = vak_storage::segments::frame_bytes(&segment).unwrap();
            assert!(vak_storage::records::located_entries(&bytes, 0, None).is_ok());
        }
    }
}
