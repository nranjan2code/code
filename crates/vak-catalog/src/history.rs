//! A session's own history, for the turn recall (docs/design/68 §3): where
//! each entry is, which entries lie on a branch, and the compact record of
//! each settled turn, searchable. These address and rank; the caller loads
//! the canonical entry and checks it before anything reaches a model.
//!
//! Branch membership is logarithmic: each entry keeps jumps to its 2^k-th
//! ancestor, so "is A on the branch ending at B" is a handful of lookups
//! however long the history is.

use crate::{Catalog, CatalogError, ingest};
use rusqlite::{OptionalExtension, Transaction, params};
use std::path::PathBuf;
use vak_session::tail::Position;
use vak_session::{Entry, EntryPayload};

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

/// Where one entry is: its ledger and the position just before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLocation {
    pub dir: PathBuf,
    pub at: Position,
    pub parent: Option<String>,
}

/// The bounded discovery projection of a settled turn, regenerated from its
/// canonical card. Bounded by whole words, never by a character count.
pub fn compact_turn_record(record: &vak_session::TurnCardRecord) -> String {
    fn words(text: &str, max: usize) -> String {
        let all: Vec<&str> = text.split_whitespace().collect();
        if all.len() <= max {
            all.join(" ")
        } else {
            format!("{}\u{2026}", all[..max].join(" "))
        }
    }
    let asked = words(&record.card.asked, 120);
    let answer = words(&record.card.answered.narration, 160);
    let titles = record
        .card
        .answered
        .presentations
        .iter()
        .map(|p| words(&p.title, 12))
        .collect::<Vec<_>>()
        .join(" ");
    format!("Asked: {asked}\nAnswered: {answer}\nOutputs: {titles}")
}

/// Records where `entry` is, its place in the branch tree, and a settled
/// turn's compact record.
pub(crate) fn index_entry(
    tx: &Transaction<'_>,
    session: &str,
    at: Position,
    entry: &Entry,
) -> rusqlite::Result<()> {
    let parent: Option<(u64, Option<String>)> = match &entry.parent_id {
        Some(id) => tx
            .query_row(
                "SELECT depth, reset FROM entries WHERE session = ?1 AND entry_id = ?2",
                params![session, id],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get(1)?)),
            )
            .optional()?,
        None => None,
    };
    let depth = parent.as_ref().map_or(0, |(depth, _)| depth + 1);
    let reset = match &entry.payload {
        EntryPayload::Compaction(record) if record.reset_all => Some(entry.id.clone()),
        _ => parent.and_then(|(_, reset)| reset),
    };
    tx.execute(
        "INSERT OR IGNORE INTO entries (session, entry_id, segment, frame, parent, depth, reset)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            session,
            entry.id,
            at.segment as i64,
            at.frames as i64,
            entry.parent_id,
            depth as i64,
            reset
        ],
    )?;
    if let Some(parent) = &entry.parent_id {
        tx.execute(
            "INSERT OR IGNORE INTO jumps (session, entry_id, level, ancestor) VALUES (?1, ?2, 0, ?3)",
            params![session, entry.id, parent],
        )?;
        let mut previous = parent.clone();
        for level in 1..63 {
            let next: Option<String> = tx
                .query_row(
                    "SELECT ancestor FROM jumps WHERE session = ?1 AND entry_id = ?2 AND level = ?3",
                    params![session, previous, level - 1],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(next) = next else {
                break;
            };
            tx.execute(
                "INSERT OR IGNORE INTO jumps (session, entry_id, level, ancestor) VALUES (?1, ?2, ?3, ?4)",
                params![session, entry.id, level, next],
            )?;
            previous = next;
        }
    }
    if let EntryPayload::TurnCard(record) = &entry.payload {
        tx.execute(
            "INSERT OR IGNORE INTO turn_records (session, entry_id, turn_id, record, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session,
                entry.id,
                record.turn_id,
                compact_turn_record(record),
                entry.ts.to_rfc3339()
            ],
        )?;
    }
    Ok(())
}

fn is_ancestor(
    conn: &rusqlite::Connection,
    session: &str,
    ancestor: &str,
    leaf: &str,
) -> rusqlite::Result<bool> {
    let depth = |id: &str| -> rusqlite::Result<Option<u64>> {
        conn.query_row(
            "SELECT depth FROM entries WHERE session = ?1 AND entry_id = ?2",
            params![session, id],
            |row| Ok(row.get::<_, i64>(0)? as u64),
        )
        .optional()
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
                "SELECT ancestor FROM jumps WHERE session = ?1 AND entry_id = ?2 AND level = ?3",
                params![session, cursor, level],
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

impl Catalog {
    /// Takes what the session ledger at `dir` appended: the hint after a
    /// turn.
    pub fn catch_up_session(&self, dir: &std::path::Path) -> Result<usize, CatalogError> {
        self.catch_up_source(&crate::Source::Session(dir.to_path_buf()))
    }

    /// Where entry `entry_id` of session `session` (plain or `ses_`) is.
    /// The newest indexed entry of `session` (the deepest, then the last
    /// written): where a read of another conversation's history starts.
    pub fn newest_entry(&self, session: &str) -> Result<Option<String>, CatalogError> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT entry_id FROM entries WHERE session = ?1
                 ORDER BY depth DESC, segment DESC, frame DESC LIMIT 1",
                [ingest::session_node(session)],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn entry_location(
        &self,
        session: &str,
        entry_id: &str,
    ) -> Result<Option<EntryLocation>, CatalogError> {
        let node = ingest::session_node(session);
        let conn = self.conn();
        let found: Option<(i64, i64, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT e.segment, e.frame, e.parent, n.locator FROM entries e
                 JOIN nodes n ON n.id = e.session
                 WHERE e.session = ?1 AND e.entry_id = ?2",
                params![node, entry_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        Ok(found.and_then(|(segment, frame, parent, locator)| {
            Some(EntryLocation {
                dir: PathBuf::from(locator?),
                at: Position {
                    segment: segment as u64,
                    frames: frame as u64,
                },
                parent,
            })
        }))
    }

    /// Whether `ancestor` lies on the branch ending at `leaf`. An id from a
    /// sibling branch is never admitted by accident.
    pub fn is_ancestor(
        &self,
        session: &str,
        ancestor: &str,
        leaf: &str,
    ) -> Result<bool, CatalogError> {
        Ok(is_ancestor(
            &self.conn(),
            &ingest::session_node(session),
            ancestor,
            leaf,
        )?)
    }

    /// The entry holding turn `turn`'s card on the branch ending at `leaf`.
    pub fn locate_turn_record(
        &self,
        session: &str,
        turn: &str,
        leaf: &str,
    ) -> Result<Option<String>, CatalogError> {
        let session = ingest::session_node(session);
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT entry_id FROM turn_records WHERE session = ?1 AND turn_id = ?2 LIMIT 8",
        )?;
        let ids = statement
            .query_map(params![session, turn], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in ids {
            if is_ancestor(&conn, &session, &id, leaf)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Searches the settled turns on the branch ending at `leaf`. A person's
    /// words are quoted, never read as query syntax; `stop` ends the search
    /// early, which is an error, never an empty answer.
    pub fn search_turns(
        &self,
        session: &str,
        leaf: &str,
        query: &str,
        limit: usize,
        stop: &(dyn Fn() -> bool + Sync),
    ) -> Result<TurnSearchResult, CatalogError> {
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
        let session = ingest::session_node(session);
        let conn = self.conn();
        let started = std::time::Instant::now();
        let over = || stop() || started.elapsed() >= std::time::Duration::from_millis(250);
        let mut statement = conn.prepare(
            "SELECT r.turn_id, r.entry_id, r.record, bm25(turn_records_fts), r.recorded_at
             FROM turn_records_fts JOIN turn_records r ON r.rowid = turn_records_fts.rowid
             WHERE turn_records_fts MATCH ?1 AND r.session = ?2
             ORDER BY bm25(turn_records_fts), r.entry_id LIMIT 64",
        )?;
        let hits = statement
            .query_map(params![terms.join(" OR "), session], |row| {
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
            if over() {
                return Err(CatalogError::Interrupted);
            }
            if is_ancestor(&conn, &session, &hit.record_id, leaf)? {
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
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn logarithmic_links_distinguish_distant_ancestors_and_sibling_branches() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(&dir.path().join("catalog.db"), dir.path()).unwrap();
        let mut ids = Vec::new();
        catalog
            .seed(|tx| {
                let mut previous = None;
                for n in 0..2048u64 {
                    let entry = Entry::new(
                        previous,
                        EntryPayload::Message(vak_session::MessageRecord {
                            message: vak_llm::Message::user_text("subject"),
                            meta: None,
                        }),
                    );
                    let at = Position {
                        segment: 1,
                        frames: n,
                    };
                    index_entry(tx, "ses_s", at, &entry)?;
                    previous = Some(entry.id.clone());
                    ids.push(entry.id);
                }
                let sibling = Entry::new(
                    Some(ids[2].clone()),
                    EntryPayload::Message(vak_session::MessageRecord {
                        message: vak_llm::Message::user_text("different branch"),
                        meta: None,
                    }),
                );
                index_entry(tx, "ses_s", Position::default(), &sibling)?;
                ids.push(sibling.id);
                Ok(())
            })
            .unwrap();
        let sibling = ids[2048].clone();
        assert!(catalog.is_ancestor("s", &ids[3], &ids[2047]).unwrap());
        assert!(!catalog.is_ancestor("other", &ids[3], &ids[2047]).unwrap());
        assert!(catalog.is_ancestor("s", &ids[2], &sibling).unwrap());
        assert!(!catalog.is_ancestor("s", &ids[3], &sibling).unwrap());
        assert!(!catalog.is_ancestor("s", &sibling, &ids[2047]).unwrap());
        // A distant lookup uses logarithmic jumps, not one row per ancestor.
        let jumps: i64 = catalog
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM jumps WHERE entry_id = ?1",
                [&ids[2047]],
                |row| row.get(0),
            )
            .unwrap();
        assert!(jumps <= 12);
    }
}
