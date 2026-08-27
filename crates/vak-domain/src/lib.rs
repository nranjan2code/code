//! Stable, transport-neutral contracts shared by vakcoder surfaces and runtime.
//!
//! This crate deliberately contains no I/O, process, filesystem, or policy
//! implementation. It is the vocabulary at the Runtime command boundary.

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7().to_string())
            }

            pub fn from_string(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                if Uuid::parse_str(&value).is_ok() {
                    Ok(Self(value))
                } else {
                    Err(DomainError::InvalidId {
                        kind: stringify!($name).to_string(),
                        value,
                    })
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = DomainError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::from_string(value)
            }
        }
    };
}

id_type!(ProjectId);
id_type!(SessionId);
id_type!(RunId);
id_type!(TaskId);
id_type!(ApprovalId);
id_type!(DeliveryId);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectContext {
    pub id: ProjectId,
    pub root: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionContract {
    pub provider: String,
    pub model: String,
    pub route_ladder: Vec<RouteLeg>,
    pub system_prompt: String,
    pub permission_mode: PermissionMode,
    pub sandbox: SandboxMode,
    pub tool_catalogue_revision: String,
    pub context_limit: u32,
    pub budget_ceiling: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteLeg {
    pub provider: String,
    pub model: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunContext {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub project: ProjectContext,
    pub contract: SessionContract,
    pub capability_epoch: CapabilityEpoch,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct CapabilityEpoch(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PermissionMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SandboxMode {
    Seatbelt,
    Landlock,
    Docker,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RunStatus {
    Queued,
    Admitted,
    Running,
    WaitingForApproval,
    Cancelling,
    Finishing,
    Completed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn transition(self, next: Self) -> Result<Self, DomainError> {
        let valid = match self {
            Self::Queued => matches!(next, Self::Admitted | Self::Cancelling | Self::Failed),
            Self::Admitted => matches!(next, Self::Running | Self::Cancelling | Self::Failed),
            Self::Running => matches!(
                next,
                Self::WaitingForApproval | Self::Cancelling | Self::Finishing | Self::Failed
            ),
            Self::WaitingForApproval => {
                matches!(next, Self::Running | Self::Cancelling | Self::Failed)
            }
            Self::Cancelling => matches!(next, Self::Finishing | Self::Cancelled | Self::Failed),
            Self::Finishing => matches!(next, Self::Completed | Self::Failed | Self::Cancelled),
            Self::Completed | Self::Failed | Self::Cancelled => false,
        };
        valid.then_some(next).ok_or(DomainError::InvalidTransition {
            from: self,
            to: next,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Command {
    RegisterProject {
        project: ProjectContext,
    },
    CreateSession {
        project_id: ProjectId,
        contract: SessionContract,
    },
    StartRun {
        session_id: SessionId,
        input: String,
    },
    SteerRun {
        run_id: RunId,
        input: String,
    },
    CancelRun {
        run_id: RunId,
        reason: Option<String>,
    },
    ChangePermissionMode {
        mode: PermissionMode,
    },
    ResolveApproval {
        approval_id: ApprovalId,
        decision: ApprovalDecision,
    },
    RestoreCheckpoint {
        session_id: SessionId,
        checkpoint_id: String,
    },
    UpdateConfig {
        revision: u64,
        patch: serde_json::Value,
    },
    SetProviderKey {
        provider: String,
        key_reference: String,
    },
    CreateTask {
        task_id: TaskId,
        definition: serde_json::Value,
    },
    RunTask {
        task_id: TaskId,
    },
    UpdateMemory {
        project_id: ProjectId,
        entry: String,
    },
    PromoteSkill {
        proposal_id: String,
    },
    DeliverMessage {
        delivery_id: DeliveryId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Query {
    ListProjects,
    ListSessions {
        project_id: Option<ProjectId>,
    },
    GetTranscript {
        session_id: SessionId,
    },
    GetRun {
        run_id: RunId,
    },
    Search {
        project_id: Option<ProjectId>,
        query: String,
    },
    GetConfig,
    ListApprovals,
    ListTasks,
    GetDiagnostics,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Event {
    ProjectRegistered {
        project: ProjectContext,
    },
    SessionCreated {
        session_id: SessionId,
        project_id: ProjectId,
    },
    RunStatusChanged {
        run_id: RunId,
        status: RunStatus,
    },
    RunOutput {
        run_id: RunId,
        delta: String,
        snapshot: String,
    },
    ApprovalRequested {
        approval_id: ApprovalId,
        run_id: RunId,
    },
    RunFinished {
        run_id: RunId,
        status: RunStatus,
        output: String,
    },
    ConfigChanged {
        revision: u64,
    },
    DeliveryUpdated {
        delivery_id: DeliveryId,
        status: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Response<T> {
    pub request_id: String,
    pub result: Result<T, DomainError>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DomainError {
    InvalidId {
        kind: String,
        value: String,
    },
    InvalidTransition {
        from: RunStatus,
        to: RunStatus,
    },
    NotFound {
        resource: String,
        id: String,
    },
    Conflict {
        resource: String,
        reason: String,
    },
    PermissionDenied {
        operation: String,
    },
    CapabilityRevoked {
        expected: CapabilityEpoch,
        actual: CapabilityEpoch,
    },
    InvalidRequest {
        message: String,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId { kind, .. } => write!(f, "invalid {kind}"),
            Self::InvalidTransition { from, to } => {
                write!(f, "invalid run transition: {from:?} -> {to:?}")
            }
            Self::NotFound { resource, id } => write!(f, "{resource} not found: {id}"),
            Self::Conflict { resource, reason } => write!(f, "{resource} conflict: {reason}"),
            Self::PermissionDenied { operation } => write!(f, "permission denied: {operation}"),
            Self::CapabilityRevoked { expected, actual } => {
                write!(f, "capability epoch revoked ({expected:?} != {actual:?})")
            }
            Self::InvalidRequest { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_uuid_strings_and_round_trip() {
        let first = ProjectId::new();
        let second = ProjectId::new();
        assert_ne!(first, second);
        let parsed = first.to_string().parse();
        assert_eq!(parsed, Ok(first.clone()));
        assert!(ProjectId::from_string("not-an-id").is_err());
    }

    #[test]
    fn terminal_runs_cannot_transition() {
        for terminal in [
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            assert!(terminal.transition(RunStatus::Running).is_err());
            assert!(terminal.is_terminal());
        }
    }

    #[test]
    fn normal_run_lifecycle_is_valid() {
        let status = RunStatus::Queued
            .transition(RunStatus::Admitted)
            .and_then(|s| s.transition(RunStatus::Running))
            .and_then(|s| s.transition(RunStatus::Finishing))
            .and_then(|s| s.transition(RunStatus::Completed));
        assert!(status.is_ok());
        let status = status.unwrap_or(RunStatus::Failed);
        assert_eq!(status, RunStatus::Completed);
    }
}
