#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use vak_core::sandbox_docker::DockerSandbox;
use vak_tools::sandbox::SandboxMode;

/// Run a command exactly like BashTool does: the sandbox wrapper goes
/// through one more shell layer.
fn run_wrapped(wrapped: &str, _timeout: Duration) -> (bool, String) {
    let out = Command::new("sh")
        .arg("-c")
        .arg(wrapped)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match out {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.success(), text)
        }
        Err(e) => (false, format!("spawn failed: {e}")),
    }
}

fn ensure_image(image: &str) {
    let have = Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !have {
        let pulled = Command::new("docker")
            .args(["pull", "-q", image])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(pulled, "docker pull {image} failed");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn docker_backend_executes_mounts_and_denies_network() {
    if !DockerSandbox::available() {
        println!("skipping: no reachable docker daemon");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ws: PathBuf = dir.path().to_path_buf();
    let sb = DockerSandbox::new(SandboxMode::WorkspaceWrite, None, &ws);
    ensure_image("alpine:3.20");

    // 1. Execution inside the container.
    let (ok, out) = run_wrapped(
        &sb.wrap_command("echo container-alive"),
        Duration::from_secs(60),
    );
    assert!(ok, "container exec failed: {out}");
    assert!(out.contains("container-alive"), "{out}");

    // 2. Writes land in the HOST workspace through the bind mount.
    let (ok, out) = run_wrapped(
        &sb.wrap_command("echo from-container > mounted.txt"),
        Duration::from_secs(60),
    );
    assert!(ok, "write failed: {out}");
    let written = std::fs::read_to_string(ws.join("mounted.txt")).unwrap();
    assert_eq!(written.trim(), "from-container");

    // 3. Network is denied (--network none); busybox wget must fail fast.
    let (ok, out) = run_wrapped(
        &sb.wrap_command(
            "wget -q -T 3 -O /dev/null http://example.com >/dev/null 2>&1 && echo NETUP || echo NETBLOCKED",
        ),
        Duration::from_secs(60),
    );
    assert!(ok);
    assert!(out.contains("NETBLOCKED"), "network must be denied: {out}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn docker_readonly_mode_blocks_workspace_writes() {
    if !DockerSandbox::available() {
        println!("skipping: no reachable docker daemon");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    std::fs::write(ws.join("existing.txt"), "keep").unwrap();
    ensure_image("alpine:3.20");

    let sb = DockerSandbox::new(SandboxMode::ReadOnly, None, &ws);
    let (ok, out) = run_wrapped(
        &sb.wrap_command("(echo x > existing.txt && echo WROTE) || echo ROENFORCED"),
        Duration::from_secs(60),
    );
    assert!(ok);
    assert!(
        out.contains("ROENFORCED"),
        "read-only mount must block writes: {out}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("existing.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn effective_sandbox_name_reports_docker_when_selected() {
    // Config-level selection flows through Core without needing a daemon.
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join(".vakcoder");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("config.toml"),
        "[sandbox]\nbackend = \"docker\"\nimage = \"alpine:3.20\"\n",
    )
    .unwrap();
    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    core.set_permission_mode(vak_config::PermissionMode::WorkspaceWrite);
    let name = core.effective_sandbox_name();
    assert!(name.starts_with("docker"), "got {name}");
}
