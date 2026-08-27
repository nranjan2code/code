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
        // Every frontend rebuild produces new content-hashed filenames
        // (Vite), and copy_dir only ever adds files, never removes ones
        // absent from the source. Without clearing the destination
        // first, every reinstall left the previous build's JS and CSS
        // behind alongside the new one — harmless to which file
        // actually gets served (index.html always names the current
        // hash), but unbounded bloat, and confusing to anyone
        // inspecting the bundle who has no way to tell which files are
        // live. Only the hashed subtree is cleared, not all of
        // Resources: that directory also holds install.json (written
        // after this function returns) and Info.plist (written above).
        let stale_assets = root.resources_dir().join("assets");
        if stale_assets.exists() {
            std::fs::remove_dir_all(&stale_assets)
                .map_err(|e| format!("clear stale assets at {}: {e}", stale_assets.display()))?;
        }
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

    #[test]
    fn reinstall_removes_the_previous_builds_hashed_assets() {
        // Every rebuild of the frontend produces new content-hashed
        // filenames; copy_dir only adds, it never removes. Without
        // clearing the destination first, a reinstall accumulated every
        // prior build's JS and CSS forever.
        let d = tempfile::tempdir().unwrap();
        let root = InstallRoot::at(d.path().join("VakCoder.app"));

        let first_dist = d.path().join("dist-v1");
        std::fs::create_dir_all(first_dist.join("assets")).unwrap();
        std::fs::write(first_dist.join("assets/index-OLDHASH.js"), b"old").unwrap();
        std::fs::write(
            first_dist.join("index.html"),
            b"<script src=assets/index-OLDHASH.js>",
        )
        .unwrap();
        write_metadata(&root, "1.0.0", Some(&first_dist)).unwrap();
        assert!(
            root.resources_dir()
                .join("assets/index-OLDHASH.js")
                .exists()
        );

        let second_dist = d.path().join("dist-v2");
        std::fs::create_dir_all(second_dist.join("assets")).unwrap();
        std::fs::write(second_dist.join("assets/index-NEWHASH.js"), b"new").unwrap();
        std::fs::write(
            second_dist.join("index.html"),
            b"<script src=assets/index-NEWHASH.js>",
        )
        .unwrap();
        write_metadata(&root, "1.0.1", Some(&second_dist)).unwrap();

        assert!(
            !root
                .resources_dir()
                .join("assets/index-OLDHASH.js")
                .exists(),
            "the previous build's asset must be gone, not merely superseded"
        );
        assert!(
            root.resources_dir()
                .join("assets/index-NEWHASH.js")
                .exists()
        );
    }
}
