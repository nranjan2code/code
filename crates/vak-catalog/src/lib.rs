//! The data catalog (data-architecture plan M6, docs/design/73 §9): one
//! SQLite file per tenant answering "what is this, where does it live, what
//! caused it, and what mentions this word" for every session, turn, tool
//! call, file a call wrote, run, effect, trigger, memory note and
//! commitment.
//!
//! It is Derived. The records are its only inputs: session ledgers, the
//! `runs/`, `effects/` and commitment chains, and the trigger and memory
//! Documents. A tailer keeps one cursor per source (`vak_session::tail`)
//! and takes only what was appended since; it runs after each turn, on the
//! scheduler tick and before a query answers, and a writer only hints, so a
//! missed hint costs latency, never correctness. Each source's catch-up is
//! one transaction with its cursor, so ingest is idempotent from any
//! process. [`Catalog::rebuild`] replays every source from its start and
//! must produce the same rows.
//!
//! The catalog decides nothing that must be right (plan §M6 design): it
//! finds and explains. Lineage edges point from a thing to what produced
//! it: a file is `produced_by` its call, a call `produced_by` its turn, a
//! turn `produced_by` its run and `part_of` its session, a session
//! `caused_by` its run, a run `caused_by` its trigger or parent run, and an
//! effect `produced_by` its run.
//!
//! The text index holds doc 73's projections only: user and assistant
//! text, each tool result's digest, memory notes, trigger names and
//! commitment objectives. Never thinking, never raw tool output.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use vak_session::tail::{self, Position};

mod history;
mod ingest;
mod sources;

pub use history::{EntryLocation, TurnSearchHit, TurnSearchResult, compact_turn_record};
pub use sources::Source;

/// The catalog's directory in a tenant home (doc 73 §6), holding the
/// SQLite file and its sidecars.
pub const DIR_NAME: &str = "catalog";
/// The catalog's file name in that directory.
pub const FILE_NAME: &str = "catalog.db";

/// The schema this build writes. A catalog stamped with another is dropped
/// and rebuilt, never adapted: it is derived.
const SCHEMA_VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("catalog database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("catalog file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the lookup was cancelled or ran out of time")]
    Interrupted,
}

/// One addressable thing the catalog knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The Agent's id as people and configuration name it (`vak`,
    /// `scout`), where the record says it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// When anything was last added to it (a session's newest entry).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// How many entries a session holds; a file's bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    /// Where its bytes live: a ledger directory, a chain, a Document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
}

/// Who is asking, and so what a query may return (invariant 37). The
/// default reads everything the owner may, held intake items aside.
#[derive(Debug, Clone, Default)]
pub struct Audience {
    /// The Agents whose things may be returned (as `AgentId`s); `None` is
    /// every Agent.
    pub agents: Option<Vec<String>>,
    /// The conversation audience a result must belong to; `None` is any.
    /// Something with no audience of its own (an Agent's memory note, its
    /// entities) belongs to its Agent: it is returned to an audience only
    /// when `agents` names that Agent.
    pub audience: Option<String>,
    /// Sessions that must not appear (the trash), as their plain ids.
    pub exclude_sessions: HashSet<String>,
    /// A principal outside the owner's own audience (an invited person,
    /// plan M8.2): only what an active grant opens to them, a granted
    /// conversation's turns and calls included, is returned.
    pub principal: Option<String>,
    /// Whether intake items detection held (or a person quarantined) are
    /// returned. Only a person's own view sets it; an Agent's retrieval
    /// never does (plan M6.5, doc 76 §5), so the default is closed.
    pub held: bool,
}

/// What a search looks through, beside who is asking: a space (things of
/// no space, such as profile notes, are always in), and kinds.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub space: Option<String>,
    pub kinds: Option<Vec<String>>,
}

/// One search result.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub node: Node,
    pub snippet: String,
    pub score: f64,
}

/// What caused a node: the path up to its root, and the coordinates a
/// person asks for first.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Lineage {
    /// The node, then each thing above it, nearest first.
    pub path: Vec<Node>,
    pub run: Option<Node>,
    pub session: Option<Node>,
    pub turn: Option<Node>,
    pub agent: Option<String>,
    pub space: Option<String>,
    pub actor: Option<String>,
    pub cause: Option<String>,
}

/// What a catch-up took.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CatchUp {
    pub sources: usize,
    pub rows: usize,
}

pub struct Catalog {
    path: PathBuf,
    /// The data home whose records it reads.
    data: PathBuf,
    conn: Mutex<Connection>,
}

impl Catalog {
    /// Opens (or creates) the catalog at `path` over the records under the
    /// data home `data`.
    pub fn open(path: &Path, data: &Path) -> Result<Self, CatalogError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = connect(path)?;
        let stamped: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |row| {
                row.get(0)
            })
            .optional()
            .unwrap_or(None);
        let conn = if stamped.as_deref() == Some(&SCHEMA_VERSION.to_string()) {
            conn
        } else {
            drop(conn);
            remove_db(path);
            connect(path)?
        };
        Ok(Self {
            path: path.to_path_buf(),
            data: data.to_path_buf(),
            conn: Mutex::new(conn),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Takes what every source appended since its cursor.
    pub fn catch_up(&self) -> Result<CatchUp, CatalogError> {
        let mut total = CatchUp::default();
        for source in sources::discover(&self.data) {
            total.rows += self.catch_up_source(&source)?;
            total.sources += 1;
        }
        Ok(total)
    }

    /// Takes what one source appended since its cursor: the hint a writer
    /// sends after an append. Returns how many rows it took.
    pub fn catch_up_source(&self, source: &Source) -> Result<usize, CatalogError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let from = cursor(&tx, &source.key())?;
        let (to, rows) = ingest::source(&tx, source, from)?;
        if to != from {
            tx.execute(
                "INSERT INTO cursors (source, segment, frames) VALUES (?1, ?2, ?3)
                 ON CONFLICT(source) DO UPDATE SET segment = ?2, frames = ?3",
                params![source.key(), to.segment as i64, to.frames as i64],
            )?;
        }
        tx.commit()?;
        Ok(rows)
    }

    /// The session ledger at `dir` as a source, for a hint after a turn.
    pub fn session_source(&self, dir: &Path) -> Source {
        Source::Session(dir.to_path_buf())
    }

    /// Whether any source has rows the catalog has not taken.
    pub fn stale(&self) -> Result<bool, CatalogError> {
        let conn = self.conn();
        for source in sources::discover(&self.data) {
            let from = cursor(&conn, &source.key())?;
            if source.has_more(from) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Drops every row and replays every source from its start.
    pub fn rebuild(&self) -> Result<CatchUp, CatalogError> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            for table in [
                "nodes",
                "edges",
                "texts",
                "calls",
                "cursors",
                "entries",
                "jumps",
                "turn_records",
                "grants",
                "broken",
            ] {
                tx.execute(&format!("DELETE FROM {table}"), [])?;
            }
            tx.commit()?;
        }
        self.catch_up()
    }

    /// The node `id`, if the catalog knows it. A session's plain id finds
    /// its node too.
    pub fn open_node(&self, id: &str) -> Result<Option<Node>, CatalogError> {
        let conn = self.conn();
        if let Some(node) = node(&conn, id)? {
            return Ok(Some(node));
        }
        node(&conn, &format!("ses_{id}"))
    }

    /// Where the session `id` (plain or `ses_`) is stored: its ledger
    /// directory.
    pub fn session_dir(&self, id: &str) -> Result<Option<PathBuf>, CatalogError> {
        Ok(self
            .open_node(id)?
            .filter(|node| node.kind == "session")
            .and_then(|node| node.locator)
            .map(PathBuf::from))
    }

    /// The nodes of `kind` the audience may see, newest first.
    pub fn list(
        &self,
        kind: &str,
        audience: &Audience,
        limit: usize,
    ) -> Result<Vec<Node>, CatalogError> {
        let conn = self.conn();
        let (filter, args) = audience_filter(audience, 2);
        let sql = format!(
            "SELECT {NODE_COLUMNS} FROM nodes n WHERE n.kind = ?1 AND {filter}
             ORDER BY n.created_at DESC LIMIT {}",
            limit.clamp(1, 10_000)
        );
        let mut values: Vec<rusqlite::types::Value> = vec![kind.to_string().into()];
        values.extend(args);
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), row_node)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// What a session holds (its turns, calls, files, effects, notes),
    /// oldest first, without the session itself.
    pub fn in_session(&self, session: &str, limit: usize) -> Result<Vec<Node>, CatalogError> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {NODE_COLUMNS} FROM nodes n WHERE n.session = ?1 AND n.id != ?1
             ORDER BY n.created_at, n.id LIMIT {}",
            limit.clamp(1, 10_000)
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map([ingest::session_node(session)], row_node)?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// The things matching `query` that `audience` may see, best first.
    /// The audience filters before ranking, so a result the caller may not
    /// open never takes a place in the ranking (invariant 37).
    pub fn search(
        &self,
        query: &str,
        audience: &Audience,
        scope: &Scope,
        limit: usize,
    ) -> Result<Vec<Hit>, CatalogError> {
        let Some(matched) = fts_query(query) else {
            return Ok(Vec::new());
        };
        let conn = self.conn();
        let (mut filter, mut args) = audience_filter(audience, 2);
        let mut next = 2 + args.len();
        if let Some(space) = &scope.space {
            filter.push_str(&format!(" AND (n.space = ?{next} OR n.space IS NULL)"));
            args.push(space.clone().into());
            next += 1;
        }
        if let Some(kinds) = &scope.kinds {
            filter.push_str(&format!(
                " AND n.kind IN (SELECT value FROM json_each(?{next}))"
            ));
            args.push(
                serde_json::to_string(kinds)
                    .unwrap_or_else(|_| "[]".into())
                    .into(),
            );
        }
        let sql = format!(
            "SELECT {NODE_COLUMNS}, snippet(texts_fts, 0, '', '', '…', 16), bm25(texts_fts)
             FROM texts_fts
             JOIN texts t ON t.id = texts_fts.rowid
             JOIN nodes n ON n.id = t.node
             WHERE texts_fts MATCH ?1 AND {filter}
             ORDER BY bm25(texts_fts) LIMIT {}",
            limit.clamp(1, 200)
        );
        let mut values: Vec<rusqlite::types::Value> = vec![matched.into()];
        values.extend(args);
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok(Hit {
                node: row_node(row)?,
                snippet: row.get(NODE_COLUMN_COUNT)?,
                score: -row.get::<_, f64>(NODE_COLUMN_COUNT + 1)?,
            })
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// What caused `id`: the path from it up to its root, and its run,
    /// session, turn, Agent, space, actor and cause.
    pub fn lineage(&self, id: &str) -> Result<Option<Lineage>, CatalogError> {
        let conn = self.conn();
        let start = match node(&conn, id)? {
            Some(start) => start,
            None => match node(&conn, &format!("ses_{id}"))? {
                Some(start) => start,
                None => return Ok(None),
            },
        };
        let mut lineage = Lineage::default();
        let mut seen = HashSet::new();
        let mut frontier = vec![start];
        let mut up = conn.prepare(
            "SELECT dst FROM edges WHERE src = ?1
             AND kind IN ('produced_by', 'part_of', 'caused_by', 'derived_from') ORDER BY kind, dst",
        )?;
        while let Some(current) = frontier.first().cloned() {
            frontier.remove(0);
            if !seen.insert(current.id.clone()) {
                continue;
            }
            let parents: Vec<String> = up
                .query_map([&current.id], |row| row.get(0))?
                .filter_map(Result::ok)
                .collect();
            for parent in parents {
                if let Some(parent) = node(&conn, &parent)? {
                    frontier.push(parent);
                }
            }
            match current.kind.as_str() {
                "run" if lineage.run.is_none() => lineage.run = Some(current.clone()),
                "session" if lineage.session.is_none() => lineage.session = Some(current.clone()),
                "turn" if lineage.turn.is_none() => lineage.turn = Some(current.clone()),
                _ => {}
            }
            lineage.path.push(current);
            if lineage.path.len() >= 64 {
                break;
            }
        }
        let first = |pick: fn(&Node) -> Option<String>| {
            lineage.run.iter().chain(lineage.path.iter()).find_map(pick)
        };
        lineage.agent = first(|node| node.agent.clone());
        lineage.space = first(|node| node.space.clone());
        lineage.actor = first(|node| node.actor.clone());
        lineage.cause = first(|node| node.cause.clone());
        Ok(Some(lineage))
    }

    /// How many nodes of each kind the catalog holds.
    pub fn counts(&self) -> Result<BTreeMap<String, u64>, CatalogError> {
        let conn = self.conn();
        let mut statement = conn.prepare("SELECT kind, COUNT(*) FROM nodes GROUP BY kind")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Every row, sorted, as text: for tests that compare two catalogs.
    #[doc(hidden)]
    pub fn dump(&self) -> Result<Vec<String>, CatalogError> {
        let conn = self.conn();
        let mut out = Vec::new();
        for sql in [
            "SELECT id || '|' || kind || '|' || IFNULL(space,'') || '|' || IFNULL(agent,'') || '|' ||
             IFNULL(session,'') || '|' || IFNULL(turn,'') || '|' || IFNULL(run,'') || '|' ||
             IFNULL(actor,'') || '|' || IFNULL(cause,'') || '|' || IFNULL(audience,'') || '|' ||
             IFNULL(title,'') || '|' || IFNULL(status,'') || '|' || IFNULL(created_at,'') || '|' ||
             IFNULL(updated_at,'') || '|' || IFNULL(size,'') || '|' || IFNULL(agent_name,'') || '|' ||
             IFNULL(locator,'') FROM nodes",
            "SELECT 'edge|' || src || '|' || kind || '|' || dst FROM edges",
            "SELECT 'grant|' || grant_id || '|' || node || '|' || principal || '|' || role || '|' ||
                    IFNULL(expires,'') || '|' || revoked FROM grants",
            "SELECT 'broken|' || node FROM broken",
            "SELECT 'text|' || node || '|' || body FROM texts",
            "SELECT 'entry|' || session || '|' || entry_id || '|' || segment || '|' || frame || '|' ||
             IFNULL(parent,'') || '|' || depth || '|' || IFNULL(reset,'') FROM entries",
            "SELECT 'jump|' || session || '|' || entry_id || '|' || level || '|' || ancestor FROM jumps",
            "SELECT 'turn|' || session || '|' || entry_id || '|' || turn_id || '|' || record FROM turn_records",
        ] {
            let mut statement = conn.prepare(sql)?;
            out.extend(
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .filter_map(Result::ok),
            );
        }
        out.sort();
        Ok(out)
    }

    /// Runs `seed` against the open connection inside one transaction: for
    /// tests that load a catalog of a given size without records behind it.
    #[doc(hidden)]
    pub fn seed(
        &self,
        seed: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<()>,
    ) -> Result<(), CatalogError> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        seed(&tx)?;
        tx.commit()?;
        Ok(())
    }
}

fn connect(path: &Path) -> Result<Connection, CatalogError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT OR IGNORE INTO meta (key, value) VALUES ('schema', '{SCHEMA_VERSION}');
         CREATE TABLE IF NOT EXISTS cursors (
             source TEXT PRIMARY KEY, segment INTEGER NOT NULL, frames INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS nodes (
             id TEXT PRIMARY KEY, kind TEXT NOT NULL, space TEXT, agent TEXT, agent_name TEXT,
             session TEXT, turn TEXT, run TEXT, actor TEXT, cause TEXT, audience TEXT,
             title TEXT, status TEXT, created_at TEXT, updated_at TEXT, size INTEGER,
             locator TEXT);
         CREATE INDEX IF NOT EXISTS nodes_kind ON nodes (kind, created_at);
         CREATE INDEX IF NOT EXISTS nodes_session ON nodes (session);
         CREATE TABLE IF NOT EXISTS edges (
             src TEXT NOT NULL, kind TEXT NOT NULL, dst TEXT NOT NULL,
             PRIMARY KEY (src, kind, dst));
         CREATE INDEX IF NOT EXISTS edges_dst ON edges (dst, kind);
         CREATE TABLE IF NOT EXISTS grants (
             grant_id TEXT PRIMARY KEY, node TEXT NOT NULL, principal TEXT NOT NULL,
             role TEXT NOT NULL, expires TEXT, revoked INTEGER NOT NULL DEFAULT 0);
         CREATE INDEX IF NOT EXISTS grants_principal ON grants (principal, node);
         CREATE TABLE IF NOT EXISTS broken (node TEXT PRIMARY KEY);
         CREATE TABLE IF NOT EXISTS calls (
             node TEXT PRIMARY KEY, tool TEXT NOT NULL, input TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS texts (
             id INTEGER PRIMARY KEY, node TEXT NOT NULL UNIQUE, body TEXT NOT NULL);
         CREATE VIRTUAL TABLE IF NOT EXISTS texts_fts USING fts5(
             body, content='texts', content_rowid='id', tokenize='unicode61');
         CREATE TRIGGER IF NOT EXISTS texts_ai AFTER INSERT ON texts BEGIN
             INSERT INTO texts_fts (rowid, body) VALUES (new.id, new.body); END;
         CREATE TRIGGER IF NOT EXISTS texts_ad AFTER DELETE ON texts BEGIN
             INSERT INTO texts_fts (texts_fts, rowid, body) VALUES ('delete', old.id, old.body); END;
         CREATE TRIGGER IF NOT EXISTS texts_au AFTER UPDATE ON texts BEGIN
             INSERT INTO texts_fts (texts_fts, rowid, body) VALUES ('delete', old.id, old.body);
             INSERT INTO texts_fts (rowid, body) VALUES (new.id, new.body); END;
         CREATE TABLE IF NOT EXISTS entries (
             session TEXT NOT NULL, entry_id TEXT NOT NULL, segment INTEGER NOT NULL,
             frame INTEGER NOT NULL, parent TEXT, depth INTEGER NOT NULL, reset TEXT,
             PRIMARY KEY (session, entry_id));
         CREATE TABLE IF NOT EXISTS jumps (
             session TEXT NOT NULL, entry_id TEXT NOT NULL, level INTEGER NOT NULL,
             ancestor TEXT NOT NULL, PRIMARY KEY (session, entry_id, level));
         CREATE TABLE IF NOT EXISTS turn_records (
             session TEXT NOT NULL, entry_id TEXT NOT NULL, turn_id TEXT NOT NULL,
             record TEXT NOT NULL, recorded_at TEXT NOT NULL,
             PRIMARY KEY (session, entry_id));
         CREATE INDEX IF NOT EXISTS turn_records_turn ON turn_records (session, turn_id);
         CREATE VIRTUAL TABLE IF NOT EXISTS turn_records_fts USING fts5(
             record, content='turn_records', content_rowid='rowid', tokenize='unicode61');
         CREATE TRIGGER IF NOT EXISTS turn_records_ai AFTER INSERT ON turn_records BEGIN
             INSERT INTO turn_records_fts (rowid, record) VALUES (new.rowid, new.record); END;
         CREATE TRIGGER IF NOT EXISTS turn_records_ad AFTER DELETE ON turn_records BEGIN
             INSERT INTO turn_records_fts (turn_records_fts, rowid, record)
             VALUES ('delete', old.rowid, old.record); END;"
    ))?;
    Ok(conn)
}

fn remove_db(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

fn cursor(conn: &Connection, source: &str) -> Result<Position, CatalogError> {
    Ok(conn
        .query_row(
            "SELECT segment, frames FROM cursors WHERE source = ?1",
            [source],
            |row| {
                Ok(Position {
                    segment: row.get::<_, i64>(0)? as u64,
                    frames: row.get::<_, i64>(1)? as u64,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

const NODE_COLUMNS: &str = "n.id, n.kind, n.space, n.agent, n.agent_name, n.session, n.turn, \
     n.run, n.actor, n.cause, n.audience, n.title, n.status, n.created_at, n.updated_at, n.size, \
     n.locator";
const NODE_COLUMN_COUNT: usize = 17;

fn row_node(row: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
        id: row.get(0)?,
        kind: row.get(1)?,
        space: row.get(2)?,
        agent: row.get(3)?,
        agent_name: row.get(4)?,
        session: row.get(5)?,
        turn: row.get(6)?,
        run: row.get(7)?,
        actor: row.get(8)?,
        cause: row.get(9)?,
        audience: row.get(10)?,
        title: row.get(11)?,
        status: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        size: row.get(15)?,
        locator: row.get(16)?,
    })
}

fn node(conn: &Connection, id: &str) -> Result<Option<Node>, CatalogError> {
    Ok(conn
        .query_row(
            &format!("SELECT {NODE_COLUMNS} FROM nodes n WHERE n.id = ?1"),
            [id],
            row_node,
        )
        .optional()?)
}

/// The SQL that keeps a query inside `audience`, with its arguments
/// numbered from `first`.
fn audience_filter(audience: &Audience, first: usize) -> (String, Vec<rusqlite::types::Value>) {
    let mut clauses = vec!["1 = 1".to_string()];
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    let mut next = first;
    if let Some(agents) = &audience.agents {
        clauses.push(format!("n.agent IN (SELECT value FROM json_each(?{next}))"));
        args.push(
            serde_json::to_string(agents)
                .unwrap_or_else(|_| "[]".into())
                .into(),
        );
        next += 1;
    }
    if let Some(wanted) = &audience.audience {
        if audience.agents.is_some() {
            clauses.push(format!("(n.audience = ?{next} OR n.audience IS NULL)"));
        } else {
            clauses.push(format!("n.audience = ?{next}"));
        }
        args.push(wanted.clone().into());
        next += 1;
    }
    if !audience.held {
        clauses.push("(n.kind != 'item' OR n.status = 'accepted')".to_string());
    }
    // An Agent reads an artifact through its Space only while the artifact
    // inherits; once a share breaks inheritance, a grant must name it.
    if audience.agents.is_some() {
        clauses.push("(n.kind != 'artifact' OR n.id NOT IN (SELECT node FROM broken))".to_string());
    }
    if let Some(principal) = &audience.principal {
        clauses.push(format!(
            "(n.id IN ({granted}) OR IFNULL(n.session, '') IN ({granted}))",
            granted = format!(
                "SELECT node FROM grants WHERE principal = ?{next} AND revoked = 0 \
                 AND (expires IS NULL OR expires > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))"
            )
        ));
        args.push(principal.clone().into());
        next += 1;
    }
    if !audience.exclude_sessions.is_empty() {
        let excluded: Vec<String> = audience
            .exclude_sessions
            .iter()
            .map(|id| ingest::session_node(id))
            .collect();
        clauses.push(format!(
            "IFNULL(n.session, '') NOT IN (SELECT value FROM json_each(?{next}))"
        ));
        args.push(
            serde_json::to_string(&excluded)
                .unwrap_or_else(|_| "[]".into())
                .into(),
        );
    }
    (clauses.join(" AND "), args)
}

/// A person's words as an FTS5 query: every word must appear, each quoted
/// so nothing they type is read as query syntax.
fn fts_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .map(|word| format!("\"{}\"", word.replace('"', "")))
        .collect();
    (!words.is_empty()).then(|| words.join(" "))
}

/// The tail position a source starts from, for tests and tools.
#[doc(hidden)]
pub fn head_of(dir: &Path) -> Position {
    tail::head(dir)
}

#[cfg(test)]
mod tests;
