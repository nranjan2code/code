//! Workspace contributions use the existing capability roots and plugin activation.
use crate::Core;
use serde::Serialize;
use vak_plugin::{PluginError, PluginStore, workspace::WorkspaceSpec};

#[derive(Clone, Serialize)]
pub struct WorkspaceContribution {
    pub plugin: String,
    pub digest: String,
    pub generation: u64,
    pub scope: String,
    pub spec: WorkspaceSpec,
    #[serde(skip)]
    pub root: std::path::PathBuf,
}
impl WorkspaceContribution {
    pub fn commit<T>(&self, apply: impl FnOnce() -> T) -> Result<T, PluginError> {
        PluginStore::new(&self.root).with_enabled(
            &self.plugin,
            &self.digest,
            self.generation,
            apply,
        )
    }
}
impl Core {
    pub fn canvas_workspaces(&self) -> Result<Vec<WorkspaceContribution>, PluginError> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for root in self.capability_roots() {
            let registry = PluginStore::new(&root.path).load()?;
            for plugin in registry.plugins.values() {
                if !seen.insert(plugin.name.clone())
                    || !plugin.enabled
                    || plugin.capabilities.workspaces.is_empty()
                    || !self.effective_plugins().is_enabled(&plugin.name)
                {
                    continue;
                }
                if self.channel_policy().is_some_and(|policy| {
                    !Core::allowed_by(&policy.plugins_allow, &policy.plugins_deny, &plugin.name)
                }) {
                    continue;
                }
                let package = vak_plugin::inspect_package(&plugin.package_path)?;
                if package.digest != plugin.digest {
                    return Err(PluginError::InvalidManifest(
                        "installed workspace package digest changed".into(),
                    ));
                }
                for spec in package.workspace_specs()? {
                    out.push(WorkspaceContribution {
                        plugin: plugin.name.clone(),
                        digest: plugin.digest.clone(),
                        generation: registry.generation,
                        scope: root.scope.label().into(),
                        spec,
                        root: root.path.clone(),
                    });
                }
            }
        }
        Ok(out)
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkspaceAction {
    Create {
        workspace_type: String,
        digest: String,
        generation: u64,
    },
    Command {
        resource: String,
        revision: u64,
        digest: String,
        generation: u64,
        command: String,
        arguments: serde_json::Value,
    },
    Undo {
        resource: String,
        revision: u64,
        digest: String,
        generation: u64,
    },
    Redo {
        resource: String,
        revision: u64,
        digest: String,
        generation: u64,
    },
}
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceFailure {
    #[error("workspace contribution is unavailable or changed")]
    Activation,
    #[error("workspace command is invalid: {0}")]
    Invalid(String),
    #[error(transparent)]
    State(#[from] vak_session::workspaces::WorkspaceError),
}
impl Core {
    /// Human and Agent edits use one revision/activation/validation path.
    pub fn apply_workspace(
        &self,
        store: &vak_session::workspaces::WorkspaceStore,
        action: WorkspaceAction,
        trace: &vak_session::trace::TraceKey,
    ) -> Result<vak_session::workspaces::Resource, WorkspaceFailure> {
        let definitions = self
            .canvas_workspaces()
            .map_err(|_| WorkspaceFailure::Activation)?;
        let resolve = |kind: &str, digest: &str, generation: u64| {
            definitions
                .iter()
                .find(|item| {
                    item.spec.id == kind && item.digest == digest && item.generation == generation
                })
                .ok_or(WorkspaceFailure::Activation)
        };
        let redo = matches!(&action, WorkspaceAction::Redo { .. });
        let result = match action {
            WorkspaceAction::Create {
                workspace_type,
                digest,
                generation,
            } => {
                let definition = resolve(&workspace_type, &digest, generation)?;
                definition.commit(|| {
                    store.create(
                        &workspace_type,
                        &digest,
                        &definition.spec.title,
                        definition.spec.initial.clone(),
                        trace,
                    )
                })
            }
            WorkspaceAction::Command {
                resource,
                revision,
                digest,
                generation,
                command,
                arguments,
            } => {
                let current = store.get(&resource)?;
                let definition = resolve(&current.workspace_type, &digest, generation)?;
                let next = definition
                    .spec
                    .apply(&current.data, &command, &arguments)
                    .map_err(WorkspaceFailure::Invalid)?;
                definition
                    .commit(|| store.change(&resource, revision, &digest, &command, next, trace))
            }
            WorkspaceAction::Undo {
                resource,
                revision,
                digest,
                generation,
            }
            | WorkspaceAction::Redo {
                resource,
                revision,
                digest,
                generation,
            } => {
                let current = store.get(&resource)?;
                let definition = resolve(&current.workspace_type, &digest, generation)?;
                let restored = if redo {
                    current.redo.last().map(|edit| &edit.after)
                } else {
                    current.undo.last().map(|edit| &edit.before)
                };
                if let Some(data) = restored {
                    vak_tools::validate_input(&definition.spec.state_schema, data)
                        .map_err(WorkspaceFailure::Invalid)?;
                }
                definition.commit(|| store.history(&resource, revision, &digest, redo, trace))
            }
        }
        .map_err(|_| WorkspaceFailure::Activation)??;
        Ok(result)
    }
}
