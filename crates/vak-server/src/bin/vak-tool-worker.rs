//! Standalone `__tool_worker` broker endpoint. The server process itself
//! doubles as the worker in production (the `vakcoder` binary wires the
//! same subcommand); this tiny binary exists so tests can pin a REAL
//! worker executable via `Core::set_tool_worker_exe` — a cargo test
//! harness cannot speak the broker protocol.

fn main() -> std::process::ExitCode {
    let invoked = std::env::args_os()
        .nth(1)
        .as_deref()
        .is_some_and(|a| a == std::ffi::OsStr::new(vak_tools::broker::WORKER_SUBCOMMAND));
    if !invoked {
        eprintln!("internal tool worker; do not invoke directly");
        return std::process::ExitCode::from(64);
    }
    let code = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(vak_tools::broker::worker_main()),
        Err(_) => 125,
    };
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(125))
}
