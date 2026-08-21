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
    /// Static flow DAGs: list, check, run
    Flow {
        #[command(subcommand)]
        action: FlowAction,
    },
    /// Plan and execute an open-ended task with a dynamic planner
    Plan {
        task: String,
        #[arg(long)]
        yes: bool,
    },
    /// Run the built-in eval suite (deterministic, in-process)
    Eval {
        /// Write JSON report to this path
        #[arg(long)]
        report: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum FlowAction {
    /// List discovered flows
    List,
    /// Validate a flow without running it
    Check { name: String },
    /// Run a flow (optionally resuming a previous run)
    Run {
        name: String,
        #[arg(long)]
        resume: bool,
        #[arg(long)]
        yes: bool,
    },
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
        Some(Command::Flow { action }) => run_flow(cwd, action).await,
        Some(Command::Plan { task, yes }) => run_plan(cwd, task, yes).await,
        Some(Command::Eval { report }) => run_eval(report).await,
    };
    std::process::exit(code);
}

fn flow_dirs(cwd: &std::path::Path) -> Vec<PathBuf> {
    let mut dirs = vec![cwd.join(".vakcoder/flows")];
    if let Some(home) = std::env::var_os("VAKCODER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".vakcoder")))
    {
        dirs.push(home.join("flows"));
    }
    dirs
}

fn discover_flows(cwd: &std::path::Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for dir in flow_dirs(cwd) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("toml") {
                out.push((
                    p.file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    p,
                ));
            }
        }
    }
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

async fn run_flow(cwd: PathBuf, action: FlowAction) -> i32 {
    match action {
        FlowAction::List => {
            let flows = discover_flows(&cwd);
            if flows.is_empty() {
                println!("no flows found (.vakcoder/flows/*.toml)");
                return 0;
            }
            for (name, path) in flows {
                println!("{name}  {}", path.display());
            }
            0
        }
        FlowAction::Check { name } => {
            let Some((_, path)) = discover_flows(&cwd).into_iter().find(|(n, _)| *n == name) else {
                eprintln!("error: flow '{name}' not found");
                return 2;
            };
            let toml_str = std::fs::read_to_string(&path).unwrap_or_default();
            match vak_flow::parse_flow(&toml_str) {
                Ok(flow) => {
                    let layers = vak_flow::parse::layers(&flow).unwrap_or_default();
                    println!(
                        "✓ {} valid — {} nodes, {} layers",
                        flow.name,
                        flow.nodes.len(),
                        layers.len()
                    );
                    for (i, layer) in layers.iter().enumerate() {
                        println!("  layer {}: {}", i + 1, layer.join(", "));
                    }
                    0
                }
                Err(e) => {
                    eprintln!("✗ invalid: {e}");
                    1
                }
            }
        }
        FlowAction::Run { name, resume, yes } => run_flow_exec(cwd, name, resume, yes).await,
    }
}

async fn run_flow_exec(cwd: PathBuf, name: String, resume: bool, yes: bool) -> i32 {
    let core = match Core::new(cwd.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let Some((_, path)) = discover_flows(&core.cwd().clone())
        .into_iter()
        .find(|(n, _)| *n == name)
    else {
        eprintln!("error: flow '{name}' not found");
        return 2;
    };
    let toml_str = std::fs::read_to_string(&path).unwrap_or_default();
    let flow = match vak_flow::parse_flow(&toml_str) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("✗ invalid: {e}");
            return 1;
        }
    };

    let provider = match core.provider() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let session = match core.start_session().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let parent_session_id = session
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();

    let approver: Option<std::sync::Arc<dyn vak_agent::Approver>> = Some(if yes {
        std::sync::Arc::new(vak_agent::AutoApprove)
    } else {
        std::sync::Arc::new(vak_agent::AutoDeny)
    });

    let engine = match vak_core::build_engine(core.config()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let runs_dir = core.sessions_home().join("flow-runs").join(&name);
    let state_path = if resume {
        let mut latest: Option<PathBuf> = None;
        if let Ok(entries) = std::fs::read_dir(&runs_dir) {
            let mut files: Vec<_> = entries.flatten().map(|e| e.path()).collect();
            files.sort();
            latest = files.pop();
        }
        match latest {
            Some(p) => p,
            None => {
                eprintln!("error: no previous run to resume");
                return 2;
            }
        }
    } else {
        let run_id = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        runs_dir.join(format!("{run_id}.json"))
    };

    let mut state = load_state(&state_path, &name, &toml_str);

    let deps = vak_flow::ExecutorDeps {
        provider,
        system_prompt: core.system_prompt(),
        model: core.effective_model(),
        tools: vak_tools::default_tools(),
        read_only_tools: vak_tools::read_only_tools(),
        max_turns: core.effective_max_turns(),
        permission: Some(std::sync::Arc::new(engine)),
        mode: match core.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        },
        approver,
        sandbox: None,
        cwd: core.cwd().clone(),
        sessions_home: core.sessions_home().clone(),
        parent_session_id,
        state_path: state_path.clone(),
    };
    let executor = vak_flow::Executor::new(deps);

    let cancel = CancellationToken::new();
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("\n[cancelling…]");
                cancel.cancel();
            }
        });
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    let runner = tokio::spawn(async move { executor.run(&flow, &mut state, cancel, tx).await });

    while let Some(line) = rx.recv().await {
        eprintln!("{line}");
    }

    match runner.await {
        Ok(outcome) => match outcome {
            vak_flow::FlowOutcome::Completed { outputs } => {
                eprintln!("── flow completed · state {}", state_path.display());
                for (id, out) in outputs {
                    println!("[{id}]\n{out}\n");
                }
                0
            }
            vak_flow::FlowOutcome::Failed { node, reason, .. } => {
                eprintln!("── flow failed at '{node}': {reason}");
                eprintln!("   resume with: vakcoder flow run {name} --resume");
                1
            }
            vak_flow::FlowOutcome::Aborted => {
                eprintln!("── flow aborted · resume with: vakcoder flow run {name} --resume");
                1
            }
        },
        Err(e) => {
            eprintln!("error: flow runner crashed: {e}");
            2
        }
    }
}

fn load_state(state_path: &PathBuf, flow_name: &str, definition_toml: &str) -> vak_flow::FlowState {
    if let Ok(text) = std::fs::read_to_string(state_path)
        && let Ok(state) = serde_json::from_str::<vak_flow::FlowState>(&text)
    {
        return state;
    }
    vak_flow::FlowState {
        run_id: state_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        flow_name: flow_name.to_string(),
        definition_toml: definition_toml.to_string(),
        started_at: chrono::Utc::now(),
        nodes: Default::default(),
    }
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
            println!("sandbox          = {}", core.effective_sandbox_name());
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

async fn run_plan(cwd: PathBuf, task: String, yes: bool) -> i32 {
    let core = match Core::new(cwd.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let provider = match core.provider() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let session = match core.start_session().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let parent_session_id = session
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();

    let approver: Option<std::sync::Arc<dyn vak_agent::Approver>> = Some(if yes {
        std::sync::Arc::new(vak_agent::AutoApprove)
    } else {
        std::sync::Arc::new(vak_agent::AutoDeny)
    });
    let engine = match vak_core::build_engine(core.config()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let deps = vak_flow::ExecutorDeps {
        provider,
        system_prompt: core.system_prompt(),
        model: core.effective_model(),
        tools: vak_tools::default_tools(),
        read_only_tools: vak_tools::read_only_tools(),
        max_turns: core.effective_max_turns(),
        permission: Some(std::sync::Arc::new(engine)),
        mode: match core.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        },
        approver,
        sandbox: None,
        cwd: core.cwd().clone(),
        sessions_home: core.sessions_home().clone(),
        parent_session_id,
        state_path: core.sessions_home().join("flow-runs/plan"),
    };

    let cancel = CancellationToken::new();
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("\n[cancelling…]");
                cancel.cancel();
            }
        });
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(256);
    let runner = tokio::spawn(async move {
        vak_flow::plan_and_run(std::sync::Arc::new(deps), &task, cancel, tx).await
    });

    while let Some(line) = rx.recv().await {
        eprintln!("{line}");
    }

    match runner.await {
        Ok(vak_flow::PlanOutcome::Completed { outputs, attempts }) => {
            eprintln!("── plan completed after {attempts} attempt(s)");
            for (id, out) in outputs {
                println!("[{id}]\n{out}\n");
            }
            0
        }
        Ok(vak_flow::PlanOutcome::PlanningFailed { reason }) => {
            eprintln!("── planning_failed (fail-closed): {reason}");
            1
        }
        Ok(vak_flow::PlanOutcome::Failed { node, reason }) => {
            eprintln!("── plan execution failed at '{node}': {reason}");
            1
        }
        Ok(vak_flow::PlanOutcome::Aborted) => {
            eprintln!("── aborted");
            1
        }
        Err(e) => {
            eprintln!("error: planner crashed: {e}");
            2
        }
    }
}

async fn run_eval(report_path: Option<PathBuf>) -> i32 {
    let cases = vak_eval::builtin_suite();
    let mut reports = Vec::with_capacity(cases.len());
    for case in &cases {
        let r = vak_eval::run_case(case).await;
        println!(
            "{:<12} {:>6}  in {:>5} / out {:>4}  {:>5}ms  {}",
            r.task_id,
            if r.passed { "PASS" } else { "FAIL" },
            r.tokens_in,
            r.tokens_out,
            r.duration_ms,
            r.error.as_deref().unwrap_or("")
        );
        reports.push(r);
    }

    let passed = reports.iter().filter(|r| r.passed).count();
    let total = reports.len();
    let tokens_in: u64 = reports.iter().map(|r| r.tokens_in).sum();
    let tokens_out: u64 = reports.iter().map(|r| r.tokens_out).sum();

    if let Some(path) = report_path {
        let json = serde_json::to_string_pretty(&serde_json::json!({
            "generated_at": chrono::Utc::now(),
            "passed": passed,
            "total": total,
            "tokens_in": tokens_in,
            "tokens_out": tokens_out,
            "cases": reports,
        }))
        .unwrap_or_default();
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&path, json) {
            Ok(_) => eprintln!("report written to {}", path.display()),
            Err(e) => {
                eprintln!("error writing report: {e}");
                return 2;
            }
        }
    }

    println!("\n{passed}/{total} passed · tokens in {tokens_in} / out {tokens_out}");
    if passed == total { 0 } else { 1 }
}
