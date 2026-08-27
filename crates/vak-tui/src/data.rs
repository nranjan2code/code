use vak_client::{Client, ClientError, Config, EventStream, Session};
use vak_domain::{
    PermissionMode, ProjectContext, ProjectId, RunId, SandboxMode, SessionContract, SessionId,
};

#[derive(Clone)]
pub struct ClientData {
    pub client: Client,
    pub project: ProjectContext,
}
fn error(error: ClientError) -> String {
    format!("Runtime request failed: {error}")
}

impl ClientData {
    pub async fn connect(url: &str, token: &str, root: String) -> Result<Self, String> {
        let client = Client::new(url, token).map_err(error)?;
        Self::connect_client(client, root).await
    }

    /// Attach an already-resolved typed Runtime client. Connection discovery
    /// belongs to `vak-client`, not to this terminal adapter.
    pub async fn connect_client(client: Client, root: String) -> Result<Self, String> {
        client.health().await.map_err(error)?;
        client.version().await.map_err(error)?;
        let project = client
            .projects()
            .await
            .map_err(error)?
            .into_iter()
            .find(|p| p.root == root)
            .unwrap_or_else(|| ProjectContext {
                id: ProjectId::new(),
                root: root.clone(),
                display_name: Some(root),
            });
        if !client
            .projects()
            .await
            .map_err(error)?
            .iter()
            .any(|p| p.id == project.id)
        {
            client.register_project(&project).await.map_err(error)?;
        }
        Ok(Self { client, project })
    }
    pub async fn sessions(&self) -> Result<Vec<Session>, String> {
        self.client
            .sessions(Some(&self.project.id))
            .await
            .map_err(error)
    }
    pub async fn create_session(&self) -> Result<SessionId, String> {
        self.client
            .create_session(&vak_client::CreateSession {
                session_id: None,
                project_id: self.project.id.clone(),
                contract: SessionContract {
                    provider: "default".into(),
                    model: "default".into(),
                    route_ladder: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: PermissionMode::ReadOnly,
                    sandbox: SandboxMode::Landlock,
                    tool_catalogue_revision: "runtime".into(),
                    context_limit: 0,
                    budget_ceiling: None,
                },
            })
            .await
            .map(|r| r.session_id)
            .map_err(error)
    }
    pub async fn run(
        &self,
        session_id: SessionId,
        input: String,
    ) -> Result<(RunId, EventStream), String> {
        let response = self
            .client
            .start_run(&vak_client::StartRun {
                run_id: None,
                session_id,
                project_id: self.project.id.clone(),
                input,
            })
            .await
            .map_err(error)?;
        let id = response.run.run_id.clone();
        Ok((id.clone(), self.client.events(&id).await.map_err(error)?))
    }
    pub async fn tasks(&self) -> Result<Vec<vak_client::Task>, String> {
        self.client
            .tasks(Some(&self.project.id))
            .await
            .map_err(error)
    }
    pub async fn memory(&self) -> Result<Vec<vak_client::Memory>, String> {
        self.client
            .memory(Some(&self.project.id), "workspace")
            .await
            .map_err(error)
    }
    pub async fn config(&self) -> Result<Config, String> {
        self.client.config(&self.project.id).await.map_err(error)
    }
}
