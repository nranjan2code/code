use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    Off,
    ReadOnly,
    WorkspaceWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxTarget {
    WorkerProcess,
    ToolCommand,
}

pub trait Sandbox: Send + Sync {
    fn name(&self) -> &str;
    fn wrap(&self, command: &str) -> String;

    fn target(&self) -> SandboxTarget {
        SandboxTarget::WorkerProcess
    }

    fn read_only_variant(&self) -> Option<Arc<dyn Sandbox>> {
        None
    }

    /// This sandbox, plus listening on a port: for a dev-server preview the
    /// user configured, and nothing else. Outbound connections stay denied.
    /// `None` when the backend cannot grant it, and the caller keeps the
    /// closed sandbox.
    fn listening_variant(&self) -> Option<Arc<dyn Sandbox>> {
        None
    }
}

pub fn no_sandbox() -> Option<Arc<dyn Sandbox>> {
    None
}

#[derive(Debug, Clone)]
pub struct DenySandbox {
    reason: String,
}

impl DenySandbox {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Sandbox for DenySandbox {
    fn name(&self) -> &str {
        "unavailable-deny"
    }

    fn wrap(&self, _command: &str) -> String {
        format!(
            "echo {} >&2; exit 126",
            shell_quote(&format!("vak sandbox unavailable: {}", self.reason))
        )
    }

    fn read_only_variant(&self) -> Option<Arc<dyn Sandbox>> {
        Some(Arc::new(self.clone()))
    }
}

#[derive(Debug, Clone)]
pub struct Seatbelt {
    pub mode: SandboxMode,
    pub read_paths: Vec<PathBuf>,
    pub write_paths: Vec<PathBuf>,
    /// Task copies must not inherit the broad host temp write allowances.
    pub allow_host_temp: bool,
    /// May accept connections on a port (`Sandbox::listening_variant`).
    /// Seatbelt cannot limit this to loopback: a server that binds every
    /// interface is reachable from the LAN, as it would be run by hand.
    /// Outbound connections stay denied.
    pub allow_listen: bool,
}

impl Seatbelt {
    pub fn new(mode: SandboxMode, cwd: &Path) -> Self {
        Self::build(mode, cwd, true)
    }

    /// Contain a retained task copy, including when the original workspace
    /// itself lives below a host temp directory.
    pub fn task_copy(mode: SandboxMode, cwd: &Path) -> Self {
        Self::build(mode, cwd, false)
    }

    fn build(mode: SandboxMode, cwd: &Path, allow_host_temp: bool) -> Self {
        let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let mut read_paths: Vec<PathBuf> = [
            "/System",
            "/usr",
            "/bin",
            "/sbin",
            "/Library",
            "/Applications",
            "/opt",
            "/private/etc",
            "/private/var/db",
            "/dev",
        ]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect();
        read_paths.push(canonical.clone());
        if let Ok(executable) = std::env::current_exe()
            && let Some(parent) = executable.parent()
        {
            read_paths.push(parent.to_path_buf());
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for relative in [
                ".cargo/bin",
                ".cargo/git",
                ".cargo/registry",
                ".rustup",
                ".local/bin",
            ] {
                let path = home.join(relative);
                if path.exists() {
                    read_paths.push(path);
                }
            }
        }
        if allow_host_temp {
            for path in Self::temp_write_paths() {
                let path = PathBuf::from(path);
                if path.exists() && !read_paths.contains(&path) {
                    read_paths.push(path);
                }
            }
        }
        let write_paths = match mode {
            SandboxMode::WorkspaceWrite => vec![canonical],
            _ => Vec::new(),
        };
        Seatbelt {
            mode,
            read_paths,
            write_paths,
            allow_host_temp,
            allow_listen: false,
        }
    }

    /// Extra write allowances that keep real-world tooling working under
    /// workspace-write: the OS per-user temp/cache areas (macOS TMPDIR lives
    /// under /var/folders, NOT /tmp) and /tmp itself. Without these, any
    /// test suite using tempfile/std::env::temp_dir fails under the sandbox
    /// — found by dogfooding `cargo test` through the agent.
    pub fn temp_write_paths() -> Vec<String> {
        let mut out = vec![
            "/private/tmp".to_string(),
            "/private/var/tmp".to_string(),
            "/private/var/folders".to_string(),
            "/tmp".to_string(),
        ];
        if let Some(tmpdir) = std::env::var_os("TMPDIR") {
            let p = std::path::PathBuf::from(&tmpdir).display().to_string();
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }

    pub fn profile(&self) -> String {
        let mut p = String::from("(version 1)\n");
        match self.mode {
            SandboxMode::Off => return "(version 1)(allow default)".to_string(),
            SandboxMode::ReadOnly => {
                p.push_str("(deny default)\n");
                self.append_read_allowances(&mut p);
                p.push_str("(allow process-exec)\n");
                p.push_str("(allow process-fork)\n");
                p.push_str("(allow sysctl-read)\n");
                p.push_str("(allow mach-lookup)\n");
                p.push_str("(allow iokit-get-properties)\n");
            }
            SandboxMode::WorkspaceWrite => {
                p.push_str("(deny default)\n");
                self.append_read_allowances(&mut p);
                p.push_str("(allow process-exec)\n");
                p.push_str("(allow process-fork)\n");
                p.push_str("(allow sysctl-read)\n");
                p.push_str("(allow mach-lookup)\n");
                p.push_str("(allow iokit-get-properties)\n");
                for path in &self.write_paths {
                    p.push_str(&format!(
                        "(allow file-write* (subpath {}))\n",
                        sbpl_quote(&path.display().to_string())
                    ));
                }
                if self.allow_host_temp {
                    for tmp in Self::temp_write_paths() {
                        p.push_str(&format!(
                            "(allow file-write* (subpath {}))\n",
                            sbpl_quote(&tmp)
                        ));
                    }
                }
                for dev in ["/dev/null", "/dev/urandom"] {
                    p.push_str(&format!(
                        "(allow file-write* (subpath {}))\n",
                        sbpl_quote(dev)
                    ));
                }
            }
        }
        if self.allow_listen {
            p.push_str("(allow network-bind (local ip \"localhost:*\"))\n");
            p.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
        }
        p
    }

    fn append_read_allowances(&self, profile: &mut String) {
        profile.push_str("(allow file-read-metadata)\n");
        profile.push_str("(allow file-read* (literal \"/\"))\n");
        for path in &self.read_paths {
            profile.push_str(&format!(
                "(allow file-read* (subpath {}))\n",
                sbpl_quote(&path.display().to_string())
            ));
        }
    }
}

/// SBPL string literal: escape backslash and double quote so a path
/// containing quotes cannot break out of (or corrupt) the profile.
fn sbpl_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Sandbox for Seatbelt {
    fn name(&self) -> &str {
        "seatbelt"
    }

    fn wrap(&self, command: &str) -> String {
        if self.mode == SandboxMode::Off {
            return command.to_string();
        }
        // The profile itself goes through single-quote shell escaping —
        // stripping characters would silently alter the policy for paths
        // containing them.
        format!(
            "sandbox-exec -p {} -- sh -c {}",
            shell_quote(&self.profile()),
            shell_quote(command)
        )
    }

    fn read_only_variant(&self) -> Option<Arc<dyn Sandbox>> {
        Some(Arc::new(Seatbelt {
            mode: SandboxMode::ReadOnly,
            read_paths: self.read_paths.clone(),
            write_paths: Vec::new(),
            allow_host_temp: self.allow_host_temp,
            allow_listen: self.allow_listen,
        }))
    }

    fn listening_variant(&self) -> Option<Arc<dyn Sandbox>> {
        Some(Arc::new(Seatbelt {
            allow_listen: true,
            ..self.clone()
        }))
    }
}

fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn task_copy_profile_does_not_grant_host_temp_writes() {
        let copy = tempfile::tempdir().expect("task copy");
        let sandbox = Seatbelt::task_copy(SandboxMode::WorkspaceWrite, copy.path());
        let profile = sandbox.profile();
        let canonical = copy.path().canonicalize().expect("canonical copy");
        assert!(profile.contains(&format!(
            "(allow file-write* (subpath \"{}\"))",
            canonical.display()
        )));
        assert!(!profile.contains("(allow file-write* (subpath \"/private/tmp\"))"));
        assert!(!profile.contains("(allow file-write* (subpath \"/private/var/folders\"))"));
        assert!(!sandbox.read_paths.contains(&PathBuf::from("/private/tmp")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn task_copy_worker_can_write_copy_but_not_sibling_workspace() {
        let root = tempfile::tempdir().expect("test root");
        let copy = root.path().join("task-copy");
        let source = root.path().join("original-workspace");
        std::fs::create_dir_all(&copy).expect("copy directory");
        std::fs::create_dir_all(&source).expect("source directory");
        let sandbox = Seatbelt::task_copy(SandboxMode::WorkspaceWrite, &copy);
        let inside = copy.join("allowed.txt");
        let outside = source.join("forbidden.txt");
        let escaped = copy.join("outside-link");
        std::os::unix::fs::symlink(&source, &escaped).expect("link to original workspace");
        let allowed = sandbox.wrap(&format!(
            "echo allowed > {}",
            shell_quote(&inside.display().to_string())
        ));
        let denied = sandbox.wrap(&format!(
            "echo forbidden > {}",
            shell_quote(&outside.display().to_string())
        ));
        let denied_link = sandbox.wrap(&format!(
            "echo forbidden > {}",
            shell_quote(&escaped.join("through-link.txt").display().to_string())
        ));
        assert!(
            std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(allowed)
                .status()
                .expect("allowed command")
                .success()
        );
        assert!(
            !std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(denied)
                .status()
                .expect("denied command")
                .success()
        );
        assert!(
            !std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(denied_link)
                .status()
                .expect("symlink escape command")
                .success()
        );
        assert_eq!(
            std::fs::read_to_string(inside).expect("copy write"),
            "allowed\n"
        );
        assert!(!outside.exists());
        assert!(!source.join("through-link.txt").exists());
    }

    /// Measured against the real `sandbox-exec`: the closed profile refuses
    /// to listen, the listening variant accepts, and neither can connect out.
    #[cfg(target_os = "macos")]
    #[test]
    fn only_the_listening_variant_listens_and_neither_connects_out() {
        let python = |code: &str| format!("python3 -c '{code}'");
        let dir = tempfile::tempdir().expect("workspace");
        // From the workspace, as a preview runs: Python reads its working
        // directory on import, and the profile only admits the workspace.
        let run = |wrapped: String| {
            std::process::Command::new("sh")
                .arg("-c")
                .arg(wrapped)
                .current_dir(dir.path())
                .output()
                .expect("sh runs")
        };
        if !run("python3 --version".into()).status.success() {
            eprintln!("python3 unavailable; skipping");
            return;
        }
        let closed = Seatbelt::new(SandboxMode::WorkspaceWrite, dir.path());
        let open = Sandbox::listening_variant(&closed).expect("seatbelt can listen");
        let listen = python(
            "import socket; s = socket.socket(); s.bind((\"127.0.0.1\", 0)); s.listen(); print(\"listening\")",
        );
        let connect = python(
            "import socket; socket.create_connection((\"1.1.1.1\", 53), timeout=3); print(\"connected\")",
        );

        let refused = run(closed.wrap(&listen));
        assert!(!String::from_utf8_lossy(&refused.stdout).contains("listening"));
        let listened = run(open.wrap(&listen));
        assert!(
            String::from_utf8_lossy(&listened.stdout).contains("listening"),
            "{}",
            String::from_utf8_lossy(&listened.stderr)
        );
        for sandbox in [&closed as &dyn Sandbox, open.as_ref()] {
            let out = run(sandbox.wrap(&connect));
            assert!(!String::from_utf8_lossy(&out.stdout).contains("connected"));
        }
    }

    #[test]
    fn deny_sandbox_wrap_emits_exit_126_with_reason() {
        let sb = DenySandbox::new("docker unavailable");
        let wrapped = sb.wrap("echo hello");
        assert!(
            wrapped.contains("exit 126"),
            "deny must refuse to run: {wrapped}"
        );
        assert!(
            wrapped.contains("vak sandbox unavailable: docker unavailable"),
            "{wrapped}"
        );
        // The original command must never appear unguarded.
        assert!(
            !wrapped.contains("echo hello") && !wrapped.contains("'echo hello'"),
            "denied command must not be embedded: {wrapped}"
        );
    }

    #[test]
    fn deny_sandbox_read_only_variant_preserves_deny() {
        let sb = DenySandbox::new("unsupported host");
        let ro = Sandbox::read_only_variant(&sb).expect("deny must expose read-only variant");
        let wrapped = ro.wrap("anything");
        assert!(wrapped.contains("exit 126"), "{wrapped}");
        assert_eq!(ro.name(), "unavailable-deny");
    }

    #[test]
    fn seatbelt_read_only_variant_strips_write_paths() {
        let tmp = std::path::Path::new("/tmp");
        let sb = Seatbelt::new(SandboxMode::WorkspaceWrite, tmp);
        let ro = Sandbox::read_only_variant(&sb).expect("read-only variant must exist");
        // The profile is embedded inside wrap()'s sandbox-exec invocation;
        // inspect it there rather than downcasting the trait object.
        let wrapped = ro.wrap("true");
        let profile_start = wrapped.find("(version 1)").expect("profile present");
        let profile_end = wrapped
            .rfind("' -- sh -c")
            .expect("command follows profile");
        let profile = &wrapped[profile_start..profile_end];
        assert!(
            !profile.contains("file-write*"),
            "read-only variant must not grant any write allowance: {profile}"
        );
    }

    #[test]
    fn seatbelt_read_only_variant_name_and_mode() {
        let sb = Seatbelt::new(SandboxMode::WorkspaceWrite, std::path::Path::new("/tmp"));
        let ro = Sandbox::read_only_variant(&sb).expect("variant");
        assert_eq!(ro.name(), "seatbelt");
    }

    // ── quoting safety ────────────────────────────────────────────────

    #[test]
    fn shell_quote_survives_single_quotes_and_semicolons() {
        let cmd = "echo 'it's great'; rm -rf /";
        let quoted = shell_quote(cmd);
        // Must be a single balanced pair wrapping the whole string, with
        // embedded single quotes POSIX-escaped as '\''.
        assert_eq!(
            quoted.matches('\'').count() % 2,
            1,
            "must have odd count (outer pair + escapes): {quoted}"
        );

        // Re-executing the quoted string through sh must reproduce the
        // original verbatim (POSIX round-trip via single-quote escape).
        let verified = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf %s {}", quoted))
            .output();
        if let Ok(out) = verified {
            let text = String::from_utf8_lossy(&out.stdout);
            assert_eq!(text, cmd, "shell_quote round-trip failed: {quoted}");
        } else {
            panic!("sh not available for round-trip test");
        }
    }

    #[test]
    fn sbpl_quote_escapes_backslash_and_double_quote() {
        // A path containing a double-quote can't break out of the SBPL literal.
        let path = r#"path"with"quotes"#;
        let q = sbpl_quote(path);
        assert!(q.starts_with('"') && q.ends_with('"'));
        assert!(
            q.contains(r#"\""#),
            "embedded double-quote must be escaped: {q}"
        );
        // A backslash before a quote must be preserved literally, not
        // consumed as an escape by the shell-quote layer.
        let bs = r#"a\"b"#;
        let q2 = sbpl_quote(bs);
        assert!(q2.contains(r#"\\""#), "backslash must be escaped: {q2}");
    }

    #[test]
    fn off_mode_passes_command_through_unchanged() {
        let sb = Seatbelt::new(SandboxMode::Off, std::path::Path::new("/tmp"));
        assert_eq!(Sandbox::wrap(&sb, "echo hi"), "echo hi");
    }

    #[test]
    fn workspace_write_wrap_embeds_sandbox_exec_and_profile() {
        let dir = tempfile::tempdir().unwrap();
        let sb = Seatbelt::new(SandboxMode::WorkspaceWrite, dir.path());
        let wrapped = sb.wrap("ls -la");
        assert!(wrapped.starts_with("sandbox-exec -p '"));
        assert!(wrapped.contains("(version 1)"));
        assert!(wrapped.ends_with("-- sh -c 'ls -la'"));
    }
}
