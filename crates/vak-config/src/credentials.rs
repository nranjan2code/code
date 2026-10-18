//! Cross-platform secret storage (docs/design/44-shared-config.md,
//! "Secrets Chain"). Replaces plaintext `.env` files as the persisted
//! layer of the secrets chain:
//!
//! ```text
//! process environment  >  project secret  >  shared secret  >  operator prompt
//! ```
//!
//! `process environment` is handled by callers directly (`std::env::var`)
//! and never touches this module. Everything this module stores is a
//! `(scope, var)` pair, where `scope` is derived from the directory the
//! caller used to identify a project/shared/agent layer (previously the
//! parent of a literal `.env` path) and `var` is the credential's name
//! (`ANTHROPIC_API_KEY`, an MCP server's referenced variable, etc).
//!
//! Backend selection is automatic, not configured: an OS-native secret
//! service (macOS Keychain / Windows Credential Manager / Linux Secret
//! Service) is used when reachable; otherwise secrets fall back to an
//! encrypted-at-rest file under the shared data home. There is no
//! plaintext persisted backend — headless environments get the encrypted
//! file, not a `.env` file, as a first-class choice rather than a
//! degraded one.

use std::io;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One secret backend. `scope` is an opaque, stable identifier for a
/// project/shared/agent layer — never a filesystem path passed straight
/// through, so a backend is free to store scopes any way it likes.
pub trait CredentialStore: Send + Sync {
    fn get(&self, scope: &str, var: &str) -> Option<String>;
    /// Fallible point lookup for callers that must distinguish a missing
    /// credential from an unreadable or corrupt store.
    fn try_get(&self, scope: &str, var: &str) -> io::Result<Option<String>> {
        Ok(self.get(scope, var))
    }
    fn set(&self, scope: &str, var: &str, value: &str) -> io::Result<()>;
    fn remove(&self, scope: &str, var: &str) -> io::Result<()>;
    /// Best-effort enumeration of everything stored for `scope`, used only
    /// to seed the legacy bulk process-env cache (`load_env_file`,
    /// `replace_env_files`) at startup for consumers that read env vars
    /// directly rather than asking this module by name. OS-native secret
    /// services have no portable "list everything for this service" API,
    /// so that backend returns an empty list here — its point lookups via
    /// `get` are unaffected. Only the encrypted-file backend can answer
    /// this fully.
    fn list(&self, scope: &str) -> Vec<(String, String)>;
    /// Fallible enumeration for callers that must not treat store damage as
    /// an empty scope.
    fn try_list(&self, scope: &str) -> io::Result<Vec<(String, String)>> {
        Ok(self.list(scope))
    }
}

fn fnv16(text: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// What a scope hint's directory belongs to.
enum Owner {
    /// Somewhere in the data home: the home itself, an Agent home, a
    /// tenant's key vault, or another of its folders. Never a project.
    Home(String),
    /// A project folder, filed under its space.
    Space(std::path::PathBuf),
}

fn owner_of(hint_path: &Path) -> Owner {
    let dir = hint_path.parent().unwrap_or(hint_path);
    let forms = |path: &Path| {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        [path.to_path_buf(), canonical]
    };
    let data = crate::paths::data_home();
    // The data home's identity is its location: two homes on one machine
    // (a `VAK_HOME` home beside the default) never share an OS keychain
    // entry.
    let home = format!(
        "home-{}",
        fnv16(
            &data
                .canonicalize()
                .unwrap_or_else(|_| data.clone())
                .to_string_lossy()
        )
    );
    for data in forms(&data) {
        for dir in forms(dir) {
            let Ok(rest) = dir.strip_prefix(&data) else {
                continue;
            };
            let parts: Vec<String> = rest
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            return Owner::Home(match parts.as_slice() {
                [] => home,
                [agents, agent] if agents == "agents" => format!("{home}-agent-{agent}"),
                [tenants, tenant, keys] if tenants == "tenants" && keys == "keys" => {
                    format!("{home}-tenant-{tenant}-keys")
                }
                _ => format!("{home}-{}", fnv16(&parts.join("/"))),
            });
        }
    }
    Owner::Space(dir.to_path_buf())
}

/// [`scope_key_for`] for a write: storing a secret for a project folder is
/// an act about it as a workspace, so the folder is bound to its space first
/// and the secret is never filed under an `unbound-` key it would leave
/// behind. A folder in the data home is never bound.
fn bound_scope_key_for(hint_path: &Path) -> io::Result<String> {
    if let Owner::Space(dir) = owner_of(hint_path) {
        crate::spaces::bind(&dir).map_err(io::Error::other)?;
    }
    Ok(scope_key_for(hint_path))
}

/// The secret scope named by the directory a caller used for a secret layer
/// (the parent of what was once a literal `<dir>/.env`), keyed by owner
/// rather than by path (review R12): a scope in the data home is named for
/// that home and its owner (`home-<id>`, `home-<id>-agent-<agent>`,
/// `home-<id>-tenant-<t>-keys`), and a project folder is the space it is
/// bound to, `space-<spc_…>`.
fn scope_key_for(hint_path: &Path) -> String {
    match owner_of(hint_path) {
        Owner::Home(key) => key,
        Owner::Space(dir) => format!("space-{}", crate::spaces::key(&dir)),
    }
}

fn store() -> &'static dyn CredentialStore {
    if crate::paths::home_is_isolated_for_tests() {
        static ISOLATED_STORE: EncryptedFileStore = EncryptedFileStore { root: None };
        return &ISOLATED_STORE;
    }
    static STORE: OnceLock<Box<dyn CredentialStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            if os_keyring_reachable_cached() {
                Box::new(OsKeyringStore) as Box<dyn CredentialStore>
            } else {
                Box::new(EncryptedFileStore::default()) as Box<dyn CredentialStore>
            }
        })
        .as_ref()
}

/// The probe's answer, asked once in a process.
fn os_keyring_reachable_cached() -> bool {
    static REACHABLE: OnceLock<bool> = OnceLock::new();
    *REACHABLE.get_or_init(os_keyring_reachable_with_timeout)
}

/// A brand-new keychain item can trigger a native OS permission dialog
/// (macOS: "vak wants to use your confidential information stored in
/// keychain"). In any context with no one present to click it — headless,
/// sandboxed, CI, an install running non-interactively — that dialog never
/// resolves, and an unbounded probe hangs the whole process forever. This
/// runs the real probe on a background thread and gives up after a short
/// timeout, falling back to the encrypted-file backend exactly as if the
/// keychain were unreachable. A slow-but-eventually-successful keychain
/// does not get a second chance this process run — that trade is
/// deliberate: a store that sometimes takes 10s to answer "is a key set"
/// is worse than always using the encrypted file on that host.
fn os_keyring_reachable_with_timeout() -> bool {
    const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(os_keyring_reachable());
    });
    rx.recv_timeout(PROBE_TIMEOUT).unwrap_or(false)
}

/// One-time, cheap round-trip probe: can we actually reach a working OS
/// secret service on this host? Headless Linux (no D-Bus session, no
/// secrets daemon running) is the common case where this fails — that is
/// the expected, first-class path to the encrypted-file backend below, not
/// an error condition. A sandboxed process is another: some sandbox
/// profiles let a keychain write return success at the API layer without
/// actually persisting anything retrievable, so this checks the *value*
/// round-trips, not just that each call returned `Ok`. Uses the same
/// service-name shape production traffic uses (`service_name(scope)`), so
/// the probe exercises the exact access path being decided on rather than
/// a special-cased one that could be allowed differently. Only ever called
/// with a hard timeout above — never call this directly.
fn os_keyring_reachable() -> bool {
    let scope = "__vak_probe__";
    let var = "__vak_probe__";
    let probe_value = "probe";
    let Ok(writer) = keyring::Entry::new(&service_name(scope), var) else {
        return false;
    };
    if writer.set_password(probe_value).is_err() {
        return false;
    }
    // A fresh `Entry` for the read half: some sandbox profiles let a
    // write appear to succeed while only an in-process handle (not the
    // actual daemon-backed item) would echo it back, which reusing the
    // same `Entry` value could mask.
    let roundtripped = keyring::Entry::new(&service_name(scope), var)
        .and_then(|reader| reader.get_password())
        .ok()
        .as_deref()
        == Some(probe_value);
    let _ = writer.delete_credential();
    roundtripped
}

pub fn get(scope_hint: &Path, var: &str) -> Option<String> {
    try_get(scope_hint, var).ok().flatten()
}

pub fn try_get(scope_hint: &Path, var: &str) -> io::Result<Option<String>> {
    store().try_get(&scope_key_for(scope_hint), var)
}

pub fn set(scope_hint: &Path, var: &str, value: &str) -> io::Result<()> {
    validate_index_component(var)?;
    let scope = bound_scope_key_for(scope_hint)?;
    with_index_lock(|index| {
        index_mark_present(index, &scope, var)?;
        store().set(&scope, var, value)
    })
}

pub fn remove(scope_hint: &Path, var: &str) -> io::Result<()> {
    validate_index_component(var)?;
    let scope = bound_scope_key_for(scope_hint)?;
    with_index_lock(|index| {
        store().remove(&scope, var)?;
        index_mark_absent(index, &scope, var)
    })
}

/// Which store holds the secrets on this machine: `keychain` (the OS
/// secret service) or `encrypted_file` (the fallback in the data home).
pub fn backend() -> &'static str {
    if crate::paths::home_is_isolated_for_tests() || !os_keyring_reachable_cached() {
        "encrypted_file"
    } else {
        "keychain"
    }
}

/// Removes every stored secret, in every scope: the erasure of the whole
/// install (data-architecture plan M7b-f). The presence index names them,
/// because the OS store cannot be listed. The report counts removed and
/// retained entries; index, lock, or store failures are never reported as an
/// empty cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgetReport {
    pub removed: usize,
    pub remaining: usize,
}

pub fn forget_all() -> io::Result<ForgetReport> {
    with_index_lock(|index| {
        let entries = index_entries(index)?;
        let (removed, retained) =
            remove_indexed_entries(&entries, |scope, var| store().remove(scope, var).is_ok());
        index_write(index, &retained)?;
        Ok(ForgetReport {
            removed,
            remaining: retained.len(),
        })
    })
}

fn remove_indexed_entries(
    entries: &[String],
    mut remove: impl FnMut(&str, &str) -> bool,
) -> (usize, Vec<String>) {
    let mut removed = 0;
    let mut retained = Vec::new();
    for entry in entries {
        let Some((scope, var)) = entry.split_once('\0') else {
            // `index_entries` already rejects malformed rows. Keep this
            // defensive branch conservative if this helper gains a caller.
            retained.push(entry.clone());
            continue;
        };
        if remove(scope, var) {
            removed += 1;
        } else {
            retained.push(entry.clone());
        }
    }
    (removed, retained)
}

fn validate_index_component(value: &str) -> io::Result<()> {
    if value.is_empty() || value.chars().any(|ch| matches!(ch, '\0' | '\r' | '\n')) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "credential variable name cannot be represented in the presence index",
        ));
    }
    Ok(())
}

pub fn list(scope_hint: &Path) -> Vec<(String, String)> {
    store().list(&scope_key_for(scope_hint))
}

pub fn try_list(scope_hint: &Path) -> io::Result<Vec<(String, String)>> {
    store().try_list(&scope_key_for(scope_hint))
}

/// Whether *any* credential is stored for this scope — used for trust
/// decisions (e.g. "does this project have a secret that makes it
/// privileged") that a literal `.env`'s existence used to answer.
///
/// The OS-native backend has no portable way to enumerate what it holds
/// (see `CredentialStore::list`), so this is answered by a small
/// non-secret presence index maintained alongside every `set`/`remove`
/// call here — it records only `(scope, var)` pairs that exist, never
/// values, so it carries none of the sensitivity a secret-listing API
/// would.
pub fn scope_has_any(scope_hint: &Path) -> bool {
    scope_has_any_conservative(try_scope_has_any(scope_hint))
}

/// Fallible form used by callers that need to distinguish an empty scope
/// from an unreadable or corrupt presence index. Trust decisions should
/// treat errors conservatively as privileged.
pub fn try_scope_has_any(scope_hint: &Path) -> io::Result<bool> {
    let scope = scope_key_for(scope_hint);
    with_index_lock(|index| scope_has_any_in(index, &scope))
}

fn index_path() -> PathBuf {
    crate::paths::credential_index()
}

/// Lock a stable sibling inode before opening the replaceable index. Every
/// read and update goes through this helper, including backend writes whose
/// presence marker must be visible before the secret can become visible.
fn with_index_lock<T>(f: impl FnOnce(&mut std::fs::File) -> io::Result<T>) -> io::Result<T> {
    with_index_lock_at(&index_path(), &crate::paths::credential_index_lock(), f)
}

fn with_index_lock_at<T>(
    path: &Path,
    lock_path: &Path,
    f: impl FnOnce(&mut std::fs::File) -> io::Result<T>,
) -> io::Result<T> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let lock = open_private_file(lock_path, true)?;
    lock.lock()?;
    let mut file = open_private_file(path, true)?;
    f(&mut file)
}

fn open_private_file(path: &Path, create: bool) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    if create {
        options.create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
    }
    options.open(path)
}

fn index_entries(file: &mut std::fs::File) -> io::Result<Vec<String>> {
    file.seek(SeekFrom::Start(0))?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in text.lines() {
        let Some((scope, var)) = entry.split_once('\0') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "credential presence index is malformed",
            ));
        };
        if scope.is_empty() || var.is_empty() || var.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "credential presence index is malformed",
            ));
        }
        entries.push(entry.to_string());
    }
    Ok(entries)
}

fn scope_has_any_in(file: &mut std::fs::File, scope: &str) -> io::Result<bool> {
    let prefix = format!("{scope}\0");
    Ok(index_entries(file)?
        .iter()
        .any(|entry| entry.starts_with(&prefix)))
}

fn scope_has_any_conservative(result: io::Result<bool>) -> bool {
    result.unwrap_or(true)
}

fn index_write(file: &mut std::fs::File, entries: &[String]) -> io::Result<()> {
    index_write_at(file, &index_path(), entries)
}

fn index_write_at(file: &mut std::fs::File, path: &Path, entries: &[String]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("credential index has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let tmp = unique_temporary_path(path);
    write_private_synced(&tmp, entries.join("\n").as_bytes())?;
    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    sync_parent(parent)?;
    *file = open_private_file(path, false)?;
    Ok(())
}

fn index_mark_present(file: &mut std::fs::File, scope: &str, var: &str) -> io::Result<()> {
    let key = format!("{scope}\0{var}");
    let mut entries = index_entries(file)?;
    if !entries.contains(&key) {
        entries.push(key);
        index_write(file, &entries)?;
    }
    Ok(())
}

fn index_mark_absent(file: &mut std::fs::File, scope: &str, var: &str) -> io::Result<()> {
    let key = format!("{scope}\0{var}");
    let mut entries = index_entries(file)?;
    let before = entries.len();
    entries.retain(|e| e != &key);
    if entries.len() != before {
        index_write(file, &entries)?;
    }
    Ok(())
}

fn unique_temporary_path(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let suffix = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("credential"),
        std::process::id(),
        suffix
    ))
}

fn sync_parent(parent: &Path) -> io::Result<()> {
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn write_private_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn publish_key(path: &Path, key: &[u8; 32]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("credential key has no parent directory"))?;
    let tmp = unique_temporary_path(path);
    write_private_synced(&tmp, key)?;
    let published = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    published?;
    sync_parent(parent)
}

// ---- OS-native backend -----------------------------------------------

struct OsKeyringStore;

fn service_name(scope: &str) -> String {
    format!("vak/{scope}")
}

fn keyring_err(err: keyring::Error) -> io::Error {
    io::Error::other(err.to_string())
}

impl CredentialStore for OsKeyringStore {
    fn get(&self, scope: &str, var: &str) -> Option<String> {
        self.try_get(scope, var).ok().flatten()
    }

    fn try_get(&self, scope: &str, var: &str) -> io::Result<Option<String>> {
        let entry = keyring::Entry::new(&service_name(scope), var).map_err(keyring_err)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_err(error)),
        }
    }

    fn set(&self, scope: &str, var: &str, value: &str) -> io::Result<()> {
        keyring::Entry::new(&service_name(scope), var)
            .map_err(keyring_err)?
            .set_password(value)
            .map_err(keyring_err)
    }

    fn remove(&self, scope: &str, var: &str) -> io::Result<()> {
        match keyring::Entry::new(&service_name(scope), var) {
            Ok(entry) => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(keyring_err(err)),
            },
            Err(err) => Err(keyring_err(err)),
        }
    }

    fn list(&self, _scope: &str) -> Vec<(String, String)> {
        // No portable "list all secrets for this service" API across
        // Keychain/Credential Manager/Secret Service — point lookups via
        // `get` are the supported access pattern for this backend.
        Vec::new()
    }
}

// ---- Encrypted-file fallback ------------------------------------------
//
// Used whenever no OS secret service is reachable (the common case for
// headless Linux: servers, containers, CI). One AES-256-GCM-encrypted file
// holds every scope's secrets; the key lives in a separate 0600 file next
// to it. This is deliberately a first-class backend, not a degraded
// stand-in for `.env` — a stolen/backed-up copy of the data file alone
// reveals nothing without the key file.

/// Deliberately holds no cached paths. This backend lives inside a
/// `OnceLock`-cached trait object (see `store()`) that is only ever built
/// once per process — if paths were resolved once at construction and
/// cached on the struct, any later change to `VAK_HOME`/the home override
/// (a test calling `isolate_home_for_tests()` after some earlier test
/// already triggered first use, or — in principle — a production process
/// changing its home override at runtime) would be silently ignored for
/// the rest of the process. Every call below re-resolves `home()` fresh
/// instead; this is a handful of path joins and env lookups, not I/O, so
/// there is no real cost to paying it on every access.
#[derive(Default)]
struct EncryptedFileStore {
    /// Test-only explicit root; production always resolves the canonical home.
    root: Option<PathBuf>,
}

impl EncryptedFileStore {
    fn home(&self) -> PathBuf {
        // The data home, never `default_workspace()`: that directory is
        // the built-in Agent's workspace, so a file there (and the key
        // beside it) is reachable by the workspace tools (invariant 10).
        self.root.clone().unwrap_or_else(crate::paths::data_home)
    }

    fn data_path(&self) -> PathBuf {
        self.home().join("credentials.enc")
    }

    fn key_path(&self) -> PathBuf {
        self.home().join(".credential_key")
    }

    fn lock_path(&self) -> PathBuf {
        self.home().join(".credential_key.lock")
    }

    /// Exclusive, cross-process advisory lock spanning key
    /// generation/read plus the whole load-modify-save cycle. Without
    /// it, two concurrent first-time callers can each generate a
    /// different random key and each believe theirs is the one on disk,
    /// or two concurrent writers can interleave a read-modify-write and
    /// silently drop one write. The lock file is separate from the key
    /// file so a reader never needs write access to the key itself.
    fn with_lock<T>(&self, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
        let lock_path = self.lock_path();
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = open_private_file(&lock_path, true)?;
        lock.lock()?;
        f()
    }

    fn key(&self) -> io::Result<[u8; 32]> {
        let key_path = self.key_path();
        match std::fs::read(&key_path) {
            Ok(bytes) if bytes.len() == 32 => {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                return Ok(key);
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "credential key is malformed",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        match std::fs::metadata(self.data_path()) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "credential key is missing while encrypted credentials exist",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let rng = ring::rand::SystemRandom::new();
        let mut key = [0u8; 32];
        ring::rand::SecureRandom::fill(&rng, &mut key)
            .map_err(|_| io::Error::other("failed to generate credential key"))?;
        let parent = key_path
            .parent()
            .ok_or_else(|| io::Error::other("credential key has no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        publish_key(&key_path, &key)?;
        Ok(key)
    }

    /// An absent ciphertext file means an empty store. Once ciphertext
    /// exists, every key, authentication, UTF-8, and row error is fatal so a
    /// mutation cannot replace unknown credentials with an empty snapshot.
    fn load_all(&self) -> io::Result<Vec<(String, String, String)>> {
        let bytes = match std::fs::read(self.data_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let key = self.key()?;
        let plaintext = decrypt(&key, &bytes)?;
        let plaintext = String::from_utf8(plaintext).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "credential store is not valid UTF-8",
            )
        })?;
        if plaintext.is_empty() {
            return Ok(Vec::new());
        }
        if plaintext.ends_with('\n') || plaintext.contains('\r') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "credential store row is malformed",
            ));
        }
        let mut entries = Vec::new();
        for line in plaintext.split('\n') {
            if line.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "credential store row is malformed",
                ));
            }
            let mut fields = line.split('\0');
            let scope = fields.next().unwrap_or_default();
            let var = fields.next().unwrap_or_default();
            let value = fields.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "credential store row is malformed",
                )
            })?;
            if fields.next().is_some() || scope.is_empty() || var.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "credential store row is malformed",
                ));
            }
            validate_index_component(var)?;
            if entries
                .iter()
                .any(|(known_scope, known_var, _): &(String, String, String)| {
                    known_scope == scope && known_var == var
                })
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "credential store contains duplicate entries",
                ));
            }
            entries.push((scope.to_string(), var.to_string(), value.to_string()));
        }
        Ok(entries)
    }

    fn save_all(&self, entries: &[(String, String, String)]) -> io::Result<()> {
        let key = self.key()?;
        for (scope, var, value) in entries {
            if scope.is_empty()
                || scope.chars().any(|ch| matches!(ch, '\0' | '\r' | '\n'))
                || value.chars().any(|ch| matches!(ch, '\0' | '\r' | '\n'))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "credential entry cannot be represented in the encrypted store",
                ));
            }
            validate_index_component(var)?;
        }
        let plaintext = entries
            .iter()
            .map(|(scope, var, value)| format!("{scope}\0{var}\0{value}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ciphertext = encrypt(&key, plaintext.as_bytes())
            .map_err(|_| io::Error::other("failed to encrypt credential store"))?;
        let data_path = self.data_path();
        let parent = data_path
            .parent()
            .ok_or_else(|| io::Error::other("credential store has no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        let tmp = unique_temporary_path(&data_path);
        write_private_synced(&tmp, &ciphertext)?;
        if let Err(error) = std::fs::rename(&tmp, &data_path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(error);
        }
        sync_parent(parent)
    }
}

impl CredentialStore for EncryptedFileStore {
    fn get(&self, scope: &str, var: &str) -> Option<String> {
        self.try_get(scope, var).ok().flatten()
    }

    fn try_get(&self, scope: &str, var: &str) -> io::Result<Option<String>> {
        self.with_lock(|| {
            Ok(self
                .load_all()?
                .into_iter()
                .find(|(s, v, _)| s == scope && v == var)
                .map(|(_, _, value)| value))
        })
    }

    fn set(&self, scope: &str, var: &str, value: &str) -> io::Result<()> {
        self.with_lock(|| {
            let mut entries = self.load_all()?;
            entries.retain(|(s, v, _)| !(s == scope && v == var));
            entries.push((scope.to_string(), var.to_string(), value.to_string()));
            self.save_all(&entries)
        })
    }

    fn remove(&self, scope: &str, var: &str) -> io::Result<()> {
        self.with_lock(|| {
            let mut entries = self.load_all()?;
            let before = entries.len();
            entries.retain(|(s, v, _)| !(s == scope && v == var));
            if entries.len() == before {
                return Ok(());
            }
            self.save_all(&entries)
        })
    }

    fn list(&self, scope: &str) -> Vec<(String, String)> {
        self.try_list(scope).unwrap_or_default()
    }

    fn try_list(&self, scope: &str) -> io::Result<Vec<(String, String)>> {
        self.with_lock(|| {
            Ok(self
                .load_all()?
                .into_iter()
                .filter(|(s, _, _)| s == scope)
                .map(|(_, var, value)| (var, value))
                .collect())
        })
    }
}

fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, ring::error::Unspecified> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
    let rng = ring::rand::SystemRandom::new();
    let mut nonce_bytes = [0u8; NONCE_LEN];
    ring::rand::SecureRandom::fill(&rng, &mut nonce_bytes)?;
    let unbound = UnboundKey::new(&AES_256_GCM, key)?;
    let sealing_key = LessSafeKey::new(unbound);
    let mut in_out = plaintext.to_vec();
    sealing_key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce_bytes),
        Aad::empty(),
        &mut in_out,
    )?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&in_out);
    Ok(out)
}

fn decrypt(key: &[u8; 32], ciphertext: &[u8]) -> io::Result<Vec<u8>> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
    if ciphertext.len() < NONCE_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "credential ciphertext is truncated",
        ));
    }
    let (nonce_bytes, sealed) = ciphertext.split_at(NONCE_LEN);
    let unbound = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "credential key is invalid"))?;
    let opening_key = LessSafeKey::new(unbound);
    let mut in_out = sealed.to_vec();
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "credential nonce is invalid"))?;
    let plaintext = opening_key
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "credential ciphertext is invalid",
            )
        })?;
    Ok(plaintext.to_vec())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Production `EncryptedFileStore` holds no per-instance paths — every
    /// access re-resolves `data_home()` fresh (see the type's doc
    /// comment: a `OnceLock`-cached instance must never bake in a path
    /// from whenever it happened to be first constructed). Testing it
    /// therefore means pointing the real global home somewhere private.
    ///
    /// This uses `paths::isolate_home_for_tests()` rather than its own
    /// `set_home_override` call: that override is a single value shared
    /// by every thread in this test binary, and another test
    /// (`vak_config::lib::tests`, around line 5054) also calls
    /// `isolate_home_for_tests()` — under `cargo test`'s default
    /// parallelism, two DIFFERENT override values racing on the same
    /// global map is exactly the corruption this store's own
    /// cross-process file lock cannot help with (the lock protects the
    /// file, not which file the override points the whole process at).
    /// `isolate_home_for_tests()` is the one mechanism every such test
    /// already cooperates through — confirmed reproducible without it.
    #[test]
    fn encrypted_file_store_roundtrips_and_scopes_correctly() {
        let home = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore {
            root: Some(home.path().to_path_buf()),
        };
        // Scope/var names unique to this test so it can safely share the
        // process-wide isolated home with sibling tests.
        store
            .set("credentials-rs-test-scope-a", "ANTHROPIC_API_KEY", "key-a")
            .unwrap();
        store
            .set("credentials-rs-test-scope-b", "ANTHROPIC_API_KEY", "key-b")
            .unwrap();
        let key_path = store.key_path();
        assert_eq!(std::fs::read(&key_path).unwrap().len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            store.get("credentials-rs-test-scope-a", "ANTHROPIC_API_KEY"),
            Some("key-a".to_string())
        );
        assert_eq!(
            store.get("credentials-rs-test-scope-b", "ANTHROPIC_API_KEY"),
            Some("key-b".to_string())
        );
        assert_eq!(store.get("credentials-rs-test-scope-a", "OTHER_KEY"), None);

        store
            .remove("credentials-rs-test-scope-a", "ANTHROPIC_API_KEY")
            .unwrap();
        assert_eq!(
            store.get("credentials-rs-test-scope-a", "ANTHROPIC_API_KEY"),
            None
        );
        assert_eq!(
            store.get("credentials-rs-test-scope-b", "ANTHROPIC_API_KEY"),
            Some("key-b".to_string())
        );

        // The file on disk must not contain the plaintext secret.
        let raw = std::fs::read(store.data_path()).unwrap();
        assert!(!raw.windows(5).any(|w| w == b"key-b"));

        store
            .remove("credentials-rs-test-scope-b", "ANTHROPIC_API_KEY")
            .unwrap();
    }

    #[test]
    fn corrupt_ciphertext_or_key_makes_mutations_fail_without_replacement() {
        let home = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore {
            root: Some(home.path().to_path_buf()),
        };
        store.set("scope", "TOKEN", "secret-value").unwrap();
        let data = store.data_path();
        let good = std::fs::read(&data).unwrap();

        std::fs::write(&data, b"truncated ciphertext").unwrap();
        assert!(store.try_get("scope", "TOKEN").is_err());
        assert!(store.try_list("scope").is_err());
        assert!(store.set("scope", "OTHER", "must-not-write").is_err());
        assert!(store.remove("scope", "TOKEN").is_err());
        assert_eq!(std::fs::read(&data).unwrap(), b"truncated ciphertext");

        let key_bytes = store.key().unwrap();
        let malformed = encrypt(&key_bytes, b"scope\0TOKEN").unwrap();
        std::fs::write(&data, &malformed).unwrap();
        assert!(store.set("scope", "OTHER", "must-not-write").is_err());
        assert_eq!(std::fs::read(&data).unwrap(), malformed);

        std::fs::write(&data, &good).unwrap();
        let key = store.key_path();
        let original_key = std::fs::read(&key).unwrap();
        std::fs::write(&key, b"bad key").unwrap();
        assert!(store.set("scope", "OTHER", "must-not-write").is_err());
        assert!(store.remove("scope", "TOKEN").is_err());
        assert_eq!(std::fs::read(&data).unwrap(), good);
        assert_eq!(std::fs::read(&key).unwrap(), b"bad key");

        std::fs::write(&key, original_key).unwrap();
        std::fs::remove_file(key).unwrap();
        assert!(store.set("scope", "OTHER", "must-not-write").is_err());
        assert!(std::fs::read(&data).unwrap() == good);
    }

    #[test]
    fn failed_store_lock_never_runs_a_mutation() {
        let home = tempfile::tempdir().unwrap();
        let store = EncryptedFileStore {
            root: Some(home.path().to_path_buf()),
        };
        std::fs::create_dir(store.lock_path()).unwrap();
        assert!(store.set("scope", "TOKEN", "value").is_err());
        assert!(!store.data_path().exists());
    }

    #[test]
    fn index_replacement_keeps_the_locked_handle_current_and_lock_failures_stop() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("credential_index.json");
        let lock = home.path().join(".credential_index.lock");
        with_index_lock_at(&path, &lock, |index| {
            index_write_at(index, &path, &["scope\0TOKEN".into()])?;
            assert_eq!(index_entries(index)?, vec!["scope\0TOKEN"]);
            Ok(())
        })
        .unwrap();
        let mut reopened = open_private_file(&path, false).unwrap();
        assert_eq!(index_entries(&mut reopened).unwrap(), vec!["scope\0TOKEN"]);

        let bad_lock = home.path().join("lock-is-a-directory");
        std::fs::create_dir(&bad_lock).unwrap();
        let mut ran = false;
        assert!(
            with_index_lock_at(&path, &bad_lock, |_| {
                ran = true;
                Ok(())
            })
            .is_err()
        );
        assert!(!ran);
    }

    /// Exit test of M3b slice 5 (review R12): a secret scope is named by
    /// its owner's id, never by a path, so every spelling of one folder
    /// reaches the same secrets and the key reveals no path.
    #[test]
    fn secret_scopes_keyed_by_id() {
        crate::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let space = crate::spaces::bind(dir.path()).unwrap();
        set(&dir.path().join(".env"), "SCOPE_TEST_KEY", "one").unwrap();
        assert_eq!(
            get(&canonical.join(".env"), "SCOPE_TEST_KEY").as_deref(),
            Some("one")
        );
        let key = scope_key_for(&dir.path().join(".env"));
        assert_eq!(key, format!("space-{space}"));
        assert!(!key.contains('/'), "a scope key names an id, never a path");
        remove(&dir.path().join(".env"), "SCOPE_TEST_KEY").unwrap();
    }

    #[test]
    fn scope_key_for_is_stable_and_direction_specific() {
        crate::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.env");
        let b = dir.path().join("b.env");
        // Same parent directory -> same scope, matching old same-file behavior.
        assert_eq!(scope_key_for(&a), scope_key_for(&b));
        assert!(scope_key_for(&a).starts_with("space-unbound-"));
        let space = crate::spaces::bind(dir.path()).unwrap();
        assert_eq!(scope_key_for(&a), format!("space-{space}"));
        let agent = crate::paths::agent_home("mira").join(".env");
        assert!(scope_key_for(&agent).ends_with("-agent-mira"));
        let home = scope_key_for(&crate::paths::data_home().join(".env"));
        assert!(home.starts_with("home-") && !home.contains('/'));
        let vault = crate::paths::local_tenant_home().join("keys/vault");
        assert!(
            scope_key_for(&vault)
                .ends_with(&format!("-tenant-{}-keys", crate::paths::LOCAL_TENANT))
        );
        let _ = set(&vault, "KEY_VAULT_PROBE", "x");
        assert!(
            crate::spaces::all()
                .iter()
                .all(|space| space.folder.as_deref() != vault.parent()),
            "the key vault's folder is never bound as a project"
        );
    }

    #[test]
    fn presence_index_updates_and_readers_share_one_lock() {
        crate::paths::isolate_home_for_tests();
        let writers = 8;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(writers + 4));
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut threads = Vec::new();
        for writer in 0..writers {
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for item in 0..12 {
                    with_index_lock(|index| {
                        index_mark_present(index, "index-contention", &format!("{writer}-{item}"))
                    })
                    .unwrap();
                }
            }));
        }
        for _ in 0..4 {
            let barrier = barrier.clone();
            let done = done.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                while !done.load(std::sync::atomic::Ordering::Acquire) {
                    with_index_lock(index_entries).unwrap();
                }
            }));
        }
        // Writers and readers start together; only signal completion after
        // every writer has finished its last read-modify-write.
        let readers = threads.split_off(writers);
        for writer in threads {
            writer.join().unwrap();
        }
        done.store(true, std::sync::atomic::Ordering::Release);
        for reader in readers {
            reader.join().unwrap();
        }

        with_index_lock(|index| {
            let entries = index_entries(index)?;
            for writer in 0..writers {
                for item in 0..12 {
                    assert!(entries.contains(&format!("index-contention\0{writer}-{item}")));
                }
            }
            for writer in 0..writers {
                for item in 0..12 {
                    index_mark_absent(index, "index-contention", &format!("{writer}-{item}"))?;
                }
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn corrupt_presence_index_is_conservative_for_trust() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credential_index.json");
        std::fs::write(&path, "not-a-scope-var-pair").unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        assert!(scope_has_any_conservative(scope_has_any_in(
            &mut file,
            "some-scope"
        )));
    }

    #[test]
    fn invalid_variable_names_are_rejected_before_persistence() {
        crate::paths::isolate_home_for_tests();
        let scope_hint = tempfile::tempdir().unwrap().path().join(".env");
        for var in ["", "HAS\0NUL", "HAS\rRETURN", "HAS\nNEWLINE"] {
            assert_eq!(
                set(&scope_hint, var, "value").unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert!(!scope_has_any(&scope_hint));
    }

    #[test]
    fn forget_all_retains_entries_when_removal_fails() {
        let entries = vec!["scope\0removed".to_string(), "scope\0retained".to_string()];
        let (removed, retained) = remove_indexed_entries(&entries, |_, var| var == "removed");
        assert_eq!(removed, 1);
        assert_eq!(retained, vec!["scope\0retained"]);
    }
}
