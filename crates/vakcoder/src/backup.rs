//! `vakcoder backup export|import` over the Runtime backup contract.

use std::path::{Path, PathBuf};

use vak_client::{BackupExportRequest, BackupImportRequest, Client};

#[cfg(test)]
fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

pub async fn run_backup(_cwd: PathBuf, action: crate::cli::BackupAction) -> i32 {
    let client = match crate::connect::discover(None, None, None)
        .and_then(|resolved| Client::new(resolved.url, resolved.token).map_err(|e| e.to_string()))
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("error connecting to Runtime: {error}");
            return 2;
        }
    };
    match action {
        crate::cli::BackupAction::Export {
            dir,
            include_secrets,
        } => export(&client, &dir, include_secrets).await,
        crate::cli::BackupAction::Import { dir, conflict } => {
            let Some(mode) = parse_conflict(&conflict) else {
                eprintln!("error: unknown --conflict '{conflict}' (skip | rename)");
                return 2;
            };
            import(&client, &dir, &mode).await
        }
    }
}

fn parse_conflict(word: &str) -> Option<String> {
    match word {
        "skip" => Some("skip".into()),
        "rename" => Some("rename".into()),
        _ => None,
    }
}

async fn export(client: &Client, dir: &Path, include_secrets: bool) -> i32 {
    if include_secrets {
        eprintln!();
        eprintln!("!! WARNING: --include-secrets will copy the user .env");
        eprintln!("!! containing provider API keys into the destination.");
        eprintln!("!! Store that directory encrypted and share it with no one.");
        eprintln!();
    }
    match client
        .backup_export(&BackupExportRequest {
            directory: dir.display().to_string(),
            include_secrets,
        })
        .await
    {
        Ok(manifest) => {
            println!(
                "backed up {} file(s), {} bytes → {}",
                manifest.file_count,
                manifest.total_bytes,
                dir.display()
            );
            println!("manifest: {}", dir.join("manifest.json").display());
            0
        }
        Err(e) => {
            eprintln!("error: export failed: {e}");
            1
        }
    }
}

async fn import(client: &Client, dir: &Path, conflict: &str) -> i32 {
    match client
        .backup_import(&BackupImportRequest {
            directory: dir.display().to_string(),
            conflict: conflict.to_owned(),
        })
        .await
    {
        Ok(report) => {
            println!(
                "restored {} file(s) into Runtime data home ({} renamed aside, {} skipped as already present)",
                report.file_count, report.renamed, report.skipped
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
        assert_eq!(parse_conflict("skip"), Some("skip".into()));
        assert_eq!(parse_conflict("rename"), Some("rename".into()));
        assert_eq!(parse_conflict("merge"), None);
        assert_eq!(parse_conflict(""), None);
    }
}
