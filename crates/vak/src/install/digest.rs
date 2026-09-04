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

/// Lowercase hex SHA-256 over a whole directory tree.
///
/// The desktop frontend is a *tree* — `index.html` plus a directory of
/// content-hashed assets — and digesting only `index.html` would miss the
/// failure that actually happens: the shell arrives and the JS it names
/// does not, so the app opens a blank window with nothing in the console
/// to explain it. That has shipped from this repository twice by other
/// routes, and both times the shell was fine.
///
/// The digest covers each file's path as well as its bytes, so a deleted
/// asset, an added one, and a renamed one all change the result. Paths are
/// sorted, so the digest does not depend on directory-iteration order.
#[cfg(test)]
pub fn of_tree(root: &Path) -> Result<String, String> {
    of_tree_excluding(root, &[])
}

/// [`of_tree`], skipping paths that must not be inside their own digest.
///
/// In a macOS bundle the install manifest lives at
/// `Contents/Resources/install.json` — the same directory as the desktop
/// frontend. A manifest cannot contain a digest of a tree that contains the
/// manifest: the value would change the moment it was written, and
/// `verify` would report every fresh install as corrupt. (It did.)
pub fn of_tree_excluding(root: &Path, exclude: &[&Path]) -> Result<String, String> {
    let mut files = Vec::new();
    collect(root, root, &mut files)?;
    files.retain(|rel| !exclude.iter().any(|e| *e == root.join(rel)));
    files.sort();

    let mut hasher = Sha256::new();
    for rel in &files {
        hasher.update(rel.as_bytes());
        hasher.update([0u8]);
        hasher.update(of_file(&root.join(rel))?.as_bytes());
        hasher.update([0u8]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read dir {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| format!("{} escaped {}", path.display(), root.display()))?;
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
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
    fn tree_digest_notices_a_missing_asset() {
        // The exact shape of the failure this guards: index.html intact,
        // the bundle it names gone.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("index.html"), b"<script src=/assets/a.js>").unwrap();
        std::fs::write(root.join("assets/a.js"), b"console.log(1)").unwrap();

        let before = of_tree(root).unwrap();
        std::fs::remove_file(root.join("assets/a.js")).unwrap();
        assert_ne!(
            before,
            of_tree(root).unwrap(),
            "a deleted asset must change the tree digest"
        );
    }

    #[test]
    fn tree_digest_is_stable_and_notices_a_rename() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("assets/one.js"), b"x").unwrap();
        std::fs::write(root.join("assets/two.js"), b"y").unwrap();

        let a = of_tree(root).unwrap();
        assert_eq!(a, of_tree(root).unwrap(), "digest must not vary run to run");

        // Same bytes, different name: content-hashed filenames are how a
        // frontend rebuild announces itself, so the digest must move.
        std::fs::rename(root.join("assets/one.js"), root.join("assets/three.js")).unwrap();
        assert_ne!(a, of_tree(root).unwrap());
    }

    #[test]
    fn a_tree_digest_can_exclude_the_file_that_will_hold_it() {
        // The bundle case: the manifest lands in the directory it
        // describes, so a digest that counted it would never match twice.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("index.html"), b"shell").unwrap();

        let manifest = root.join("install.json");
        let before = of_tree_excluding(root, &[manifest.as_path()]).unwrap();
        std::fs::write(&manifest, b"{}").unwrap();
        assert_eq!(
            before,
            of_tree_excluding(root, &[manifest.as_path()]).unwrap(),
            "writing the excluded file must not move the digest"
        );
        assert_ne!(
            before,
            of_tree(root).unwrap(),
            "and without the exclusion it must, or the test proves nothing"
        );
    }

    #[test]
    fn comparison_tolerates_case_and_surrounding_whitespace() {
        let d = of_bytes(b"x");
        assert!(matches(&format!("  {}  ", d.to_uppercase()), &d));
        assert!(!matches("00", &d));
    }
}
