//! A session's own history, for the turn recall (docs/design/68 §3): where
//! each entry is, which entries lie on a branch, and the compact record of
//! each settled turn, searchable. These address and rank; the caller loads
//! the canonical entry and checks it before anything reaches a model.
//!
//! Branch membership is logarithmic in lookups and constant in storage:
//! besides its parent, each entry keeps one skew-binary jump pointer
//! (Myers' "applicative random-access stack"), so "is A on the branch
//! ending at B" walks O(log n) entries however long the history is. It
//! replaced a table of jumps to every 2^k-th ancestor, which held about
//! eight rows per entry and grew as n log n (measured 2026-10-09: 1.05 MB
//! of a 1.5 MB catalog after one 30-turn conversation).

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

/// A canonical frame address. No conversation body is duplicated in this
/// index; every reader checks the digest after opening the encrypted frame.
#[derive(Debug, Clone)]
pub struct HistoryFrame {
    pub id: String,
    pub parent: Option<String>,
    pub segment: u64,
    pub offset: u64,
    pub length: u64,
    pub bytes: u64,
    pub digest: String,
    pub directive: bool,
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub goal_head: Option<String>,
    pub goal_revision: Option<String>,
    pub goal_parent: Option<String>,
    pub selection_head: Option<String>,
}

pub(crate) fn index_frame(
    tx: &Transaction<'_>,
    session: &str,
    entry: &Entry,
    offset: u64,
    length: u64,
    bytes: &[u8],
) -> rusqlite::Result<()> {
    let target = match &entry.payload {
        EntryPayload::CardAction(record) => Some(record.presentation_id.clone()),
        EntryPayload::TurnSetAside(record) | EntryPayload::TurnRestored(record) => {
            Some(record.turn.clone())
        }
        EntryPayload::GoalUpdate(_) => Some("goal".into()),
        EntryPayload::Activity(record)
            if record.kind == vak_session::types::ActivityKind::PresentationSelection =>
        {
            Some("selection".into())
        }
        _ => None,
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let (mut turns, mut input, mut output): (i64, i64, i64) = match entry.parent_id.as_deref() {
        Some(parent) => tx.query_row("SELECT turns, input_tokens, output_tokens FROM history_frames WHERE session = ?1 AND entry_id = ?2",
            params![session, parent], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?.unwrap_or_default(),
        None => (0, 0, 0),
    };
    let (mut goal_head, mut goal_revision, mut selection_head): (Option<String>, Option<String>, Option<String>) = match entry.parent_id.as_deref() {
        Some(parent) => tx.query_row("SELECT goal_head, goal_revision, selection_head FROM history_frames WHERE session = ?1 AND entry_id = ?2",
            params![session, parent], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?.unwrap_or_default(),
        None => (None, None, None),
    };
    let mut goal_parent = None;
    if let EntryPayload::GoalUpdate(update) = &entry.payload {
        goal_revision = Some(entry.id.clone());
        if update.changes_state() {
            goal_parent = goal_head.clone();
            goal_head = Some(entry.id.clone());
        }
    }
    if target.as_deref() == Some("selection") {
        selection_head = Some(entry.id.clone());
    }
    turns += i64::from(entry.is_directive());
    if let EntryPayload::Message(record) = &entry.payload
        && let Some(usage) = record.meta.as_ref().and_then(|meta| meta.usage.as_ref())
    {
        input += usage.input_tokens as i64;
        output += usage.output_tokens as i64;
    }
    tx.execute("INSERT OR IGNORE INTO history_frames
        (session, entry_id, offset, length, bytes, digest, directive, target, turns, input_tokens, output_tokens, turn_id, goal_head, goal_revision, goal_parent, selection_head)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)", params![session, entry.id, offset as i64,
        length as i64, bytes.len() as i64, vak_session::types::line_digest(text), entry.is_directive(), target, turns, input, output, entry.at_turn, goal_head, goal_revision, goal_parent, selection_head])?;
    Ok(())
}

const FRAME_COLUMNS: &str = "e.entry_id, e.parent, e.segment, h.offset, h.length, h.bytes, h.digest, h.directive, h.turns, h.input_tokens, h.output_tokens, h.goal_head, h.goal_revision, h.goal_parent, h.selection_head";

fn frame_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryFrame> {
    Ok(HistoryFrame {
        id: row.get(0)?,
        parent: row.get(1)?,
        segment: row.get::<_, i64>(2)? as u64,
        offset: row.get::<_, i64>(3)? as u64,
        length: row.get::<_, i64>(4)? as u64,
        bytes: row.get::<_, i64>(5)? as u64,
        digest: row.get(6)?,
        directive: row.get(7)?,
        turns: row.get::<_, i64>(8)? as u64,
        input_tokens: row.get::<_, i64>(9)? as u64,
        output_tokens: row.get::<_, i64>(10)? as u64,
        goal_head: row.get(11)?,
        goal_revision: row.get(12)?,
        goal_parent: row.get(13)?,
        selection_head: row.get(14)?,
    })
}

impl Catalog {
    /// Sandbox rows for the calls in a human history page, read from their
    /// canonical frames. Index catch-up opens only the unread suffix.
    pub fn sandbox_rows(
        &self,
        dir: &std::path::Path,
        owner: Option<&str>,
        session: &str,
        executions: &[String],
    ) -> Result<Vec<serde_json::Value>, CatalogError> {
        if executions.is_empty() || !dir.is_dir() {
            return Ok(Vec::new());
        }
        let source = crate::Source::Sandbox {
            dir: dir.into(),
            session: owner.map(str::to_string),
        };
        self.catch_up_source(&source)?;
        let key = source.key();
        let addresses = {
            let conn = self.conn()?;
            let mut query = conn.prepare("SELECT segment, frame, offset, length, digest FROM sandbox_frames
                WHERE source = ?1 AND session = ?2 AND (execution = ?3 OR candidate IN
                    (SELECT candidate FROM sandbox_frames WHERE source = ?1 AND session = ?2 AND execution = ?3 AND candidate IS NOT NULL))")?;
            let mut found = std::collections::BTreeMap::new();
            for execution in executions {
                let rows = query.query_map(params![key, session, execution], |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u64,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, i64>(3)? as u64,
                        row.get::<_, String>(4)?,
                    ))
                })?;
                for row in rows {
                    let (segment, frame, offset, length, digest) = row?;
                    found.insert((segment, frame), (offset, length, digest));
                }
            }
            found
        };
        let chain = vak_session::chain::RecordChain::at(dir);
        let mut rows = Vec::new();
        for ((segment, _), (offset, length, digest)) in addresses {
            let mut row = chain.read_indexed(segment, offset, length, &digest)?;
            if vak_session::content::restore(&mut row)? == vak_session::content::Restored::Whole {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    pub fn history_header(&self, session: &str) -> Result<Option<HistoryFrame>, CatalogError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {FRAME_COLUMNS} FROM entries e JOIN history_frames h
            USING (session, entry_id) WHERE e.session = ?1 AND e.depth = 0 LIMIT 1"
                ),
                [ingest::session_node(session)],
                frame_row,
            )
            .optional()?)
    }
    pub fn history_frame(
        &self,
        session: &str,
        id: &str,
    ) -> Result<Option<HistoryFrame>, CatalogError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {FRAME_COLUMNS} FROM entries e JOIN history_frames h
            USING (session, entry_id) WHERE e.session = ?1 AND e.entry_id = ?2"
                ),
                params![ingest::session_node(session), id],
                frame_row,
            )
            .optional()?)
    }

    /// Physical append order identifies the active leaf, including a retry
    /// whose new branch is shorter than the branch it replaced.
    pub fn history_head(&self, session: &str) -> Result<Option<String>, CatalogError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row(
                "SELECT entry_id FROM entries WHERE session = ?1
            ORDER BY segment DESC, frame DESC LIMIT 1",
                [ingest::session_node(session)],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn history_turn_end(
        &self,
        session: &str,
        turn: &str,
        leaf: &str,
    ) -> Result<Option<String>, CatalogError> {
        let conn = self.conn()?;
        let key = ingest::session_node(session);
        let mut query = conn.prepare("SELECT h.entry_id FROM history_frames h JOIN entries e USING (session, entry_id)
            WHERE h.session = ?1 AND (h.turn_id = ?2 OR h.entry_id = ?2) ORDER BY e.segment DESC, e.frame DESC")?;
        let candidates = query.query_map(params![key, turn], |row| row.get::<_, String>(0))?;
        for id in candidates {
            let id = id?;
            if is_ancestor(&conn, &key, &id, leaf)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    pub fn history_page_end_after(
        &self,
        session: &str,
        end: &str,
        leaf: &str,
        turns: u64,
    ) -> Result<Option<String>, CatalogError> {
        let current = self.history_frame(session, end)?.map_or(0, |row| row.turns);
        let conn = self.conn()?;
        let key = ingest::session_node(session);
        let mut query = conn.prepare("SELECT h.entry_id FROM history_frames h JOIN entries e USING (session, entry_id)
            WHERE h.session = ?1 AND h.turns > ?2 AND h.turns <= ?3 ORDER BY e.segment DESC, e.frame DESC")?;
        let candidates = query.query_map(
            params![key, current as i64, current.saturating_add(turns) as i64],
            |row| row.get::<_, String>(0),
        )?;
        for id in candidates {
            let id = id?;
            if is_ancestor(&conn, &key, &id, leaf)? {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Candidate overlay addresses for a page, checked against its current
    /// branch by the caller. Targets are turn/presentation ids, never text.
    pub fn history_overlays(
        &self,
        session: &str,
        target: &str,
    ) -> Result<Vec<HistoryFrame>, CatalogError> {
        let conn = self.conn()?;
        let mut query = conn.prepare(&format!("SELECT {FRAME_COLUMNS} FROM history_frames h JOIN entries e
            USING (session, entry_id) WHERE h.session = ?1 AND h.target = ?2 ORDER BY e.segment, e.frame"))?;
        Ok(query
            .query_map(params![ingest::session_node(session), target], frame_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
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
    // How its calls ended is part of the record: a turn whose calls failed
    // or whose cards were refused is never read as an ordinary one
    // (docs/design/99-turns-that-do-not-poison.md §4 C).
    let calls = record.card.calls();
    let ended = match record.card.outcome.as_str() {
        "" | "completed" => String::new(),
        outcome => format!("\nEnded: {outcome}"),
    };
    let unmet = if record.card.unmet.is_empty() {
        String::new()
    } else {
        format!("\nNot delivered: {}", record.card.unmet_names())
    };
    format!("Asked: {asked}\nCalls: {calls}\nAnswered: {answer}\nOutputs: {titles}{unmet}{ended}")
}

/// Records where `entry` is, its place in the branch tree, and a settled
/// turn's compact record.
pub(crate) fn index_entry(
    tx: &Transaction<'_>,
    session: &str,
    at: Position,
    entry: &Entry,
) -> rusqlite::Result<()> {
    let parent = match &entry.parent_id {
        Some(id) => node(tx, session, id)?.map(|node| (id.clone(), node)),
        None => None,
    };
    let depth = parent.as_ref().map_or(0, |(_, node)| node.depth + 1);
    let reset = match &entry.payload {
        EntryPayload::Compaction(record) if record.reset_all => Some(entry.id.clone()),
        _ => parent.as_ref().and_then(|(_, node)| node.reset.clone()),
    };
    // The jump: two equal hops back from the parent when the parent's jump
    // and its jump's jump span equal distances, else the parent itself.
    let jump = match &parent {
        Some((id, up)) => {
            let skip = match (&up.jump, up.jump_depth) {
                (Some(jump), Some(jump_depth)) => node(tx, session, jump)?
                    .and_then(|over| Some((over.jump?, over.jump_depth?)))
                    .filter(|(_, far)| up.depth - jump_depth == jump_depth - far),
                _ => None,
            };
            Some(skip.unwrap_or((id.clone(), up.depth)))
        }
        None => None,
    };
    tx.execute(
        "INSERT OR IGNORE INTO entries
         (session, entry_id, segment, frame, parent, depth, reset, jump, jump_depth)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            session,
            entry.id,
            at.segment as i64,
            at.frames as i64,
            entry.parent_id,
            depth as i64,
            reset,
            jump.as_ref().map(|(id, _)| id.clone()),
            jump.as_ref().map(|(_, depth)| *depth as i64)
        ],
    )?;
    if let EntryPayload::TurnSetAside(record) | EntryPayload::TurnRestored(record) = &entry.payload
    {
        let aside = matches!(entry.payload, EntryPayload::TurnSetAside(_));
        tx.execute(
            "INSERT OR IGNORE INTO turn_marks (session, entry_id, turn_id, aside)
             VALUES (?1, ?2, ?3, ?4)",
            params![session, entry.id, record.turn, aside],
        )?;
        let turn = crate::ingest::turn_node(&record.turn);
        if aside {
            tx.execute(
                "INSERT OR IGNORE INTO turns_aside (turn) VALUES (?1)",
                [turn],
            )?;
        } else {
            tx.execute("DELETE FROM turns_aside WHERE turn = ?1", [turn])?;
        }
    }
    if let EntryPayload::TurnRetried(record) = &entry.payload {
        // The turn retried and every turn after it on its branch left the
        // chain: like a turn set aside, no search finds their calls or the
        // notes naming them (docs/design/99-turns-that-do-not-poison.md §4 F).
        let turns = tx
            .prepare("SELECT DISTINCT turn_id FROM turn_records WHERE session = ?1")?
            .query_map([session], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut left = vec![record.turn.clone()];
        for turn in turns {
            if turn != record.turn && is_ancestor(tx, session, &record.turn, &turn)? {
                left.push(turn);
            }
        }
        for turn in left {
            tx.execute(
                "INSERT OR IGNORE INTO turns_aside (turn) VALUES (?1)",
                [crate::ingest::turn_node(&turn)],
            )?;
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

/// One entry's place in its session's tree.
struct Node {
    depth: u64,
    parent: Option<String>,
    reset: Option<String>,
    jump: Option<String>,
    jump_depth: Option<u64>,
}

fn node(conn: &rusqlite::Connection, session: &str, id: &str) -> rusqlite::Result<Option<Node>> {
    conn.query_row(
        "SELECT depth, parent, reset, jump, jump_depth FROM entries
         WHERE session = ?1 AND entry_id = ?2",
        params![session, id],
        |row| {
            Ok(Node {
                depth: row.get::<_, i64>(0)? as u64,
                parent: row.get(1)?,
                reset: row.get(2)?,
                jump: row.get(3)?,
                jump_depth: row.get::<_, Option<i64>>(4)?.map(|depth| depth as u64),
            })
        },
    )
    .optional()
}

/// The ancestor of `leaf` at `depth`, and how many entries the walk read:
/// it takes the jump while that does not overshoot, else the parent.
fn ancestor_at(
    conn: &rusqlite::Connection,
    session: &str,
    leaf: &str,
    depth: u64,
) -> rusqlite::Result<(Option<String>, usize)> {
    let mut cursor = leaf.to_string();
    let mut steps = 0;
    loop {
        steps += 1;
        let Some(here) = node(conn, session, &cursor)? else {
            return Ok((None, steps));
        };
        if here.depth == depth {
            return Ok((Some(cursor), steps));
        }
        if here.depth < depth {
            return Ok((None, steps));
        }
        cursor = match (here.jump, here.jump_depth) {
            (Some(jump), Some(jump_depth)) if jump_depth >= depth => jump,
            _ => match here.parent {
                Some(parent) => parent,
                None => return Ok((None, steps)),
            },
        };
    }
}

fn is_ancestor(
    conn: &rusqlite::Connection,
    session: &str,
    ancestor: &str,
    leaf: &str,
) -> rusqlite::Result<bool> {
    let Some(target) = node(conn, session, ancestor)? else {
        return Ok(false);
    };
    let (found, _) = ancestor_at(conn, session, leaf, target.depth)?;
    Ok(found.as_deref() == Some(ancestor))
}

/// Whether a person set turn `turn` aside on the branch ending at `leaf`:
/// the deepest of its marks on that branch decides.
fn aside_on_branch(
    conn: &rusqlite::Connection,
    session: &str,
    turn: &str,
    leaf: &str,
) -> rusqlite::Result<bool> {
    let mut statement = conn.prepare(
        "SELECT m.entry_id, m.aside FROM turn_marks m
         JOIN entries e ON e.session = m.session AND e.entry_id = m.entry_id
         WHERE m.session = ?1 AND m.turn_id = ?2 ORDER BY e.depth DESC",
    )?;
    let marks = statement
        .query_map(params![session, turn], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (mark, aside) in marks {
        if is_ancestor(conn, session, &mark, leaf)? {
            return Ok(aside);
        }
    }
    Ok(false)
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
        let conn = self.conn()?;
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
        let conn = self.conn()?;
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
        let conn = self.conn()?;
        Ok(is_ancestor(
            &conn,
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
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT entry_id FROM turn_records WHERE session = ?1 AND turn_id = ?2 LIMIT 8",
        )?;
        let ids = statement
            .query_map(params![session, turn], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for id in ids {
            if is_ancestor(&conn, &session, &id, leaf)? {
                if aside_on_branch(&conn, &session, turn, leaf)? {
                    return Ok(None);
                }
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
        let query_terms = vak_session::turns::history_query_terms(query);
        let terms: Vec<String> = query_terms
            .iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect();
        if terms.is_empty() {
            return Ok(TurnSearchResult {
                matches: vec![],
                candidate_limit_reached: false,
            });
        }
        let session = ingest::session_node(session);
        let conn = self.conn()?;
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
            if is_ancestor(&conn, &session, &hit.record_id, leaf)?
                && !aside_on_branch(&conn, &session, &hit.turn_id, leaf)?
            {
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
    #[allow(clippy::disallowed_macros)] // Explicit test measurements, never runtime output.
    fn summary_pages_cover_tied_timestamps_at_chat_scale() {
        vak_config::paths::isolate_home_for_tests();
        for count in [100, 1_000, 10_000] {
            let dir = tempfile::tempdir().unwrap();
            let catalog = Catalog::open(&dir.path().join("catalog.db"), dir.path()).unwrap();
            catalog.seed(|tx| {
                for number in 0..count {
                    tx.execute("INSERT INTO nodes (id, kind, created_at, updated_at) VALUES (?1, 'session', '2026-10-21', '2026-10-21')", [format!("ses_{number:05}")])?;
                }
                Ok(())
            }).unwrap();
            let started = std::time::Instant::now();
            let first = catalog
                .session_page(&crate::Audience::default(), None, 40)
                .unwrap();
            assert_eq!(first.len(), 40);
            let first_ms = started.elapsed().as_millis();
            let mut seen = std::collections::HashSet::new();
            let mut boundary: Option<(String, String)> = None;
            loop {
                let rows = catalog
                    .session_page(
                        &crate::Audience::default(),
                        boundary.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
                        40,
                    )
                    .unwrap();
                if rows.is_empty() {
                    break;
                }
                for row in &rows {
                    assert!(seen.insert(row.id.clone()));
                }
                let last = rows.last().unwrap();
                boundary = Some((last.updated_at.clone().unwrap(), last.id.clone()));
            }
            assert_eq!(seen.len(), count);
            println!(
                "chats={count} first_page_ms={first_ms} summaries={}",
                first.len()
            );
        }
    }

    #[test]
    fn sandbox_addresses_select_only_requested_execution_and_catch_up() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(&dir.path().join("catalog.db"), dir.path()).unwrap();
        let path = dir.path().join("executions");
        let chain =
            vak_session::chain::RecordChain::at(&path).of_conversation("sandbox-history-test");
        chain.append_all(&[
            serde_json::json!({"kind":"ExecutionStarted","execution_id":"a","code_preview":"first"}),
            serde_json::json!({"kind":"ExecutionStarted","execution_id":"b","code_preview":"unrelated"}),
            serde_json::json!({"kind":"ExecutionFinished","execution_id":"a","exit_code":0}),
        ]).unwrap();
        let rows = catalog
            .sandbox_rows(
                &path,
                Some("sandbox-history-test"),
                "sandbox-history-test",
                &["a".into()],
            )
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row["execution_id"] == "a"));
        chain.append(&serde_json::json!({"kind":"ArtifactGenerated","execution_id":"a","path":"draft.txt"})).unwrap();
        let rows = catalog
            .sandbox_rows(
                &path,
                Some("sandbox-history-test"),
                "sandbox-history-test",
                &["a".into()],
            )
            .unwrap();
        assert_eq!(rows.len(), 3);
        assert!(
            catalog
                .sandbox_rows(&path, Some("sandbox-history-test"), "other", &["a".into()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn recall_search_ranks_the_turn_that_carries_the_subject_first() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(&dir.path().join("catalog.db"), dir.path()).unwrap();
        let mut entry_ids = Vec::new();
        catalog
            .seed(|tx| {
                let mut parent = None;
                for (turn, record) in [
                    (
                        "old-art-club",
                        "Asked: Plan a 35-minute art club for 8 kids, setup, painting and cleanup. Budget $28 for materials and paint with a supply checklist.",
                    ),
                    (
                        "book-club",
                        "Asked: 24-minute book club schedule welcome reading wrap-up budget tea fruit.",
                    ),
                ] {
                    let entry = Entry::new(
                        parent,
                        EntryPayload::Message(vak_session::MessageRecord {
                            message: vak_llm::Message::user_text("turn"),
                            meta: None,
                        }),
                    );
                    index_entry(
                        tx,
                        "ses_recall",
                        Position {
                            segment: 1,
                            frames: entry_ids.len() as u64,
                        },
                        &entry,
                    )?;
                    tx.execute(
                        "INSERT INTO turn_records (session, entry_id, turn_id, record, recorded_at)\n                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            "ses_recall",
                            entry.id,
                            turn,
                            record,
                            "2026-10-10T00:00:00Z"
                        ],
                    )?;
                    parent = Some(entry.id.clone());
                    entry_ids.push(entry.id);
                }
                Ok(())
            })
            .unwrap();

        let result = catalog
            .search_turns(
                "recall",
                entry_ids.last().unwrap(),
                "24-minute book club schedule welcome reading budget tea fruit",
                8,
                &|| false,
            )
            .unwrap();

        assert_eq!(
            result.matches[0].turn_id, "book-club",
            "{:#?}",
            result.matches
        );

        // The query that missed live on 2026-10-15: words the turn's record
        // lacks (a misspelt "remember", an id) must not hide the turn whose
        // words it shares.
        let result = catalog
            .search_turns(
                "recall",
                entry_ids.last().unwrap(),
                "book club remember that 01a13fee-7d7a-7171 assistant vakyartha",
                8,
                &|| false,
            )
            .unwrap();
        assert_eq!(
            result.matches[0].turn_id, "book-club",
            "{:#?}",
            result.matches
        );
    }

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
        // Every pair on the chain agrees with a plain walk up the parents,
        // in a logarithmic number of reads, and each entry is one row.
        let conn = catalog.conn().unwrap();
        let mut longest = 0;
        for leaf in [5usize, 100, 1023, 1024, 2047] {
            for target in [0usize, 1, 2, 3, 511, 512, 1000, leaf] {
                if target > leaf {
                    continue;
                }
                let (found, steps) =
                    ancestor_at(&conn, "ses_s", &ids[leaf], target as u64).unwrap();
                assert_eq!(
                    found.as_deref(),
                    Some(ids[target].as_str()),
                    "{leaf} -> {target}"
                );
                longest = longest.max(steps);
            }
        }
        assert!(longest <= 3 * 11 + 2, "{longest} reads for 2048 entries");
        let rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE session = 'ses_s'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 2049);
    }

    /// A turn a person set aside is found by no search on the branch that
    /// set it aside, and is again once restored (docs/design/99 §4 F).
    #[test]
    fn a_turn_set_aside_is_not_found_until_restored() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(&dir.path().join("catalog.db"), dir.path()).unwrap();
        let mut leaves = Vec::new();
        catalog
            .seed(|tx| {
                let directive = Entry::new(
                    None,
                    EntryPayload::Message(vak_session::MessageRecord {
                        message: vak_llm::Message::user_text("turn"),
                        meta: None,
                    }),
                );
                let mark = |parent: &str, aside: bool| {
                    let record = vak_session::turn_marks::TurnMarkRecord {
                        turn: "route".into(),
                        trace: None,
                        actor: None,
                    };
                    Entry::new(
                        Some(parent.to_string()),
                        if aside {
                            EntryPayload::TurnSetAside(record)
                        } else {
                            EntryPayload::TurnRestored(record)
                        },
                    )
                };
                let aside = mark(&directive.id, true);
                let restored = mark(&aside.id, false);
                for (frame, entry) in [&directive, &aside, &restored].into_iter().enumerate() {
                    index_entry(
                        tx,
                        "ses_aside",
                        Position {
                            segment: 1,
                            frames: frame as u64,
                        },
                        entry,
                    )?;
                }
                tx.execute(
                    "INSERT INTO turn_records (session, entry_id, turn_id, record, recorded_at)
                     VALUES ('ses_aside', ?1, 'route', 'Asked: bike route map', '2026-10-17T00:00:00Z')",
                    [&directive.id],
                )?;
                leaves = vec![directive.id, aside.id, restored.id];
                Ok(())
            })
            .unwrap();
        let found = |leaf: &str| {
            catalog
                .search_turns("aside", leaf, "bike route", 8, &|| false)
                .unwrap()
                .matches
                .len()
        };
        assert_eq!(found(&leaves[0]), 1);
        assert_eq!(found(&leaves[1]), 0);
        assert_eq!(
            catalog
                .locate_turn_record("aside", "route", &leaves[1])
                .unwrap(),
            None
        );
        assert_eq!(found(&leaves[2]), 1);
    }

    /// Retrying a turn takes it, and every turn after it on its branch, out
    /// of every search; the turns before it stay (docs/design/99 §4 F).
    #[test]
    fn a_retried_turn_and_the_turns_after_it_leave_every_search() {
        let dir = tempfile::tempdir().unwrap();
        let catalog = Catalog::open(&dir.path().join("catalog.db"), dir.path()).unwrap();
        let mut turns = Vec::new();
        catalog
            .seed(|tx| {
                let message = |parent: Option<&str>, text: &str| {
                    Entry::new(
                        parent.map(str::to_string),
                        EntryPayload::Message(vak_session::MessageRecord {
                            message: vak_llm::Message::user_text(text),
                            meta: None,
                        }),
                    )
                };
                let first = message(None, "one");
                let second = message(Some(&first.id), "two");
                let third = message(Some(&second.id), "three");
                let retried = Entry::new(
                    Some(first.id.clone()),
                    EntryPayload::TurnRetried(vak_session::turn_marks::TurnMarkRecord {
                        turn: second.id.clone(),
                        trace: None,
                        actor: None,
                    }),
                );
                for (frame, entry) in [&first, &second, &third].into_iter().enumerate() {
                    index_entry(
                        tx,
                        "ses_retry",
                        Position {
                            segment: 1,
                            frames: frame as u64,
                        },
                        entry,
                    )?;
                    tx.execute(
                        "INSERT INTO turn_records (session, entry_id, turn_id, record, recorded_at)
                         VALUES ('ses_retry', ?1, ?1, 'Asked: turn', '2026-10-17T00:00:00Z')",
                        [&entry.id],
                    )?;
                }
                index_entry(
                    tx,
                    "ses_retry",
                    Position {
                        segment: 1,
                        frames: 3,
                    },
                    &retried,
                )?;
                turns = vec![first.id, second.id, third.id];
                Ok(())
            })
            .unwrap();
        let conn = catalog.conn.lock().unwrap();
        let out = |turn: &str| -> bool {
            conn.query_row(
                "SELECT COUNT(*) FROM turns_aside WHERE turn = ?1",
                [crate::ingest::turn_node(turn)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        };
        assert!(!out(&turns[0]));
        assert!(out(&turns[1]));
        assert!(out(&turns[2]));
    }
}
