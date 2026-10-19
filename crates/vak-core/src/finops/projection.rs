//! Encrypted derived index for the authoritative append-only FinOps cost chain.
//!
//! The index stores only aggregate day/session rows and settled reservation
//! metadata. Catch-up and cursor advancement share one immediate transaction;
//! any invalid source row rolls the whole suffix back.

use super::{
    CostRow, DailyCostTotals, DailySpend, SessionSpend, SettledReservation, fold_cost_row,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use vak_session::objects::TenantObjects;
use vak_session::tail::{self, Position};
use vak_session::types::SessionError;

const SCHEMA: &str = "1";

pub(super) struct Projection {
    conn: Connection,
    tenant: Arc<TenantObjects>,
}

impl Projection {
    pub(super) fn open(path: &Path, tenant_home: &Path) -> Result<Self, SessionError> {
        vak_session::fence::check()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("db.open.lock"))?;
        lock.lock()?;
        let tenant = TenantObjects::for_tenant(tenant_home)?;
        let key = match tenant.scope_key("finops-index") {
            Ok(key) => key,
            Err(SessionError::Erased(scope)) => return Err(SessionError::Erased(scope)),
            Err(_) => tenant.create_scope_key("finops-index")?,
        };
        let mut conn = connect(path, &key.0)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let metadata_exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='metadata')",
                [],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        let any_tables: bool = tx
            .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')", [], |r| r.get(0))
            .map_err(sql_error)?;
        let version: Option<String> = if metadata_exists {
            tx.query_row("SELECT value FROM metadata WHERE key='schema'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql_error)?
        } else {
            None
        };
        match version.as_deref() {
            Some(SCHEMA) => tx.commit().map_err(sql_error)?,
            Some(_) => {
                return Err(SessionError::Objects(
                    "unsupported FinOps projection schema".into(),
                ));
            }
            None => {
                if any_tables {
                    return Err(SessionError::Objects(
                        "FinOps projection has no supported schema stamp".into(),
                    ));
                }
                vak_session::fence::check()?;
                tx.execute_batch(
                    "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     CREATE TABLE day (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     CREATE TABLE sessions (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     CREATE TABLE reservations (id TEXT PRIMARY KEY, unknown INTEGER NOT NULL, value TEXT NOT NULL);
                     CREATE INDEX reservations_unknown_id ON reservations(unknown,id);
                     INSERT INTO metadata(key,value) VALUES ('schema','1');
                     INSERT INTO metadata(key,value) VALUES ('position','{\"segment\":0,\"frames\":0}');
                     INSERT INTO metadata(key,value) VALUES ('unmapped_delegated','0');"
                ).map_err(sql_error)?;
                tx.commit().map_err(sql_error)?;
            }
        }
        Ok(Self { conn, tenant })
    }

    pub(super) fn check_key(&self) -> Result<(), SessionError> {
        self.tenant.scope_key("finops-index").map(|_| ())
    }

    pub(super) fn sync(&mut self, chain: &Path) -> Result<Position, SessionError> {
        vak_session::fence::check()?;
        self.check_key()?;
        let source = canonical_source_path(chain)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let bound: Option<String> = tx
            .query_row("SELECT value FROM metadata WHERE key='source'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql_error)?;
        if bound
            .as_deref()
            .is_some_and(|p| p != source.to_string_lossy())
        {
            return Err(SessionError::Objects(
                "FinOps projection source does not match its bound chain".into(),
            ));
        }
        if bound.is_none() {
            tx.execute(
                "INSERT INTO metadata(key,value) VALUES ('source',?1)",
                [source.to_string_lossy().as_ref()],
            )
            .map_err(sql_error)?;
        }
        let raw: String = tx
            .query_row("SELECT value FROM metadata WHERE key='position'", [], |r| {
                r.get(0)
            })
            .map_err(sql_error)?;
        let from: Position = serde_json::from_str(&raw).map_err(json_error)?;
        let to = tail::try_tail(chain, from, |_, bytes| {
            let row: CostRow =
                serde_json::from_slice(bytes).map_err(|e| SessionError::Corrupt {
                    line: 0,
                    message: e.to_string(),
                })?;
            let day = row.ts.date_naive();
            let session = row
                .budget_session_id
                .as_deref()
                .unwrap_or(&row.session_id)
                .to_string();
            let prior = row
                .reservation_id
                .as_deref()
                .map(|id| load_reservation(&tx, id))
                .transpose()?
                .flatten();
            let mut mini = DailyCostTotals::default();
            let mut touched_days = vec![day];
            let mut touched_sessions = vec![session.clone()];
            if let Some(previous) = &prior {
                touched_days.push(previous.day);
                touched_sessions.push(previous.session_id.clone());
            }
            touched_days.sort();
            touched_days.dedup();
            touched_sessions.sort();
            touched_sessions.dedup();
            for key in &touched_days {
                if let Some(value) = load_day(&tx, *key)? {
                    mini.days.insert(*key, value);
                }
            }
            for key in &touched_sessions {
                if let Some(value) = load_session(&tx, key)? {
                    mini.sessions.insert(key.clone(), value);
                }
            }
            if let (Some(id), Some(previous)) = (&row.reservation_id, prior) {
                mini.reservations.insert(id.clone(), previous);
            }
            mini.unmapped_delegated = get_meta_u64(&tx, "unmapped_delegated")?;
            fold_cost_row(&mut mini, bytes)?;
            for key in touched_days {
                save_day(&tx, key, mini.days.get(&key))?;
            }
            for key in touched_sessions {
                save_session(&tx, &key, mini.sessions.get(&key))?;
            }
            if let Some(id) = &row.reservation_id {
                save_reservation(&tx, id, mini.reservations.get(id))?;
            }
            set_meta_u64(&tx, "unmapped_delegated", mini.unmapped_delegated)?;
            Ok(true)
        })?;
        if to != from {
            let value = serde_json::to_string(&to).map_err(json_error)?;
            tx.execute("UPDATE metadata SET value=?1 WHERE key='position'", [value])
                .map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)?;
        Ok(to)
    }

    pub(super) fn select(
        &self,
        day: Option<chrono::NaiveDate>,
        session: Option<&str>,
        ids: &[String],
        include_unknown: bool,
    ) -> Result<(DailyCostTotals, Position), SessionError> {
        self.select_inner(day, session, ids, include_unknown, false)
    }

    fn select_inner(
        &self,
        day: Option<chrono::NaiveDate>,
        session: Option<&str>,
        ids: &[String],
        include_unknown: bool,
        all_reservations: bool,
    ) -> Result<(DailyCostTotals, Position), SessionError> {
        self.check_key()?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let raw: String = tx
            .query_row("SELECT value FROM metadata WHERE key='position'", [], |r| {
                r.get(0)
            })
            .map_err(sql_error)?;
        let position: Position = serde_json::from_str(&raw).map_err(json_error)?;
        let mut totals = DailyCostTotals::default();
        if let Some(day) = day {
            if let Some(value) = load_day(&tx, day)? {
                totals.days.insert(day, value);
            }
        } else if all_reservations {
            let mut stmt = tx.prepare("SELECT key,value FROM day").map_err(sql_error)?;
            for item in stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(sql_error)?
            {
                let (key, value) = item.map_err(sql_error)?;
                let key = chrono::NaiveDate::parse_from_str(&key, "%Y-%m-%d").map_err(|e| {
                    SessionError::Objects(format!("FinOps projection day key: {e}"))
                })?;
                let stored: StoredSpend = serde_json::from_str(&value).map_err(json_error)?;
                totals.days.insert(key, stored.daily()?);
            }
        }
        if let Some(session) = session {
            if let Some(value) = load_session(&tx, session)? {
                totals.sessions.insert(session.to_string(), value);
            }
        } else if all_reservations {
            let mut stmt = tx
                .prepare("SELECT key,value FROM sessions")
                .map_err(sql_error)?;
            for item in stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(sql_error)?
            {
                let (key, value) = item.map_err(sql_error)?;
                let stored: StoredSpend = serde_json::from_str(&value).map_err(json_error)?;
                totals.sessions.insert(key, stored.session()?);
            }
        }
        let mut wanted = ids.to_vec();
        if include_unknown || all_reservations {
            let sql = if all_reservations {
                "SELECT id FROM reservations"
            } else {
                "SELECT id FROM reservations WHERE unknown=1"
            };
            let mut stmt = tx.prepare(sql).map_err(sql_error)?;
            wanted.extend(
                stmt.query_map([], |r| r.get::<_, String>(0))
                    .map_err(sql_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(sql_error)?,
            );
        }
        wanted.sort();
        wanted.dedup();
        for id in wanted {
            if let Some(value) = load_reservation(&tx, &id)? {
                totals.reservations.insert(id, value);
            }
        }
        totals.unmapped_delegated = get_meta_u64(&tx, "unmapped_delegated")?;
        tx.commit().map_err(sql_error)?;
        Ok((totals, position))
    }

    #[cfg(test)]
    pub(super) fn all_totals(&self) -> Result<(DailyCostTotals, Position), SessionError> {
        self.select_inner(None, None, &[], false, true)
    }
}

#[derive(Serialize, Deserialize)]
struct StoredSpend {
    usd_bits: u64,
    unknown_calls: u64,
}

impl StoredSpend {
    fn from_usd(usd: f64, unknown_calls: u64) -> Self {
        Self {
            usd_bits: usd.to_bits(),
            unknown_calls,
        }
    }

    fn usd(&self) -> Result<f64, SessionError> {
        let usd = f64::from_bits(self.usd_bits);
        if !usd.is_finite() || usd < 0.0 {
            return Err(SessionError::Objects(
                "FinOps projection spend is invalid".into(),
            ));
        }
        Ok(usd)
    }

    fn daily(&self) -> Result<DailySpend, SessionError> {
        Ok(DailySpend {
            usd: self.usd()?,
            unknown_calls: self.unknown_calls,
        })
    }

    fn session(&self) -> Result<SessionSpend, SessionError> {
        Ok(SessionSpend {
            usd: self.usd()?,
            unknown_calls: self.unknown_calls,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct StoredReservation {
    record: SettledReservation,
    usd_bits: Option<u64>,
}

fn load_day(conn: &Connection, key: chrono::NaiveDate) -> Result<Option<DailySpend>, SessionError> {
    load_json::<StoredSpend>(conn, "SELECT value FROM day WHERE key=?1", key.to_string())?
        .map(|stored| stored.daily())
        .transpose()
}
fn load_session(conn: &Connection, key: &str) -> Result<Option<SessionSpend>, SessionError> {
    load_json::<StoredSpend>(conn, "SELECT value FROM sessions WHERE key=?1", key)?
        .map(|stored| stored.session())
        .transpose()
}
fn load_reservation(
    conn: &Connection,
    key: &str,
) -> Result<Option<SettledReservation>, SessionError> {
    let stored =
        load_json::<StoredReservation>(conn, "SELECT value FROM reservations WHERE id=?1", key)?;
    stored
        .map(|stored| {
            let mut record = stored.record;
            record.usd = stored.usd_bits.map(f64::from_bits);
            if record.usd.is_some_and(|usd| !usd.is_finite() || usd < 0.0) {
                return Err(SessionError::Objects(
                    "FinOps projection reservation USD is invalid".into(),
                ));
            }
            Ok(record)
        })
        .transpose()
}
fn load_json<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    sql: &str,
    key: impl rusqlite::ToSql,
) -> Result<Option<T>, SessionError> {
    let raw: Option<String> = conn
        .query_row(sql, [key], |r| r.get(0))
        .optional()
        .map_err(sql_error)?;
    raw.map(|v| serde_json::from_str(&v).map_err(json_error))
        .transpose()
}
fn save_day(
    tx: &rusqlite::Transaction<'_>,
    key: chrono::NaiveDate,
    value: Option<&DailySpend>,
) -> Result<(), SessionError> {
    let stored = value.map(|value| StoredSpend::from_usd(value.usd, value.unknown_calls));
    save_json(
        tx,
        "INSERT INTO day(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        key.to_string(),
        stored.as_ref(),
    )
}
fn save_session(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    value: Option<&SessionSpend>,
) -> Result<(), SessionError> {
    let stored = value.map(|value| StoredSpend::from_usd(value.usd, value.unknown_calls));
    save_json(
        tx,
        "INSERT INTO sessions(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        key,
        stored.as_ref(),
    )
}
fn save_reservation(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    value: Option<&SettledReservation>,
) -> Result<(), SessionError> {
    if let Some(value) = value {
        let stored = StoredReservation {
            record: SettledReservation {
                usd: None,
                ..value.clone()
            },
            usd_bits: value.usd.map(f64::to_bits),
        };
        let raw = serde_json::to_string(&stored).map_err(json_ser_error)?;
        tx.execute("INSERT INTO reservations(id,unknown,value) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET unknown=excluded.unknown,value=excluded.value", params![key, value.usd.is_none(), raw]).map_err(sql_error)?;
    }
    Ok(())
}
fn save_json<T: Serialize>(
    tx: &rusqlite::Transaction<'_>,
    sql: &str,
    key: impl ToString,
    value: Option<&T>,
) -> Result<(), SessionError> {
    if let Some(value) = value {
        tx.execute(
            sql,
            params![
                key.to_string(),
                serde_json::to_string(value).map_err(json_ser_error)?
            ],
        )
        .map_err(sql_error)?;
    }
    Ok(())
}
fn get_meta_u64(conn: &Connection, key: &str) -> Result<u64, SessionError> {
    let value: String = conn
        .query_row("SELECT value FROM metadata WHERE key=?1", [key], |r| {
            r.get(0)
        })
        .map_err(sql_error)?;
    value
        .parse()
        .map_err(|e| SessionError::Objects(format!("FinOps projection metadata: {e}")))
}
fn set_meta_u64(tx: &rusqlite::Transaction<'_>, key: &str, value: u64) -> Result<(), SessionError> {
    tx.execute(
        "UPDATE metadata SET value=?1 WHERE key=?2",
        params![value.to_string(), key],
    )
    .map_err(sql_error)?;
    Ok(())
}

fn connect(path: &Path, key: &[u8; 32]) -> Result<Connection, SessionError> {
    let conn = Connection::open(path).map_err(sql_error)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(sql_error)?;
    let hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
    conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
        .map_err(|_| SessionError::Objects("FinOps projection encryption failed".into()))?;
    let version: Result<String, _> = conn.pragma_query_value(None, "cipher_version", |r| r.get(0));
    if !version.is_ok_and(|v| !v.trim().is_empty()) {
        return Err(SessionError::Objects("SQLCipher is unavailable".into()));
    }
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(sql_error)?;
    conn.pragma_update(None, "cipher_memory_security", "ON")
        .map_err(sql_error)?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|_| SessionError::Objects("FinOps projection could not be decrypted".into()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(sql_error)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(sql_error)?;
    Ok(conn)
}
fn canonical_source_path(path: &Path) -> Result<PathBuf, SessionError> {
    match std::fs::canonicalize(path) {
        Ok(path) => return Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for c in absolute.components() {
        match c {
            Component::Prefix(p) => normalized.push(p.as_os_str()),
            Component::RootDir => normalized.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(n) => normalized.push(n),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut root) => {
                for part in suffix.iter().rev() {
                    root.push(part);
                }
                return Ok(root);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = ancestor.file_name() else {
                    return Err(e.into());
                };
                suffix.push(name.to_os_string());
                let Some(parent) = ancestor.parent() else {
                    return Err(e.into());
                };
                ancestor = parent;
            }
            Err(e) => return Err(e.into()),
        }
    }
}
fn sql_error(error: rusqlite::Error) -> SessionError {
    SessionError::Objects(format!("FinOps projection database: {error}"))
}
fn json_error(error: serde_json::Error) -> SessionError {
    SessionError::Objects(format!("FinOps projection record: {error}"))
}
fn json_ser_error(error: serde_json::Error) -> SessionError {
    SessionError::Objects(format!("FinOps projection record: {error}"))
}
