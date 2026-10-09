//! Key authority: wraps scope and object keys under a tenant KEK.
//! An unhealthy authority fails closed: no wrap, no unwrap.

use crate::seal::{self, KEY_LEN};
use crate::{Result, StorageError};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
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
    /// The KEK version new wraps use.
    fn version(&self) -> Result<u32>;
    /// Destroys a scope: nothing wrapped under it can be unwrapped again.
    fn revoke(&self, scope: &str) -> Result<()>;
    fn health(&self) -> Result<()>;
}

/// A fresh random 256-bit key, wrapped for `scope`.
/// Signs `message` with the PKCS#8 Ed25519 key `pkcs8`; returns the
/// signature and the public key that checks it.
pub fn sign(pkcs8: &[u8], message: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    use ring::signature::KeyPair;
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8)
        .map_err(|_| StorageError::Crypto("signing key"))?;
    Ok((
        pair.sign(message).as_ref().to_vec(),
        pair.public_key().as_ref().to_vec(),
    ))
}

/// Whether `signature` is `public_key`'s over `message`.
pub fn verify(public_key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(message, signature)
        .is_ok()
}

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

    fn version(&self) -> Result<u32> {
        Ok((self.live()?.keks.len() - 1) as u32)
    }

    fn revoke(&self, scope: &str) -> Result<()> {
        self.live()?.revoked.insert(scope.into());
        Ok(())
    }

    fn health(&self) -> Result<()> {
        self.live().map(|_| ())
    }
}

/// Where a durable authority keeps its KEKs: a secret store outside the
/// data it protects (locally, the OS credential store).
pub trait KekVault: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str) -> std::io::Result<()>;
}

const KEK_CURRENT: &str = "VAK_TENANT_KEK_CURRENT";
const ID_KEY: &str = "VAK_TENANT_OBJECT_ID_KEY";

fn kek_name(version: u32) -> String {
    format!("VAK_TENANT_KEK_{version}")
}

fn decode_key(hex: &str) -> Result<[u8; KEY_LEN]> {
    let bad = StorageError::Malformed("vault key");
    if hex.len() != KEY_LEN * 2 {
        return Err(bad);
    }
    let mut out = [0u8; KEY_LEN];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(
            hex.get(i * 2..i * 2 + 2)
                .ok_or(StorageError::Malformed("vault key"))?,
            16,
        )
        .map_err(|_| StorageError::Malformed("vault key"))?;
    }
    Ok(out)
}

const SIGNING_KEY: &str = "receipt-signing-key";

/// Everything a tenant's vault holds, as the vault stores it: what a
/// second machine needs to read the same data. Never logged, and written
/// nowhere but a vault or a passphrase-sealed key file.
#[derive(Clone, PartialEq, Eq)]
pub struct KeyMaterial {
    /// Every KEK version, oldest first.
    pub keks: Vec<String>,
    pub id_key: String,
    pub signing_key: String,
}

impl KeyMaterial {
    /// What `vault` holds. Fails when any part is missing.
    pub fn read(vault: &dyn KekVault) -> Result<Self> {
        let missing = || StorageError::AuthorityUnavailable("the vault is incomplete".into());
        let current: u32 = vault
            .get(KEK_CURRENT)
            .ok_or_else(missing)?
            .trim()
            .parse()
            .map_err(|_| StorageError::Malformed("kek version"))?;
        Ok(Self {
            keks: (0..=current)
                .map(|v| vault.get(&kek_name(v)).ok_or_else(missing))
                .collect::<Result<_>>()?,
            id_key: vault.get(ID_KEY).ok_or_else(missing)?,
            signing_key: vault.get(SIGNING_KEY).ok_or_else(missing)?,
        })
    }

    /// Puts this material into `vault`, replacing what it held.
    pub fn install(&self, vault: &dyn KekVault) -> Result<()> {
        if self.keks.is_empty() {
            return Err(StorageError::Malformed("key file"));
        }
        for (version, kek) in self.keks.iter().enumerate() {
            decode_key(kek)?;
            vault.set(&kek_name(version as u32), kek)?;
        }
        decode_key(&self.id_key)?;
        vault.set(ID_KEY, &self.id_key)?;
        vault.set(SIGNING_KEY, &self.signing_key)?;
        vault.set(KEK_CURRENT, &(self.keks.len() - 1).to_string())?;
        Ok(())
    }

    fn encode(&self) -> Vec<u8> {
        let mut lines = vec![self.id_key.clone(), self.signing_key.clone()];
        lines.extend(self.keks.iter().cloned());
        lines.join("\n").into_bytes()
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let bad = || StorageError::Malformed("key file");
        let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
        let mut lines = text.lines().map(str::to_string);
        let id_key = lines.next().ok_or_else(bad)?;
        let signing_key = lines.next().ok_or_else(bad)?;
        let keks: Vec<String> = lines.collect();
        if keks.is_empty() {
            return Err(bad());
        }
        Ok(Self {
            keks,
            id_key,
            signing_key,
        })
    }

    const MAGIC: &'static str = "vakyartha-key-file-1";
    const ITERATIONS: u32 = 600_000;

    fn passphrase_key(passphrase: &str, salt: &[u8], iterations: u32) -> Result<[u8; KEY_LEN]> {
        let iterations =
            std::num::NonZeroU32::new(iterations).ok_or(StorageError::Malformed("key file"))?;
        let mut key = [0u8; KEY_LEN];
        ring::pbkdf2::derive(
            ring::pbkdf2::PBKDF2_HMAC_SHA256,
            iterations,
            salt,
            passphrase.as_bytes(),
            &mut key,
        );
        Ok(key)
    }

    /// This material sealed under `passphrase` (PBKDF2-HMAC-SHA256, then
    /// the store's AEAD), as the text of a key file.
    pub fn seal(&self, passphrase: &str) -> Result<String> {
        let salt: [u8; 16] = seal::random()?;
        let key = Self::passphrase_key(passphrase, &salt, Self::ITERATIONS)?;
        let sealed = seal::seal(&key, Self::MAGIC.as_bytes(), &self.encode())?;
        Ok(format!(
            "{}\n{}\n{}\n{}\n",
            Self::MAGIC,
            Self::ITERATIONS,
            seal::hex(&salt),
            seal::hex(&sealed)
        ))
    }

    /// Opens a key file. A wrong passphrase and a damaged file read the
    /// same: `Crypto`.
    pub fn open(file: &str, passphrase: &str) -> Result<Self> {
        let bad = || StorageError::Malformed("key file");
        let unhex = |hex: &str| -> Result<Vec<u8>> {
            if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
                return Err(bad());
            }
            (0..hex.len() / 2)
                .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| bad()))
                .collect()
        };
        let mut lines = file.lines();
        if lines.next() != Some(Self::MAGIC) {
            return Err(bad());
        }
        let iterations: u32 = lines.next().and_then(|n| n.parse().ok()).ok_or_else(bad)?;
        if !(100_000..=10_000_000).contains(&iterations) {
            return Err(bad());
        }
        let salt = unhex(lines.next().ok_or_else(bad)?)?;
        let sealed = unhex(lines.next().ok_or_else(bad)?)?;
        let key = Self::passphrase_key(passphrase, &salt, iterations)?;
        Self::decode(&seal::open(&key, Self::MAGIC.as_bytes(), &sealed)?)
    }
}

/// The tenant KEKs held in a `KekVault`, with revocations recorded in a
/// file beside the data. Opening creates version 0 when the vault holds
/// none; the caller serialises opens of one tenant (a file lock), so two
/// processes never mint competing first keys.
pub struct VaultKeyAuthority {
    vault: Box<dyn KekVault>,
    revoked_path: PathBuf,
    state: Mutex<MemState>,
}

impl VaultKeyAuthority {
    pub fn open(vault: Box<dyn KekVault>, revoked_path: &Path) -> Result<Self> {
        let unavailable = |what: &str| StorageError::AuthorityUnavailable(what.into());
        let keks = match vault.get(KEK_CURRENT) {
            Some(current) => {
                let current: u32 = current
                    .trim()
                    .parse()
                    .map_err(|_| StorageError::Malformed("kek version"))?;
                (0..=current)
                    .map(|v| {
                        vault
                            .get(&kek_name(v))
                            .ok_or_else(|| unavailable("a kek version is missing"))
                            .and_then(|hex| decode_key(&hex))
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            None => {
                let first: [u8; KEY_LEN] = seal::random()?;
                vault.set(&kek_name(0), &seal::hex(&first))?;
                vault.set(KEK_CURRENT, "0")?;
                vec![first]
            }
        };
        let revoked = match fs::read_to_string(revoked_path) {
            Ok(text) => text
                .lines()
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            vault,
            revoked_path: revoked_path.to_path_buf(),
            state: Mutex::new(MemState {
                keks,
                revoked,
                healthy: true,
            }),
        })
    }

    /// The tenant's object id key, minted into the vault on first use.
    pub fn id_key(&self) -> Result<crate::objects::IdKey> {
        let bytes = match self.vault.get(ID_KEY) {
            Some(hex) => decode_key(&hex)?,
            None => {
                let fresh: [u8; KEY_LEN] = seal::random()?;
                self.vault.set(ID_KEY, &seal::hex(&fresh))?;
                fresh
            }
        };
        Ok(crate::objects::IdKey::new(&bytes))
    }

    /// The tenant's receipt-signing key (Ed25519, PKCS#8), minted into the
    /// vault on first use: what signs an erasure receipt, so one can be
    /// checked with the public key alone (plan M7a-e).
    pub fn signing_key(&self) -> Result<Vec<u8>> {
        const NAME: &str = SIGNING_KEY;
        let unhex = |hex: &str| -> Result<Vec<u8>> {
            let hex = hex.trim();
            if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
                return Err(StorageError::Malformed("signing key"));
            }
            (0..hex.len() / 2)
                .map(|i| {
                    u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                        .map_err(|_| StorageError::Malformed("signing key"))
                })
                .collect()
        };
        if let Some(hex) = self.vault.get(NAME) {
            return unhex(&hex);
        }
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng)
            .map_err(|_| StorageError::Crypto("signing key generation"))?;
        self.vault.set(NAME, &seal::hex(pkcs8.as_ref()))?;
        Ok(pkcs8.as_ref().to_vec())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, MemState>> {
        self.state
            .lock()
            .map_err(|_| StorageError::AuthorityUnavailable("state poisoned".into()))
    }
}

impl KeyAuthority for VaultKeyAuthority {
    fn wrap(&self, scope: &str, key: &[u8]) -> Result<WrappedKey> {
        let s = self.lock()?;
        if s.revoked.contains(scope) {
            return Err(StorageError::Revoked(scope.into()));
        }
        let version = (s.keks.len() - 1) as u32;
        let bytes = seal::seal(&s.keks[version as usize], &aad(scope, version), key)?;
        Ok(WrappedKey {
            version,
            scope: scope.into(),
            bytes,
        })
    }

    fn unwrap(&self, w: &WrappedKey) -> Result<Vec<u8>> {
        let mut s = self.lock()?;
        if s.revoked.contains(&w.scope) {
            return Err(StorageError::Revoked(w.scope.clone()));
        }
        // Another process may have rotated since this one opened the
        // vault: read the versions it added.
        while (s.keks.len() as u32) <= w.version {
            let Some(hex) = self.vault.get(&kek_name(s.keks.len() as u32)) else {
                break;
            };
            s.keks.push(decode_key(&hex)?);
        }
        let kek = s
            .keks
            .get(w.version as usize)
            .ok_or(StorageError::Crypto("unknown kek version"))?;
        seal::open(kek, &aad(&w.scope, w.version), &w.bytes)
    }

    fn rotate(&self) -> Result<u32> {
        let mut s = self.lock()?;
        let next: [u8; KEY_LEN] = seal::random()?;
        let version = s.keks.len() as u32;
        self.vault.set(&kek_name(version), &seal::hex(&next))?;
        self.vault.set(KEK_CURRENT, &version.to_string())?;
        s.keks.push(next);
        Ok(version)
    }

    fn version(&self) -> Result<u32> {
        Ok((self.lock()?.keks.len() - 1) as u32)
    }

    fn revoke(&self, scope: &str) -> Result<()> {
        let mut s = self.lock()?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.revoked_path)?;
        file.write_all(format!("{scope}\n").as_bytes())?;
        file.sync_all()?;
        s.revoked.insert(scope.into());
        Ok(())
    }

    fn health(&self) -> Result<()> {
        drop(self.lock()?);
        self.vault
            .get(KEK_CURRENT)
            .map(|_| ())
            .ok_or_else(|| StorageError::AuthorityUnavailable("vault unreachable".into()))
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

    struct MapVault(Mutex<std::collections::HashMap<String, String>>);

    impl KekVault for std::sync::Arc<MapVault> {
        fn get(&self, name: &str) -> Option<String> {
            self.0.lock().unwrap().get(name).cloned()
        }
        fn set(&self, name: &str, value: &str) -> std::io::Result<()> {
            self.0.lock().unwrap().insert(name.into(), value.into());
            Ok(())
        }
    }

    #[test]
    fn vault_authority_survives_reopen_rotation_and_revocation() {
        let dir = tempfile::tempdir().unwrap();
        let revoked = dir.path().join("revoked");
        let vault = std::sync::Arc::new(MapVault(Mutex::new(Default::default())));
        let a = VaultKeyAuthority::open(Box::new(vault.clone()), &revoked).unwrap();
        let (k, w) = new_scope_key(&a, "conv").unwrap();
        let id = a.id_key().unwrap().id(b"x");
        assert_eq!(a.rotate().unwrap(), 1);
        let (_, gone) = new_scope_key(&a, "gone").unwrap();
        a.revoke("gone").unwrap();
        drop(a);

        let b = VaultKeyAuthority::open(Box::new(vault.clone()), &revoked).unwrap();
        assert_eq!(b.unwrap(&w).unwrap(), k);
        assert_eq!(b.wrap("conv", &k).unwrap().version, 1);
        assert_eq!(b.id_key().unwrap().id(b"x"), id);
        assert!(matches!(b.unwrap(&gone), Err(StorageError::Revoked(_))));
        b.health().unwrap();
    }

    #[test]
    fn a_key_file_opens_only_with_its_passphrase() {
        let vault = std::sync::Arc::new(MapVault(Mutex::new(Default::default())));
        let dir = tempfile::tempdir().unwrap();
        let a =
            VaultKeyAuthority::open(Box::new(vault.clone()), &dir.path().join("revoked")).unwrap();
        a.id_key().unwrap();
        a.signing_key().unwrap();
        a.rotate().unwrap();
        let (key, wrapped) = new_scope_key(&a, "conv").unwrap();
        let material = KeyMaterial::read(&vault).unwrap();
        assert_eq!(material.keks.len(), 2);
        let file = material.seal("correct horse battery").unwrap();
        assert!(!file.contains(&material.keks[0]) && !file.contains(&material.id_key));
        assert!(KeyMaterial::open(&file, "wrong horse battery").is_err());
        assert!(KeyMaterial::open(&file.replace("a", "b"), "correct horse battery").is_err());
        let opened = KeyMaterial::open(&file, "correct horse battery").unwrap();
        assert!(opened == material);

        // Installed in another vault, it unwraps what the first wrapped.
        let other = std::sync::Arc::new(MapVault(Mutex::new(Default::default())));
        opened.install(&other).unwrap();
        let b = VaultKeyAuthority::open(Box::new(other), &dir.path().join("revoked-b")).unwrap();
        assert_eq!(b.unwrap(&wrapped).unwrap(), key.to_vec());
    }
}
