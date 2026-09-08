use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::sandbox::SandboxTarget;
use crate::{Tool, ToolContext, ToolOutput};

pub const WORKER_SUBCOMMAND: &str = "__tool_worker";
pub(crate) const WORKER_ENV: &str = "VAK_INTERNAL_TOOL_WORKER";
const PROTOCOL_VERSION: u8 = 1;
const MAX_PROTOCOL_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct WorkerRequest {
    version: u8,
    tool: String,
    args: Value,
}

#[derive(Serialize, Deserialize)]
struct WorkerResponse {
    version: u8,
    content: String,
    is_error: bool,
}

pub struct BrokeredTool {
    inner: Arc<dyn Tool>,
    worker_exe: PathBuf,
}

impl BrokeredTool {
    pub fn new(inner: Arc<dyn Tool>, worker_exe: PathBuf) -> Self {
        Self { inner, worker_exe }
    }
}

#[async_trait]
impl Tool for BrokeredTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> Value {
        self.inner.schema()
    }

    fn claims(&self, args: &Value) -> crate::ResourceClaims {
        self.inner.claims(args)
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        execute(self.name(), args, &self.worker_exe, ctx).await
    }
}

async fn execute(tool: &str, args: &Value, worker_exe: &Path, ctx: &ToolContext) -> ToolOutput {
    if !worker_exe.is_file() {
        return ToolOutput::error(format!(
            "tool broker unavailable: worker executable not found: {}",
            worker_exe.display()
        ));
    }
    let worker_command = format!(
        "{} {}",
        shell_quote(&worker_exe.display().to_string()),
        WORKER_SUBCOMMAND
    );
    let mut request_args = args.clone();
    let effective = resolve_worker_command(
        ctx.sandbox.as_deref(),
        tool,
        &mut request_args,
        &worker_command,
    );
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(effective)
        .current_dir(&ctx.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::bash::scrub_environment(&mut cmd);
    cmd.env(WORKER_ENV, "1");
    crate::bash::isolate_process_group(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return ToolOutput::error(format!("tool broker spawn failed: {error}")),
    };
    let request = WorkerRequest {
        version: PROTOCOL_VERSION,
        tool: tool.to_string(),
        args: request_args,
    };
    let payload = match serde_json::to_vec(&request) {
        Ok(payload) => payload,
        Err(error) => return ToolOutput::error(format!("tool broker encode failed: {error}")),
    };
    let Some(mut stdin) = child.stdin.take() else {
        crate::bash::kill_process_group(&child.id());
        return ToolOutput::error("tool broker has no stdin");
    };
    if let Err(error) = stdin.write_all(&payload).await {
        crate::bash::kill_process_group(&child.id());
        let _ = child.wait().await;
        return ToolOutput::error(format!("tool broker request failed: {error}"));
    }
    drop(stdin);

    let pid = child.id();
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    let (result, cancelled) = tokio::select! {
        _ = ctx.cancel.cancelled() => {
            crate::bash::kill_process_group(&pid);
            (wait.await, true)
        }
        output = &mut wait => (output, false),
    };
    let output = match result {
        Ok(output) => output,
        Err(error) => return ToolOutput::error(format!("tool broker wait failed: {error}")),
    };
    if cancelled {
        return ToolOutput::error("tool broker cancelled");
    }
    if output.stdout.len() as u64 > MAX_PROTOCOL_BYTES
        || output.stderr.len() as u64 > MAX_PROTOCOL_BYTES
    {
        return ToolOutput::error("tool broker output exceeded protocol limit");
    }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return ToolOutput::error(format!(
            "tool broker exited with {}: {}",
            output.status.code().unwrap_or(-1),
            detail.trim()
        ));
    }
    let response: WorkerResponse = match serde_json::from_slice(&output.stdout) {
        Ok(response) => response,
        Err(error) => {
            let detail = String::from_utf8_lossy(&output.stderr);
            return ToolOutput::error(format!(
                "tool broker returned invalid protocol: {error}; stderr: {}",
                detail.trim()
            ));
        }
    };
    if response.version != PROTOCOL_VERSION {
        return ToolOutput::error(format!(
            "tool broker protocol mismatch: expected {PROTOCOL_VERSION}, got {}",
            response.version
        ));
    }
    ToolOutput {
        content: response.content,
        is_error: response.is_error,
    }
}

pub async fn worker_main() -> i32 {
    let mut bytes = Vec::new();
    if tokio::io::stdin()
        .take(MAX_PROTOCOL_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .is_err()
        || bytes.len() as u64 > MAX_PROTOCOL_BYTES
    {
        return 125;
    }
    let request: WorkerRequest = match serde_json::from_slice::<WorkerRequest>(&bytes) {
        Ok(request) if request.version == PROTOCOL_VERSION => request,
        _ => return 125,
    };
    let tool = crate::default_tools()
        .into_iter()
        .find(|candidate| candidate.name() == request.tool);
    let output = match tool {
        Some(tool) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            tool.execute(&request.args, &ToolContext::new(cwd)).await
        }
        None => ToolOutput::error(format!("worker does not expose tool '{}'", request.tool)),
    };
    let response = WorkerResponse {
        version: PROTOCOL_VERSION,
        content: output.content,
        is_error: output.is_error,
    };
    let payload = match serde_json::to_vec(&response) {
        Ok(payload) if payload.len() as u64 <= MAX_PROTOCOL_BYTES => payload,
        _ => return 125,
    };
    let mut stdout = tokio::io::stdout();
    if stdout.write_all(&payload).await.is_err() || stdout.flush().await.is_err() {
        return 125;
    }
    0
}

fn shell_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for character in value.chars() {
        if character == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(character);
        }
    }
    out.push('\'');
    out
}

/// Decides how a sandbox wrapper applies to a brokered tool invocation.
///
/// Two strategies:
/// - **`WorkerProcess` target** (Seatbelt/Landlock): the entire worker
///   process is wrapped, so the sandbox binary itself is sandboxed at exec.
/// - **`ToolCommand` target** (Docker): the worker runs on the host and only
///   the tool's own command is wrapped, so the model-controlled shell lands
///   inside the container.
///
/// Returns the effective shell command string and mutates `request_args`
/// in place when the command-level wrapping path is taken.
fn resolve_worker_command(
    sandbox: Option<&dyn crate::sandbox::Sandbox>,
    tool: &str,
    request_args: &mut serde_json::Value,
    worker_command: &str,
) -> String {
    match sandbox {
        Some(sb) if sb.target() == SandboxTarget::WorkerProcess => sb.wrap(worker_command),
        Some(sb) => {
            if tool == "bash"
                && let Some(command) = request_args
                    .get("command")
                    .and_then(serde_json::Value::as_str)
            {
                request_args["command"] = serde_json::Value::String(sb.wrap(command));
            }
            worker_command.to_string()
        }
        None => worker_command.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::sandbox::{Sandbox, SandboxMode, Seatbelt};
    use serde_json::json;

    #[test]
    fn no_sandbox_passes_worker_command_through() {
        let worker_cmd = "/path/to/vak __tool_worker";
        let mut args = json!({"command": "echo hi"});
        let effective = resolve_worker_command(None, "bash", &mut args, worker_cmd);
        assert_eq!(effective, worker_cmd);
        // args untouched
        assert_eq!(args["command"], json!("echo hi"));
    }

    #[test]
    fn worker_process_target_wraps_the_worker_executable() {
        let dir = std::env::temp_dir();
        let sb: Arc<dyn Sandbox> = Arc::new(Seatbelt::new(SandboxMode::ReadOnly, &dir));
        assert_eq!(sb.target(), SandboxTarget::WorkerProcess);
        let worker_cmd = "/vak __tool_worker";
        let mut args = json!({"command": "echo hi"});
        let effective = resolve_worker_command(Some(&*sb), "bash", &mut args, worker_cmd);
        // The worker executable itself — not the inner bash command — is
        // wrapped, so the whole broker runs under sandbox-exec.
        assert!(
            effective.starts_with("sandbox-exec -p "),
            "expected sandbox-exec wrapper, got: {effective}"
        );
        assert!(effective.contains("__tool_worker"));
        // The inner bash command must NOT be wrapped — it travels through
        // the worker protocol, not the shell wrapper.
        assert!(!effective.contains("echo hi"));
    }

    #[test]
    fn tool_command_target_wraps_bash_command_in_args() {
        // A Docker-style sandbox (ToolCommand target) must wrap the bash
        // command inside the worker request args — the host worker stays
        // a protocol adapter and only the model-controlled shell is sandboxed.
        let sb: Arc<dyn Sandbox> = Arc::new(DockerStub {
            target: SandboxTarget::ToolCommand,
            wrap_fn: |cmd| format!("docker-run-wrapped({})", cmd),
        });
        assert_eq!(sb.target(), SandboxTarget::ToolCommand);

        let worker_cmd = "/vak __tool_worker";

        // bash tool: command gets wrapped in the args
        let mut args = json!({"command": "echo from-bash"});
        let effective = resolve_worker_command(Some(&*sb), "bash", &mut args, worker_cmd);
        assert_eq!(
            effective, worker_cmd,
            "worker command passes through unwrapped"
        );
        assert_eq!(
            args["command"],
            json!("docker-run-wrapped(echo from-bash)"),
            "bash command must be wrapped in the request args"
        );

        // non-bash tool: args untouched, worker command passes through
        let mut args2 = json!({"path": "src/main.rs"});
        let effective2 = resolve_worker_command(Some(&*sb), "read", &mut args2, worker_cmd);
        assert_eq!(effective2, worker_cmd);
        assert_eq!(args2["path"], json!("src/main.rs"));
    }

    #[test]
    fn tool_command_target_does_not_wrap_bash_without_command_arg() {
        let sb: Arc<dyn Sandbox> = Arc::new(DockerStub {
            target: SandboxTarget::ToolCommand,
            wrap_fn: |cmd| format!("wrapped({})", cmd),
        });
        let worker_cmd = "/vak __tool_worker";
        let mut args = json!({});
        let effective = resolve_worker_command(Some(&*sb), "bash", &mut args, worker_cmd);
        assert_eq!(effective, worker_cmd);
        assert_eq!(
            args["command"],
            json!(Value::Null),
            "no command key to wrap"
        );
    }

    /// Minimal Sandbox stub that lets us test ToolCommand-target wrapping
    /// without a Docker daemon.
    struct DockerStub {
        target: SandboxTarget,
        wrap_fn: fn(&str) -> String,
    }

    impl Sandbox for DockerStub {
        fn name(&self) -> &str {
            "docker-stub"
        }
        fn wrap(&self, command: &str) -> String {
            (self.wrap_fn)(command)
        }
        fn target(&self) -> SandboxTarget {
            self.target
        }
    }
}
