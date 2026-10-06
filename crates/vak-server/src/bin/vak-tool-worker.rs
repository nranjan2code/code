//! Standalone `__tool_worker` broker endpoint. The server process itself
//! doubles as the worker in production (the `vak` binary wires the
//! same subcommand); this tiny binary exists so tests can pin a REAL
//! worker executable via `Core::set_tool_worker_exe` — a cargo test
//! harness cannot speak the broker protocol.
// This worker's stderr is its parent's to read.
#![allow(clippy::disallowed_macros)]

fn main() -> std::process::ExitCode {
    let invoked = std::env::args_os().nth(1);
    let tool_worker =
        invoked.as_deref() == Some(std::ffi::OsStr::new(vak_tools::broker::WORKER_SUBCOMMAND));
    let persistent_worker = invoked.as_deref()
        == Some(std::ffi::OsStr::new(
            vak_tools::broker::PERSISTENT_WORKER_SUBCOMMAND,
        ));
    if !tool_worker && !persistent_worker {
        eprintln!("internal tool worker; do not invoke directly");
        return std::process::ExitCode::from(64);
    }
    let code = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) if persistent_worker => {
            runtime.block_on(vak_tools::broker::persistent_worker_main())
        }
        Ok(runtime) => runtime.block_on(vak_tools::broker::worker_main()),
        Err(_) => 125,
    };
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(125))
}
