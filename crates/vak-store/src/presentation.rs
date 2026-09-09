//! Durable user/workspace presentation definitions.
//!
//! The library is a replaceable projection, not session truth. Session
//! selection receipts remain append-only in `vak-session`; this file stores
//! only the current reusable definitions and activation pointers.

use std::fs;
use std::path::{Path, PathBuf};

use vak_presentation::{PresentationLibrary, StoredPresentation};

const PRESENTATION_STORE_SCHEMA: u16 = 1;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PresentationDisk {
    schema_version: u16,
    definitions: Vec<StoredPresentation>,
    activations: Vec<vak_presentation::PresentationActivation>,
}

#[derive(Debug, thiserror::Error)]
pub enum PresentationStoreError {
    #[error("presentation store io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("presentation store json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("presentation store validation error: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct PresentationStore {
    path: PathBuf,
}

impl PresentationStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<PresentationLibrary, PresentationStoreError> {
        if !self.path.exists() {
            return Ok(PresentationLibrary::default());
        }
        let bytes = fs::read(&self.path)?;
        let disk: PresentationDisk = serde_json::from_slice(&bytes)?;
        if disk.schema_version != PRESENTATION_STORE_SCHEMA {
            return Err(PresentationStoreError::Invalid(format!(
                "unsupported presentation store schema {}",
                disk.schema_version
            )));
        }
        vak_presentation::PresentationLibrary::from_parts(disk.definitions, disk.activations)
            .map_err(|error| PresentationStoreError::Invalid(error.to_string()))
    }

    pub fn save(&self, library: &PresentationLibrary) -> Result<(), PresentationStoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let disk = PresentationDisk {
            schema_version: PRESENTATION_STORE_SCHEMA,
            definitions: library.definitions().cloned().collect(),
            activations: library.activations().to_vec(),
        };
        let bytes = serde_json::to_vec_pretty(&disk)?;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let temp = self
            .path
            .with_extension(format!("tmp-{}-{nonce}", std::process::id()));
        fs::write(&temp, bytes)?;
        fs::rename(temp, &self.path)?;
        Ok(())
    }

    pub fn register_pack(
        &self,
        records: impl IntoIterator<Item = StoredPresentation>,
    ) -> Result<usize, PresentationStoreError> {
        let mut library = self.load()?;
        let mut count = 0;
        for mut record in records {
            // Registration is preview-only; activation is a separate,
            // explicit operation owned by the control plane.
            record.enabled = false;
            library
                .register(record)
                .map_err(|error| PresentationStoreError::Invalid(error.to_string()))?;
            count += 1;
        }
        self.save(&library)?;
        Ok(count)
    }

    pub fn revoke_plugin(&self, plugin_id: &str) -> Result<usize, PresentationStoreError> {
        let mut library = self.load()?;
        let removed = library.revoke_plugin(plugin_id);
        self.save(&library)?;
        Ok(removed)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use vak_presentation::{LibraryScope, PresentationOrigin, PresentationSpec, digest};

    #[test]
    fn missing_store_is_empty_and_roundtrips_atomically() {
        let dir = tempdir().expect("tempdir");
        let store = PresentationStore::new(dir.path().join("nested/presentations.json"));
        let library = store.load().expect("load");
        store.save(&library).expect("save");
        assert_eq!(store.load().expect("reload"), library);
    }

    #[test]
    fn pack_registration_and_plugin_revocation_are_atomic() {
        let dir = tempdir().expect("tempdir");
        let store = PresentationStore::new(dir.path().join("presentations.json"));
        let spec: PresentationSpec = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "id": "demo.card",
            "revision": 1,
            "accepts": ["demo"],
            "root": { "primitive": "title", "props": {}, "children": [] }
        }))
        .expect("spec");
        let record = StoredPresentation {
            digest: digest(&spec).expect("digest"),
            spec,
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "workspace".into(),
                plugin_id: Some("demo-pack".into()),
                generation: Some("1".into()),
            },
            enabled: true,
        };
        assert_eq!(store.register_pack([record]).expect("register"), 1);
        assert!(
            !store
                .load()
                .expect("load after register")
                .definitions()
                .next()
                .expect("definition")
                .enabled
        );
        assert_eq!(store.revoke_plugin("demo-pack").expect("revoke"), 1);
        assert!(store.load().expect("load").definitions().next().is_none());
    }

    #[test]
    fn activation_is_durable_and_revoke_clears_it_immediately() {
        let dir = tempdir().expect("tempdir");
        let store = PresentationStore::new(dir.path().join("presentations.json"));
        let spec: PresentationSpec = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "id": "next.turn.card",
            "revision": 1,
            "accepts": ["demo"],
            "root": { "primitive": "title", "props": {}, "children": [] }
        }))
        .expect("spec");
        let record = StoredPresentation {
            digest: digest(&spec).expect("digest"),
            spec: spec.clone(),
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "workspace".into(),
                plugin_id: Some("next-turn-pack".into()),
                generation: Some("1".into()),
            },
            enabled: false,
        };
        store.register_pack([record]).expect("register preview");

        // Registration is preview-only; activation is explicit and persists
        // for the next turn without requiring a process restart.
        let mut library = store.load().expect("load preview");
        assert!(
            library
                .select("demo", LibraryScope::Workspace, "workspace")
                .is_none()
        );
        library
            .activate("next.turn.card", 1, LibraryScope::Workspace, "workspace")
            .expect("activate");
        store.save(&library).expect("save activation");
        let reloaded = store.load().expect("reload activation");
        assert_eq!(
            reloaded
                .select("demo", LibraryScope::Workspace, "workspace")
                .map(|entry| entry.spec.id.as_str()),
            Some("next.turn.card")
        );

        // Revoke is level-triggered: the persisted projection is removed
        // immediately while historical session receipts remain elsewhere.
        assert_eq!(store.revoke_plugin("next-turn-pack").expect("revoke"), 1);
        let revoked = store.load().expect("reload revoked");
        assert!(revoked.activations().is_empty());
        assert!(revoked.get("next.turn.card", 1).is_none());
    }

    #[test]
    fn live_pack_lifecycle_exports_imports_and_preserves_history_projection() {
        let source_dir = tempdir().expect("tempdir");
        let source = PresentationStore::new(source_dir.path().join("presentations.json"));
        let spec = PresentationSpec {
            schema_version: 1,
            id: "live.pack.card".into(),
            revision: 1,
            accepts: vec!["live-pack-value".into()],
            root: vak_presentation::SpecNode {
                primitive: vak_presentation::Primitive::Title,
                props: std::collections::BTreeMap::new(),
                children: Vec::new(),
                each: None,
                item: None,
            },
            fallback: Default::default(),
            accessibility: Default::default(),
            metadata: Default::default(),
        };
        let record = StoredPresentation {
            digest: digest(&spec).expect("digest"),
            spec,
            origin: PresentationOrigin {
                scope: LibraryScope::Workspace,
                owner: "workspace-live".into(),
                plugin_id: Some("live-pack".into()),
                generation: Some("1".into()),
            },
            enabled: true,
        };

        // Author/validate/import is preview-only; the enabled bit from an
        // external pack cannot grant activation.
        assert_eq!(source.register_pack([record.clone()]).expect("import"), 1);
        let preview = source.load().expect("preview");
        assert!(!preview.get("live.pack.card", 1).expect("record").enabled);
        assert!(
            preview
                .select("live-pack-value", LibraryScope::Workspace, "workspace-live")
                .is_none()
        );

        // Export is a durable projection round-trip, not a second selection
        // contract. A fresh store can import it without a process restart.
        let exported = serde_json::to_vec(&(
            preview.definitions().cloned().collect::<Vec<_>>(),
            preview.activations().to_vec(),
        ))
        .expect("export");
        let (definitions, _activations): (
            Vec<StoredPresentation>,
            Vec<vak_presentation::PresentationActivation>,
        ) = serde_json::from_slice(&exported).expect("import export");
        let restored_dir = tempdir().expect("tempdir");
        let restored = PresentationStore::new(restored_dir.path().join("presentations.json"));
        assert_eq!(
            restored.register_pack(definitions).expect("restore pack"),
            1
        );

        // Explicit enable is visible on the next selection immediately.
        let mut library = restored.load().expect("load restored");
        library
            .activate(
                "live.pack.card",
                1,
                LibraryScope::Workspace,
                "workspace-live",
            )
            .expect("enable");
        restored.save(&library).expect("persist enable");
        assert_eq!(
            restored
                .load()
                .expect("next turn")
                .select("live-pack-value", LibraryScope::Workspace, "workspace-live")
                .map(|selected| selected.spec.id.as_str()),
            Some("live.pack.card")
        );

        // Revoke removes only the live projection; an already-recorded
        // selection receipt would remain in the append-only session ledger.
        assert_eq!(restored.revoke_plugin("live-pack").expect("revoke"), 1);
        assert!(
            restored
                .load()
                .expect("post revoke")
                .get("live.pack.card", 1)
                .is_none()
        );
    }

    #[test]
    fn unsupported_store_schema_fails_closed() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("presentations.json");
        std::fs::write(
            &path,
            serde_json::json!({ "schema_version": 99, "definitions": [], "activations": [] })
                .to_string(),
        )
        .expect("write");
        let error = PresentationStore::new(path)
            .load()
            .expect_err("schema must fail");
        assert!(
            error
                .to_string()
                .contains("unsupported presentation store schema")
        );
    }
}
