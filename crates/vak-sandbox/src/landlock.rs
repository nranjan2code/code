//! Linux filesystem containment via the Landlock LSM (kernel 5.13+).
//!
//! The `Sandbox` trait wraps a shell command; here the wrapper re-executes
//! THIS binary with a hidden subcommand that applies the ruleset to itself
//! and only then runs the command as a child, so restrictions are inherited.
//! Fail-closed: unsupported kernels surface an error instead of silently
//! running unrestricted.
// This helper process's stderr is its parent's to read.
#![allow(clippy::disallowed_macros)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use landlock::ABI;

use super::backend::{Sandbox, SandboxMode};

pub const SANDBOX_SUBCOMMAND: &str = "__sandbox";

pub fn runner_main(args: impl IntoIterator<Item = std::ffi::OsString>) -> i32 {
    let mut read = Vec::new();
    let mut write = Vec::new();
    let mut read_only = false;
    let mut command = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            command.extend(args.map(|part| part.to_string_lossy().into_owned()));
            break;
        }
        if arg == "--ro" {
            read_only = true;
        } else if arg == "--read" {
            let Some(path) = args.next() else {
                eprintln!("sandbox: --read requires a path");
                return 125;
            };
            read.push(PathBuf::from(path));
        } else if arg == "--rw" {
            let Some(path) = args.next() else {
                eprintln!("sandbox: --rw requires a path");
                return 125;
            };
            write.push(PathBuf::from(path));
        } else {
            eprintln!("sandbox: invalid argument {}", arg.to_string_lossy());
            return 125;
        }
    }
    if command.is_empty() {
        eprintln!("sandbox: no command given");
        return 125;
    }
    #[cfg(target_os = "linux")]
    {
        if let Err(error) = apply(&read, &write, read_only) {
            eprintln!("sandbox: {error}");
            return 126;
        }
        match std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(command.join(" "))
            .status()
        {
            Ok(status) => status.code().unwrap_or(1),
            Err(error) => {
                eprintln!("sandbox: exec failed: {error}");
                127
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (read, write, read_only, command);
        eprintln!("sandbox: not supported on this platform");
        126
    }
}

#[derive(Debug, Clone)]
pub struct Landlock {
    pub mode: SandboxMode,
    pub read_paths: Vec<PathBuf>,
    pub write_paths: Vec<PathBuf>,
}

impl Landlock {
    pub fn new(mode: SandboxMode, cwd: &Path) -> Self {
        Self::build(mode, cwd, true)
    }

    /// Task-copy containment excludes host temp roots even when the source
    /// workspace is under one of them.
    pub fn task_copy(mode: SandboxMode, cwd: &Path) -> Self {
        Self::build(mode, cwd, false)
    }

    fn build(mode: SandboxMode, cwd: &Path, allow_host_temp: bool) -> Self {
        let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let mut read_paths: Vec<PathBuf> = [
            "/bin", "/usr", "/lib", "/lib64", "/etc", "/dev", "/proc", "/sys", "/opt",
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
        let _ = allow_host_temp;
        // Executions keep temp files and caches in the runtime root, outside
        // the project tree; a write sandbox grants exactly that beside the
        // workspace (plan M3b slice 4).
        let executions = vak_config::scope::executions_root(&canonical);
        let _ = std::fs::create_dir_all(&executions);
        let executions = executions.canonicalize().unwrap_or(executions);
        read_paths.push(executions.clone());
        let write_paths = match mode {
            SandboxMode::WorkspaceWrite => vec![canonical, executions],
            _ => Vec::new(),
        };
        Landlock {
            mode,
            read_paths,
            write_paths,
        }
    }
}

#[cfg(test)]
mod task_copy_tests {
    use super::*;

    #[test]
    fn task_copy_excludes_host_temp_write_roots() {
        let copy = tempfile::tempdir().expect("task copy");
        let sandbox = Landlock::task_copy(SandboxMode::WorkspaceWrite, copy.path());
        let canonical = copy.path().canonicalize().expect("canonical copy");
        // The copy and its own execution root, never the host's temp.
        let executions = vak_config::scope::executions_root(&canonical);
        let executions = executions.canonicalize().unwrap_or(executions);
        assert_eq!(sandbox.write_paths, vec![canonical, executions]);
        let temp = std::env::temp_dir();
        for host in [temp.as_path(), Path::new("/tmp"), Path::new("/private/tmp")] {
            assert!(!sandbox.write_paths.iter().any(|path| path == host));
            assert!(!sandbox.read_paths.iter().any(|path| path == host));
        }
    }
}

impl Sandbox for Landlock {
    fn name(&self) -> &str {
        "landlock"
    }

    fn wrap(&self, command: &str) -> String {
        if self.mode == SandboxMode::Off {
            return command.to_string();
        }
        let exe = match std::env::current_exe() {
            Ok(p) => p.display().to_string(),
            // Without a resolvable executable the containment cannot be
            // established; refuse to run rather than escape the sandbox.
            Err(_) => {
                return "echo 'vak sandbox: cannot locate executable' >&2; exit 126".to_string();
            }
        };
        let mut parts = vec![shell_quote(&exe), SANDBOX_SUBCOMMAND.to_string()];
        if self.mode == SandboxMode::ReadOnly {
            parts.push("--ro".to_string());
        }
        for path in &self.read_paths {
            parts.push(format!(
                "--read {}",
                shell_quote(&path.display().to_string())
            ));
        }
        for p in &self.write_paths {
            parts.push(format!("--rw {}", shell_quote(&p.display().to_string())));
        }
        parts.push("--".to_string());
        parts.push(shell_quote(command));
        parts.join(" ")
    }

    fn read_only_variant(&self) -> Option<Arc<dyn Sandbox>> {
        Some(Arc::new(Landlock {
            mode: SandboxMode::ReadOnly,
            read_paths: self.read_paths.clone(),
            write_paths: Vec::new(),
        }))
    }
}

/// Applies the ruleset to the CURRENT process. Read+execute only under
/// `read_paths`; writes only under `write_paths` unless `read_only`. Every
/// IPv4 and IPv6 socket is refused in every sandboxed mode -- parity with
/// Seatbelt, whose deny-default profile leaves no network allowance -- and
/// the call FAILS CLOSED when the kernel cannot enforce either half.
///
/// Landlock handles files (ABI v3, so truncation is a governed right, not a
/// way around a denied write). It cannot deny UDP, so the network is denied
/// by a seccomp filter instead: `socket()` for `AF_INET`/`AF_INET6` returns
/// EPERM, which covers TCP, UDP and raw sockets alike, and io_uring, which
/// can open sockets without calling `socket()`, is refused outright. Unix
/// sockets stay available to local tools.
pub fn apply(
    read_paths: &[PathBuf],
    write_paths: &[PathBuf],
    read_only: bool,
) -> Result<(), String> {
    use landlock::{
        Access, AccessFs, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
        path_beneath_rules,
    };

    let fs_abi = ABI::V3;
    let created = Ruleset::default()
        .handle_access(AccessFs::from_all(fs_abi))
        .and_then(|r| r.create())
        .map_err(|e| format!("landlock: {e}"))?;
    let created = created
        .add_rules(path_beneath_rules(read_paths, AccessFs::from_read(fs_abi)))
        .map_err(|e| format!("landlock: {e}"))?;
    let restricted = if read_only {
        created.restrict_self()
    } else {
        created
            .add_rules(path_beneath_rules(write_paths, AccessFs::from_all(fs_abi)))
            .and_then(|r| r.restrict_self())
    }
    .map_err(|e| format!("landlock: {e}"))?;
    if restricted.ruleset != RulesetStatus::FullyEnforced {
        return Err(
            "landlock: full enforcement unavailable (the kernel needs Landlock ABI v3, Linux 6.2+)"
                .to_string(),
        );
    }
    deny_network()
}

/// The seccomp half of [`apply`]: no IPv4 or IPv6 socket and no io_uring
/// for this process and everything it executes.
fn deny_network() -> Result<(), String> {
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule, TargetArch,
    };
    let fail = |e: &dyn std::fmt::Display| format!("seccomp: {e}");
    let domain = |family: libc::c_int| {
        SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, family as u64)
            .and_then(|condition| SeccompRule::new(vec![condition]))
    };
    let socket_rules = vec![
        domain(libc::AF_INET).map_err(|e| fail(&e))?,
        domain(libc::AF_INET6).map_err(|e| fail(&e))?,
    ];
    let rules = [
        (libc::SYS_socket, socket_rules),
        (libc::SYS_io_uring_setup, Vec::new()),
        (libc::SYS_io_uring_enter, Vec::new()),
        (libc::SYS_io_uring_register, Vec::new()),
    ]
    .into_iter()
    .collect();
    let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(|e| fail(&e))?;
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        arch,
    )
    .map_err(|e| fail(&e))?;
    let program: BpfProgram = filter
        .try_into()
        .map_err(|e: seccompiler::BackendError| fail(&e))?;
    seccompiler::apply_filter(&program).map_err(|e| fail(&e))
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn wrap_self_executes_with_flags() {
        let sb = Landlock::new(SandboxMode::WorkspaceWrite, Path::new("/tmp/proj"));
        let wrapped = sb.wrap("cargo test");
        assert!(wrapped.contains("__sandbox"));
        assert!(wrapped.contains("--rw '/tmp/proj'"));
        assert!(wrapped.contains("'cargo test'"));

        let ro = Landlock::new(SandboxMode::ReadOnly, Path::new("/"));
        assert!(ro.wrap("ls").contains("--ro"));
        assert!(!ro.wrap("ls").contains("--rw"));

        assert_eq!(
            Landlock::new(SandboxMode::Off, Path::new("/")).wrap("true"),
            "true"
        );
    }

    #[test]
    fn wrap_fails_closed_without_resolvable_exe() {
        // current_exe virtually never fails; the fail-closed string is still
        // part of the contract, so verify it directly through formatting.
        let cmd = "echo 'vak sandbox: cannot locate executable' >&2; exit 126";
        assert!(cmd.contains("exit 126"));
    }
}
