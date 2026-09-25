//! The one way a small durable file is rewritten in place: a read-modify-
//! write held under one process-wide lock from the read to the rename, and
//! published by renaming a temporary file no other write shares.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Held by every [`update_file`], from the read to the rename. One lock for
/// every path rather than one per path: a file can be reachable under more
/// than one spelling, and a per-path lock would first have to agree which
/// spellings name one file. These writes are rare and small.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, thiserror::Error)]
pub enum UpdateError<E> {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Edit(E),
}

/// Read `path` (`None` when it does not exist), let `edit` decide the new
/// contents, and atomically replace the file with them. `edit` returns
/// `None` to leave the file untouched.
///
/// Holding the lock from the read to the rename is what stops two edits made
/// at the same moment from each rewriting the file from a read taken before
/// the other landed. `edit` must preserve whatever it does not own, and must
/// not call `update_file` itself: the lock is not reentrant.
pub fn update_file<T, E>(
    path: &Path,
    edit: impl FnOnce(Option<&str>) -> Result<(Option<String>, T), E>,
) -> Result<T, UpdateError<E>> {
    let _guard = UPDATE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(UpdateError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let (next, outcome) = edit(current.as_deref()).map_err(UpdateError::Edit)?;
    if let Some(next) = next
        && current.as_deref() != Some(next.as_str())
    {
        replace_file(path, &next).map_err(|source| UpdateError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(outcome)
}

/// Write `contents` to a new sibling of `path` and rename it over `path`, so
/// a reader sees the whole old document or the whole new one. The temporary
/// name carries the process id and a per-process sequence number and is
/// created exclusively, so no two writes ever share one; it is removed if
/// the write fails.
pub fn replace_file(path: &Path, contents: &str) -> std::io::Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(std::io::Error::other("path has no parent directory"));
    };
    std::fs::create_dir_all(parent)?;
    let mut attempts = 0;
    let (temp, mut file) = loop {
        let temp = parent.join(format!(
            ".{}.{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (temp, file),
            // Only an earlier process that had this pid can hold a fresh
            // name; step past its file rather than write into it.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempts < 8 => {
                attempts += 1;
            }
            Err(error) => return Err(error),
        }
    };
    let written =
        std::io::Write::write_all(&mut file, contents.as_bytes()).and_then(|()| file.sync_all());
    drop(file);
    written
        .and_then(|()| std::fs::rename(&temp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_appends_all_land_and_leave_no_temporaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("list.txt");
        std::thread::scope(|scope| {
            for writer in 0..16 {
                let path = &path;
                scope.spawn(move || {
                    for line in 0..25 {
                        update_file(path, |current| {
                            let mut next = current.unwrap_or_default().to_owned();
                            next.push_str(&format!("{writer}-{line}\n"));
                            Ok::<_, std::convert::Infallible>((Some(next), ()))
                        })
                        .unwrap();
                    }
                });
            }
        });
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 16 * 25);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("list.txt")]);
    }

    #[test]
    fn declined_or_failed_edit_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept.txt");
        std::fs::write(&path, "original").unwrap();
        update_file(&path, |_| Ok::<_, &str>((None, ()))).unwrap();
        assert!(matches!(
            update_file(&path, |_| Err::<(Option<String>, ()), _>("no")),
            Err(UpdateError::Edit("no"))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
    }
}
