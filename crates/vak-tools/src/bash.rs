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

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(command) = args.get("command").and_then(|c| c.as_str()) else {
            return ToolOutput::error("missing required parameter: command");
        };
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|t| t.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1000);

        let mut cmd = shell_command(command);
        cmd.current_dir(&ctx.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        #[allow(unsafe_code)]
        {
            unsafe {
                cmd.pre_exec(|| {
                    libc::setpgid(0, 0);
                    Ok(())
                });
            }
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("spawn failed: {e}")),
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
                kill_tree(&child.id());
                let _ = child.wait().await;
                return ToolOutput::error(format!("command timed out after {timeout_ms}ms"));
            }
            _ = cancelled => {
                kill_tree(&child.id());
                let _ = child.wait().await;
                return ToolOutput::error("command cancelled");
            }
            status = child.wait() => {
                let status = match status {
                    Ok(s) => s,
                    Err(e) => return ToolOutput::error(format!("wait failed: {e}")),
                };
                let out = out_fut.await.unwrap_or_default();
                let err = err_fut.await.unwrap_or_default();

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

fn kill_tree(pid: &Option<u32>) {
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
