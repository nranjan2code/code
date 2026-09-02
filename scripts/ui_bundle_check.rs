// Shared by `vak-server/build.rs` and `vak-desktop/build.rs` via `include!`.
//
// Both crates embed or ship a frontend bundle built by a separate `npm run
// build`, and both have the same trap: editing the UI source and running
// `cargo build` produces a binary carrying the previous bundle, with nothing
// anywhere to say so. `npm run build` writes `dist/.src-manifest` (one
// sha256 per source file); this re-verifies it.
//
// `include!` rather than a shared crate because a build script cannot depend
// on a workspace member without making that member a build-dependency of
// every consumer — a lot of machinery for sixty lines.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// `Ok(())` when dist/ was built from exactly the sources on disk.
///
/// A missing manifest is accepted: a checkout whose dist/ predates the
/// manifest, or a vendored copy, must still build. The check tightens on its
/// own the first time anyone runs `npm run build`.
fn check_bundle_matches_source(ui: &Path) -> Result<(), String> {
    let manifest_path = ui.join("dist/.src-manifest");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        return Ok(());
    };

    let mut listed: Vec<(String, String)> = Vec::new();
    for line in manifest.lines().filter(|l| !l.trim().is_empty()) {
        let Some((digest, rel)) = line.split_once("  ") else {
            return Err(format!(
                "{}: malformed line {line:?}",
                manifest_path.display()
            ));
        };
        listed.push((rel.to_string(), digest.to_string()));
    }

    let mut stale = Vec::new();
    for (rel, expected) in &listed {
        match std::fs::read(ui.join(rel)) {
            Ok(bytes) => {
                let actual = format!("{:x}", Sha256::digest(&bytes));
                if &actual != expected {
                    stale.push(format!("{rel} changed"));
                }
            }
            Err(_) => stale.push(format!("{rel} was deleted")),
        }
    }
    // A brand-new file the bundle never saw is just as stale as an edited
    // one, and is the easier case to miss.
    for path in ui_walk(&ui.join("src")) {
        let rel = path
            .strip_prefix(ui)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if !listed.iter().any(|(listed, _)| listed == &rel) {
            stale.push(format!("{rel} is new"));
        }
    }

    if stale.is_empty() {
        return Ok(());
    }
    stale.sort();
    Err(format!(
        "dist is stale — the shipped frontend would not match its source ({})",
        stale.join(", ")
    ))
}

fn ui_walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(ui_walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}
