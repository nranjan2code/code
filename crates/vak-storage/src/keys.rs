//! Key authority: wraps scope and object keys under a tenant KEK.
//! An unhealthy authority fails closed: no wrap, no unwrap.

use crate::seal::{self, KEY_LEN};
use crate::{Result, StorageError};
use std::collections::HashSet;
use std::sync::Mutex;

/// A key wrapped under one KEK version and bound to a scope name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrappedKey {
    pub version: u32,
    pub scope: String,
    pub bytes: Vec<u8>,
}

impl WrappedKey {
    pub fn encode(&self) -> Vec<u8> {
        let s = self.scope.as_bytes();
        let mut out = Vec::with_capacity(8 + s.len() + self.bytes.len());
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s);
        out.extend_from_slice(&self.bytes);
        out
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let bad = StorageError::Malformed("wrapped key");
        if b.len() < 8 {
            return Err(bad);
        }
        let version = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let n = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
        let rest = &b[8..];
        if rest.len() < n {
            return Err(bad);
        }
        let scope = String::from_utf8(rest[..n].to_vec())
            .map_err(|_| StorageError::Malformed("wrapped key scope"))?;
        Ok(Self {
            version,
            scope,
            bytes: rest[n..].to_vec(),
        })
    }
}

pub trait KeyAuthority: Send + Sync {
    fn wrap(&self, scope: &str, key: &[u8]) -> Result<WrappedKey>;
    fn unwrap(&self, wrapped: &WrappedKey) -> Result<Vec<u8>>;
    /// Starts a new KEK version for future wraps; returns it. Old versions
    /// still unwrap.
    fn rotate(&self) -> Result<u32>;
    /// Destroys a scope: nothing wrapped under it can be unwrapped again.
    fn revoke(&self, scope: &str) -> Result<()>;
    fn health(&self) -> Result<()>;
}

/// A fresh random 256-bit key, wrapped for `scope`.
pub fn new_scope_key(a: &dyn KeyAuthority, scope: &str) -> Result<([u8; KEY_LEN], WrappedKey)> {
    let k: [u8; KEY_LEN] = seal::random()?;
    let w = a.wrap(scope, &k)?;
    Ok((k, w))
}

fn aad(scope: &str, version: u32) -> Vec<u8> {
    let mut v = version.to_le_bytes().to_vec();
    v.extend_from_slice(scope.as_bytes());
    v
}

struct MemState {
    keks: Vec<[u8; KEY_LEN]>,
    revoked: HashSet<String>,
    healthy: bool,
}

pub struct MemoryKeyAuthority {
    state: Mutex<MemState>,
}

impl MemoryKeyAuthority {
    pub fn new() -> Result<Self> {
        Ok(Self {
            state: Mutex::new(MemState {
                keks: vec![seal::random()?],
                revoked: HashSet::new(),
                healthy: true,
            }),
        })
    }

    /// Simulates the credential store becoming unreachable.
    pub fn set_healthy(&self, healthy: bool) -> Result<()> {
        self.lock()?.healthy = healthy;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemState>> {
        self.state
            .lock()
            .map_err(|_| StorageError::AuthorityUnavailable("state poisoned".into()))
    }

    fn live(&self) -> Result<std::sync::MutexGuard<'_, MemState>> {
        let s = self.lock()?;
        if s.healthy {
            Ok(s)
        } else {
            Err(StorageError::AuthorityUnavailable("unhealthy".into()))
        }
    }
}

impl KeyAuthority for MemoryKeyAuthority {
    fn wrap(&self, scope: &str, key: &[u8]) -> Result<WrappedKey> {
        let s = self.live()?;
        if s.revoked.contains(scope) {
            return Err(StorageError::Revoked(scope.into()));
        }
        let version = (s.keks.len() - 1) as u32;
        let kek = s.keks[version as usize];
        let bytes = seal::seal(&kek, &aad(scope, version), key)?;
        Ok(WrappedKey {
            version,
            scope: scope.into(),
            bytes,
        })
    }

    fn unwrap(&self, w: &WrappedKey) -> Result<Vec<u8>> {
        let s = self.live()?;
        if s.revoked.contains(&w.scope) {
            return Err(StorageError::Revoked(w.scope.clone()));
        }
        let kek = s
            .keks
            .get(w.version as usize)
            .ok_or(StorageError::Crypto("unknown kek version"))?;
        seal::open(kek, &aad(&w.scope, w.version), &w.bytes)
    }

    fn rotate(&self) -> Result<u32> {
        let mut s = self.live()?;
        s.keks.push(seal::random()?);
        Ok((s.keks.len() - 1) as u32)
    }

    fn revoke(&self, scope: &str) -> Result<()> {
        self.live()?.revoked.insert(scope.into());
        Ok(())
    }

    fn health(&self) -> Result<()> {
        self.live().map(|_| ())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn key_authority_unavailable_fails_closed() {
        let a = MemoryKeyAuthority::new().unwrap();
        let (_, w) = new_scope_key(&a, "conv").unwrap();
        a.set_healthy(false).unwrap();
        assert!(matches!(
            a.unwrap(&w),
            Err(StorageError::AuthorityUnavailable(_))
        ));
        assert!(a.wrap("conv", &[1; 32]).is_err());
        assert!(a.health().is_err());
        a.set_healthy(true).unwrap();
        assert!(a.unwrap(&w).is_ok());
    }

    #[test]
    fn rotate_keeps_old_and_revoke_destroys() {
        let a = MemoryKeyAuthority::new().unwrap();
        let (k, w) = new_scope_key(&a, "c").unwrap();
        assert_eq!(a.rotate().unwrap(), 1);
        assert_eq!(a.unwrap(&w).unwrap(), k);
        assert_eq!(a.wrap("c", &k).unwrap().version, 1);
        a.revoke("c").unwrap();
        assert!(matches!(a.unwrap(&w), Err(StorageError::Revoked(_))));
        let rt = WrappedKey::decode(&w.encode()).unwrap();
        assert_eq!(rt, w);
    }
}
