//! Versioned named content. Every save is an immutable version object (it
//! names its content object, its parent version and its sequence number), and
//! `doc/<name>` is a CAS ref to the current version. Forgetting a document
//! tombstones the ref and drops this scope's grants on every version and
//! content object, so the next collection frees them.

use crate::objects::ObjectId;
use crate::store::Store;
use crate::{Result, StorageError};
use std::sync::Arc;

const TOMBSTONE: &[u8] = b"forgotten";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    /// The version object's id.
    pub id: ObjectId,
    pub content: ObjectId,
    pub parent: Option<ObjectId>,
    /// 1 for the first version of a document, then counting up.
    pub seq: u64,
    /// When the version was saved, in seconds since the epoch. Absent on a
    /// version written before versions carried a time: its age is unknown.
    pub at: Option<i64>,
}

/// A version record. It names its document, so two documents never share
/// one: a first version of the same content saved in the same second
/// would otherwise be one object, and releasing it for one document would
/// take it from the other.
fn encode(name: &str, content: &ObjectId, parent: Option<&ObjectId>, seq: u64, at: i64) -> Vec<u8> {
    format!(
        "v1\nparent:{}\ncontent:{}\nseq:{seq}\nat:{at}\ndoc:{name}",
        parent.map_or("-", |p| p.0.as_str()),
        content.0
    )
    .into_bytes()
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

fn decode(id: &ObjectId, bytes: &[u8]) -> Result<Version> {
    let bad = || StorageError::Malformed("document version");
    let s = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let mut it = s.lines();
    if it.next() != Some("v1") {
        return Err(bad());
    }
    let parent = it
        .next()
        .and_then(|l| l.strip_prefix("parent:"))
        .ok_or_else(bad)?;
    let content = it
        .next()
        .and_then(|l| l.strip_prefix("content:"))
        .ok_or_else(bad)?;
    let seq = it
        .next()
        .and_then(|l| l.strip_prefix("seq:"))
        .and_then(|v| v.parse().ok())
        .ok_or_else(bad)?;
    let at = it
        .next()
        .and_then(|l| l.strip_prefix("at:"))
        .and_then(|v| v.parse().ok());
    Ok(Version {
        id: id.clone(),
        content: ObjectId(content.to_string()),
        parent: (parent != "-").then(|| ObjectId(parent.to_string())),
        seq,
        at,
    })
}

pub struct Documents {
    store: Arc<dyn Store>,
    scope: String,
    epoch: u64,
}

impl Documents {
    /// `epoch` is the writer epoch this holder acts under; a store restored
    /// since refuses a lower one.
    pub fn new(store: Arc<dyn Store>, scope: &str, epoch: u64) -> Self {
        Self {
            store,
            scope: scope.to_string(),
            epoch,
        }
    }

    fn ref_name(name: &str) -> String {
        format!("doc/{name}")
    }

    /// Live documents whose names start with `prefix`, sorted; forgotten
    /// ones are left out.
    pub fn names(&self, prefix: &str) -> Result<Vec<String>> {
        let mut live = Vec::new();
        for full in self.store.ref_names(&Self::ref_name(prefix))? {
            let Some(name) = full.strip_prefix("doc/") else {
                continue;
            };
            if matches!(self.head(name), Ok(Some(_))) {
                live.push(name.to_string());
            }
        }
        Ok(live)
    }

    fn version(&self, id: &ObjectId) -> Result<Version> {
        decode(id, &self.store.get_object(id, &self.scope)?)
    }

    /// The generation to pass as `expected` to `save`, `None` for a document
    /// that has never existed.
    pub fn generation(&self, name: &str) -> Result<Option<u64>> {
        Ok(self
            .store
            .get_ref(&Self::ref_name(name))?
            .map(|r| r.generation))
    }

    /// Saves a new version. `expected` is the generation the caller read; a
    /// move since then is a `Conflict` and nothing is written.
    pub fn save(&self, name: &str, content: &[u8], expected: Option<u64>) -> Result<Version> {
        self.save_at(name, content, expected, now_secs())
    }

    /// As `save`, stamping the version with `at` (seconds since the epoch).
    pub fn save_at(
        &self,
        name: &str,
        content: &[u8],
        expected: Option<u64>,
        at: i64,
    ) -> Result<Version> {
        let rname = Self::ref_name(name);
        let cur = self.store.get_ref(&rname)?;
        let cur_gen = cur.as_ref().map(|c| c.generation);
        if cur_gen != expected {
            return Err(StorageError::Conflict {
                expected,
                current: cur_gen,
            });
        }
        let (parent, seq) = match cur.as_ref().filter(|c| c.target != TOMBSTONE) {
            Some(c) => {
                let id = ObjectId(String::from_utf8_lossy(&c.target).into_owned());
                let v = self.version(&id)?;
                (Some(id), v.seq + 1)
            }
            None => (None, 1),
        };
        let content_id = self.store.put_object(content, &self.scope)?;
        let vbytes = encode(name, &content_id, parent.as_ref(), seq, at);
        let vid = self.store.put_object(&vbytes, &self.scope)?;
        self.store
            .cas_ref(&rname, expected, self.epoch, vid.0.as_bytes())?;
        Ok(Version {
            id: vid,
            content: content_id,
            parent,
            seq,
            at: Some(at),
        })
    }

    fn head(&self, name: &str) -> Result<Option<ObjectId>> {
        match self.store.get_ref(&Self::ref_name(name))? {
            None => Ok(None),
            Some(r) if r.target == TOMBSTONE => Err(StorageError::Forgotten),
            Some(r) => Ok(Some(ObjectId(
                String::from_utf8(r.target).map_err(|_| StorageError::Malformed("document ref"))?,
            ))),
        }
    }

    pub fn current(&self, name: &str) -> Result<Option<(Version, Vec<u8>)>> {
        let Some(id) = self.head(name)? else {
            return Ok(None);
        };
        let v = self.version(&id)?;
        let body = self.store.get_object(&v.content, &self.scope)?;
        Ok(Some((v, body)))
    }

    /// Newest first. The history ends where an earlier version was pruned.
    pub fn history(&self, name: &str) -> Result<Vec<Version>> {
        let mut out: Vec<Version> = Vec::new();
        let mut next = self.head(name)?;
        while let Some(id) = next {
            let v = match self.version(&id) {
                Ok(v) => v,
                // A parent that is gone is where retention cut the history.
                Err(StorageError::NotFound | StorageError::NoGrant) if !out.is_empty() => break,
                Err(error) => return Err(error),
            };
            next = v.parent.clone();
            out.push(v);
        }
        Ok(out)
    }

    /// Unlinks every version saved at or before `cutoff` and everything
    /// older than it, never the current one. Only the version records are
    /// released: a content object may be another document's too, so what
    /// nothing names any more is left for the store's collection. Returns
    /// how many versions left the history.
    pub fn prune(&self, name: &str, cutoff: i64) -> Result<usize> {
        let history = self.history(name)?;
        let first_old = history
            .iter()
            .skip(1)
            .position(|v| v.at.is_some_and(|at| at <= cutoff))
            .map(|index| index + 1);
        let Some(first_old) = first_old else {
            return Ok(0);
        };
        // A record with no time predates records that name their document
        // and may be shared; it is left, unreachable, for collection.
        for v in history[first_old..].iter().filter(|v| v.at.is_some()) {
            self.store.remove_grant(&v.id, &self.scope)?;
        }
        Ok(history.len() - first_old)
    }

    pub fn read_version(&self, v: &Version) -> Result<Vec<u8>> {
        self.store.get_object(&v.content, &self.scope)
    }

    /// Tombstones the document and releases this scope's grants on all of its
    /// objects. Returns how many versions were forgotten.
    pub fn forget(&self, name: &str) -> Result<usize> {
        let rname = Self::ref_name(name);
        let history = self.history(name)?;
        let cur = self.generation(name)?;
        self.store.cas_ref(&rname, cur, self.epoch, TOMBSTONE)?;
        for v in &history {
            self.store.remove_grant(&v.content, &self.scope)?;
            self.store.remove_grant(&v.id, &self.scope)?;
        }
        Ok(history.len())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::keys::MemoryKeyAuthority;

    #[test]
    fn pruning_unlinks_old_versions_and_keeps_the_current_and_shared_content() {
        let store: Arc<dyn Store> = Arc::new(MemoryStore::new(IdKey::new(&[7; 32])));
        let docs = Documents::new(store, "tenant", 0);
        let day = 86_400;
        let mut generation = None;
        for (n, text) in ["one", "two", "three"].into_iter().enumerate() {
            docs.save_at("note", text.as_bytes(), generation, n as i64 * day)
                .unwrap();
            generation = docs.generation("note").unwrap();
        }
        // Another document with the text an old version of `note` has.
        docs.save_at("other", b"one", None, 0).unwrap();

        // Nothing is as old as the cutoff: nothing goes.
        assert_eq!(docs.prune("note", -1).unwrap(), 0);
        // The first two are; the current one stays however old.
        assert_eq!(docs.prune("note", 5 * day).unwrap(), 2);
        let left = docs.history("note").unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].at, Some(2 * day));
        assert_eq!(docs.current("note").unwrap().unwrap().1, b"three");
        assert_eq!(
            docs.current("other").unwrap().unwrap().1,
            b"one",
            "shared content stays"
        );
        assert_eq!(
            docs.prune("note", 5 * day).unwrap(),
            0,
            "pruning twice is pruning once"
        );
        // A document saved once has no history to prune.
        assert_eq!(docs.prune("other", 5 * day).unwrap(), 0);
        // Saving goes on from the kept head.
        let next = docs.generation("note").unwrap();
        docs.save_at("note", b"four", next, 9 * day).unwrap();
        assert_eq!(docs.history("note").unwrap().len(), 2);
    }
    use crate::objects::IdKey;
    use crate::store::{LocalStore, MemoryStore};

    fn stores() -> (tempfile::TempDir, Vec<Arc<dyn Store>>) {
        let d = tempfile::tempdir().unwrap();
        let a = Arc::new(MemoryKeyAuthority::new().unwrap());
        let local = LocalStore::open(d.path(), IdKey::new(&[9; 32]), a).unwrap();
        let mem = MemoryStore::new(IdKey::new(&[9; 32]));
        (d, vec![Arc::new(local), Arc::new(mem)])
    }

    #[test]
    fn documents_version_history() {
        let (_d, all) = stores();
        for s in all {
            let docs = Documents::new(s.clone(), "space", 0);
            assert!(docs.current("plan").unwrap().is_none());
            let v1 = docs.save("plan", b"first", None).unwrap();
            let g = docs.generation("plan").unwrap();
            let v2 = docs.save("plan", b"second", g).unwrap();
            let g = docs.generation("plan").unwrap();
            let v3 = docs.save("plan", b"third", g).unwrap();
            assert_eq!((v1.seq, v2.seq, v3.seq), (1, 2, 3));
            assert_eq!(v2.parent.as_ref(), Some(&v1.id));
            let (cur, body) = docs.current("plan").unwrap().unwrap();
            assert_eq!((cur.id.clone(), body), (v3.id.clone(), b"third".to_vec()));
            let h = docs.history("plan").unwrap();
            assert_eq!(h.len(), 3);
            assert_eq!(docs.read_version(&h[2]).unwrap(), b"first");
            assert_eq!(docs.read_version(&h[1]).unwrap(), b"second");
        }
    }

    #[test]
    fn a_stale_save_conflicts_and_writes_nothing() {
        let (_d, all) = stores();
        for s in all {
            let docs = Documents::new(s.clone(), "space", 0);
            docs.save("n", b"a", None).unwrap();
            let before = s.commit_generation().unwrap();
            assert!(matches!(
                docs.save("n", b"b", None),
                Err(StorageError::Conflict { .. })
            ));
            assert_eq!(s.commit_generation().unwrap(), before);
            assert_eq!(docs.history("n").unwrap().len(), 1);
        }
    }

    #[test]
    fn saving_identical_content_still_makes_a_new_version() {
        let (_d, all) = stores();
        let docs = Documents::new(all[1].clone(), "s", 0);
        let a = docs.save("n", b"same", None).unwrap();
        let b = docs.save("n", b"same", Some(1)).unwrap();
        assert_eq!(a.content, b.content);
        assert_ne!(a.id, b.id);
        assert_eq!(docs.history("n").unwrap().len(), 2);
    }

    #[test]
    fn forget_tombstones_and_frees_the_objects() {
        let (_d, all) = stores();
        for s in all {
            let docs = Documents::new(s.clone(), "space", 0);
            let v1 = docs.save("n", b"one", None).unwrap();
            docs.save("n", b"two", Some(1)).unwrap();
            assert_eq!(docs.forget("n").unwrap(), 2);
            assert!(matches!(docs.current("n"), Err(StorageError::Forgotten)));
            assert!(matches!(docs.history("n"), Err(StorageError::Forgotten)));
            assert_eq!(s.gc(&|_| false).unwrap(), 4);
            assert!(s.get_object(&v1.content, "space").is_err());
            let g = docs.generation("n").unwrap();
            let fresh = docs.save("n", b"again", g).unwrap();
            assert_eq!(fresh.seq, 1);
            assert!(fresh.parent.is_none());
        }
    }

    #[test]
    fn a_stale_epoch_cannot_save() {
        let (_d, all) = stores();
        let s = all[1].clone();
        let docs = Documents::new(s.clone(), "space", 0);
        docs.save("n", b"a", None).unwrap();
        s.restore().unwrap();
        assert!(matches!(
            docs.save("n", b"b", Some(1)),
            Err(StorageError::StaleEpoch { .. })
        ));
        let fenced = Documents::new(s, "space", 1);
        fenced.save("n", b"b", Some(1)).unwrap();
    }
}
