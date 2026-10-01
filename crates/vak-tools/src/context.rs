use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ToolContext {
    pub cwd: PathBuf,
    pub cancel: CancellationToken,
    pub sandbox: Option<Arc<dyn crate::sandbox::Sandbox>>,
    pub sandbox_sink: Option<crate::sandbox_events::SandboxEventSink>,
    pub agent_id: Option<String>,
    /// The run's trace key, when the call belongs to a real run. The Agent
    /// name above stays the scratch/authorship label; this is the identity
    /// that travels to the broker worker and onto sandbox records.
    pub trace: Option<vak_session::trace::TraceKey>,
    /// Workspace paths (with `/`) of Office files that are new to the
    /// workspace a task copy was made from, though the copy holds them:
    /// their Word edits are written clean (docs/design/72, R7). Set by the
    /// runtime, never by the model; empty outside a revision's copy.
    pub new_documents: Vec<String>,
}

impl ToolContext {
    pub fn new(cwd: PathBuf) -> Self {
        ToolContext {
            cwd,
            cancel: CancellationToken::new(),
            sandbox: None,
            sandbox_sink: None,
            agent_id: None,
            trace: None,
            new_documents: Vec::new(),
        }
    }

    pub fn with_sandbox_sink(mut self, sink: crate::sandbox_events::SandboxEventSink) -> Self {
        self.sandbox_sink = Some(sink);
        self
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    pub fn with_trace(mut self, trace: vak_session::trace::TraceKey) -> Self {
        self.trace = Some(trace);
        self
    }

    pub fn with_new_documents(mut self, paths: Vec<String>) -> Self {
        self.new_documents = paths;
        self
    }

    pub fn resolve(&self, p: &Path) -> PathBuf {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.cwd.join(p)
        }
    }
}

impl Default for ToolContext {
    fn default() -> Self {
        Self::new(PathBuf::new())
    }
}

pub fn shared_ctx(dir: &Path) -> Arc<ToolContext> {
    Arc::new(ToolContext::new(dir.to_path_buf()))
}
