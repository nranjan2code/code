//! `vak backup export|import` (docs/design/29-personal-os.md P3) over
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
    // The registry is relative to the data home, which a backup covers whole.
    let home = core.shared_scope().into_root();
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
    if backup::within_home(dir, home) {
        eprintln!(
            "error: refusing to export into the live vak home ({}) — pick a directory outside it",
            home.display()
        );
        return 2;
    }
    if include_secrets {
        eprintln!();
        eprintln!("!! WARNING: --include-secrets will copy the local encrypted");
        eprintln!("!! credential store (if this host uses one) into the destination,");
        eprintln!("!! including the key that decrypts it. A host using the OS");
        eprintln!("!! keychain has nothing to copy here — that store is never backed");
        eprintln!("!! up by this command.");
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
            if !manifest.content_keys_included {
                println!(
                    "note: conversation history and Agent memory are encrypted under this machine's keys, which this backup does not carry; it restores them only on this machine"
                );
            }
            if include_secrets && !manifest.secrets_copied {
                println!(
                    "note: nothing secret was copied (either none stored, or this host uses the OS keychain, which this command cannot back up)"
                );
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
    if backup::within_home(dir, home) {
        eprintln!(
            "error: refusing to import from the live vak home ({}) — pick a backup directory outside it",
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn a_backup_never_lands_in_the_home_it_copies() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        assert!(backup::within_home(&home, &home));
        assert!(backup::within_home(
            &home.join("."),
            &std::fs::canonicalize(&home).unwrap()
        ));
        assert!(
            backup::within_home(&home.join("agents/vak/not-yet"), &home),
            "a fresh folder inside the home is refused too"
        );
        assert!(!backup::within_home(&dir.path().join("other"), &home));
    }

    #[test]
    fn conflict_words_map_to_modes() {
        assert_eq!(parse_conflict("skip"), Some(Conflict::Skip));
        assert_eq!(parse_conflict("rename"), Some(Conflict::Rename));
        assert_eq!(parse_conflict("merge"), None);
        assert_eq!(parse_conflict(""), None);
    }
}
