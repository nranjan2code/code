use serde_json::json;
use vak_agent::{ApprovalMode, auto_approve};
use vak_permission::{AskSource, Mode};

#[test]
fn approve_safe_never_bypasses_explicit_rules_or_circuit_breaker()
-> Result<(), Box<dyn std::error::Error>> {
    let cwd = tempfile::tempdir()?;
    let input = json!({"path": "file.txt"});

    assert!(!auto_approve(
        ApprovalMode::ApproveSafe,
        AskSource::Rule,
        "write",
        &input,
        Mode::WorkspaceWrite,
        true,
        cwd.path(),
    ));
    assert!(!auto_approve(
        ApprovalMode::ApproveSafe,
        AskSource::CircuitBreaker,
        "read",
        &input,
        Mode::ReadOnly,
        true,
        cwd.path(),
    ));
    assert!(!auto_approve(
        ApprovalMode::AutoApprove,
        AskSource::CircuitBreaker,
        "read",
        &input,
        Mode::ReadOnly,
        true,
        cwd.path(),
    ));
    Ok(())
}

#[test]
fn approve_safe_rejects_paths_that_escape_the_workspace() -> Result<(), Box<dyn std::error::Error>>
{
    let cwd = tempfile::tempdir()?;
    let input = json!({"path": "../outside.txt"});

    assert!(!auto_approve(
        ApprovalMode::ApproveSafe,
        AskSource::Scope,
        "write",
        &input,
        Mode::WorkspaceWrite,
        true,
        cwd.path(),
    ));
    Ok(())
}
