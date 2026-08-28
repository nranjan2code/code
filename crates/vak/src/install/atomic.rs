//! Filesystem writes that either land completely or not at all.
//!
//! Every install and update writes through here. A partial write to an
//! executable or a manifest leaves an install that reports itself healthy
//! while being unrunnable, which is the failure mode hardest to diagnose
//! after the fact.

use std::path::Path;

/// Write bytes to `path` via a temporary file in the same directory, then
/// rename. The rename is atomic within a filesystem, so a reader sees
/// either the old contents or the new ones.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    // Unique per process so two concurrent writers cannot share a temp
    // file and interleave their bytes.
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("vak-write"),
        std::process::id()
    ));
    let result = (|| {
        std::fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("rename into {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Write bytes and mark the result executable.
pub fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write(path, bytes)?;
    set_executable(path)
}

pub fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod {}: {e}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Recursively copy a directory tree, each file written atomically.
pub fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("read dir {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("dir entry under {}: {e}", src.display()))?;
        let ty = entry
            .file_type()
            .map_err(|e| format!("file type {}: {e}", entry.path().display()))?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            let bytes = std::fs::read(entry.path())
                .map_err(|e| format!("read {}: {e}", entry.path().display()))?;
            write(&target, &bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn write_replaces_content_and_leaves_no_temp_behind() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.json");
        write(&f, b"one").unwrap();
        write(&f, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "two");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must not survive a write");
    }

    #[test]
    fn executable_bit_is_set_on_a_written_binary() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("nested").join("dst");
        write_executable(&dst, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&dst).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn copy_dir_reproduces_a_nested_tree() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("a/b")).unwrap();
        std::fs::write(src.join("top.txt"), b"top").unwrap();
        std::fs::write(src.join("a/b/deep.txt"), b"deep").unwrap();
        let dst = dir.path().join("dst");
        copy_dir(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("top.txt")).unwrap(), "top");
        assert_eq!(
            std::fs::read_to_string(dst.join("a/b/deep.txt")).unwrap(),
            "deep"
        );
    }
}
