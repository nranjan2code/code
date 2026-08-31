//! Docker execution backend (docs/design/25-docker-sandbox.md): runs bash
//! commands inside a throwaway container with the workspace bind-mounted at
//! its real absolute path, so host-side file tools and container-side shell
//! see identical paths.
//!
//! Posture: no network, capped CPU/memory, read-only rootfs in ReadOnly
//! mode. Only BashTool consults the sandbox seam today — file tools keep
//! their host-side confinement via the permission engine.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;

use vak_tools::sandbox::SandboxMode;

pub const DEFAULT_IMAGE: &str = "alpine:3.20";
const MEMORY_CAP: &str = "2g";
const CPUS_CAP: &str = "2";

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
        format!(
            "docker run --rm --network none --read-only --tmpfs /tmp:rw,nosuid,size=512m \
             --memory {MEMORY_CAP} --cpus {CPUS_CAP} --pids-limit 256 --cap-drop ALL \
             --security-opt no-new-privileges -v {mount} -w {workdir} {image} sh -c {cmd}",
            cmd = shell_quote(command),
        )
    }
}

impl vak_tools::sandbox::Sandbox for DockerSandbox {
    fn name(&self) -> &str {
        match self.mode {
            SandboxMode::ReadOnly => "docker-ro",
            SandboxMode::WorkspaceWrite | SandboxMode::Off => "docker",
        }
    }

    fn wrap(&self, command: &str) -> String {
        self.wrap_command(command)
    }

    fn target(&self) -> vak_tools::sandbox::SandboxTarget {
        vak_tools::sandbox::SandboxTarget::ToolCommand
    }

    fn read_only_variant(&self) -> Option<Arc<dyn vak_tools::sandbox::Sandbox>> {
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
    use vak_tools::sandbox::Sandbox as _;

    #[test]
    fn wrap_embeds_workspace_mount_and_quotes_inner_command() {
        let sb = DockerSandbox::new(
            SandboxMode::WorkspaceWrite,
            Some("test-image:9".into()),
            Path::new("/tmp/ws"),
        );
        let wrapped = sb.wrap("echo 'hello world' && ls");
        assert!(wrapped.starts_with("docker run --rm --network none --read-only"));
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
        assert!(wrapped.contains("--tmpfs /tmp:rw,nosuid,size=512m"));
        assert!(wrapped.contains(DEFAULT_IMAGE));
    }
}
