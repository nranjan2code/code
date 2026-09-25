//! macOS application-bundle metadata.
//!
//! The managed install and the desktop app are the same `.app`, so the
//! bundle identity here must agree with `crates/vak-desktop/tauri.conf.json`.
//! `scripts/check-version.sh` enforces that they do.

use std::path::Path;

use super::atomic;
use super::layout::InstallRoot;

/// Bundle identifier, matching `tauri.conf.json`.
pub const IDENTIFIER: &str = "dev.vak.desktop";

/// The bundle launches `vak-desktop`, the actual application.
///
/// It used to launch `vak-tray` with `LSUIElement`, which made the
/// whole `.app` a background menu-bar agent. That had a fatal
/// consequence: `com.vak.tray` runs the same binary as a launchd
/// service, so macOS considered the app already running and answered a
/// double-click in Finder by sending an activate event to that existing
/// process rather than launching anything. No new process meant no code
/// of ours ran at all -- double-clicking Vak did nothing visible,
/// and no amount of logic inside the tray's startup could have fixed it,
/// because startup never happened.
///
/// `vak-desktop` is what a user double-clicking the app is asking for,
/// and it already handles being launched twice: `tauri_plugin_single_instance`
/// refocuses the open window instead of starting a rival instance. The
/// tray keeps its menu-bar-only presence by setting
/// `ActivationPolicy::Accessory` on its own event loop, so it no longer
/// depends on a bundle-wide `LSUIElement` that would also have hidden
/// the app this bundle now launches.
pub fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Vakyartha</string>
  <key>CFBundleDisplayName</key><string>Vakyartha</string>
  <key>CFBundleIdentifier</key><string>{IDENTIFIER}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleExecutable</key><string>vak-desktop</string>
  <key>CFBundleIconFile</key><string>icon.icns</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    )
}

/// Refuse a bundle install that would produce an unlaunchable app.
///
/// Separate from [`write_metadata`] so it can run **before** the install
/// transaction commits. Failing after the commit left the binaries in place
/// with no manifest beside them — a half-install that `status` reads as
/// broken and `verify` cannot even find, which is a worse outcome than
/// either finishing or not starting.
pub fn require_frontend(
    root: &InstallRoot,
    assets: Option<&Path>,
    desktop: bool,
) -> Result<(), String> {
    if !root.is_bundle() || !desktop || assets.is_some_and(Path::exists) {
        return Ok(());
    }
    Err(
        "the desktop app is being installed into a bundle, but its frontend was not found at \
         crates/vak-client-ui/dist. The installed app would open a blank window. Build it \
         first:\n    cd crates/vak-client-ui && npm ci && npm run build\n(or install without \
         the desktop app: scripts/build.sh --no-desktop)"
            .to_string(),
    )
}

/// Write `Info.plist` and copy the desktop frontend assets into
/// `Contents/Resources`, so a bundle install is launchable from Finder.
///
/// `desktop` says whether the `vak-desktop` binary is part of this install.
/// When it is, the frontend is **required**: `tauri.conf.json` points
/// `frontendDist` at `crates/vak-client-ui/dist`, and the installed app
/// loads that copy out of `Contents/Resources`. Without it the app opens a
/// blank window with nothing in the console to explain why.
///
/// This used to be a silent skip — `if let Some(dist) = assets.filter(...)`
/// with no else — so `vak self install` into the default macOS prefix with
/// no built frontend reported success, wrote a manifest, and produced an
/// app that did not work. `verify` then passed, because it only looked at
/// the binaries. An install that cannot run is a failed install, and it has
/// to say so at the moment it happens.
pub fn write_metadata(
    root: &InstallRoot,
    version: &str,
    assets: Option<&Path>,
    desktop: bool,
) -> Result<(), String> {
    if !root.is_bundle() {
        return Ok(());
    }
    let contents = root.prefix().join("Contents");
    std::fs::create_dir_all(contents.join("Resources"))
        .map_err(|e| format!("mkdir {}: {e}", contents.display()))?;
    // Plist first: a bundle without one is not launchable as an app.
    atomic::write(&contents.join("Info.plist"), info_plist(version).as_bytes())?;
    require_frontend(root, assets, desktop)?;
    let dist = assets.filter(|p| p.exists());
    if let Some(dist) = dist {
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
    // A bundle whose Info.plist names an icon (above) but never carries
    // one shows Finder's generic placeholder — silently, since a missing
    // CFBundleIconFile target is not an error macOS surfaces anywhere.
    if let Some(icon) = locate_icon() {
        std::fs::copy(&icon, contents.join("Resources/icon.icns"))
            .map_err(|e| format!("copy icon {}: {e}", icon.display()))?;
    }
    Ok(())
}

/// Locate the built desktop frontend relative to the running binary,
/// covering both a dev tree (`target/release/vak`) and an installed
/// bundle (`Contents/MacOS/vak`).
pub fn locate_frontend_assets() -> Option<std::path::PathBuf> {
    locate_repo_relative("crates/vak-client-ui/dist")
}

/// Locate the app icon (`.icns`), same search shape as
/// [`locate_frontend_assets`] — both are repo-relative resources a
/// release binary needs to find without knowing where the checkout is.
pub fn locate_icon() -> Option<std::path::PathBuf> {
    locate_repo_relative("crates/vak-desktop/icons/icon.icns")
}

/// Locate the feed runtime bundled beside a release binary or in the source
/// checkout. The server executes these scripts as a subprocess, so an
/// install must carry the complete runtime rather than relying on the user's
/// workspace containing the repository's `scripts/feeds` tree.
pub fn locate_feed_assets() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let bin_dir = exe.parent()?;
    let mut candidates = vec![
        bin_dir.join("feeds"),
        bin_dir.join("..").join("Resources").join("feeds"),
        bin_dir.join("..").join("share").join("vak").join("feeds"),
        bin_dir
            .join("..")
            .join("..")
            .join("share")
            .join("vak")
            .join("feeds"),
    ];
    if let Some(repo_assets) = locate_repo_relative("scripts/feeds") {
        candidates.push(repo_assets);
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.join("feed_ingest.py").exists())
}

fn locate_repo_relative(rel: &str) -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..5 {
        let candidate = dir.join(rel);
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
        // The bundle launches the app the user double-clicks, not the
        // background menu-bar agent. When it launched the tray under
        // LSUIElement, macOS treated the app as already running (the
        // tray also runs as a launchd service) and answered a
        // double-click by activating that process instead of launching
        // anything -- so opening Vak did nothing at all.
        assert!(
            p.contains("<key>CFBundleExecutable</key><string>vak-desktop</string>"),
            "the bundle must launch the desktop app"
        );
        assert!(
            !p.contains("LSUIElement"),
            "a bundle-wide LSUIElement would hide the app this bundle launches; \
             the tray sets ActivationPolicy::Accessory on its own event loop instead"
        );
    }

    #[test]
    fn metadata_is_a_no_op_for_a_plain_prefix() {
        let d = tempfile::tempdir().unwrap();
        let root = InstallRoot::at(d.path().to_path_buf());
        write_metadata(&root, "1.0.0", None, false).unwrap();
        assert!(!d.path().join("Contents").exists());
    }

    #[test]
    fn bundle_metadata_writes_a_launchable_plist() {
        let d = tempfile::tempdir().unwrap();
        let root = InstallRoot::at(d.path().join("Vak.app"));
        write_metadata(&root, "0.8.0", None, false).unwrap();
        let plist = std::fs::read_to_string(root.prefix().join("Contents/Info.plist")).unwrap();
        assert!(plist.contains("0.8.0"));
        assert!(
            plist.contains("<key>CFBundleIconFile</key><string>icon.icns</string>"),
            "a bundle with no icon key falls back to Finder's generic placeholder"
        );
    }

    #[test]
    fn bundle_metadata_carries_the_real_icon_when_one_is_found_relative_to_the_test_binary() {
        // This test binary runs from target/debug, still inside the real
        // checkout, so locate_icon() finds the genuine icon the same way
        // a release binary would -- proving the copy actually happens,
        // not just that the plist names a file that isn't there.
        let d = tempfile::tempdir().unwrap();
        let root = InstallRoot::at(d.path().join("Vak.app"));
        write_metadata(&root, "0.8.0", None, false).unwrap();
        if locate_icon().is_some() {
            let copied = root.prefix().join("Contents/Resources/icon.icns");
            assert!(copied.is_file(), "icon.icns must be copied into the bundle");
            assert!(std::fs::metadata(&copied).unwrap().len() > 0);
        }
    }

    #[test]
    fn reinstall_removes_the_previous_builds_hashed_assets() {
        // Every rebuild of the frontend produces new content-hashed
        // filenames; copy_dir only adds, it never removes. Without
        // clearing the destination first, a reinstall accumulated every
        // prior build's JS and CSS forever.
        let d = tempfile::tempdir().unwrap();
        let root = InstallRoot::at(d.path().join("Vak.app"));

        let first_dist = d.path().join("dist-v1");
        std::fs::create_dir_all(first_dist.join("assets")).unwrap();
        std::fs::write(first_dist.join("assets/index-OLDHASH.js"), b"old").unwrap();
        std::fs::write(
            first_dist.join("index.html"),
            b"<script src=assets/index-OLDHASH.js>",
        )
        .unwrap();
        write_metadata(&root, "1.0.0", Some(&first_dist), true).unwrap();
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
        write_metadata(&root, "1.0.1", Some(&second_dist), true).unwrap();

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

    /// The regression this whole change exists for: a bundle install that
    /// carries the desktop app but no frontend used to succeed, and the
    /// installed app opened a blank window.
    #[test]
    fn a_bundle_with_the_desktop_app_and_no_frontend_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("Vak.app");
        std::fs::create_dir_all(prefix.join("Contents/MacOS")).unwrap();
        let root = InstallRoot::resolve(Some(prefix));

        let err = write_metadata(&root, "1.0.0", None, true)
            .expect_err("a desktop bundle with no frontend must not install");
        assert!(
            err.contains("blank window") && err.contains("npm run build"),
            "the error must name the consequence and the fix, got: {err}"
        );

        // Without the desktop app there is no frontend to require.
        write_metadata(&root, "1.0.0", None, false).expect("a CLI-only bundle needs no frontend");
    }

    /// An assets path that is recorded but absent is the same failure as
    /// no path at all, and was the likelier of the two: a stale
    /// `locate_frontend_assets` hit after someone deleted `dist/`.
    #[test]
    fn a_frontend_path_that_does_not_exist_is_refused_too() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("Vak.app");
        std::fs::create_dir_all(prefix.join("Contents/MacOS")).unwrap();
        let root = InstallRoot::resolve(Some(prefix));

        let gone = dir.path().join("was-here");
        assert!(write_metadata(&root, "1.0.0", Some(&gone), true).is_err());
    }
}
