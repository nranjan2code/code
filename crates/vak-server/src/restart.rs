//! A `vak serve` that a restore, a pull or a key file fenced starts again
//! by itself: it lets the answer that fenced it reach the person, drains
//! its connections and replaces itself with a fresh copy of the same
//! command, under the same process id, so a service manager sees nothing
//! stop and never starts a second one (invariant 25). An embedded server
//! (the desktop shell, `vak setup`) does not, and its answers keep
//! `restarting` false so the person starts it again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static SELF_RESTARTS: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);

/// Whether this process starts again by itself once it is fenced.
pub(crate) fn self_restarts() -> bool {
    SELF_RESTARTS.load(Ordering::SeqCst)
}

/// While set, this process is stopping for good (everything is being
/// erased) and must not start again.
pub(crate) fn stopping(on: bool) {
    STOPPING.store(on, Ordering::SeqCst);
}

/// Resolves once this process is fenced and should start again.
pub(crate) async fn when_fenced() {
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if vak_session::fence::is_fenced() && !STOPPING.load(Ordering::SeqCst) {
            // The answer that fenced this process reaches its caller first.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            if !STOPPING.load(Ordering::SeqCst) {
                return;
            }
        }
    }
}

/// Marks this process as one that starts again by itself, where it can.
pub(crate) fn enable() {
    SELF_RESTARTS.store(cfg!(unix), Ordering::SeqCst);
}

/// Replaces this process with the same command. Returns only if that
/// failed.
#[cfg(unix)]
pub(crate) fn exec_again() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => return error,
    };
    std::process::Command::new(program)
        .args(std::env::args_os().skip(1))
        .exec()
}

#[cfg(not(unix))]
pub(crate) fn exec_again() -> std::io::Error {
    std::io::Error::other("starting again is not supported on this platform")
}
