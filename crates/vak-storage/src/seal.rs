use crate::{Result, StorageError};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};

pub const KEY_LEN: usize = 32;
pub const MAX_PLAINTEXT: usize = 256 * 1024 * 1024;

pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    SystemRandom::new()
        .fill(&mut out)
        .map_err(|_| StorageError::Crypto("random source failed"))?;
    Ok(out)
}

fn key(k: &[u8]) -> Result<LessSafeKey> {
    UnboundKey::new(&AES_256_GCM, k)
        .map(LessSafeKey::new)
        .map_err(|_| StorageError::Crypto("bad key length"))
}

/// Output: nonce || ciphertext || tag.
pub fn seal(k: &[u8], aad: &[u8], plain: &[u8]) -> Result<Vec<u8>> {
    let nonce_bytes: [u8; NONCE_LEN] = random()?;
    let mut buf = plain.to_vec();
    key(k)?
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(aad),
            &mut buf,
        )
        .map_err(|_| StorageError::Crypto("seal failed"))?;
    let mut out = Vec::with_capacity(NONCE_LEN + buf.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&buf);
    Ok(out)
}

pub fn open(k: &[u8], aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < NONCE_LEN + 16 {
        return Err(StorageError::Malformed("sealed value too short"));
    }
    let (n, ct) = sealed.split_at(NONCE_LEN);
    let mut nb = [0u8; NONCE_LEN];
    nb.copy_from_slice(n);
    let mut buf = ct.to_vec();
    let plain = key(k)?
        .open_in_place(Nonce::assume_unique_for_key(nb), Aad::from(aad), &mut buf)
        .map_err(|_| StorageError::Crypto("open failed"))?;
    Ok(plain.to_vec())
}

pub fn compress(data: &[u8]) -> Result<Vec<u8>> {
    zstd::bulk::compress(data, 3).map_err(|e| StorageError::Compression(e.to_string()))
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    zstd::bulk::decompress(data, MAX_PLAINTEXT)
        .map_err(|e| StorageError::Compression(e.to_string()))
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
