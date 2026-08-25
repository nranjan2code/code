//! `vakcoder backup export|import` (docs/design/29-personal-os.md P3) over
//! `vak_core::backup`. The CLI layer adds the human guardrails the library
//! deliberately leaves out: refusing to export onto (or import from) the
//! live home itself, and a loud terminal warning when secrets ride along.

use std::path::{Path, PathBuf};

use vak_core::Core;
use vak_core::backup::{self, Conflict};

pub fn run_backup(cwd: PathBuf, action: crate::cli::BackupAction) -> i32 {
    let core = match Core::new(cwd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let home = core.sessions_home();
    match action {
        crate::cli::BackupAction::Export {
            dir,
            include_secrets,
        } => export(&home, &dir, include_secrets),
        crate::cli::BackupAction::Import { dir, conflict } => {
            let Some(mode) = parse_conflict(&conflict) else {
                eprintln!("error: unknown --conflict '{conflict}' (skip | rename)");
                return 2;
            };
            import(&home, &dir, mode)
        }
    }
}

fn parse_conflict(word: &str) -> Option<Conflict> {
    match word {
        "skip" => Some(Conflict::Skip),
        "rename" => Some(Conflict::Rename),
        _ => None,
    }
}

fn export(home: &Path, dir: &Path, include_secrets: bool) -> i32 {
    if same_dir(dir, home) {
        eprintln!(
            "error: refusing to export into the live vakcoder home ({}) — pick a directory outside it",
            home.display()
        );
        return 2;
    }
    if include_secrets {
        eprintln!();
        eprintln!("!! WARNING: --include-secrets will copy the user .env");
        eprintln!("!! containing provider API keys into the destination.");
        eprintln!("!! Store that directory encrypted and share it with no one.");
        eprintln!();
    }
    match backup::export_to(home, dir, include_secrets) {
        Ok(manifest) => {
            println!(
                "backed up {} file(s), {} bytes → {}",
                manifest.file_count,
                manifest.total_bytes,
                dir.display()
            );
            println!("manifest: {}", dir.join("manifest.json").display());
            if include_secrets && !home.join(".env").is_file() {
                println!("note: no user .env present; nothing secret was copied");
            }
            0
        }
        Err(e) => {
            eprintln!("error: export failed: {e}");
            1
        }
    }
}

fn import(home: &Path, dir: &Path, conflict: Conflict) -> i32 {
    if same_dir(dir, home) {
        eprintln!(
            "error: refusing to import from the live vakcoder home ({}) — pick a backup directory outside it",
            home.display()
        );
        return 2;
    }
    match backup::import_from(dir, home, conflict) {
        Ok(report) => {
            println!(
                "restored {} file(s) into {} ({} renamed aside, {} skipped as already present)",
                report.copied,
                home.display(),
                report.renamed,
                report.skipped
            );
            0
        }
        Err(e) => {
            eprintln!("error: import failed: {e}");
            1
        }
    }
}

/// Same-directory refusal must survive symlinks and trailing separators,
/// so compare canonical forms when both sides resolve.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn same_dir_detects_alias_forms() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("home");
        std::fs::create_dir_all(&inner).unwrap();
        assert!(same_dir(&inner, &inner));
        assert!(same_dir(
            &inner.join("."),
            &std::fs::canonicalize(&inner).unwrap()
        ));
        assert!(!same_dir(&inner, &dir.path().join("other")));
    }

    #[test]
    fn conflict_words_map_to_modes() {
        assert_eq!(parse_conflict("skip"), Some(Conflict::Skip));
        assert_eq!(parse_conflict("rename"), Some(Conflict::Rename));
        assert_eq!(parse_conflict("merge"), None);
        assert_eq!(parse_conflict(""), None);
    }
}
