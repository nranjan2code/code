//! Content-addressed objects. The id is HMAC-SHA256 of the plaintext under a
//! tenant id key (dedupe inside a tenant, no cross-tenant confirmation). The
//! body is zstd-compressed, then sealed under a per-object random key. Each
//! referencing scope holds a grant: the object key wrapped for that scope.
//! An object with no surviving grant is unreadable and collectable.

use crate::keys::{KeyAuthority, WrappedKey};
use crate::seal::{self, KEY_LEN};
use crate::{Result, StorageError};
use ring::hmac;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId(pub String);

pub struct IdKey(hmac::Key);

impl IdKey {
    pub fn new(bytes: &[u8; KEY_LEN]) -> Self {
        Self(hmac::Key::new(hmac::HMAC_SHA256, bytes))
    }

    pub fn id(&self, plaintext: &[u8]) -> ObjectId {
        ObjectId(seal::hex(hmac::sign(&self.0, plaintext).as_ref()))
    }
}

pub struct LocalObjectStore {
    root: PathBuf,
    id_key: IdKey,
    authority: Arc<dyn KeyAuthority>,
    seq: AtomicU64,
}

fn valid_id(id: &ObjectId) -> Result<()> {
    if id.0.len() == 64 && id.0.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(StorageError::Malformed("object id"))
    }
}

impl LocalObjectStore {
    pub fn open(root: &Path, id_key: IdKey, authority: Arc<dyn KeyAuthority>) -> Result<Self> {
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("grants"))?;
        fs::create_dir_all(root.join("tmp"))?;
        Ok(Self {
            root: root.to_path_buf(),
            id_key,
            authority,
            seq: AtomicU64::new(0),
        })
    }

    fn object_path(&self, id: &ObjectId) -> PathBuf {
        self.root.join("objects").join(&id.0[..2]).join(&id.0[2..])
    }

    fn grant_dir(&self, id: &ObjectId) -> PathBuf {
        self.root.join("grants").join(&id.0)
    }

    fn grant_path(&self, id: &ObjectId, scope: &str) -> PathBuf {
        self.grant_dir(id).join(seal::hex(scope.as_bytes()))
    }

    /// Write-then-rename, so a reader never sees a partial file.
    fn write_atomic(&self, dest: &Path, bytes: &[u8]) -> Result<()> {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .root
            .join("tmp")
            .join(format!("{}-{n}", std::process::id()));
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&tmp, dest)?;
        if let Some(parent) = dest.parent()
            && let Ok(d) = fs::File::open(parent)
        {
            let _ = d.sync_all();
        }
        Ok(())
    }

    fn seal_body(&self, id: &ObjectId, key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        seal::seal(key, id.0.as_bytes(), &seal::compress(plaintext)?)
    }

    fn write_grant(&self, id: &ObjectId, scope: &str, key: &[u8]) -> Result<()> {
        let w = self.authority.wrap(scope, key)?;
        self.write_atomic(&self.grant_path(id, scope), &w.encode())
    }

    fn read_grant_key(&self, path: &Path) -> Result<Vec<u8>> {
        let w = WrappedKey::decode(&fs::read(path)?)?;
        self.authority.unwrap(&w)
    }

    /// Any key that still unwraps from an existing grant of this object.
    fn recover_key(&self, id: &ObjectId) -> Result<Option<Vec<u8>>> {
        let Ok(rd) = fs::read_dir(self.grant_dir(id)) else {
            return Ok(None);
        };
        for e in rd {
            match self.read_grant_key(&e?.path()) {
                Ok(k) => return Ok(Some(k)),
                Err(StorageError::AuthorityUnavailable(m)) => {
                    return Err(StorageError::AuthorityUnavailable(m));
                }
                Err(_) => continue,
            }
        }
        Ok(None)
    }

    /// Stores `plaintext` (once per tenant) and grants `scope` access.
    pub fn put(&self, plaintext: &[u8], scope: &str) -> Result<ObjectId> {
        self.authority.health()?;
        let id = self.id_key.id(plaintext);
        let path = self.object_path(&id);
        if path.exists() {
            if self.grant_path(&id, scope).exists() {
                return Ok(id);
            }
            if let Some(key) = self.recover_key(&id)? {
                self.write_grant(&id, scope, &key)?;
                return Ok(id);
            }
        }
        let key: [u8; KEY_LEN] = seal::random()?;
        let body = self.seal_body(&id, &key, plaintext)?;
        self.write_grant(&id, scope, &key)?;
        self.write_atomic(&path, &body)?;
        Ok(id)
    }

    pub fn get(&self, id: &ObjectId, scope: &str) -> Result<Vec<u8>> {
        valid_id(id)?;
        let gp = self.grant_path(id, scope);
        if !gp.exists() {
            return Err(StorageError::NoGrant);
        }
        let key = self.read_grant_key(&gp)?;
        let body = fs::read(self.object_path(id)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NotFound
            } else {
                e.into()
            }
        })?;
        let plain = seal::decompress(&seal::open(&key, id.0.as_bytes(), &body)?)?;
        if self.id_key.id(&plain) != *id {
            return Err(StorageError::Integrity);
        }
        Ok(plain)
    }

    /// Gives `to` access to an object `from` can already read.
    pub fn grant(&self, id: &ObjectId, from: &str, to: &str) -> Result<()> {
        valid_id(id)?;
        let gp = self.grant_path(id, from);
        if !gp.exists() {
            return Err(StorageError::NoGrant);
        }
        let key = self.read_grant_key(&gp)?;
        self.write_grant(id, to, &key)
    }

    pub fn remove_grant(&self, id: &ObjectId, scope: &str) -> Result<()> {
        valid_id(id)?;
        match fs::remove_file(self.grant_path(id, scope)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Removes every object with no live grant. Returns how many.
    pub fn gc(&self) -> Result<usize> {
        self.gc_holding(&|_| false)
    }

    /// As `gc`, but a grant held by a scope under hold is always live.
    pub fn gc_holding(&self, held: &dyn Fn(&str) -> bool) -> Result<usize> {
        let mut removed = 0;
        for shard in fs::read_dir(self.root.join("objects"))? {
            let shard = shard?;
            let prefix = shard.file_name().to_string_lossy().into_owned();
            for obj in fs::read_dir(shard.path())? {
                let obj = obj?;
                let id = ObjectId(format!("{prefix}{}", obj.file_name().to_string_lossy()));
                if valid_id(&id).is_err() {
                    continue;
                }
                if !self.has_live_grant(&id, held)? {
                    fs::remove_file(obj.path())?;
                    let _ = fs::remove_dir_all(self.grant_dir(&id));
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    /// A grant is live while its scope is held or its key still unwraps. An
    /// unreachable authority is an error, never a reason to collect.
    fn has_live_grant(&self, id: &ObjectId, held: &dyn Fn(&str) -> bool) -> Result<bool> {
        let Ok(rd) = fs::read_dir(self.grant_dir(id)) else {
            return Ok(false);
        };
        for e in rd {
            let Ok(bytes) = fs::read(e?.path()) else {
                return Ok(true);
            };
            let Ok(w) = WrappedKey::decode(&bytes) else {
                return Ok(true);
            };
            if held(&w.scope) {
                return Ok(true);
            }
            match self.authority.unwrap(&w) {
                Err(StorageError::Revoked(_)) => {}
                Err(StorageError::AuthorityUnavailable(m)) => {
                    return Err(StorageError::AuthorityUnavailable(m));
                }
                _ => return Ok(true),
            }
        }
        Ok(false)
    }

    pub fn exists(&self, id: &ObjectId) -> bool {
        valid_id(id).is_ok() && self.object_path(id).exists()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::keys::MemoryKeyAuthority;

    fn store(dir: &Path, id_key: [u8; 32], a: &Arc<MemoryKeyAuthority>) -> LocalObjectStore {
        LocalObjectStore::open(dir, IdKey::new(&id_key), a.clone()).unwrap()
    }

    #[test]
    fn keyed_ids_dedupe_within_a_tenant_and_differ_across_tenants() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let t1 = store(d1.path(), [1; 32], &a);
        let t2 = store(d2.path(), [2; 32], &a);
        let x = t1.put(b"same file", "conv-a").unwrap();
        let y = t1.put(b"same file", "conv-b").unwrap();
        assert_eq!(x, y);
        assert_eq!(t1.gc().unwrap(), 0);
        let objects = fs::read_dir(d1.path().join("objects")).unwrap().count();
        assert_eq!(objects, 1);
        assert_ne!(x, t2.put(b"same file", "conv-a").unwrap());
        assert_eq!(t1.get(&y, "conv-b").unwrap(), b"same file");
    }

    #[test]
    fn grants_gate_reads_and_gc_respects_them() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), [7; 32], &a);
        let id = s.put(b"payload", "a").unwrap();
        assert!(matches!(s.get(&id, "b"), Err(StorageError::NoGrant)));
        s.grant(&id, "a", "b").unwrap();
        a.revoke("a").unwrap();
        assert!(s.get(&id, "a").is_err());
        assert_eq!(s.get(&id, "b").unwrap(), b"payload");
        assert_eq!(s.gc().unwrap(), 0);
        s.remove_grant(&id, "a").unwrap();
        assert_eq!(s.gc().unwrap(), 0);
        s.remove_grant(&id, "b").unwrap();
        assert_eq!(s.gc().unwrap(), 1);
        assert!(!s.exists(&id));
    }

    #[test]
    fn dedupe_after_all_grants_revoked_rewrites_object() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), [7; 32], &a);
        let id = s.put(b"p", "a").unwrap();
        a.revoke("a").unwrap();
        let id2 = s.put(b"p", "c").unwrap();
        assert_eq!(id, id2);
        assert_eq!(s.get(&id, "c").unwrap(), b"p");
    }

    #[test]
    fn tamper_and_unhealthy_authority_fail() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let d = tempfile::tempdir().unwrap();
        let s = store(d.path(), [7; 32], &a);
        let id = s.put(b"secret", "a").unwrap();
        a.set_healthy(false).unwrap();
        assert!(s.get(&id, "a").is_err());
        assert!(s.put(b"other", "a").is_err());
        a.set_healthy(true).unwrap();
        let p = s.object_path(&id);
        let mut b = fs::read(&p).unwrap();
        let n = b.len() - 1;
        b[n] ^= 1;
        fs::write(&p, b).unwrap();
        assert!(s.get(&id, "a").is_err());
    }
}
