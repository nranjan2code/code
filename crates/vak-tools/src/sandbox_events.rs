//! Structured sandbox execution events for live observation.
//!
//! Execution tools (such as bash) emit these events through a
//! sink threaded into `ToolContext`. The events flow through `AgentEvent` →
//! `EventBus` → SSE → the Workbench panel, giving the user full visibility
//! into what the sandbox/command execution is doing without having to trust the chat summary.

use serde::Serialize;
use tokio::sync::mpsc;

/// A single sandbox execution event. Serialized through `AgentEvent::Sandbox`
/// and delivered to the client via SSE.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum SandboxEvent {
    /// A sandbox execution is starting.
    ExecutionStarted {
        tool: String,
        /// The code or command about to execute (first ~2000 chars).
        code_preview: String,
        /// Language hint for syntax highlighting.
        language: String,
        /// The scratch directory being used.
        scratch_dir: String,
    },

    /// A chunk of stdout arrived from the running process.
    Stdout { chunk: String },

    /// A chunk of stderr arrived from the running process.
    Stderr { chunk: String },

    /// A package was installed in the sandbox environment.
    PackageInstalled { packages: Vec<String> },

    /// A file artifact was generated in the scratch directory.
    ArtifactGenerated {
        path: String,
        /// e.g. "image/png", "text/csv", "text/html"
        mime_type: String,
        size_bytes: u64,
    },

    /// The execution finished.
    ExecutionFinished {
        exit_code: i32,
        duration_ms: u64,
        /// Paths of all artifacts generated during this execution.
        artifacts: Vec<String>,
    },
}

/// Channel-based sink that sandbox tools write events into. The agent loop
/// reads the receiving end and forwards events as `AgentEvent::Sandbox`.
#[derive(Clone)]
pub struct SandboxEventSink {
    tx: mpsc::UnboundedSender<SandboxEvent>,
}

impl SandboxEventSink {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<SandboxEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (SandboxEventSink { tx }, rx)
    }

    /// Emit an event. Best-effort: dropped if the receiver is gone.
    pub fn emit(&self, event: SandboxEvent) {
        let _ = self.tx.send(event);
    }

    pub fn emit_execution_started(
        &self,
        tool: &str,
        code: &str,
        language: &str,
        scratch_dir: &str,
    ) {
        let preview = if code.len() > 2000 {
            format!("{}…", &code[..2000])
        } else {
            code.to_string()
        };
        self.emit(SandboxEvent::ExecutionStarted {
            tool: tool.to_string(),
            code_preview: preview,
            language: language.to_string(),
            scratch_dir: scratch_dir.to_string(),
        });
    }

    pub fn emit_stdout(&self, chunk: &str) {
        if !chunk.is_empty() {
            self.emit(SandboxEvent::Stdout {
                chunk: chunk.to_string(),
            });
        }
    }

    pub fn emit_stderr(&self, chunk: &str) {
        if !chunk.is_empty() {
            self.emit(SandboxEvent::Stderr {
                chunk: chunk.to_string(),
            });
        }
    }

    pub fn emit_packages_installed(&self, packages: &[String]) {
        if !packages.is_empty() {
            self.emit(SandboxEvent::PackageInstalled {
                packages: packages.to_vec(),
            });
        }
    }

    pub fn emit_artifact(&self, path: &str, mime_type: &str, size_bytes: u64) {
        self.emit(SandboxEvent::ArtifactGenerated {
            path: path.to_string(),
            mime_type: mime_type.to_string(),
            size_bytes,
        });
    }

    pub fn emit_finished(&self, exit_code: i32, duration_ms: u64, artifacts: Vec<String>) {
        self.emit(SandboxEvent::ExecutionFinished {
            exit_code,
            duration_ms,
            artifacts,
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn sink_emits_and_receiver_collects() {
        let (sink, mut rx) = SandboxEventSink::new();
        sink.emit_execution_started("bash", "echo 'hello'", "bash", "/workspace");
        sink.emit_stdout("hello\n");
        sink.emit_finished(0, 42, vec![]);

        let ev1 = rx.try_recv().unwrap();
        assert!(matches!(ev1, SandboxEvent::ExecutionStarted { .. }));
        let ev2 = rx.try_recv().unwrap();
        assert!(matches!(ev2, SandboxEvent::Stdout { .. }));
        let ev3 = rx.try_recv().unwrap();
        assert!(matches!(
            ev3,
            SandboxEvent::ExecutionFinished { exit_code: 0, .. }
        ));
    }

    #[test]
    fn code_preview_truncation() {
        let (sink, mut rx) = SandboxEventSink::new();
        let long_code = "x".repeat(3000);
        sink.emit_execution_started("bash", &long_code, "bash", "/workspace");

        if let SandboxEvent::ExecutionStarted { code_preview, .. } = rx.try_recv().unwrap() {
            assert!(code_preview.len() < 2100);
            assert!(code_preview.ends_with('…'));
        } else {
            panic!("expected ExecutionStarted");
        }
    }

    #[test]
    fn empty_stdout_not_emitted() {
        let (sink, mut rx) = SandboxEventSink::new();
        sink.emit_stdout("");
        assert!(rx.try_recv().is_err());
    }
}
