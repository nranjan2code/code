#![allow(clippy::unwrap_used)]
use serde_json::json;
use vak_tools::{Tool, ToolContext, SandboxEvent, SandboxEventSink};
use vak_tools::bash::BashTool;

#[tokio::test]
async fn audit_scratch_is_not_cwd_and_nested_artifacts_are_omitted() {
    let dir = tempfile::tempdir().unwrap();
    let (sink, mut rx) = SandboxEventSink::new();
    let mut ctx = ToolContext::new(dir.path().to_path_buf());
    ctx.sandbox_sink = Some(sink);
    let out = BashTool.execute(&json!({"command":"printf root > root-output.txt; mkdir -p .vak/scratch/nested; printf nested > .vak/scratch/nested/result.txt; printf flat > .vak/scratch/result.txt"}), &ctx).await;
    assert!(!out.is_error, "{}", out.content);
    assert!(dir.path().join("root-output.txt").exists());
    let mut paths = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let SandboxEvent::ArtifactGenerated { path, .. } = event { paths.push(path); }
    }
    assert_eq!(paths, vec![".vak/scratch/result.txt"]);
}

#[tokio::test]
async fn audit_timeout_discards_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ToolContext::new(dir.path().to_path_buf());
    let out = BashTool.execute(&json!({"command":"printf audit-partial; sleep 3", "timeout_ms":1000}), &ctx).await;
    assert!(out.is_error);
    assert!(out.content.contains("timed out"));
    assert!(!out.content.contains("audit-partial"));
}

#[test]
fn audit_unicode_preview_can_panic() {
    let (sink, _rx) = SandboxEventSink::new();
    let code = format!("{}€", "x".repeat(1999));
    assert!(std::panic::catch_unwind(|| sink.emit_execution_started("bash", &code, "bash", ".")).is_err());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn audit_workspace_control_files_are_not_excluded_by_seatbelt() {
    use std::sync::Arc;
    use vak_tools::sandbox::{Seatbelt, SandboxMode};
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".vak")).unwrap();
    std::fs::write(dir.path().join(".env"), "AUDIT_FAKE=fixture-only").unwrap();
    std::fs::write(dir.path().join(".vak/config.toml"), "# fixture").unwrap();
    let mut ctx = ToolContext::new(dir.path().to_path_buf());
    ctx.sandbox = Some(Arc::new(Seatbelt::new(SandboxMode::WorkspaceWrite, dir.path())));
    let out = BashTool.execute(&json!({"command":"cat .env; printf '# changed' > .vak/config.toml"}), &ctx).await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("fixture-only"));
    assert_eq!(std::fs::read_to_string(dir.path().join(".vak/config.toml")).unwrap(), "# changed");
}
