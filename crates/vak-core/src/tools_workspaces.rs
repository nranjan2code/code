//! Session-bound workspace reads/edits through the normal tool permission path.
use serde_json::{Value, json};
use vak_tools::{Fault, Tool, ToolContext, ToolOutput};

pub struct WorkspaceTool {
    pub core: crate::Core,
    pub agent: String,
    pub session: String,
    pub edit: bool,
}
#[async_trait::async_trait]
impl Tool for WorkspaceTool {
    fn name(&self) -> &str {
        if self.edit {
            "workspace_edit"
        } else {
            "workspace_read"
        }
    }
    fn description(&self) -> &str {
        if self.edit {
            "Apply a declared command or undo/redo to an installed Canvas workspace in this conversation. Read its current revision, activation digest/generation and command arguments with workspace_read first. Commands cannot execute host code or bypass external tool permissions."
        } else {
            "List installed Canvas workspace types and saved resources, or read one resource's current state and declared commands. This is scoped to the current conversation. Returned data is the saved version, not unsent UI edits."
        }
    }
    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(if self.edit {
            vak_permission::Reach::Learning
        } else {
            vak_permission::Reach::ReadStore
        })
    }
    fn serves(&self) -> Vec<&str> {
        vec!["documents"]
    }
    fn schema(&self) -> Value {
        if self.edit {
            json!({"type":"object","properties":{
            "action":{"type":"string","enum":["create","command","undo","redo"]},
            "workspace_type":{"type":"string"},"resource":{"type":"string"},"revision":{"type":"integer"},
            "digest":{"type":"string"},"generation":{"type":"integer"},"command":{"type":"string"},"arguments":{"type":"object"}
        },"required":["action","digest","generation"],"additionalProperties":false})
        } else {
            json!({"type":"object","properties":{"resource":{"type":"string"}},"additionalProperties":false})
        }
    }
    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let perform = || -> Result<Value, String> {
            if ctx.cancel.is_cancelled() {
                return Err("workspace operation was cancelled".into());
            }
            if self.session.is_empty() {
                return Err("workspace tools require an admitted conversation".into());
            }
            let store = vak_session::workspaces::WorkspaceStore::new(
                &self.core.shared_scope().agent(&self.agent),
                &self.session,
            )
            .map_err(|error| error.to_string())?;
            if self.edit {
                let trace = ctx
                    .trace
                    .as_ref()
                    .ok_or("workspace edits require an admitted run")?;
                let action =
                    serde_json::from_value(args.clone()).map_err(|_| "invalid workspace action")?;
                if ctx.cancel.is_cancelled() {
                    return Err("workspace operation was cancelled".into());
                }
                let resource = self
                    .core
                    .apply_workspace(&store, action, trace)
                    .map_err(|error| error.to_string())?;
                Ok(
                    json!({"resource":resource.id,"workspace_type":resource.workspace_type,"revision":resource.revision,"data":resource.data,"undo_available":!resource.undo.is_empty(),"redo_available":!resource.redo.is_empty()}),
                )
            } else {
                let definitions = self
                    .core
                    .canvas_workspaces()
                    .map_err(|error| error.to_string())?;
                if let Some(id) = args.get("resource").and_then(Value::as_str) {
                    let resource = store.get(id).map_err(|error| error.to_string())?;
                    let definition = definitions
                        .iter()
                        .find(|item| item.spec.id == resource.workspace_type);
                    Ok(
                        json!({"resource":resource.id,"workspace_type":resource.workspace_type,"revision":resource.revision,"data":resource.data,"contribution":definition,"undo_available":!resource.undo.is_empty(),"redo_available":!resource.redo.is_empty()}),
                    )
                } else {
                    let resources=store.list().map_err(|error|error.to_string())?.into_iter().map(|resource|json!({"id":resource.id,"workspace_type":resource.workspace_type,"title":resource.title,"revision":resource.revision})).collect::<Vec<_>>();
                    Ok(json!({"contributions":definitions,"resources":resources}))
                }
            }
        };
        match perform() {
            Ok(value) => ToolOutput::ok(value.to_string()),
            Err(error) => ToolOutput::error(Fault::Tool, error),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn agent_tools_share_revision_activation_and_conversation_boundaries() {
        vak_config::paths::isolate_home_for_tests();
        let project = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::create_dir(source.path().join("workspaces")).unwrap();
        std::fs::write(
            source.path().join("vak-plugin.json"),
            include_str!("../../../examples/canvas-workspace/vak-plugin.json"),
        )
        .unwrap();
        std::fs::write(
            source.path().join("workspaces/parameters.json"),
            include_str!("../../../examples/canvas-workspace/workspaces/parameters.json"),
        )
        .unwrap();
        let core = crate::Core::new_with_trust(project.path().into(), true)
            .unwrap()
            .with_run_admission(
                crate::admission::RunAdmission::default()
                    .actor(vak_session::trace::local::local_owner()),
            );
        let plugins = vak_plugin::PluginStore::new(&core.capability_roots()[0].path);
        let package = plugins
            .install_local(source.path(), vak_plugin::InstallOptions::default())
            .unwrap();
        plugins.enable(&package.name).unwrap();
        let definition = core.canvas_workspaces().unwrap().remove(0);
        let trace = core.mint_trace(None);
        let mut run = core.runs().begin(&trace, None, None).unwrap();
        let ctx = ToolContext::new(project.path().into()).with_trace(trace);
        let mut tool = WorkspaceTool {
            core: core.clone(),
            agent: "vak".into(),
            session: "agent-workspace-test".into(),
            edit: true,
        };
        let create = json!({"action":"create","workspace_type":definition.spec.id,"digest":definition.digest,"generation":definition.generation});
        let created = tool.execute(&create, &ctx).await;
        assert!(created.fault.is_none(), "{}", created.content);
        let resource: Value = serde_json::from_str(&created.content).unwrap();
        let command = json!({"action":"command","resource":resource["resource"],"revision":1,"digest":definition.digest,"generation":definition.generation,"command":"set-parameters","arguments":{"name":"Stator","length":48,"enabled":false}});
        assert!(tool.execute(&command, &ctx).await.fault.is_none());
        assert!(tool.execute(&command, &ctx).await.fault.is_some());
        tool.session = "another-agent-conversation".into();
        let mut current = command;
        current["revision"] = json!(2);
        assert!(tool.execute(&current, &ctx).await.fault.is_some());
        tool.session = "agent-workspace-test".into();
        plugins.disable(&package.name).unwrap();
        assert!(tool.execute(&current, &ctx).await.fault.is_some());
        run.settle_with(vak_session::runs::RunOutcome::Completed, None);
    }
}
