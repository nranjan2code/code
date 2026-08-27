//! Small adapter helpers for command-line surfaces.
//!
//! The adapter deliberately delegates every operation to [`Client`]; it has
//! no persistence and is safe to use from a CLI or an embedded UI.
use crate::types::{Diagnostics, Health, RunSnapshot, Session};
use crate::{Client, Result};
use vak_domain::ProjectContext;
use vak_domain::{ProjectId, RunId, SessionId};

#[derive(Clone, Debug)]
pub struct CliAdapter {
    client: Client,
}

impl CliAdapter {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
    pub async fn health(&self) -> Result<Health> {
        self.client.health().await
    }
    pub async fn projects(&self) -> Result<Vec<ProjectContext>> {
        self.client.projects().await
    }
    pub async fn sessions(&self, project: Option<&ProjectId>) -> Result<Vec<Session>> {
        self.client.sessions(project).await
    }
    pub async fn run_status(&self, run: &RunId) -> Result<RunSnapshot> {
        self.client.run(run).await
    }
    pub async fn session(&self, session: &SessionId) -> Result<Vec<Session>> {
        self.client
            .sessions(None)
            .await
            .map(|items| items.into_iter().filter(|s| &s.id == session).collect())
    }
    pub async fn diagnostics(&self) -> Result<Diagnostics> {
        self.client.diagnostics().await
    }
}
