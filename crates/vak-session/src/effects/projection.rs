//! Encrypted, bounded SQLite projection of the Effects record chain.
//!
//! This is a derived lookup index. The append-only chain remains authoritative;
//! any unreadable tail causes the entire catch-up transaction to roll back.

use super::{EffectEvent, EffectKind, EffectRecord, EffectStatus, EffectStep};
use crate::ids::{EffectId, RunId};
use crate::objects::TenantObjects;
use crate::tail::{self, Position};
use crate::types::SessionError;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;

const SCHEMA: &str = "1";

pub struct Projection {
    conn: Connection,
    tenant: Arc<TenantObjects>,
}

impl Projection {
    /// Opens an encrypted projection. Existing databases with an unknown
    /// schema are refused; the source chain can be used to rebuild explicitly.
    pub fn open(path: &Path, tenant_home: &Path) -> Result<Self, SessionError> {
        crate::fence::check()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Serialize connection initialization (including the first WAL mode
        // change) across processes, as the catalog does. SQLite transactions
        // serialize projection catch-up after this short-lived open lock.
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("db.open.lock"))?;
        lock.lock()?;
        let tenant = TenantObjects::for_tenant(tenant_home)?;
        let key = match tenant.scope_key("effects-index") {
            Ok(key) => key,
            Err(SessionError::Erased(scope)) => return Err(SessionError::Erased(scope)),
            Err(_) => tenant.create_scope_key("effects-index")?,
        };
        let mut conn = connect(path, &key.0)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let metadata_exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='metadata')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        let effects_exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='effects')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        let version: Option<String> = if metadata_exists {
            tx.query_row("SELECT value FROM metadata WHERE key = 'schema'", [], |r| {
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
                    "unsupported effects projection schema".into(),
                ));
            }
            None => {
                if metadata_exists || effects_exists {
                    return Err(SessionError::Objects(
                        "effects projection has no supported schema stamp".into(),
                    ));
                }
                crate::fence::check()?;
                tx.execute_batch(
                    "CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     CREATE TABLE IF NOT EXISTS effects (
                       id TEXT PRIMARY KEY, prepared_at TEXT NOT NULL, run TEXT,
                       status TEXT NOT NULL, dispatchable INTEGER NOT NULL,
                       signature TEXT NOT NULL, idempotency_key TEXT NOT NULL,
                       record TEXT NOT NULL
                     );
                     CREATE INDEX IF NOT EXISTS effects_prepared ON effects(prepared_at DESC, id DESC);
                     CREATE INDEX IF NOT EXISTS effects_run_prepared ON effects(run, prepared_at DESC, id DESC);
                     CREATE INDEX IF NOT EXISTS effects_status_prepared ON effects(status, prepared_at DESC, id DESC);
                     CREATE INDEX IF NOT EXISTS effects_run_status_prepared ON effects(run, status, prepared_at DESC, id DESC);
                     CREATE INDEX IF NOT EXISTS effects_dispatchable ON effects(dispatchable, prepared_at ASC, id ASC);
                     CREATE INDEX IF NOT EXISTS effects_signature ON effects(signature);
                     CREATE INDEX IF NOT EXISTS effects_idempotency_key ON effects(idempotency_key);
                     INSERT INTO metadata(key,value) VALUES ('schema','1');
                     INSERT INTO metadata(key,value) VALUES ('position','{\"segment\":0,\"frames\":0}');"
                ).map_err(sql_error)?;
                tx.commit().map_err(sql_error)?;
            }
        }
        Ok(Self { conn, tenant })
    }

    fn check_key(&self) -> Result<(), SessionError> {
        self.tenant.scope_key("effects-index").map(|_| ())
    }

    /// Folds only rows after the persisted source cursor. Rows and cursor are
    /// committed together, so interruption cannot leave a skipped suffix.
    pub fn sync(&mut self, chain: &Path) -> Result<(), SessionError> {
        crate::fence::check()?;
        self.check_key()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let source = canonical_source_path(chain)?;
        let bound: Option<String> = tx
            .query_row("SELECT value FROM metadata WHERE key='source'", [], |r| {
                r.get(0)
            })
            .optional()
            .map_err(sql_error)?;
        if bound
            .as_deref()
            .is_some_and(|previous| previous != source.to_string_lossy())
        {
            return Err(SessionError::Objects(
                "effects projection source does not match its bound chain".into(),
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
            .query_row(
                "SELECT value FROM metadata WHERE key = 'position'",
                [],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        let from: Position = serde_json::from_str(&raw).map_err(json_error)?;
        let to = tail::try_tail(chain, from, |_, bytes| {
            fold_row(&tx, bytes)?;
            Ok(true)
        })?;
        if to != from {
            let value = serde_json::to_string(&to).map_err(json_error)?;
            tx.execute("UPDATE metadata SET value=?1 WHERE key='position'", [value])
                .map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)
    }

    pub fn get(&self, id: EffectId) -> Result<Option<EffectRecord>, SessionError> {
        self.check_key()?;
        let encoded: Option<String> = self
            .conn
            .query_row(
                "SELECT record FROM effects WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let record = encoded
            .map(|s| serde_json::from_str::<EffectRecord>(&s).map_err(json_error))
            .transpose()?;
        if record.as_ref().is_some_and(|record| record.id != id) {
            return Err(SessionError::Corrupt {
                line: 0,
                message: "effects projection id mismatch".into(),
            });
        }
        Ok(record)
    }

    /// Newest-first page. `has_more` indicates another row after this page.
    pub fn list_page(
        &self,
        run: Option<RunId>,
        state: Option<EffectStatus>,
        limit: usize,
        cursor: Option<(DateTime<Utc>, EffectId)>,
    ) -> Result<(Vec<EffectRecord>, bool), SessionError> {
        self.check_key()?;
        let limit = limit.clamp(1, 1000) as i64;
        let cursor_time = cursor.as_ref().map(|(at, _)| sortable_time(*at));
        let cursor_id = cursor.as_ref().map(|(_, id)| id.to_string());
        let run = run.map(|r| r.to_string());
        let state = state.map(status_name);
        let sql = match (run.is_some(), state.is_some(), cursor.is_some()) {
            (false, false, false) => {
                "SELECT record FROM effects ORDER BY prepared_at DESC,id DESC LIMIT ?1"
            }
            (true, false, false) => {
                "SELECT record FROM effects WHERE run=?1 ORDER BY prepared_at DESC,id DESC LIMIT ?2"
            }
            (false, true, false) => {
                "SELECT record FROM effects WHERE status=?1 ORDER BY prepared_at DESC,id DESC LIMIT ?2"
            }
            (true, true, false) => {
                "SELECT record FROM effects WHERE run=?1 AND status=?2 ORDER BY prepared_at DESC,id DESC LIMIT ?3"
            }
            (false, false, true) => {
                "SELECT record FROM effects WHERE (prepared_at,id)<(?1,?2) ORDER BY prepared_at DESC,id DESC LIMIT ?3"
            }
            (true, false, true) => {
                "SELECT record FROM effects WHERE run=?1 AND (prepared_at,id)<(?2,?3) ORDER BY prepared_at DESC,id DESC LIMIT ?4"
            }
            (false, true, true) => {
                "SELECT record FROM effects WHERE status=?1 AND (prepared_at,id)<(?2,?3) ORDER BY prepared_at DESC,id DESC LIMIT ?4"
            }
            (true, true, true) => {
                "SELECT record FROM effects WHERE run=?1 AND status=?2 AND (prepared_at,id)<(?3,?4) ORDER BY prepared_at DESC,id DESC LIMIT ?5"
            }
        };
        let mut statement = self.conn.prepare(sql).map_err(sql_error)?;
        let mut records = match (run, state, cursor_time, cursor_id) {
            (None, None, None, None) => statement
                .query_map([limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            (Some(run), None, None, None) => statement
                .query_map(params![run, limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            (None, Some(state), None, None) => statement
                .query_map(params![state, limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            (Some(run), Some(state), None, None) => statement
                .query_map(params![run, state, limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            (None, None, Some(at), Some(id)) => statement
                .query_map(params![at, id, limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            (Some(run), None, Some(at), Some(id)) => statement
                .query_map(params![run, at, id, limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            (None, Some(state), Some(at), Some(id)) => statement
                .query_map(params![state, at, id, limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            (Some(run), Some(state), Some(at), Some(id)) => statement
                .query_map(params![run, state, at, id, limit + 1], decode_record)
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?,
            _ => return Err(SessionError::Objects("invalid effect cursor".into())),
        };
        let more = records.len() > limit as usize;
        records.truncate(limit as usize);
        Ok((records, more))
    }

    pub fn list(&self) -> Result<Vec<EffectRecord>, SessionError> {
        self.check_key()?;
        let mut statement = self
            .conn
            .prepare("SELECT record FROM effects ORDER BY prepared_at DESC,id DESC")
            .map_err(sql_error)?;
        statement
            .query_map([], decode_record)
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)
    }

    /// Every effect produced by one run, newest first.
    pub fn of_run(&self, run: RunId) -> Result<Vec<EffectRecord>, SessionError> {
        self.check_key()?;
        let mut statement = self
            .conn
            .prepare("SELECT record FROM effects WHERE run=?1 ORDER BY prepared_at DESC,id DESC")
            .map_err(sql_error)?;
        statement
            .query_map([run.to_string()], decode_record)
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)
    }

    /// Effects the background sender may take now, oldest first.
    pub fn dispatchable(&self) -> Result<Vec<EffectRecord>, SessionError> {
        self.check_key()?;
        let mut statement = self
            .conn
            .prepare(
                "SELECT record FROM effects WHERE dispatchable=1 ORDER BY prepared_at ASC,id ASC",
            )
            .map_err(sql_error)?;
        statement
            .query_map([], decode_record)
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)
    }

    /// Counts exact prior matches without decoding unrelated records.
    pub fn matching_count(
        &self,
        run: Option<RunId>,
        kind: &EffectKind,
        target: &str,
        payload_digest: &str,
    ) -> Result<usize, SessionError> {
        self.check_key()?;
        let signature = effect_signature(run, kind, target, payload_digest)?;
        self.conn
            .query_row(
                "SELECT count(*) FROM effects WHERE signature=?1",
                [&signature],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as usize)
            .map_err(sql_error)
    }

    /// Finds an existing once-only effect by idempotency key.
    pub fn find_key(&self, key: &str) -> Result<Option<EffectRecord>, SessionError> {
        self.check_key()?;
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT id,record FROM effects WHERE idempotency_key=?1 ORDER BY prepared_at DESC,id DESC LIMIT 1",
                [key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sql_error)?;
        row.map(|(id, encoded)| {
            let id = id.parse::<EffectId>().map_err(|error| {
                SessionError::Objects(format!("effects projection id: {error}"))
            })?;
            let record: EffectRecord = serde_json::from_str(&encoded).map_err(json_error)?;
            if record.id != id || record.idempotency_key != key {
                return Err(SessionError::Corrupt {
                    line: 0,
                    message: "effects projection key mismatch".into(),
                });
            }
            Ok(record)
        })
        .transpose()
    }
}

fn connect(path: &Path, key: &[u8; 32]) -> Result<Connection, SessionError> {
    let conn = Connection::open(path).map_err(sql_error)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(sql_error)?;
    let hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
    conn.execute_batch(&format!("PRAGMA key = \"x'{hex}'\";"))
        .map_err(|_| SessionError::Objects("effects projection encryption failed".into()))?;
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
    .map_err(|_| SessionError::Objects("effects projection could not be decrypted".into()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(sql_error)?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(sql_error)?;
    Ok(conn)
}

fn fold_row(tx: &Transaction<'_>, bytes: &[u8]) -> Result<(), SessionError> {
    let event: EffectEvent = serde_json::from_slice(bytes).map_err(json_error)?;
    let id = event.effect.to_string();
    let existing: Option<String> = tx
        .query_row("SELECT record FROM effects WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(sql_error)?;
    let existed = existing.is_some();
    let mut record = match existing {
        Some(encoded) => {
            let record = serde_json::from_str::<EffectRecord>(&encoded).map_err(json_error)?;
            if record.id != event.effect {
                return Err(SessionError::Corrupt {
                    line: 0,
                    message: "effects projection id mismatch".into(),
                });
            }
            record
        }
        None => EffectRecord::prepared(
            event.effect,
            event.at,
            event.trace.clone(),
            event.step.clone(),
        )
        .ok_or_else(|| SessionError::Corrupt {
            line: 0,
            message: "effect event precedes preparation".into(),
        })?,
    };
    if !matches!(event.step, EffectStep::Prepared { .. }) || existed {
        record.apply(event.at, event.step);
    }
    let encoded = serde_json::to_string(&record).map_err(json_error)?;
    let dispatchable = record.kind.retries() && record.is_dispatchable();
    let signature = effect_signature(
        record.run,
        &record.kind,
        &record.target,
        &record.payload_digest,
    )?;
    tx.execute("INSERT INTO effects(id,prepared_at,run,status,dispatchable,signature,idempotency_key,record) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(id) DO UPDATE SET status=excluded.status,dispatchable=excluded.dispatchable,record=excluded.record",
        params![record.id.to_string(), sortable_time(record.prepared_at), record.run.map(|r| r.to_string()), status_name(record.status), dispatchable, signature, record.idempotency_key, encoded]).map_err(sql_error)?;
    Ok(())
}

fn decode_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<EffectRecord> {
    let raw: String = row.get(0)?;
    serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn status_name(status: EffectStatus) -> &'static str {
    match status {
        EffectStatus::Queued => "queued",
        EffectStatus::Sending => "sending",
        EffectStatus::Sent => "sent",
        EffectStatus::Retrying => "retrying",
        EffectStatus::Failed => "failed",
        EffectStatus::Unknown => "unknown",
        EffectStatus::Superseded => "superseded",
    }
}
fn sortable_time(at: DateTime<Utc>) -> String {
    // Fixed-width biased seconds preserve chronological ordering even for
    // dates outside the four-digit-year RFC3339 range.
    let seconds = i128::from(at.timestamp()) + i128::from(i64::MAX) + 1;
    format!("{seconds:020}.{:09}", at.timestamp_subsec_nanos())
}

fn effect_signature(
    run: Option<RunId>,
    kind: &EffectKind,
    target: &str,
    payload_digest: &str,
) -> Result<String, SessionError> {
    let bytes = serde_json::to_vec(&(run, kind, target, payload_digest)).map_err(json_error)?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Canonicalizes the nearest existing ancestor and appends the missing
/// suffix. This gives an empty, not-yet-created chain a stable binding while
/// still distinguishing a deleted chain from a previously indexed one.
fn canonical_source_path(path: &Path) -> Result<std::path::PathBuf, SessionError> {
    use std::path::{Component, PathBuf};

    match std::fs::canonicalize(path) {
        Ok(path) => return Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                }
            }
            Component::Normal(name) => normalized.push(name),
        }
    }

    let mut ancestor = normalized.as_path();
    let mut suffix = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(canonical) => {
                let mut result = canonical;
                for component in suffix.iter().rev() {
                    result.push(component);
                }
                return Ok(result);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = ancestor.file_name() else {
                    return Err(error.into());
                };
                suffix.push(name.to_os_string());
                let Some(parent) = ancestor.parent() else {
                    return Err(error.into());
                };
                ancestor = parent;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
fn sql_error(error: rusqlite::Error) -> SessionError {
    SessionError::Objects(format!("effects projection database: {error}"))
}
fn json_error(error: serde_json::Error) -> SessionError {
    SessionError::Objects(format!("effects projection record: {error}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn indexed_time_order_handles_dates_outside_four_digit_years() {
        let dates: Vec<_> = [
            "-0001-01-01T00:00:00Z",
            "0001-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z",
            "+10000-01-01T00:00:00Z",
        ]
        .into_iter()
        .map(|date| date.parse::<DateTime<Utc>>().unwrap())
        .collect();
        let keys: Vec<_> = dates.into_iter().map(sortable_time).collect();
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn first_open_is_serialized_and_source_binding_cannot_be_reused() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let tenant_home = dir.path().join("tenant");
        TenantObjects::for_tenant(&tenant_home)
            .unwrap()
            .create_scope_key("effects-index")
            .unwrap();
        let path = dir.path().join("index/effects.db");
        let source = dir.path().join("effects");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (path, source, tenant_home, barrier) = (
                    path.clone(),
                    source.clone(),
                    tenant_home.clone(),
                    barrier.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut projection = Projection::open(&path, &tenant_home)?;
                    projection.sync(&source)
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        let mut projection = Projection::open(&path, &tenant_home).unwrap();
        assert!(
            projection
                .sync(&dir.path().join("different-chain"))
                .is_err()
        );
    }
}
