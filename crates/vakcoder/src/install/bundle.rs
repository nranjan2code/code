//! macOS application-bundle metadata.
//!
//! The managed install and the desktop app are the same `.app`, so the
//! bundle identity here must agree with `crates/vak-desktop/tauri.conf.json`.
//! `scripts/check-version.sh` enforces that they do.

use std::path::Path;

use super::atomic;
use super::layout::InstallRoot;

/// Bundle identifier, matching `tauri.conf.json`.
pub const IDENTIFIER: &str = "dev.vakcoder.desktop";

/// `LSUIElement` keeps the tray out of the Dock: a menu-bar agent, per
/// the macOS HIG.
pub fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>VakCoder</string>
  <key>CFBundleDisplayName</key><string>VakCoder</string>
  <key>CFBundleIdentifier</key><string>{IDENTIFIER}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>vakcoder-tray</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    )
}

/// Write `Info.plist` and copy the desktop frontend assets into
/// `Contents/Resources`, so a bundle install is launchable from Finder.
pub fn write_metadata(
    root: &InstallRoot,
    version: &str,
    assets: Option<&Path>,
) -> Result<(), String> {
    if !root.is_bundle() {
        return Ok(());
    }
    let contents = root.prefix().join("Contents");
    std::fs::create_dir_all(contents.join("Resources"))
        .map_err(|e| format!("mkdir {}: {e}", contents.display()))?;
    // Plist first: a bundle without one is not launchable as an app.
    atomic::write(&contents.join("Info.plist"), info_plist(version).as_bytes())?;
    if let Some(dist) = assets.filter(|p| p.exists()) {
        atomic::copy_dir(dist, &root.resources_dir())?;
    }
    Ok(())
}

/// Locate the built desktop frontend relative to the running binary,
/// covering both a dev tree (`target/release/vakcoder`) and an installed
/// bundle (`Contents/MacOS/vakcoder`).
pub fn locate_frontend_assets() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..5 {
        let candidate = dir.join("crates/vak-desktop/ui/dist");
        if candidate.exists() {
            return Some(candidate);
        }
        dir = dir.parent()?.to_path_buf();
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn plist_carries_the_version_it_was_given() {
        let p = info_plist("1.2.3");
        assert!(p.contains("<key>CFBundleShortVersionString</key><string>1.2.3</string>"));
        assert!(p.contains(IDENTIFIER));
        assert!(
            p.contains("<key>LSUIElement</key><true/>"),
            "stays out of the Dock"
        );
    }

    #[test]
    fn metadata_is_a_no_op_for_a_plain_prefix() {
        let d = tempfile::tempdir().unwrap();
        let root = InstallRoot::at(d.path().to_path_buf());
        write_metadata(&root, "1.0.0", None).unwrap();
        assert!(!d.path().join("Contents").exists());
    }

    #[test]
    fn bundle_metadata_writes_a_launchable_plist() {
        let d = tempfile::tempdir().unwrap();
        let root = InstallRoot::at(d.path().join("VakCoder.app"));
        write_metadata(&root, "0.8.0", None).unwrap();
        let plist = std::fs::read_to_string(root.prefix().join("Contents/Info.plist")).unwrap();
        assert!(plist.contains("0.8.0"));
    }
}
