//! A ledger's large payloads live as tenant objects, and the ledger keeps
//! a reference (docs/design/73-data-architecture-and-lifecycle.md §5,
//! plan M3b slice 2). Each conversation reads through its own scope, so an
//! object shared by two conversations is stored once and erasing one
//! conversation's grant leaves the other's intact.

use crate::types::SessionError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use vak_storage::keys::{KekVault, VaultKeyAuthority};
use vak_storage::objects::ObjectId;
use vak_storage::store::{LocalStore, Store};

/// A payload held outside the ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub id: String,
    /// Plaintext bytes, so a reader can size a window without fetching.
    pub len: u64,
}

pub trait Objects: Send + Sync {
    fn put(&self, bytes: &[u8], scope: &str) -> Result<ObjectRef, SessionError>;
    fn get(&self, object: &ObjectRef, scope: &str) -> Result<Vec<u8>, SessionError>;
    /// The id `put` would give `bytes`, without storing them.
    fn id_of(&self, bytes: &[u8]) -> String;
    /// Removes `scope`'s grant; the object lives on while another scope
    /// holds one.
    fn release(&self, object: &ObjectRef, scope: &str) -> Result<(), SessionError>;
    /// Deletes every object no scope holds a grant for; returns how many.
    fn collect(&self) -> Result<usize, SessionError>;
}

/// The scope a conversation's objects are granted to.
pub fn conversation_scope(session_id: &str) -> String {
    format!("conversation:{session_id}")
}

fn scope_error(scope: &str, error: vak_storage::StorageError) -> SessionError {
    match error {
        vak_storage::StorageError::Revoked(_) => SessionError::Erased(scope.to_string()),
        other => objects_error(other),
    }
}

/// The scope what one person other than the owner wrote into a
/// conversation is granted to (plan M7a-b): destroying it removes that
/// person's contributions and leaves the conversation readable.
pub fn contributor_scope(session_id: &str, principal: &str) -> String {
    format!("contributor:{session_id}:{principal}")
}

/// The scope what a connected provider account returned is granted to,
/// wherever it was read (plan M7a-b): destroying it removes that account's
/// data from every conversation and leaves the conversations.
pub fn account_scope(account: &str) -> String {
    format!("account:{account}")
}

/// The principal a contributor scope names.
pub fn contributor_of(scope: &str) -> Option<&str> {
    let mut parts = scope.strip_prefix("contributor:")?.splitn(2, ':');
    parts.next()?;
    parts.next()
}

pub(crate) fn objects_error(error: vak_storage::StorageError) -> SessionError {
    SessionError::Objects(error.to_string())
}

struct CredentialVault(PathBuf);

impl KekVault for CredentialVault {
    fn get(&self, name: &str) -> Option<String> {
        vak_config::credentials::get(&self.0, name)
    }

    fn set(&self, name: &str, value: &str) -> std::io::Result<()> {
        vak_config::credentials::set(&self.0, name, value)
    }
}

/// The tenant's store at `<tenant>/store` (objects and refs), its keys in
/// the credential store and its revocations in `<tenant>/keys/revoked`.
/// One per tenant per process (`TenantObjects::for_tenant`), shared by
/// ledgers and Documents. The store's writer epoch is read once, at open:
/// it is what this process presents to every ref it moves, so a restore
/// after it started fences it (`crate::fence`).
pub struct TenantObjects {
    store: Arc<LocalStore>,
    scopes: vak_storage::scopes::ScopeKeys,
    /// Scope keys this process has unwrapped, by scope.
    unwrapped: Mutex<HashMap<String, vak_storage::records::ScopeKey>>,
    /// The tenant's receipt-signing key (Ed25519, PKCS#8).
    signing: Vec<u8>,
    writer_epoch: crate::fence::WriterEpoch,
}

type OpenTenants = Mutex<HashMap<PathBuf, Arc<TenantObjects>>>;

fn open_tenants() -> &'static OpenTenants {
    static OPEN: std::sync::OnceLock<OpenTenants> = std::sync::OnceLock::new();
    OPEN.get_or_init(Default::default)
}

/// Every tenant store this process has opened.
pub(crate) fn opened() -> Vec<Arc<TenantObjects>> {
    open_tenants()
        .lock()
        .map(|open| open.values().cloned().collect())
        .unwrap_or_default()
}

impl TenantObjects {
    /// The process's handle on the tenant at `tenant_home`, opened once.
    pub fn for_tenant(tenant_home: &Path) -> Result<Arc<Self>, SessionError> {
        let mut open = open_tenants()
            .lock()
            .map_err(|_| SessionError::Objects("tenant stores poisoned".into()))?;
        if let Some(found) = open.get(tenant_home) {
            return Ok(found.clone());
        }
        let opened = Arc::new(Self::open(tenant_home)?);
        open.insert(tenant_home.to_path_buf(), opened.clone());
        Ok(opened)
    }

    fn open(tenant_home: &Path) -> Result<Self, SessionError> {
        let keys = tenant_home.join("keys");
        std::fs::create_dir_all(&keys)?;
        let lock = File::create(keys.join("LOCK"))?;
        lock.lock()?;
        let authority = VaultKeyAuthority::open(
            Box::new(CredentialVault(keys.join("vault"))),
            &keys.join("revoked"),
        )
        .map_err(objects_error)?;
        let id_key = authority.id_key().map_err(objects_error)?;
        let signing = authority.signing_key().map_err(objects_error)?;
        let _ = lock.unlock();
        let authority: Arc<dyn vak_storage::keys::KeyAuthority> = Arc::new(authority);
        let scopes = vak_storage::scopes::ScopeKeys::open(&keys.join("scopes"), authority.clone())
            .map_err(objects_error)?;
        let store = LocalStore::open(&tenant_home.join("store"), id_key, authority)
            .map_err(objects_error)?;
        let writer_epoch = crate::fence::WriterEpoch(store.epoch().map_err(objects_error)?);
        Ok(Self {
            store: Arc::new(store),
            scopes,
            unwrapped: Default::default(),
            signing,
            writer_epoch,
        })
    }

    /// The tenant's keys as its vault holds them, for a key file. The
    /// tenant must have been opened once, which is what mints them.
    pub fn key_material(
        tenant_home: &Path,
    ) -> Result<vak_storage::keys::KeyMaterial, SessionError> {
        Self::for_tenant(tenant_home)?;
        vak_storage::keys::KeyMaterial::read(&CredentialVault(tenant_home.join("keys/vault")))
            .map_err(objects_error)
    }

    /// Replaces the tenant's keys with `material`, from another machine's
    /// key file. A process that already opened the tenant keeps the keys
    /// it read, so it must stop before anything is written under them.
    pub fn install_key_material(
        tenant_home: &Path,
        material: &vak_storage::keys::KeyMaterial,
    ) -> Result<(), SessionError> {
        let keys = tenant_home.join("keys");
        std::fs::create_dir_all(&keys)?;
        let lock = File::create(keys.join("LOCK"))?;
        lock.lock()?;
        let installed = material
            .install(&CredentialVault(keys.join("vault")))
            .map_err(objects_error);
        let _ = lock.unlock();
        installed
    }

    /// The key of `scope`, created when the scope has none. A destroyed
    /// scope is never created again.
    pub fn create_scope_key(
        &self,
        scope: &str,
    ) -> Result<vak_storage::records::ScopeKey, SessionError> {
        crate::fence::check()?;
        let key = self
            .scopes
            .create(scope)
            .map_err(|e| scope_error(scope, e))?;
        self.remember(scope, &key);
        Ok(key)
    }

    /// The key of `scope`. Whether the scope was destroyed is read each
    /// time, so a key this process unwrapped earlier stops opening anything
    /// once another process destroys it.
    pub fn scope_key(&self, scope: &str) -> Result<vak_storage::records::ScopeKey, SessionError> {
        if self.scopes.is_shredded(scope) {
            self.forget(scope);
            return Err(SessionError::Erased(scope.to_string()));
        }
        if let Ok(unwrapped) = self.unwrapped.lock()
            && let Some(key) = unwrapped.get(scope)
        {
            return Ok(key.clone());
        }
        let key = self.scopes.get(scope).map_err(|e| scope_error(scope, e))?;
        self.remember(scope, &key);
        Ok(key)
    }

    /// Signs `message` as this tenant: the signature and the public key
    /// that checks it, both as hex.
    pub fn sign(&self, message: &[u8]) -> Result<(String, String), SessionError> {
        let hex = |bytes: Vec<u8>| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        vak_storage::keys::sign(&self.signing, message)
            .map(|(signature, public)| (hex(signature), hex(public)))
            .map_err(objects_error)
    }

    /// Every scope with a key whose name starts with `prefix`.
    pub fn scopes_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SessionError> {
        self.scopes.with_prefix(prefix).map_err(objects_error)
    }

    /// Every destroyed scope whose name starts with `prefix`.
    pub fn destroyed_with_prefix(&self, prefix: &str) -> Result<Vec<String>, SessionError> {
        self.scopes
            .shredded_with_prefix(prefix)
            .map_err(objects_error)
    }

    /// Puts `scope` on hold: its key cannot be destroyed until released.
    pub fn hold_scope(&self, scope: &str) -> Result<(), SessionError> {
        crate::fence::check()?;
        self.scopes.hold(scope).map_err(|e| scope_error(scope, e))
    }

    pub fn release_scope(&self, scope: &str) -> Result<(), SessionError> {
        crate::fence::check()?;
        self.scopes
            .release(scope)
            .map_err(|e| scope_error(scope, e))
    }

    pub fn scope_held(&self, scope: &str) -> bool {
        self.scopes.is_held(scope)
    }

    /// Whether `scope`'s key was destroyed.
    pub fn scope_destroyed(&self, scope: &str) -> bool {
        self.scopes.is_shredded(scope)
    }

    /// Destroys again every key a restored backup brought back for a
    /// scope that was destroyed (`ScopeKeys::reshred`), and forgets every
    /// key this process had unwrapped. Returns how many it removed.
    pub fn reapply_destroyed_keys(&self) -> Result<usize, SessionError> {
        crate::fence::check()?;
        let removed = self.scopes.reshred().map_err(objects_error)?;
        if let Ok(mut unwrapped) = self.unwrapped.lock() {
            unwrapped.clear();
        }
        Ok(removed)
    }

    /// Every scope on hold.
    pub fn held_scopes(&self) -> Vec<String> {
        let mut held: Vec<String> = self
            .scopes
            .held_scopes()
            .map(|held| held.into_iter().collect())
            .unwrap_or_default();
        held.sort();
        held
    }

    /// How many scopes are on hold.
    pub fn held_count(&self) -> usize {
        self.scopes.held_scopes().map_or(0, |held| held.len())
    }

    /// Starts a new tenant key and wraps every scope key and every object
    /// grant under it. Earlier keys stay in the credential store, so a
    /// backup made before still opens. Returns the new version and how
    /// many keys it wrapped again.
    pub fn rotate_keys(&self) -> Result<(u32, usize), SessionError> {
        crate::fence::check()?;
        let (version, scopes) = self.scopes.rotate().map_err(objects_error)?;
        let grants = self.store.rewrap_grants().map_err(objects_error)?;
        if let Ok(mut unwrapped) = self.unwrapped.lock() {
            unwrapped.clear();
        }
        Ok((version, scopes + grants))
    }

    /// Wraps every key again under the current tenant key, then destroys
    /// every earlier tenant key. Returns how many were destroyed. A key
    /// file or backup made before still holds what it held; this install
    /// can no longer open what was wrapped under one only.
    pub fn retire_earlier_keys(&self) -> Result<usize, SessionError> {
        crate::fence::check()?;
        self.scopes.rewrap().map_err(objects_error)?;
        self.store.rewrap_grants().map_err(objects_error)?;
        let grants = self.store.oldest_grant_version().map_err(objects_error)?;
        let retired = self.scopes.retire_earlier(grants).map_err(objects_error)?;
        if let Ok(mut unwrapped) = self.unwrapped.lock() {
            unwrapped.clear();
        }
        Ok(retired)
    }

    /// Whether anything is stored under this tenant's keys: a scope key,
    /// a destroyed one, or any object. A tenant that holds nothing can
    /// take another machine's keys.
    pub fn holds_nothing(&self) -> bool {
        self.scopes.counts() == (0, 0) && self.store.holds_nothing()
    }

    /// Every ref of the tenant's store as one portable file
    /// (`LocalStore::export_refs`).
    pub fn export_refs(&self) -> Result<Vec<u8>, SessionError> {
        self.store.export_refs().map_err(objects_error)
    }

    /// Replaces the store's refs with those of a remote's file, and
    /// returns the writer epoch that store had. Only a pull does this,
    /// and it ends by moving this store's epoch past it.
    pub fn import_refs(&self, file: &[u8]) -> Result<u64, SessionError> {
        crate::fence::check()?;
        self.store.import_refs(file).map_err(objects_error)
    }

    /// The tenant key version new keys are wrapped under, and the oldest
    /// one any scope key is still under.
    pub fn key_versions(&self) -> Result<(u32, u32), SessionError> {
        let current = self.scopes.current_version().map_err(objects_error)?;
        let oldest = self.scopes.oldest_version().map_err(objects_error)?;
        Ok((current, oldest.unwrap_or(current)))
    }

    /// How many scopes have a key, and how many were destroyed.
    pub fn scope_counts(&self) -> (usize, usize) {
        self.scopes.counts()
    }

    /// Destroys `scope`'s key: every frame sealed under it and every object
    /// granted only to it stops being readable, and no byte of either
    /// changes. Refused while the scope is held.
    pub fn destroy_scope_key(&self, scope: &str) -> Result<(), SessionError> {
        crate::fence::check()?;
        self.scopes
            .shred(scope)
            .map_err(|e| scope_error(scope, e))?;
        self.forget(scope);
        Ok(())
    }

    fn remember(&self, scope: &str, key: &vak_storage::records::ScopeKey) {
        if let Ok(mut unwrapped) = self.unwrapped.lock() {
            unwrapped.insert(scope.to_string(), key.clone());
        }
    }

    fn forget(&self, scope: &str) {
        if let Ok(mut unwrapped) = self.unwrapped.lock() {
            unwrapped.remove(scope);
        }
    }

    /// The store under these objects, for Documents.
    pub fn store(&self) -> Arc<dyn Store> {
        self.store.clone()
    }

    /// The epoch this process read when it opened the store.
    pub fn writer_epoch(&self) -> crate::fence::WriterEpoch {
        self.writer_epoch
    }
}

impl Objects for TenantObjects {
    fn put(&self, bytes: &[u8], scope: &str) -> Result<ObjectRef, SessionError> {
        crate::fence::check()?;
        let id = self.store.put_object(bytes, scope).map_err(objects_error)?;
        Ok(ObjectRef {
            id: id.0,
            len: bytes.len() as u64,
        })
    }

    fn get(&self, object: &ObjectRef, scope: &str) -> Result<Vec<u8>, SessionError> {
        // The authority's own revocations are read when a process opens it;
        // the tombstone is read here, so an erasure in another process
        // holds in this one.
        if self.scopes.is_shredded(scope) {
            return Err(SessionError::Erased(scope.to_string()));
        }
        self.store
            .get_object(&ObjectId(object.id.clone()), scope)
            .map_err(objects_error)
    }

    fn id_of(&self, bytes: &[u8]) -> String {
        self.store.object_id(bytes).0
    }

    fn release(&self, object: &ObjectRef, scope: &str) -> Result<(), SessionError> {
        self.store
            .remove_grant(&ObjectId(object.id.clone()), scope)
            .map_err(objects_error)
    }

    fn collect(&self) -> Result<usize, SessionError> {
        self.store.gc(&|_| false).map_err(objects_error)
    }
}

/// Objects held in memory: for tests and for a ledger with no tenant.
#[derive(Default)]
pub struct MemoryObjects {
    bodies: Mutex<HashMap<(String, String), Vec<u8>>>,
}

impl Objects for MemoryObjects {
    fn put(&self, bytes: &[u8], scope: &str) -> Result<ObjectRef, SessionError> {
        let id = self.id_of(bytes);
        self.bodies
            .lock()
            .map_err(|_| SessionError::Objects("memory objects poisoned".into()))?
            .insert((id.clone(), scope.to_string()), bytes.to_vec());
        Ok(ObjectRef {
            id,
            len: bytes.len() as u64,
        })
    }

    fn get(&self, object: &ObjectRef, scope: &str) -> Result<Vec<u8>, SessionError> {
        self.bodies
            .lock()
            .map_err(|_| SessionError::Objects("memory objects poisoned".into()))?
            .get(&(object.id.clone(), scope.to_string()))
            .cloned()
            .ok_or_else(|| SessionError::Objects("no such object in this scope".into()))
    }

    fn id_of(&self, bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn release(&self, object: &ObjectRef, scope: &str) -> Result<(), SessionError> {
        self.bodies
            .lock()
            .map_err(|_| SessionError::Objects("memory objects poisoned".into()))?
            .remove(&(object.id.clone(), scope.to_string()));
        Ok(())
    }

    fn collect(&self) -> Result<usize, SessionError> {
        Ok(0)
    }
}
