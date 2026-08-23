#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use tempfile::tempdir;
use vak_tools::sandbox::{Sandbox, SandboxMode, SandboxTarget, Seatbelt};
use vak_tools::{ToolContext, brokered_default_tools};

fn worker() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vakcoder"))
}

fn tool(name: &str) -> Arc<dyn vak_tools::Tool> {
    brokered_default_tools(worker())
        .into_iter()
        .find(|tool| tool.name() == name)
        .expect("built-in tool")
}

struct CommandSandbox;

impl Sandbox for CommandSandbox {
    fn name(&self) -> &str {
        "test-command"
    }

    fn wrap(&self, command: &str) -> String {
        format!("printf 'command-boundary\\n'; {command}")
    }

    fn target(&self) -> SandboxTarget {
        SandboxTarget::ToolCommand
    }
}

#[tokio::test]
async fn builtins_execute_in_worker_process() {
    let workspace = tempdir().expect("workspace");
    let ctx = ToolContext::new(workspace.path().to_path_buf());

    let write = tool("write")
        .execute(&json!({"path": "inside.txt", "content": "brokered"}), &ctx)
        .await;
    assert!(!write.is_error, "{}", write.content);

    let read = tool("read")
        .execute(&json!({"path": "inside.txt"}), &ctx)
        .await;
    assert!(!read.is_error, "{}", read.content);
    assert!(read.content.contains("brokered"));
}

#[tokio::test]
async fn missing_worker_fails_closed() {
    let workspace = tempdir().expect("workspace");
    let mut tools = vak_tools::brokered_default_tools(workspace.path().join("missing-worker"));
    let output = tools
        .remove(0)
        .execute(
            &json!({"path": "anything"}),
            &ToolContext::new(workspace.path().to_path_buf()),
        )
        .await;
    assert!(output.is_error);
    assert!(output.content.contains("broker unavailable"));
}

#[tokio::test]
async fn cancellation_terminates_worker_process_group() {
    let workspace = tempdir().expect("workspace");
    let ctx = ToolContext::new(workspace.path().to_path_buf());
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cancel.cancel();
    });

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tool("bash").execute(&json!({"command": "sleep 30"}), &ctx),
    )
    .await
    .expect("broker cancellation deadline");
    assert!(output.is_error);
    assert!(output.content.contains("cancelled"));
}

#[tokio::test]
async fn command_scoped_sandbox_wraps_bash_inside_protocol() {
    let workspace = tempdir().expect("workspace");
    let mut ctx = ToolContext::new(workspace.path().to_path_buf());
    ctx.sandbox = Some(Arc::new(CommandSandbox));

    let output = tool("bash")
        .execute(&json!({"command": "printf 'tool-command\\n'"}), &ctx)
        .await;
    assert!(!output.is_error, "{}", output.content);
    assert!(output.content.contains("command-boundary"));
    assert!(output.content.contains("tool-command"));
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn restricted_worker_cannot_read_home_outside_workspace() {
    let workspace = tempdir().expect("workspace");
    let home = PathBuf::from(std::env::var_os("HOME").expect("home"));
    let protected = tempfile::Builder::new()
        .prefix("vak-broker-protected-")
        .tempdir_in(home)
        .expect("protected directory");
    let secret = protected.path().join("secret.txt");
    std::fs::write(&secret, "must-not-cross-boundary").expect("secret fixture");
    let mut ctx = ToolContext::new(workspace.path().to_path_buf());
    let mut seatbelt = Seatbelt::new(SandboxMode::WorkspaceWrite, workspace.path());
    seatbelt
        .read_paths
        .push(worker().parent().expect("worker parent").to_path_buf());
    ctx.sandbox = Some(Arc::new(seatbelt));

    let output = tool("read").execute(&json!({"path": secret}), &ctx).await;
    assert!(output.is_error);
    assert!(!output.content.contains("must-not-cross-boundary"));
    assert!(
        !output.content.contains("tool broker exited"),
        "{}",
        output.content
    );
}
