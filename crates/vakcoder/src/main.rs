use std::io::Write as _;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, TurnOutcome};
use vak_core::Core;
use vak_llm::stream::StreamEvent;

#[derive(Parser)]
#[command(name = "vakcoder", version, about = "A coding agent harness")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive terminal UI (default when no subcommand given)
    Tui,
    /// Run one prompt headless and print the result
    Exec {
        prompt: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = 40)]
        max_turns: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        permission_mode: Option<String>,
    },
    /// Show the effective composed configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// List recorded sessions for this project
    Sessions,
}

#[derive(Subcommand)]
enum ConfigAction {
    Dump,
}
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let code = match cli.command {
        None | Some(Command::Tui) => run_tui(cwd).await,
        Some(Command::Exec {
            prompt,
            model,
            provider,
            max_turns,
            json,
            yes,
            permission_mode,
        }) => {
            run_exec(
                cwd,
                prompt,
                model,
                provider,
                max_turns,
                json,
                yes,
                permission_mode,
            )
            .await
        }
        Some(Command::Config { .. }) => {
            run_config_dump(cwd);
            0
        }
        Some(Command::Sessions) => {
            run_sessions_list(cwd);
            0
        }
    };
    std::process::exit(code);
}

async fn run_tui(cwd: PathBuf) -> i32 {
    let core = match Core::new(cwd.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    vak_tui::run(core, vak_tui::UiConfig { cwd }).await
}

#[allow(clippy::too_many_arguments)]
async fn run_exec(
    cwd: PathBuf,
    prompt: String,
    model: Option<String>,
    provider: Option<String>,
    max_turns: usize,
    _json: bool,
    yes: bool,
    permission_mode: Option<String>,
) -> i32 {
    let core = match Core::new(cwd.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    if let Some(m) = model {
        core.set_model(m);
    }
    if let Some(p) = provider {
        core.set_provider(p);
    }
    core.set_max_turns(max_turns);
    if let Some(pm) = permission_mode {
        match vak_config::PermissionMode::deserialize_str(&pm) {
            Some(m) => core.set_permission_mode(m),
            None => {
                eprintln!(
                    "error: unknown --permission-mode '{pm}' (read-only | workspace-write | full-access)"
                );
                return 2;
            }
        }
    }

    let session = match core.start_session().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let session_path = session.path().to_path_buf();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(1024);
    let cancel = CancellationToken::new();
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\n[cancelling…]");
            cancel_for_signal.cancel();
        }
    });

    let approver: Option<std::sync::Arc<dyn vak_agent::Approver>> = Some(if yes {
        std::sync::Arc::new(vak_agent::AutoApprove)
    } else {
        std::sync::Arc::new(vak_agent::AutoDeny)
    });

    let runner = tokio::spawn(async move {
        core.run_turn_with(session, &prompt, cancel, approver, None, tx)
            .await
    });

    let mut total_in: u64 = 0;
    let mut total_out: u64 = 0;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::TurnStart { turn } => {
                if turn > 0 {
                    writeln!(out).ok();
                }
            }
            AgentEvent::Stream(StreamEvent::TextDelta { delta, .. }) => {
                write!(out, "{delta}").ok();
                out.flush().ok();
            }
            AgentEvent::Stream(StreamEvent::ThinkingDelta { .. }) => {}
            AgentEvent::ToolCallStart { name, .. } => {
                eprintln!("▸ {name}");
            }
            AgentEvent::ToolCallEnd { name, is_error, .. } => {
                eprintln!(
                    "{}",
                    if is_error {
                        format!("✗ {name}")
                    } else {
                        format!("✓ {name}")
                    }
                );
            }
            AgentEvent::TurnEnd { usage } => {
                total_in += usage.input_tokens;
                total_out += usage.output_tokens;
            }
            _ => {}
        }
    }

    let outcome = match runner.await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            eprintln!("error: {e}");
            return 2;
        }
        Err(e) => {
            eprintln!("error: runner crashed: {e}");
            return 2;
        }
    };

    writeln!(out).ok();
    eprintln!(
        "\n── {} · tokens in {total_in} / out {total_out} · session {}",
        match &outcome {
            TurnOutcome::Completed { .. } => "completed",
            TurnOutcome::Aborted { .. } => "aborted",
            TurnOutcome::Failed { .. } => "failed",
            TurnOutcome::MaxTurnsReached => "max turns reached",
        },
        session_path.display()
    );
    if let TurnOutcome::Failed { error } = outcome {
        eprintln!("error: {error}");
        return 1;
    }
    0
}

fn run_config_dump(cwd: PathBuf) {
    match Core::new(cwd.clone()) {
        Ok(core) => {
            println!("# vakcoder effective config");
            println!("version          = {}", vak_core::APP_VERSION);
            println!("cwd              = {}", core.cwd().display());
            println!("provider         = {}", core.effective_provider());
            println!("model            = {}", core.effective_model());
            println!("max_tokens       = {}", core.config().max_tokens);
            println!("max_turns        = {}", core.effective_max_turns());
            println!("permission_mode  = {:?}", core.effective_permission_mode());
            println!("sessions_home    = {}", core.sessions_home().display());
            println!(
                "anthropic_base   = {}",
                core.config()
                    .anthropic_base_url
                    .clone()
                    .unwrap_or_else(|| "https://api.anthropic.com".into())
            );
            println!("tools            = {}", core.tool_names().join(", "));
            for w in &core.config().warnings {
                println!("warning          = {w}");
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
        }
    }
}

fn run_sessions_list(cwd: PathBuf) {
    let Ok(core) = Core::new(cwd) else {
        return;
    };
    let dir = vak_session::SessionPath::sessions_dir(core.sessions_home(), core.cwd());
    let Ok(entries) = std::fs::read_dir(&dir) else {
        println!("no sessions yet ({})", dir.display());
        return;
    };
    let mut rows: Vec<(std::time::SystemTime, u64, String)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((
                meta.modified().ok()?,
                meta.len(),
                e.file_name().to_string_lossy().into_owned(),
            ))
        })
        .collect();
    rows.sort_by_key(|(mtime, _, _)| std::cmp::Reverse(*mtime));
    if rows.is_empty() {
        println!("no sessions yet ({})", dir.display());
        return;
    }
    for (_mtime, size, name) in rows {
        println!("{name}  {size:>10} bytes");
    }
}
