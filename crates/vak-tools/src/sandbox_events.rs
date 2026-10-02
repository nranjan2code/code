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
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(tag = "kind")]
pub enum SandboxEvent {
    /// A sandbox execution is starting.
    ExecutionStarted {
        execution_id: String,
        /// Session that owns this execution. This is the stable rehydration
        /// key when a parent delegates work to a child session.
        owner_session_id: Option<String>,
        tool: String,
        /// The code or command about to execute (first ~2000 chars).
        code_preview: String,
        /// Language hint for syntax highlighting.
        language: String,
        /// The scratch directory being used.
        scratch_dir: String,
    },

    /// A chunk of stdout arrived from the running process.
    Stdout {
        execution_id: String,
        chunk: String,
    },

    /// A chunk of stderr arrived from the running process.
    Stderr {
        execution_id: String,
        chunk: String,
    },
    OutputTruncated {
        execution_id: String,
    },

    /// A package was installed in the sandbox environment.
    PackageInstalled {
        execution_id: String,
        packages: Vec<String>,
    },

    /// A file artifact was generated in the scratch directory.
    ArtifactGenerated {
        execution_id: String,
        path: String,
        /// e.g. "image/png", "text/csv", "text/html"
        mime_type: String,
        size_bytes: u64,
    },

    /// Periodic resource usage telemetry for long-running executions.
    ProcessTelemetry {
        execution_id: String,
        elapsed_ms: u64,
        cpu_percent: f32,
        memory_bytes: u64,
    },

    /// The execution finished.
    ExecutionFinished {
        execution_id: String,
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
    execution_id: String,
    owner_session_id: Option<String>,
    trace: Option<vak_session::trace::TraceKey>,
}

impl SandboxEventSink {
    pub fn new() -> (Self, mpsc::UnboundedReceiver<SandboxEvent>) {
        Self::new_with_id("unidentified".to_string())
    }

    pub fn new_with_id(execution_id: String) -> (Self, mpsc::UnboundedReceiver<SandboxEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            SandboxEventSink {
                tx,
                execution_id,
                owner_session_id: None,
                trace: None,
            },
            rx,
        )
    }

    pub fn with_owner_session(mut self, session_id: impl Into<String>) -> Self {
        self.owner_session_id = Some(session_id.into());
        self
    }

    /// The run this execution belongs to, when there is one.
    pub fn with_trace(mut self, trace: vak_session::trace::TraceKey) -> Self {
        self.trace = Some(trace);
        self
    }

    pub fn trace(&self) -> Option<&vak_session::trace::TraceKey> {
        self.trace.as_ref()
    }

    /// Emit an event. Best-effort: dropped if the receiver is gone.
    pub fn emit(&self, event: SandboxEvent) {
        let event = match event {
            SandboxEvent::ExecutionStarted {
                execution_id,
                owner_session_id,
                tool,
                code_preview,
                language,
                scratch_dir,
            } => SandboxEvent::ExecutionStarted {
                execution_id,
                owner_session_id: owner_session_id.or_else(|| self.owner_session_id.clone()),
                tool,
                code_preview,
                language,
                scratch_dir,
            },
            other => other,
        };
        let _ = self.tx.send(event);
    }

    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }

    /// Whether this tool call was admitted by the runtime and should receive
    /// the workspace control-file restrictions that model fixtures omit.
    pub fn is_runtime_call(&self) -> bool {
        self.owner_session_id.is_some()
    }

    pub fn emit_execution_started(
        &self,
        tool: &str,
        code: &str,
        language: &str,
        scratch_dir: &str,
    ) {
        let preview = if code.chars().count() > 2000 {
            let truncated: String = code.chars().take(2000).collect();
            format!("{truncated}…")
        } else {
            code.to_string()
        };
        self.emit(SandboxEvent::ExecutionStarted {
            execution_id: self.execution_id.clone(),
            owner_session_id: self.owner_session_id.clone(),
            tool: tool.to_string(),
            code_preview: preview,
            language: language.to_string(),
            scratch_dir: scratch_dir.to_string(),
        });
    }

    pub fn emit_stdout(&self, chunk: &str) {
        if !chunk.is_empty() {
            self.emit(SandboxEvent::Stdout {
                execution_id: self.execution_id.clone(),
                chunk: chunk.to_string(),
            });
        }
    }

    pub fn emit_stderr(&self, chunk: &str) {
        if !chunk.is_empty() {
            self.emit(SandboxEvent::Stderr {
                execution_id: self.execution_id.clone(),
                chunk: chunk.to_string(),
            });
        }
    }

    pub fn emit_output_truncated(&self) {
        self.emit(SandboxEvent::OutputTruncated {
            execution_id: self.execution_id.clone(),
        });
    }

    pub fn emit_packages_installed(&self, packages: &[String]) {
        if !packages.is_empty() {
            self.emit(SandboxEvent::PackageInstalled {
                execution_id: self.execution_id.clone(),
                packages: packages.to_vec(),
            });
        }
    }

    pub fn emit_artifact(&self, path: &str, mime_type: &str, size_bytes: u64) {
        self.emit(SandboxEvent::ArtifactGenerated {
            execution_id: self.execution_id.clone(),
            path: path.to_string(),
            mime_type: mime_type.to_string(),
            size_bytes,
        });
    }

    pub fn emit_telemetry(&self, elapsed_ms: u64, cpu_percent: f32, memory_bytes: u64) {
        self.emit(SandboxEvent::ProcessTelemetry {
            execution_id: self.execution_id.clone(),
            elapsed_ms,
            cpu_percent,
            memory_bytes,
        });
    }

    pub fn emit_finished(&self, exit_code: i32, duration_ms: u64, artifacts: Vec<String>) {
        self.emit(SandboxEvent::ExecutionFinished {
            execution_id: self.execution_id.clone(),
            exit_code,
            duration_ms,
            artifacts,
        });
    }
}

/// Folds carriage returns (`\r`) in terminal output so progress bars and
/// in-place line rewrites don't produce duplicate or chaotic lines.
pub fn fold_carriage_returns(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    let has_trailing_newline = text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::with_capacity(text.len());
    for (i, line) in lines.iter().enumerate() {
        if line.contains('\r') {
            if let Some(last) = line.split('\r').rfind(|s| !s.is_empty()) {
                out.push_str(last);
            }
        } else {
            out.push_str(line);
        }
        if i + 1 < lines.len() || has_trailing_newline {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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

        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev, SandboxEvent::ExecutionStarted { .. }));
        if let SandboxEvent::ExecutionStarted { code_preview, .. } = ev {
            assert!(code_preview.len() < 2100);
            assert!(code_preview.ends_with('…'));
        }
    }

    #[test]
    fn code_preview_truncation_is_utf8_safe() {
        let (sink, mut rx) = SandboxEventSink::new();
        let code = format!("{}€", "x".repeat(1999));
        sink.emit_execution_started("bash", &code, "bash", ".");
        let ev = rx.try_recv().unwrap();
        assert!(matches!(ev, SandboxEvent::ExecutionStarted { .. }));
    }

    #[test]
    fn execution_start_records_owner_session_for_tree_rehydration() {
        let (sink, mut rx) = SandboxEventSink::new_with_id("exec-child".into());
        let sink = sink.with_owner_session("session-child");
        sink.emit_execution_started("bash", "echo hi", "bash", "/scratch");
        let event = rx.try_recv().unwrap();
        match event {
            SandboxEvent::ExecutionStarted {
                owner_session_id, ..
            } => {
                assert_eq!(owner_session_id.as_deref(), Some("session-child"));
            }
            _ => panic!("expected execution start"),
        }
    }

    #[test]
    fn empty_stdout_not_emitted() {
        let (sink, mut rx) = SandboxEventSink::new();
        sink.emit_stdout("");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn telemetry_emitted_and_collected() {
        let (sink, mut rx) = SandboxEventSink::new();
        sink.emit_telemetry(1500, 12.5, 45_000_000);
        let ev = rx.try_recv().unwrap();
        assert!(matches!(
            ev,
            SandboxEvent::ProcessTelemetry {
                elapsed_ms: 1500,
                memory_bytes: 45_000_000,
                ..
            }
        ));
    }

    #[test]
    fn test_fold_carriage_returns() {
        let raw = "Downloading: 10%\rDownloading: 50%\rDownloading: 100%\nDone!\n";
        let folded = fold_carriage_returns(raw);
        assert_eq!(folded, "Downloading: 100%\nDone!\n");

        let no_cr = "hello\nworld\n";
        assert_eq!(fold_carriage_returns(no_cr), no_cr);
    }
}
