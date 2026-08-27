#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use async_trait::async_trait;
use std::sync::Arc;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use vak_agent::{ModelResponse, Provider, ToolDispatcher};
use vak_domain::{
    PermissionMode, ProjectContext, ProjectId, RouteLeg, RunId, SandboxMode, SessionContract,
    SessionId,
};
use vak_runtime::Runtime;
use vak_session::{EntryPayload, SessionLog, SessionPath};

struct ProviderOnce;

#[async_trait]
impl Provider for ProviderOnce {
    async fn respond(
        &self,
        _context: &vak_domain::RunContext,
        _transcript: &str,
        _cancel: CancellationToken,
    ) -> Result<ModelResponse, vak_agent::EngineError> {
        Ok(ModelResponse {
            text: "answer".into(),
            tool_calls: Vec::new(),
        })
    }
}

struct NoTools;

#[async_trait]
impl ToolDispatcher for NoTools {
    async fn dispatch(
        &self,
        _context: &vak_domain::RunContext,
        _call: &vak_agent::ToolCall,
        _cancel: CancellationToken,
    ) -> Result<vak_agent::ToolResult, vak_agent::EngineError> {
        Ok(vak_agent::ToolResult {
            output: "unused".into(),
            is_error: true,
        })
    }
}

fn contract() -> SessionContract {
    SessionContract {
        provider: "test".into(),
        model: "test".into(),
        route_ladder: vec![RouteLeg {
            provider: "test".into(),
            model: "test".into(),
        }],
        system_prompt: "system".into(),
        permission_mode: PermissionMode::ReadOnly,
        sandbox: SandboxMode::None,
        tool_catalogue_revision: "test".into(),
        context_limit: 4096,
        budget_ceiling: None,
    }
}

#[tokio::test]
async fn runtime_run_reconstructs_model_visible_messages_from_session_log() {
    let home = tempdir().expect("home");
    let project = tempdir().expect("project");
    let runtime =
        Runtime::open_with_provider(home.path(), Arc::new(ProviderOnce), Arc::new(NoTools))
            .expect("runtime");
    let project_id = ProjectId::new();
    let session_id = SessionId::new();
    let run_id = RunId::new();
    runtime
        .register_project(ProjectContext {
            id: project_id.clone(),
            root: project.path().display().to_string(),
            display_name: None,
        })
        .await
        .expect("project");
    runtime
        .create_session(session_id.clone(), project_id.clone(), contract())
        .await
        .expect("session");
    let handle = runtime
        .start_run(run_id, session_id.clone(), project_id.clone())
        .await
        .expect("run");
    runtime
        .clone()
        .execute_run(handle, session_id.clone(), "question".into())
        .await;
    let path = SessionPath::new_session_file(home.path(), project_id.as_str(), session_id.as_str())
        .expect("path");
    let log = SessionLog::open(path).expect("log");
    let messages = log.derive_messages();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].text_content(), "question");
    assert_eq!(messages[1].text_content(), "answer");
    assert!(
        log.entries()
            .iter()
            .any(|entry| matches!(entry.payload, EntryPayload::Message(_)))
    );
}
