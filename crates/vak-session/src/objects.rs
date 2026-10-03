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
use vak_storage::objects::{LocalObjectStore, ObjectId};

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

fn objects_error(error: vak_storage::StorageError) -> SessionError {
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

/// The tenant's object store at `<tenant>/objects`, its keys in the
/// credential store and its revocations in `<tenant>/keys/revoked`.
pub struct TenantObjects {
    store: LocalObjectStore,
}

impl TenantObjects {
    pub fn open(tenant_home: &Path) -> Result<Self, SessionError> {
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
        drop(lock);
        let store =
            LocalObjectStore::open(&tenant_home.join("objects"), id_key, Arc::new(authority))
                .map_err(objects_error)?;
        Ok(Self { store })
    }
}

impl Objects for TenantObjects {
    fn put(&self, bytes: &[u8], scope: &str) -> Result<ObjectRef, SessionError> {
        let id = self.store.put(bytes, scope).map_err(objects_error)?;
        Ok(ObjectRef {
            id: id.0,
            len: bytes.len() as u64,
        })
    }

    fn get(&self, object: &ObjectRef, scope: &str) -> Result<Vec<u8>, SessionError> {
        self.store
            .get(&ObjectId(object.id.clone()), scope)
            .map_err(objects_error)
    }

    fn id_of(&self, bytes: &[u8]) -> String {
        self.store.id(bytes).0
    }

    fn release(&self, object: &ObjectRef, scope: &str) -> Result<(), SessionError> {
        self.store
            .remove_grant(&ObjectId(object.id.clone()), scope)
            .map_err(objects_error)
    }

    fn collect(&self) -> Result<usize, SessionError> {
        self.store.gc().map_err(objects_error)
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
