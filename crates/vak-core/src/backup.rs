//! Backup export/import over the vak home directory
//! (docs/design/29-personal-os.md P3): a plain directory copy of whatever
//! the durable state registry declares as backed up. The encrypted-file
//! credential store's two files (docs/design/44-shared-config.md, "Secrets
//! Chain") are excluded unless explicitly requested — and then a loud
//! WARNING.txt travels beside them, since the key that unlocks them
//! travels in the same backup. A host using the OS keychain instead has
//! nothing here to exclude or include. Import never deletes or silently
//! overwrites existing data; conflicts skip or rename.
//!
//! **What a backup covers comes from `crate::state`, not from a list kept
//! here.** This module used to hardcode five directories and four files,
//! so anything added to the data home afterwards was silently outside
//! every backup taken — `gateway/` with the whole channel allowlist,
//! `operations/` with the incident ledger, `inbox.jsonl`, `learning/`.
//! That is the drift a registry exists to prevent.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MANIFEST_NAME: &str = "manifest.json";
/// Encrypted-file credential backend's two files (docs/design/44-shared-config.md,
/// "Secrets Chain"). Present only on hosts with no reachable OS secret
/// service; on a host using the OS keychain there is nothing here to back
/// up — the keychain is outside this directory entirely.
const CREDENTIALS_FILE: &str = "credentials.enc";
const CREDENTIAL_KEY_FILE: &str = ".credential_key";
const WARNING_FILE: &str = "WARNING.txt";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackupManifest {
    /// Backup layout version, bumped only on breaking shape changes.
    pub version: u32,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub file_count: u64,
    pub total_bytes: u64,
    /// Whether an `include_secrets` export actually found and copied a
    /// credential file. Lets callers report "nothing secret was copied"
    /// accurately instead of guessing from a literal path that may not
    /// exist even when a real credential backend (the OS keychain) is in
    /// use (docs/design/44-shared-config.md, "Secrets Chain").
    #[serde(default)]
    pub secrets_copied: bool,
    /// Whether the tenant keys that decrypt the backed-up object store
    /// (ledger payloads, Agent memory and other Documents) travel with this
    /// backup. They live in the credential store, so they do only when
    /// secrets were included from a host using the encrypted-file backend;
    /// otherwise the store restores only on a machine that holds them (this
    /// one). Interim until erasure and restore epochs (plan M7a, M9).
    #[serde(default)]
    pub content_keys_included: bool,
    /// The tenant store's writer epoch and ref generation when the backup
    /// was taken (plan M7a-h). Absent in a backup made by `export_to`
    /// alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_generation: Option<u64>,
    /// How many scopes had a key and how many had been destroyed.
    #[serde(default)]
    pub scope_keys: u64,
    #[serde(default)]
    pub scopes_destroyed: u64,
    /// The erasure watermark: the id of every erasure receipt the backup
    /// holds. An erasure recorded since is re-applied on restore.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub erasures: Vec<String>,
    /// How many Agent workspace files were left out because their
    /// workspace was over its limits (`state::WORKSPACE_LIMITS`), and the
    /// first [`SKIPPED_LISTED`] of them, under the data home.
    #[serde(default)]
    pub workspace_files_skipped: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_skipped: Vec<String>,
}

/// How many skipped workspace files a manifest names.
pub const SKIPPED_LISTED: usize = 100;

impl Default for BackupManifest {
    fn default() -> Self {
        BackupManifest {
            version: 1,
            timestamp: chrono::Utc::now(),
            file_count: 0,
            total_bytes: 0,
            secrets_copied: false,
            content_keys_included: false,
            store_epoch: None,
            ref_generation: None,
            scope_keys: 0,
            scopes_destroyed: 0,
            erasures: Vec::new(),
            workspace_files_skipped: 0,
            workspace_skipped: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid backup at {path}: {reason}")]
    InvalidBackup { path: PathBuf, reason: String },
    #[error("corrupt manifest in {path}: {source}")]
    Manifest {
        path: PathBuf,
        source: serde_json::Error,
    },
}

fn io_err(path: &Path, source: std::io::Error) -> BackupError {
    BackupError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Copy one file, creating parent directories as needed. Returns its size.
fn copy_file(from: &Path, to: &Path) -> Result<u64, BackupError> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|source| io_err(parent, source))?;
    }
    std::fs::copy(from, to).map_err(|source| io_err(from, source))
}

/// Deterministic recursive listing of every regular file under `root`.
fn list_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else if path.is_file() {
            files.push(path);
        }
    }
    dirs.sort();
    files.sort();
    out.extend(files);
    for dir in dirs {
        out.extend(list_files(&dir));
    }
    out
}

/// Whether `dir` is the data home `home` or inside it: a backup there would
/// copy into its own source, and a restore from there would read what it
/// writes. A `dir` that does not exist yet is judged by its nearest existing
/// ancestor, so a fresh destination cannot slip past the check.
pub fn within_home(dir: &Path, home: &Path) -> bool {
    let home = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
    let mut existing = dir;
    let mut rest = Vec::new();
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            break;
        };
        if let Some(name) = existing.file_name() {
            rest.push(name.to_os_string());
        }
        existing = parent;
    }
    let mut resolved = existing
        .canonicalize()
        .unwrap_or_else(|_| existing.to_path_buf());
    for name in rest.into_iter().rev() {
        resolved.push(name);
    }
    resolved.starts_with(&home)
}

/// Export `home` into `dest_dir`, returning the written manifest. Missing
/// source entries are simply absent from the backup — an empty home yields
/// a valid, empty backup. The manifest itself is not counted in its own
/// totals.
pub fn export_to(
    home: &Path,
    dest_dir: &Path,
    include_secrets: bool,
) -> Result<BackupManifest, BackupError> {
    std::fs::create_dir_all(dest_dir).map_err(|source| io_err(dest_dir, source))?;

    let mut manifest = BackupManifest::default();

    let mut jobs: Vec<(PathBuf, PathBuf)> = Vec::new();
    let carried = crate::state::backup_files(crate::state::Root::Data, home);
    for file in carried.files {
        let rel = file
            .strip_prefix(home)
            .map_err(|_| BackupError::InvalidBackup {
                path: file.clone(),
                reason: "file escaped home root".into(),
            })?
            .to_path_buf();
        jobs.push((file, dest_dir.join(rel)));
    }
    manifest.workspace_files_skipped = carried.skipped.len() as u64;
    manifest.workspace_skipped = carried
        .skipped
        .iter()
        .take(SKIPPED_LISTED)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();

    for (from, to) in &jobs {
        manifest.total_bytes += copy_file(from, to)?;
        manifest.file_count += 1;
    }

    if include_secrets {
        // The encrypted-file credential backend lives in the data home,
        // which is `home` (docs/design/44-shared-config.md, "Secrets Chain").
        let secret_files = [CREDENTIALS_FILE, CREDENTIAL_KEY_FILE];
        let mut copied_any = false;
        for name in secret_files {
            let src = home.join(name);
            if src.is_file() {
                manifest.total_bytes += copy_file(&src, &dest_dir.join(name))?;
                manifest.file_count += 1;
                copied_any = true;
            }
        }
        // Nothing to copy on a host using the OS keychain/Credential
        // Manager/Secret Service backend — its secrets live outside this
        // directory and this backup simply doesn't cover them.
        manifest.secrets_copied = copied_any;
        manifest.content_keys_included = copied_any;
        if copied_any {
            std::fs::write(
                dest_dir.join(WARNING_FILE),
                "WARNING: this backup CONTAINS SECRETS. The credentials file is \
                 encrypted, but its key travels alongside it in this same backup \
                 — together they are as sensitive as plaintext. Store it \
                 encrypted, share it with no one, and delete it as soon as it is \
                 restored.\n",
            )
            .map_err(|source| io_err(&dest_dir.join(WARNING_FILE), source))?;
        }
    }

    std::fs::write(
        dest_dir.join(MANIFEST_NAME),
        serde_json::to_string_pretty(&manifest).map_err(|source| BackupError::Manifest {
            path: dest_dir.join(MANIFEST_NAME),
            source,
        })?,
    )
    .map_err(|source| io_err(&dest_dir.join(MANIFEST_NAME), source))?;

    Ok(manifest)
}

/// Reads a backup's manifest.
pub fn read_manifest(src_dir: &Path) -> Result<BackupManifest, BackupError> {
    let path = src_dir.join(MANIFEST_NAME);
    let text = std::fs::read_to_string(&path).map_err(|_| BackupError::InvalidBackup {
        path: src_dir.to_path_buf(),
        reason: "it has no manifest, so it is not a backup".into(),
    })?;
    serde_json::from_str(&text).map_err(|source| BackupError::Manifest { path, source })
}

fn write_manifest(dest_dir: &Path, manifest: &BackupManifest) -> Result<(), BackupError> {
    let path = dest_dir.join(MANIFEST_NAME);
    let text = serde_json::to_string_pretty(manifest).map_err(|source| BackupError::Manifest {
        path: path.clone(),
        source,
    })?;
    std::fs::write(&path, text).map_err(|source| io_err(&path, source))
}

/// What restoring a backup would do, before anything is copied.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RestorePreview {
    pub manifest: BackupManifest,
    /// Erasures recorded here since the backup was taken, by receipt id.
    /// The backup still holds what they erased; restore erases it again.
    pub erasures_to_reapply: Vec<String>,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RestoreReport {
    pub copied: usize,
    pub renamed: usize,
    pub skipped: usize,
    /// Erasures recorded here that the backup predates.
    pub erasures_reapplied: usize,
    /// Keys the backup brought back for scopes that were destroyed, and
    /// that were destroyed again.
    pub keys_removed: usize,
    pub objects_deleted: usize,
    /// The store's writer epoch after the restore: every process that had
    /// the store open before is fenced and must be started again.
    pub writer_epoch: u64,
}

impl crate::Core {
    fn local_tenant(
        &self,
    ) -> Result<std::sync::Arc<vak_session::objects::TenantObjects>, BackupError> {
        let home = self.shared_scope().into_root();
        vak_session::objects::TenantObjects::for_tenant(&vak_config::paths::tenant_home_at(
            &home,
            vak_config::paths::LOCAL_TENANT,
        ))
        .map_err(|error| BackupError::InvalidBackup {
            path: home,
            reason: error.to_string(),
        })
    }

    /// What every import of another copy's files ends with, a restore and
    /// a pull alike: every key the copy brought back for a destroyed scope
    /// is destroyed again, each erased conversation is marked erased, the
    /// rollups and search are rebuilt from the records, what no key holds
    /// is deleted, and the writer epoch moves past `past`, which fences
    /// this process.
    /// Returns the keys removed, the objects deleted and the new epoch.
    pub(crate) fn settle_imported(&self, past: u64) -> Result<(usize, usize, u64), String> {
        let tenant = self.local_tenant().map_err(|error| error.to_string())?;
        let keys_removed = tenant
            .reapply_destroyed_keys()
            .map_err(|error| error.to_string())?;
        let shared = self.shared_scope();
        let erased: Vec<String> = self
            .erasure_receipts()
            .into_iter()
            .filter(|receipt| receipt.scope == "conversation")
            .map(|receipt| receipt.subject)
            .collect();
        crate::trash::mark_erased(&shared, &erased).map_err(|error| error.to_string())?;
        let _ = vak_session::documents::forget(&shared.artifacts_rollup());
        self.catalog()
            .map_err(|error| error.to_string())?
            .rebuild_after_erasure()
            .map_err(|error| error.to_string())?;
        let objects_deleted = {
            use vak_session::objects::Objects;
            tenant.collect().unwrap_or(0)
        };
        // The epoch ends past this store's own and past `past`, the
        // epoch of the copy that was imported, so a process on either
        // side that still holds an older one is fenced.
        let bump = || tenant.store().restore().map_err(|error| error.to_string());
        let mut writer_epoch = bump()?;
        while writer_epoch <= past {
            writer_epoch = bump()?;
        }
        Ok((keys_removed, objects_deleted, writer_epoch))
    }

    /// Backs the data home up into `dest_dir`: the records and objects as
    /// they are stored, which is encrypted, the wrapped keys, and a
    /// manifest that says where the store stood (its writer epoch and ref
    /// generation, its keys, and which erasures it already holds).
    pub fn backup_create(
        &self,
        dest_dir: &Path,
        include_secrets: bool,
    ) -> Result<BackupManifest, BackupError> {
        let home = self.shared_scope().into_root();
        let tenant = self.local_tenant()?;
        // Read before the copy: a backup may hold more than its manifest
        // says, never less, so nothing it lacks is counted as kept.
        let store = tenant.store();
        let epoch = store.epoch().ok();
        let generation = store.commit_generation().ok();
        let (keys, destroyed) = tenant.scope_counts();
        let erasures = self
            .erasure_receipts()
            .into_iter()
            .map(|receipt| receipt.id)
            .collect();
        let mut manifest = export_to(&home, dest_dir, include_secrets)?;
        manifest.version = 2;
        manifest.store_epoch = epoch;
        manifest.ref_generation = generation;
        manifest.scope_keys = keys as u64;
        manifest.scopes_destroyed = destroyed as u64;
        manifest.erasures = erasures;
        write_manifest(dest_dir, &manifest)?;
        Ok(manifest)
    }

    /// What restoring `src_dir` here would do.
    pub fn restore_preview(&self, src_dir: &Path) -> Result<RestorePreview, BackupError> {
        let manifest = read_manifest(src_dir)?;
        let erasures_to_reapply = self
            .erasure_receipts()
            .into_iter()
            .map(|receipt| receipt.id)
            .filter(|id| !manifest.erasures.contains(id))
            .collect();
        Ok(RestorePreview {
            manifest,
            erasures_to_reapply,
        })
    }

    /// Restores `src_dir` into this data home. Nothing here is overwritten
    /// or deleted. Before the restore ends, every erasure this home has on
    /// record is applied again: a key the backup brought back for a
    /// destroyed scope is destroyed, each erased conversation is marked
    /// erased, the rollups and search are rebuilt from the records, and
    /// what no key holds is deleted. Last, the store's writer epoch moves,
    /// which fences every process that had it open, this one included.
    pub fn backup_restore(
        &self,
        src_dir: &Path,
        conflict: Conflict,
    ) -> Result<RestoreReport, BackupError> {
        let failed = |reason: String| BackupError::InvalidBackup {
            path: src_dir.to_path_buf(),
            reason,
        };
        vak_session::fence::check().map_err(|error| failed(error.to_string()))?;
        let preview = self.restore_preview(src_dir)?;
        let home = self.shared_scope().into_root();
        let imported = import_from(src_dir, &home, conflict)?;
        let (keys_removed, objects_deleted, writer_epoch) =
            self.settle_imported(0).map_err(failed)?;
        tracing::info!(
            kind = "backup",
            outcome = "restored",
            count = keys_removed,
            "a backup was restored and its erasures applied again"
        );
        Ok(RestoreReport {
            copied: imported.copied,
            renamed: imported.renamed,
            skipped: imported.skipped,
            erasures_reapplied: preview.erasures_to_reapply.len(),
            keys_removed,
            objects_deleted,
            writer_epoch,
        })
    }
}

/// What to do when a restored file already exists at the destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Keep the existing file; the incoming copy is dropped.
    Skip,
    /// Keep both: write the incoming copy under a `.importN` suffix.
    Rename,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub copied: usize,
    pub renamed: usize,
    pub skipped: usize,
}

/// Restore a backup directory into `home`. Existing files are NEVER
/// overwritten or deleted: per [`Conflict`], clashes are skipped or
/// renamed aside. Manifest and warning files are metadata, not data, and
/// are not restored.
pub fn import_from(
    src_dir: &Path,
    home: &Path,
    conflict: Conflict,
) -> Result<ImportReport, BackupError> {
    if !src_dir.is_dir() {
        return Err(BackupError::InvalidBackup {
            path: src_dir.to_path_buf(),
            reason: "not a directory".into(),
        });
    }
    std::fs::create_dir_all(home).map_err(|source| io_err(home, source))?;
    let mut report = ImportReport::default();

    for file in list_files(src_dir) {
        let rel = file
            .strip_prefix(src_dir)
            .map_err(|_| BackupError::InvalidBackup {
                path: file.clone(),
                reason: "file escaped backup root".into(),
            })?;
        // Metadata files never restore.
        if rel == Path::new(MANIFEST_NAME) || rel == Path::new(WARNING_FILE) {
            continue;
        }
        let target = home.join(rel);
        if !target.exists() {
            copy_file(&file, &target)?;
            report.copied += 1;
            continue;
        }
        match conflict {
            Conflict::Skip => report.skipped += 1,
            Conflict::Rename => {
                let stem = target
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("file")
                    .to_string();
                let ext = target
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .unwrap_or_default();
                let parent = target.parent().unwrap_or(home);
                let mut n = 1u32;
                loop {
                    let candidate = parent.join(format!("{stem}.import{n}{ext}"));
                    if !candidate.exists() {
                        copy_file(&file, &candidate)?;
                        break;
                    }
                    n += 1;
                }
                report.renamed += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use tempfile::tempdir;

    fn seed_home(home: &Path) {
        for rel in [
            "agents/vak/sessions/spc_test/s1/seg-00000001.log",
            "tenants/t1/store/refs.db",
            "checkpoints/s1/000.json",
            "tenants/t1/keys/revoked",
            "trusted/allow.toml",
        ] {
            let p = home.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, rel).unwrap();
        }
        for rel in ["cost-log", "routing-evidence", "desktop.json"] {
            std::fs::write(home.join(rel), rel).unwrap();
        }
    }

    /// An Agent's workspace in the data home, with the attachments its
    /// conversations name, is in the backup; a run's environment is not.
    #[test]
    fn export_carries_agent_workspaces_and_not_run_environments() {
        let home = tempdir().unwrap();
        let inbox = home
            .path()
            .join("tenants/t1/workspaces/spc_a/agt_b/inbox/photo.txt");
        std::fs::create_dir_all(inbox.parent().unwrap()).unwrap();
        std::fs::write(&inbox, b"bytes").unwrap();
        let scratch = home.path().join("tenants/t1/environments/run_1/work.txt");
        std::fs::create_dir_all(scratch.parent().unwrap()).unwrap();
        std::fs::write(&scratch, b"scratch").unwrap();
        let dest = tempdir().unwrap();
        let manifest = export_to(home.path(), dest.path(), false).unwrap();
        assert!(
            dest.path()
                .join("tenants/t1/workspaces/spc_a/agt_b/inbox/photo.txt")
                .is_file()
        );
        assert!(!dest.path().join("tenants/t1/environments").exists());
        assert_eq!(manifest.workspace_files_skipped, 0);
        assert!(manifest.workspace_skipped.is_empty());
    }

    #[test]
    fn export_manifest_counts_and_roundtrip_is_byte_identical() {
        let home = tempdir().unwrap();
        seed_home(home.path());

        let dest = tempdir().unwrap();
        let manifest = export_to(home.path(), dest.path(), false).unwrap();

        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.file_count, 8);
        let expected_bytes: u64 = [
            "agents/vak/sessions/spc_test/s1/seg-00000001.log",
            "tenants/t1/store/refs.db",
            "checkpoints/s1/000.json",
            "tenants/t1/keys/revoked",
            "trusted/allow.toml",
            "cost-log",
            "routing-evidence",
            "desktop.json",
        ]
        .iter()
        .map(|r| r.len() as u64)
        .sum();
        assert_eq!(manifest.total_bytes, expected_bytes);

        // Round-trip into a fresh home restores every ledger byte-for-byte.
        let restored = tempdir().unwrap();
        let report = import_from(dest.path(), restored.path(), Conflict::Skip).unwrap();
        assert_eq!(report.copied, 8);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.renamed, 0);
        for rel in [
            "agents/vak/sessions/spc_test/s1/seg-00000001.log",
            "cost-log",
            "desktop.json",
            "tenants/t1/store/refs.db",
        ] {
            assert_eq!(
                std::fs::read(restored.path().join(rel)).unwrap(),
                std::fs::read(home.path().join(rel)).unwrap(),
                "{rel} must survive export/import unchanged"
            );
        }
    }

    #[test]
    fn import_skip_never_touches_existing_data() {
        let dest = tempdir().unwrap();
        std::fs::write(dest.path().join("cost-log"), "{\"seed\":true}\n").unwrap();
        std::fs::create_dir_all(dest.path().join("agents/vak/sessions/x")).unwrap();
        std::fs::write(dest.path().join("agents/vak/sessions/x/keep.log"), "keep").unwrap();

        let home = tempdir().unwrap();
        std::fs::write(home.path().join("cost-log"), "{\"new\":1}\n").unwrap();
        let s = home.path().join("agents/vak/sessions/x");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(s.join("keep.log"), "REPLACEMENT-attempt").unwrap();

        let report = import_from(dest.path(), home.path(), Conflict::Skip).unwrap();
        assert_eq!(report.skipped, 2);
        assert_eq!(report.copied, 0);
        assert_eq!(
            std::fs::read_to_string(home.path().join("cost-log")).unwrap(),
            "{\"new\":1}\n",
            "existing file must win under Skip"
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("agents/vak/sessions/x/keep.log")).unwrap(),
            "REPLACEMENT-attempt",
            "Skip leaves the pre-existing home copy standing"
        );
    }

    #[test]
    fn import_rename_preserves_both_copies() {
        let home = tempdir().unwrap();
        std::fs::write(home.path().join("desktop.json"), "existing").unwrap();

        let dest = tempdir().unwrap();
        std::fs::write(dest.path().join("desktop.json"), "incoming").unwrap();
        std::fs::create_dir_all(dest.path().join("tenants/t1/keys")).unwrap();
        std::fs::write(
            dest.path().join("tenants/t1/keys/revoked"),
            "conversation:x\n",
        )
        .unwrap();

        let report = import_from(dest.path(), home.path(), Conflict::Rename).unwrap();
        assert_eq!(report.renamed, 1);
        assert_eq!(report.copied, 1);
        assert_eq!(
            std::fs::read_to_string(home.path().join("desktop.json")).unwrap(),
            "existing"
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join("desktop.import1.json")).unwrap(),
            "incoming"
        );

        // A second import renames every clashing file to the next free
        // suffix (desktop.json and the already-restored memory note).
        let report2 = import_from(dest.path(), home.path(), Conflict::Rename).unwrap();
        assert_eq!(report2.renamed, 2);
        assert!(home.path().join("desktop.import2.json").is_file());
    }

    #[test]
    fn empty_home_yields_valid_empty_backup() {
        let home = tempdir().unwrap();
        let dest = tempdir().unwrap();
        let manifest = export_to(home.path(), dest.path(), false).unwrap();
        assert_eq!(manifest.file_count, 0);
        assert_eq!(manifest.total_bytes, 0);
        let raw = std::fs::read_to_string(dest.path().join(MANIFEST_NAME)).unwrap();
        let parsed: BackupManifest = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn backup_covers_every_agent_home_and_skips_undeclared_subpaths() {
        // The registry declares Agent homes per subpath; a backup must
        // expand that pattern rather than look for a literal `{agent}`.
        let home = tempdir().unwrap();
        for rel in [
            "agents/vak/sessions/h/a.jsonl",
            "agents/writer/checkpoints/s/0000.json",
            "agents/writer/stray/undeclared.txt",
        ] {
            let p = home.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, rel).unwrap();
        }
        let dest = tempdir().unwrap();
        let manifest = export_to(home.path(), dest.path(), false).unwrap();
        assert_eq!(manifest.file_count, 2);
        assert!(dest.path().join("agents/vak/sessions/h/a.jsonl").is_file());
        assert!(
            dest.path()
                .join("agents/writer/checkpoints/s/0000.json")
                .is_file()
        );
        assert!(!dest.path().join("agents/writer/stray").exists());
    }

    #[test]
    fn missing_source_directory_is_a_typed_error() {
        let nowhere = tempdir().unwrap().path().join("does-not-exist");
        let home = tempdir().unwrap();
        assert!(matches!(
            import_from(&nowhere, home.path(), Conflict::Skip),
            Err(BackupError::InvalidBackup { .. })
        ));
    }
}
