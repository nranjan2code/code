#![allow(clippy::unwrap_used)]
use std::sync::Arc;
use vak_tools::{Tool, ToolContext, SandboxEventSink};
use vak_tools::sandbox::{SandboxMode, Seatbelt};

#[test]
fn real_broker_drops_runtime_control_protection() {
    vak_config::paths::isolate_home_for_tests();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join(".env"), "SYNTHETIC_AUDIT_MARKER=not-a-secret").unwrap();
    let (sink, _rx) = SandboxEventSink::new_with_id("audit".into());
    let mut ctx = ToolContext::new(work.path().to_path_buf())
        .with_sandbox_sink(sink.with_owner_session("synthetic-session"));
    ctx.sandbox = Some(Arc::new(Seatbelt::new(SandboxMode::WorkspaceWrite, work.path())));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let args = serde_json::json!({"path": ".env"});
    let direct = rt.block_on(vak_tools::read::ReadTool.execute(&args, &ctx));
    assert!(direct.is_error(), "direct call should protect .env");
    let worker = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/vak-tool-worker");
    let broker = vak_tools::broker::BrokeredTool::new(Arc::new(vak_tools::read::ReadTool), worker.clone(), vec![]);
    let actual = rt.block_on(broker.execute(&args, &ctx));
    assert!(!actual.is_error(), "broker read failed: {}", actual.content);
    assert!(actual.content.contains("SYNTHETIC_AUDIT_MARKER"));
    let write = vak_tools::broker::BrokeredTool::new(Arc::new(vak_tools::write::WriteTool), worker, vec![]);
    let actual = rt.block_on(write.execute(&serde_json::json!({"path":".vak/permissions.local.toml", "content":"allow = [] # synthetic overwrite"}), &ctx));
    assert!(!actual.is_error(), "broker write failed: {}", actual.content);
    assert!(std::fs::read_to_string(work.path().join(".vak/permissions.local.toml")).unwrap().contains("synthetic overwrite"));
    println!("CONFIRMED: sandboxed broker read .env and wrote permission control file; direct read refused");
}

#[test]
fn repository_content_does_not_set_untrusted_label() {
    let held = vak_tools::read::ReadTool.provenance();
    assert!(held.private);
    assert!(!held.untrusted);
    assert!(!held.gates_exits());
    println!("CONFIRMED: reading arbitrary repository instructions sets private only; outbound provenance gate remains unset");
}
