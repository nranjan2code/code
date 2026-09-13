//! FTS5 full-text search and structured query API.

use crate::{IndexedEntry, Store, StoreError};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchResult {
    pub entries: Vec<SearchHit>,
    pub total: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub entry_id: String,
    pub session_id: String,
    pub project_hash: String,
    pub ts: String,
    pub kind: String,
    pub role: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tool_name: Option<String>,
    pub score: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    pub session_id: Option<String>,
    pub project_hash: Option<String>,
    pub kind: Option<String>,
    pub role: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub exclude_session: Option<String>,
}

impl Store {
    /// Full-text search using FTS5 BM25 ranking. The query is passed
    /// directly to FTS5 (supports `AND`, `OR`, `"exact phrase"`, `NEAR`,
    /// prefix `*`).
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        filter: &SearchFilter,
    ) -> Result<SearchResult, StoreError> {
        let conn = self.conn();
        let limit = limit.clamp(1, 100);

        // Build the FTS5 query with optional structured filters.
        let mut conditions = vec!["entries_fts MATCH ?1".to_string()];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params.push(Box::new(query.to_string()));

        if let Some(ref sid) = filter.session_id {
            conditions.push("entries_fts.session_id = ?".to_string());
            params.push(Box::new(sid.clone()));
        }
        if let Some(ref hash) = filter.project_hash {
            conditions.push("entries_fts.project_hash = ?".to_string());
            params.push(Box::new(hash.clone()));
        }
        if let Some(ref kind) = filter.kind {
            conditions.push("entries_fts.kind = ?".to_string());
            params.push(Box::new(kind.clone()));
        }
        if let Some(ref role) = filter.role {
            conditions.push("entries_fts.role = ?".to_string());
            params.push(Box::new(role.clone()));
        }
        if let Some(ref since) = filter.since {
            conditions.push("entries_fts.ts >= ?".to_string());
            params.push(Box::new(since.clone()));
        }
        if let Some(ref until) = filter.until {
            conditions.push("entries_fts.ts <= ?".to_string());
            params.push(Box::new(until.clone()));
        }
        if let Some(ref excl) = filter.exclude_session {
            conditions.push("entries_fts.session_id != ?".to_string());
            params.push(Box::new(excl.clone()));
        }

        let where_clause = conditions.join(" AND ");
        let sql = format!(
            "SELECT entries_fts.entry_id, entries_fts.session_id,
                    entries_fts.project_hash, entries_fts.ts,
                    entries_fts.kind, entries_fts.role,
                    e.provider, e.model, e.tool_name,
                    bm25(entries_fts) AS score,
                    snippet(entries_fts, 6, '<b>', '</b>', '…', 32) AS snippet
             FROM entries_fts
             LEFT JOIN entries e ON e.entry_id = entries_fts.entry_id
             WHERE {where_clause}
             ORDER BY bm25(entries_fts)
             LIMIT {limit}"
        );

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(SearchHit {
                entry_id: row.get(0)?,
                session_id: row.get(1)?,
                project_hash: row.get(2)?,
                ts: row.get(3)?,
                kind: row.get(4)?,
                role: row.get(5)?,
                provider: row.get(6)?,
                model: row.get(7)?,
                tool_name: row.get(8)?,
                score: row.get(9)?,
                snippet: row.get(10)?,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        let total = entries.len();
        Ok(SearchResult { entries, total })
    }

    /// Structured query without FTS — filter by metadata only.
    pub fn query(
        &self,
        filter: &SearchFilter,
        limit: usize,
    ) -> Result<Vec<IndexedEntry>, StoreError> {
        self.query_page(filter, limit, 0, false)
            .map(|(entries, _)| entries)
    }

    /// Structured query with pagination, ordering, and total matching count.
    pub fn query_page(
        &self,
        filter: &SearchFilter,
        limit: usize,
        offset: usize,
        ascending: bool,
    ) -> Result<(Vec<IndexedEntry>, usize), StoreError> {
        let conn = self.conn();
        let limit = limit.clamp(1, 500);

        let mut conditions = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref sid) = filter.session_id {
            conditions.push("session_id = ?".to_string());
            params.push(Box::new(sid.clone()));
        }
        if let Some(ref hash) = filter.project_hash {
            conditions.push("project_hash = ?".to_string());
            params.push(Box::new(hash.clone()));
        }
        if let Some(ref kind) = filter.kind {
            conditions.push("kind = ?".to_string());
            params.push(Box::new(kind.clone()));
        }
        if let Some(ref role) = filter.role {
            conditions.push("role = ?".to_string());
            params.push(Box::new(role.clone()));
        }
        if let Some(ref since) = filter.since {
            conditions.push("ts >= ?".to_string());
            params.push(Box::new(since.clone()));
        }
        if let Some(ref until) = filter.until {
            conditions.push("ts <= ?".to_string());
            params.push(Box::new(until.clone()));
        }
        if let Some(ref excl) = filter.exclude_session {
            conditions.push("session_id != ?".to_string());
            params.push(Box::new(excl.clone()));
        }

        let where_clause = if conditions.is_empty() {
            "1=1".to_string()
        } else {
            conditions.join(" AND ")
        };

        let count_sql = format!("SELECT COUNT(*) FROM entries WHERE {where_clause}");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let total: usize = conn.query_row(&count_sql, param_refs.as_slice(), |row| row.get(0))?;

        let order_dir = if ascending { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT entry_id, session_id, project_hash, parent_id, ts,
                    kind, role, provider, model, tool_name,
                    content_text, is_error
             FROM entries
             WHERE {where_clause}
             ORDER BY ts {order_dir}, entry_id {order_dir}
             LIMIT {limit} OFFSET {offset}"
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            let kind_str: String = row.get(5)?;
            Ok(IndexedEntry {
                entry_id: row.get(0)?,
                session_id: row.get(1)?,
                project_hash: row.get(2)?,
                parent_id: row.get(3)?,
                ts: row.get(4)?,
                kind: crate::EntryKind::parse_str(&kind_str).unwrap_or(crate::EntryKind::Message),
                role: row.get(6)?,
                provider: row.get(7)?,
                model: row.get(8)?,
                tool_name: row.get(9)?,
                content_text: row.get(10)?,
                is_error: row.get::<_, i32>(11)? != 0,
            })
        })?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok((entries, total))
    }

    /// List all distinct session IDs with entry counts.
    pub fn list_sessions(&self) -> Result<Vec<SessionInfo>, StoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT session_id, project_hash,
                    COUNT(*) as count,
                    MIN(ts) as first_ts,
                    MAX(ts) as last_ts
             FROM entries
             GROUP BY session_id
             ORDER BY last_ts DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(SessionInfo {
                session_id: row.get(0)?,
                project_hash: row.get(1)?,
                entry_count: row.get(2)?,
                first_ts: row.get(3)?,
                last_ts: row.get(4)?,
            })
        })?;

        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub project_hash: String,
    pub entry_count: usize,
    pub first_ts: String,
    pub last_ts: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::Store;
    use std::io::Write;
    use std::path::Path;
    use vak_llm::types::{ContentBlock, Message};
    use vak_session::log::SessionPath;
    use vak_session::types::{Entry, EntryPayload, FrozenContract, MessageRecord, SessionHeader};

    fn test_header(id: &str) -> SessionHeader {
        SessionHeader {
            agent: None,
            session_id: id.to_string(),
            created_at: chrono::Utc::now(),
            cwd: std::env::current_dir().unwrap(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
            contract: FrozenContract {
                app_version: "test".into(),
                provider: "openai".into(),
                model: "gpt-4o".into(),
                route_ladder: vec![],
                route_objective: String::new(),
                route_annotations: vec![],
                system_prompt: String::new(),
                permission_mode: "workspace-write".into(),
                capabilities: Vec::new(),
                prompt_layers: Vec::new(),
            },
        }
    }

    fn user_msg(text: &str) -> MessageRecord {
        MessageRecord {
            message: Message {
                role: vak_llm::Role::User,
                content: vec![ContentBlock::text(text)],
            },
            meta: None,
        }
    }

    fn assistant_msg(text: &str) -> MessageRecord {
        MessageRecord {
            message: Message {
                role: vak_llm::Role::Assistant,
                content: vec![ContentBlock::text(text)],
            },
            meta: None,
        }
    }

    fn write_session(home: &Path, cwd: &Path, id: &str, msgs: &[MessageRecord]) {
        let path = SessionPath::new_session_file(home, cwd, id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&path).unwrap();
        let mut w = std::io::BufWriter::new(file);
        let header = Entry::new(None, EntryPayload::Header(test_header(id)));
        serde_json::to_writer(&mut w, &header).unwrap();
        w.write_all(b"\n").unwrap();
        let mut parent = Some(header.id.clone());
        for m in msgs {
            let entry = Entry {
                prev_hash: None,
                id: uuid::Uuid::now_v7().to_string(),
                parent_id: parent.clone(),
                ts: chrono::Utc::now(),
                payload: EntryPayload::Message(m.clone()),
            };
            parent = Some(entry.id.clone());
            serde_json::to_writer(&mut w, &entry).unwrap();
            w.write_all(b"\n").unwrap();
        }
        w.flush().unwrap();
    }

    #[test]
    fn fts_search_returns_ranked_results() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(
            home,
            cwd,
            "s1",
            &[
                user_msg("the deploy pipeline handles rollbacks gracefully"),
                assistant_msg("confirmed — rollback windows pause before deploy"),
            ],
        );
        write_session(
            home,
            cwd,
            "s2",
            &[user_msg("pizza toppings are irrelevant to this query")],
        );

        let store = Store::open(home).unwrap();
        store.rebuild(home).unwrap();

        let result = store
            .search("deploy rollback", 10, &SearchFilter::default())
            .unwrap();
        assert!(result.total >= 2, "both deploy messages should match");
        // All hits should be from s1.
        assert!(result.entries.iter().all(|h| h.session_id == "s1"));
        // First hit should have a snippet with highlight markers.
        assert!(result.entries[0].snippet.contains("<b>"));
    }

    #[test]
    fn fts_search_respects_filters() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(
            home,
            cwd,
            "s1",
            &[user_msg("rust compiler optimization flags")],
        );
        write_session(
            home,
            cwd,
            "s2",
            &[assistant_msg("rust compiler error messages are cryptic")],
        );

        let store = Store::open(home).unwrap();
        store.rebuild(home).unwrap();

        // Filter by role=user.
        let result = store
            .search(
                "rust compiler",
                10,
                &SearchFilter {
                    role: Some("user".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            result
                .entries
                .iter()
                .all(|h| h.role.as_deref() == Some("user"))
        );

        // Exclude session s1.
        let result = store
            .search(
                "rust compiler",
                10,
                &SearchFilter {
                    exclude_session: Some("s1".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(result.entries.iter().all(|h| h.session_id == "s2"));
    }

    #[test]
    fn structured_query_by_kind() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(home, cwd, "s1", &[user_msg("hello world")]);

        let store = Store::open(home).unwrap();
        store.rebuild(home).unwrap();

        let headers = store
            .query(
                &SearchFilter {
                    kind: Some("header".into()),
                    ..Default::default()
                },
                10,
            )
            .unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].kind, crate::EntryKind::Header);

        let messages = store
            .query(
                &SearchFilter {
                    kind: Some("message".into()),
                    ..Default::default()
                },
                10,
            )
            .unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn list_sessions_groups_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(
            home,
            cwd,
            "sess-aaa",
            &[user_msg("first"), assistant_msg("second")],
        );
        write_session(home, cwd, "sess-bbb", &[user_msg("third")]);
        write_session(home, cwd, "sess-empty", &[]);

        let store = Store::open(home).unwrap();
        store.rebuild(home).unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 3);
        let aaa = sessions
            .iter()
            .find(|s| s.session_id == "sess-aaa")
            .unwrap();
        assert_eq!(aaa.entry_count, 3); // 1 header + 2 messages
        let bbb = sessions
            .iter()
            .find(|s| s.session_id == "sess-bbb")
            .unwrap();
        assert_eq!(bbb.entry_count, 2); // 1 header + 1 message
        let empty = sessions
            .iter()
            .find(|s| s.session_id == "sess-empty")
            .unwrap();
        assert_eq!(empty.entry_count, 1); // 1 header
    }

    #[test]
    fn query_page_paginates_and_orders_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(
            home,
            cwd,
            "sess-p",
            &[user_msg("msg 1"), assistant_msg("msg 2"), user_msg("msg 3")],
        );

        let store = Store::open(home).unwrap();
        store.rebuild(home).unwrap();

        let filter = SearchFilter {
            session_id: Some("sess-p".into()),
            ..Default::default()
        };

        // Total entries = 1 header + 3 messages = 4
        let (page1, total) = store.query_page(&filter, 2, 0, true).unwrap();
        assert_eq!(total, 4);
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].kind, crate::EntryKind::Header);

        let (page2, total2) = store.query_page(&filter, 2, 2, true).unwrap();
        assert_eq!(total2, 4);
        assert_eq!(page2.len(), 2);

        let (page3, total3) = store.query_page(&filter, 2, 4, true).unwrap();
        assert_eq!(total3, 4);
        assert_eq!(page3.len(), 0);
    }

    #[test]
    fn import_session_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(
            home,
            cwd,
            "s-idem",
            &[user_msg("unique term idempotency check")],
        );

        let store = Store::open(home).unwrap();
        let path = SessionPath::new_session_file(home, cwd, "s-idem");
        let s1 = store.import_session(home, &path).unwrap();
        assert_eq!(s1.entries_indexed, 2);
        let s2 = store.import_session(home, &path).unwrap();
        assert_eq!(s2.skipped, 2);
        assert_eq!(s2.entries_indexed, 0);

        // Search still works after double-import.
        let result = store
            .search("idempotency", 5, &SearchFilter::default())
            .unwrap();
        assert_eq!(result.total, 1);
    }
}
