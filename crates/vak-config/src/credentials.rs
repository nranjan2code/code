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
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One secret backend. `scope` is an opaque, stable identifier for a
/// project/shared/agent layer — never a filesystem path passed straight
/// through, so a backend is free to store scopes any way it likes.
pub trait CredentialStore: Send + Sync {
    fn get(&self, scope: &str, var: &str) -> Option<String>;
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
        return &EncryptedFileStore;
    }
    static STORE: OnceLock<Box<dyn CredentialStore>> = OnceLock::new();
    STORE
        .get_or_init(|| {
            if os_keyring_reachable_with_timeout() {
                Box::new(OsKeyringStore) as Box<dyn CredentialStore>
            } else {
                Box::new(EncryptedFileStore) as Box<dyn CredentialStore>
            }
        })
        .as_ref()
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
    store().get(&scope_key_for(scope_hint), var)
}

pub fn set(scope_hint: &Path, var: &str, value: &str) -> io::Result<()> {
    let scope = bound_scope_key_for(scope_hint)?;
    store().set(&scope, var, value)?;
    index_mark_present(&scope, var);
    Ok(())
}

pub fn remove(scope_hint: &Path, var: &str) -> io::Result<()> {
    let scope = bound_scope_key_for(scope_hint)?;
    store().remove(&scope, var)?;
    index_mark_absent(&scope, var);
    Ok(())
}

/// Removes every stored secret, in every scope: the erasure of the whole
/// install (data-architecture plan M7b-f). The presence index names them,
/// because the OS store cannot be listed. Returns how many it removed.
pub fn forget_all() -> usize {
    let mut removed = 0;
    for entry in index_entries() {
        if let Some((scope, var)) = entry.split_once('\0')
            && store().remove(scope, var).is_ok()
        {
            removed += 1;
        }
    }
    index_write(&[]);
    removed
}

pub fn list(scope_hint: &Path) -> Vec<(String, String)> {
    store().list(&scope_key_for(scope_hint))
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
    let scope = scope_key_for(scope_hint);
    let prefix = format!("{scope}\0");
    index_entries()
        .iter()
        .any(|entry| entry.starts_with(&prefix))
}

fn index_path() -> PathBuf {
    crate::paths::data_home().join("credential_index.json")
}

fn index_entries() -> Vec<String> {
    std::fs::read_to_string(index_path())
        .ok()
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn index_write(entries: &[String]) {
    let path = index_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, entries.join("\n"));
}

fn index_mark_present(scope: &str, var: &str) {
    let key = format!("{scope}\0{var}");
    let mut entries = index_entries();
    if !entries.contains(&key) {
        entries.push(key);
        index_write(&entries);
    }
}

fn index_mark_absent(scope: &str, var: &str) {
    let key = format!("{scope}\0{var}");
    let mut entries = index_entries();
    let before = entries.len();
    entries.retain(|e| e != &key);
    if entries.len() != before {
        index_write(&entries);
    }
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
        keyring::Entry::new(&service_name(scope), var)
            .ok()?
            .get_password()
            .ok()
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
struct EncryptedFileStore;

impl EncryptedFileStore {
    fn home() -> PathBuf {
        // The data home, never `default_workspace()`: that directory is
        // the built-in Agent's workspace, so a file there (and the key
        // beside it) is reachable by the workspace tools (invariant 10).
        crate::paths::data_home()
    }

    fn data_path() -> PathBuf {
        Self::home().join("credentials.enc")
    }

    fn key_path() -> PathBuf {
        Self::home().join(".credential_key")
    }

    fn lock_path() -> PathBuf {
        Self::home().join(".credential_key.lock")
    }

    /// Exclusive, cross-process advisory lock spanning key
    /// generation/read plus the whole load-modify-save cycle. Without
    /// it, two concurrent first-time callers can each generate a
    /// different random key and each believe theirs is the one on disk,
    /// or two concurrent writers can interleave a read-modify-write and
    /// silently drop one write. The lock file is separate from the key
    /// file so a reader never needs write access to the key itself.
    fn with_lock<T>(&self, f: impl FnOnce() -> T) -> T {
        let lock_path = Self::lock_path();
        if let Some(parent) = lock_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
        {
            Ok(file) => {
                // Blocks until acquired; released on drop at the end of
                // this scope regardless of how `f` returns.
                let _ = file.lock();
                f()
            }
            // No lock file, no lock — degrade to unsynchronized rather
            // than fail the whole credential store over it.
            Err(_) => f(),
        }
    }

    fn key(&self) -> io::Result<[u8; 32]> {
        let key_path = Self::key_path();
        if let Ok(bytes) = std::fs::read(&key_path)
            && bytes.len() == 32
        {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        let rng = ring::rand::SystemRandom::new();
        let mut key = [0u8; 32];
        ring::rand::SecureRandom::fill(&rng, &mut key)
            .map_err(|_| io::Error::other("failed to generate credential key"))?;
        if let Some(parent) = key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&key_path, key)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(key)
    }

    /// Loads and decrypts every `(scope, var, value)` triple. An absent or
    /// corrupt file is treated as empty rather than an error — there is
    /// nothing meaningful to recover, and the store is rewritten wholesale
    /// on every `set`/`remove`.
    fn load_all(&self) -> Vec<(String, String, String)> {
        let Ok(key) = self.key() else {
            return Vec::new();
        };
        let Ok(bytes) = std::fs::read(Self::data_path()) else {
            return Vec::new();
        };
        let Some(plaintext) = decrypt(&key, &bytes) else {
            return Vec::new();
        };
        let Ok(plaintext) = String::from_utf8(plaintext) else {
            return Vec::new();
        };
        plaintext
            .lines()
            .filter_map(|line| {
                let (scope, rest) = line.split_once('\0')?;
                let (var, value) = rest.split_once('\0')?;
                Some((scope.to_string(), var.to_string(), value.to_string()))
            })
            .collect()
    }

    fn save_all(&self, entries: &[(String, String, String)]) -> io::Result<()> {
        let key = self.key()?;
        let plaintext = entries
            .iter()
            .map(|(scope, var, value)| format!("{scope}\0{var}\0{value}"))
            .collect::<Vec<_>>()
            .join("\n");
        let ciphertext = encrypt(&key, plaintext.as_bytes())
            .map_err(|_| io::Error::other("failed to encrypt credential store"))?;
        let data_path = Self::data_path();
        if let Some(parent) = data_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = data_path.with_extension("enc.tmp");
        std::fs::write(&tmp, ciphertext)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &data_path)
    }
}

impl CredentialStore for EncryptedFileStore {
    fn get(&self, scope: &str, var: &str) -> Option<String> {
        self.with_lock(|| {
            self.load_all()
                .into_iter()
                .find(|(s, v, _)| s == scope && v == var)
                .map(|(_, _, value)| value)
        })
    }

    fn set(&self, scope: &str, var: &str, value: &str) -> io::Result<()> {
        self.with_lock(|| {
            let mut entries = self.load_all();
            entries.retain(|(s, v, _)| !(s == scope && v == var));
            entries.push((scope.to_string(), var.to_string(), value.to_string()));
            self.save_all(&entries)
        })
    }

    fn remove(&self, scope: &str, var: &str) -> io::Result<()> {
        self.with_lock(|| {
            let mut entries = self.load_all();
            entries.retain(|(s, v, _)| !(s == scope && v == var));
            self.save_all(&entries)
        })
    }

    fn list(&self, scope: &str) -> Vec<(String, String)> {
        self.with_lock(|| {
            self.load_all()
                .into_iter()
                .filter(|(s, _, _)| s == scope)
                .map(|(_, var, value)| (var, value))
                .collect()
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

fn decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Option<Vec<u8>> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
    if ciphertext.len() < NONCE_LEN {
        return None;
    }
    let (nonce_bytes, sealed) = ciphertext.split_at(NONCE_LEN);
    let unbound = UnboundKey::new(&AES_256_GCM, key).ok()?;
    let opening_key = LessSafeKey::new(unbound);
    let mut in_out = sealed.to_vec();
    let nonce = Nonce::try_assume_unique_for_key(nonce_bytes).ok()?;
    let plaintext = opening_key
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .ok()?;
    Some(plaintext.to_vec())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// `EncryptedFileStore` holds no per-instance paths — every access
    /// re-resolves `data_home()` fresh (see the type's doc
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
        crate::paths::isolate_home_for_tests();
        let store = EncryptedFileStore;
        // Scope/var names unique to this test so it can safely share the
        // process-wide isolated home with sibling tests.
        store
            .set("credentials-rs-test-scope-a", "ANTHROPIC_API_KEY", "key-a")
            .unwrap();
        store
            .set("credentials-rs-test-scope-b", "ANTHROPIC_API_KEY", "key-b")
            .unwrap();
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
        let raw = std::fs::read(EncryptedFileStore::data_path()).unwrap();
        assert!(!raw.windows(5).any(|w| w == b"key-b"));

        store
            .remove("credentials-rs-test-scope-b", "ANTHROPIC_API_KEY")
            .unwrap();
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
}
