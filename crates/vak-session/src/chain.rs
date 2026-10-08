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

    fn write<T: Serialize>(&self, rows: &[T], sync: bool) -> Result<(), SessionError> {
        if rows.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.dir)?;
        let segments = vak_storage::segments::SegmentSet::open(&self.dir).map_err(storage_error)?;
        let lock = segments.lock().map_err(storage_error)?;
        crate::fence::check()?;
        segments.recover(&lock).map_err(storage_error)?;
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
        Ok(())
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
