//! Bulk import and incremental file-level indexing.

use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::path::Path;

use walkdir::WalkDir;

use crate::{ImportStats, RebuildStats, Store, StoreError};
use vak_session::types::Entry;

impl Store {
    /// Scan every `<home>/sessions/<hash>/*.jsonl` file and index entries.
    pub(crate) fn import_all(
        &self,
        conn: &rusqlite::Connection,
        sessions_home: &Path,
    ) -> Result<RebuildStats, StoreError> {
        let mut roots = Vec::new();
        let direct = vak_config::scope::AgentScope::new(sessions_home).sessions_root();
        if direct.exists() {
            roots.push((sessions_home.to_path_buf(), direct));
        }
        if let Ok(agents) =
            std::fs::read_dir(vak_config::scope::AgentScope::new(sessions_home).agents_dir())
        {
            for agent in agents.flatten() {
                let s = vak_config::scope::AgentScope::new(agent.path()).sessions_root();
                if s.exists() {
                    roots.push((agent.path(), s));
                }
            }
        }
        if let Some(siblings) = sessions_home
            .parent()
            .filter(|p| p.file_name().and_then(|s| s.to_str()) == Some("agents"))
            .and_then(|p| std::fs::read_dir(p).ok())
        {
            for sibling in siblings.flatten() {
                let s = vak_config::scope::AgentScope::new(sibling.path()).sessions_root();
                if s.exists() && !roots.iter().any(|(_, r)| r == &s) {
                    roots.push((sibling.path(), s));
                }
            }
        }
        if roots.is_empty() {
            return Ok(RebuildStats {
                files_scanned: 0,
                entries_indexed: 0,
                fts_rows: 0,
            });
        }
        let mut files_scanned = 0usize;
        let mut entries_indexed = 0usize;
        let mut fts_rows = 0usize;

        for (home, root) in roots {
            for entry in WalkDir::new(&root)
                .min_depth(2)
                .max_depth(2)
                .follow_links(false)
                .into_iter()
            {
                let entry = entry.map_err(|e| std::io::Error::other(e.to_string()))?;
                let path = entry.path();
                if vak_config::scope::ledger_session_id(path).is_none() {
                    continue;
                }
                let stats = self.import_file(conn, &home, path)?;
                entries_indexed += stats.entries_indexed;
                fts_rows += stats.fts_rows;
                files_scanned += 1;
            }
        }

        Ok(RebuildStats {
            files_scanned,
            entries_indexed,
            fts_rows,
        })
    }

    /// Parse a single JSONL file and insert all entries into the index.
    /// Already-indexed entry IDs are skipped.
    pub(crate) fn import_file(
        &self,
        conn: &rusqlite::Connection,
        _sessions_home: &Path,
        jsonl_path: &Path,
    ) -> Result<ImportStats, StoreError> {
        self.import_file_bounded(conn, _sessions_home, jsonl_path, None)
    }

    pub(crate) fn import_file_bounded(
        &self,
        conn: &rusqlite::Connection,
        _sessions_home: &Path,
        jsonl_path: &Path,
        max_bytes: Option<u64>,
    ) -> Result<ImportStats, StoreError> {
        self.import_file_chunk(conn, _sessions_home, jsonl_path, max_bytes, None)
    }

    /// Imports a session ledger (a directory of record segments) segment by
    /// segment, each with its own cursor, and adds up what they did.
    pub(crate) fn import_file_chunk(
        &self,
        conn: &rusqlite::Connection,
        _sessions_home: &Path,
        ledger: &Path,
        max_bytes: Option<u64>,
        chunk_bytes: Option<u64>,
    ) -> Result<ImportStats, StoreError> {
        let mut total = ImportStats {
            entries_indexed: 0,
            fts_rows: 0,
            skipped: 0,
            bytes_read: 0,
            committed_offset: 0,
            observed_length: 0,
            made_progress: false,
        };
        for segment in vak_session::SessionLog::segment_files(ledger) {
            let stats = self.import_segment(conn, ledger, &segment, max_bytes, chunk_bytes)?;
            total.entries_indexed += stats.entries_indexed;
            total.fts_rows += stats.fts_rows;
            total.skipped += stats.skipped;
            total.bytes_read += stats.bytes_read;
            total.committed_offset += stats.committed_offset;
            total.observed_length += stats.observed_length;
            total.made_progress |= stats.made_progress;
        }
        Ok(total)
    }

    fn import_segment(
        &self,
        conn: &rusqlite::Connection,
        ledger: &Path,
        segment: &Path,
        max_bytes: Option<u64>,
        chunk_bytes: Option<u64>,
    ) -> Result<ImportStats, StoreError> {
        let session_id = ledger
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        // Derive project hash from the parent directory name.
        // Directory structure: <home>/sessions/<hash>/<session>.jsonl
        let space_id = ledger
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let path_key = segment.canonicalize()?.to_string_lossy().into_owned();
        let frames = vak_storage::segments::frame_bytes(segment)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let file_len = frames.len() as u64;
        let transaction = conn.unchecked_transaction()?;
        let cursor: Option<(u64, String)> = transaction
            .query_row(
                "SELECT offset, anchor FROM ledger_cursors WHERE path = ?1",
                [&path_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let mut bytes_read = 0;
        let mut offset = 0;
        if max_bytes.is_some() && cursor.is_none() {
            return Err(std::io::Error::other("history index is warming").into());
        }
        if let Some((previous, expected)) = cursor {
            let valid = if previous <= file_len {
                let (found, read) = cursor_anchor(&frames, previous);
                bytes_read += read;
                found == expected
            } else {
                false
            };
            if valid {
                offset = previous;
            } else {
                if max_bytes.is_some() {
                    return Err(
                        std::io::Error::other("history index needs a background replay").into(),
                    );
                }
                // A replacement/truncation must not leave searchable old rows.
                // UUID session identities are global; locators are per file.
                transaction.execute(
                    "DELETE FROM entries_fts WHERE session_id = ?1",
                    [&session_id],
                )?;
                transaction.execute("DELETE FROM entries WHERE session_id = ?1", [&session_id])?;
                transaction.execute(
                    "DELETE FROM entry_jumps WHERE entry_id IN
                    (SELECT entry_id FROM entry_lineage WHERE session_id = ?1)",
                    [&session_id],
                )?;
                transaction.execute(
                    "DELETE FROM entry_lineage WHERE session_id = ?1",
                    [&session_id],
                )?;
                transaction.execute(
                    "DELETE FROM turn_descriptors WHERE session_id = ?1",
                    [&session_id],
                )?;
                transaction.execute(
                    "DELETE FROM entry_locators WHERE session_id = ?1",
                    [&session_id],
                )?;
            }
        }
        if max_bytes.is_some_and(|limit| file_len.saturating_sub(offset) > limit) {
            return Err(std::io::Error::other(
                "history append exceeds request-path indexing budget",
            )
            .into());
        }
        let mut entries_indexed = 0usize;
        let mut fts_rows = 0usize;
        let mut skipped = 0usize;
        let conn = &transaction;

        let sequence: u64 = conn.query_row(
            "SELECT COALESCE(MAX(sequence) + 1, 0) FROM entry_locators WHERE path = ?1",
            [&path_key],
            |row| row.get(0),
        )?;
        let mut insert_locator = conn.prepare(
            "INSERT OR REPLACE INTO entry_locators(entry_id, session_id, path, sequence, offset, length, parent_id, digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        // Prepare check and insert statements once.
        let mut check_stmt = conn.prepare("SELECT 1 FROM entries WHERE entry_id = ?1 LIMIT 1")?;
        let mut insert_entry = conn.prepare(
            "INSERT OR IGNORE INTO entries
             (entry_id, session_id, space_id, parent_id, ts, kind, role,
              provider, model, tool_name, content_text, is_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        let mut insert_fts = conn.prepare(
            "INSERT INTO entries_fts(entry_id, session_id, space_id, ts, kind, role, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        let initial_offset = offset;
        let chunk_started = std::time::Instant::now();
        let located = vak_storage::records::located_entries(&frames, offset)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        for (step, frame) in located.into_iter().enumerate() {
            // End only between frames: the cursor always addresses one.
            if offset > initial_offset
                && chunk_bytes.is_some_and(|target| {
                    offset.saturating_sub(initial_offset) >= target
                        || chunk_started.elapsed() >= std::time::Duration::from_millis(100)
                })
            {
                break;
            }
            bytes_read += frame.len;
            let record_offset = frame.offset;
            offset = frame.offset + frame.len;
            let entry: Entry = serde_json::from_slice(&frame.entry)?;
            let canonical_line = std::str::from_utf8(&frame.entry)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let digest = vak_session::types::line_digest(canonical_line);
            insert_locator.execute(rusqlite::params![
                entry.id,
                session_id,
                path_key,
                sequence + step as u64,
                record_offset,
                frame.len,
                entry.parent_id,
                digest
            ])?;
            crate::history::index_history_record(conn, &session_id, &entry)?;

            // Skip already indexed.
            let already: bool = check_stmt
                .query_row(rusqlite::params![entry.id], |row| row.get::<_, i32>(0))
                .is_ok();
            if already {
                skipped += 1;
                continue;
            }

            if let Some(meta) = self.extract_meta_with_hash(&session_id, &space_id, &entry) {
                let has_text = !meta.content_text.trim().is_empty();

                insert_entry.execute(rusqlite::params![
                    meta.entry_id,
                    meta.session_id,
                    meta.space_id,
                    meta.parent_id,
                    meta.ts,
                    meta.kind.as_str(),
                    meta.role,
                    meta.provider,
                    meta.model,
                    meta.tool_name,
                    meta.content_text,
                    meta.is_error as i32,
                ])?;

                if has_text {
                    insert_fts.execute(rusqlite::params![
                        meta.entry_id,
                        meta.session_id,
                        meta.space_id,
                        meta.ts,
                        meta.kind.as_str(),
                        meta.role.as_deref().unwrap_or(""),
                        meta.content_text,
                    ])?;
                    fts_rows += 1;
                }

                entries_indexed += 1;
            }
        }
        drop(insert_locator);
        drop(check_stmt);
        drop(insert_entry);
        drop(insert_fts);
        // Watermark and all derived rows commit together. If the process
        // dies first, replay is idempotent and cannot duplicate FTS entries.
        let (anchor, read) = cursor_anchor(&frames, offset);
        bytes_read += read;
        transaction.execute(
            "INSERT OR REPLACE INTO ledger_cursors(path, session_id, offset, anchor) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![path_key, session_id, offset, anchor],
        )?;
        transaction.commit()?;
        Ok(ImportStats {
            entries_indexed,
            fts_rows,
            skipped,
            bytes_read,
            committed_offset: offset,
            observed_length: file_len,
            made_progress: offset > initial_offset,
        })
    }

    /// Like `extract_meta` but attaches space_id.
    fn extract_meta_with_hash(
        &self,
        session_id: &str,
        space_id: &str,
        entry: &Entry,
    ) -> Option<crate::IndexedEntry> {
        let mut meta = self.extract_meta(session_id, entry)?;
        meta.space_id = space_id.to_string();
        Some(meta)
    }
}

/// Constant-size validation of the first and last committed bytes. Ledgers
/// are append-only; hash-chain verification remains the canonical reader's
/// responsibility. This detects replacement/truncation and torn-cursor reuse.
fn cursor_anchor(frames: &[u8], offset: u64) -> (String, u64) {
    let mut digest = Sha256::new();
    let mut bytes_read = 0;
    let end = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(frames.len());
    for (start, stop) in [(0, end.min(4096)), (end.saturating_sub(4096), end)] {
        digest.update(&frames[start..stop]);
        bytes_read += (stop - start) as u64;
    }
    (format!("{:x}", digest.finalize()), bytes_read)
}
