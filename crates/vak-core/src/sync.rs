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
const LEASE: &str = "lease.json";

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
    #[error("another machine holds the work; this one is standing by. Take over to work here")]
    NotHolder,
    #[error(
        "another machine took the work over from this one. {0} files here were never pushed; pull to stand by again"
    )]
    Lost(usize),
    #[error(
        "the other machine has not handed over. Hand over there first, or take over by force if it is lost"
    )]
    HeldElsewhere,
    #[error("work is running; let it finish or stop it first")]
    Busy,
    #[error(
        "the keys were changed on the other machine; export a new key file there and import it here"
    )]
    KeysChanged,
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
    /// The tenant key version the pusher wraps under. A machine whose
    /// key file is older cannot read what was wrapped since.
    #[serde(default)]
    pub key_version: u32,
    pub files: BTreeMap<String, Entry>,
}

/// The remote's `lease.json`: which machine may write. One machine works
/// at a time; the other stands by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub holder: String,
    pub since: DateTime<Utc>,
    /// The holder handed over: nothing is running there, its last push
    /// is in, and the other machine may take over.
    #[serde(default)]
    pub released: bool,
}

/// Where a machine stands with its remote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// A remote is named and nothing was synced yet.
    #[default]
    Unset,
    /// This machine works and pushes.
    Holder,
    /// The other machine works; this one begins no turn.
    StandingBy,
    /// The other machine took over by force while this one held the
    /// work. It begins no turn until it pulls.
    Lost,
}

/// What this machine knows about its remote. Machine-local: it is never
/// pushed, and a home that was pulled or restored sets it up again.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Local {
    pub remote: PathBuf,
    /// This machine's id, minted at setup.
    pub machine: String,
    #[serde(default)]
    pub role: Role,
    /// The remote generation this machine last pushed or pulled.
    #[serde(default)]
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<DateTime<Utc>>,
    /// What this machine and the remote both held after that sync.
    #[serde(default)]
    pub synced: BTreeMap<String, Entry>,
    /// Automatic pushes that failed in a row, and when the last one was
    /// tried: what paces the next try.
    #[serde(default)]
    pub failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tried_at: Option<DateTime<Utc>>,
    /// Why the last push did not go through, in plain words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
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
    pub role: Role,
    /// Why the last push did not go through; it is tried again.
    pub last_error: Option<String>,
    /// Whether the machine that holds the work handed it over.
    pub released: bool,
    /// Whether another machine holds the work.
    pub held_elsewhere: bool,
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

    /// This machine's note, whether or not a remote is named in it.
    fn sync_note(&self) -> Option<Local> {
        serde_json::from_slice(&std::fs::read(self.sync_local_path()).ok()?).ok()
    }

    fn sync_local(&self) -> Result<Local, SyncError> {
        self.sync_note()
            .filter(|local| !local.remote.as_os_str().is_empty())
            .ok_or(SyncError::NotSetUp)
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
        for file in crate::state::backup_files(crate::state::Root::Data, &home).files {
            if let Ok(relative) = file.strip_prefix(&home)
                && travels(relative)
            {
                found.push((key_of(relative), file.clone()));
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

    fn read_lease(remote: &Path) -> Result<Option<Lease>, SyncError> {
        if !remote.is_dir() {
            return Err(SyncError::Unreachable);
        }
        match std::fs::read(remote.join(LEASE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(failed),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(SyncError::Unreachable),
        }
    }

    /// Writes the lease and reads it back: on a shared folder the last
    /// writer wins, so the machine that reads its own name holds it.
    fn write_lease(remote: &Path, machine: &str, released: bool) -> Result<(), SyncError> {
        let lease = Lease {
            holder: machine.to_string(),
            since: Utc::now(),
            released,
        };
        let bytes = serde_json::to_vec_pretty(&lease).map_err(failed)?;
        write_atomic(&remote.join(LEASE), &bytes).map_err(|_| SyncError::Unreachable)?;
        match Self::read_lease(remote)? {
            Some(read) if read.holder == machine => Ok(()),
            _ => Err(SyncError::NotHolder),
        }
    }

    fn unpushed(local: &Local, held: &BTreeMap<String, Entry>) -> usize {
        held.iter()
            .filter(|(key, entry)| local.synced.get(*key) != Some(entry))
            .count()
            + local
                .synced
                .keys()
                .filter(|key| !held.contains_key(*key))
                .count()
    }

    /// Refuses a new turn on a machine that is standing by, or that lost
    /// the work to the other machine: what it wrote would never reach the
    /// remote. Reads only this machine's own note of where it stands.
    pub(crate) fn refuse_standing_by(&self) -> Result<(), crate::CoreError> {
        match self.sync_local().map(|local| local.role) {
            Ok(Role::StandingBy) | Ok(Role::Lost) => Err(crate::CoreError::StandingBy),
            _ => Ok(()),
        }
    }

    /// Whether any run is open under a process that is still alive.
    fn work_in_flight(&self) -> bool {
        let runs = self.runs();
        let now = Utc::now();
        runs.list().is_ok_and(|records| {
            records.iter().any(|record| {
                record.is_open()
                    && record
                        .holder
                        .is_some_and(|holder| runs.is_alive(&holder, now).unwrap_or(true))
            })
        })
    }

    /// Whether this machine may begin work of its own accord: not while
    /// it stands by, and not once it has lost the work.
    pub fn sync_may_work(&self) -> bool {
        self.refuse_standing_by().is_ok()
    }

    /// The scheduler's push: when this machine holds the work, something
    /// changed since the last sync, and the last failure is old enough
    /// (a minute, doubling to about half an hour). A push that cannot
    /// reach the folder loses nothing; it is tried again. `None` when
    /// there was nothing to do.
    pub fn sync_auto(&self) -> Option<Result<SyncReport, SyncError>> {
        let mut local = self.sync_local().ok()?;
        if local.role != Role::Holder {
            return None;
        }
        if let Some(tried) = local.tried_at
            && local.failures > 0
        {
            let wait = chrono::Duration::seconds(60 << local.failures.min(5));
            if Utc::now() < tried + wait {
                return None;
            }
        }
        let (held, _) = self.sync_snapshot(&mut local).ok()?;
        if Self::unpushed(&local, &held) == 0 {
            return None;
        }
        let pushed = self.sync_push();
        // The push saved what it learned; add how the try went.
        let mut local = self.sync_local().ok()?;
        local.tried_at = Some(Utc::now());
        match &pushed {
            Ok(_) => {
                local.failures = 0;
                local.last_error = None;
            }
            Err(error) => {
                local.failures = local.failures.saturating_add(1);
                local.last_error = Some(error.to_string());
                tracing::warn!(
                    kind = "sync",
                    outcome = "failed",
                    count = local.failures,
                    "the remote copy could not be brought up to date; it will be tried again"
                );
            }
        }
        let _ = self.save_sync_local(&local);
        Some(pushed)
    }

    /// Hands the work over: refused while anything is running, then a
    /// last push, then the lease is released and this machine stands by.
    /// The other machine takes over from there.
    pub fn sync_handover(&self) -> Result<SyncReport, SyncError> {
        if self.work_in_flight() {
            return Err(SyncError::Busy);
        }
        let report = self.sync_push()?;
        let mut local = self.sync_local()?;
        Self::write_lease(&local.remote, &local.machine, true)?;
        local.role = Role::StandingBy;
        self.save_sync_local(&local)?;
        Ok(report)
    }

    /// Takes the work over: pulls what the remote holds and takes the
    /// lease. Refused until the other machine has handed over, unless
    /// `force`, which is for a machine that is lost: whatever it never
    /// pushed is not here, and it is fenced when it next reaches the
    /// remote. `None` when this machine already held the work.
    pub fn sync_takeover(
        &self,
        force: bool,
        discard: bool,
    ) -> Result<Option<SyncReport>, SyncError> {
        let local = self.sync_local()?;
        let lease = Self::read_lease(&local.remote)?;
        let mine = lease
            .as_ref()
            .is_none_or(|lease| lease.holder == local.machine);
        if mine && local.role == Role::Holder {
            return Ok(None);
        }
        if !mine && !force && !lease.as_ref().is_some_and(|lease| lease.released) {
            return Err(SyncError::HeldElsewhere);
        }
        let index = Self::read_index(&local.remote)?;
        let behind = index
            .as_ref()
            .is_some_and(|index| index.generation != local.generation);
        let report = if behind || local.role == Role::Lost {
            Some(self.sync_pull(discard)?)
        } else {
            None
        };
        let mut local = self.sync_local()?;
        Self::write_lease(&local.remote, &local.machine, false)?;
        local.role = Role::Holder;
        self.save_sync_local(&local)?;
        Ok(report.or(Some(SyncReport {
            generation: local.generation,
            copied: 0,
            removed: 0,
            bytes: 0,
            files: local.synced.len() as u64,
        })))
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
        let mut local = self.sync_note().unwrap_or_default();
        if local.machine.is_empty() {
            local.machine = format!("mch_{}", uuid::Uuid::now_v7());
        }
        if local.remote != remote {
            // The same copy at a new place (a drive mounted elsewhere)
            // changes nothing but where it is. Any other folder is a
            // remote this machine has not synced with.
            let moved = Self::read_index(&remote)
                .ok()
                .flatten()
                .is_some_and(|index| {
                    index.generation == local.generation && index.files == local.synced
                });
            local.remote = remote;
            if !moved {
                local.role = Role::Unset;
                local.generation = 0;
                local.synced_at = None;
                local.synced.clear();
            }
        }
        self.save_sync_local(&local)?;
        Ok(local)
    }

    /// Forgets the remote. Nothing in the folder is touched, and this
    /// machine keeps its id, so the lease it held there is still its own
    /// if the folder is set up again.
    pub fn sync_forget(&self) -> Result<(), SyncError> {
        let Some(note) = self.sync_note() else {
            return Ok(());
        };
        self.save_sync_local(&Local {
            machine: note.machine,
            ..Local::default()
        })
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
        let lease = Self::read_lease(&local.remote).ok().flatten();
        let unpushed = if local.role == Role::StandingBy {
            0
        } else {
            unpushed
        };
        Ok(SyncStatus {
            role: local.role,
            last_error: local.last_error.clone(),
            released: lease.as_ref().is_some_and(|lease| lease.released),
            held_elsewhere: lease
                .as_ref()
                .is_some_and(|lease| lease.holder != local.machine),
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
        match Self::read_lease(&local.remote)? {
            Some(lease) if lease.holder != local.machine => {
                // The other machine holds the work. A machine that
                // thought it did has lost it, and says what it kept.
                let lost = local.role == Role::Holder || local.role == Role::Lost;
                local.role = if lost { Role::Lost } else { Role::StandingBy };
                let kept = Self::unpushed(&local, &held);
                self.save_sync_local(&local)?;
                return Err(if lost {
                    SyncError::Lost(kept)
                } else {
                    SyncError::NotHolder
                });
            }
            Some(_) if local.role == Role::StandingBy => return Err(SyncError::NotHolder),
            Some(_) => {}
            None => Self::write_lease(&local.remote, &local.machine, false)?,
        }
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
            key_version: tenant.key_versions().map_err(failed)?.0,
            files: held,
        };
        let encoded = serde_json::to_vec_pretty(&index).map_err(failed)?;
        // Look once more before the push counts: if the other machine took
        // the work or pushed while the files were being sent, this push
        // must not land on top of it.
        let still_mine =
            Self::read_lease(&local.remote)?.is_some_and(|lease| lease.holder == local.machine);
        let unmoved = Self::read_index(&local.remote)?.map_or(0, |now| now.generation) + 1
            == index.generation;
        if !still_mine || !unmoved {
            local.role = Role::Lost;
            let kept = Self::unpushed(&local, &index.files);
            self.save_sync_local(&local)?;
            return Err(SyncError::Lost(kept));
        }
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
        local.role = Role::Holder;
        local.failures = 0;
        local.last_error = None;
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
        let tenant = self.tenant_objects().map_err(failed)?;
        if index.key_version > tenant.key_versions().map_err(failed)?.0 {
            return Err(SyncError::KeysChanged);
        }
        let (held, _) = self.sync_snapshot(&mut local)?;
        let marker = self.shared_scope().sync().join(PULLING);

        let mine = held
            .iter()
            .filter(|(key, entry)| local.synced.get(*key) != Some(entry))
            .count();
        let fresh = local.synced.is_empty() && tenant.holds_nothing();
        // A machine that was standing by did no work: what it wrote since
        // (its own bookkeeping) is not work to keep.
        let standing_by = local.role == Role::StandingBy;
        if mine > 0 && !fresh && !discard && !standing_by && !marker.exists() {
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
        let mut past = index.epoch;
        if let Some(refs) = refs {
            past = past.max(tenant.import_refs(&refs).map_err(failed)?);
        }
        local.generation = index.generation;
        local.synced_at = Some(Utc::now());
        local.synced = index.files.clone();
        local.role = Role::StandingBy;
        local.seen.clear();
        self.save_sync_local(&local)?;
        self.settle_imported(past).map_err(SyncError::Failed)?;
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
