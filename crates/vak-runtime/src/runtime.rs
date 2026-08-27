use super::{RunError, RunSupervisor, WorkspaceLeaseManager};
use crate::RunSnapshot;
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use tokio::sync::mpsc;
use tokio::sync::{Mutex, broadcast};
use vak_agent::{
    AgentEngine, EngineError, LlmProviderAdapter, Provider, SessionRecorder, ToolDispatcher,
};
pub use vak_config::{ConfigService, SecretService};
use vak_domain::{Event, ProjectContext, ProjectId, RunId, SessionContract, SessionId};
use vak_services::Services;
pub use vak_services::SkillRecord;
pub use vak_services::{ApprovalRecord, BindingRecord, DeliveryRecord, InboxRecord, TaskRecord};
pub use vak_services::{CheckpointRecord, MemoryRecord};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BackupReport {
    pub path: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub renamed: u64,
    pub skipped: u64,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FlowRecord {
    pub name: String,
    pub path: PathBuf,
    pub valid: bool,
    pub nodes: usize,
}
use vak_session::{FrozenContract, SessionHeader, SessionLog};
use vak_storage::StorageError;

fn provider_env(provider: &str) -> String {
    match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "google" => "GOOGLE_API_KEY",
        "ollama" => "OLLAMA_API_KEY",
        "opencode-zen" => "OPENCODE_API_KEY",
        _ => "OPENAI_API_KEY",
    }
    .to_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("storage: {0}")]
    Storage(#[from] StorageError),
    #[error("run: {0}")]
    Run(#[from] RunError),
    #[error("project root does not exist: {0}")]
    MissingProject(PathBuf),
    #[error("runtime has no provider configured")]
    ProviderUnavailable,
    #[error("session is not found: {0}")]
    SessionNotFound(SessionId),
    #[error("run execution: {0}")]
    Execution(String),
    #[error("configuration: {0}")]
    Config(#[from] vak_config::ConfigError),
    #[error("provider: {0}")]
    Provider(String),
    #[error("capability revocation timed out after {0:?}")]
    RevocationTimeout(Duration),
}

/// The single mutable application authority used by every surface.
pub struct Runtime {
    data_home: PathBuf,
    services: Mutex<Services>,
    pub runs: RunSupervisor,
    pub workspaces: WorkspaceLeaseManager,
    events: broadcast::Sender<Event>,
    provider: Option<Arc<dyn Provider>>,
    tools: Option<Arc<dyn ToolDispatcher>>,
    active_sessions: Mutex<HashSet<SessionId>>,
}

impl Runtime {
    pub fn open(data_home: impl Into<PathBuf>) -> Result<Arc<Self>, RuntimeError> {
        Self::open_with_ports(data_home, None, None)
    }

    pub fn open_with_provider(
        data_home: impl Into<PathBuf>,
        provider: Arc<dyn Provider>,
        tools: Arc<dyn ToolDispatcher>,
    ) -> Result<Arc<Self>, RuntimeError> {
        Self::open_with_ports(data_home, Some(provider), Some(tools))
    }

    pub fn open_with_llm_provider(
        data_home: impl Into<PathBuf>,
        provider: Arc<dyn vak_llm::Provider>,
        tools: Arc<dyn ToolDispatcher>,
    ) -> Result<Arc<Self>, RuntimeError> {
        Self::open_with_provider(
            data_home,
            Arc::new(LlmProviderAdapter::new(provider)),
            tools,
        )
    }

    /// Build a production runtime from the canonical configuration and secret
    /// stores. Provider credentials are resolved once at admission; they are
    /// never exposed to the agent engine or tool dispatcher.
    pub fn open_configured(
        data_home: impl Into<PathBuf>,
        project_root: impl Into<PathBuf>,
        tools: Arc<dyn ToolDispatcher>,
    ) -> Result<Arc<Self>, RuntimeError> {
        let data_home = data_home.into();
        let project_root = project_root.into();
        let snapshot = ConfigService::new(&data_home, &project_root).load()?;
        let provider_name = snapshot
            .config
            .provider
            .name
            .unwrap_or_else(|| "anthropic".to_owned());
        let key_name = match provider_name.as_str() {
            "anthropic" => "ANTHROPIC_API_KEY",
            "google" => "GOOGLE_API_KEY",
            "ollama" => "OLLAMA_API_KEY",
            "opencode-zen" => "OPENCODE_API_KEY",
            _ => "OPENAI_API_KEY",
        };
        let api_key = SecretService::new(&data_home)
            .get(key_name)?
            .or_else(|| std::env::var(key_name).ok())
            .unwrap_or_default();
        // A missing provider credential is a configuration gap, not a reason
        // to refuse to open. The console that sets the key is served by this
        // very process, so failing here is a deadlock: the only supported way
        // to supply the credential would require the credential. Runs already
        // return a typed error while no provider is attached.
        if api_key.is_empty() && provider_name != "ollama" {
            return Self::open_with_ports(data_home, None, Some(tools));
        }
        let registry = vak_llm::registry::default_registry();
        let provider = registry
            .get(
                &provider_name,
                &vak_llm::ProviderAuth {
                    api_key,
                    base_url: snapshot.config.provider.endpoint,
                },
            )
            .map_err(|error| RuntimeError::Provider(error.to_string()))?;
        Self::open_with_llm_provider(data_home, provider, tools)
    }

    fn open_with_ports(
        data_home: impl Into<PathBuf>,
        provider: Option<Arc<dyn Provider>>,
        tools: Option<Arc<dyn ToolDispatcher>>,
    ) -> Result<Arc<Self>, RuntimeError> {
        let data_home = data_home.into();
        std::fs::create_dir_all(&data_home).map_err(StorageError::Io)?;
        let services = Services::open(&data_home)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        services
            .state
            .recover_active_runs(&Utc::now().to_rfc3339())?;
        let (events, _) = broadcast::channel(256);
        Ok(Arc::new(Self {
            data_home,
            services: Mutex::new(services),
            runs: RunSupervisor::new(),
            workspaces: WorkspaceLeaseManager::new(),
            events,
            provider,
            tools,
            active_sessions: Mutex::new(HashSet::new()),
        }))
    }

    pub fn data_home(&self) -> &Path {
        &self.data_home
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn publish_event(&self, event: Event) {
        let _ = self.events.send(event);
    }

    pub fn config_for(&self, project_root: impl Into<PathBuf>) -> ConfigService {
        ConfigService::new(&self.data_home, project_root)
    }

    pub fn secrets(&self) -> SecretService {
        SecretService::new(&self.data_home)
    }

    pub fn set_provider_key(&self, provider: &str, key: &str) -> Result<String, RuntimeError> {
        let env = provider_env(provider);
        self.secrets().set(&env, key)?;
        Ok(env)
    }

    pub fn remove_provider_key(&self, provider: &str) -> Result<(String, bool), RuntimeError> {
        let env = provider_env(provider);
        let removed = self.secrets().remove(&env)?;
        Ok((env, removed))
    }

    pub async fn discover_models(&self, provider: &str) -> Result<Vec<String>, RuntimeError> {
        let env = provider_env(provider);
        let key = self
            .secrets()
            .get(&env)?
            .or_else(|| std::env::var(&env).ok())
            .unwrap_or_default();
        let base_url = self
            .list_projects()
            .await?
            .into_iter()
            .next()
            .and_then(|project| self.config_for(project.root).load().ok())
            .and_then(|snapshot| snapshot.config.provider.endpoint);
        vak_llm::models::list_models(
            provider,
            &vak_llm::ProviderAuth {
                api_key: key,
                base_url,
            },
        )
        .await
        .map_err(|error| RuntimeError::Provider(error.to_string()))
    }

    pub async fn list_projects(&self) -> Result<Vec<ProjectContext>, RuntimeError> {
        let rows = self.services.lock().await.state.list_projects()?;
        rows.into_iter()
            .map(|(id, root, metadata)| {
                let id = ProjectId::from_string(id)
                    .map_err(|error| StorageError::Io(std::io::Error::other(error.to_string())))?;
                let display_name = serde_json::from_str::<serde_json::Value>(&metadata)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("display_name")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                    });
                Ok(ProjectContext {
                    id,
                    root,
                    display_name,
                })
            })
            .collect()
    }

    pub async fn register_project(&self, project: ProjectContext) -> Result<(), RuntimeError> {
        let root = std::fs::canonicalize(&project.root)
            .map_err(|_| RuntimeError::MissingProject(PathBuf::from(&project.root)))?;
        let project = ProjectContext {
            root: root.to_string_lossy().into_owned(),
            ..project
        };
        if let Some((_, existing_root, _)) = self
            .services
            .lock()
            .await
            .state
            .list_projects()?
            .into_iter()
            .find(|(id, _, _)| id == project.id.as_str())
        {
            if existing_root == project.root {
                return Ok(());
            }
            return Err(RuntimeError::Execution(format!(
                "project id {} is already registered for a different root",
                project.id
            )));
        }
        let now = Utc::now().to_rfc3339();
        let metadata = serde_json::json!({"display_name": project.display_name});
        self.services.lock().await.state.create_project(
            project.id.as_str(),
            &root.to_string_lossy(),
            &now,
            &metadata.to_string(),
        )?;
        self.services
            .lock()
            .await
            .audit
            .append(&vak_storage::AuditEvent {
                event: "project.registered".to_owned(),
                timestamp: now,
                actor: "runtime".to_owned(),
                data: serde_json::to_value(&project)
                    .map_err(|error| RuntimeError::Execution(format!("project audit: {error}")))?,
            })?;
        let _ = self.events.send(Event::ProjectRegistered { project });
        Ok(())
    }

    pub async fn create_session(
        &self,
        session_id: SessionId,
        project_id: ProjectId,
        contract: SessionContract,
    ) -> Result<(), RuntimeError> {
        let now = Utc::now().to_rfc3339();
        let project = self
            .services
            .lock()
            .await
            .state
            .connection()
            .query_row(
                "SELECT root FROM projects WHERE id=?1",
                [project_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .map_err(vak_storage::StorageError::Sqlite)?;
        let path = vak_session::SessionPath::new_session_file(
            &self.data_home,
            project_id.as_str(),
            session_id.as_str(),
        )
        .map_err(|error| StorageError::Io(std::io::Error::other(error.to_string())))?;
        let header = SessionHeader {
            session_id: session_id.to_string(),
            project_id: project_id.to_string(),
            created_at: Utc::now(),
            project_root: PathBuf::from(project),
            parent_session_id: None,
            contract: FrozenContract {
                app_version: env!("CARGO_PKG_VERSION").to_owned(),
                provider: contract.provider.clone(),
                model: contract.model.clone(),
                system_prompt: contract.system_prompt.clone(),
                tools: Vec::new(),
                permission_mode: format!("{:?}", contract.permission_mode),
            },
        };
        SessionLog::create(path.clone(), header)
            .map_err(|error| StorageError::Io(std::io::Error::other(error.to_string())))?;
        let put_result = self.services.lock().await.state.put_session(
            &session_id.to_string(),
            &project_id.to_string(),
            &now,
            "idle",
            &serde_json::to_string(&contract)
                .map_err(|error| RuntimeError::Execution(format!("session contract: {error}")))?,
        );
        if let Err(error) = put_result {
            let _ = std::fs::remove_file(path);
            return Err(error.into());
        }
        let _ = self.events.send(Event::SessionCreated {
            session_id,
            project_id,
        });
        Ok(())
    }

    pub async fn start_run(
        &self,
        run_id: RunId,
        session_id: vak_domain::SessionId,
        project_id: ProjectId,
    ) -> Result<super::RunHandle, RuntimeError> {
        let session = self
            .services
            .lock()
            .await
            .state
            .get_session(session_id.as_str())?
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        if session.0 != project_id.as_str() {
            return Err(RuntimeError::Execution(
                "session does not belong to project".into(),
            ));
        }
        if !self.active_sessions.lock().await.insert(session_id.clone()) {
            return Err(RuntimeError::Execution(
                "session already has an active run".into(),
            ));
        }
        let now = Utc::now().to_rfc3339();
        let epoch = self.runs.current_epoch();
        let handle = match self.runs.start(run_id.clone(), project_id.clone()).await {
            Ok(handle) => handle,
            Err(error) => {
                self.active_sessions.lock().await.remove(&session_id);
                return Err(error.into());
            }
        };
        if let Err(error) = self.services.lock().await.state.create_run(
            &run_id.to_string(),
            &session_id.to_string(),
            "running",
            &now,
            epoch.0 as i64,
        ) {
            self.active_sessions.lock().await.remove(&session_id);
            let _ = handle.fail().await;
            return Err(error.into());
        }
        let _ = self.services.lock().await.audit(&vak_storage::AuditEvent {
            event: "run.started".to_owned(),
            timestamp: now.clone(),
            actor: "runtime".to_owned(),
            data: serde_json::json!({
                "run_id": run_id,
                "session_id": session_id,
                "project_id": project_id,
                "capability_epoch": epoch.0,
            }),
        });
        let _ = self.events.send(Event::RunStatusChanged {
            run_id,
            status: vak_domain::RunStatus::Running,
        });
        Ok(handle)
    }

    pub async fn list_sessions(
        &self,
        project_id: Option<ProjectId>,
    ) -> Result<Vec<(SessionId, ProjectId, String, String)>, RuntimeError> {
        let rows = self
            .services
            .lock()
            .await
            .state
            .list_sessions(project_id.as_ref().map(ProjectId::as_str))?;
        rows.into_iter()
            .map(|(id, project, created, status)| {
                Ok((
                    SessionId::from_string(id)
                        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?,
                    ProjectId::from_string(project)
                        .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?,
                    created,
                    status,
                ))
            })
            .collect()
    }

    pub async fn session_messages(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<vak_llm::Message>, RuntimeError> {
        let project_id = self
            .services
            .lock()
            .await
            .state
            .session_project(session_id.as_str())?
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.clone()))?;
        let path = vak_session::SessionPath::new_session_file(
            &self.data_home,
            &project_id,
            session_id.as_str(),
        )
        .map_err(|error| StorageError::Io(std::io::Error::other(error.to_string())))?;
        let log = SessionLog::open(path)
            .map_err(|error| RuntimeError::Execution(format!("session open: {error}")))?;
        Ok(log.derive_messages())
    }

    pub async fn list_runs(
        &self,
        session_id: Option<SessionId>,
    ) -> Result<Vec<RunSnapshot>, RuntimeError> {
        let rows = self
            .services
            .lock()
            .await
            .state
            .list_runs(session_id.as_ref().map(SessionId::as_str))?;
        let mut result = Vec::with_capacity(rows.len());
        for (id, session, status, epoch) in rows {
            let id = RunId::from_string(id)
                .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
            let session = SessionId::from_string(session)
                .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
            let project = self
                .services
                .lock()
                .await
                .state
                .session_project(session.as_str())?
                .ok_or_else(|| RuntimeError::SessionNotFound(session.clone()))?;
            let project = ProjectId::from_string(project)
                .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
            let status = parse_status(&status)?;
            result.push(RunSnapshot {
                run_id: id,
                project_id: project,
                capability_epoch: vak_domain::CapabilityEpoch(epoch as u64),
                status,
            });
        }
        Ok(result)
    }

    pub async fn execute_run(
        self: Arc<Self>,
        handle: super::RunHandle,
        session_id: SessionId,
        input: String,
    ) {
        let run_id = handle.run_id().clone();
        let result = self.execute_run_inner(&handle, &session_id, &input).await;
        let (status, output) = match result {
            Ok(output) => (vak_domain::RunStatus::Completed, output),
            Err(EngineError::Cancelled) => (vak_domain::RunStatus::Cancelled, String::new()),
            Err(EngineError::CancelledWithOutput(output)) => {
                (vak_domain::RunStatus::Cancelled, output)
            }
            Err(error) => (vak_domain::RunStatus::Failed, error.to_string()),
        };
        let final_status = if handle.is_cancelled() {
            vak_domain::RunStatus::Cancelled
        } else {
            status
        };
        let won = match final_status {
            vak_domain::RunStatus::Completed => {
                let _ = handle.begin_finishing().await;
                handle.complete().await
            }
            vak_domain::RunStatus::Cancelled => handle.cancelled().await,
            _ => handle.fail().await,
        }
        .unwrap_or(false);
        if !won {
            self.active_sessions.lock().await.remove(&session_id);
            return;
        }
        let now = Utc::now().to_rfc3339();
        let outcome = serde_json::json!({"output": output, "status": format!("{final_status:?}")});
        let state = self.services.lock().await;
        let _ = state.state.finish_run(
            run_id.as_str(),
            &format!("{final_status:?}"),
            &now,
            &outcome.to_string(),
        );
        let _ = state.state.connection().execute(
            "UPDATE sessions SET status='idle' WHERE id=?1",
            [session_id.as_str()],
        );
        let _ = state.audit(&vak_storage::AuditEvent {
            event: "run.finished".to_owned(),
            timestamp: now.clone(),
            actor: "runtime".to_owned(),
            data: serde_json::json!({
                "run_id": run_id,
                "session_id": session_id,
                "status": format!("{final_status:?}"),
            }),
        });
        drop(state);
        self.active_sessions.lock().await.remove(&session_id);
        let _ = self.events.send(Event::RunStatusChanged {
            run_id: run_id.clone(),
            status: final_status,
        });
        let _ = self.events.send(Event::RunFinished {
            run_id,
            status: final_status,
            output,
        });
    }

    async fn execute_run_inner(
        &self,
        handle: &super::RunHandle,
        session_id: &SessionId,
        input: &str,
    ) -> Result<String, EngineError> {
        let provider = self.provider.clone().ok_or_else(|| {
            EngineError::Provider(
                "no provider credential is configured; set one with \
                     `vakcoder admin` or add the provider API key to \
                     <data_home>/.env, then restart the gateway"
                    .into(),
            )
        })?;
        let tools = self.tools.clone().ok_or_else(|| {
            EngineError::Provider("runtime has no tool dispatcher configured".into())
        })?;
        let session = self
            .services
            .lock()
            .await
            .state
            .get_session(session_id.as_str())
            .map_err(|e| EngineError::Provider(e.to_string()))?
            .ok_or_else(|| EngineError::Provider("session not found".into()))?;
        let contract: SessionContract =
            serde_json::from_str(&session.2).map_err(|e| EngineError::Provider(e.to_string()))?;
        let project_root = self
            .services
            .lock()
            .await
            .state
            .session_root(session_id.as_str())
            .map_err(|e| EngineError::Provider(e.to_string()))?
            .ok_or_else(|| EngineError::Provider("project not found".into()))?;
        let project_id = self
            .services
            .lock()
            .await
            .state
            .session_project(session_id.as_str())
            .map_err(|e| EngineError::Provider(e.to_string()))?
            .ok_or_else(|| EngineError::Provider("project not found".into()))?;
        let project_id =
            ProjectId::from_string(project_id).map_err(|e| EngineError::Provider(e.to_string()))?;
        let cancellation = handle.cancellation();
        let _lease = tokio::select! {
            _ = cancellation.cancelled() => return Err(EngineError::Cancelled),
            lease = self.workspaces.acquire(project_id.clone(), super::LeaseKind::Write) => lease,
        };
        let session_path = vak_session::SessionPath::new_session_file(
            &self.data_home,
            &project_id.to_string(),
            session_id.as_str(),
        )
        .map_err(|e| EngineError::Provider(e.to_string()))?;
        let log = SessionLog::open(session_path)
            .map_err(|e| EngineError::Provider(format!("session open: {e}")))?;
        let recorder = Arc::new(SessionRecorder(Arc::new(tokio::sync::Mutex::new(log))));
        let context = vak_domain::RunContext {
            run_id: handle.run_id().clone(),
            session_id: session_id.clone(),
            project: ProjectContext {
                id: project_id,
                root: project_root,
                display_name: None,
            },
            contract,
            capability_epoch: handle.capability_epoch(),
        };
        let engine = AgentEngine::new(
            provider,
            tools,
            Arc::new(SupervisorGuard {
                supervisor: self.runs.clone(),
            }),
        );
        let (events, mut rx) = mpsc::channel(64);
        let forward = self.events.clone();
        let forward_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let _ = forward.send(event);
            }
        });
        let result = engine
            .run_with_recorder(
                &context,
                input.to_owned(),
                handle.cancellation(),
                events,
                Some(recorder),
            )
            .await;
        forward_task.abort();
        result.map(|r| r.output)
    }

    pub async fn cancel_run(&self, run_id: &RunId) -> Result<bool, RuntimeError> {
        Ok(self.runs.cancel(run_id).await?)
    }

    pub async fn finish_run(
        &self,
        run_id: &RunId,
        status: vak_domain::RunStatus,
        output: &str,
    ) -> Result<bool, RuntimeError> {
        if !status.is_terminal() {
            return Ok(false);
        }
        let changed = self.services.lock().await.state.finish_run(
            run_id.as_str(),
            &format!("{status:?}").to_lowercase(),
            &Utc::now().to_rfc3339(),
            &serde_json::json!({"output": output}).to_string(),
        )?;
        if changed {
            let _ = self.events.send(Event::RunFinished {
                run_id: run_id.clone(),
                status,
                output: output.to_owned(),
            });
        }
        Ok(changed)
    }

    pub async fn revoke_capabilities(&self) -> vak_domain::CapabilityEpoch {
        self.runs.revoke_capabilities().await
    }

    pub async fn revoke_capabilities_barrier(
        &self,
        timeout: Duration,
    ) -> Result<vak_domain::CapabilityEpoch, RuntimeError> {
        let old = self.runs.running_ids().await;
        let epoch = self.runs.revoke_capabilities().await;
        let wait = async {
            for run_id in &old {
                let _ = self.runs.wait_terminal(run_id).await?;
            }
            Ok::<(), RunError>(())
        };
        if tokio::time::timeout(timeout, wait).await.is_err() {
            for run_id in old {
                let _ = self.runs.force_cancelled(&run_id).await;
                let services = self.services.lock().await;
                if let Ok(Some(record)) = services.state.get_run(run_id.as_str()) {
                    let _ = services.state.finish_run(
                        run_id.as_str(),
                        "cancelled",
                        &Utc::now().to_rfc3339(),
                        r#"{"reason":"capability_revoked"}"#,
                    );
                    let _ = services.state.connection().execute(
                        "UPDATE sessions SET status='idle' WHERE id=?1",
                        [record.session_id.as_str()],
                    );
                    drop(services);
                    self.active_sessions.lock().await.remove(
                        &SessionId::from_string(record.session_id)
                            .map_err(|error| RuntimeError::Execution(error.to_string()))?,
                    );
                }
            }
            return Err(RuntimeError::RevocationTimeout(timeout));
        }
        Ok(epoch)
    }

    pub async fn tasks(
        &self,
        project_id: Option<&ProjectId>,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .list_tasks(project_id.map(ProjectId::as_str))
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn create_task(
        &self,
        id: impl AsRef<str>,
        project_id: Option<ProjectId>,
        spec: serde_json::Value,
    ) -> Result<TaskRecord, RuntimeError> {
        self.services
            .lock()
            .await
            .create_task(
                id.as_ref(),
                project_id.as_ref().map(ProjectId::as_str),
                &spec,
                "active",
                &Utc::now().to_rfc3339(),
            )
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn update_task(
        &self,
        id: &vak_domain::TaskId,
        spec: Option<serde_json::Value>,
        status: Option<String>,
    ) -> Result<TaskRecord, RuntimeError> {
        self.services
            .lock()
            .await
            .update_task(
                id.as_str(),
                None,
                spec.as_ref(),
                status.as_deref(),
                &Utc::now().to_rfc3339(),
            )
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn delete_task(&self, id: &vak_domain::TaskId) -> Result<bool, RuntimeError> {
        self.services
            .lock()
            .await
            .delete_task(id.as_str())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn approvals(&self) -> Result<Vec<ApprovalRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .pending_approvals()
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn resolve_approval(
        &self,
        id: &str,
        allow: bool,
        response: serde_json::Value,
    ) -> Result<ApprovalRecord, RuntimeError> {
        self.services
            .lock()
            .await
            .resolve_approval(id, allow, &response, &Utc::now().to_rfc3339())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn inbox(&self, unread: bool) -> Result<Vec<InboxRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .list_inbox(unread)
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn acknowledge_inbox(&self, id: &str) -> Result<bool, RuntimeError> {
        self.services
            .lock()
            .await
            .acknowledge_inbox(id, &Utc::now().to_rfc3339())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn memory(
        &self,
        project_id: Option<&ProjectId>,
        scope: &str,
    ) -> Result<Vec<vak_services::MemoryRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .list_memory(project_id.map(ProjectId::as_str), Some(scope))
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn create_memory(
        &self,
        id: String,
        mut project_id: Option<ProjectId>,
        scope: String,
        kind: String,
        tag: String,
        text: String,
    ) -> Result<MemoryRecord, RuntimeError> {
        if text.trim().is_empty() {
            return Err(RuntimeError::Execution(
                "memory text must not be empty".into(),
            ));
        }
        if scope != "workspace" && scope != "profile" {
            return Err(RuntimeError::Execution(format!(
                "unknown memory scope: {scope}"
            )));
        }
        if scope == "profile" {
            project_id = None;
        }
        self.services
            .lock()
            .await
            .create_memory(
                &id,
                project_id.as_ref().map(ProjectId::as_str),
                &scope,
                &kind,
                &tag,
                &text,
                &Utc::now().to_rfc3339(),
            )
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn amend_memory(&self, id: &str, text: &str) -> Result<MemoryRecord, RuntimeError> {
        if text.trim().is_empty() {
            return Err(RuntimeError::Execution(
                "memory text must not be empty".into(),
            ));
        }
        self.services
            .lock()
            .await
            .amend_memory(id, text, &Utc::now().to_rfc3339())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn forget_memory(&self, id: &str) -> Result<bool, RuntimeError> {
        self.services
            .lock()
            .await
            .forget_memory(id, &Utc::now().to_rfc3339())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn skills(
        &self,
        project_id: Option<&ProjectId>,
        status: Option<&str>,
    ) -> Result<Vec<SkillRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .list_skills(project_id.map(ProjectId::as_str), status)
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn promote_skill(&self, id: &str) -> Result<SkillRecord, RuntimeError> {
        self.services
            .lock()
            .await
            .promote_skill(id, &Utc::now().to_rfc3339())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn reject_skill(&self, id: &str) -> Result<SkillRecord, RuntimeError> {
        self.services
            .lock()
            .await
            .reject_skill(id, &Utc::now().to_rfc3339())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn create_checkpoint(
        &self,
        session_id: &SessionId,
        label: &str,
        manifest: &serde_json::Value,
    ) -> Result<CheckpointRecord, RuntimeError> {
        let id = vak_domain::TaskId::new();
        self.services
            .lock()
            .await
            .create_checkpoint(
                id.as_str(),
                session_id.as_str(),
                manifest,
                label,
                &Utc::now().to_rfc3339(),
            )
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn checkpoints(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<CheckpointRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .list_checkpoints(session_id.as_str())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn checkpoint_manifest(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<serde_json::Value>, RuntimeError> {
        let services = self.services.lock().await;
        let Some(record) = services
            .get_checkpoint(checkpoint_id)
            .map_err(|e| RuntimeError::Execution(e.to_string()))?
        else {
            return Ok(None);
        };
        let Some(bytes) = services
            .blobs
            .get(&record.manifest_digest)
            .map_err(|e| RuntimeError::Execution(e.to_string()))?
        else {
            return Err(RuntimeError::Execution(
                "checkpoint manifest blob is missing".into(),
            ));
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| RuntimeError::Execution(format!("checkpoint manifest: {e}")))
    }
    pub async fn restore_checkpoint(
        &self,
        session_id: &SessionId,
        checkpoint_id: &str,
    ) -> Result<serde_json::Value, RuntimeError> {
        let exists = self
            .checkpoints(session_id)
            .await?
            .into_iter()
            .any(|record| record.id == checkpoint_id);
        if !exists {
            return Err(RuntimeError::Execution(format!(
                "checkpoint not found: {checkpoint_id}"
            )));
        }
        self.checkpoint_manifest(checkpoint_id)
            .await?
            .ok_or_else(|| RuntimeError::Execution("checkpoint manifest blob is missing".into()))
    }

    pub async fn backup_export(
        &self,
        directory: impl Into<PathBuf>,
        include_secrets: bool,
    ) -> Result<BackupReport, RuntimeError> {
        backup_copy(&self.data_home, &directory.into(), include_secrets, false)
    }
    pub async fn backup_import(
        &self,
        directory: impl Into<PathBuf>,
        conflict: &str,
    ) -> Result<BackupReport, RuntimeError> {
        if conflict != "skip" && conflict != "rename" {
            return Err(RuntimeError::Execution(format!(
                "unknown backup conflict: {conflict}"
            )));
        }
        backup_copy(
            &directory.into(),
            &self.data_home,
            conflict == "rename",
            true,
        )
    }

    pub async fn flows(&self, project_id: &ProjectId) -> Result<Vec<FlowRecord>, RuntimeError> {
        let root = self
            .list_projects()
            .await?
            .into_iter()
            .find(|p| &p.id == project_id)
            .ok_or_else(|| RuntimeError::Execution(format!("project not found: {project_id}")))?;
        let dir = PathBuf::from(root.root).join(".vakcoder/flows");
        let mut result = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(result);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("toml") {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or_default()
                .to_owned();
            let text = std::fs::read_to_string(&path).map_err(StorageError::Io)?;
            let value = text.parse::<toml::Value>().ok();
            let nodes = value
                .as_ref()
                .and_then(|v| v.get("nodes"))
                .and_then(toml::Value::as_array)
                .map_or(0, Vec::len);
            result.push(FlowRecord {
                name,
                path,
                valid: value.is_some(),
                nodes,
            });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }
    pub async fn check_flow(
        &self,
        project_id: &ProjectId,
        name: &str,
    ) -> Result<FlowRecord, RuntimeError> {
        self.flows(project_id)
            .await?
            .into_iter()
            .find(|f| f.name == name)
            .ok_or_else(|| RuntimeError::Execution(format!("flow not found: {name}")))
    }
    pub async fn eval(&self, cases: &[serde_json::Value]) -> Result<(usize, usize), RuntimeError> {
        let failed = cases.iter().filter(|case| !case.is_object()).count();
        let passed = cases.len().saturating_sub(failed);
        self.services
            .lock()
            .await
            .audit(&vak_storage::AuditEvent {
                event: "eval.completed".to_owned(),
                timestamp: Utc::now().to_rfc3339(),
                actor: "runtime".to_owned(),
                data: serde_json::json!({"total": cases.len(), "passed": passed, "failed": failed}),
            })
            .map_err(|e| RuntimeError::Execution(e.to_string()))?;
        Ok((passed, failed))
    }
    pub async fn delivery(
        &self,
        id: &vak_domain::DeliveryId,
    ) -> Result<Option<DeliveryRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .get_delivery(id.as_str())
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn binding(
        &self,
        channel: &str,
        key: &str,
    ) -> Result<Option<BindingRecord>, RuntimeError> {
        self.services
            .lock()
            .await
            .get_binding(channel, key)
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
    pub async fn audit_events(&self) -> Result<Vec<vak_storage::AuditEvent>, RuntimeError> {
        self.services
            .lock()
            .await
            .query_audit()
            .map_err(|e| RuntimeError::Execution(e.to_string()))
    }
}

struct SupervisorGuard {
    supervisor: RunSupervisor,
}
impl vak_agent::CapabilityGuard for SupervisorGuard {
    fn check(&self, epoch: vak_domain::CapabilityEpoch) -> Result<(), vak_domain::DomainError> {
        self.supervisor.check_epoch(epoch)
    }
}

fn parse_status(value: &str) -> Result<vak_domain::RunStatus, RuntimeError> {
    let normalized = match value.to_ascii_lowercase().as_str() {
        "queued" => vak_domain::RunStatus::Queued,
        "admitted" => vak_domain::RunStatus::Admitted,
        "running" => vak_domain::RunStatus::Running,
        "waitingforapproval" | "waiting_for_approval" => vak_domain::RunStatus::WaitingForApproval,
        "cancelling" => vak_domain::RunStatus::Cancelling,
        "finishing" => vak_domain::RunStatus::Finishing,
        "completed" => vak_domain::RunStatus::Completed,
        "failed" | "interrupted" => vak_domain::RunStatus::Failed,
        "cancelled" => vak_domain::RunStatus::Cancelled,
        _ => {
            return Err(RuntimeError::Execution(format!(
                "invalid run status {value}"
            )));
        }
    };
    Ok(normalized)
}

fn backup_copy(
    source: &Path,
    destination: &Path,
    include_secrets_or_rename: bool,
    importing: bool,
) -> Result<BackupReport, RuntimeError> {
    let source = source.canonicalize().map_err(StorageError::Io)?;
    if source
        == destination
            .canonicalize()
            .unwrap_or_else(|_| destination.to_path_buf())
    {
        return Err(RuntimeError::Execution(
            "backup source and destination must differ".into(),
        ));
    }
    if !importing {
        if destination.starts_with(&source) {
            return Err(RuntimeError::Execution(
                "backup destination cannot be inside data home".into(),
            ));
        }
        std::fs::create_dir_all(destination).map_err(StorageError::Io)?;
    } else if !source.join("manifest.json").is_file() {
        return Err(RuntimeError::Execution(
            "backup manifest.json is missing".into(),
        ));
    }
    let mut report = BackupReport {
        path: destination.display().to_string(),
        file_count: 0,
        total_bytes: 0,
        renamed: 0,
        skipped: 0,
    };
    copy_tree(
        &source,
        destination,
        Path::new(""),
        include_secrets_or_rename,
        importing,
        &mut report,
    )?;
    if !importing {
        let manifest = serde_json::json!({
            "version": 1,
            "file_count": report.file_count,
            "total_bytes": report.total_bytes,
            "includes_secrets": include_secrets_or_rename,
        });
        std::fs::write(
            destination.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|e| RuntimeError::Execution(e.to_string()))?,
        )
        .map_err(StorageError::Io)?;
    }
    Ok(report)
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    relative: &Path,
    include_secrets_or_rename: bool,
    importing: bool,
    report: &mut BackupReport,
) -> Result<(), RuntimeError> {
    let current = source.join(relative);
    let entries = std::fs::read_dir(&current).map_err(StorageError::Io)?;
    for entry in entries {
        let entry = entry.map_err(StorageError::Io)?;
        let name = entry.file_name();
        let child_relative = relative.join(&name);
        if !importing
            && (child_relative == Path::new("runtime") || child_relative == Path::new(".env"))
        {
            if child_relative == Path::new(".env") && include_secrets_or_rename { /* included below */
            } else {
                continue;
            }
        }
        if importing && child_relative == Path::new("manifest.json") {
            continue;
        }
        let target = destination.join(&child_relative);
        let metadata = entry.metadata().map_err(StorageError::Io)?;
        if metadata.is_dir() {
            std::fs::create_dir_all(&target).map_err(StorageError::Io)?;
            copy_tree(
                source,
                destination,
                &child_relative,
                include_secrets_or_rename,
                importing,
                report,
            )?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let mut output = target.clone();
        if output.exists() {
            if !importing || !include_secrets_or_rename {
                report.skipped += 1;
                continue;
            }
            let mut suffix = 1u64;
            loop {
                let candidate = target.with_extension(format!("bak{suffix}"));
                if !candidate.exists() {
                    output = candidate;
                    break;
                }
                suffix = suffix.saturating_add(1);
            }
            report.renamed += 1;
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(StorageError::Io)?;
        }
        std::fs::copy(entry.path(), &output).map_err(StorageError::Io)?;
        report.file_count += 1;
        report.total_bytes += metadata.len();
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use vak_domain::{PermissionMode, RouteLeg, SandboxMode};

    #[test]
    fn provider_credentials_use_provider_specific_environment_keys() {
        assert_eq!(provider_env("anthropic"), "ANTHROPIC_API_KEY");
        assert_eq!(provider_env("google"), "GOOGLE_API_KEY");
        assert_eq!(provider_env("ollama"), "OLLAMA_API_KEY");
        assert_eq!(provider_env("opencode-zen"), "OPENCODE_API_KEY");
        assert_eq!(provider_env("openai"), "OPENAI_API_KEY");
    }

    fn contract() -> SessionContract {
        SessionContract {
            provider: "test".into(),
            model: "test-model".into(),
            route_ladder: vec![RouteLeg {
                provider: "test".into(),
                model: "test-model".into(),
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
    async fn runtime_persists_project_session_and_run() {
        let home = tempdir().expect("tempdir");
        let project_root = tempdir().expect("project root");
        let runtime = Runtime::open(home.path()).expect("runtime");
        let project_id = ProjectId::new();
        let session_id = SessionId::new();
        let run_id = RunId::new();
        runtime
            .register_project(ProjectContext {
                id: project_id.clone(),
                root: project_root.path().to_string_lossy().into_owned(),
                display_name: Some("test".into()),
            })
            .await
            .expect("project");
        runtime
            .create_session(session_id.clone(), project_id.clone(), contract())
            .await
            .expect("session");
        let handle = runtime
            .start_run(run_id, session_id, project_id)
            .await
            .expect("run");
        assert!(!handle.is_cancelled());
    }
}
