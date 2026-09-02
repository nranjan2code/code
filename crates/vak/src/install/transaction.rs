//! All-or-nothing placement of a set of files.
//!
//! Install and update both replace several executables at once. Doing that
//! file by file leaves a window in which the CLI is new and the tray is
//! old, and a failure halfway through leaves no way back. A transaction
//! stages every file first, verifies the whole set, and only then moves
//! them into place — restoring what it displaced if any move fails.

use std::path::{Path, PathBuf};

use super::atomic;
use super::layout::InstallRoot;

/// A file waiting in staging for its destination.
struct Staged {
    name: String,
    staged_at: PathBuf,
    destination: PathBuf,
    executable: bool,
}

/// A file already moved into place, with whatever it displaced.
struct Committed {
    destination: PathBuf,
    displaced: Option<PathBuf>,
}

pub struct Transaction {
    staging: PathBuf,
    backup: PathBuf,
    staged: Vec<Staged>,
    committed: Vec<Committed>,
    finished: bool,
}

impl Transaction {
    /// Open a transaction against an install root. Staging and backup
    /// both live inside the prefix so every move is a same-filesystem
    /// rename rather than a copy that could itself fail halfway.
    pub fn begin(root: &InstallRoot) -> Result<Self, String> {
        let staging = root.staging_dir();
        let backup = root.backup_dir();
        // Debris from a process that died mid-transaction is not state to
        // preserve; the manifest is the record of truth.
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&backup);
        std::fs::create_dir_all(&staging)
            .map_err(|e| format!("mkdir {}: {e}", staging.display()))?;
        std::fs::create_dir_all(&backup).map_err(|e| format!("mkdir {}: {e}", backup.display()))?;
        Ok(Self {
            staging,
            backup,
            staged: Vec::new(),
            committed: Vec::new(),
            finished: false,
        })
    }

    fn staging_path(&self, name: &str) -> PathBuf {
        // Flatten into staging: two components may share a basename in
        // different destination directories.
        self.staging
            .join(name.replace(std::path::MAIN_SEPARATOR, "_"))
    }

    /// Stage bytes destined for `destination`.
    pub fn stage_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        destination: PathBuf,
        executable: bool,
    ) -> Result<(), String> {
        let staged_at = self.staging_path(name);
        if executable {
            atomic::write_executable(&staged_at, bytes)?;
        } else {
            atomic::write(&staged_at, bytes)?;
        }
        self.staged.push(Staged {
            name: name.to_string(),
            staged_at,
            destination,
            executable,
        });
        Ok(())
    }

    /// Stage an existing file destined for `destination`.
    pub fn stage_file(
        &mut self,
        name: &str,
        source: &Path,
        destination: PathBuf,
        executable: bool,
    ) -> Result<(), String> {
        let bytes = std::fs::read(source).map_err(|e| format!("read {}: {e}", source.display()))?;
        self.stage_bytes(name, &bytes, destination, executable)
    }

    pub fn is_empty(&self) -> bool {
        self.staged.is_empty()
    }

    /// Move every staged file into place. On the first failure, every
    /// file already moved is restored and the error is returned, so the
    /// install is left exactly as it was found.
    pub fn commit(mut self) -> Result<(), String> {
        let staged = std::mem::take(&mut self.staged);
        for item in staged {
            if let Err(e) = self.place(&item) {
                let restore = self.rollback_committed();
                self.finished = true;
                self.cleanup();
                return Err(match restore {
                    Ok(()) => format!("{e} (no changes were kept)"),
                    Err(r) => format!("{e}; rollback also failed: {r}"),
                });
            }
        }
        self.finished = true;
        self.cleanup();
        Ok(())
    }

    fn place(&mut self, item: &Staged) -> Result<(), String> {
        // Every failure in this function names the component, so an
        // operator reading one line knows which piece of the release
        // could not be placed and where.
        if let Some(parent) = item.destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "install {}: cannot create {}: {e}",
                    item.name,
                    parent.display()
                )
            })?;
        }
        // Preserve whatever is there so a later failure can put it back.
        let displaced = if item.destination.exists() {
            let keep = self.backup.join(format!(
                "{}.{}",
                item.name.replace(std::path::MAIN_SEPARATOR, "_"),
                std::process::id()
            ));
            std::fs::rename(&item.destination, &keep).map_err(|e| {
                format!(
                    "install {}: cannot set aside existing {}: {e}",
                    item.name,
                    item.destination.display()
                )
            })?;
            Some(keep)
        } else {
            None
        };
        match std::fs::rename(&item.staged_at, &item.destination) {
            Ok(()) => {
                if item.executable {
                    atomic::set_executable(&item.destination)?;
                }
                self.committed.push(Committed {
                    destination: item.destination.clone(),
                    displaced,
                });
                Ok(())
            }
            Err(e) => {
                // Put back what we displaced before reporting, so this
                // one destination is already whole.
                if let Some(keep) = displaced {
                    let _ = std::fs::rename(&keep, &item.destination);
                }
                Err(format!(
                    "install {} to {}: {e}",
                    item.name,
                    item.destination.display()
                ))
            }
        }
    }

    fn rollback_committed(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        // Reverse order so a destination touched twice ends on its
        // original contents.
        for done in self.committed.drain(..).rev() {
            let _ = std::fs::remove_file(&done.destination);
            if let Some(keep) = done.displaced
                && let Err(e) = std::fs::rename(&keep, &done.destination)
            {
                failures.push(format!("restore {}: {e}", done.destination.display()));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.staging);
        let _ = std::fs::remove_dir_all(&self.backup);
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        // Dropped without commit: the caller bailed out. Undo anything
        // already placed and leave no scratch directories behind.
        if !self.finished {
            let _ = self.rollback_committed();
            self.cleanup();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, InstallRoot) {
        let d = tempfile::tempdir().unwrap();
        let r = InstallRoot::at(d.path().to_path_buf());
        (d, r)
    }

    #[test]
    fn commit_places_every_staged_file_and_clears_scratch() {
        let (_d, root) = root();
        let a = root.bin_dir().join("vak");
        let b = root.bin_dir().join("vak-delivery-worker");
        let mut tx = Transaction::begin(&root).unwrap();
        tx.stage_bytes("vak", b"new-cli", a.clone(), true).unwrap();
        tx.stage_bytes("vak-delivery-worker", b"new-tray", b.clone(), true)
            .unwrap();
        tx.commit().unwrap();

        assert_eq!(std::fs::read(&a).unwrap(), b"new-cli");
        assert_eq!(std::fs::read(&b).unwrap(), b"new-tray");
        assert!(!root.staging_dir().exists());
        assert!(!root.backup_dir().exists());
    }

    #[test]
    fn a_failed_placement_restores_every_earlier_file() {
        let (_d, root) = root();
        let good = root.bin_dir().join("vak");
        std::fs::create_dir_all(root.bin_dir()).unwrap();
        std::fs::write(&good, b"old-cli").unwrap();

        // Second destination sits under a path whose parent is a regular
        // file, so creating its directory fails and the transaction must
        // put the first file back. (A directory at the destination is not
        // enough: it would simply be moved aside into backup and the
        // placement would succeed.)
        let wall = root.bin_dir().join("not-a-dir");
        std::fs::write(&wall, b"regular file").unwrap();
        let blocked = wall.join("vak-delivery-worker");

        let mut tx = Transaction::begin(&root).unwrap();
        tx.stage_bytes("vak", b"new-cli", good.clone(), true)
            .unwrap();
        tx.stage_bytes("vak-delivery-worker", b"new-tray", blocked, true)
            .unwrap();
        let err = tx.commit().unwrap_err();

        assert!(
            err.contains("vak-delivery-worker"),
            "error names the failure: {err}"
        );
        assert_eq!(
            std::fs::read(&good).unwrap(),
            b"old-cli",
            "the earlier file must be back to its original contents"
        );
        assert!(!root.staging_dir().exists());
        assert!(!root.backup_dir().exists());
    }

    #[test]
    fn dropping_without_commit_leaves_the_install_untouched() {
        let (_d, root) = root();
        let target = root.bin_dir().join("vak");
        std::fs::create_dir_all(root.bin_dir()).unwrap();
        std::fs::write(&target, b"old").unwrap();
        {
            let mut tx = Transaction::begin(&root).unwrap();
            tx.stage_bytes("vak", b"new", target.clone(), true).unwrap();
            // No commit: the guard must undo staging on the way out.
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!root.staging_dir().exists());
    }

    #[test]
    fn stale_scratch_from_a_killed_process_is_discarded_on_begin() {
        let (_d, root) = root();
        std::fs::create_dir_all(root.staging_dir()).unwrap();
        std::fs::write(root.staging_dir().join("debris"), b"x").unwrap();
        let tx = Transaction::begin(&root).unwrap();
        assert!(tx.is_empty());
        assert!(!root.staging_dir().join("debris").exists());
    }
}
