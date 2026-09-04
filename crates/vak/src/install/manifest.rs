//! The record of what is installed, and the integrity data that lets
//! `status` and `update` tell the truth about it.
//!
//! A v1 manifest recorded only `(name, path)` pairs and a single version
//! string. That could not answer the two questions that matter after an
//! update: is every component actually at the recorded version, and is
//! the file on disk the file we put there? v2 records a digest per
//! component, so drift is detected rather than assumed absent.

use std::path::PathBuf;

use super::layout::InstallRoot;
use crate::install::digest;

/// Current manifest schema. Bump only for a breaking shape change; new
/// optional fields do not require it.
pub const SCHEMA: u32 = 2;

/// One installed executable or asset.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub path: PathBuf,
    /// Lowercase hex SHA-256 of the file as installed.
    pub sha256: String,
    /// A missing required component means the install is broken, not
    /// merely partial — `status` and `verify` treat the two differently.
    #[serde(default)]
    pub required: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct Manifest {
    #[serde(default)]
    pub schema: u32,
    pub version: String,
    pub git_sha: String,
    pub installed_at: String,
    /// The prefix this manifest describes. Self-describing so tooling
    /// that finds a manifest knows the root without re-deriving it.
    #[serde(default)]
    pub prefix: PathBuf,
    #[serde(default)]
    pub components: Vec<Component>,
}

/// What `verify` found wrong with an install.
#[derive(Debug, PartialEq, Eq)]
pub enum Defect {
    Missing { name: String, path: PathBuf },
    Corrupt { name: String, path: PathBuf },
    Unreadable { name: String, detail: String },
}

impl std::fmt::Display for Defect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Defect::Missing { name, path } => {
                write!(f, "{name}: missing at {}", path.display())
            }
            Defect::Corrupt { name, path } => write!(
                f,
                "{name}: contents differ from the manifest digest ({})",
                path.display()
            ),
            Defect::Unreadable { name, detail } => write!(f, "{name}: {detail}"),
        }
    }
}

impl Manifest {
    /// Build a manifest describing a freshly placed install.
    pub fn new(version: String, prefix: PathBuf, components: Vec<Component>) -> Self {
        Self {
            schema: SCHEMA,
            version,
            git_sha: build_git_sha(),
            installed_at: now_rfc3339(),
            prefix,
            components,
        }
    }

    pub fn read(root: &InstallRoot) -> Result<Self, String> {
        let path = root.manifest_path();
        let raw = std::fs::read(&path).map_err(|_| {
            format!(
                "no managed install at {} — run `vak self install`",
                root.prefix().display()
            )
        })?;
        let mut m: Manifest = serde_json::from_slice(&raw)
            .map_err(|e| format!("manifest at {} is unreadable: {e}", path.display()))?;

        // AGENTS.md invariant 29: an install written before the baseline is
        // refused whole. There is no v1 fold-forward any more -- a manifest
        // that old describes a tree this build cannot reason about, and
        // half-adopting it produced an install that `verify` and `update`
        // disagreed about.
        if vak_core::baseline::is_pre_baseline(&m.version) {
            return Err(vak_core::baseline::refusal(
                "The install manifest",
                &m.version,
            ));
        }
        if m.schema > SCHEMA {
            return Err(format!(
                "manifest at {} is schema {} but this build supports schema {SCHEMA} \
                 -- it was written by a newer vak; upgrade rather than downgrade",
                path.display(),
                m.schema
            ));
        }
        if m.prefix.as_os_str().is_empty() {
            m.prefix = root.prefix().to_path_buf();
        }
        if m.schema == 0 {
            m.schema = SCHEMA;
        }
        Ok(m)
    }

    pub fn write(&self, root: &InstallRoot) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        super::atomic::write(&root.manifest_path(), &json)
    }

    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }

    /// Path of the installed CLI, which every service unit execs.
    pub fn cli_path(&self) -> Result<PathBuf, String> {
        self.component("vak")
            .map(|c| c.path.clone())
            .ok_or_else(|| "manifest lacks a vak entry — reinstall to repair".into())
    }

    /// Check every recorded component against the filesystem. An empty
    /// result means the install is exactly what the manifest claims.
    pub fn verify(&self) -> Vec<Defect> {
        let manifest_path = vak_core::install::manifest_path_for_prefix(&self.prefix);
        let mut defects = Vec::new();
        for c in &self.components {
            if !c.path.exists() {
                if c.required {
                    defects.push(Defect::Missing {
                        name: c.name.clone(),
                        path: c.path.clone(),
                    });
                }
                continue;
            }
            // An entry with no digest on record predates digesting for
            // that component; absence of a digest is not evidence of
            // corruption.
            if c.sha256.is_empty() {
                continue;
            }
            // A component may be a tree — the desktop frontend is one —
            // and `of_file` on a directory fails with an IO error that
            // would read as corruption rather than as the wrong check.
            let computed = if c.path.is_dir() {
                // Same exclusion the install used: in a bundle this
                // manifest lives inside the tree it describes.
                digest::of_tree_excluding(&c.path, &[manifest_path.as_path()])
            } else {
                digest::of_file(&c.path)
            };
            match computed {
                Ok(actual) if actual == c.sha256 => {}
                Ok(_) => defects.push(Defect::Corrupt {
                    name: c.name.clone(),
                    path: c.path.clone(),
                }),
                Err(detail) => defects.push(Defect::Unreadable {
                    name: c.name.clone(),
                    detail,
                }),
            }
        }
        defects
    }
}

/// Compile-time version of the running binary.
pub fn build_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Commit the running binary was built from, when the build recorded one.
pub fn build_git_sha() -> String {
    option_env!("VAK_GIT_SHA")
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".into())
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> (tempfile::TempDir, InstallRoot) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("vak-manifest-{tag}-"))
            .tempdir()
            .unwrap();
        let root = InstallRoot::at(dir.path().to_path_buf());
        (dir, root)
    }

    #[test]
    fn a_pre_baseline_manifest_is_refused_with_the_shared_message() {
        // AGENTS.md invariant 29. The v1 shape used to be folded forward
        // here; an install that old is now refused whole, and the operator
        // is told the one command that resolves it rather than being left
        // with an install `verify` and `update` disagree about.
        let (_d, root) = temp_root("pre-baseline");
        let bin = root.bin_dir().join("vak");
        std::fs::create_dir_all(root.bin_dir()).unwrap();
        std::fs::write(&bin, b"binary").unwrap();
        let old = serde_json::json!({
            "schema": SCHEMA,
            "version": "1.0.3",
            "git_sha": "abc",
            "installed_at": "2026-01-01T00:00:00Z",
            "components": [{"name": "vak", "path": bin, "sha256": "", "required": true}],
        });
        std::fs::create_dir_all(root.manifest_path().parent().unwrap()).unwrap();
        std::fs::write(root.manifest_path(), old.to_string()).unwrap();

        let err = Manifest::read(&root).unwrap_err();
        assert!(err.contains("1.0.3"), "the refusal names what it found");
        assert!(
            err.contains(vak_core::baseline::BASELINE),
            "and the baseline it expected"
        );
        assert!(
            err.contains("vak self uninstall --purge"),
            "and the one command that resolves it"
        );
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_half_read() {
        let (_d, root) = temp_root("newer-schema");
        let newer = serde_json::json!({
            "schema": SCHEMA + 1,
            "version": "9.0.0",
            "git_sha": "abc",
            "installed_at": "2026-01-01T00:00:00Z",
            "components": [],
        });
        std::fs::create_dir_all(root.manifest_path().parent().unwrap()).unwrap();
        std::fs::write(root.manifest_path(), newer.to_string()).unwrap();

        let err = Manifest::read(&root).unwrap_err();
        assert!(err.contains("written by a newer vak"));
    }

    #[test]
    fn verify_reports_a_component_whose_bytes_changed() {
        let (_d, root) = temp_root("corrupt");
        std::fs::create_dir_all(root.bin_dir()).unwrap();
        let bin = root.bin_dir().join("vak");
        std::fs::write(&bin, b"original").unwrap();
        let m = Manifest {
            schema: SCHEMA,
            version: "2.0.0".into(),
            git_sha: "abc".into(),
            installed_at: now_rfc3339(),
            prefix: root.prefix().to_path_buf(),
            components: vec![Component {
                name: "vak".into(),
                path: bin.clone(),
                sha256: digest::of_file(&bin).unwrap(),
                required: true,
            }],
        };
        assert!(m.verify().is_empty());

        std::fs::write(&bin, b"tampered").unwrap();
        assert_eq!(
            m.verify(),
            vec![Defect::Corrupt {
                name: "vak".into(),
                path: bin
            }]
        );
    }

    #[test]
    fn verify_reports_a_missing_required_component_but_tolerates_optional() {
        let (_d, root) = temp_root("missing");
        let required = root.bin_dir().join("vak");
        let optional = root.bin_dir().join("vak-delivery-worker");
        let m = Manifest {
            schema: SCHEMA,
            version: "2.0.0".into(),
            git_sha: "abc".into(),
            installed_at: now_rfc3339(),
            prefix: root.prefix().to_path_buf(),
            components: vec![
                Component {
                    name: "vak".into(),
                    path: required.clone(),
                    sha256: "deadbeef".into(),
                    required: true,
                },
                Component {
                    name: "vak-delivery-worker".into(),
                    path: optional,
                    sha256: "deadbeef".into(),
                    required: false,
                },
            ],
        };
        assert_eq!(
            m.verify(),
            vec![Defect::Missing {
                name: "vak".into(),
                path: required
            }],
            "an absent optional component is not a defect"
        );
    }
}
