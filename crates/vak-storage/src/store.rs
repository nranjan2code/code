//! The `Store` seam: objects plus refs behind one commit generation.
//!
//! Every mutating call bumps the commit generation and then runs the
//! pre-acknowledgement hook with the new generation before returning success.
//! A hook that refuses (a remote that has not confirmed) makes the call an
//! error although the change is locally durable; that is how a strong
//! durability mode is added later without changing callers. `restore` bumps
//! the writer epoch, which fences every holder of the old epoch with no
//! remote involved.

use crate::keys::KeyAuthority;
use crate::objects::{IdKey, LocalObjectStore, ObjectId};
use crate::refs::{MemoryRefStore, RefStore, RefValue, SqliteRefStore};
use crate::{Result, StorageError};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub type PreAck = Arc<dyn Fn(u64) -> Result<()> + Send + Sync>;

const COMMIT_REF: &str = "_store/commit";
const EPOCH_REF: &str = "_store/epoch";

pub trait Store: Send + Sync {
    /// The id `put_object` gives `plaintext`, without storing it.
    fn object_id(&self, plaintext: &[u8]) -> ObjectId;
    /// User ref names starting with `prefix`, sorted.
    fn ref_names(&self, prefix: &str) -> Result<Vec<String>>;
    fn put_object(&self, plaintext: &[u8], scope: &str) -> Result<ObjectId>;
    fn get_object(&self, id: &ObjectId, scope: &str) -> Result<Vec<u8>>;
    fn remove_grant(&self, id: &ObjectId, scope: &str) -> Result<()>;
    /// Every object `scope` holds a grant on, with the grant's age.
    fn granted_to(&self, scope: &str) -> Result<Vec<(ObjectId, std::time::Duration)>>;
    /// Collects objects with no live grant (scopes in `held` always count).
    fn gc(&self, held: &dyn Fn(&str) -> bool) -> Result<usize>;
    fn get_ref(&self, name: &str) -> Result<Option<RefValue>>;
    /// Refused with `StaleEpoch` when `epoch` is older than the store's.
    fn cas_ref(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        target: &[u8],
    ) -> Result<RefValue>;
    /// Removes a ref whose name is never reused; refused like `cas_ref`.
    fn remove_ref(&self, name: &str, expected: u64, epoch: u64) -> Result<()>;
    /// Count of committed mutations; strictly increasing, survives reopen.
    fn commit_generation(&self) -> Result<u64>;
    fn epoch(&self) -> Result<u64>;
    /// Called after restoring a store from a backup: fences the old writer.
    fn restore(&self) -> Result<u64>;
    fn set_pre_ack(&self, hook: Option<PreAck>) -> Result<()>;
}

/// A remote copy of a store. Declared so the durability modes can name it;
/// no implementation exists in this crate.
pub trait Remote: Send + Sync {
    fn push_object(&self, id: &ObjectId, sealed: &[u8]) -> Result<()>;
    fn fetch_object(&self, id: &ObjectId) -> Result<Option<Vec<u8>>>;
    fn push_ref(&self, name: &str, value: &RefValue) -> Result<()>;
    /// The highest commit generation the remote has durably accepted.
    fn acknowledged_generation(&self) -> Result<u64>;
}

fn user_ref(name: &str) -> Result<()> {
    if name.starts_with("_store/") {
        Err(StorageError::Malformed("reserved ref name"))
    } else {
        Ok(())
    }
}

fn counter(refs: &dyn RefStore, name: &str) -> Result<u64> {
    match refs.get(name)? {
        None => Ok(0),
        Some(v) => {
            let b: [u8; 8] = v
                .target
                .as_slice()
                .try_into()
                .map_err(|_| StorageError::Malformed("store counter"))?;
            Ok(u64::from_le_bytes(b))
        }
    }
}

fn bump(refs: &dyn RefStore, name: &str) -> Result<u64> {
    let cur = refs.get(name)?;
    let next = counter(refs, name)? + 1;
    refs.cas(name, cur.map(|c| c.generation), 0, &next.to_le_bytes())?;
    Ok(next)
}

struct Meta {
    refs: Box<dyn RefStore>,
    serial: Mutex<()>,
    hook: Mutex<Option<PreAck>>,
}

impl Meta {
    fn new(refs: Box<dyn RefStore>) -> Self {
        Self {
            refs,
            serial: Mutex::new(()),
            hook: Mutex::new(None),
        }
    }

    fn committed(&self) -> Result<u64> {
        let generation = {
            let _g = self
                .serial
                .lock()
                .map_err(|_| StorageError::Malformed("store state poisoned"))?;
            bump(self.refs.as_ref(), COMMIT_REF)?
        };
        let hook = self
            .hook
            .lock()
            .map_err(|_| StorageError::Malformed("store state poisoned"))?
            .clone();
        if let Some(h) = hook {
            h(generation)?;
        }
        Ok(generation)
    }

    fn cas_ref(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        target: &[u8],
    ) -> Result<RefValue> {
        user_ref(name)?;
        let current = counter(self.refs.as_ref(), EPOCH_REF)?;
        if epoch < current {
            return Err(StorageError::StaleEpoch {
                presented: epoch,
                current,
            });
        }
        let v = self.refs.cas(name, expected, epoch, target)?;
        self.committed()?;
        Ok(v)
    }

    fn remove_ref(&self, name: &str, expected: u64, epoch: u64) -> Result<()> {
        user_ref(name)?;
        let current = counter(self.refs.as_ref(), EPOCH_REF)?;
        if epoch < current {
            return Err(StorageError::StaleEpoch {
                presented: epoch,
                current,
            });
        }
        self.refs.remove(name, expected, epoch)?;
        self.committed().map(|_| ())
    }

    fn restore(&self) -> Result<u64> {
        let e = {
            let _g = self
                .serial
                .lock()
                .map_err(|_| StorageError::Malformed("store state poisoned"))?;
            bump(self.refs.as_ref(), EPOCH_REF)?
        };
        self.committed()?;
        Ok(e)
    }

    fn set_hook(&self, hook: Option<PreAck>) -> Result<()> {
        *self
            .hook
            .lock()
            .map_err(|_| StorageError::Malformed("store state poisoned"))? = hook;
        Ok(())
    }
}

pub struct LocalStore {
    objects: LocalObjectStore,
    meta: Meta,
}

impl LocalStore {
    pub fn open(root: &Path, id_key: IdKey, authority: Arc<dyn KeyAuthority>) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        Ok(Self {
            objects: LocalObjectStore::open(&root.join("cas"), id_key, authority)?,
            meta: Meta::new(Box::new(SqliteRefStore::open(&root.join("refs.db"))?)),
        })
    }

    /// Every ref as one portable file: a line each of the name in hex, its
    /// generation, its epoch and its target in hex, sorted by name. What a
    /// remote holds in place of the refs database (plan M9). A ref whose
    /// name starts with one of `local` belongs to this machine alone and
    /// is left out: a liveness ref renewed every tick would otherwise make
    /// the remote look one change behind for ever.
    pub fn export_refs(&self, local: &[&str]) -> Result<Vec<u8>> {
        let mut out = String::new();
        for name in self.meta.refs.names("")? {
            // The commit counter counts this store's own writes.
            if name == COMMIT_REF || local.iter().any(|prefix| name.starts_with(prefix)) {
                continue;
            }
            if let Some(value) = self.meta.refs.get(&name)? {
                out.push_str(&format!(
                    "{} {} {} {}\n",
                    crate::seal::hex(name.as_bytes()),
                    value.generation,
                    value.epoch,
                    crate::seal::hex(&value.target)
                ));
            }
        }
        Ok(out.into_bytes())
    }

    /// Makes this store's refs those of an [`export_refs`] file, apart
    /// from this machine's own refs under `local`, which stay; keeping
    /// this store's own writer epoch: each ref is stamped with an epoch
    /// no newer than it, so the process that imports can still finish
    /// what the import needs. Returns the epoch the file's store had; the
    /// caller ends by moving this store's epoch past it.
    pub fn import_refs(&self, file: &[u8], local: &[&str]) -> Result<u64> {
        let bad = || StorageError::Malformed("refs file");
        let unhex = |hex: &str| -> Result<Vec<u8>> {
            if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
                return Err(bad());
            }
            (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| bad()))
                .collect()
        };
        let text = std::str::from_utf8(file).map_err(|_| bad())?;
        let mut rows = Vec::new();
        for line in text.lines() {
            let mut parts = line.split(' ');
            let name =
                String::from_utf8(unhex(parts.next().ok_or_else(bad)?)?).map_err(|_| bad())?;
            let generation = parts.next().and_then(|n| n.parse().ok()).ok_or_else(bad)?;
            let epoch = parts.next().and_then(|n| n.parse().ok()).ok_or_else(bad)?;
            let target = unhex(parts.next().unwrap_or(""))?;
            rows.push((
                name,
                RefValue {
                    generation,
                    epoch,
                    target,
                },
            ));
        }
        let here = counter(self.meta.refs.as_ref(), EPOCH_REF)?;
        let mut theirs = 0;
        let mut kept = Vec::with_capacity(rows.len());
        let is_local =
            |name: &str| name == COMMIT_REF || local.iter().any(|prefix| name.starts_with(prefix));
        for name in self.meta.refs.names("")? {
            if is_local(&name)
                && let Some(value) = self.meta.refs.get(&name)?
            {
                kept.push((name, value));
            }
        }
        for (name, mut value) in rows {
            if is_local(&name) {
                continue;
            }
            if name == EPOCH_REF {
                let bytes: [u8; 8] = value
                    .target
                    .as_slice()
                    .try_into()
                    .map_err(|_| StorageError::Malformed("store counter"))?;
                theirs = u64::from_le_bytes(bytes);
                continue;
            }
            value.epoch = value.epoch.min(here);
            kept.push((name, value));
        }
        if let Some(epoch) = self.meta.refs.get(EPOCH_REF)? {
            kept.push((EPOCH_REF.to_string(), epoch));
        }
        self.meta.refs.replace_all(&kept)?;
        Ok(theirs)
    }

    /// Whether the store holds no object.
    pub fn holds_nothing(&self) -> bool {
        self.objects.is_empty()
    }

    /// Wraps again every object grant not under the current KEK version.
    pub fn rewrap_grants(&self) -> Result<usize> {
        self.objects.rewrap()
    }

    /// The oldest KEK version any object grant is still wrapped under.
    pub fn oldest_grant_version(&self) -> Result<Option<u32>> {
        self.objects.oldest_version()
    }
}

impl Store for LocalStore {
    fn object_id(&self, plaintext: &[u8]) -> ObjectId {
        self.objects.id(plaintext)
    }

    fn ref_names(&self, prefix: &str) -> Result<Vec<String>> {
        user_ref(prefix)?;
        self.meta.refs.names(prefix)
    }

    fn put_object(&self, plaintext: &[u8], scope: &str) -> Result<ObjectId> {
        let id = self.objects.put(plaintext, scope)?;
        self.meta.committed()?;
        Ok(id)
    }

    fn get_object(&self, id: &ObjectId, scope: &str) -> Result<Vec<u8>> {
        self.objects.get(id, scope)
    }

    fn remove_grant(&self, id: &ObjectId, scope: &str) -> Result<()> {
        self.objects.remove_grant(id, scope)?;
        self.meta.committed().map(|_| ())
    }

    fn granted_to(&self, scope: &str) -> Result<Vec<(ObjectId, std::time::Duration)>> {
        self.objects.granted_to(scope)
    }

    fn gc(&self, held: &dyn Fn(&str) -> bool) -> Result<usize> {
        let n = self.objects.gc_holding(held)?;
        if n > 0 {
            self.meta.committed()?;
        }
        Ok(n)
    }

    fn get_ref(&self, name: &str) -> Result<Option<RefValue>> {
        user_ref(name)?;
        self.meta.refs.get(name)
    }

    fn cas_ref(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        target: &[u8],
    ) -> Result<RefValue> {
        self.meta.cas_ref(name, expected, epoch, target)
    }

    fn remove_ref(&self, name: &str, expected: u64, epoch: u64) -> Result<()> {
        self.meta.remove_ref(name, expected, epoch)
    }

    fn commit_generation(&self) -> Result<u64> {
        counter(self.meta.refs.as_ref(), COMMIT_REF)
    }

    fn epoch(&self) -> Result<u64> {
        counter(self.meta.refs.as_ref(), EPOCH_REF)
    }

    fn restore(&self) -> Result<u64> {
        self.meta.restore()
    }

    fn set_pre_ack(&self, hook: Option<PreAck>) -> Result<()> {
        self.meta.set_hook(hook)
    }
}

#[derive(Default)]
struct MemObjects {
    bodies: HashMap<ObjectId, Vec<u8>>,
    grants: HashMap<ObjectId, HashSet<String>>,
}

/// In-memory store for tests and ephemeral work. No encryption: nothing here
/// reaches a disk.
pub struct MemoryStore {
    id_key: IdKey,
    objects: Mutex<MemObjects>,
    meta: Meta,
}

impl MemoryStore {
    pub fn new(id_key: IdKey) -> Self {
        Self {
            id_key,
            objects: Mutex::new(MemObjects::default()),
            meta: Meta::new(Box::new(MemoryRefStore::new())),
        }
    }

    fn objs(&self) -> Result<std::sync::MutexGuard<'_, MemObjects>> {
        self.objects
            .lock()
            .map_err(|_| StorageError::Malformed("store state poisoned"))
    }
}

impl Store for MemoryStore {
    fn object_id(&self, plaintext: &[u8]) -> ObjectId {
        self.id_key.id(plaintext)
    }

    fn ref_names(&self, prefix: &str) -> Result<Vec<String>> {
        user_ref(prefix)?;
        self.meta.refs.names(prefix)
    }

    fn put_object(&self, plaintext: &[u8], scope: &str) -> Result<ObjectId> {
        let id = self.id_key.id(plaintext);
        {
            let mut o = self.objs()?;
            o.bodies
                .entry(id.clone())
                .or_insert_with(|| plaintext.to_vec());
            o.grants
                .entry(id.clone())
                .or_default()
                .insert(scope.to_string());
        }
        self.meta.committed()?;
        Ok(id)
    }

    fn get_object(&self, id: &ObjectId, scope: &str) -> Result<Vec<u8>> {
        let o = self.objs()?;
        if !o.grants.get(id).is_some_and(|g| g.contains(scope)) {
            return Err(StorageError::NoGrant);
        }
        o.bodies.get(id).cloned().ok_or(StorageError::NotFound)
    }

    fn remove_grant(&self, id: &ObjectId, scope: &str) -> Result<()> {
        if let Some(g) = self.objs()?.grants.get_mut(id) {
            g.remove(scope);
        }
        self.meta.committed().map(|_| ())
    }

    /// Nothing here keeps a time, so every grant reads as long held.
    fn granted_to(&self, scope: &str) -> Result<Vec<(ObjectId, std::time::Duration)>> {
        Ok(self
            .objs()?
            .grants
            .iter()
            .filter(|(_, scopes)| scopes.contains(scope))
            .map(|(id, _)| (id.clone(), std::time::Duration::MAX))
            .collect())
    }

    fn gc(&self, _held: &dyn Fn(&str) -> bool) -> Result<usize> {
        let n = {
            let mut o = self.objs()?;
            let dead: Vec<ObjectId> = o
                .bodies
                .keys()
                .filter(|id| !o.grants.get(*id).is_some_and(|g| !g.is_empty()))
                .cloned()
                .collect();
            for id in &dead {
                o.bodies.remove(id);
                o.grants.remove(id);
            }
            dead.len()
        };
        if n > 0 {
            self.meta.committed()?;
        }
        Ok(n)
    }

    fn get_ref(&self, name: &str) -> Result<Option<RefValue>> {
        user_ref(name)?;
        self.meta.refs.get(name)
    }

    fn cas_ref(
        &self,
        name: &str,
        expected: Option<u64>,
        epoch: u64,
        target: &[u8],
    ) -> Result<RefValue> {
        self.meta.cas_ref(name, expected, epoch, target)
    }

    fn remove_ref(&self, name: &str, expected: u64, epoch: u64) -> Result<()> {
        self.meta.remove_ref(name, expected, epoch)
    }

    fn commit_generation(&self) -> Result<u64> {
        counter(self.meta.refs.as_ref(), COMMIT_REF)
    }

    fn epoch(&self) -> Result<u64> {
        counter(self.meta.refs.as_ref(), EPOCH_REF)
    }

    fn restore(&self) -> Result<u64> {
        self.meta.restore()
    }

    fn set_pre_ack(&self, hook: Option<PreAck>) -> Result<()> {
        self.meta.set_hook(hook)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::keys::MemoryKeyAuthority;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn local(dir: &Path) -> LocalStore {
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        LocalStore::open(dir, IdKey::new(&[5; 32]), a).unwrap()
    }

    fn exercise(s: &dyn Store) {
        let g0 = s.commit_generation().unwrap();
        let id = s.put_object(b"hello", "a").unwrap();
        assert_eq!(s.get_object(&id, "a").unwrap(), b"hello");
        assert!(matches!(s.get_object(&id, "b"), Err(StorageError::NoGrant)));
        let r = s.cas_ref("head", None, 0, id.0.as_bytes()).unwrap();
        assert_eq!(r.generation, 1);
        assert!(s.commit_generation().unwrap() >= g0 + 2);
        assert!(s.cas_ref("_store/epoch", None, 0, b"x").is_err());
        s.remove_grant(&id, "a").unwrap();
        assert_eq!(s.gc(&|_| false).unwrap(), 1);
        assert!(s.get_object(&id, "a").is_err());
    }

    #[test]
    fn both_stores_satisfy_the_contract() {
        let d = tempfile::tempdir().unwrap();
        exercise(&local(d.path()));
        exercise(&MemoryStore::new(IdKey::new(&[5; 32])));
    }

    #[test]
    fn commit_generation_survives_reopen() {
        let d = tempfile::tempdir().unwrap();
        let g = {
            let s = local(d.path());
            s.put_object(b"x", "a").unwrap();
            s.commit_generation().unwrap()
        };
        assert_eq!(g, 1);
        assert_eq!(local(d.path()).commit_generation().unwrap(), 1);
    }

    #[test]
    fn restore_bumps_epoch() {
        let d = tempfile::tempdir().unwrap();
        for s in [
            Box::new(local(d.path())) as Box<dyn Store>,
            Box::new(MemoryStore::new(IdKey::new(&[1; 32]))),
        ] {
            let old = s.epoch().unwrap();
            let v = s.cas_ref("head", None, old, b"1").unwrap();
            let new = s.restore().unwrap();
            assert_eq!(new, old + 1);
            assert_eq!(s.epoch().unwrap(), new);
            assert!(matches!(
                s.cas_ref("head", Some(v.generation), old, b"2"),
                Err(StorageError::StaleEpoch { .. })
            ));
            assert!(matches!(
                s.cas_ref("other", None, old, b"2"),
                Err(StorageError::StaleEpoch { .. })
            ));
            let v = s.cas_ref("head", Some(v.generation), new, b"3").unwrap();
            assert!(matches!(
                s.remove_ref("head", v.generation, old),
                Err(StorageError::StaleEpoch { .. })
            ));
            s.remove_ref("head", v.generation, new).unwrap();
            assert!(s.get_ref("head").unwrap().is_none());
            assert!(s.remove_ref("_store/epoch", 1, new).is_err());
        }
    }

    #[test]
    fn pre_ack_hook_sees_each_generation_and_can_refuse() {
        let s = MemoryStore::new(IdKey::new(&[1; 32]));
        let seen = Arc::new(AtomicU64::new(0));
        let s2 = seen.clone();
        s.set_pre_ack(Some(Arc::new(move |g| {
            s2.store(g, Ordering::SeqCst);
            Ok(())
        })))
        .unwrap();
        s.put_object(b"a", "x").unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 1);
        s.set_pre_ack(Some(Arc::new(|_| {
            Err(StorageError::AuthorityUnavailable("remote down".into()))
        })))
        .unwrap();
        assert!(s.put_object(b"b", "x").is_err());
        assert_eq!(s.commit_generation().unwrap(), 2);
        s.set_pre_ack(None).unwrap();
        s.put_object(b"c", "x").unwrap();
    }
}
