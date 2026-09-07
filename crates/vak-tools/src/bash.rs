use std::process::Stdio;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::{Tool, ToolContext, ToolOutput};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_CAPTURE: usize = 1 << 20;

pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return combined stdout/stderr with the exit code. The process tree is killed on timeout or cancellation."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command to execute"},
                "timeout_ms": {"type": "integer", "minimum": 1000, "description": "Timeout in milliseconds (default 120000)"}
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
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1000);

        let effective = match &ctx.sandbox {
            Some(sb) => sb.wrap(command),
            None => command.to_string(),
        };

        let mut cmd = shell_command(&effective);
        cmd.current_dir(&ctx.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub_environment(&mut cmd);

        if std::env::var_os(crate::broker::WORKER_ENV).is_none() {
            isolate_process_group(&mut cmd);
        }

        let scratch_dir = ctx.cwd.join(".vak").join("scratch");
        let _ = std::fs::create_dir_all(&scratch_dir);
        let before_scratch = collect_scratch_files(&scratch_dir);

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
                let duration_ms = start_instant.elapsed().as_millis() as u64;
                if let Some(ref sink) = ctx.sandbox_sink {
                    sink.emit_finished(-1, duration_ms, Vec::new());
                }
                return ToolOutput::error(format!("command timed out after {timeout_ms}ms"));
            }
            _ = cancelled => {
                telemetry_cancel.cancel();
                let _ = telemetry_handle.await;
                kill_process_group(&child.id());
                let _ = child.wait().await;
                let duration_ms = start_instant.elapsed().as_millis() as u64;
                if let Some(ref sink) = ctx.sandbox_sink {
                    sink.emit_finished(-1, duration_ms, Vec::new());
                }
                return ToolOutput::error("command cancelled");
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

                let new_artifacts = scan_new_scratch_artifacts(&scratch_dir, &before_scratch, &ctx.cwd);
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

pub(crate) fn scrub_environment(cmd: &mut tokio::process::Command) {
    let allowed = [
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
    let inherited: Vec<(String, std::ffi::OsString)> = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            if allowed.contains(&key.as_str()) || key.starts_with("LC_") || key.starts_with("XDG_")
            {
                Some((key, value))
            } else {
                None
            }
        })
        .collect();
    cmd.env_clear();
    cmd.envs(inherited);
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
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let space = MAX_CAPTURE.saturating_sub(buf.len());
                let taken = n.min(space);
                buf.extend_from_slice(&chunk[..taken]);
                if let Some(ref sink) = sink {
                    let chunk_str = String::from_utf8_lossy(&chunk[..n]);
                    if is_stderr {
                        sink.emit_stderr(&chunk_str);
                    } else {
                        sink.emit_stdout(&chunk_str);
                    }
                }
                if buf.len() >= MAX_CAPTURE {
                    break;
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

fn collect_scratch_files(
    dir: &std::path::Path,
) -> std::collections::HashMap<std::path::PathBuf, std::time::SystemTime> {
    let mut map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && let Ok(meta) = path.metadata()
            {
                let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                map.insert(path, mtime);
            }
        }
    }
    map
}

fn scan_new_scratch_artifacts(
    scratch_dir: &std::path::Path,
    before: &std::collections::HashMap<std::path::PathBuf, std::time::SystemTime>,
    cwd: &std::path::Path,
) -> Vec<(String, String, u64)> {
    let mut artifacts = Vec::new();
    if let Ok(entries) = std::fs::read_dir(scratch_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
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
mod tests {
    use super::scrub_environment;

    #[test]
    fn restricted_environment_drops_unlisted_secrets() {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.env("VAK_TEST_SECRET", "must-not-survive");
        scrub_environment(&mut cmd);
        assert!(
            cmd.as_std()
                .get_envs()
                .all(|(key, _)| key != "VAK_TEST_SECRET")
        );
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
}
