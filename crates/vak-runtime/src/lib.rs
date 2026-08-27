//! The stateful supervision primitives shared by every vakcoder surface.
//!
//! This crate intentionally contains no HTTP, UI, provider, or filesystem
//! policy.  It owns only run lifetime, capability revocation, and the
//! project-scoped leases used to serialize workspace effects.

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use thiserror::Error;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
pub use vak_domain::{CapabilityEpoch, DomainError, ProjectId, RunId, RunStatus};

mod runtime;
pub use runtime::{
    ApprovalRecord, BackupReport, BindingRecord, CheckpointRecord, ConfigService, DeliveryRecord,
    FlowRecord, InboxRecord, MemoryRecord, Runtime, RuntimeError, SecretService, SkillRecord,
    TaskRecord,
};

/// Production dispatcher for the brokered built-in tools. The worker path is
/// explicit so a Runtime cannot accidentally execute model-supplied commands
/// in its own process.
pub struct BrokerToolDispatcher {
    tools: Vec<Arc<dyn vak_tools::Tool>>,
}

impl BrokerToolDispatcher {
    pub fn new(worker_executable: std::path::PathBuf) -> Self {
        Self {
            tools: vak_tools::brokered_default_tools(worker_executable),
        }
    }
}

#[async_trait::async_trait]
impl vak_agent::ToolDispatcher for BrokerToolDispatcher {
    fn definitions(&self) -> Vec<vak_llm::ToolDefinition> {
        vak_tools::definitions(&self.tools)
    }

    async fn dispatch(
        &self,
        context: &vak_domain::RunContext,
        call: &vak_agent::ToolCall,
        cancel: CancellationToken,
    ) -> Result<vak_agent::ToolResult, vak_agent::EngineError> {
        let read_only = matches!(call.name.as_str(), "read" | "glob" | "grep");
        let write_tool = matches!(call.name.as_str(), "write" | "edit");
        let allowed = match context.contract.permission_mode {
            vak_domain::PermissionMode::ReadOnly => read_only,
            vak_domain::PermissionMode::WorkspaceWrite => read_only || write_tool,
            vak_domain::PermissionMode::FullAccess => true,
        };
        if !allowed {
            return Ok(vak_agent::ToolResult {
                output: format!("tool '{}' is denied by the run permission mode", call.name),
                is_error: true,
            });
        }
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == call.name)
            .ok_or_else(|| vak_agent::EngineError::Tool {
                name: call.name.clone(),
                message: "unknown tool".to_owned(),
            })?;
        let sandbox: Option<Arc<dyn vak_tools::sandbox::Sandbox>> = match context.contract.sandbox {
            vak_domain::SandboxMode::None => None,
            vak_domain::SandboxMode::Seatbelt => Some(Arc::new(vak_tools::sandbox::Seatbelt::new(
                match context.contract.permission_mode {
                    vak_domain::PermissionMode::ReadOnly => {
                        vak_tools::sandbox::SandboxMode::ReadOnly
                    }
                    vak_domain::PermissionMode::WorkspaceWrite
                    | vak_domain::PermissionMode::FullAccess => {
                        vak_tools::sandbox::SandboxMode::WorkspaceWrite
                    }
                },
                std::path::Path::new(&context.project.root),
            ))),
            vak_domain::SandboxMode::Landlock => {
                #[cfg(target_os = "linux")]
                {
                    Some(Arc::new(vak_tools::landlock::Landlock::new(
                        match context.contract.permission_mode {
                            vak_domain::PermissionMode::ReadOnly => {
                                vak_tools::sandbox::SandboxMode::ReadOnly
                            }
                            vak_domain::PermissionMode::WorkspaceWrite
                            | vak_domain::PermissionMode::FullAccess => {
                                vak_tools::sandbox::SandboxMode::WorkspaceWrite
                            }
                        },
                        std::path::Path::new(&context.project.root),
                    )))
                }
                #[cfg(not(target_os = "linux"))]
                {
                    Some(Arc::new(vak_tools::sandbox::DenySandbox::new(
                        "landlock is unavailable on this platform",
                    )))
                }
            }
            vak_domain::SandboxMode::Docker => Some(Arc::new(
                vak_tools::sandbox::DenySandbox::new("docker backend is not configured"),
            )),
        };
        let ctx = vak_tools::ToolContext {
            cwd: std::path::PathBuf::from(&context.project.root),
            cancel,
            limits: vak_tools::OutputLimits::default(),
            sandbox,
        };
        let output = tool.execute(&call.arguments, &ctx).await;
        Ok(vak_agent::ToolResult {
            output: output.content,
            is_error: output.is_error,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub run_id: RunId,
    pub project_id: ProjectId,
    pub capability_epoch: CapabilityEpoch,
    pub status: RunStatus,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum RunError {
    #[error("run {0} already exists")]
    AlreadyExists(RunId),
    #[error("run {0} does not exist")]
    NotFound(RunId),
    #[error("run {0} is already terminal")]
    Terminal(RunId),
}

struct RunRecord {
    snapshot: RunSnapshot,
    cancel: CancellationToken,
}

/// Supervises all runs admitted by one Runtime instance.
#[derive(Clone, Default)]
pub struct RunSupervisor {
    inner: Arc<SupervisorInner>,
}

#[derive(Default)]
struct SupervisorInner {
    runs: Mutex<HashMap<RunId, RunRecord>>,
    epoch: std::sync::atomic::AtomicU64,
    changed: Notify,
}

impl RunSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current_epoch(&self) -> CapabilityEpoch {
        CapabilityEpoch(self.inner.epoch.load(std::sync::atomic::Ordering::Acquire))
    }

    /// Reject work admitted under a superseded capability epoch.
    pub fn check_epoch(&self, expected: CapabilityEpoch) -> Result<(), DomainError> {
        let actual = self.current_epoch();
        (actual == expected)
            .then_some(())
            .ok_or(DomainError::CapabilityRevoked { expected, actual })
    }

    /// Admit a run at the current capability epoch.
    pub async fn start(&self, run_id: RunId, project_id: ProjectId) -> Result<RunHandle, RunError> {
        let mut runs = self.inner.runs.lock().await;
        if runs.contains_key(&run_id) {
            return Err(RunError::AlreadyExists(run_id));
        }
        let cancel = CancellationToken::new();
        let snapshot = RunSnapshot {
            run_id: run_id.clone(),
            project_id,
            capability_epoch: self.current_epoch(),
            status: RunStatus::Running,
        };
        runs.insert(
            run_id.clone(),
            RunRecord {
                snapshot: snapshot.clone(),
                cancel: cancel.clone(),
            },
        );
        Ok(RunHandle {
            supervisor: self.clone(),
            snapshot,
            cancel,
        })
    }

    pub async fn snapshot(&self, run_id: &RunId) -> Option<RunSnapshot> {
        self.inner
            .runs
            .lock()
            .await
            .get(run_id)
            .map(|record| record.snapshot.clone())
    }

    pub async fn running_ids(&self) -> Vec<RunId> {
        self.inner
            .runs
            .lock()
            .await
            .values()
            .filter(|record| !record.snapshot.status.is_terminal())
            .map(|record| record.snapshot.run_id.clone())
            .collect()
    }

    pub async fn force_cancelled(&self, run_id: &RunId) -> Result<bool, RunError> {
        let mut runs = self.inner.runs.lock().await;
        let record = runs
            .get_mut(run_id)
            .ok_or_else(|| RunError::NotFound(run_id.clone()))?;
        if record.snapshot.status.is_terminal() {
            return Ok(false);
        }
        if record.snapshot.status != RunStatus::Cancelling {
            if record
                .snapshot
                .status
                .transition(RunStatus::Cancelling)
                .is_err()
            {
                return Ok(false);
            }
            record.snapshot.status = RunStatus::Cancelling;
        }
        if record
            .snapshot
            .status
            .transition(RunStatus::Cancelled)
            .is_err()
        {
            return Ok(false);
        }
        record.snapshot.status = RunStatus::Cancelled;
        record.cancel.cancel();
        self.inner.changed.notify_waiters();
        Ok(true)
    }

    /// Request cancellation.  An idle/nonexistent run is not implicitly
    /// created or poisoned; callers must have an admitted run id.
    pub async fn cancel(&self, run_id: &RunId) -> Result<bool, RunError> {
        let mut runs = self.inner.runs.lock().await;
        let record = runs
            .get_mut(run_id)
            .ok_or_else(|| RunError::NotFound(run_id.clone()))?;
        if record.snapshot.status.is_terminal() || record.snapshot.status == RunStatus::Cancelling {
            return Ok(false);
        }
        record.snapshot.status = RunStatus::Cancelling;
        record.cancel.cancel();
        self.inner.changed.notify_waiters();
        Ok(true)
    }

    /// Advance the capability epoch and cancel every run admitted under an
    /// older epoch.  The returned epoch is the only epoch accepted for new
    /// work; old handles can still finish their terminal transition safely.
    pub async fn revoke_capabilities(&self) -> CapabilityEpoch {
        let mut runs = self.inner.runs.lock().await;
        let epoch = CapabilityEpoch(
            self.inner
                .epoch
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
                + 1,
        );
        for record in runs.values_mut() {
            if record.snapshot.capability_epoch.0 < epoch.0 && !record.snapshot.status.is_terminal()
            {
                record.snapshot.status = RunStatus::Cancelling;
                record.cancel.cancel();
            }
        }
        self.inner.changed.notify_waiters();
        epoch
    }

    pub async fn wait_terminal(&self, run_id: &RunId) -> Result<RunSnapshot, RunError> {
        loop {
            if let Some(snapshot) = self.snapshot(run_id).await {
                if snapshot.status.is_terminal() {
                    return Ok(snapshot);
                }
            } else {
                return Err(RunError::NotFound(run_id.clone()));
            }
            self.inner.changed.notified().await;
        }
    }

    async fn finish(&self, run_id: &RunId, status: RunStatus) -> Result<bool, RunError> {
        if !status.is_terminal() {
            return Ok(false);
        }
        let mut runs = self.inner.runs.lock().await;
        let record = runs
            .get_mut(run_id)
            .ok_or_else(|| RunError::NotFound(run_id.clone()))?;
        if record.snapshot.status.is_terminal() {
            return Ok(false);
        }
        if record.snapshot.status.transition(status).is_err() {
            return Ok(false);
        }
        record.snapshot.status = status;
        self.inner.changed.notify_waiters();
        Ok(true)
    }

    async fn transition(&self, run_id: &RunId, status: RunStatus) -> Result<bool, RunError> {
        let mut runs = self.inner.runs.lock().await;
        let record = runs
            .get_mut(run_id)
            .ok_or_else(|| RunError::NotFound(run_id.clone()))?;
        if record.snapshot.status.is_terminal()
            || record.snapshot.status.transition(status).is_err()
        {
            return Ok(false);
        }
        record.snapshot.status = status;
        self.inner.changed.notify_waiters();
        Ok(true)
    }
}

pub struct RunHandle {
    supervisor: RunSupervisor,
    snapshot: RunSnapshot,
    cancel: CancellationToken,
}

impl Clone for RunHandle {
    fn clone(&self) -> Self {
        Self {
            supervisor: self.supervisor.clone(),
            snapshot: self.snapshot.clone(),
            cancel: self.cancel.clone(),
        }
    }
}

impl RunHandle {
    pub fn clone_for_task(&self) -> Self {
        self.clone()
    }
}

impl RunHandle {
    pub fn snapshot(&self) -> &RunSnapshot {
        &self.snapshot
    }

    pub fn run_id(&self) -> &RunId {
        &self.snapshot.run_id
    }

    pub fn capability_epoch(&self) -> CapabilityEpoch {
        self.snapshot.capability_epoch
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn check_capability(&self) -> Result<(), DomainError> {
        self.supervisor.check_epoch(self.snapshot.capability_epoch)
    }

    pub async fn complete(&self) -> Result<bool, RunError> {
        let _ = self
            .supervisor
            .transition(&self.snapshot.run_id, RunStatus::Finishing)
            .await?;
        self.supervisor
            .finish(&self.snapshot.run_id, RunStatus::Completed)
            .await
    }

    pub async fn begin_finishing(&self) -> Result<bool, RunError> {
        self.supervisor
            .transition(&self.snapshot.run_id, RunStatus::Finishing)
            .await
    }

    pub async fn fail(&self) -> Result<bool, RunError> {
        self.supervisor
            .finish(&self.snapshot.run_id, RunStatus::Failed)
            .await
    }

    pub async fn cancelled(&self) -> Result<bool, RunError> {
        self.supervisor
            .finish(&self.snapshot.run_id, RunStatus::Cancelled)
            .await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseKind {
    Read,
    Write,
    Exclusive,
}

#[derive(Debug, Error, Eq, PartialEq)]
#[error("workspace lease for project {0} was closed")]
pub struct LeaseClosed(pub ProjectId);

#[derive(Default)]
struct LeaseState {
    holders: HashMap<u64, (ProjectId, LeaseKind)>,
    next_id: u64,
}

/// Project-scoped lease manager. Reads may share a project; writes and
/// exclusive operations conflict with every other holder in that project.
#[derive(Clone, Default)]
pub struct WorkspaceLeaseManager {
    state: Arc<std::sync::Mutex<LeaseState>>,
    changed: Arc<Notify>,
}

impl WorkspaceLeaseManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(
        &self,
        project_id: impl Into<ProjectId>,
        kind: LeaseKind,
    ) -> WorkspaceLease {
        let project_id = project_id.into();
        loop {
            let lease = {
                let mut state = match self.state.lock() {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let available = state.holders.values().all(|(project, held)| {
                    project != &project_id || (kind == LeaseKind::Read && *held == LeaseKind::Read)
                });
                if available {
                    state.next_id += 1;
                    let id = state.next_id;
                    state.holders.insert(id, (project_id.clone(), kind));
                    Some(WorkspaceLease {
                        manager: self.clone(),
                        id,
                        project_id: project_id.clone(),
                        kind,
                    })
                } else {
                    None
                }
            };
            if let Some(lease) = lease {
                return lease;
            }
            self.changed.notified().await;
        }
    }
}

pub struct WorkspaceLease {
    manager: WorkspaceLeaseManager,
    id: u64,
    project_id: ProjectId,
    kind: LeaseKind,
}

impl WorkspaceLease {
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn kind(&self) -> LeaseKind {
        self.kind
    }
}

impl Drop for WorkspaceLease {
    fn drop(&mut self) {
        let removed = match self.manager.state.lock() {
            Ok(mut state) => state.holders.remove(&self.id).is_some(),
            Err(poisoned) => poisoned.into_inner().holders.remove(&self.id).is_some(),
        };
        if removed {
            self.manager.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tokio::time::{Duration, sleep, timeout};

    #[tokio::test]
    async fn idle_cancel_does_not_poison_next_run() {
        let supervisor = RunSupervisor::new();
        let idle = RunId::new();
        assert_eq!(
            supervisor.cancel(&idle).await,
            Err(RunError::NotFound(idle))
        );
        let run = supervisor
            .start(RunId::new(), ProjectId::new())
            .await
            .unwrap();
        assert!(!run.is_cancelled());
    }

    #[tokio::test]
    async fn terminal_transition_is_exactly_once() {
        let supervisor = RunSupervisor::new();
        let run_id = RunId::new();
        let run = supervisor
            .start(run_id.clone(), ProjectId::new())
            .await
            .unwrap();
        assert!(run.complete().await.unwrap());
        assert!(!run.cancelled().await.unwrap());
        assert_eq!(
            supervisor.snapshot(&run_id).await.unwrap().status,
            RunStatus::Completed
        );
    }

    #[tokio::test]
    async fn revocation_cancels_old_epoch_and_new_run_uses_new_epoch() {
        let supervisor = RunSupervisor::new();
        let old_id = RunId::new();
        let old = supervisor
            .start(old_id.clone(), ProjectId::new())
            .await
            .unwrap();
        let epoch = supervisor.revoke_capabilities().await;
        assert!(old.is_cancelled());
        assert_eq!(
            supervisor.snapshot(&old_id).await.unwrap().status,
            RunStatus::Cancelling
        );
        let new = supervisor
            .start(RunId::new(), ProjectId::new())
            .await
            .unwrap();
        assert_eq!(new.capability_epoch(), epoch);
        assert!(!new.is_cancelled());
    }

    #[tokio::test]
    async fn lease_conflicts_are_project_scoped() {
        let manager = WorkspaceLeaseManager::new();
        let project_a = ProjectId::new();
        let project_b = ProjectId::new();
        let read = manager.acquire(project_a.clone(), LeaseKind::Read).await;
        let other_project = timeout(
            Duration::from_millis(20),
            manager.acquire(project_b, LeaseKind::Write),
        )
        .await;
        assert!(other_project.is_ok());
        let blocked = timeout(
            Duration::from_millis(20),
            manager.acquire(project_a.clone(), LeaseKind::Write),
        )
        .await;
        assert!(blocked.is_err());
        drop(read);
        sleep(Duration::from_millis(1)).await;
        let write = timeout(
            Duration::from_millis(100),
            manager.acquire(project_a, LeaseKind::Write),
        )
        .await;
        assert!(write.is_ok());
    }
}
