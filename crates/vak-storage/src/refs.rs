//! Compare-and-swap refs. A ref moves only with the expected generation and
//! a writer epoch no older than the one it holds, which fences a deposed
//! writer after failover.

use crate::{Result, StorageError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefValue {
    pub generation: u64,
    pub epoch: u64,
    pub target: Vec<u8>,
}

pub trait RefStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<RefValue>>;
    /// `expected` is the generation the caller read (`None` to create).
    fn cas(&self, name: &str, expected: Option<u64>, epoch: u64, target: &[u8])
    -> Result<RefValue>;
    /// Names starting with `prefix`, sorted.
    fn names(&self, prefix: &str) -> Result<Vec<String>>;
}

fn decide(cur: Option<&RefValue>, expected: Option<u64>, epoch: u64) -> Result<u64> {
    if let Some(c) = cur
        && epoch < c.epoch
    {
        return Err(StorageError::StaleEpoch {
            presented: epoch,
            current: c.epoch,
        });
    }
    let cg = cur.map(|c| c.generation);
    if cg != expected {
        return Err(StorageError::Conflict {
            expected,
            current: cg,
        });
    }
    Ok(cg.map_or(1, |g| g + 1))
}

#[derive(Default)]
pub struct MemoryRefStore {
    map: Mutex<HashMap<String, RefValue>>,
}

impl MemoryRefStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, RefValue>>> {
        self.map
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))
    }
}

impl RefStore for MemoryRefStore {
    fn get(&self, name: &str) -> Result<Option<RefValue>> {
        Ok(self.lock()?.get(name).cloned())
    }

    fn names(&self, prefix: &str) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .lock()?
            .keys()
            .filter(|name| name.starts_with(prefix))
            .cloned()
            .collect();
        names.sort();
        Ok(names)
    }

    fn cas(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        target: &[u8],
    ) -> Result<RefValue> {
        let mut m = self.lock()?;
        let generation = decide(m.get(name), expected, epoch)?;
        let v = RefValue {
            generation,
            epoch,
            target: target.to_vec(),
        };
        m.insert(name.into(), v.clone());
        Ok(v)
    }
}

pub struct SqliteRefStore {
    conn: Mutex<Connection>,
}

impl SqliteRefStore {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
             CREATE TABLE IF NOT EXISTS refs(
               name TEXT PRIMARY KEY, generation INTEGER NOT NULL,
               epoch INTEGER NOT NULL, target BLOB NOT NULL);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn read(conn: &Connection, name: &str) -> Result<Option<RefValue>> {
        Ok(conn
            .query_row(
                "SELECT generation, epoch, target FROM refs WHERE name=?1",
                params![name],
                |r| {
                    Ok(RefValue {
                        generation: r.get::<_, i64>(0)? as u64,
                        epoch: r.get::<_, i64>(1)? as u64,
                        target: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }
}

impl RefStore for SqliteRefStore {
    fn get(&self, name: &str) -> Result<Option<RefValue>> {
        let c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        Self::read(&c, name)
    }

    fn names(&self, prefix: &str) -> Result<Vec<String>> {
        let c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let mut statement =
            c.prepare("SELECT name FROM refs WHERE substr(name, 1, ?2) = ?1 ORDER BY name")?;
        let names = statement
            .query_map(params![prefix, prefix.len() as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(names)
    }

    fn cas(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        target: &[u8],
    ) -> Result<RefValue> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cur = Self::read(&tx, name)?;
        let generation = decide(cur.as_ref(), expected, epoch)?;
        tx.execute(
            "INSERT INTO refs(name, generation, epoch, target) VALUES(?1,?2,?3,?4)
             ON CONFLICT(name) DO UPDATE SET generation=?2, epoch=?3, target=?4",
            params![name, generation as i64, epoch as i64, target],
        )?;
        tx.commit()?;
        Ok(RefValue {
            generation,
            epoch,
            target: target.to_vec(),
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn exercise(s: &dyn RefStore) {
        let v = s.cas("head", None, 1, b"a").unwrap();
        assert_eq!(v.generation, 1);
        let v = s.cas("head", Some(1), 2, b"b").unwrap();
        assert_eq!((v.generation, v.epoch), (2, 2));
        assert!(matches!(
            s.cas("head", Some(2), 1, b"c"),
            Err(StorageError::StaleEpoch { .. })
        ));
        assert!(matches!(
            s.cas("head", Some(1), 2, b"c"),
            Err(StorageError::Conflict { .. })
        ));
        assert!(matches!(
            s.cas("head", None, 2, b"c"),
            Err(StorageError::Conflict { .. })
        ));
        assert_eq!(s.get("head").unwrap().unwrap().target, b"b");
        assert!(s.get("none").unwrap().is_none());
    }

    #[test]
    fn stale_epoch_cannot_commit() {
        exercise(&MemoryRefStore::new());
        exercise(&SqliteRefStore::open_in_memory().unwrap());
    }

    #[test]
    fn sqlite_persists() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("refs.db");
        SqliteRefStore::open(&p)
            .unwrap()
            .cas("r", None, 3, b"x")
            .unwrap();
        let v = SqliteRefStore::open(&p).unwrap().get("r").unwrap().unwrap();
        assert_eq!((v.generation, v.epoch), (1, 3));
    }
}
