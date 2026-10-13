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
    /// The space's execution root, resolved by the host so a worker with a
    /// scrubbed environment finds the same one; `None` resolves it here.
    pub executions: Option<PathBuf>,
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
            executions: None,
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

    pub fn with_executions(mut self, root: PathBuf) -> Self {
        self.executions = Some(root);
        self
    }

    /// The space's execution root: where executions keep temp files,
    /// caches and drafts (`vak_config::scope::executions_root`).
    pub fn executions_root(&self) -> PathBuf {
        self.executions
            .clone()
            .unwrap_or_else(|| vak_config::scope::executions_root(&self.cwd))
    }

    /// The Agent this call runs for, `vak` when the session names none.
    pub fn agent(&self) -> &str {
        self.agent_id.as_deref().unwrap_or("vak")
    }

    pub fn with_new_documents(mut self, paths: Vec<String>) -> Self {
        self.new_documents = paths;
        self
    }

    /// What a failure on `p` adds when `p` is an absolute path outside the
    /// workspace: the fact a "no such file" alone does not state. A path
    /// written with a leading slash is taken as the machine's, not the
    /// workspace's, and the caller was told only that nothing is there.
    pub fn outside_note(&self, p: &Path) -> String {
        if p.is_absolute() && !within(p, &self.cwd) {
            outside_facts(&self.cwd)
        } else {
            String::new()
        }
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

/// `p` with its links resolved through its nearest existing ancestor, so
/// `/tmp/x` and `/private/tmp/x` are one place and a file that does not
/// exist yet still has an answer. The one resolver every workspace check
/// uses.
pub fn resolved(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = p;
    loop {
        if let Ok(real) = at.canonicalize() {
            return rest.iter().rev().fold(real, |path, part| path.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                at = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Whether `p` is inside `root`, both `resolved`.
pub fn within(p: &Path, root: &Path) -> bool {
    p.starts_with(root) || resolved(p).starts_with(resolved(root))
}

/// The refusal for `name`, a path outside the workspace `cwd`: the same
/// facts `ToolContext::outside_note` adds, so every tool names the
/// workspace one way.
pub fn outside_refusal(name: &str, cwd: &Path) -> String {
    format!("access denied: {name}{}", outside_facts(cwd))
}

fn outside_facts(cwd: &Path) -> String {
    format!(
        ". This path is outside the workspace, which is {}: a relative path names a file in the workspace, and a full path must start with the workspace's",
        cwd.display()
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod outside_tests {
    use super::*;

    /// A failure on a path written with a leading slash says the path is
    /// outside the workspace; a workspace path, relative or absolute, adds
    /// nothing. Found live 2026-10-13: told only "No such file" for
    /// `/notes.md`, a model went looking across the machine.
    #[test]
    fn a_failure_outside_the_workspace_says_so() {
        let ctx = ToolContext::new(PathBuf::from("/work/space"));
        assert!(
            ctx.outside_note(Path::new("/notes.md"))
                .contains("outside the workspace, which is /work/space:")
        );
        assert_eq!(ctx.outside_note(Path::new("notes.md")), "");
        assert_eq!(ctx.outside_note(Path::new("/work/space/notes.md")), "");
    }

    /// Found live 2026-10-13: the workspace was `/private/tmp/…` and a path
    /// written through the `/tmp` link was called outside it.
    #[cfg(unix)]
    #[test]
    fn a_path_through_a_link_to_the_workspace_is_inside() {
        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let ctx = ToolContext::new(real.canonicalize().unwrap());
        assert_eq!(ctx.outside_note(&link.join("missing/notes.md")), "");
        assert_ne!(ctx.outside_note(&base.path().join("notes.md")), "");
    }
}
