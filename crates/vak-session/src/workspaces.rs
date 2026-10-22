//! Revisioned workspace resources, sealed with their conversation in existing Documents.
use crate::trace::TraceKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Frozen, person-selected workspace context attached to a logged directive.
/// The projection is a conversation-scoped object, never a large inline frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceContextRecord {
    pub resource: String,
    pub revision: u64,
    pub workspace_type: String,
    pub plugin_digest: String,
    pub generation: u64,
    pub fields: Vec<String>,
    pub projection: crate::objects::ObjectRef,
    pub trace: Option<TraceKey>,
    pub actor: Option<crate::ids::PrincipalId>,
}
crate::impl_traced!(WorkspaceContextRecord, "workspace_context");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub id: String,
    pub command: String,
    pub before: Value,
    pub after: Value,
    pub trace: TraceKey,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub id: String,
    pub workspace_type: String,
    pub plugin_digest: String,
    pub title: String,
    pub revision: u64,
    pub data: Value,
    pub undo: Vec<Change>,
    pub redo: Vec<Change>,
    pub trace: TraceKey,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    schema: u32,
    resources: Vec<Resource>,
}
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("workspace changed; reload before applying this command")]
    Conflict,
    #[error("workspace resource is unavailable")]
    Missing,
    #[error("workspace input is invalid: {0}")]
    Invalid(String),
    #[error("workspace state could not be read or saved")]
    Store(String),
}
#[derive(Clone)]
pub struct WorkspaceStore {
    path: PathBuf,
    session: String,
}
impl WorkspaceStore {
    pub fn new(
        scope: &vak_config::scope::AgentScope,
        session: &str,
    ) -> Result<Self, WorkspaceError> {
        if session.is_empty()
            || session.len() > 128
            || !session
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        {
            return Err(WorkspaceError::Invalid("conversation identity".into()));
        }
        Ok(Self {
            path: scope.canvas_workspaces(session),
            session: session.into(),
        })
    }
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
    pub fn list(&self) -> Result<Vec<Resource>, WorkspaceError> {
        let raw = crate::documents::read_sealed(
            &self.path,
            &crate::objects::conversation_scope(&self.session),
        )
        .map_err(WorkspaceError::Store)?;
        Ok(decode(raw.as_deref())?.resources)
    }
    pub fn get(&self, id: &str) -> Result<Resource, WorkspaceError> {
        self.list()?
            .into_iter()
            .find(|item| item.id == id)
            .ok_or(WorkspaceError::Missing)
    }
    pub fn create(
        &self,
        workspace_type: &str,
        digest: &str,
        title: &str,
        data: Value,
        trace: &TraceKey,
    ) -> Result<Resource, WorkspaceError> {
        if trace.actor.is_none() {
            return Err(WorkspaceError::Invalid(
                "mutation needs an explicit actor".into(),
            ));
        }
        let resource = Resource {
            id: uuid::Uuid::now_v7().to_string(),
            workspace_type: workspace_type.into(),
            plugin_digest: digest.into(),
            title: title.into(),
            revision: 1,
            data,
            undo: vec![],
            redo: vec![],
            trace: trace.clone(),
        };
        self.update(|snapshot| {
            snapshot.resources.push(resource.clone());
            Ok(resource.clone())
        })
    }
    pub fn change(
        &self,
        id: &str,
        revision: u64,
        digest: &str,
        command: &str,
        data: Value,
        trace: &TraceKey,
    ) -> Result<Resource, WorkspaceError> {
        self.mutate(
            id,
            revision,
            digest,
            |resource| {
                let change = Change {
                    id: uuid::Uuid::now_v7().to_string(),
                    command: command.into(),
                    before: resource.data.clone(),
                    after: data.clone(),
                    trace: trace.clone(),
                };
                resource.data = data.clone();
                resource.undo.push(change);
                resource.redo.clear();
                Ok(())
            },
            trace,
        )
    }
    pub fn history(
        &self,
        id: &str,
        revision: u64,
        digest: &str,
        redo: bool,
        trace: &TraceKey,
    ) -> Result<Resource, WorkspaceError> {
        self.mutate(
            id,
            revision,
            digest,
            |resource| {
                let change = if redo {
                    resource.redo.pop()
                } else {
                    resource.undo.pop()
                }
                .ok_or_else(|| WorkspaceError::Invalid("no edit to undo or redo".into()))?;
                resource.data = if redo {
                    change.after.clone()
                } else {
                    change.before.clone()
                };
                if redo {
                    resource.undo.push(change);
                } else {
                    resource.redo.push(change);
                }
                Ok(())
            },
            trace,
        )
    }
    fn mutate(
        &self,
        id: &str,
        revision: u64,
        digest: &str,
        mut apply: impl FnMut(&mut Resource) -> Result<(), WorkspaceError>,
        trace: &TraceKey,
    ) -> Result<Resource, WorkspaceError> {
        if trace.actor.is_none() {
            return Err(WorkspaceError::Invalid(
                "mutation needs an explicit actor".into(),
            ));
        }
        self.update(|snapshot| {
            let resource = snapshot
                .resources
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or(WorkspaceError::Missing)?;
            if resource.revision != revision {
                return Err(WorkspaceError::Conflict);
            }
            apply(resource)?;
            resource.revision = resource
                .revision
                .checked_add(1)
                .ok_or_else(|| WorkspaceError::Invalid("revision overflow".into()))?;
            resource.trace = trace.clone();
            resource.plugin_digest = digest.into();
            Ok(resource.clone())
        })
    }
    fn update<T>(
        &self,
        mut apply: impl FnMut(&mut Snapshot) -> Result<T, WorkspaceError>,
    ) -> Result<T, WorkspaceError> {
        let mut refused = None;
        let result = crate::documents::update_sealed(
            &self.path,
            &crate::objects::conversation_scope(&self.session),
            |raw| {
                let mut snapshot = decode(raw).map_err(|error| error.to_string())?;
                let value = match apply(&mut snapshot) {
                    Ok(value) => value,
                    Err(error) => {
                        refused = Some(error);
                        return Err("workspace operation refused".into());
                    }
                };
                let text = serde_json::to_string(&snapshot).map_err(|error| error.to_string())?;
                if text.len() > 2 * 1024 * 1024 {
                    refused = Some(WorkspaceError::Invalid(
                        "resource state exceeds the editing limit; store large data as artifacts"
                            .into(),
                    ));
                    return Err("workspace operation refused".into());
                }
                Ok(Some((text, value)))
            },
        );
        if let Some(error) = refused {
            return Err(error);
        }
        result
            .map_err(WorkspaceError::Store)?
            .ok_or_else(|| WorkspaceError::Store("no receipt".into()))
    }
}
fn decode(raw: Option<&str>) -> Result<Snapshot, WorkspaceError> {
    let Some(raw) = raw else {
        return Ok(Snapshot {
            schema: 1,
            resources: vec![],
        });
    };
    let snapshot: Snapshot =
        serde_json::from_str(raw).map_err(|error| WorkspaceError::Store(error.to_string()))?;
    if snapshot.schema != 1 {
        return Err(WorkspaceError::Invalid(
            "unsupported resource schema".into(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    let uuid7 = |id: &str| uuid::Uuid::parse_str(id).is_ok_and(|id| id.get_version_num() == 7);
    for resource in &snapshot.resources {
        if !uuid7(&resource.id)
            || !ids.insert(&resource.id)
            || resource.revision == 0
            || resource.trace.actor.is_none()
            || resource.workspace_type.is_empty()
            || resource.plugin_digest.is_empty()
        {
            return Err(WorkspaceError::Invalid(
                "corrupt resource identity or provenance".into(),
            ));
        }
        let mut changes = std::collections::HashSet::new();
        for change in resource.undo.iter().chain(&resource.redo) {
            if !uuid7(&change.id) || !changes.insert(&change.id) || change.trace.actor.is_none() {
                return Err(WorkspaceError::Invalid("corrupt command history".into()));
            }
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    #[test]
    fn sealed_revisions_undo_redo_conflicts_and_conversation_scope() {
        let home = vak_config::paths::isolate_home_for_tests();
        let scope = vak_config::scope::SharedScope::new(&home).agent("workspace-test");
        let store = WorkspaceStore::new(&scope, "conversation-a").unwrap();
        let trace = TraceKey::root(
            crate::trace::local::tenant(),
            crate::trace::local::space(&home),
            crate::trace::local::agent("workspace-test"),
            crate::trace::Cause::User {
                request_id: uuid::Uuid::now_v7().to_string(),
            },
        )
        .acting(crate::trace::local::local_owner(), None);
        let runs = crate::runs::Runs::at(
            home.join("runs"),
            vak_config::paths::tenant_home_at(&home, vak_config::paths::LOCAL_TENANT),
        );
        let mut run = runs.begin(&trace, None, None).unwrap();
        let made = store
            .create(
                "workspace-lab/parameters",
                "digest",
                "Component",
                serde_json::json!({"length":12}),
                &trace,
            )
            .unwrap();
        let changed = store
            .change(
                &made.id,
                1,
                "digest",
                "set",
                serde_json::json!({"length":24}),
                &trace,
            )
            .unwrap();
        assert_eq!(changed.revision, 2);
        assert!(matches!(
            store.change(&made.id, 1, "digest", "set", serde_json::json!({}), &trace),
            Err(WorkspaceError::Conflict)
        ));
        let undo = store.history(&made.id, 2, "digest", false, &trace).unwrap();
        assert_eq!(undo.data["length"], 12);
        let redo = store.history(&made.id, 3, "digest", true, &trace).unwrap();
        assert_eq!(redo.data["length"], 24);
        assert!(
            WorkspaceStore::new(&scope, "conversation-b")
                .unwrap()
                .get(&made.id)
                .is_err()
        );
        assert!(
            crate::documents::read(&store.path)
                .unwrap()
                .unwrap()
                .contains("ciphertext")
        );
        assert!(
            !crate::documents::read(&store.path)
                .unwrap()
                .unwrap()
                .contains("length")
        );
        assert!(
            crate::documents::read_sealed(
                &store.path,
                &crate::objects::conversation_scope("conversation-b")
            )
            .is_err()
        );
        let concurrent = store
            .create(
                "workspace-lab/parameters",
                "digest",
                "Concurrent",
                serde_json::json!({"length":0}),
                &trace,
            )
            .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads = (1..=2)
            .map(|length| {
                let store = store.clone();
                let id = concurrent.id.clone();
                let trace = trace.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.change(
                        &id,
                        1,
                        "digest",
                        "set",
                        serde_json::json!({"length":length}),
                        &trace,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(store.get(&concurrent.id).unwrap().revision, 2);
        run.settle_with(crate::runs::RunOutcome::Completed, None);
    }
}
