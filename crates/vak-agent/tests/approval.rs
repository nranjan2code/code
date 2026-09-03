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

/// An approver that refuses without consulting anyone must say so.
///
/// `AutoDeny` used to produce "denied by user: ..." on unattended
/// surfaces. No user was asked — the gate was never put to a person — and
/// the wording sent the model looking for a substitute tool and the
/// operator looking for a decision nobody made. The refusal is the same;
/// only the attribution is corrected.
#[test]
fn an_unanswerable_approver_does_not_blame_a_user() {
    assert!(
        !vak_agent::Approver::answerable(&vak_agent::AutoDeny),
        "AutoDeny consults no one and must not claim to be answerable"
    );
    assert!(
        vak_agent::Approver::answerable(&vak_agent::AutoApprove),
        "AutoApprove resolves gates, so it is answerable"
    );
}
