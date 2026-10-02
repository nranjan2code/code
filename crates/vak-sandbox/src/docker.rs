//! Docker execution backend (docs/design/25-docker-sandbox.md): runs bash
//! commands inside a throwaway container with the workspace bind-mounted at
//! its real absolute path, so host-side file tools and container-side shell
//! see identical paths.
//!
//! Posture: no network, capped CPU/memory, read-only rootfs in ReadOnly
//! mode, and a disposable writable rootfs in WorkspaceWrite mode. Only BashTool consults the sandbox seam today — file tools keep
//! their host-side confinement via the permission engine.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backend::SandboxMode;

pub const DEFAULT_IMAGE: &str = "alpine:3.20";
const MEMORY_CAP: &str = "2g";
const CPUS_CAP: &str = "2";
const PIDS_CAP: &str = "256";
const STORAGE_CAP: &str = "4g";
static TASK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A task-scoped Docker environment. Its writable container layer survives
/// multiple commands and is destroyed explicitly or when the owner drops it.
/// The workspace is the only host path it can see.
pub struct DockerTaskEnvironment {
    container_id: String,
    workspace: PathBuf,
}

impl DockerTaskEnvironment {
    pub fn create(
        mode: SandboxMode,
        image: Option<String>,
        workspace: &Path,
        broker_socket: Option<&Path>,
    ) -> Result<Self, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|e| format!("workspace is not accessible: {e}"))?;
        let image = image.unwrap_or_else(|| DEFAULT_IMAGE.to_string());
        let name = format!(
            "vak-task-{}-{}-{}",
            std::process::id(),
            vak_session::ids::ExecutionId::new(),
            TASK_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let ws = workspace.display().to_string();
        let mount = match mode {
            SandboxMode::ReadOnly => format!("{ws}:{ws}:ro"),
            SandboxMode::WorkspaceWrite | SandboxMode::Off => format!("{ws}:{ws}"),
        };
        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--rm".to_string(),
            "--name".to_string(),
            name,
            "--network".to_string(),
            "none".to_string(),
            "--memory".to_string(),
            MEMORY_CAP.to_string(),
            "--cpus".to_string(),
            CPUS_CAP.to_string(),
            "--pids-limit".to_string(),
            PIDS_CAP.to_string(),
            "--cap-drop".to_string(),
            "ALL".to_string(),
            "--security-opt".to_string(),
            "no-new-privileges".to_string(),
            "-v".to_string(),
            mount,
        ];
        args.extend([
            "--storage-opt".to_string(),
            format!("size={STORAGE_CAP}"),
            "--tmpfs".to_string(),
            "/tmp:rw,nosuid,nodev,noexec,size=512m".to_string(),
        ]);
        if mode == SandboxMode::ReadOnly {
            args.push("--read-only".to_string());
        }
        if let Some(socket) = broker_socket
            && socket.exists()
        {
            args.extend([
                "-v".to_string(),
                format!("{}:{}", socket.display(), socket.display()),
                "-e".to_string(),
                format!("VAK_AGENT_NETWORK_SOCKET={}", socket.display()),
            ]);
        }
        args.extend([
            "-w".to_string(),
            ws,
            image,
            "sh".to_string(),
            "-c".to_string(),
            "while :; do sleep 3600; done".to_string(),
        ]);
        let output = Command::new("docker")
            .args(args)
            .output()
            .map_err(|e| format!("docker task environment spawn failed: {e}"))?;
        if !output.status.success() {
            return Err(format_docker_error(
                "docker task environment create failed",
                &output,
            ));
        }
        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if container_id.is_empty() {
            return Err("docker task environment returned no container id".into());
        }
        Ok(Self {
            container_id,
            workspace,
        })
    }

    pub fn exec(&self, command: &str) -> Result<Output, String> {
        Command::new("docker")
            .args(["exec", "-w"])
            .arg(&self.workspace)
            .arg(&self.container_id)
            .args(["sh", "-c", command])
            .output()
            .map_err(|e| format!("docker task environment exec failed: {e}"))
    }

    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    pub fn destroy(&self) -> Result<(), String> {
        let output = Command::new("docker")
            .args(["rm", "-f", &self.container_id])
            .output()
            .map_err(|e| format!("docker task environment cleanup failed: {e}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format_docker_error(
                "docker task environment cleanup failed",
                &output,
            ))
        }
    }
}

/// Sandbox adapter for a task environment whose root layer survives each
/// command in the same agent turn.
pub struct DockerTaskSandbox {
    environment: Arc<DockerTaskEnvironment>,
    mode: SandboxMode,
    workspace: PathBuf,
    image: Option<String>,
    broker_socket: Option<PathBuf>,
}

impl DockerTaskSandbox {
    pub fn create(
        mode: SandboxMode,
        image: Option<String>,
        workspace: &Path,
        broker_socket: Option<&Path>,
    ) -> Result<Self, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|e| format!("workspace is not accessible: {e}"))?;
        Ok(Self {
            environment: Arc::new(DockerTaskEnvironment::create(
                mode,
                image.clone(),
                &workspace,
                broker_socket,
            )?),
            mode,
            workspace,
            image,
            broker_socket: broker_socket.map(Path::to_path_buf),
        })
    }
}

impl crate::backend::Sandbox for DockerTaskSandbox {
    fn name(&self) -> &str {
        match self.mode {
            SandboxMode::ReadOnly => "docker-task-ro",
            SandboxMode::WorkspaceWrite | SandboxMode::Off => "docker-task",
        }
    }

    fn wrap(&self, command: &str) -> String {
        format!(
            "docker exec -w {} {} sh -c {}",
            shell_quote(&self.workspace.display().to_string()),
            shell_quote(self.environment.container_id()),
            shell_quote(command),
        )
    }

    fn target(&self) -> crate::backend::SandboxTarget {
        crate::backend::SandboxTarget::ToolCommand
    }

    fn read_only_variant(&self) -> Option<Arc<dyn crate::backend::Sandbox>> {
        if self.mode == SandboxMode::ReadOnly {
            return Some(Arc::new(Self {
                environment: self.environment.clone(),
                mode: SandboxMode::ReadOnly,
                workspace: self.workspace.clone(),
                image: self.image.clone(),
                broker_socket: self.broker_socket.clone(),
            }));
        }
        Self::create(
            SandboxMode::ReadOnly,
            self.image.clone(),
            &self.workspace,
            self.broker_socket.as_deref(),
        )
        .ok()
        .map(|sandbox| Arc::new(sandbox) as Arc<dyn crate::backend::Sandbox>)
    }
}

impl Drop for DockerTaskEnvironment {
    fn drop(&mut self) {
        let _ = self.destroy();
    }
}

fn format_docker_error(prefix: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}: {stderr}")
    }
}

pub struct DockerSandbox {
    mode: SandboxMode,
    image: String,
    workspace: PathBuf,
}

impl DockerSandbox {
    pub fn new(mode: SandboxMode, image: Option<String>, workspace: &Path) -> Self {
        DockerSandbox {
            mode,
            image: image.unwrap_or_else(|| DEFAULT_IMAGE.to_string()),
            workspace: workspace.to_path_buf(),
        }
    }

    /// One-shot probe with process-lifetime caching; `docker info` answers
    /// whether a daemon is reachable without pulling anything.
    pub fn available() -> bool {
        static OK: OnceLock<bool> = OnceLock::new();
        *OK.get_or_init(|| {
            std::process::Command::new("docker")
                .arg("info")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
    }

    /// The command BashTool will run, as `docker run … sh -c <quoted>`.
    pub fn wrap_command(&self, command: &str) -> String {
        let ws = self.workspace.display().to_string();
        let ro_suffix = match self.mode {
            SandboxMode::ReadOnly => ":ro",
            SandboxMode::WorkspaceWrite | SandboxMode::Off => "",
        };
        let mount = shell_quote(&format!("{ws}:{ws}{ro_suffix}"));
        let workdir = shell_quote(&ws);
        let image = shell_quote(&self.image);
        let rootfs = match self.mode {
            SandboxMode::ReadOnly => "--read-only ",
            // This layer is private to the throwaway container and disappears
            // with --rm, so package installation cannot modify the host.
            SandboxMode::WorkspaceWrite | SandboxMode::Off => "",
        };
        format!(
            "docker run --rm --network none {rootfs}--tmpfs /tmp:rw,nosuid,size=512m \
             --memory {MEMORY_CAP} --cpus {CPUS_CAP} --pids-limit 256 --cap-drop ALL \
             --security-opt no-new-privileges -v {mount} -w {workdir} {image} sh -c {cmd}",
            cmd = shell_quote(command),
        )
    }
}

impl crate::backend::Sandbox for DockerSandbox {
    fn name(&self) -> &str {
        match self.mode {
            SandboxMode::ReadOnly => "docker-ro",
            SandboxMode::WorkspaceWrite | SandboxMode::Off => "docker",
        }
    }

    fn wrap(&self, command: &str) -> String {
        self.wrap_command(command)
    }

    fn target(&self) -> crate::backend::SandboxTarget {
        crate::backend::SandboxTarget::ToolCommand
    }

    fn read_only_variant(&self) -> Option<Arc<dyn crate::backend::Sandbox>> {
        Some(Arc::new(DockerSandbox::new(
            SandboxMode::ReadOnly,
            Some(self.image.clone()),
            &self.workspace,
        )))
    }
}

/// Single-argument POSIX shell quoting, sufficient for the inner layer of
/// the two-layer quoting discipline (BashTool quotes the outer layer).
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    // Trait method access inside this module.
    use crate::backend::Sandbox as _;

    #[test]
    fn wrap_embeds_workspace_mount_and_quotes_inner_command() {
        let sb = DockerSandbox::new(
            SandboxMode::WorkspaceWrite,
            Some("test-image:9".into()),
            Path::new("/tmp/ws"),
        );
        let wrapped = sb.wrap("echo 'hello world' && ls");
        assert!(wrapped.starts_with("docker run --rm --network none "));
        assert!(!wrapped.contains("--read-only"));
        assert!(wrapped.contains("-v '/tmp/ws:/tmp/ws'"));
        assert!(wrapped.contains("-w '/tmp/ws'"));
        assert!(wrapped.contains("--memory 2g"));
        assert!(wrapped.contains("--pids-limit 256"));
        assert!(wrapped.contains("--cap-drop ALL"));
        assert!(wrapped.contains("'test-image:9'"));
        assert!(
            wrapped.ends_with("sh -c 'echo '\\''hello world'\\'' && ls'"),
            "inner command must be single-quoted: {wrapped}"
        );
    }

    #[test]
    fn readonly_mode_mounts_ro_and_adds_tmp() {
        let sb = DockerSandbox::new(SandboxMode::ReadOnly, None, Path::new("/tmp/ws"));
        let wrapped = sb.wrap("true");
        assert!(wrapped.contains("'/tmp/ws:/tmp/ws:ro'"));
        assert!(wrapped.contains("--network none --read-only --tmpfs"));
        assert!(wrapped.contains("--tmpfs /tmp:rw,nosuid,size=512m"));
        assert!(wrapped.contains(DEFAULT_IMAGE));
    }
}
