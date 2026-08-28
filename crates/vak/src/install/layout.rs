//! Where a managed install lives, and how its pieces are addressed.
//!
//! Every `self` subcommand resolves the install root through
//! [`InstallRoot::resolve`] so that `--prefix` means the same thing to
//! `install`, `status`, `update`, `services-sync`, and `uninstall`. A
//! prefix that only `install` understood was the source of installs that
//! could not afterwards be inspected or removed.

use std::path::{Path, PathBuf};

/// Application bundle name on macOS. Matches `productName` in
/// `tauri.conf.json`; the desktop bundle and the managed install are the
/// same directory, so the two spellings must never diverge. Note that
/// macOS volumes are case-insensitive by default — `Vak.app` and
/// `vak.app` are the same path, so a second spelling is not a
/// second location, it is a collision.
pub const BUNDLE_NAME: &str = "Vak.app";

/// Overrides the install prefix without threading `--prefix` through
/// every invocation. Useful for staging and for tests.
pub const PREFIX_ENV: &str = "VAK_PREFIX";

const BIN_DIR: &str = "bin";
const MANIFEST_FILE: &str = "install.json";
const STAGING_DIR: &str = ".staging";
const BACKUP_DIR: &str = ".backup";

/// A resolved install root together with the layout it implies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRoot {
    prefix: PathBuf,
    bundle: bool,
}

impl InstallRoot {
    /// Resolve the install root, in precedence order:
    /// explicit `--prefix`, then `VAK_PREFIX`, then the platform
    /// default. The first two are taken literally so callers can install
    /// to a plain directory on any platform.
    pub fn resolve(explicit: Option<PathBuf>) -> Self {
        if let Some(p) = explicit {
            return Self::at(p);
        }
        if let Some(p) = std::env::var_os(PREFIX_ENV).filter(|v| !v.is_empty()) {
            return Self::at(PathBuf::from(p));
        }
        Self::at(platform_default())
    }

    /// Treat `prefix` as an install root, inferring the layout from it.
    pub fn at(prefix: PathBuf) -> Self {
        let bundle = is_bundle(&prefix);
        Self { prefix, bundle }
    }

    pub fn prefix(&self) -> &Path {
        &self.prefix
    }

    /// True when the root is a macOS application bundle, which puts
    /// executables under `Contents/MacOS` rather than `bin/`.
    pub fn is_bundle(&self) -> bool {
        self.bundle
    }

    pub fn bin_dir(&self) -> PathBuf {
        if self.bundle {
            self.prefix.join("Contents").join("MacOS")
        } else {
            self.prefix.join(BIN_DIR)
        }
    }

    pub fn manifest_path(&self) -> PathBuf {
        if self.bundle {
            self.prefix
                .join("Contents")
                .join("Resources")
                .join(MANIFEST_FILE)
        } else {
            self.prefix.join(MANIFEST_FILE)
        }
    }

    /// Frontend assets and the manifest live here in a bundle; plain
    /// prefixes keep them beside the binaries.
    pub fn resources_dir(&self) -> PathBuf {
        if self.bundle {
            self.prefix.join("Contents").join("Resources")
        } else {
            self.prefix.join("share")
        }
    }

    /// Scratch space for a transaction, on the same filesystem as the
    /// destination so the final move is an atomic rename rather than a
    /// cross-device copy.
    pub fn staging_dir(&self) -> PathBuf {
        self.prefix.join(STAGING_DIR)
    }

    /// Where displaced files wait until a transaction commits.
    pub fn backup_dir(&self) -> PathBuf {
        self.prefix.join(BACKUP_DIR)
    }

    /// True when something is already installed here.
    pub fn is_installed(&self) -> bool {
        self.manifest_path().exists()
    }
}

/// Platform default install root. macOS gets a real application bundle so
/// the app is launchable from Finder; other platforms get a managed
/// prefix under the data home.
fn platform_default() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let system = PathBuf::from("/Applications").join(BUNDLE_NAME);
        if system.exists() || dir_writable(Path::new("/Applications")) {
            return system;
        }
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h).join("Applications").join(BUNDLE_NAME);
        }
        system
    }
    #[cfg(not(target_os = "macos"))]
    {
        vak_config::paths::data_home().join("local").join("release")
    }
}

#[cfg(target_os = "macos")]
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".vak-write-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn is_bundle(root: &Path) -> bool {
    root.extension().and_then(|e| e.to_str()) == Some("app")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn explicit_prefix_wins_and_is_taken_literally() {
        let root = InstallRoot::resolve(Some(PathBuf::from("/tmp/vak-explicit")));
        assert_eq!(root.prefix(), Path::new("/tmp/vak-explicit"));
        assert!(!root.is_bundle(), "a plain directory is not a bundle");
        assert_eq!(root.bin_dir(), PathBuf::from("/tmp/vak-explicit/bin"));
        assert_eq!(
            root.manifest_path(),
            PathBuf::from("/tmp/vak-explicit/install.json")
        );
    }

    #[test]
    fn bundle_layout_puts_binaries_and_manifest_inside_contents() {
        let root = InstallRoot::at(PathBuf::from("/Applications/Vak.app"));
        assert!(root.is_bundle());
        assert_eq!(
            root.bin_dir(),
            PathBuf::from("/Applications/Vak.app/Contents/MacOS")
        );
        assert_eq!(
            root.manifest_path(),
            PathBuf::from("/Applications/Vak.app/Contents/Resources/install.json")
        );
    }

    #[test]
    fn staging_and_backup_sit_inside_the_prefix() {
        // Same filesystem as the destination, so committing a
        // transaction is a rename and never a cross-device copy.
        let root = InstallRoot::at(PathBuf::from("/opt/vak"));
        assert!(root.staging_dir().starts_with(root.prefix()));
        assert!(root.backup_dir().starts_with(root.prefix()));
    }
}
