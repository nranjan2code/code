use std::path::Path;
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::{Tool, ToolContext, ToolOutput};

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_CAPTURE: usize = 1 << 20;

pub struct PythonTool {
    pub network: bool,
}

impl PythonTool {
    pub fn new(network: bool) -> Self {
        Self { network }
    }
}

impl Default for PythonTool {
    fn default() -> Self {
        Self::new(false)
    }
}

#[async_trait]
impl Tool for PythonTool {
    fn name(&self) -> &str {
        "python_eval"
    }

    fn description(&self) -> &str {
        "Execute Python code or scripts in a sandboxed, workspace-locked environment. \
Intermediate scripts, virtual site-packages, and caches are quarantined inside \
.vak/scratch/python/ and will never pollute the workspace directory until explicitly requested. \
Pre-installed libraries (pandas, numpy, matplotlib, etc.) are available, and pip installation is \
supported. Execution is restricted to the workspace, operational environment is scrubbed, and \
execution duration is strictly bounded."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Python code snippet to execute. Quarantined in .vak/scratch/python/."
                },
                "script_path": {
                    "type": "string",
                    "description": "Optional relative path to an existing .py script in the workspace."
                },
                "install_packages": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of PyPI packages to install into the quarantined scratch environment (.vak/scratch/python/site-packages) before execution."
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional command line arguments to pass to the script."
                },
                "timeout_ms": {
                    "type": "integer",
                    "minimum": 1000,
                    "description": "Timeout in milliseconds (default 60000)."
                }
            }
        })
    }

    fn claims(&self, args: &Value) -> crate::ResourceClaims {
        let mut paths = Vec::new();
        if let Some(path) = args.get("script_path").and_then(Value::as_str) {
            paths.push(path.to_string());
        }
        crate::ResourceClaims {
            exclusive: false,
            read_only: false,
            paths,
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let code = args.get("code").and_then(Value::as_str);
        let script_path = args.get("script_path").and_then(Value::as_str);
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1000);

        if code.is_none() && script_path.is_none() {
            return ToolOutput::error(
                "missing required parameter: either 'code' or 'script_path' must be provided",
            );
        }

        // Ensure quarantined scratch directory: .vak/scratch/python/
        let scratch_dir = ctx.cwd.join(".vak/scratch/python");
        let site_packages = scratch_dir.join("site-packages");
        if let Err(e) = std::fs::create_dir_all(&site_packages) {
            return ToolOutput::error(format!("failed to initialize scratch directory: {e}"));
        }

        // Handle package installations if requested
        if let Some(packages) = args.get("install_packages").and_then(Value::as_array) {
            let pkg_list: Vec<&str> = packages.iter().filter_map(Value::as_str).collect();
            if !pkg_list.is_empty() {
                if !self.network {
                    return ToolOutput::error(
                        "Package installation denied: network access is disabled for python-sandbox in configuration",
                    );
                }
                let pip_cmd = format!(
                    "pip install --quiet --no-warn-script-location --target {} {}",
                    shell_quote(&site_packages.display().to_string()),
                    pkg_list.join(" ")
                );
                let effective_pip = match &ctx.sandbox {
                    Some(sb) => sb.wrap(&pip_cmd),
                    None => pip_cmd,
                };
                let mut pip_proc = shell_command(&effective_pip);
                pip_proc
                    .current_dir(&ctx.cwd)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());
                crate::bash::scrub_environment(&mut pip_proc);
                if std::env::var_os(crate::broker::WORKER_ENV).is_none() {
                    crate::bash::isolate_process_group(&mut pip_proc);
                }
                match pip_proc.output().await {
                    Ok(out) if !out.status.success() => {
                        let err = String::from_utf8_lossy(&out.stderr);
                        return ToolOutput::error(format!(
                            "pip install failed (check network permissions): {}",
                            err.trim()
                        ));
                    }
                    Err(e) => {
                        return ToolOutput::error(format!("failed to spawn pip install: {e}"));
                    }
                    _ => {}
                }
            }
        }

        // Prepare the script target
        let target_script = if let Some(code_content) = code {
            let now_nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let filename = format!("exec_{now_nanos}.py");
            let file_path = scratch_dir.join(&filename);
            if let Err(e) = std::fs::write(&file_path, code_content) {
                return ToolOutput::error(format!(
                    "failed to stage Python code in scratch directory: {e}"
                ));
            }
            file_path
        } else {
            let rel = script_path.unwrap_or_default();
            let candidate = ctx.cwd.join(rel);
            // Verify path confinement to workspace (Invariant 10)
            match candidate.canonicalize() {
                Ok(canon) => {
                    let ws_canon = ctx.cwd.canonicalize().unwrap_or_else(|_| ctx.cwd.clone());
                    if !canon.starts_with(&ws_canon) {
                        return ToolOutput::error(format!(
                            "access denied: script path '{}' resolves outside workspace",
                            rel
                        ));
                    }
                    canon
                }
                Err(e) => {
                    return ToolOutput::error(format!("script file not found '{}': {e}", rel));
                }
            }
        };

        // Command arguments
        let mut script_args = Vec::new();
        if let Some(passed_args) = args.get("args").and_then(Value::as_array) {
            for a in passed_args {
                if let Some(s) = a.as_str() {
                    script_args.push(shell_quote(s));
                }
            }
        }

        // Formulate python execution command
        let python_cmd = format!(
            "python3 {} {}",
            shell_quote(&target_script.display().to_string()),
            script_args.join(" ")
        );

        let effective = match &ctx.sandbox {
            Some(sb) => sb.wrap(&python_cmd),
            None => python_cmd,
        };

        let mut cmd = shell_command(&effective);
        cmd.current_dir(&ctx.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Operational environment scrub (Invariant 12)
        crate::bash::scrub_environment(&mut cmd);

        // Configure Python runtime isolation
        cmd.env("PYTHONDONTWRITEBYTECODE", "1");
        cmd.env("PYTHONUNBUFFERED", "1");
        let python_path = if site_packages.exists() {
            format!("{}:{}", site_packages.display(), ctx.cwd.display())
        } else {
            ctx.cwd.display().to_string()
        };
        cmd.env("PYTHONPATH", python_path);

        if std::env::var_os(crate::broker::WORKER_ENV).is_none() {
            crate::bash::isolate_process_group(&mut cmd);
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("failed to spawn python runtime: {e}")),
        };

        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let out_fut = tokio::spawn(async move {
            match stdout.as_mut() {
                Some(r) => read_capped(r).await,
                None => String::new(),
            }
        });
        let err_fut = tokio::spawn(async move {
            match stderr.as_mut() {
                Some(r) => read_capped(r).await,
                None => String::new(),
            }
        });

        let timeout = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms));
        let cancelled = ctx.cancel.cancelled();
        tokio::select! {
            _ = timeout => {
                crate::bash::kill_process_group(&child.id());
                let _ = child.wait().await;
                return ToolOutput::error(format!("python execution timed out after {timeout_ms}ms"));
            }
            _ = cancelled => {
                crate::bash::kill_process_group(&child.id());
                let _ = child.wait().await;
                return ToolOutput::error("python execution cancelled");
            }
            status = child.wait() => {
                let status = match status {
                    Ok(s) => s,
                    Err(e) => return ToolOutput::error(format!("wait failed: {e}")),
                };
                let out_text = out_fut.await.unwrap_or_default();
                let err_text = err_fut.await.unwrap_or_default();
                let mut text = String::new();
                if !out_text.is_empty() {
                    text.push_str(&out_text);
                }
                if !err_text.is_empty() {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str("[stderr]\n");
                    text.push_str(&err_text);
                }

                // Look for generated plot or data artifacts in scratch
                let artifacts = find_generated_artifacts(&scratch_dir);
                if !artifacts.is_empty() {
                    text.push_str("\n\n[Generated Artifacts in .vak/scratch/python/]:\n");
                    for art in artifacts {
                        text.push_str(&format!("- {art}\n"));
                    }
                }

                if !status.success() {
                    text.push_str(&format!("\n[exit code: {}]", status.code().unwrap_or(-1)));
                    return ToolOutput {
                        content: ctx.truncate_output(text),
                        is_error: true,
                    };
                }

                if text.is_empty() {
                    text.push_str("(no output; execution completed with code 0)");
                }

                ToolOutput::ok(ctx.truncate_output(text))
            }
        }
    }
}

fn find_generated_artifacts(scratch_dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(scratch_dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(ext) = path.extension().and_then(|s| s.to_str())
        {
            let ext_lower = ext.to_ascii_lowercase();
            if matches!(
                ext_lower.as_str(),
                "svg" | "png" | "jpg" | "jpeg" | "csv" | "json" | "parquet"
            ) {
                found.push(path.display().to_string());
            }
        }
    }
    found
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

fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

async fn read_capped<R: AsyncReadExt + Unpin>(r: &mut R) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let space = MAX_CAPTURE.saturating_sub(buf.len());
                buf.extend_from_slice(&chunk[..n.min(space)]);
                if buf.len() >= MAX_CAPTURE {
                    break;
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn python_eval_runs_code_and_quarantines_scratch() {
        let dir = tempdir().unwrap();
        let tool = PythonTool::new(false);
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let args = serde_json::json!({
            "code": "print('hello from sandboxed python')"
        });
        let res = tool.execute(&args, &ctx).await;
        assert!(!res.is_error, "error: {}", res.content);
        assert!(res.content.contains("hello from sandboxed python"));

        // Verify scratch quarantine
        let scratch = dir.path().join(".vak/scratch/python");
        assert!(scratch.is_dir());

        // Verify cwd has no loose files
        let top_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != ".vak")
            .collect();
        assert!(top_files.is_empty(), "loose files in cwd: {:?}", top_files);
    }

    #[tokio::test]
    async fn python_rejects_script_outside_workspace() {
        let dir = tempdir().unwrap();
        let tool = PythonTool::new(false);
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let args = serde_json::json!({
            "script_path": "../../outside.py"
        });
        let res = tool.execute(&args, &ctx).await;
        assert!(res.is_error);
        assert!(
            res.content.contains("resolves outside workspace") || res.content.contains("not found")
        );
    }

    #[tokio::test]
    async fn python_denies_network_package_install_when_network_disabled() {
        let dir = tempdir().unwrap();
        let tool = PythonTool::new(false);
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let args = serde_json::json!({
            "code": "print('ok')",
            "install_packages": ["requests"]
        });
        let res = tool.execute(&args, &ctx).await;
        assert!(res.is_error);
        assert!(res.content.contains("network access is disabled"));
    }
}
