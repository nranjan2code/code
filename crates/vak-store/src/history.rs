//! Compact turn addresses and logarithmic active-branch membership.
//! These are derived accelerators, not an authorization boundary.

use rusqlite::{Connection, OptionalExtension, params};
use vak_session::{Entry, EntryPayload};

use crate::{Store, StoreError};

#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnSearchHit {
    pub turn_id: String,
    pub record_id: String,
    pub record: String,
    pub recorded_at: String,
    /// A ranking score, never a calibrated probability.
    pub match_score: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TurnSearchResult {
    pub matches: Vec<TurnSearchHit>,
    /// A bounded candidate pass is not an exhaustive absence proof.
    pub candidate_limit_reached: bool,
}

pub(crate) fn index_history_record(
    conn: &Connection,
    session_id: &str,
    entry: &Entry,
) -> Result<(), StoreError> {
    let existing: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM entry_lineage WHERE entry_id = ?1)",
        [&entry.id],
        |row| row.get(0),
    )?;
    if !existing {
        let parent: Option<(u64, Option<String>)> = match &entry.parent_id {
            Some(id) => conn.query_row(
                "SELECT depth, reset_id FROM entry_lineage WHERE session_id = ?1 AND entry_id = ?2",
                params![session_id, id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?,
            None => None,
        };
        if entry.parent_id.is_some() && parent.is_none() {
            return Err(
                std::io::Error::other("history parent is missing or outside this session").into(),
            );
        }
        let depth = parent.as_ref().map_or(0, |(depth, _)| depth + 1);
        let reset = match &entry.payload {
            EntryPayload::Compaction(record) if record.reset_all => Some(entry.id.clone()),
            _ => parent.and_then(|(_, reset)| reset),
        };
        conn.execute(
            "INSERT INTO entry_lineage(entry_id, session_id, depth, reset_id) VALUES (?1, ?2, ?3, ?4)",
            params![entry.id, session_id, depth, reset],
        )?;
        if let Some(parent) = &entry.parent_id {
            conn.execute(
                "INSERT INTO entry_jumps(entry_id, level, ancestor_id) VALUES (?1, 0, ?2)",
                params![entry.id, parent],
            )?;
            let mut previous = parent.clone();
            for level in 1..63 {
                let next: Option<String> = conn
                    .query_row(
                        "SELECT ancestor_id FROM entry_jumps WHERE entry_id = ?1 AND level = ?2",
                        params![previous, level - 1],
                        |row| row.get(0),
                    )
                    .optional()?;
                let Some(next) = next else {
                    break;
                };
                conn.execute(
                    "INSERT INTO entry_jumps(entry_id, level, ancestor_id) VALUES (?1, ?2, ?3)",
                    params![entry.id, level, next],
                )?;
                previous = next;
            }
        }
    }
    if let EntryPayload::TurnCard(record) = &entry.payload {
        let summary = compact_turn_record(record);
        conn.execute(
            "INSERT OR IGNORE INTO turn_descriptors(entry_id, session_id, turn_id, record) VALUES (?1, ?2, ?3, ?4)",
            params![entry.id, session_id, record.turn_id, summary],
        )?;
    }
    Ok(())
}

/// The bounded discovery projection, regenerated from a verified canonical record.
pub fn compact_turn_record(record: &vak_session::TurnCardRecord) -> String {
    let asked = record.card.asked.chars().take(600).collect::<String>();
    let answer = record
        .card
        .answered
        .narration
        .chars()
        .take(800)
        .collect::<String>();
    let titles = record
        .card
        .answered
        .presentations
        .iter()
        .flat_map(|p| p.title.chars().chain(std::iter::once(' ')))
        .take(180)
        .collect::<String>();
    format!("Asked: {asked}\nAnswered: {answer}\nOutputs: {titles}")
        .chars()
        .take(1600)
        .collect()
}

fn is_ancestor(
    conn: &Connection,
    session: &str,
    ancestor: &str,
    leaf: &str,
) -> Result<bool, StoreError> {
    let depth = |id: &str| -> Result<Option<u64>, StoreError> {
        Ok(conn
            .query_row(
                "SELECT depth FROM entry_lineage WHERE session_id = ?1 AND entry_id = ?2",
                params![session, id],
                |row| row.get(0),
            )
            .optional()?)
    };
    let (Some(target), Some(current)) = (depth(ancestor)?, depth(leaf)?) else {
        return Ok(false);
    };
    if target > current {
        return Ok(false);
    }
    let difference = current - target;
    let mut cursor = leaf.to_string();
    for level in 0..63 {
        if difference & (1u64 << level) == 0 {
            continue;
        }
        let next: Option<String> = conn
            .query_row(
                "SELECT ancestor_id FROM entry_jumps WHERE entry_id = ?1 AND level = ?2",
                params![cursor, level],
                |row| row.get(0),
            )
            .optional()?;
        let Some(next) = next else {
            return Ok(false);
        };
        cursor = next;
    }
    Ok(cursor == ancestor)
}

impl Store {
    /// Return a settled turn's closing record on the requested branch.
    pub fn locate_turn_record(
        &self,
        session: &str,
        turn: &str,
        leaf: &str,
    ) -> Result<Option<String>, StoreError> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT entry_id FROM turn_descriptors WHERE session_id = ?1 AND turn_id = ?2 LIMIT 8",
        )?;
        let ids = statement
            .query_map(params![session, turn], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in ids {
            if is_ancestor(&conn, session, &id, leaf)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// A stable ID from a sibling branch must never be admitted by accident.
    pub fn history_is_ancestor(
        &self,
        session: &str,
        ancestor: &str,
        leaf: &str,
    ) -> Result<bool, StoreError> {
        is_ancestor(&self.conn(), session, ancestor, leaf)
    }

    /// Search bounded compact records on one canonical active branch.
    /// Natural language is escaped; it cannot become arbitrary FTS syntax.
    pub fn search_turns(
        &self,
        session: &str,
        leaf: &str,
        query: &str,
        limit: usize,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<TurnSearchResult, StoreError> {
        if cancel.is_cancelled() {
            return Err(std::io::Error::other("history lookup cancelled").into());
        }
        let terms: Vec<String> = vak_session::turns::history_query_terms(query)
            .into_iter()
            .take(32)
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect();
        if terms.is_empty() {
            return Ok(TurnSearchResult {
                matches: vec![],
                candidate_limit_reached: false,
            });
        }
        let query = terms.join(" OR ");
        let conn = self.conn();
        let started = std::time::Instant::now();
        let query_cancel = cancel.clone();
        conn.progress_handler(
            1000,
            Some(move || {
                query_cancel.is_cancelled()
                    || started.elapsed() >= std::time::Duration::from_millis(250)
            }),
        );
        let result = (|| {
            let mut statement = conn.prepare(
                "SELECT d.turn_id, d.entry_id, d.record, bm25(entries_fts), entries_fts.ts
             FROM entries_fts JOIN turn_descriptors d ON d.entry_id = entries_fts.entry_id
             WHERE entries_fts MATCH ?1 AND d.session_id = ?2
             ORDER BY bm25(entries_fts), d.entry_id LIMIT 64",
            )?;
            let hits = statement
                .query_map(params![query, session], |row| {
                    Ok(TurnSearchHit {
                        turn_id: row.get(0)?,
                        record_id: row.get(1)?,
                        record: row.get(2)?,
                        match_score: row.get(3)?,
                        recorded_at: row.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let candidate_limit_reached = hits.len() == 64;
            let mut selected = Vec::new();
            for hit in hits {
                if cancel.is_cancelled()
                    || started.elapsed() >= std::time::Duration::from_millis(250)
                {
                    return Err(std::io::Error::other(
                        "history search cancelled or exceeded its deadline",
                    )
                    .into());
                }
                if is_ancestor(&conn, session, &hit.record_id, leaf)? {
                    selected.push(hit);
                    if selected.len() >= limit.clamp(1, 20) {
                        break;
                    }
                }
            }
            Ok(TurnSearchResult {
                matches: selected,
                candidate_limit_reached,
            })
        })();
        conn.progress_handler(0, None::<fn() -> bool>);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logarithmic_links_distinguish_distant_ancestors_and_sibling_branches()
    -> Result<(), StoreError> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path())?;
        let conn = store.conn();
        let mut previous = None;
        let mut ids = Vec::new();
        for _ in 0..2048 {
            let entry = Entry::new(
                previous,
                EntryPayload::Message(vak_session::MessageRecord {
                    message: vak_llm::Message::user_text("subject"),
                    meta: None,
                }),
            );
            index_history_record(&conn, "session", &entry)?;
            previous = Some(entry.id.clone());
            ids.push(entry.id);
        }
        assert!(is_ancestor(&conn, "session", &ids[3], &ids[2047])?);
        assert!(!is_ancestor(&conn, "another-session", &ids[3], &ids[2047])?);
        let sibling = Entry::new(
            Some(ids[2].clone()),
            EntryPayload::Message(vak_session::MessageRecord {
                message: vak_llm::Message::user_text("different branch"),
                meta: None,
            }),
        );
        index_history_record(&conn, "session", &sibling)?;
        assert!(is_ancestor(&conn, "session", &ids[2], &sibling.id)?);
        assert!(!is_ancestor(&conn, "session", &ids[3], &sibling.id)?);
        assert!(!is_ancestor(&conn, "session", &sibling.id, &ids[2047])?);
        // A distant lookup uses logarithmic jumps, not one row per ancestor.
        let jump_count: u64 = conn.query_row(
            "SELECT COUNT(*) FROM entry_jumps WHERE entry_id = ?1",
            [&ids[2047]],
            |row| row.get(0),
        )?;
        assert!(jump_count <= 12);
        Ok(())
    }
}
