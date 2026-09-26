//! Shared install-root resolution (docs/design/32-release-engineering.md).
//!
//! `vak self install/status/verify` (crate `vak`, via
//! `install::layout::InstallRoot`) and `vak_core::health`'s "self version
//! parity" check must agree on where the installed-release manifest
//! lives. `vak-core` cannot depend on the `vak` binary crate (the
//! dependency runs the other way), so this module is the single source
//! of truth for manifest-path resolution and `vak::install::layout`
//! delegates to it instead of re-deriving the platform default.
//!
//! Before this module existed, `vak_core::health::install_manifest_path`
//! hardcoded the Linux `~/.local/share/vak/...`-style layout
//! (`home.join("local/release/install.json")`) for every platform,
//! including macOS, where `self install` actually writes the manifest
//! into the app bundle at `<prefix>/Contents/Resources/install.json`.
//! `vak doctor` reported "no installed release manifest" unconditionally
//! on macOS even for a correctly installed app.

use std::path::{Path, PathBuf};

/// Application bundle name on macOS. Matches `productName` in
/// `tauri.conf.json`; the desktop bundle and the managed install are the
/// same directory, so the two spellings must never diverge.
pub const BUNDLE_NAME: &str = "Vakyartha.app";

/// Overrides the install prefix without threading `--prefix` through
/// every invocation. Useful for staging and for tests.
pub const PREFIX_ENV: &str = "VAK_PREFIX";

const MANIFEST_FILE: &str = "install.json";

/// True when `root` is a macOS application bundle, which puts the
/// manifest (and executables) under `Contents/...` rather than beside
/// the prefix.
pub fn is_bundle(root: &Path) -> bool {
    root.extension().and_then(|e| e.to_str()) == Some("app")
}

/// Where the install manifest lives under a given prefix, accounting for
/// the bundle layout.
pub fn manifest_path_for_prefix(prefix: &Path) -> PathBuf {
    if is_bundle(prefix) {
        prefix
            .join("Contents")
            .join("Resources")
            .join(MANIFEST_FILE)
    } else {
        prefix.join(MANIFEST_FILE)
    }
}

/// Platform default install root. macOS gets a real application bundle
/// so the app is launchable from Finder; other platforms get a managed
/// prefix under the XDG/canonical data home.
pub fn platform_default_prefix() -> PathBuf {
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

/// Resolve the install manifest path the same way `InstallRoot::resolve`
/// resolves the prefix: explicit prefix, then `VAK_PREFIX`, then the
/// platform default.
pub fn resolve_manifest_path(explicit_prefix: Option<PathBuf>) -> PathBuf {
    let prefix = explicit_prefix
        .or_else(|| {
            std::env::var_os(PREFIX_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(platform_default_prefix);
    manifest_path_for_prefix(&prefix)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn plain_prefix_puts_manifest_beside_it() {
        let path = manifest_path_for_prefix(Path::new("/opt/vak"));
        assert_eq!(path, PathBuf::from("/opt/vak/install.json"));
    }

    #[test]
    fn bundle_prefix_puts_manifest_inside_contents_resources() {
        let path = manifest_path_for_prefix(Path::new("/Applications/Vakyartha.app"));
        assert_eq!(
            path,
            PathBuf::from("/Applications/Vakyartha.app/Contents/Resources/install.json")
        );
    }

    #[test]
    fn explicit_prefix_wins_over_default() {
        let path = resolve_manifest_path(Some(PathBuf::from("/tmp/vak-explicit")));
        assert_eq!(path, PathBuf::from("/tmp/vak-explicit/install.json"));
    }
}
