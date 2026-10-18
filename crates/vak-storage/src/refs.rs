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
    /// Removes the ref, under the same generation and epoch rules as `cas`.
    /// A name that is created again later starts over at generation 1, so
    /// remove only refs whose name is never reused.
    fn remove(&self, name: &str, expected: u64, epoch: u64) -> Result<()>;
    /// Advances a numeric counter ref atomically across every handle.
    fn bump_counter(&self, name: &str) -> Result<u64>;
    /// CAS guarded by the store-wide epoch ref, checked in the same atomic
    /// operation as the user ref mutation.
    fn cas_guarded(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        epoch_name: &str,
        commit_name: &str,
        target: &[u8],
    ) -> Result<(RefValue, u64)>;
    /// Remove guarded by the store-wide epoch ref, checked atomically.
    fn remove_guarded(
        &self,
        name: &str,
        expected: u64,
        epoch: u64,
        epoch_name: &str,
        commit_name: &str,
    ) -> Result<u64>;
    /// Advances the writer epoch and commit generation atomically.
    fn advance_epoch(&self, epoch_name: &str, commit_name: &str) -> Result<(u64, u64)>;
    /// Names starting with `prefix`, sorted.
    fn names(&self, prefix: &str) -> Result<Vec<String>>;
    /// Makes the store hold exactly `rows`, generations and epochs as
    /// given: what a pull from a remote does (plan M9). Not a CAS; the
    /// caller is the only writer.
    fn replace_all(&self, rows: &[(String, RefValue)]) -> Result<()>;
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

fn check_epoch(value: Option<&RefValue>, presented: u64) -> Result<()> {
    let current = counter_value(value)?;
    if presented < current {
        return Err(StorageError::StaleEpoch { presented, current });
    }
    Ok(())
}

fn counter_value(value: Option<&RefValue>) -> Result<u64> {
    match value {
        None => Ok(0),
        Some(value) => {
            let bytes: [u8; 8] = value
                .target
                .as_slice()
                .try_into()
                .map_err(|_| StorageError::Malformed("store counter"))?;
            Ok(u64::from_le_bytes(bytes))
        }
    }
}

fn next_counter(value: Option<&RefValue>) -> Result<(u64, u64)> {
    let next = counter_value(value)?
        .checked_add(1)
        .ok_or(StorageError::Malformed("store counter overflow"))?;
    let generation = value.map_or(Ok(1), |value| {
        value
            .generation
            .checked_add(1)
            .ok_or(StorageError::Malformed("store counter generation overflow"))
    })?;
    Ok((next, generation))
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

    fn replace_all(&self, rows: &[(String, RefValue)]) -> Result<()> {
        *self.lock()? = rows.iter().cloned().collect();
        Ok(())
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

    fn remove(&self, name: &str, expected: u64, epoch: u64) -> Result<()> {
        let mut m = self.lock()?;
        decide(m.get(name), Some(expected), epoch)?;
        m.remove(name);
        Ok(())
    }

    fn bump_counter(&self, name: &str) -> Result<u64> {
        let mut m = self.lock()?;
        let (next, generation) = next_counter(m.get(name))?;
        m.insert(
            name.into(),
            RefValue {
                generation,
                epoch: 0,
                target: next.to_le_bytes().to_vec(),
            },
        );
        Ok(next)
    }

    fn cas_guarded(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        epoch_name: &str,
        commit_name: &str,
        target: &[u8],
    ) -> Result<(RefValue, u64)> {
        let mut m = self.lock()?;
        check_epoch(m.get(epoch_name), epoch)?;
        let generation = decide(m.get(name), expected, epoch)?;
        let (commit, commit_generation) = next_counter(m.get(commit_name))?;
        let value = RefValue {
            generation,
            epoch,
            target: target.to_vec(),
        };
        m.insert(name.into(), value.clone());
        m.insert(
            commit_name.into(),
            RefValue {
                generation: commit_generation,
                epoch: 0,
                target: commit.to_le_bytes().to_vec(),
            },
        );
        Ok((value, commit))
    }

    fn remove_guarded(
        &self,
        name: &str,
        expected: u64,
        epoch: u64,
        epoch_name: &str,
        commit_name: &str,
    ) -> Result<u64> {
        let mut m = self.lock()?;
        check_epoch(m.get(epoch_name), epoch)?;
        decide(m.get(name), Some(expected), epoch)?;
        let (commit, generation) = next_counter(m.get(commit_name))?;
        m.remove(name);
        m.insert(
            commit_name.into(),
            RefValue {
                generation,
                epoch: 0,
                target: commit.to_le_bytes().to_vec(),
            },
        );
        Ok(commit)
    }

    fn advance_epoch(&self, epoch_name: &str, commit_name: &str) -> Result<(u64, u64)> {
        let mut m = self.lock()?;
        let (epoch, epoch_generation) = next_counter(m.get(epoch_name))?;
        let (commit, commit_generation) = next_counter(m.get(commit_name))?;
        m.insert(
            epoch_name.into(),
            RefValue {
                generation: epoch_generation,
                epoch: 0,
                target: epoch.to_le_bytes().to_vec(),
            },
        );
        m.insert(
            commit_name.into(),
            RefValue {
                generation: commit_generation,
                epoch: 0,
                target: commit.to_le_bytes().to_vec(),
            },
        );
        Ok((epoch, commit))
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

    fn replace_all(&self, rows: &[(String, RefValue)]) -> Result<()> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let tx = c.transaction()?;
        tx.execute("DELETE FROM refs", [])?;
        for (name, value) in rows {
            tx.execute(
                "INSERT INTO refs(name, generation, epoch, target) VALUES (?1, ?2, ?3, ?4)",
                params![
                    name,
                    value.generation as i64,
                    value.epoch as i64,
                    value.target
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
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

    fn remove(&self, name: &str, expected: u64, epoch: u64) -> Result<()> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let cur = Self::read(&tx, name)?;
        decide(cur.as_ref(), Some(expected), epoch)?;
        tx.execute("DELETE FROM refs WHERE name=?1", params![name])?;
        tx.commit()?;
        Ok(())
    }

    fn bump_counter(&self, name: &str) -> Result<u64> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let next = bump_counter_tx(&tx, name)?;
        tx.commit()?;
        Ok(next)
    }

    fn cas_guarded(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        epoch_name: &str,
        commit_name: &str,
        target: &[u8],
    ) -> Result<(RefValue, u64)> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_epoch(Self::read(&tx, epoch_name)?.as_ref(), epoch)?;
        let cur = Self::read(&tx, name)?;
        let generation = decide(cur.as_ref(), expected, epoch)?;
        tx.execute(
            "INSERT INTO refs(name, generation, epoch, target) VALUES(?1,?2,?3,?4)
             ON CONFLICT(name) DO UPDATE SET generation=?2, epoch=?3, target=?4",
            params![name, generation as i64, epoch as i64, target],
        )?;
        let commit = bump_counter_tx(&tx, commit_name)?;
        tx.commit()?;
        Ok((
            RefValue {
                generation,
                epoch,
                target: target.to_vec(),
            },
            commit,
        ))
    }

    fn remove_guarded(
        &self,
        name: &str,
        expected: u64,
        epoch: u64,
        epoch_name: &str,
        commit_name: &str,
    ) -> Result<u64> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_epoch(Self::read(&tx, epoch_name)?.as_ref(), epoch)?;
        let cur = Self::read(&tx, name)?;
        decide(cur.as_ref(), Some(expected), epoch)?;
        tx.execute("DELETE FROM refs WHERE name=?1", params![name])?;
        let commit = bump_counter_tx(&tx, commit_name)?;
        tx.commit()?;
        Ok(commit)
    }

    fn advance_epoch(&self, epoch_name: &str, commit_name: &str) -> Result<(u64, u64)> {
        let mut c = self
            .conn
            .lock()
            .map_err(|_| StorageError::Malformed("ref state poisoned"))?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let epoch = bump_counter_tx(&tx, epoch_name)?;
        let commit = bump_counter_tx(&tx, commit_name)?;
        tx.commit()?;
        Ok((epoch, commit))
    }
}

fn bump_counter_tx(tx: &rusqlite::Transaction<'_>, name: &str) -> Result<u64> {
    let current = SqliteRefStore::read(tx, name)?;
    let (next, generation) = next_counter(current.as_ref())?;
    tx.execute(
        "INSERT INTO refs(name, generation, epoch, target) VALUES(?1,?2,0,?3)
         ON CONFLICT(name) DO UPDATE SET generation=?2, epoch=0, target=?3",
        params![name, generation as i64, next.to_le_bytes().as_slice()],
    )?;
    Ok(next)
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
        assert!(matches!(
            s.remove("head", 2, 1),
            Err(StorageError::StaleEpoch { .. })
        ));
        assert!(matches!(
            s.remove("head", 1, 2),
            Err(StorageError::Conflict { .. })
        ));
        s.remove("head", 2, 2).unwrap();
        assert!(s.get("head").unwrap().is_none());
        assert!(s.names("head").unwrap().is_empty());
        assert!(matches!(
            s.remove("head", 2, 2),
            Err(StorageError::Conflict { current: None, .. })
        ));
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
