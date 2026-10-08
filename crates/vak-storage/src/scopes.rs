//! Persistent scope keys. Each scope (a conversation, contributor, space or
//! artifact) has one random key kept on disk only wrapped by the key
//! authority. `grant` wraps the same key for a second scope, `revoke` removes
//! one scope's wrapped copy, `shred` destroys the scope for good (a tombstone
//! is written, then the authority forgets the scope), and `hold` blocks
//! shredding and garbage collection until released. Shredding never touches
//! the ciphertext it orphans: the record chains verify unchanged.

use crate::keys::{KeyAuthority, WrappedKey};
use crate::records::ScopeKey;
use crate::seal::{self, KEY_LEN};
use crate::{Result, StorageError};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ScopeKeys {
    root: PathBuf,
    authority: Arc<dyn KeyAuthority>,
    seq: AtomicU64,
}

impl ScopeKeys {
    pub fn open(root: &Path, authority: Arc<dyn KeyAuthority>) -> Result<Self> {
        for d in ["keys", "tombstones", "holds", "tmp"] {
            fs::create_dir_all(root.join(d))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            authority,
            seq: AtomicU64::new(0),
        })
    }

    fn path(&self, kind: &str, scope: &str) -> PathBuf {
        self.root.join(kind).join(seal::hex(scope.as_bytes()))
    }

    fn write_atomic(&self, dest: &Path, bytes: &[u8]) -> Result<()> {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .root
            .join("tmp")
            .join(format!("{}-{n}", std::process::id()));
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, dest)?;
        if let Some(Ok(d)) = dest.parent().map(fs::File::open) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    pub fn is_shredded(&self, scope: &str) -> bool {
        self.path("tombstones", scope).exists()
    }

    pub fn is_held(&self, scope: &str) -> bool {
        self.path("holds", scope).exists()
    }

    pub fn held_scopes(&self) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        for e in fs::read_dir(self.root.join("holds"))? {
            let name = e?.file_name().to_string_lossy().into_owned();
            if let Some(s) = unhex(&name) {
                out.insert(s);
            }
        }
        Ok(out)
    }

    /// Creates the scope's key, or returns the existing one. A shredded scope
    /// can never be created again.
    pub fn create(&self, scope: &str) -> Result<ScopeKey> {
        if self.is_shredded(scope) {
            return Err(StorageError::Revoked(scope.into()));
        }
        if self.path("keys", scope).exists() {
            return self.get(scope);
        }
        let key: [u8; KEY_LEN] = seal::random()?;
        let w = self.authority.wrap(scope, &key)?;
        if self.write_new(&self.path("keys", scope), &w.encode())? {
            Ok(ScopeKey(key))
        } else {
            self.get(scope)
        }
    }

    /// Writes `dest` only if nothing is there; `false` when another creator
    /// got there first, whose bytes stand.
    fn write_new(&self, dest: &Path, bytes: &[u8]) -> Result<bool> {
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        let tmp = self
            .root
            .join("tmp")
            .join(format!("{}-{n}", std::process::id()));
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        let linked = fs::hard_link(&tmp, dest);
        let _ = fs::remove_file(&tmp);
        match linked {
            Ok(()) => {
                if let Some(Ok(d)) = dest.parent().map(fs::File::open) {
                    let _ = d.sync_all();
                }
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get(&self, scope: &str) -> Result<ScopeKey> {
        if self.is_shredded(scope) {
            return Err(StorageError::Revoked(scope.into()));
        }
        let bytes = fs::read(self.path("keys", scope)).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StorageError::NoGrant
            } else {
                e.into()
            }
        })?;
        let w = WrappedKey::decode(&bytes)?;
        if w.scope != scope {
            return Err(StorageError::Integrity);
        }
        let raw = self.authority.unwrap(&w)?;
        let key: [u8; KEY_LEN] = raw
            .try_into()
            .map_err(|_| StorageError::Crypto("bad scope key length"))?;
        Ok(ScopeKey(key))
    }

    /// Lets `to` read what `from`'s key protects, by wrapping the same key
    /// for `to`.
    pub fn grant(&self, from: &str, to: &str) -> Result<()> {
        if self.is_shredded(to) {
            return Err(StorageError::Revoked(to.into()));
        }
        let key = self.get(from)?;
        let w = self.authority.wrap(to, &key.0)?;
        self.write_atomic(&self.path("keys", to), &w.encode())
    }

    /// Removes one scope's wrapped copy. Other scopes' copies are untouched.
    pub fn revoke(&self, scope: &str) -> Result<()> {
        match fs::remove_file(self.path("keys", scope)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Destroys the scope. Refused while it is held.
    pub fn shred(&self, scope: &str) -> Result<()> {
        if self.is_held(scope) {
            return Err(StorageError::Held(scope.into()));
        }
        self.write_atomic(&self.path("tombstones", scope), b"shredded")?;
        self.authority.revoke(scope)?;
        self.revoke(scope)
    }

    pub fn hold(&self, scope: &str) -> Result<()> {
        if self.is_shredded(scope) {
            return Err(StorageError::Revoked(scope.into()));
        }
        self.write_atomic(&self.path("holds", scope), b"held")
    }

    pub fn release(&self, scope: &str) -> Result<()> {
        match fs::remove_file(self.path("holds", scope)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn unhex(h: &str) -> Option<String> {
    if !h.len().is_multiple_of(2) || !h.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..h.len() / 2)
        .map(|i| u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::keys::MemoryKeyAuthority;
    use crate::objects::{IdKey, LocalObjectStore};
    use crate::records::{RecordWriter, read_entries, verify_chain};

    fn keys(dir: &Path, a: &Arc<MemoryKeyAuthority>) -> ScopeKeys {
        ScopeKeys::open(dir, a.clone()).unwrap()
    }

    #[test]
    fn keys_persist_across_reopen_and_are_only_wrapped_on_disk() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let d = tempfile::tempdir().unwrap();
        let k = keys(d.path(), &a).create("conv").unwrap();
        let again = keys(d.path(), &a).get("conv").unwrap();
        assert_eq!(k.0, again.0);
        let on_disk = fs::read(d.path().join("keys").join(seal::hex(b"conv"))).unwrap();
        assert!(!on_disk.windows(KEY_LEN).any(|w| w == k.0));
    }

    #[test]
    fn grant_shares_a_key_and_revoke_removes_one_copy() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let d = tempfile::tempdir().unwrap();
        let s = keys(d.path(), &a);
        let k = s.create("a").unwrap();
        s.grant("a", "b").unwrap();
        assert_eq!(s.get("b").unwrap().0, k.0);
        s.revoke("a").unwrap();
        assert!(matches!(s.get("a"), Err(StorageError::NoGrant)));
        assert_eq!(s.get("b").unwrap().0, k.0);
    }

    #[test]
    fn shred_leaves_bytes_unchanged_and_content_unreadable() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let d = tempfile::tempdir().unwrap();
        let s = keys(d.path(), &a);
        let k = s.create("conv").unwrap();
        let log = d.path().join("conv.log");
        let mut w = RecordWriter::open(&log).unwrap();
        w.append(b"private words", Some(&k)).unwrap();
        w.append(b"more", Some(&k)).unwrap();
        let before = fs::read(&log).unwrap();
        assert_eq!(read_entries(&log, Some(&k)).unwrap().len(), 2);

        s.shred("conv").unwrap();
        assert_eq!(fs::read(&log).unwrap(), before);
        assert_eq!(verify_chain(&log).unwrap().entries, 2);
        assert!(matches!(s.get("conv"), Err(StorageError::Revoked(_))));
        assert!(s.create("conv").is_err());
        assert!(s.is_shredded("conv"));
        let reopened = keys(d.path(), &a);
        assert!(reopened.get("conv").is_err());
        assert!(matches!(
            read_entries(&log, None),
            Err(StorageError::Undecryptable)
        ));
    }

    #[test]
    fn two_creators_of_one_scope_get_one_key() {
        let d = tempfile::tempdir().unwrap();
        let authority: Arc<dyn KeyAuthority> =
            Arc::new(crate::keys::MemoryKeyAuthority::new().unwrap());
        let keys = Arc::new(ScopeKeys::open(d.path(), authority).unwrap());
        let made: Vec<_> = (0..8)
            .map(|_| {
                let keys = keys.clone();
                std::thread::spawn(move || keys.create("conversation:one").unwrap().0)
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect();
        assert!(made.iter().all(|key| key == &made[0]));
        assert_eq!(keys.get("conversation:one").unwrap().0, made[0]);
    }

    #[test]
    fn hold_blocks_shred_until_released() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let d = tempfile::tempdir().unwrap();
        let s = keys(d.path(), &a);
        s.create("c").unwrap();
        s.hold("c").unwrap();
        assert!(s.held_scopes().unwrap().contains("c"));
        assert!(matches!(s.shred("c"), Err(StorageError::Held(_))));
        assert!(s.get("c").is_ok());
        s.release("c").unwrap();
        s.shred("c").unwrap();
        assert!(s.hold("c").is_err());
    }

    #[test]
    fn gc_keeps_everything_reachable_for_live_grants() {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let (kd, od) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let s = keys(kd.path(), &a);
        let objs = LocalObjectStore::open(od.path(), IdKey::new(&[3; 32]), a.clone()).unwrap();
        let shared = objs.put(b"shared", "a").unwrap();
        objs.grant(&shared, "a", "b").unwrap();
        let only_a = objs.put(b"only a", "a").unwrap();
        let held = objs.put(b"held content", "h").unwrap();
        let live = objs.put(b"live", "l").unwrap();
        s.create("a").unwrap();
        s.create("h").unwrap();
        s.hold("h").unwrap();

        s.shred("a").unwrap();
        a.revoke("h").unwrap();
        let removed = objs.gc_holding(&|scope| s.is_held(scope)).unwrap();
        assert_eq!(removed, 1);
        assert!(!objs.exists(&only_a));
        assert!(objs.exists(&shared));
        assert!(objs.exists(&held));
        assert!(objs.exists(&live));
        assert_eq!(objs.get(&shared, "b").unwrap(), b"shared");
        assert!(objs.get(&shared, "a").is_err());

        a.set_healthy(false).unwrap();
        assert!(objs.gc().is_err());
        assert!(objs.exists(&live));
    }
}
