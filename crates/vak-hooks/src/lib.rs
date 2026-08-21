//! vak-hooks: user-defined script handlers at agent lifecycle points.
//!
//! Handlers receive JSON context on stdin and may answer with JSON on
//! stdout (`{"decision":"block"|"approve","reason":"..."}`) or exit code 2
//! (block, stderr becomes the reason). Exit 0 without output = no opinion.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use serde_json::Value;
use tokio::io::AsyncWriteExt;

use vak_permission::Rule;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    SessionStart,
    PreToolUse,
    PostToolUse,
    Stop,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::SessionStart => "session_start",
            HookEvent::PreToolUse => "pre_tool_use",
            HookEvent::PostToolUse => "post_tool_use",
            HookEvent::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone)]
pub struct HookDef {
    pub event: HookEvent,
    pub matcher: Option<Rule>,
    pub command: String,
    pub timeout_ms: u64,
}

pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const MAX_CAPTURE: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookOutcome {
    pub blocked: bool,
    pub reason: Option<String>,
}

impl HookOutcome {
    fn merge(&mut self, other: HookOutcome) {
        if other.blocked && !self.blocked {
            self.blocked = true;
            self.reason = other.reason.or_else(|| Some("blocked by hook".into()));
        }
    }
}

fn hook_input(
    event: HookEvent,
    session_id: &str,
    cwd: &Path,
    tool: Option<(&str, &Value)>,
    extra: Option<&str>,
) -> Value {
    let mut v = serde_json::json!({
        "event": event.as_str(),
        "session_id": session_id,
        "cwd": cwd.display().to_string(),
    });
    if let Some((name, input)) = tool {
        v["tool"] = serde_json::json!({"name": name, "input": input});
    }
    if let Some(text) = extra {
        v["text"] = Value::String(text.to_string());
    }
    v
}

/// Runs every matching hook for the event sequentially; first block wins.
pub async fn run_hooks(
    hooks: Arc<Vec<HookDef>>,
    event: HookEvent,
    session_id: &str,
    cwd: &Path,
    tool: Option<(&str, &Value)>,
    extra: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> HookOutcome {
    let mut combined = HookOutcome::default();
    for hook in hooks.iter() {
        if hook.event != event {
            continue;
        }
        if let (Some(rule), Some((name, input))) = (&hook.matcher, tool)
            && !rule.matches(name, input)
        {
            continue;
        }

        let outcome = run_one(hook, event, session_id, cwd, tool, extra, cancel).await;
        combined.merge(outcome);
        if combined.blocked {
            return combined;
        }
    }
    combined
}

async fn run_one(
    hook: &HookDef,
    event: HookEvent,
    session_id: &str,
    cwd: &Path,
    tool: Option<(&str, &Value)>,
    extra: Option<&str>,
    cancel: &tokio_util::sync::CancellationToken,
) -> HookOutcome {
    let payload =
        serde_json::to_vec(&hook_input(event, session_id, cwd, tool, extra)).unwrap_or_default();

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c")
        .arg(&hook.command)
        .current_dir(cwd)
        .stdin(Stdio::piped())
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
        Err(e) => {
            return HookOutcome {
                blocked: false,
                reason: Some(format!("hook spawn failed: {e}")),
            };
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&payload).await;
        let _ = stdin.shutdown().await;
    }

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_fut = tokio::spawn(async move { read_capped(&mut stdout).await });
    let err_fut = tokio::spawn(async move { read_capped(&mut stderr).await });

    let timeout = tokio::time::sleep(std::time::Duration::from_millis(hook.timeout_ms));
    tokio::select! {
        _ = timeout => {
            kill_tree(&child.id());
            let _ = child.wait().await;
            HookOutcome { blocked: false, reason: Some("hook timed out".into()) }
        }
        _ = cancel.cancelled() => {
            kill_tree(&child.id());
            let _ = child.wait().await;
            HookOutcome::default()
        }
        status = child.wait() => {
            let status = match status {
                Ok(s) => s,
                Err(e) => return HookOutcome { blocked: false, reason: Some(format!("hook wait failed: {e}")) },
            };
            let out = out_fut.await.unwrap_or_default();
            let err = err_fut.await.unwrap_or_default();

            if status.code() == Some(2) {
                let reason = if err.trim().is_empty() { out.trim() } else { err.trim() };
                return HookOutcome {
                    blocked: true,
                    reason: Some(if reason.is_empty() { "blocked by hook (exit 2)".into() } else { reason.to_string() }),
                };
            }

            if let Ok(v) = serde_json::from_str::<Value>(out.trim()) {
                match v.get("decision").and_then(|d| d.as_str()) {
                    Some("block") => {
                        return HookOutcome {
                            blocked: true,
                            reason: Some(
                                v.get("reason").and_then(|r| r.as_str()).unwrap_or("blocked by hook").to_string(),
                            ),
                        };
                    }
                    Some("approve") | None => {}
                    Some(other) => {
                        return HookOutcome { blocked: false, reason: Some(format!("unknown hook decision '{other}'")) };
                    }
                }
            }

            HookOutcome::default()
        }
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

async fn read_capped<R: tokio::io::AsyncRead + Unpin>(r: &mut Option<R>) -> String {
    use tokio::io::AsyncReadExt;
    let Some(r) = r else {
        return String::new();
    };
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
