//! SHA-256 over installed and downloaded artifacts.
//!
//! Update writes an executable that arrived over the network. Without a
//! digest to check it against, a truncated body or a substituted file is
//! indistinguishable from a good download, and the failure surfaces later
//! as a binary that will not run.

use std::io::Read as _;
use std::path::Path;

use sha2::{Digest as _, Sha256};

const CHUNK: usize = 64 * 1024;

/// Lowercase hex SHA-256 of a file, streamed so a large artifact is never
/// held in memory twice.
pub fn of_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Lowercase hex SHA-256 of a byte slice.
pub fn of_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Compare a computed digest against an expected one, case- and
/// whitespace-insensitively so a digest pasted from `shasum` output works.
pub fn matches(expected: &str, actual: &str) -> bool {
    expected.trim().eq_ignore_ascii_case(actual.trim())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn file_and_bytes_digests_agree() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, b"vak").unwrap();
        assert_eq!(of_file(&p).unwrap(), of_bytes(b"vak"));
    }

    #[test]
    fn digest_spans_content_larger_than_one_chunk() {
        // Guards the streaming loop: a bug that hashed only the first
        // chunk would still pass a small-file test.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big");
        let body = vec![7u8; CHUNK * 3 + 17];
        std::fs::write(&p, &body).unwrap();
        assert_eq!(of_file(&p).unwrap(), of_bytes(&body));
    }

    #[test]
    fn comparison_tolerates_case_and_surrounding_whitespace() {
        let d = of_bytes(b"x");
        assert!(matches(&format!("  {}  ", d.to_uppercase()), &d));
        assert!(!matches("00", &d));
    }
}
