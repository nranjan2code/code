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
pub(crate) const WORKER_ENV: &str = "VAKCODER_INTERNAL_TOOL_WORKER";
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
    let effective = match ctx.sandbox.as_ref() {
        Some(sandbox) if sandbox.target() == SandboxTarget::WorkerProcess => {
            sandbox.wrap(&worker_command)
        }
        Some(sandbox) => {
            if tool == "bash"
                && let Some(command) = request_args.get("command").and_then(Value::as_str)
            {
                request_args["command"] = Value::String(sandbox.wrap(command));
            }
            worker_command
        }
        None => worker_command,
    };
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
