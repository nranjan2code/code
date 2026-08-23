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
            shell_quote(&format!("vakcoder sandbox unavailable: {}", self.reason))
        )
    }
}

#[derive(Debug, Clone)]
pub struct Seatbelt {
    pub mode: SandboxMode,
    pub read_paths: Vec<PathBuf>,
    pub write_paths: Vec<PathBuf>,
}

impl Seatbelt {
    pub fn new(mode: SandboxMode, cwd: &Path) -> Self {
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
        for path in Self::temp_write_paths() {
            let path = PathBuf::from(path);
            if path.exists() && !read_paths.contains(&path) {
                read_paths.push(path);
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
                for tmp in Self::temp_write_paths() {
                    p.push_str(&format!(
                        "(allow file-write* (subpath {}))\n",
                        sbpl_quote(&tmp)
                    ));
                }
                for dev in ["/dev/null", "/dev/urandom"] {
                    p.push_str(&format!(
                        "(allow file-write* (subpath {}))\n",
                        sbpl_quote(dev)
                    ));
                }
            }
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
