#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Guards `paths.rs`'s claim to be THE source of truth for the data home.
//!
//! The 0.7 drift incident is why this module owns every home path and why
//! every entry point reads from it. There is no migration step and no legacy
//! dotdir: sessions, gateway state, config, and the shared workspace layer land
//! only in the canonical `data_home()` / `cache_home()` / `logs_dir()`
//! locations, so the tree never has a second home to drift from. The check
//! below fails bluntly on any new hand-rolled `HOME`-relative or
//! `~/.config/vak` path and points the author at `vak_config::paths`.
//!
//! This test reads the workspace's own source. It is deliberately blunt: any
//! new hand-rolled home path fails here with the file and line, and the fix is
//! to call `vak_config::paths`.

use std::path::{Path, PathBuf};

/// Sites that legitimately need the operating-system account home rather than
/// vak's data home, with the reason each is exempt.
const ALLOWED: &[(&str, &str)] = &[
    (
        "crates/vak-config/src/paths.rs",
        "the module that defines the layout",
    ),
    (
        "crates/vak-core/src/install.rs",
        "macOS bundle prefix under ~/Applications",
    ),
    (
        "crates/vak-ops/src/lib.rs",
        "launchd/systemd unit paths are account-home-relative by definition",
    ),
    (
        "crates/vak-tools/src/sandbox.rs",
        "read-allowlists real toolchain dirs (~/.cargo, ~/.rustup)",
    ),
    (
        "crates/vak-tools/src/landlock.rs",
        "read-allowlists real toolchain dirs (~/.cargo, ~/.rustup)",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name == "target" || name == "node_modules" || name == "dist" || name == "tests" {
                continue;
            }
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn is_exempt(relative: &str) -> bool {
    ALLOWED.iter().any(|(path, _)| *path == relative)
}

#[test]
fn no_crate_rebuilds_the_data_home_by_hand() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_sources(&root.join("crates"), &mut files);
    assert!(!files.is_empty(), "found no sources to scan");

    let mut offenders = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if is_exempt(&relative) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let mut in_tests = false;
        for (index, line) in text.lines().enumerate() {
            if line.contains("#[cfg(test)]") {
                in_tests = true;
            }
            if in_tests {
                continue;
            }
            let reads_account_home =
                line.contains("var_os(\"HOME\")") || line.contains("var(\"HOME\")");
            // Probing whether HOME exists is not layout: `Core::new` falls
            // back to a cwd-local home in environments that have none.
            let is_probe = line.contains(".is_none()") || line.contains(".is_some()");
            let builds_a_path =
                line.contains("PathBuf") || line.contains(".join(") || line.contains("Path::new");
            let invents_a_convention = line.contains("\".config/vak");
            if (reads_account_home && builds_a_path && !is_probe) || invents_a_convention {
                offenders.push(format!(
                    "{relative}:{}: {}",
                    index + 1,
                    line.trim().chars().take(90).collect::<String>()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these sites resolve the home directory by hand instead of through \
         vak_config::paths — route them through data_home()/cache_home()/\
         logs_dir(), or add a justified exemption to ALLOWED:\n  {}",
        offenders.join("\n  ")
    );
}

/// Exemptions are load-bearing: a stale one silently re-opens the hole it
/// was granted for. If a file moves or stops needing the account home, its
/// entry must go with it.
#[test]
fn every_exemption_still_points_at_a_real_file_that_needs_it() {
    let root = workspace_root();
    for (relative, reason) in ALLOWED {
        let path = root.join(relative);
        assert!(
            path.is_file(),
            "exemption for '{relative}' ({reason}) names a file that no longer exists"
        );
        let text = std::fs::read_to_string(&path).expect("read exempt file");
        assert!(
            text.contains("var_os(\"HOME\")")
                || text.contains("var(\"HOME\")")
                || text.contains(".config/vak"),
            "'{relative}' no longer resolves the account home; drop its exemption"
        );
    }
}
