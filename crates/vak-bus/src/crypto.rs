//! Defense-in-depth zero-trust cryptography for the distributed message fabric.
//!
//! Provides authenticated AES-256-GCM envelope payload encryption, HKDF-SHA256
//! key derivation, and AAD (Additional Authenticated Data) metadata binding.

use ring::aead::{
    AES_256_GCM, Aad, BoundKey, NONCE_LEN, Nonce, NonceSequence, OpeningKey, SealingKey, UnboundKey,
};
use ring::hkdf::{HKDF_SHA256, Salt};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::envelope::{EncryptionMeta, MessageEnvelope};

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("envelope is already encrypted")]
    AlreadyEncrypted,
    #[error("envelope is not encrypted")]
    NotEncrypted,
    #[error("missing encryption metadata in envelope")]
    MissingMetadata,
    #[error("unsupported encryption algorithm: {0}")]
    UnsupportedAlgorithm(String),
    #[error("invalid nonce format: {0}")]
    InvalidNonce(String),
    #[error("random generation error")]
    RandomError,
    #[error("key derivation error")]
    KeyDerivationError,
    #[error("encryption operation failed")]
    EncryptionFailed,
    #[error("decryption failed or authentication tag mismatch (tampering detected)")]
    DecryptionFailed,
}

struct SingleNonce(Option<[u8; NONCE_LEN]>);

impl SingleNonce {
    fn new(nonce: [u8; NONCE_LEN]) -> Self {
        Self(Some(nonce))
    }
}

impl NonceSequence for SingleNonce {
    fn advance(&mut self) -> Result<Nonce, ring::error::Unspecified> {
        self.0
            .take()
            .map(Nonce::assume_unique_for_key)
            .ok_or(ring::error::Unspecified)
    }
}

/// Derive a 32-byte AES-256-GCM symmetric key from a workspace secret using HKDF-SHA256.
pub fn derive_workspace_key(
    workspace_secret: &[u8],
    key_id: &str,
) -> Result<[u8; 32], CryptoError> {
    struct KeyMaterial;
    impl ring::hkdf::KeyType for KeyMaterial {
        fn len(&self) -> usize {
            32
        }
    }

    let salt = Salt::new(HKDF_SHA256, b"vak-bus-hkdf-salt-v1");
    let prk = salt.extract(workspace_secret);
    let info = [b"vak-bus-aes-256-gcm-", key_id.as_bytes()];

    let okm = prk
        .expand(&info, KeyMaterial)
        .map_err(|_| CryptoError::KeyDerivationError)?;

    let mut key_bytes = [0u8; 32];
    okm.fill(&mut key_bytes)
        .map_err(|_| CryptoError::KeyDerivationError)?;

    Ok(key_bytes)
}

/// Encrypt an envelope's payload in-place using AES-256-GCM.
///
/// Binds envelope metadata as AAD so headers (id, type, workspace, seq) cannot
/// be manipulated or swapped without failing authentication.
pub fn encrypt_envelope(
    envelope: &mut MessageEnvelope,
    workspace_secret: &[u8],
    key_id: &str,
) -> Result<(), CryptoError> {
    if envelope.encrypted {
        return Err(CryptoError::AlreadyEncrypted);
    }

    let key_bytes = derive_workspace_key(workspace_secret, key_id)?;
    let unbound_key =
        UnboundKey::new(&AES_256_GCM, &key_bytes).map_err(|_| CryptoError::EncryptionFailed)?;

    // Generate 12-byte random nonce
    let rng = SystemRandom::new();
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| CryptoError::RandomError)?;

    let mut sealing_key = SealingKey::new(unbound_key, SingleNonce::new(nonce_bytes));

    let aad_bytes = envelope.aad_bytes();
    let aad = Aad::from(&aad_bytes);

    // Encrypt payload in-place
    sealing_key
        .seal_in_place_append_tag(aad, &mut envelope.payload)
        .map_err(|_| CryptoError::EncryptionFailed)?;

    // Update envelope state
    envelope.encrypted = true;
    envelope.datacontenttype = "application/octet-stream".to_string();
    envelope.encryption = Some(EncryptionMeta {
        algorithm: "AES-256-GCM".to_string(),
        key_id: key_id.to_string(),
        nonce_hex: hex::encode(nonce_bytes),
    });

    let mut hasher = Sha256::new();
    hasher.update(&envelope.payload);
    envelope.lineage.payload_hash = hex::encode(hasher.finalize());

    Ok(())
}

/// Decrypt an envelope's payload in-place using AES-256-GCM.
///
/// Verifies the AAD and tag; returns `CryptoError::DecryptionFailed` if any bit
/// of the ciphertext, metadata, or nonce was tampered with.
pub fn decrypt_envelope(
    envelope: &mut MessageEnvelope,
    workspace_secret: &[u8],
) -> Result<(), CryptoError> {
    if !envelope.encrypted {
        return Err(CryptoError::NotEncrypted);
    }

    let meta = envelope
        .encryption
        .as_ref()
        .ok_or(CryptoError::MissingMetadata)?;

    if meta.algorithm != "AES-256-GCM" {
        return Err(CryptoError::UnsupportedAlgorithm(meta.algorithm.clone()));
    }

    let nonce_raw =
        hex::decode(&meta.nonce_hex).map_err(|e| CryptoError::InvalidNonce(e.to_string()))?;
    if nonce_raw.len() != NONCE_LEN {
        return Err(CryptoError::InvalidNonce(format!(
            "expected {NONCE_LEN} bytes, got {}",
            nonce_raw.len()
        )));
    }
    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(&nonce_raw);

    let key_bytes = derive_workspace_key(workspace_secret, &meta.key_id)?;
    let unbound_key =
        UnboundKey::new(&AES_256_GCM, &key_bytes).map_err(|_| CryptoError::DecryptionFailed)?;

    let mut opening_key = OpeningKey::new(unbound_key, SingleNonce::new(nonce_bytes));

    let aad_bytes = envelope.aad_bytes();
    let aad = Aad::from(&aad_bytes);

    let decrypted_slice = opening_key
        .open_in_place(aad, &mut envelope.payload)
        .map_err(|_| CryptoError::DecryptionFailed)?;

    let decrypted_len = decrypted_slice.len();
    envelope.payload.truncate(decrypted_len);

    envelope.encrypted = false;
    envelope.datacontenttype = "application/json".to_string();
    envelope.encryption = None;

    let mut hasher = Sha256::new();
    hasher.update(&envelope.payload);
    envelope.lineage.payload_hash = hex::encode(hasher.finalize());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::MessageEnvelope;

    #[test]
    fn round_trip_encryption_decryption() {
        let secret = b"super-secret-workspace-key-2026";
        let original_data = b"{\"command\":\"cargo build\",\"exit_code\":0}";

        let mut env = MessageEnvelope::new(
            "vak://test",
            "vak.work.task",
            "ws_123",
            "agent_456",
            1,
            MessageEnvelope::GENESIS_HASH,
            original_data.to_vec(),
            None,
        );

        assert!(!env.encrypted);
        encrypt_envelope(&mut env, secret, "key_v1").expect("encryption succeeds");
        assert!(env.encrypted);
        assert_ne!(env.payload, original_data);

        decrypt_envelope(&mut env, secret).expect("decryption succeeds");
        assert!(!env.encrypted);
        assert_eq!(env.payload, original_data);
    }

    #[test]
    fn tamper_detection_ciphertext() {
        let secret = b"super-secret-workspace-key-2026";
        let mut env = MessageEnvelope::new(
            "vak://test",
            "vak.work.task",
            "ws_123",
            "agent_456",
            1,
            MessageEnvelope::GENESIS_HASH,
            b"secret-payload".to_vec(),
            None,
        );

        encrypt_envelope(&mut env, secret, "key_v1").unwrap();
        // Flip one byte in the ciphertext
        if let Some(byte) = env.payload.first_mut() {
            *byte ^= 0xFF;
        }

        let err = decrypt_envelope(&mut env, secret).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptionFailed));
    }

    #[test]
    fn tamper_detection_metadata_aad() {
        let secret = b"super-secret-workspace-key-2026";
        let mut env = MessageEnvelope::new(
            "vak://test",
            "vak.work.task",
            "ws_123",
            "agent_456",
            1,
            MessageEnvelope::GENESIS_HASH,
            b"secret-payload".to_vec(),
            None,
        );

        encrypt_envelope(&mut env, secret, "key_v1").unwrap();
        // Tamper with envelope event type (part of AAD)
        env.event_type = "vak.hacked.task".to_string();

        let err = decrypt_envelope(&mut env, secret).unwrap_err();
        assert!(matches!(err, CryptoError::DecryptionFailed));
    }
}
