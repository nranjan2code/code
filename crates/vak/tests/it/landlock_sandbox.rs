//! The Linux sandbox end to end: a command run under `vak __sandbox` reads
//! what it was granted, writes only where it was granted, and opens no IPv4
//! or IPv6 socket, TCP or UDP, while a Unix socket still works. From
//! 2026-10-02 the sandbox refused every command, because Landlock cannot
//! deny UDP; nothing ran it on Linux, so no test noticed. The network is now
//! denied by a seccomp filter.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Command;

fn sandboxed(args: &[&str], command: &str) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_vak"))
        .arg("__sandbox")
        .args(args)
        .arg("--")
        .arg(command)
        .output()
        .unwrap();
    (
        output.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn a_sandboxed_command_runs_within_its_grants_and_has_no_network() {
    let dir = tempfile::tempdir().unwrap();
    let inside = dir.path().join("work");
    std::fs::create_dir_all(&inside).unwrap();
    let outside = dir.path().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let rw = inside.display().to_string();

    // It runs at all, and reads what it was granted.
    let (code, said) = sandboxed(&["--read", "/"], "echo sandboxed");
    assert_eq!(code, 0, "{said}");
    assert!(said.contains("sandboxed"));

    // Writes land only where granted.
    let (code, said) = sandboxed(
        &["--read", "/", "--rw", &rw],
        &format!("echo kept > {rw}/note"),
    );
    assert_eq!(code, 0, "{said}");
    assert!(inside.join("note").exists());
    let (code, _) = sandboxed(
        &["--read", "/", "--rw", &rw],
        &format!("echo leaked > {}/note", outside.display()),
    );
    assert_ne!(code, 0);
    assert!(!outside.join("note").exists());
    let (code, _) = sandboxed(&["--ro", "--read", "/"], &format!("echo x > {rw}/ro"));
    assert_ne!(code, 0);
    assert!(!inside.join("ro").exists());

    // No IPv4 or IPv6 socket of either kind; a Unix socket is fine.
    let probe = |family: &str, kind: &str| {
        format!(
            "python3 -c 'import socket; socket.socket(socket.{family}, socket.{kind}); print(\"opened\")'"
        )
    };
    if Command::new("python3").arg("--version").output().is_ok() {
        for (family, kind) in [
            ("AF_INET", "SOCK_DGRAM"),
            ("AF_INET", "SOCK_STREAM"),
            ("AF_INET6", "SOCK_DGRAM"),
            ("AF_INET6", "SOCK_STREAM"),
        ] {
            let (code, said) = sandboxed(&["--read", "/"], &probe(family, kind));
            assert_ne!(code, 0, "{family} {kind} opened: {said}");
            assert!(!said.contains("opened"), "{family} {kind}: {said}");
        }
        let (code, said) = sandboxed(&["--read", "/"], &probe("AF_UNIX", "SOCK_STREAM"));
        assert_eq!(code, 0, "{said}");
        assert!(said.contains("opened"));
    }
    // The shell's own network redirection is refused too.
    let (code, _) = sandboxed(
        &["--read", "/"],
        "bash -c 'exec 3<>/dev/udp/127.0.0.1/53' 2>/dev/null",
    );
    assert_ne!(code, 0);
}
