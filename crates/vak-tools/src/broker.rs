use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::sandbox::SandboxTarget;
use crate::sandbox_events::SandboxEvent;
use crate::{Tool, ToolContext, ToolOutput};

pub const WORKER_SUBCOMMAND: &str = "__tool_worker";
pub const PERSISTENT_WORKER_SUBCOMMAND: &str = "__persistent_tool_worker";
pub(crate) const WORKER_ENV: &str = "VAK_INTERNAL_TOOL_WORKER";
const PROTOCOL_VERSION: u8 = 2;
const MAX_PROTOCOL_BYTES: u64 = 2 * 1024 * 1024;
/// Wall-clock bound on one verification worker. A hostile package that
/// pins the CPU fails its checks instead of holding a candidate open.
pub const VERIFY_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(Serialize, Deserialize)]
struct WorkerRequest {
    version: u8,
    task: WorkerTask,
}

/// What one worker process is asked to do. Verification is not a model
/// tool, so it is its own task rather than a reserved tool name.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WorkerTask {
    Tool {
        tool: String,
        args: Value,
        execution_id: String,
        /// The Agent the call runs for: scratch paths and tracked-change
        /// authorship depend on it.
        #[serde(default)]
        agent_id: Option<String>,
    },
    VerifyTargets {
        root: PathBuf,
        checks: Vec<vak_sandbox::TargetCheckPlan>,
    },
    OfficeDiff {
        before: Option<PathBuf>,
        after: PathBuf,
    },
}

#[derive(Serialize, Deserialize)]
struct WorkerResponse {
    version: u8,
    content: String,
    is_error: bool,
    events: Vec<SandboxEvent>,
}

#[derive(Serialize, Deserialize)]
struct PersistentWorkerRequest {
    version: u8,
    command: String,
    args: Vec<String>,
    environment: Vec<(String, String)>,
}

pub async fn spawn_persistent_worker(
    worker_exe: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    environment: &[(String, String)],
    sandbox: Option<&dyn crate::sandbox::Sandbox>,
) -> Result<tokio::process::Child, String> {
    if !worker_exe.is_file() {
        return Err(format!(
            "tool broker unavailable: worker executable not found: {}",
            worker_exe.display()
        ));
    }
    let worker_command = format!(
        "{} {}",
        shell_quote(&worker_exe.display().to_string()),
        PERSISTENT_WORKER_SUBCOMMAND
    );
    let mut request = PersistentWorkerRequest {
        version: PROTOCOL_VERSION,
        command: command.to_string(),
        args: args.to_vec(),
        environment: environment.to_vec(),
    };
    let effective = match sandbox {
        Some(value) if value.target() == SandboxTarget::WorkerProcess => {
            value.wrap(&worker_command)
        }
        Some(value) => {
            let invocation = std::iter::once(request.command.as_str())
                .chain(request.args.iter().map(String::as_str))
                .map(shell_quote)
                .collect::<Vec<_>>()
                .join(" ");
            request.command = "sh".into();
            request.args = vec!["-c".into(), value.wrap(&invocation)];
            worker_command
        }
        None => worker_command,
    };
    let mut process = tokio::process::Command::new("sh");
    process
        .arg("-c")
        .arg(effective)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::bash::scrub_environment(&mut process);
    process.env(WORKER_ENV, "1");
    crate::bash::isolate_process_group(&mut process);
    let mut child = process
        .spawn()
        .map_err(|error| format!("persistent tool broker spawn failed: {error}"))?;
    let payload = serde_json::to_vec(&request)
        .map_err(|error| format!("persistent tool broker encode failed: {error}"))?;
    let Some(mut stdin) = child.stdin.take() else {
        crate::bash::kill_process_group(&child.id());
        return Err("persistent tool broker has no stdin".into());
    };
    if let Err(error) = stdin.write_all(&payload).await {
        crate::bash::kill_process_group(&child.id());
        let _ = child.wait().await;
        return Err(format!("persistent tool broker request failed: {error}"));
    }
    drop(stdin);
    Ok(child)
}

pub async fn persistent_worker_main() -> i32 {
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
    let request = match serde_json::from_slice::<PersistentWorkerRequest>(&bytes) {
        Ok(request) if request.version == PROTOCOL_VERSION => request,
        _ => return 125,
    };
    let mut command = tokio::process::Command::new(&request.command);
    command
        .args(&request.args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    crate::bash::scrub_environment(&mut command);
    command.envs(request.environment);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("preview spawn failed: {error}");
            return 125;
        }
    };
    match child.wait().await {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .unwrap_or(125)
            .into(),
        Err(error) => {
            eprintln!("preview wait failed: {error}");
            125
        }
    }
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

    fn serves(&self) -> &'static [&'static str] {
        self.inner.serves()
    }

    fn always_loaded(&self) -> bool {
        self.inner.always_loaded()
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
        task: WorkerTask::Tool {
            tool: tool.to_string(),
            args: request_args,
            execution_id: ctx
                .sandbox_sink
                .as_ref()
                .map(|sink| sink.execution_id().to_string())
                .unwrap_or_else(|| "unidentified".into()),
            agent_id: ctx.agent_id.clone(),
        },
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

    let Some(mut stdout) = child.stdout.take() else {
        return ToolOutput::error("tool broker has no stdout");
    };
    let Some(mut stderr) = child.stderr.take() else {
        return ToolOutput::error("tool broker has no stderr");
    };
    let sink = ctx.sandbox_sink.clone();
    let partial = Arc::new(std::sync::Mutex::new(String::new()));
    let partial_reader = partial.clone();
    let stderr_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let mut line = Vec::new();
        while let Ok(n) = stderr.read_buf(&mut line).await {
            if n == 0 {
                break;
            }
            while let Some(pos) = line.iter().position(|b| *b == b'\n') {
                let frame: Vec<u8> = line.drain(..=pos).collect();
                if let Some(payload) = frame.strip_prefix(b"VAK_EVENT:")
                    && let Ok(event) = serde_json::from_slice::<SandboxEvent>(payload)
                {
                    if let Ok(mut output) = partial_reader.lock() {
                        match &event {
                            SandboxEvent::Stdout { chunk, .. } => {
                                output.push_str(chunk);
                            }
                            SandboxEvent::Stderr { chunk, .. } => {
                                output.push_str("[stderr]\n");
                                output.push_str(chunk);
                            }
                            _ => {}
                        }
                    }
                    if let Some(ref sink) = sink {
                        sink.emit(event);
                    }
                } else {
                    bytes.extend(frame);
                }
            }
        }
        bytes.extend(line);
        bytes
    });
    let stdout_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes).await;
        bytes
    });
    let pid = child.id();
    let (status, cancelled) = tokio::select! {
        _ = ctx.cancel.cancelled() => {
            crate::bash::kill_process_group(&pid);
            (child.wait().await, true)
        }
        output = child.wait() => (output, false),
    };
    let status = match status {
        Ok(status) => status,
        Err(error) => return ToolOutput::error(format!("tool broker wait failed: {error}")),
    };
    let stdout = stdout_reader.await.unwrap_or_default();
    let stderr = stderr_reader.await.unwrap_or_default();
    if cancelled {
        let mut content = String::from("tool broker cancelled");
        if let Ok(output) = partial.lock()
            && !output.is_empty()
        {
            content.push_str("\n\n[partial output]\n");
            content.push_str(&output);
        }
        let diagnostics = String::from_utf8_lossy(&stderr).trim().to_string();
        if !diagnostics.is_empty() {
            content.push_str("\n\n[diagnostics]\n");
            content.push_str(&diagnostics);
        }
        let _ = stdout;
        return ToolOutput::error(content);
    }
    if stdout.len() as u64 > MAX_PROTOCOL_BYTES || stderr.len() as u64 > MAX_PROTOCOL_BYTES {
        return ToolOutput::error("tool broker output exceeded protocol limit");
    }
    if !status.success() {
        let detail = String::from_utf8_lossy(&stderr);
        return ToolOutput::error(format!(
            "tool broker exited with {}: {}",
            status.code().unwrap_or(-1),
            detail.trim()
        ));
    }
    let response: WorkerResponse = match serde_json::from_slice(&stdout) {
        Ok(response) => response,
        Err(error) => {
            let detail = String::from_utf8_lossy(&stderr);
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
    if response.is_error {
        ToolOutput::error(response.content)
    } else {
        ToolOutput::ok(response.content)
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
    let (tool_name, args, execution_id, agent_id) = match request.task {
        WorkerTask::Tool {
            tool,
            args,
            execution_id,
            agent_id,
        } => (tool, args, execution_id, agent_id),
        WorkerTask::OfficeDiff { before, after } => {
            let (content, is_error) = match office_diff_in_worker(before.as_deref(), &after) {
                Ok(content) => (content, false),
                Err(error) => (error, true),
            };
            return write_response(WorkerResponse {
                version: PROTOCOL_VERSION,
                content,
                is_error,
                events: Vec::new(),
            })
            .await;
        }
        WorkerTask::VerifyTargets { root, checks } => {
            let results = vak_sandbox::default_target_verifiers().verify(&root, &checks);
            let content = serde_json::to_string(&results).unwrap_or_default();
            return write_response(WorkerResponse {
                version: PROTOCOL_VERSION,
                content,
                is_error: false,
                events: Vec::new(),
            })
            .await;
        }
    };
    let tool = crate::default_tools()
        .into_iter()
        .find(|candidate| candidate.name() == tool_name);
    let (output, events) = match tool {
        Some(tool) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let (sink, mut rx) = crate::sandbox_events::SandboxEventSink::new_with_id(execution_id);
            let mut ctx = ToolContext::new(cwd).with_sandbox_sink(sink);
            if let Some(agent_id) = agent_id {
                ctx = ctx.with_agent_id(agent_id);
            }
            let event_forwarder = tokio::spawn(async move {
                let mut stderr = tokio::io::stderr();
                while let Some(event) = rx.recv().await {
                    let Ok(mut frame) = serde_json::to_vec(&event) else {
                        continue;
                    };
                    let mut prefixed = b"VAK_EVENT:".to_vec();
                    prefixed.append(&mut frame);
                    prefixed.push(b'\n');
                    if stderr.write_all(&prefixed).await.is_err() {
                        break;
                    }
                    let _ = stderr.flush().await;
                }
            });
            let output = tool.execute(&args, &ctx).await;
            drop(ctx.sandbox_sink);
            let _ = event_forwarder.await;
            (output, Vec::new())
        }
        None => (
            ToolOutput::error(format!("worker does not expose tool '{tool_name}'")),
            Vec::new(),
        ),
    };
    write_response(WorkerResponse {
        version: PROTOCOL_VERSION,
        content: output.content,
        is_error: output.is_error,
        events,
    })
    .await
}

async fn write_response(response: WorkerResponse) -> i32 {
    let payload = match serde_json::to_vec(&response) {
        Ok(payload) if payload.len() as u64 <= MAX_PROTOCOL_BYTES => payload,
        Ok(payload) => {
            let refusal = WorkerResponse {
                version: PROTOCOL_VERSION,
                content: format!(
                    "the result is {} bytes, over the {} byte worker protocol limit; request a smaller range",
                    payload.len(),
                    MAX_PROTOCOL_BYTES
                ),
                is_error: true,
                events: Vec::new(),
            };
            match serde_json::to_vec(&refusal) {
                Ok(payload) => payload,
                Err(_) => return 125,
            }
        }
        Err(_) => return 125,
    };
    let mut stdout = tokio::io::stdout();
    if stdout.write_all(&payload).await.is_err() || stdout.flush().await.is_err() {
        return 125;
    }
    0
}

/// Runs the registered target verifiers over `checks` under `root` in a
/// worker process (docs/design/72-openxml-documents.md, F4). The worker runs
/// under a read-only, network-denied sandbox rooted at `root` whatever the
/// session's permission mode, and within [`VERIFY_DEADLINE`]. Anything that
/// stops the worker from answering fails every planned check; a check never
/// passes because verification could not run.
pub async fn verify_targets(
    worker_exe: &Path,
    root: &Path,
    checks: &[vak_sandbox::TargetCheckPlan],
) -> Vec<vak_sandbox::TargetCheckResult> {
    if checks.is_empty() {
        return Vec::new();
    }
    let failed = |reason: String| {
        checks
            .iter()
            .map(|check| vak_sandbox::TargetCheckResult {
                verifier: check.verifier.clone(),
                path: check.path.clone(),
                status: "failed".into(),
                evidence: format!("verification did not run: {reason}"),
            })
            .collect::<Vec<_>>()
    };
    let task = WorkerTask::VerifyTargets {
        root: root.to_path_buf(),
        checks: checks.to_vec(),
    };
    let content = match run_task(worker_exe, root, &[root], task).await {
        Ok(content) => content,
        Err(reason) => return failed(reason),
    };
    match serde_json::from_str::<Vec<vak_sandbox::TargetCheckResult>>(&content) {
        Ok(results) if results.len() == checks.len() => results,
        Ok(_) => failed("worker answered a different number of checks".into()),
        Err(error) => failed(format!("worker returned invalid results: {error}")),
    }
}

/// The semantic difference between an Office file as it is (`before`,
/// absent for a new file) and a draft (`after`), computed in a worker under
/// the same read-only sandbox and deadline as verification, because both
/// files are hostile input (invariant 39). Returns the diff as JSON.
pub async fn office_diff(
    worker_exe: &Path,
    before: Option<&Path>,
    after: &Path,
) -> Result<Value, String> {
    let Some(after_dir) = after.parent() else {
        return Err("the draft has no directory".into());
    };
    let mut roots = vec![after_dir];
    if let Some(before) = before.and_then(Path::parent) {
        roots.push(before);
    }
    let task = WorkerTask::OfficeDiff {
        before: before.map(Path::to_path_buf),
        after: after.to_path_buf(),
    };
    let content = run_task(worker_exe, after_dir, &roots, task).await?;
    serde_json::from_str(&content)
        .map_err(|error| format!("worker returned an invalid diff: {error}"))
}

fn office_diff_in_worker(before: Option<&Path>, after: &Path) -> Result<String, String> {
    let limits = vak_ooxml::Limits::default();
    let read = |path: &Path| -> Result<vak_ooxml::read::Document, String> {
        let file = std::fs::File::open(path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        vak_ooxml::read::read(file, limits)
            .map_err(|error| format!("{} could not be read: {error}", path.display()))
    };
    let current = before.map(read).transpose()?;
    let draft = read(after)?;
    let diff = vak_ooxml::diff::diff(current.as_ref(), &draft);
    serde_json::to_string(&serde_json::json!({
        "summary": diff.summary,
        "changes": diff.changes,
        "flags": draft.inspection.flags(),
    }))
    .map_err(|error| error.to_string())
}

/// Spawns one worker for `task` under a read-only, network-denied sandbox
/// that can read `roots`, and returns its answer's content. Every failure,
/// including the deadline, is an error; nothing is guessed.
async fn run_task(
    worker_exe: &Path,
    cwd: &Path,
    roots: &[&Path],
    task: WorkerTask,
) -> Result<String, String> {
    if !worker_exe.is_file() {
        return Err(format!(
            "worker executable not found: {}",
            worker_exe.display()
        ));
    }
    let request = WorkerRequest {
        version: PROTOCOL_VERSION,
        task,
    };
    let payload =
        serde_json::to_vec(&request).map_err(|error| format!("request encode failed: {error}"))?;
    let worker_command = format!(
        "{} {}",
        shell_quote(&worker_exe.display().to_string()),
        WORKER_SUBCOMMAND
    );
    let effective = match verification_sandbox(roots, worker_exe) {
        Some(sandbox) => sandbox.wrap(&worker_command),
        None => worker_command,
    };
    let mut command = tokio::process::Command::new("sh");
    command
        .arg("-c")
        .arg(effective)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::bash::scrub_environment(&mut command);
    command.env(WORKER_ENV, "1");
    crate::bash::isolate_process_group(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("worker spawn failed: {error}"))?;
    let pid = child.id();
    let Some(mut stdin) = child.stdin.take() else {
        crate::bash::kill_process_group(&pid);
        return Err("worker has no stdin".into());
    };
    if let Err(error) = stdin.write_all(&payload).await {
        crate::bash::kill_process_group(&pid);
        return Err(format!("worker request failed: {error}"));
    }
    drop(stdin);
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        crate::bash::kill_process_group(&pid);
        return Err("worker has no output pipes".into());
    };
    let stdout_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stdout
            .take(MAX_PROTOCOL_BYTES + 1)
            .read_to_end(&mut bytes)
            .await;
        bytes
    });
    let stderr_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.take(64 * 1024).read_to_end(&mut bytes).await;
        bytes
    });
    let status = match tokio::time::timeout(VERIFY_DEADLINE, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => return Err(format!("worker wait failed: {error}")),
        Err(_) => {
            crate::bash::kill_process_group(&pid);
            let _ = child.wait().await;
            return Err(format!(
                "worker exceeded the {}s deadline",
                VERIFY_DEADLINE.as_secs()
            ));
        }
    };
    let stdout = stdout_reader.await.unwrap_or_default();
    let stderr = stderr_reader.await.unwrap_or_default();
    if !status.success() {
        return Err(format!(
            "worker exited with {}: {}",
            status.code().unwrap_or(-1),
            String::from_utf8_lossy(&stderr).trim()
        ));
    }
    if stdout.len() as u64 > MAX_PROTOCOL_BYTES {
        return Err("worker output exceeded the protocol limit".into());
    }
    let response: WorkerResponse = serde_json::from_slice(&stdout)
        .map_err(|error| format!("worker returned invalid protocol: {error}"))?;
    if response.version != PROTOCOL_VERSION || response.is_error {
        return Err(format!("worker refused the request: {}", response.content));
    }
    Ok(response.content)
}

/// Read-only and network-denied, able to read `roots` and the worker
/// executable. `None` where no OS backend exists; the in-code bounds of
/// each reader then stand alone.
fn verification_sandbox(
    roots: &[&Path],
    worker_exe: &Path,
) -> Option<Arc<dyn crate::sandbox::Sandbox>> {
    let (first, rest) = roots.split_first()?;
    let mut extra: Vec<PathBuf> = rest
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .collect();
    extra.extend(worker_exe.parent().map(Path::to_path_buf));
    #[cfg(target_os = "macos")]
    {
        let mut sandbox =
            crate::sandbox::Seatbelt::task_copy(crate::sandbox::SandboxMode::ReadOnly, first);
        sandbox.read_paths.extend(extra);
        Some(Arc::new(sandbox))
    }
    #[cfg(target_os = "linux")]
    {
        let mut sandbox =
            crate::landlock::Landlock::task_copy(crate::sandbox::SandboxMode::ReadOnly, first);
        sandbox.read_paths.extend(extra);
        Some(Arc::new(sandbox))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (first, extra);
        None
    }
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
