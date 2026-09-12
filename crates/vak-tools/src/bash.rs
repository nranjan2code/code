use std::process::Stdio;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::{Tool, ToolContext, ToolOutput};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_CAPTURE: usize = 1 << 20;
const MAX_SCAN_ENTRIES: usize = 50_000;

pub struct BashTool;

/// The command inventory is runtime-dependent and belongs in execution output,
/// not in a prompt assembled in the parent process. Advertising a host PATH
/// here was incorrect for containerized/seatbelt workers.
pub fn runtime_capability_summary() -> String {
    "\nSandbox runtime: use universal `bash` execution. Command availability is verified by the execution result; do not claim a command succeeded without its receipt.\n".to_string()
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute any command, program, or script in the execution sandbox (workspace and quarantined `.vak/scratch/`). Use this for anything and everything: run applications, execute code in any language, run shell pipelines, process data or media, install packages and tools, run tests, and debug processes. HTML/SVG/image files written to `.vak/scratch/` are automatically previewed live in the Workbench — do not start blocking foreground HTTP servers for static file preview. Real-time stdout/stderr streams to the Workbench panel."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command to execute"},
                "timeout_ms": {"type": "integer", "minimum": 1000, "description": "Timeout in milliseconds (default 120000)"},
                "cwd": {"type": "string", "description": "Working directory relative to workspace root (e.g. '.' for workspace root, '.vak/scratch' for scratch)"},
                "quarantine": {"type": "boolean", "description": "If true, execute in quarantined `.vak/scratch/` space. If false, execute in workspace root."}
            },
            "required": ["command"]
        })
    }

    fn claims(&self, _args: &Value) -> crate::ResourceClaims {
        crate::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: Vec::new(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(command) = args.get("command").and_then(|c| c.as_str()) else {
            return ToolOutput::error("missing required parameter: command");
        };
        if ctx.sandbox_sink.is_some() && references_control_file(command) {
            return ToolOutput::error(
                "sandbox denied access to workspace control files (.env and .vak/config.toml)",
            );
        }
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1000);

        let quarantine = args
            .get("quarantine")
            .and_then(|v| v.as_bool())
            .unwrap_or_else(|| {
                ctx.sandbox_sink
                    .as_ref()
                    .is_some_and(|s| s.is_quarantined())
            });

        let scratch_root = ctx.cwd.join(".vak").join("scratch");
        let (mut execution_dir, scratch_dir) = if quarantine {
            let exec_dir = ctx
                .sandbox_sink
                .as_ref()
                .map(|sink| scratch_root.join(sink.execution_id()))
                .unwrap_or_else(|| scratch_root.clone());
            let _ = std::fs::create_dir_all(&exec_dir);
            let dot_vak = exec_dir.join(".vak");
            let _ = std::fs::create_dir_all(&dot_vak);
            let dot_scratch = dot_vak.join("scratch");
            if !dot_scratch.exists() && !dot_scratch.is_symlink() {
                #[cfg(unix)]
                {
                    if std::os::unix::fs::symlink("..", &dot_scratch).is_err() {
                        let _ = std::fs::create_dir_all(&dot_scratch);
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = std::fs::create_dir_all(&dot_scratch);
                }
            }
            (exec_dir.clone(), exec_dir)
        } else {
            let _ = std::fs::create_dir_all(&scratch_root);
            (ctx.cwd.clone(), scratch_root)
        };

        if let Some(custom_cwd) = args.get("cwd").and_then(|v| v.as_str()) {
            if custom_cwd != "." && !custom_cwd.is_empty() {
                let workspace = match ctx.cwd.canonicalize() {
                    Ok(path) => path,
                    Err(_) => return ToolOutput::error("working directory is unavailable"),
                };
                let target = match workspace.join(custom_cwd).canonicalize() {
                    Ok(path) => path,
                    Err(_) => return ToolOutput::error("working directory does not exist"),
                };
                let allowed_root = if quarantine {
                    scratch_dir
                        .canonicalize()
                        .unwrap_or_else(|_| scratch_dir.clone())
                } else {
                    workspace.clone()
                };
                if !target.starts_with(&allowed_root) || !target.is_dir() {
                    return ToolOutput::error(
                        "working directory must remain inside the active execution root",
                    );
                }
                execution_dir = target;
            }
        }

        // Command-scoped backends do not inherit the host process directory.
        // Carry the validated directory into the command before wrapping it.
        let execution_command = if ctx.sandbox.as_ref().is_some_and(|sandbox| {
            sandbox.target() == vak_sandbox::backend::SandboxTarget::ToolCommand
        }) && execution_dir != ctx.cwd
        {
            format!(
                "cd {} && ({command})",
                shell_quote(&execution_dir.display().to_string())
            )
        } else {
            command.to_string()
        };
        let effective = match &ctx.sandbox {
            Some(sb) => sb.wrap(&execution_command),
            None => execution_command.clone(),
        };

        let mut cmd = shell_command(&effective);
        cmd.current_dir(&execution_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub_environment(&mut cmd);

        // Every execution gets its own process group, including broker workers.
        // Otherwise a shell child can outlive the timed-out worker and keep the
        // captured pipes open until the original command exits.
        isolate_process_group(&mut cmd);

        let before_scratch = collect_candidate_files(&scratch_dir);

        let start_instant = std::time::Instant::now();
        if let Some(ref sink) = ctx.sandbox_sink {
            sink.emit_execution_started(
                "bash",
                command,
                "bash",
                &scratch_dir.display().to_string(),
            );
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let duration_ms = start_instant.elapsed().as_millis() as u64;
                if let Some(ref sink) = ctx.sandbox_sink {
                    sink.emit_finished(-1, duration_ms, Vec::new());
                }
                return ToolOutput::error(format!("spawn failed: {e}"));
            }
        };

        let child_pid = child.id();
        let telemetry_cancel = tokio_util::sync::CancellationToken::new();
        let telemetry_token = telemetry_cancel.clone();
        let telemetry_sink = ctx.sandbox_sink.clone();
        let telemetry_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
            let t0 = std::time::Instant::now();
            loop {
                tokio::select! {
                    _ = telemetry_token.cancelled() => break,
                    _ = interval.tick() => {
                        if let Some(ref sink) = telemetry_sink {
                            let elapsed = t0.elapsed().as_millis() as u64;
                            if elapsed >= 400 {
                                let rss = probe_process_memory(child_pid);
                                sink.emit_telemetry(elapsed, 0.0, rss);
                            }
                        }
                    }
                }
            }
        });

        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let sink_out = ctx.sandbox_sink.clone();
        let sink_err = ctx.sandbox_sink.clone();
        let out_fut = tokio::spawn(async move {
            match stdout.as_mut() {
                Some(r) => read_capped_streaming(r, sink_out, false).await,
                None => String::new(),
            }
        });
        let err_fut = tokio::spawn(async move {
            match stderr.as_mut() {
                Some(r) => read_capped_streaming(r, sink_err, true).await,
                None => String::new(),
            }
        });

        let timeout = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms));
        let cancelled = ctx.cancel.cancelled();
        tokio::select! {
            _ = timeout => {
                telemetry_cancel.cancel();
                let _ = telemetry_handle.await;
                kill_process_group(&child.id());
                let _ = child.wait().await;
                let out = out_fut.await.unwrap_or_default();
                let err = err_fut.await.unwrap_or_default();
                let duration_ms = start_instant.elapsed().as_millis() as u64;
                let mut paths = Vec::new();
                if let Some(ref sink) = ctx.sandbox_sink {
                    let artifacts = scan_new_candidate_artifacts(&scratch_dir, &before_scratch, &ctx.cwd);
                    for (rel_path, mime, size) in artifacts {
                        sink.emit_artifact(&rel_path, &mime, size);
                        paths.push(rel_path);
                    }
                    sink.emit_finished(-1, duration_ms, paths.clone());
                }
                // When a command times out but produced artifacts (e.g. wrote
                // an HTML file then launched a blocking foreground server), the
                // deliverable succeeded — the timeout was caused by a dangling
                // process, not a failure to produce the result. Tell the model
                // what was created so it does not apologize and dump code into
                // chat text.
                let has_artifacts = !paths.is_empty();
                let mut reason = format!("command timed out after {timeout_ms}ms");
                if has_artifacts {
                    reason.push_str(&format!(
                        ". However, {} artifact(s) were successfully created before timeout: {}. \
                         These are rendered live in the Workbench preview.",
                        paths.len(),
                        paths.join(", ")
                    ));
                }
                return ToolOutput {
                    content: interrupted_output(&out, &err, &reason),
                    is_error: !has_artifacts,
                };
            }
            _ = cancelled => {
                telemetry_cancel.cancel();
                let _ = telemetry_handle.await;
                kill_process_group(&child.id());
                let _ = child.wait().await;
                let out = out_fut.await.unwrap_or_default();
                let err = err_fut.await.unwrap_or_default();
                let duration_ms = start_instant.elapsed().as_millis() as u64;
                if let Some(ref sink) = ctx.sandbox_sink {
                    let artifacts = scan_new_candidate_artifacts(&scratch_dir, &before_scratch, &ctx.cwd);
                    let mut paths = Vec::new();
                    for (rel_path, mime, size) in artifacts {
                        sink.emit_artifact(&rel_path, &mime, size);
                        paths.push(rel_path);
                    }
                    sink.emit_finished(-1, duration_ms, paths);
                }
                return ToolOutput { content: interrupted_output(&out, &err, "command cancelled"), is_error: true };
            }
            status = child.wait() => {
                telemetry_cancel.cancel();
                let _ = telemetry_handle.await;
                let status = match status {
                    Ok(s) => s,
                    Err(e) => {
                        let duration_ms = start_instant.elapsed().as_millis() as u64;
                        if let Some(ref sink) = ctx.sandbox_sink {
                            sink.emit_finished(-1, duration_ms, Vec::new());
                        }
                        return ToolOutput::error(format!("wait failed: {e}"));
                    }
                };
                let out = out_fut.await.unwrap_or_default();
                let err = err_fut.await.unwrap_or_default();
                let duration_ms = start_instant.elapsed().as_millis() as u64;

                let new_artifacts = scan_new_candidate_artifacts(&scratch_dir, &before_scratch, &ctx.cwd);
                let mut artifact_paths = Vec::new();
                if let Some(ref sink) = ctx.sandbox_sink {
                    for (rel_path, mime, size) in &new_artifacts {
                        sink.emit_artifact(rel_path, mime, *size);
                        artifact_paths.push(rel_path.clone());
                    }
                    if status.success()
                        && let Some(packages) = detect_installed_packages(command)
                    {
                        sink.emit_packages_installed(&packages);
                    }
                    sink.emit_finished(status.code().unwrap_or(-1), duration_ms, artifact_paths);
                }

                let mut text = String::new();
                if !out.is_empty() {
                    text.push_str("[stdout]\n");
                    text.push_str(&out);
                    text.push('\n');
                }
                if !err.is_empty() {
                    text.push_str("[stderr]\n");
                    text.push_str(&err);
                    text.push('\n');
                }
                if !status.success() {
                    text.push_str(&format!("\n[exit code: {}]", status.code().unwrap_or(-1)));
                    return ToolOutput {
                        content: ctx.truncate_output(text),
                        is_error: true,
                    };
                }
                if text.is_empty() {
                    text.push_str("(no output)");
                }
                ToolOutput::ok(ctx.truncate_output(text))
            }
        }
    }
}

/// The minimal operational environment forwarded to sandboxed subprocesses.
/// Provider, gateway, and connector credentials never appear here — only the
/// paths a working toolchain needs.
const ALLOWED_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "TERM",
    "SHELL",
    "TMPDIR",
    "CARGO_HOME",
    "RUSTUP_HOME",
];

/// Predicate for the env-scrub allowlist. Extracted so the security boundary
/// can be unit-tested without mutating process-global state.
pub(crate) fn is_allowed_env_var(key: &str) -> bool {
    ALLOWED_ENV_VARS.contains(&key) || key.starts_with("LC_") || key.starts_with("XDG_")
}

pub(crate) fn scrub_environment(cmd: &mut tokio::process::Command) {
    let inherited: Vec<(String, std::ffi::OsString)> = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            if is_allowed_env_var(&key) {
                Some((key, value))
            } else {
                None
            }
        })
        .collect();
    cmd.env_clear();
    cmd.envs(inherited);
    // GUI-launched macOS applications do not receive the shell PATH. Keep the
    // sandbox language/toolchain-neutral, but make the normal user-installed
    // tool locations reachable by every command executed through `bash`.
    // This is deliberately a PATH repair, not a Python/Node/etc. special case.
    #[cfg(target_os = "macos")]
    {
        let mut path = std::env::var_os("PATH").unwrap_or_default();
        for candidate in [
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/usr/local/sbin",
            "/opt/local/bin",
        ] {
            let candidate = std::path::Path::new(candidate);
            if candidate.is_dir() {
                let mut repaired = candidate.as_os_str().to_os_string();
                repaired.push(":");
                repaired.push(&path);
                path = repaired;
            }
        }
        cmd.env("PATH", path);
    }
}

pub fn isolate_process_group(cmd: &mut tokio::process::Command) {
    #[cfg(unix)]
    #[allow(unsafe_code)]
    unsafe {
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }
}

fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(unix)]
    {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    }
    #[cfg(windows)]
    {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn kill_process_group(pid: &Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        #[allow(unsafe_code)]
        unsafe {
            libc::kill(-(*pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

async fn read_capped_streaming<R: AsyncReadExt + Unpin>(
    r: &mut R,
    sink: Option<crate::sandbox_events::SandboxEventSink>,
    is_stderr: bool,
) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let space = MAX_CAPTURE.saturating_sub(buf.len());
                let taken = n.min(space);
                buf.extend_from_slice(&chunk[..taken]);
                // Keep draining the pipe after the retained-output cap. A
                // child must never block because the UI chose bounded memory.
                // Do not continue forwarding unbounded data to the browser.
                if taken < n && !truncated {
                    truncated = true;
                    if let Some(ref sink) = sink {
                        sink.emit_output_truncated();
                    }
                }
                if taken > 0 && !truncated {
                    let chunk_str = String::from_utf8_lossy(&chunk[..n]);
                    if let Some(ref sink) = sink {
                        if is_stderr {
                            sink.emit_stderr(&chunk_str);
                        } else {
                            sink.emit_stdout(&chunk_str);
                        }
                    }
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn probe_process_memory(pid: Option<u32>) -> u64 {
    let Some(pid) = pid else { return 0 };
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string(format!("/proc/{pid}/statm")) {
            let mut parts = statm.split_whitespace();
            if let Some(pages_str) = parts.nth(1) {
                if let Ok(pages) = pages_str.parse::<u64>() {
                    return pages * 4096;
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            && out.status.success()
        {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Ok(kb) = text.parse::<u64>() {
                return kb * 1024;
            }
        }
    }
    0
}

fn collect_candidate_files(
    scratch_dir: &std::path::Path,
) -> std::collections::HashMap<std::path::PathBuf, std::time::SystemTime> {
    let mut map = std::collections::HashMap::new();
    fn scan(
        dir: &std::path::Path,
        map: &mut std::collections::HashMap<std::path::PathBuf, std::time::SystemTime>,
    ) {
        if !dir.exists() {
            return;
        }
        for entry in walkdir::WalkDir::new(dir)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .take(MAX_SCAN_ENTRIES)
        {
            let path = entry.path().to_path_buf();
            if path.is_file()
                && let Ok(meta) = path.metadata()
            {
                let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                map.insert(path, mtime);
            }
        }
    }
    scan(scratch_dir, &mut map);
    map
}

fn scan_new_candidate_artifacts(
    scratch_dir: &std::path::Path,
    before: &std::collections::HashMap<std::path::PathBuf, std::time::SystemTime>,
    cwd: &std::path::Path,
) -> Vec<(String, String, u64)> {
    let mut artifacts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut check_entry = |path: std::path::PathBuf| {
        if path.is_file()
            && seen.insert(path.clone())
            && let Ok(meta) = path.metadata()
        {
            let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let is_new = match before.get(&path) {
                None => true,
                Some(&prev) => mtime > prev,
            };
            if is_new {
                let rel = path
                    .strip_prefix(cwd)
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                let mime = guess_mime_type(&path);
                artifacts.push((rel, mime, meta.len()));
            }
        }
    };

    if scratch_dir.exists() {
        for entry in walkdir::WalkDir::new(scratch_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .take(MAX_SCAN_ENTRIES)
        {
            check_entry(entry.path().to_path_buf());
        }
    }
    artifacts
}

fn guess_mime_type(path: &std::path::Path) -> String {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "pdf" => "application/pdf",
        "zip" | "tar" | "gz" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn interrupted_output(stdout: &str, stderr: &str, reason: &str) -> String {
    let mut text = String::new();
    if !stdout.is_empty() {
        text.push_str("[stdout]\n");
        text.push_str(stdout);
        text.push('\n');
    }
    if !stderr.is_empty() {
        text.push_str("[stderr]\n");
        text.push_str(stderr);
        text.push('\n');
    }
    text.push_str("\n[");
    text.push_str(reason);
    text.push(']');
    text
}

fn references_control_file(command: &str) -> bool {
    [".env", ".vak/config.toml", ".vak/config"]
        .iter()
        .any(|needle| command.contains(needle))
}

fn detect_installed_packages(cmd: &str) -> Option<Vec<String>> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.len() >= 3 && ((parts[0] == "pip" || parts[0] == "pip3") && parts[1] == "install") {
        let pkgs: Vec<String> = parts[2..]
            .iter()
            .filter(|p| !p.starts_with('-'))
            .map(|p| p.to_string())
            .collect();
        if !pkgs.is_empty() {
            return Some(pkgs);
        }
    } else if parts.len() >= 3
        && (parts[0] == "npm" && (parts[1] == "install" || parts[1] == "i" || parts[1] == "add"))
    {
        let pkgs: Vec<String> = parts[2..]
            .iter()
            .filter(|p| !p.starts_with('-'))
            .map(|p| p.to_string())
            .collect();
        if !pkgs.is_empty() {
            return Some(pkgs);
        }
    } else if parts.len() >= 3 && (parts[0] == "cargo" && parts[1] == "add") {
        let pkgs: Vec<String> = parts[2..]
            .iter()
            .filter(|p| !p.starts_with('-'))
            .map(|p| p.to_string())
            .collect();
        if !pkgs.is_empty() {
            return Some(pkgs);
        }
    } else if parts.len() >= 3 && (parts[0] == "uv" && (parts[1] == "add" || parts[1] == "pip")) {
        let pkgs: Vec<String> = parts[2..]
            .iter()
            .filter(|p| !p.starts_with('-') && **p != "install")
            .map(|p| p.to_string())
            .collect();
        if !pkgs.is_empty() {
            return Some(pkgs);
        }
    } else if parts.len() >= 3 && (parts[0] == "pnpm" || parts[0] == "yarn") && (parts[1] == "add")
    {
        let pkgs: Vec<String> = parts[2..]
            .iter()
            .filter(|p| !p.starts_with('-'))
            .map(|p| p.to_string())
            .collect();
        if !pkgs.is_empty() {
            return Some(pkgs);
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{is_allowed_env_var, scrub_environment};
    use std::sync::Mutex;

    /// Serialises std::env mutation across every env-scrubbing test in the
    /// process. `set_var` is process-global and `cargo test` runs in parallel.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn is_allowed_env_var_rejects_common_secret_names() {
        // Provider / cloud / local-dev credentials that must NEVER reach a
        // sandboxed subprocess. Each of these is a real environment variable
        // naming convention an operator or CI system commonly sets.
        let secrets = [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "GITHUB_TOKEN",
            "GITHUB_PAT",
            "GITLAB_TOKEN",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "DATABASE_URL",
            "DB_PASSWORD",
            "VAULT_TOKEN",
            "VAULT_ADDR",
            "DOCKER_TOKEN",
            "NPM_TOKEN",
            "PYPI_TOKEN",
            "CLOUDSDK_AUTH_ACCESS_TOKEN",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "AZURE_CLIENT_SECRET",
            "TAVILY_API_KEY",
            "SLACK_BOT_TOKEN",
            "TELEGRAM_BOT_TOKEN",
            "DISCORD_TOKEN",
            "SECRET_KEY",
            "PRIVATE_KEY",
            "SSH_AUTH_SOCK",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "REQUESTS_CA_BUNDLE",
        ];
        for secret in secrets {
            assert!(
                !is_allowed_env_var(secret),
                "`{secret}` must not pass the env allowlist"
            );
        }
    }

    #[test]
    fn is_allowed_env_var_accepts_operational_vars() {
        for ok in [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "LANG",
            "LC_ALL",
            "LC_MONETARY",
            "LC_TIME",
            "LC_COLLATE",
            "TERM",
            "SHELL",
            "TMPDIR",
            "CARGO_HOME",
            "RUSTUP_HOME",
            "XDG_CACHE_HOME",
            "XDG_CONFIG_HOME",
            "XDG_RUNTIME_DIR",
        ] {
            assert!(is_allowed_env_var(ok), "`{ok}` must pass the env allowlist");
        }
    }

    #[test]
    fn is_allowed_env_var_prefix_matching_is_exact() {
        // Substring prefixes must NOT match — only the exact listed names or
        // LC_*/XDG_* prefixes pass. A var like "MY_PATH" must not masquerade
        // as "PATH".
        assert!(!is_allowed_env_var("MY_PATH"));
        assert!(!is_allowed_env_var("PATH_EXTRA"));
        assert!(!is_allowed_env_var("CARGO_HOME_DIR"));
        assert!(!is_allowed_env_var("LD_PRELOAD"));
        assert!(!is_allowed_env_var("LD_LIBRARY_PATH"));
        assert!(!is_allowed_env_var("PYTHONPATH"));
        assert!(!is_allowed_env_var("NODE_OPTIONS"));
        // LC_ prefix is allowed; "LC" alone is not.
        assert!(is_allowed_env_var("LC_FOO"));
        assert!(!is_allowed_env_var("LC"));
        // XDG_ prefix is allowed; "XD" is not.
        assert!(is_allowed_env_var("XDG_DATA_DIRS"));
        assert!(!is_allowed_env_var("XDG"));
    }

    #[test]
    fn scrub_environment_only_carries_allowlist_into_child() {
        // Observational test: after scrubbing, the command's env must be a
        // strict subset of the process env filtered through the allowlist.
        // This validates the real filtering pipeline without mutating any
        // process-global state.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut cmd = tokio::process::Command::new("sh");
        cmd.env("VAK_TEST_SECRET", "must-not-survive");
        cmd.env("PATH", "/bin:/usr/bin");
        scrub_environment(&mut cmd);
        let scrubbed: std::collections::HashSet<String> = cmd
            .as_std()
            .get_envs()
            .filter(|(_, v)| v.is_some())
            .filter_map(|(k, _)| k.to_str().map(str::to_string))
            .collect();
        // No var that fails the predicate may be present.
        for key in &scrubbed {
            assert!(
                is_allowed_env_var(key),
                "`{key}` survived scrubbing but is not on the allowlist"
            );
        }
        // A var we set on the command object before scrubbing must be gone.
        assert!(
            !scrubbed.contains("VAK_TEST_SECRET"),
            "command-level env must be cleared by scrub"
        );
    }

    #[test]
    fn scrub_environment_drops_process_secrets_at_runtime() {
        // End-to-end security boundary: a secret in THIS process's environment
        // must not reach the sandboxed child. We set a representative set of
        // credential variable names, scrub, and verify each is absent from the
        // command's env as seen from the child.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for secret in [
            "VAK_SCRUB_TEST_SECRET",
            "VAK_SCRUB_TEST_API_KEY",
            "VAK_SCRUB_TEST_TOKEN",
        ] {
            // SAFETY: test-only mutation of process-global env, serialised by
            // ENV_LOCK and cleaned up in the same scope. No production code
            // is affected. This is the narrow, annotated unsafe exception
            // pattern already used in this module for process-group kill.
            #[allow(unsafe_code)]
            unsafe {
                std::env::set_var(secret, "super-secret-value");
            }
        }
        let cmd = {
            let mut c = tokio::process::Command::new("sh");
            scrub_environment(&mut c);
            c
        };
        let keys: Vec<String> = cmd
            .as_std()
            .get_envs()
            .filter(|(_, v)| v.is_some())
            .filter_map(|(k, _)| k.to_str().map(str::to_string))
            .collect();
        for secret in [
            "VAK_SCRUB_TEST_SECRET",
            "VAK_SCRUB_TEST_API_KEY",
            "VAK_SCRUB_TEST_TOKEN",
        ] {
            assert!(
                !keys.contains(&secret.to_string()),
                "`{secret}` leaked into child env"
            );
        }
        // Cleanup
        for secret in [
            "VAK_SCRUB_TEST_SECRET",
            "VAK_SCRUB_TEST_API_KEY",
            "VAK_SCRUB_TEST_TOKEN",
        ] {
            #[allow(unsafe_code)]
            unsafe {
                std::env::remove_var(secret);
            }
        }
    }

    #[test]
    fn package_detection_covers_multiple_ecosystems() {
        use super::detect_installed_packages;
        assert_eq!(
            detect_installed_packages("pip install numpy pandas"),
            Some(vec!["numpy".into(), "pandas".into()])
        );
        assert_eq!(
            detect_installed_packages("npm i -D typescript solid-js"),
            Some(vec!["typescript".into(), "solid-js".into()])
        );
        assert_eq!(
            detect_installed_packages("cargo add tokio serde"),
            Some(vec!["tokio".into(), "serde".into()])
        );
        assert_eq!(
            detect_installed_packages("uv add fastapi uvicorn"),
            Some(vec!["fastapi".into(), "uvicorn".into()])
        );
        assert_eq!(
            detect_installed_packages("pnpm add tailwindcss"),
            Some(vec!["tailwindcss".into()])
        );
        assert_eq!(detect_installed_packages("ls -la"), None);
    }

    #[test]
    fn mime_type_guessing() {
        use super::guess_mime_type;
        use std::path::Path;
        assert_eq!(guess_mime_type(Path::new("chart.png")), "image/png");
        assert_eq!(guess_mime_type(Path::new("index.html")), "text/html");
        assert_eq!(guess_mime_type(Path::new("data.csv")), "text/csv");
        assert_eq!(
            guess_mime_type(Path::new("unknown.xyz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn control_files_are_not_available_to_sandbox_commands() {
        assert!(super::references_control_file("cat .env"));
        assert!(super::references_control_file("cp result .vak/config.toml"));
        assert!(!super::references_control_file("echo ok > result.txt"));
    }

    #[tokio::test]
    async fn explicit_quarantine_false_runs_in_workspace() {
        use crate::{Tool, ToolContext, sandbox_events::SandboxEventSink};
        let workspace = tempfile::tempdir().unwrap();
        let test_file = workspace.path().join("marker.txt");
        std::fs::write(&test_file, "workspace-marker").unwrap();

        let (sink, _rx) = SandboxEventSink::new_with_id("test-quarantine-false".into());
        let ctx = ToolContext::new(workspace.path().to_path_buf())
            .with_sandbox_sink(sink.with_quarantine(true));

        // When quarantine: false is explicitly set, it should run in workspace root and find marker.txt
        let tool = super::BashTool;
        let out = tool
            .execute(
                &serde_json::json!({
                    "command": "cat marker.txt",
                    "quarantine": false
                }),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("workspace-marker"));
    }

    #[tokio::test]
    async fn custom_cwd_is_respected() {
        use crate::{Tool, ToolContext, sandbox_events::SandboxEventSink};
        let workspace = tempfile::tempdir().unwrap();
        let sub = workspace
            .path()
            .join(".vak/scratch/test-custom-cwd/sub_module");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("sub.txt"), "in-sub").unwrap();

        let (sink, _rx) = SandboxEventSink::new_with_id("test-custom-cwd".into());
        let ctx = ToolContext::new(workspace.path().to_path_buf())
            .with_sandbox_sink(sink.with_quarantine(true));

        let tool = super::BashTool;
        let out = tool
            .execute(
                &serde_json::json!({
                    "command": "cat sub.txt",
                    "cwd": ".vak/scratch/test-custom-cwd/sub_module"
                }),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("in-sub"));
    }

    #[tokio::test]
    async fn custom_cwd_rejects_workspace_escape() {
        use crate::{Tool, ToolContext, sandbox_events::SandboxEventSink};
        let workspace = tempfile::tempdir().unwrap();
        let (sink, _rx) = SandboxEventSink::new_with_id("test-cwd-escape".into());
        let ctx = ToolContext::new(workspace.path().to_path_buf())
            .with_sandbox_sink(sink.with_quarantine(true));
        let out = super::BashTool
            .execute(
                &serde_json::json!({
                    "command": "pwd",
                    "cwd": ".."
                }),
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("active execution root"));
    }

    #[tokio::test]
    async fn quarantined_command_can_write_to_dot_vak_scratch() {
        use crate::{Tool, ToolContext, sandbox_events::SandboxEventSink};
        let workspace = tempfile::tempdir().unwrap();
        let (sink, mut events) = SandboxEventSink::new_with_id("test-scratch-write".into());
        let ctx = ToolContext::new(workspace.path().to_path_buf())
            .with_sandbox_sink(sink.with_quarantine(true));

        let tool = super::BashTool;
        let out = tool
            .execute(
                &serde_json::json!({
                    "command": "cat > .vak/scratch/quick_react_dashboard.html <<'EOF'\n<h1>React Dashboard</h1>\nEOF"
                }),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "stdout/stderr: {}", out.content);
        let created = workspace
            .path()
            .join(".vak/scratch/test-scratch-write/quick_react_dashboard.html");
        assert!(created.is_file(), "file should exist at {:?}", created);
        let mut saw_artifact = false;
        while let Ok(event) = events.try_recv() {
            if let crate::SandboxEvent::ArtifactGenerated { path, .. } = event {
                saw_artifact |= path.contains("quick_react_dashboard.html");
            }
        }
        assert!(saw_artifact, "artifact event should have been emitted");
    }
}
