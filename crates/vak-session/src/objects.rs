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
            writer_epoch,
        })
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
