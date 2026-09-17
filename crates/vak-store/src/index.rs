//! Bulk import and incremental file-level indexing.

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
        let direct = sessions_home.join("sessions");
        if direct.exists() {
            roots.push((sessions_home.to_path_buf(), direct));
        }
        if let Ok(agents) = std::fs::read_dir(sessions_home.join("agents")) {
            for agent in agents.flatten() {
                let s = agent.path().join("sessions");
                if s.exists() {
                    roots.push((agent.path(), s));
                }
            }
        }
        if let Some(parent) = sessions_home.parent() {
            if parent.file_name().and_then(|s| s.to_str()) == Some("agents") {
                if let Ok(siblings) = std::fs::read_dir(parent) {
                    for sibling in siblings.flatten() {
                        let s = sibling.path().join("sessions");
                        if s.exists() && !roots.iter().any(|(_, r)| r == &s) {
                            roots.push((sibling.path(), s));
                        }
                    }
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
                .filter_entry(|e| e.file_type().is_file())
            {
                let entry = entry.map_err(|e| std::io::Error::other(e.to_string()))?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
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
        let session_id = jsonl_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        // Derive project hash from the parent directory name.
        // Directory structure: <home>/sessions/<hash>/<session>.jsonl
        let project_hash = jsonl_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let content = std::fs::read_to_string(jsonl_path)?;
        let mut entries_indexed = 0usize;
        let mut fts_rows = 0usize;
        let mut skipped = 0usize;

        // Prepare check and insert statements once.
        let mut check_stmt = conn.prepare("SELECT 1 FROM entries WHERE entry_id = ?1 LIMIT 1")?;
        let mut insert_entry = conn.prepare(
            "INSERT OR IGNORE INTO entries
             (entry_id, session_id, project_hash, parent_id, ts, kind, role,
              provider, model, tool_name, content_text, is_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        let mut insert_fts = conn.prepare(
            "INSERT INTO entries_fts(entry_id, session_id, project_hash, ts, kind, role, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for (i, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let entry: Entry = match serde_json::from_str(line) {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Skip already indexed.
            let already: bool = check_stmt
                .query_row(rusqlite::params![entry.id], |row| row.get::<_, i32>(0))
                .is_ok();
            if already {
                skipped += 1;
                continue;
            }

            if let Some(meta) = self.extract_meta_with_hash(&session_id, &project_hash, &entry) {
                let has_text = !meta.content_text.trim().is_empty();

                insert_entry.execute(rusqlite::params![
                    meta.entry_id,
                    meta.session_id,
                    meta.project_hash,
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
                        meta.project_hash,
                        meta.ts,
                        meta.kind.as_str(),
                        meta.role.as_deref().unwrap_or(""),
                        meta.content_text,
                    ])?;
                    fts_rows += 1;
                }

                entries_indexed += 1;
            }

            if i % 1000 == 999 {
                conn.execute("PRAGMA optimize", [])?;
            }
        }

        Ok(ImportStats {
            entries_indexed,
            fts_rows,
            skipped,
        })
    }

    /// Like `extract_meta` but attaches project_hash.
    fn extract_meta_with_hash(
        &self,
        session_id: &str,
        project_hash: &str,
        entry: &Entry,
    ) -> Option<crate::IndexedEntry> {
        let mut meta = self.extract_meta(session_id, entry)?;
        meta.project_hash = project_hash.to_string();
        Some(meta)
    }
}
