//! The folder remote (data-architecture plan M9): a second copy of what
//! this install stores, in a folder the owner chose, kept so that another
//! machine can take the work over.
//!
//! The remote carries exactly what a backup without secrets carries: the
//! registry's backup targets under the data home, as they are stored,
//! which is encrypted. Each file is kept in the remote under its digest
//! (`blobs/`), and `index.json` names every path with its size and digest.
//! A push adds blobs and never changes one, then replaces the index, then
//! removes what the new index no longer names: stopped at any point, the
//! remote still holds a whole copy. A pull checks each file against the
//! index before it changes anything here. The refs database travels as a portable file
//! (`LocalStore::export_refs`), never as a database copied while open.
//! Secrets never leave: a second machine reads the remote with a key file
//! (`Core::export_key_file`).

use crate::Core;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const INDEX: &str = "index.json";
const BLOBS: &str = "blobs";
/// Present while a pull is being applied here; one left behind means
/// this home is part old and part new until a pull finishes.
const PULLING: &str = "pulling";
/// The refs of the tenant's store, as a file in the remote.
const REFS: &str = "refs.export";
const LOCAL: &str = "local.json";

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("no remote folder is set up; run `vak sync setup <folder>`")]
    NotSetUp,
    #[error("the remote folder cannot be reached")]
    Unreachable,
    #[error("the remote holds nothing yet; push from the machine that has the data")]
    Empty,
    #[error(
        "the remote is incomplete or damaged ({0} files do not match its index); push again from the other machine"
    )]
    Damaged(usize),
    #[error("this machine holds {0} files that were never pushed; pulling would replace them")]
    Unpushed(usize),
    #[error("{0}")]
    Failed(String),
}

fn failed(error: impl std::fmt::Display) -> SyncError {
    SyncError::Failed(error.to_string())
}

/// One file the remote holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub size: u64,
    pub sha256: String,
}

/// The remote's `index.json`: what it holds, and where the store stood
/// when it was pushed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub version: u32,
    /// Counts pushes; a newer push has a higher one.
    pub generation: u64,
    /// The store's writer epoch when it was pushed.
    pub epoch: u64,
    /// The machine that pushed it.
    pub machine: String,
    pub at: DateTime<Utc>,
    pub files: BTreeMap<String, Entry>,
}

/// What this machine knows about its remote. Machine-local: it is never
/// pushed, and a home that was pulled or restored sets it up again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Local {
    pub remote: PathBuf,
    /// This machine's id, minted at setup.
    pub machine: String,
    /// The remote generation this machine last pushed or pulled.
    #[serde(default)]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<DateTime<Utc>>,
    /// What this machine and the remote both held after that sync.
    #[serde(default)]
    pub synced: BTreeMap<String, Entry>,
    /// File digests by size and modified time, so an unchanged file is
    /// not read again.
    #[serde(default)]
    seen: BTreeMap<String, (u64, u128, String)>,
}

/// What a push or a pull did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    pub generation: u64,
    pub copied: u64,
    pub removed: u64,
    pub bytes: u64,
    pub files: u64,
}

/// Where this machine stands against its remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncStatus {
    pub remote: PathBuf,
    pub machine: String,
    pub reachable: bool,
    /// The generation this machine last pushed or pulled.
    pub generation: u64,
    /// The remote's generation now, when it can be read.
    pub remote_generation: Option<u64>,
    pub synced_at: Option<DateTime<Utc>>,
    /// Files here that differ from what was last synced.
    pub unpushed: u64,
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = dest.with_extension(format!(
        "{}part",
        dest.extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, dest)
}

fn list_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            list_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

/// Whether a path under the data home travels. Locks, scratch, the open
/// refs database (it travels as [`REFS`]) and the index of which secrets
/// exist on this machine do not.
fn travels(relative: &Path) -> bool {
    let name = relative
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name == "LOCK"
        || name.ends_with(".lock")
        || name.ends_with(".part")
        || name.starts_with("refs.db")
        || relative == Path::new("credential_index.json")
    {
        return false;
    }
    !relative.components().any(|part| part.as_os_str() == "tmp")
}

/// Where the remote keeps the bytes with this digest.
fn blob(remote: &Path, sha256: &str) -> PathBuf {
    remote
        .join(BLOBS)
        .join(sha256.get(..2).unwrap_or("00"))
        .join(sha256)
}

fn key_of(relative: &Path) -> String {
    relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

impl Core {
    fn sync_local_path(&self) -> PathBuf {
        self.shared_scope().sync().join(LOCAL)
    }

    fn sync_local(&self) -> Result<Local, SyncError> {
        let bytes = std::fs::read(self.sync_local_path()).map_err(|_| SyncError::NotSetUp)?;
        serde_json::from_slice(&bytes).map_err(failed)
    }

    fn save_sync_local(&self, local: &Local) -> Result<(), SyncError> {
        let bytes = serde_json::to_vec_pretty(local).map_err(failed)?;
        write_atomic(&self.sync_local_path(), &bytes).map_err(failed)
    }

    /// Where the tenant's refs sit among the remote's files.
    fn refs_key(&self) -> String {
        format!(
            "{}/{}/store/{REFS}",
            vak_config::paths::TENANTS_DIR,
            vak_config::paths::LOCAL_TENANT
        )
    }

    /// Every file here that travels, by its path under the data home.
    fn sync_files(&self) -> Vec<(String, PathBuf)> {
        let home = self.shared_scope().into_root();
        let mut found = Vec::new();
        for target in crate::state::backup_targets(crate::state::Root::Data, &home) {
            let path = home.join(&target);
            let mut files = Vec::new();
            if path.is_file() {
                files.push(path);
            } else {
                list_files(&path, &mut files);
            }
            for file in files {
                if let Ok(relative) = file.strip_prefix(&home)
                    && travels(relative)
                {
                    found.push((key_of(relative), file.clone()));
                }
            }
        }
        found.sort();
        found.dedup();
        found
    }

    /// What this machine holds now: every travelling file and the refs,
    /// each with its size and digest. A file whose size and modified time
    /// are unchanged is not read again.
    fn sync_snapshot(
        &self,
        local: &mut Local,
    ) -> Result<(BTreeMap<String, Entry>, Vec<u8>), SyncError> {
        let mut held = BTreeMap::new();
        let mut seen = BTreeMap::new();
        for (key, path) in self.sync_files() {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let modified = meta
                .modified()
                .ok()
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_nanos());
            let sha256 = match local.seen.get(&key) {
                Some((size, at, sha)) if *size == meta.len() && *at == modified => sha.clone(),
                _ => digest(&std::fs::read(&path).map_err(failed)?),
            };
            seen.insert(key.clone(), (meta.len(), modified, sha256.clone()));
            held.insert(
                key,
                Entry {
                    size: meta.len(),
                    sha256,
                },
            );
        }
        local.seen = seen;
        let refs = self
            .tenant_objects()
            .map_err(failed)?
            .export_refs()
            .map_err(failed)?;
        held.insert(
            self.refs_key(),
            Entry {
                size: refs.len() as u64,
                sha256: digest(&refs),
            },
        );
        Ok((held, refs))
    }

    fn read_index(remote: &Path) -> Result<Option<Index>, SyncError> {
        if !remote.is_dir() {
            return Err(SyncError::Unreachable);
        }
        match std::fs::read(remote.join(INDEX)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(failed),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(SyncError::Unreachable),
        }
    }

    /// Names the folder this machine keeps its remote copy in. The folder
    /// must exist and must not be inside the data home.
    pub fn sync_setup(&self, remote: &Path) -> Result<Local, SyncError> {
        let home = self.shared_scope().into_root();
        if !remote.is_dir() {
            return Err(SyncError::Unreachable);
        }
        let remote = remote.canonicalize().map_err(failed)?;
        if crate::backup::within_home(&remote, &home) {
            return Err(SyncError::Failed(
                "the remote folder cannot be inside Vakyartha's own data".into(),
            ));
        }
        let mut local = self.sync_local().unwrap_or_default();
        if local.machine.is_empty() {
            local.machine = format!("mch_{}", uuid::Uuid::now_v7());
        }
        if local.remote != remote {
            local.remote = remote;
            local.generation = 0;
            local.synced_at = None;
            local.synced.clear();
        }
        self.save_sync_local(&local)?;
        Ok(local)
    }

    /// Forgets the remote. Nothing in the folder is touched.
    pub fn sync_forget(&self) -> Result<(), SyncError> {
        match std::fs::remove_file(self.sync_local_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(failed(error)),
        }
    }

    /// Where this machine stands against its remote. Reads only.
    pub fn sync_status(&self) -> Result<SyncStatus, SyncError> {
        let mut local = self.sync_local()?;
        let index = Self::read_index(&local.remote);
        let (held, _) = self.sync_snapshot(&mut local)?;
        let unpushed = held
            .iter()
            .filter(|(key, entry)| local.synced.get(*key) != Some(entry))
            .count()
            + local
                .synced
                .keys()
                .filter(|key| !held.contains_key(*key))
                .count();
        Ok(SyncStatus {
            reachable: index.is_ok(),
            remote_generation: index.ok().flatten().map(|index| index.generation),
            remote: local.remote,
            machine: local.machine,
            generation: local.generation,
            synced_at: local.synced_at,
            unpushed: unpushed as u64,
        })
    }

    /// Makes the remote hold what this machine holds: copies what is new
    /// or changed, removes what is gone, and then replaces the index,
    /// which is what makes the push count. A push that stops part-way
    /// leaves the old index; the next one finishes it.
    pub fn sync_push(&self) -> Result<SyncReport, SyncError> {
        let _turn = crate::erasure::one_at_a_time();
        vak_session::fence::check().map_err(failed)?;
        let mut local = self.sync_local()?;
        let before = Self::read_index(&local.remote)?;
        let (held, refs) = self.sync_snapshot(&mut local)?;
        let home = self.shared_scope().into_root();
        let refs_key = self.refs_key();
        let (mut copied, mut bytes) = (0, 0);
        let mut pushed = BTreeMap::new();
        for (key, entry) in &held {
            let content = if *key == refs_key {
                Some(refs.clone())
            } else {
                let dest = blob(&local.remote, &entry.sha256);
                if std::fs::metadata(&dest).is_ok_and(|meta| meta.len() == entry.size) {
                    None
                } else {
                    Some(std::fs::read(home.join(key)).map_err(failed)?)
                }
            };
            // The index names what was actually sent: a file that changed
            // since it was measured goes as it is now.
            let entry = match &content {
                Some(content) => Entry {
                    size: content.len() as u64,
                    sha256: digest(content),
                },
                None => entry.clone(),
            };
            let dest = blob(&local.remote, &entry.sha256);
            if let Some(content) = content
                && !std::fs::metadata(&dest).is_ok_and(|meta| meta.len() == entry.size)
            {
                write_atomic(&dest, &content).map_err(|_| SyncError::Unreachable)?;
                copied += 1;
                bytes += content.len() as u64;
            }
            pushed.insert(key.clone(), entry);
        }
        let held = pushed;
        let tenant = self.tenant_objects().map_err(failed)?;
        let index = Index {
            version: 1,
            generation: before.map_or(0, |index| index.generation) + 1,
            epoch: tenant.store().epoch().map_err(failed)?,
            machine: local.machine.clone(),
            at: Utc::now(),
            files: held,
        };
        let encoded = serde_json::to_vec_pretty(&index).map_err(failed)?;
        write_atomic(&local.remote.join(INDEX), &encoded).map_err(|_| SyncError::Unreachable)?;
        // Only now, with the new index in place, does anything leave.
        let named: std::collections::BTreeSet<&str> = index
            .files
            .values()
            .map(|entry| entry.sha256.as_str())
            .collect();
        let mut stored = Vec::new();
        list_files(&local.remote.join(BLOBS), &mut stored);
        let mut removed = 0;
        for path in stored {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            if !name.is_some_and(|name| named.contains(name.as_str()))
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        }
        local.generation = index.generation;
        local.synced_at = Some(index.at);
        local.synced = index.files.clone();
        self.save_sync_local(&local)?;
        tracing::info!(
            kind = "sync",
            outcome = "pushed",
            count = copied,
            "the remote copy was brought up to date"
        );
        Ok(SyncReport {
            generation: index.generation,
            copied,
            removed,
            bytes,
            files: index.files.len() as u64,
        })
    }

    /// Makes this machine hold what the remote holds. Every file is
    /// checked against the index before anything here changes. Refused
    /// when this machine holds work it never pushed, unless `discard`.
    /// It ends as a restore does (`settle_imported`): erasures applied
    /// again, search rebuilt, the writer epoch moved, so every process on
    /// this data home must be started again.
    pub fn sync_pull(&self, discard: bool) -> Result<SyncReport, SyncError> {
        let _turn = crate::erasure::one_at_a_time();
        vak_session::fence::check().map_err(failed)?;
        let mut local = self.sync_local()?;
        let index = Self::read_index(&local.remote)?.ok_or(SyncError::Empty)?;
        let (held, _) = self.sync_snapshot(&mut local)?;
        let marker = self.shared_scope().sync().join(PULLING);

        let mine = held
            .iter()
            .filter(|(key, entry)| local.synced.get(*key) != Some(entry))
            .count();
        let fresh = local.synced.is_empty()
            && self
                .key_status()
                .is_ok_and(|status| status.keys + status.destroyed == 0);
        if mine > 0 && !fresh && !discard && !marker.exists() {
            return Err(SyncError::Unpushed(mine));
        }

        // Read and check everything that will be written, first.
        let mut incoming: Vec<(&String, Vec<u8>)> = Vec::new();
        let mut damaged = 0;
        for (key, entry) in &index.files {
            let path = blob(&local.remote, &entry.sha256);
            let size = std::fs::metadata(&path).map(|meta| meta.len()).ok();
            if size != Some(entry.size) {
                damaged += 1;
                continue;
            }
            if held.get(key) == Some(entry) {
                continue;
            }
            match std::fs::read(&path) {
                Ok(bytes) if digest(&bytes) == entry.sha256 => incoming.push((key, bytes)),
                _ => damaged += 1,
            }
        }
        if damaged > 0 {
            return Err(SyncError::Damaged(damaged));
        }

        let home = self.shared_scope().into_root();
        let refs_key = self.refs_key();
        write_atomic(&marker, b"").map_err(failed)?;
        let (mut copied, mut bytes, mut refs) = (0, 0, None);
        for (key, content) in incoming {
            bytes += content.len() as u64;
            copied += 1;
            if *key == refs_key {
                refs = Some(content);
            } else {
                write_atomic(&home.join(key), &content).map_err(failed)?;
            }
        }
        let mut removed = 0;
        for key in held.keys() {
            if !index.files.contains_key(key) && std::fs::remove_file(home.join(key)).is_ok() {
                removed += 1;
            }
        }
        if let Some(refs) = refs {
            self.tenant_objects()
                .map_err(failed)?
                .import_refs(&refs)
                .map_err(failed)?;
        }
        local.generation = index.generation;
        local.synced_at = Some(Utc::now());
        local.synced = index.files.clone();
        local.seen.clear();
        self.save_sync_local(&local)?;
        self.settle_imported().map_err(SyncError::Failed)?;
        let _ = std::fs::remove_file(&marker);
        tracing::info!(
            kind = "sync",
            outcome = "pulled",
            count = copied,
            "this machine was brought up to the remote copy"
        );
        Ok(SyncReport {
            generation: index.generation,
            copied,
            removed,
            bytes,
            files: index.files.len() as u64,
        })
    }
}
