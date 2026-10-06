//! Streaming for large blobs (plan M2): a stream is stored as ordinary
//! objects of at most a chunk each, sealed and granted like any other, and a
//! small manifest object naming them in order. Neither direction holds more
//! than one chunk in memory, and identical chunks dedupe within a tenant
//! like any object. The blob's id is its manifest's.

use crate::objects::ObjectId;
use crate::store::Store;
use crate::{Result, StorageError};
use std::io::{Read, Write};

/// The largest chunk a stream is cut into.
pub const CHUNK: usize = 8 * 1024 * 1024;

const MAGIC: &[u8; 8] = b"VAKBLOB1";

/// Stores everything `reader` yields for `scope`; returns the blob's id.
pub fn put(store: &dyn Store, reader: &mut dyn Read, scope: &str) -> Result<ObjectId> {
    put_chunked(store, reader, scope, CHUNK)
}

/// [`put`] with chunks of at most `chunk` bytes.
pub fn put_chunked(
    store: &dyn Store,
    reader: &mut dyn Read,
    scope: &str,
    chunk: usize,
) -> Result<ObjectId> {
    if chunk == 0 {
        return Err(StorageError::Malformed(
            "a blob chunk holds at least a byte",
        ));
    }
    let mut manifest = MAGIC.to_vec();
    let mut len = 0u64;
    let mut ids = Vec::new();
    let mut buffer = vec![0u8; chunk];
    loop {
        let mut filled = 0;
        while filled < chunk {
            let read = reader.read(&mut buffer[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == 0 {
            break;
        }
        ids.push(store.put_object(&buffer[..filled], scope)?);
        len += filled as u64;
        if filled < chunk {
            break;
        }
    }
    manifest.extend_from_slice(&len.to_le_bytes());
    manifest.extend_from_slice(&(ids.len() as u64).to_le_bytes());
    for id in &ids {
        let bytes = id.0.as_bytes();
        manifest.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        manifest.extend_from_slice(bytes);
    }
    store.put_object(&manifest, scope)
}

fn read_u64(bytes: &[u8], at: &mut usize) -> Result<u64> {
    let end = at
        .checked_add(8)
        .filter(|end| *end <= bytes.len())
        .ok_or(StorageError::Malformed("blob manifest"))?;
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[*at..end]);
    *at = end;
    Ok(u64::from_le_bytes(word))
}

/// The length and chunk ids the manifest `bytes` names.
fn manifest(bytes: &[u8]) -> Result<(u64, Vec<ObjectId>)> {
    if !bytes.starts_with(MAGIC) {
        return Err(StorageError::Malformed("not a blob manifest"));
    }
    let mut at = MAGIC.len();
    let len = read_u64(bytes, &mut at)?;
    let count = read_u64(bytes, &mut at)?;
    let mut ids = Vec::new();
    for _ in 0..count {
        let size_end = at
            .checked_add(4)
            .filter(|end| *end <= bytes.len())
            .ok_or(StorageError::Malformed("blob manifest"))?;
        let mut size = [0u8; 4];
        size.copy_from_slice(&bytes[at..size_end]);
        at = size_end;
        let end = at
            .checked_add(u32::from_le_bytes(size) as usize)
            .filter(|end| *end <= bytes.len())
            .ok_or(StorageError::Malformed("blob manifest"))?;
        let id = std::str::from_utf8(&bytes[at..end])
            .map_err(|_| StorageError::Malformed("blob manifest"))?;
        ids.push(ObjectId(id.to_owned()));
        at = end;
    }
    if at != bytes.len() {
        return Err(StorageError::Malformed("blob manifest"));
    }
    Ok((len, ids))
}

/// Writes the blob `id` to `writer`, a chunk at a time; returns its length.
/// A chunk that does not add up to the manifest's length is an error, never
/// a short read passed off as the whole.
pub fn get(store: &dyn Store, id: &ObjectId, scope: &str, writer: &mut dyn Write) -> Result<u64> {
    let (len, chunks) = manifest(&store.get_object(id, scope)?)?;
    let mut written = 0u64;
    for chunk in &chunks {
        let bytes = store.get_object(chunk, scope)?;
        written += bytes.len() as u64;
        writer.write_all(&bytes)?;
    }
    if written != len {
        return Err(StorageError::Integrity);
    }
    Ok(len)
}

/// Removes `scope`'s grant to the blob and each of its chunks; what no
/// scope holds is collected by the next `gc`.
pub fn release(store: &dyn Store, id: &ObjectId, scope: &str) -> Result<()> {
    let (_, chunks) = manifest(&store.get_object(id, scope)?)?;
    for chunk in &chunks {
        store.remove_grant(chunk, scope)?;
    }
    store.remove_grant(id, scope)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::objects::IdKey;
    use crate::store::MemoryStore;

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn a_stream_round_trips_through_chunks() {
        let store = MemoryStore::new(IdKey::new(&[3; 32]));
        for len in [0, 1, 999, 1000, 1001, 5000] {
            let data = sample(len);
            let id = put_chunked(&store, &mut data.as_slice(), "conv", 1000).unwrap();
            let mut out = Vec::new();
            assert_eq!(get(&store, &id, "conv", &mut out).unwrap(), len as u64);
            assert_eq!(out, data, "length {len}");
        }
    }

    #[test]
    fn a_released_blob_is_unreadable_and_its_chunks_go_with_it() {
        let store = MemoryStore::new(IdKey::new(&[4; 32]));
        let data = sample(3500);
        let id = put_chunked(&store, &mut data.as_slice(), "conv", 1000).unwrap();
        release(&store, &id, "conv").unwrap();
        let mut out = Vec::new();
        assert!(get(&store, &id, "conv", &mut out).is_err());
        assert!(
            store
                .get_object(&store.object_id(&data[..1000]), "conv")
                .is_err(),
            "the chunks lost their grant too"
        );
    }

    #[test]
    fn a_truncated_or_foreign_manifest_is_refused() {
        assert!(manifest(b"not a blob").is_err());
        let mut short = MAGIC.to_vec();
        short.extend_from_slice(&5u64.to_le_bytes());
        short.extend_from_slice(&3u64.to_le_bytes());
        assert!(manifest(&short).is_err());
    }
}
