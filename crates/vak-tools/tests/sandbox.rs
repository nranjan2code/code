#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use tempfile::tempdir;

use vak_tools::bash::BashTool;
use vak_tools::context::ToolContext;
use vak_tools::sandbox::{Sandbox, SandboxMode, Seatbelt};
use vak_tools::{Tool, ToolOutput};

fn ctx_with(cwd: &std::path::Path, mode: SandboxMode) -> ToolContext {
    ToolContext {
        cwd: cwd.to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        sandbox: Some(Arc::new(Seatbelt::new(mode, cwd))),
        sandbox_sink: None,
        agent_id: None,
        trace: None,
        new_documents: Vec::new(),
    }
}

async fn run(ctx: &ToolContext, cmd: &str) -> ToolOutput {
    BashTool.execute(&json!({"command": cmd}), ctx).await
}

#[test]
fn seatbelt_read_only_profile_denies_all_writes() {
    let dir = tempdir().unwrap();
    let sb = Seatbelt::new(SandboxMode::ReadOnly, dir.path());
    let p = sb.profile();
    assert!(p.contains("(deny default)"));
    assert!(p.contains("(allow file-read* (subpath"));
    assert!(!p.contains("(allow file-read*)\n"));
    assert!(
        !p.contains("file-write*"),
        "read-only must grant no write paths"
    );
}

#[test]
fn seatbelt_workspace_write_profile_scopes_to_cwd_without_host_temp() {
    let dir = tempdir().unwrap();
    let sb = Seatbelt::new(SandboxMode::WorkspaceWrite, dir.path());
    let p = sb.profile();
    let canonical = dir.path().canonicalize().unwrap();
    assert!(p.contains(&format!("(subpath \"{}\")", canonical.display())));
    assert!(!p.contains("(subpath \"/private/tmp\")"));
    assert!(p.contains("(subpath \"/dev/null\")"));
}

#[test]
fn wrap_quotes_command_and_embeds_profile() {
    let dir = tempdir().unwrap();
    let sb = Seatbelt::new(SandboxMode::ReadOnly, dir.path());
    let wrapped = sb.wrap("echo 'hello world' && ls");
    assert!(wrapped.starts_with("sandbox-exec -p '"));
    assert!(
        wrapped.ends_with("-- sh -c 'echo '\\''hello world'\\'' && ls'"),
        "inner single quotes must be POSIX-escaped, got: {wrapped}"
    );
    let profile_start = wrapped.find("(version 1)").expect("profile embedded");
    let profile_end = wrapped
        .rfind("' -- sh -c")
        .expect("command follows profile");
    assert!(
        !wrapped[profile_start..profile_end].contains('\''),
        "profile must be quote-stripped before embedding"
    );
}

#[test]
fn off_mode_passes_command_through() {
    let dir = tempdir().unwrap();
    let sb = Seatbelt::new(SandboxMode::Off, dir.path());
    assert_eq!(sb.wrap("echo hi"), "echo hi");
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn seatbelt_read_only_blocks_file_writes_but_allows_reads() {
    let dir = tempdir().unwrap();
    let ctx = ctx_with(dir.path(), SandboxMode::ReadOnly);

    let out = run(&ctx, "cat /etc/hostname >/dev/null; echo read-ok").await;
    assert!(
        !out.is_error,
        "reads must work under read-only sandbox: {}",
        out.content
    );

    let out = run(&ctx, "echo blocked > ./should-not-exist.txt").await;
    assert!(out.is_error, "writes must be denied");
    assert!(
        !dir.path().join("should-not-exist.txt").exists(),
        "denied write must not have created the file"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn seatbelt_workspace_write_allows_inside_cwd() {
    let dir = tempdir().unwrap();
    let ctx = ctx_with(dir.path(), SandboxMode::WorkspaceWrite);

    let out = run(&ctx, "echo inside > ./inside.txt && cat ./inside.txt").await;
    assert!(
        !out.is_error,
        "in-workspace writes must succeed: {}",
        out.content
    );
    assert!(out.content.contains("inside"));
    assert!(dir.path().join("inside.txt").exists());
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn seatbelt_workspace_write_blocks_outside_paths() {
    let dir = tempdir().unwrap();
    let ctx = ctx_with(dir.path(), SandboxMode::WorkspaceWrite);

    let outside = "$HOME/vak-sb-escape-test.txt";
    let out = run(&ctx, &format!("echo escape > {outside}")).await;
    assert!(out.is_error, "writes outside the workspace must be denied");
    assert!(
        out.content.contains("Operation not permitted"),
        "{}",
        out.content
    );
    let home = std::env::var("HOME").unwrap_or_default();
    assert!(
        !std::path::Path::new(&home)
            .join("vak-sb-escape-test.txt")
            .exists(),
        "the escape file must not exist"
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn seatbelt_blocks_home_reads_outside_workspace() {
    let dir = tempdir().unwrap();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let protected = tempfile::Builder::new()
        .prefix("vak-seatbelt-read-")
        .tempdir_in(home)
        .unwrap();
    std::fs::write(protected.path().join("secret"), "not-visible").unwrap();
    let ctx = ctx_with(dir.path(), SandboxMode::WorkspaceWrite);

    let out = run(
        &ctx,
        &format!("cat {}", protected.path().join("secret").display()),
    )
    .await;
    assert!(out.is_error);
    assert!(!out.content.contains("not-visible"));
}

// ── DenySandbox runtime behavior ───────────────────────────────────────

/// When the sandbox backend is unavailable or the mode refuses to engage,
/// DenySandbox must make BashTool surface an error (exit 126) rather than
/// silently running the command unsandboxed.
#[tokio::test]
async fn deny_sandbox_returns_error_at_runtime() {
    let dir = tempdir().unwrap();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        sandbox: Some(std::sync::Arc::new(vak_tools::sandbox::DenySandbox::new(
            "unavailable in this configuration",
        ))),
        sandbox_sink: None,
        agent_id: None,
        trace: None,
        new_documents: Vec::new(),
    };

    let out = BashTool
        .execute(&serde_json::json!({"command": "echo leaked"}), &ctx)
        .await;
    assert!(out.is_error, "denied command must report an error");
    assert!(
        out.content.contains("126"),
        "exit code 126 expected: {out:?}"
    );
    assert!(
        !out.content.contains("leaked"),
        "the denied command must never have executed"
    );
}

/// With no sandbox at all (FullAccess), bash writes freely — this is the
/// explicit-trust escape hatch and must keep working.
#[tokio::test]
async fn off_mode_allows_unrestricted_writes() {
    let dir = tempdir().unwrap();
    let ctx = ToolContext {
        cwd: dir.path().to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        sandbox: None,
        sandbox_sink: None,
        agent_id: None,
        trace: None,
        new_documents: Vec::new(),
    };

    let out = run(
        &ctx,
        "echo unrestricted > ./freedom.txt && cat ./freedom.txt",
    )
    .await;
    assert!(!out.is_error, "off-mode must allow writes: {}", out.content);
    assert!(dir.path().join("freedom.txt").exists());
}

/// `read_only_variant()` on a WorkspaceWrite Seatbelt must produce a profile
/// that grants no file-write* entitlements — the trait-level contract that
/// turns that want a tighter sandbox inherit this automatically.
#[test]
fn read_only_variant_strips_all_write_entitlements() {
    let dir = tempdir().unwrap();
    let sb = Seatbelt::new(SandboxMode::WorkspaceWrite, dir.path());
    let ro = Sandbox::read_only_variant(&sb).expect("read-only variant");
    let wrapped = ro.wrap("true");
    let profile_start = wrapped.find("(version 1)").expect("profile present");
    let profile_end = wrapped
        .rfind("' -- sh -c")
        .expect("command follows profile");
    let profile = &wrapped[profile_start..profile_end];
    assert!(
        !profile.contains("file-write*"),
        "read-only variant must not grant writes: {profile}"
    );
}
