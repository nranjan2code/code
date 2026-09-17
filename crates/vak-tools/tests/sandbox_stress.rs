//! Real command stress cases for the universal sandbox execution contract.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use vak_tools::{Tool, ToolContext, bash::BashTool, sandbox_events::SandboxEventSink};

#[tokio::test]
async fn nested_data_artifacts_stay_in_execution_scratch() {
    let workspace = tempfile::tempdir().unwrap();
    let (sink, mut events) = SandboxEventSink::new_with_id("stress-data".into());
    let ctx = ToolContext::new(workspace.path().to_path_buf())
        .with_sandbox_sink(sink.with_quarantine(true));
    let output = BashTool
        .execute(
            &serde_json::json!({"command": "mkdir -p nested/deeper && printf 'a,b\\n1,2\\n' > nested/deeper/data.csv && pwd"}),
            &ctx,
        )
        .await;
    assert!(!output.is_error, "{}", output.content);
    assert!(!workspace.path().join("nested").exists());
    assert!(
        workspace
            .path()
            .join(".vak/scratch/vak/stress-data/nested/deeper/data.csv")
            .is_file()
    );
    let mut saw_nested = false;
    while let Ok(event) = events.try_recv() {
        if let vak_tools::SandboxEvent::ArtifactGenerated {
            execution_id, path, ..
        } = event
        {
            assert_eq!(execution_id, "stress-data");
            saw_nested |= path.contains("nested/deeper/data.csv");
        }
    }
    assert!(saw_nested, "nested artifact was not observed");
}

#[tokio::test]
async fn timeout_keeps_partial_output_and_kills_descendant() {
    let workspace = tempfile::tempdir().unwrap();
    let ctx = ToolContext::new(workspace.path().to_path_buf());
    let start = std::time::Instant::now();
    let output = BashTool
        .execute(
            &serde_json::json!({"command": "printf before; sleep 30", "timeout_ms": 1000}),
            &ctx,
        )
        .await;
    assert!(output.is_error);
    assert!(output.content.contains("before"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn concurrent_executions_keep_event_identity_separate() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let (sink_a, mut events_a) = SandboxEventSink::new_with_id("stress-a".into());
    let (sink_b, mut events_b) = SandboxEventSink::new_with_id("stress-b".into());
    let ctx_a = Arc::new(ToolContext::new(a.path().to_path_buf()).with_sandbox_sink(sink_a));
    let ctx_b = Arc::new(ToolContext::new(b.path().to_path_buf()).with_sandbox_sink(sink_b));
    let args_a = serde_json::json!({"command": "echo alpha"});
    let args_b = serde_json::json!({"command": "echo beta"});
    let (out_a, out_b) = tokio::join!(
        BashTool.execute(&args_a, &ctx_a),
        BashTool.execute(&args_b, &ctx_b),
    );
    assert!(out_a.content.contains("alpha"));
    assert!(out_b.content.contains("beta"));
    while let Ok(event) = events_a.try_recv() {
        let id = match event {
            vak_tools::SandboxEvent::ExecutionStarted { execution_id, .. }
            | vak_tools::SandboxEvent::Stdout { execution_id, .. }
            | vak_tools::SandboxEvent::Stderr { execution_id, .. }
            | vak_tools::SandboxEvent::ExecutionFinished { execution_id, .. } => execution_id,
            _ => "stress-a".into(),
        };
        assert_eq!(id, "stress-a");
    }
    while let Ok(event) = events_b.try_recv() {
        let id = match event {
            vak_tools::SandboxEvent::ExecutionStarted { execution_id, .. }
            | vak_tools::SandboxEvent::Stdout { execution_id, .. }
            | vak_tools::SandboxEvent::Stderr { execution_id, .. }
            | vak_tools::SandboxEvent::ExecutionFinished { execution_id, .. } => execution_id,
            _ => "stress-b".into(),
        };
        assert_eq!(id, "stress-b");
    }
}

#[tokio::test]
async fn control_files_are_denied_before_spawn() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join(".env"), "SECRET=must-not-leak").unwrap();
    let (sink, _events) = SandboxEventSink::new_with_id("stress-control".into());
    let ctx = ToolContext::new(workspace.path().to_path_buf()).with_sandbox_sink(sink);
    let output = BashTool
        .execute(&serde_json::json!({"command": "cat .env"}), &ctx)
        .await;
    assert!(output.is_error);
    assert!(output.content.contains("control files"));
}
