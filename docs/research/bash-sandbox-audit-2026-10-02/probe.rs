// Disposable audit probes: assertions describe the defects observed on 2026-10-02.
// This executable also serves as a real, current-source broker worker.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use serde_json::json;
use vak_permission::{Decision, Mode, PermissionEngine};
use vak_tools::{Tool, ToolContext, bash::BashTool};
use vak_tools::sandbox::{SandboxMode, Seatbelt};
use vak_tools::sandbox_events::SandboxEventSink;
use vak_tools::sandbox::Sandbox;

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    if std::env::args().nth(1).as_deref() == Some("__tool_worker") {
        std::process::exit(rt.block_on(vak_tools::broker::worker_main()));
    }
    rt.block_on(run());
}

async fn run() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let tools = vak_tools::brokered_default_tools(std::env::current_exe().unwrap());
    let bash = tools.iter().find(|t| t.name() == "bash").unwrap();
    let write = tools.iter().find(|t| t.name() == "write").unwrap();
    let engine = PermissionEngine::from_rule_strings(&["+Bash(printf *)".into()]).unwrap();
    for command in ["printf '%s' \"$(touch hidden-effect)\"", "printf '%s' \"`touch hidden-effect`\""] {
        let decision = engine.evaluate("bash", &json!({"command":command}), Mode::WorkspaceWrite, &workspace);
        assert_eq!(decision, Decision::Allow);
        println!("quoted substitution: {:?} => {:?}", command, decision);
    }
    std::fs::write(workspace.join(".env"), "SYNTHETIC_AUDIT_MARKER").unwrap();
    std::fs::create_dir(workspace.join(".vak")).unwrap();
    std::fs::write(workspace.join(".vak/permissions.local.toml"), "original").unwrap();
    let (sink, _) = SandboxEventSink::new_with_id("audit-control".into());
    let mut native = ToolContext::new(workspace.clone()).with_sandbox_sink(sink);
    native.sandbox = Some(Arc::new(Seatbelt::new(SandboxMode::WorkspaceWrite, &workspace)));
    let allowed = "printf '%s' \"$(touch hidden-effect)\"";
    let out = bash.execute(&json!({"command":allowed}), &native).await;
    assert!(!out.is_error, "{}", out.content);
    assert!(workspace.join("hidden-effect").exists());
    println!("allowed Bash pattern: shell substitution executed its hidden file effect");
    #[cfg(target_os = "macos")]
    {
        let out = bash.execute(&json!({"command":"cat .e?v; printf changed > .vak/permissions.local.toml"}), &native).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("SYNTHETIC_AUDIT_MARKER"));
        assert_eq!(std::fs::read_to_string(workspace.join(".vak/permissions.local.toml")).unwrap(), "changed");
        println!("Seatbelt control files: indirect read and learned-policy mutation succeeded");
        let other = fixture.path().join("other-application.txt");
        std::fs::write(&other, "original").unwrap();
        let out = bash.execute(&json!({"command":format!("printf overwritten > {}", quoted(&other))}), &native).await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "overwritten");
        println!("Seatbelt host temp: unrelated sibling file overwritten");
    }
    let mut ctx = ToolContext::new(workspace.clone());
    #[cfg(target_os = "macos")]
    { ctx.sandbox = native.sandbox.clone(); }
    let cancel = ctx.cancel.clone();
    tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(500)).await; cancel.cancel(); });
    let started = Instant::now();
    let out = tokio::time::timeout(Duration::from_secs(5), bash.execute(&json!({"command":"echo $$ > cancelled-pid; sleep 2; printf survived > after-cancel.txt"}), &ctx)).await.unwrap();
    assert!(out.is_error && out.content.contains("cancelled"), "{}", out.content);
    println!("broker cancellation returned at {}ms", started.elapsed().as_millis());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(workspace.join("after-cancel.txt").exists());
    println!("broker cancellation: sandboxed shell wrote a file AFTER cancellation returned");

    let direct = ToolContext::new(workspace.clone());
    let started = Instant::now();
    let out = BashTool.execute(&json!({"command":"sleep 2 &", "timeout_ms":1000}), &direct).await;
    assert!(!out.is_error, "{}", out.content);
    assert!(started.elapsed() >= Duration::from_millis(1800));
    println!("background inherited pipe: 1000ms timeout returned SUCCESS at {}ms", started.elapsed().as_millis());
    let started = Instant::now();
    let out = BashTool.execute(&json!({"command":"(sleep 2; printf survived > after-success.txt) >/dev/null 2>&1 &", "timeout_ms":1000}), &direct).await;
    assert!(!out.is_error);
    println!("detached ordinary background: tool returned at {}ms", started.elapsed().as_millis());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(workspace.join("after-success.txt").exists());
    println!("background descendants: file written AFTER tool reported success");

    // Docker command-scoped policy leaves file-tool workers on the host.
    let docker = Arc::new(vak_sandbox::docker::DockerSandbox::new(SandboxMode::WorkspaceWrite, Some("alpine:3.20".into()), &workspace));
    let mut docker_ctx = ToolContext::new(workspace.clone());
    docker_ctx.sandbox = Some(docker.clone());
    #[cfg(unix)]
    {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let protected = tempfile::Builder::new().prefix("vak-audit-synthetic-").tempdir_in(home).unwrap();
        let victim = protected.path().join("victim.txt");
        std::fs::write(&victim, "original").unwrap();
        let out = bash.execute(&json!({"command":format!("ln -s {} allowed.txt.vak-tmp", quoted(&victim))}), &docker_ctx).await;
        assert!(!out.is_error, "{}", out.content);
        let args = json!({"path":"allowed.txt", "content":"overwritten-by-host-worker"});
        assert_eq!(PermissionEngine::default().evaluate("write", &args, Mode::WorkspaceWrite, &workspace), Decision::Allow);
        let out = write.execute(&args, &docker_ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "overwritten-by-host-worker");
        println!("Docker file-tool boundary: predictable temporary symlink overwrote HOME fixture");
        std::fs::remove_file(workspace.join("allowed.txt")).unwrap();
    }
    if std::env::args().any(|arg| arg == "--docker") {
        std::fs::create_dir(workspace.join("subfolder")).unwrap();
        let out = bash.execute(&json!({"command":"pwd", "cwd":"subfolder"}), &docker_ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains(&format!("[stdout]\n{}\n", workspace.display())), "{}", out.content);
        println!("live Docker broker: cwd=subfolder ignored; command ran at workspace root");
        let cid_file = workspace.join("audit-container-id");
        let started = Instant::now();
        let out = bash.execute(&json!({"command":"cat /etc/hostname > audit-container-id; sleep 3; printf survived > docker-after-timeout.txt", "timeout_ms":1000}), &docker_ctx).await;
        assert!(out.is_error && out.content.contains("timed out"), "{}", out.content);
        println!("live Docker timeout returned at {}ms", started.elapsed().as_millis());
        tokio::time::sleep(Duration::from_millis(3500)).await;
        let survived = workspace.join("docker-after-timeout.txt").exists();
        // Exact fixture container only; never clean up another task's container.
        if let Ok(id) = std::fs::read_to_string(cid_file) {
            let _ = std::process::Command::new("docker").args(["rm", "-f", id.trim()]).output();
        }
        assert!(survived);
        println!("live Docker timeout: container wrote a file AFTER host timeout returned");
        match vak_sandbox::docker::DockerTaskSandbox::create(SandboxMode::WorkspaceWrite, Some("alpine:3.20".into()), &workspace, None) {
            Ok(value) => {
                println!("Docker retained task creation: succeeded ({})", value.name());
                let task_sandbox = Arc::new(value);
                let mut task_ctx = ToolContext::new(workspace.clone());
                task_ctx.sandbox = Some(task_sandbox.clone());
                let cancel = task_ctx.cancel.clone();
                tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(500)).await; cancel.cancel(); });
                let out = tokio::time::timeout(Duration::from_secs(5), bash.execute(&json!({"command":"sleep 2; printf survived > docker-task-after-cancel.txt"}), &task_ctx)).await.unwrap();
                assert!(out.is_error && out.content.contains("cancelled"), "{}", out.content);
                tokio::time::sleep(Duration::from_millis(2200)).await;
                assert!(workspace.join("docker-task-after-cancel.txt").exists());
                println!("live Docker task cancellation: docker exec wrote a file AFTER cancellation returned");
                drop(task_ctx);
                drop(task_sandbox);
            },
            Err(error) => println!("Docker retained task creation: {}", error),
        }
    }
}
