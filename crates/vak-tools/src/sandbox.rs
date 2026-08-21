use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    Off,
    ReadOnly,
    WorkspaceWrite,
}

pub trait Sandbox: Send + Sync {
    fn name(&self) -> &str;
    fn wrap(&self, command: &str) -> String;
}

pub fn no_sandbox() -> Option<Arc<dyn Sandbox>> {
    None
}

#[derive(Debug, Clone)]
pub struct Seatbelt {
    pub mode: SandboxMode,
    pub write_paths: Vec<PathBuf>,
}

impl Seatbelt {
    pub fn new(mode: SandboxMode, cwd: &Path) -> Self {
        let write_paths = match mode {
            SandboxMode::WorkspaceWrite => {
                let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
                vec![canonical]
            }
            _ => Vec::new(),
        };
        Seatbelt { mode, write_paths }
    }

    pub fn profile(&self) -> String {
        let mut p = String::from("(version 1)\n");
        match self.mode {
            SandboxMode::Off => return "(version 1)(allow default)".to_string(),
            SandboxMode::ReadOnly => {
                p.push_str("(deny default)\n");
                p.push_str("(allow file-read*)\n");
                p.push_str("(allow process-exec)\n");
                p.push_str("(allow process-fork)\n");
                p.push_str("(allow sysctl-read)\n");
                p.push_str("(allow mach-lookup)\n");
                p.push_str("(allow iokit-get-properties)\n");
            }
            SandboxMode::WorkspaceWrite => {
                p.push_str("(deny default)\n");
                p.push_str("(allow file-read*)\n");
                p.push_str("(allow process-exec)\n");
                p.push_str("(allow process-fork)\n");
                p.push_str("(allow sysctl-read)\n");
                p.push_str("(allow mach-lookup)\n");
                p.push_str("(allow iokit-get-properties)\n");
                for path in &self.write_paths {
                    p.push_str(&format!(
                        "(allow file-write* (subpath \"{}\"))\n",
                        path.display()
                    ));
                }
                for tmp in [
                    "/private/tmp",
                    "/private/var/tmp",
                    "/dev/null",
                    "/dev/urandom",
                ] {
                    p.push_str(&format!("(allow file-write* (subpath \"{tmp}\"))\n"));
                }
            }
        }
        p
    }
}

impl Sandbox for Seatbelt {
    fn name(&self) -> &str {
        "seatbelt"
    }

    fn wrap(&self, command: &str) -> String {
        if self.mode == SandboxMode::Off {
            return command.to_string();
        }
        let profile = self.profile().replace('\'', "");
        format!(
            "sandbox-exec -p '{}' -- sh -c {}",
            profile,
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
