use std::io::Write as _;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, TurnOutcome};
use vak_core::Core;
use vak_llm::stream::StreamEvent;

#[derive(Parser)]
#[command(name = "VakCoder", version, about = "A coding agent harness")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive terminal UI (default when no subcommand given)
    Tui {
        /// Trust this workspace's project config and .env without prompting
        #[arg(long)]
        trust: bool,
    },
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
        /// Run in an isolated git worktree off HEAD
        #[arg(long)]
        worktree: bool,
        /// Resume an existing session instead of starting a new one
        #[arg(long)]
        session: Option<String>,
        /// Durable objective for goal mode (docs/design/27 Phase H):
        /// completion is audited against --criteria, never self-reported.
        #[arg(long)]
        goal: Option<String>,
        /// Acceptance criteria, comma-separated. Prefix `verify:` to run a
        /// criterion as a shell command; others are judged from evidence.
        #[arg(long, value_delimiter = ',')]
        criteria: Vec<String>,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
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
        /// Run in an isolated git worktree off HEAD
        #[arg(long)]
        worktree: bool,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
    /// Run the built-in eval suite
    Eval {
        /// Write JSON report to this path
        #[arg(long)]
        report: Option<PathBuf>,
        /// Run the live suite against the configured provider (needs API key)
        #[arg(long)]
        live: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Serve the agent over HTTP+SSE
    Serve {
        #[arg(long, default_value_t = 8901)]
        port: u16,
        /// Enable gateway surface routing regardless of config
        /// (docs/design/22-gateway.md)
        #[arg(long)]
        gateway: bool,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
    /// Workspace checkpoints: list or restore
    Checkpoints {
        #[command(subcommand)]
        action: CheckpointAction,
    },
    /// List durable memory notes for this workspace
    Memory,
    /// Review proposed skills: list / promote / reject
    SkillsReview {
        #[command(subcommand)]
        action: SkillsReviewAction,
    },
    /// Bridge a Telegram bot to a running gateway (docs/design/22-gateway.md)
    Telegram {
        /// Gateway base URL, e.g. http://127.0.0.1:8901
        #[arg(long)]
        server: String,
        /// Gateway bearer token (overrides VAKCODER_GATEWAY_TOKEN; the
        /// env var is the normal path so secrets never appear in `ps`)
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
enum FlowAction {
    /// List discovered flows
    List,
    /// Convert proven work into a flow file (doc 27 Phase E):
    /// --from accepts a flow-run/plan ledger JSON path or a session id.
    Adopt {
        /// Ledger JSON path, or a session id whose green bash commands
        /// become a chained bash flow.
        from: String,
        /// Name for the adopted flow (written to .vakcoder/flows/)
        #[arg(long)]
        name: String,
        /// Overwrite an existing flow file of the same name
        #[arg(long)]
        force: bool,
    },
    /// Deterministic run-vs-run diff over two ledger JSONs (no model)
    Diff {
        /// Path to first run/plan ledger JSON
        a: std::path::PathBuf,
        /// Path to second run/plan ledger JSON
        b: std::path::PathBuf,
    },
    /// Validate a flow without running it
    Check { name: String },
    /// Run a flow (optionally resuming a previous run)
    Run {
        name: String,
        #[arg(long)]
        resume: bool,
        /// Acknowledge live-file drift and resume the frozen snapshot
        #[arg(long)]
        accept_drift: bool,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Trust this workspace's project config and .env
        #[arg(long)]
        trust: bool,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    Dump,
}

#[derive(Subcommand)]
enum SkillsReviewAction {
    /// List pending proposals
    List,
    /// Promote a proposal into user-level skills
    Promote { id: String },
    /// Discard a proposal
    Reject { id: String },
}

#[derive(Subcommand)]
enum CheckpointAction {
    /// List checkpoints for the latest session in this project
    List {
        #[arg(long)]
        session: Option<String>,
    },
    /// Restore a checkpoint into the workspace
    Restore { session: String, seq: u32 },
}

fn learning_core(cwd: PathBuf) -> Option<Core> {
    match Core::new(cwd) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("error: {e}");
            None
        }
    }
}

fn run_memory_list(cwd: PathBuf) -> i32 {
    let Some(core) = learning_core(cwd) else {
        return 2;
    };
    let notes = vak_core::memory::list_notes(&core.sessions_home(), core.cwd());
    if notes.is_empty() {
        println!("no memory notes for this workspace");
        return 0;
    }
    for n in &notes {
        println!(
            "{} [{}]{} session={}",
            n.ts.to_rfc3339(),
            n.kind,
            if n.tag.is_empty() {
                String::new()
            } else {
                format!(" tag={}", n.tag)
            },
            n.session_id
        );
        println!("  {}", n.text.replace('\n', "\n  "));
    }
    0
}

fn run_skills_review(cwd: PathBuf, action: SkillsReviewAction) -> i32 {
    let Some(core) = learning_core(cwd) else {
        return 2;
    };
    match action {
        SkillsReviewAction::List => {
            let proposals = vak_core::learning::list_proposals(&core.sessions_home(), core.cwd());
            if proposals.is_empty() {
                println!("no pending skill proposals");
                return 0;
            }
            for p in &proposals {
                println!("{}  {} — {}", p.id, p.name, p.description);
            }
            0
        }
        SkillsReviewAction::Promote { id } => {
            match vak_core::learning::promote(&core.sessions_home(), core.cwd(), &id) {
                Ok(name) => {
                    println!("promoted skill '{name}'");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        SkillsReviewAction::Reject { id } => {
            match vak_core::learning::reject(&core.sessions_home(), core.cwd(), &id) {
                Ok(()) => {
                    println!("rejected proposal {id}");
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
    }
}

async fn run_checkpoints(cwd: PathBuf, action: CheckpointAction) -> i32 {
    let core = match Core::new(cwd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match action {
        CheckpointAction::List { session } => {
            let sid = match session {
                Some(s) => s,
                None => match latest_session_id(&core) {
                    Some(s) => s,
                    None => {
                        println!("no sessions yet");
                        return 0;
                    }
                },
            };
            match vak_core::checkpoints::list(&core.sessions_home(), &sid) {
                Ok(list) if list.is_empty() => {
                    println!("no checkpoints for {sid}");
                    0
                }
                Ok(list) => {
                    for cp in list {
                        println!(
                            "{:04}  {} files  {}  {}",
                            cp.seq,
                            cp.files.len(),
                            cp.created_at.format("%H:%M:%S"),
                            cp.label.chars().take(60).collect::<String>()
                        );
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
        }
        CheckpointAction::Restore { session, seq } => {
            match vak_core::checkpoints::load(&core.sessions_home(), &session, seq) {
                Ok(cp) => match vak_core::checkpoints::restore(core.cwd(), &cp) {
                    Ok((restored, deleted)) => {
                        println!("restored {restored} files, removed {deleted} (checkpoint {seq})");
                        0
                    }
                    Err(e) => {
                        eprintln!("error: restore failed: {e}");
                        1
                    }
                },
                Err(e) => {
                    eprintln!("error: checkpoint not found: {e}");
                    2
                }
            }
        }
    }
}

fn latest_session_id(core: &Core) -> Option<String> {
    let dir = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd());
    let mut rows: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((
                meta.modified().ok()?,
                e.file_name().to_string_lossy().into_owned(),
            ))
        })
        .collect();
    rows.sort_by_key(|(m, _)| std::cmp::Reverse(*m));
    rows.first()
        .map(|(_, name)| name.trim_end_matches(".jsonl").to_string())
}
#[tokio::main]
async fn main() {
    let internal = std::env::args_os().nth(1);
    #[cfg(target_os = "linux")]
    {
        if internal.as_deref()
            == Some(std::ffi::OsStr::new(
                vak_tools::landlock::SANDBOX_SUBCOMMAND,
            ))
        {
            std::process::exit(vak_tools::landlock::runner_main(
                std::env::args_os().skip(2),
            ));
        }
    }
    if internal.as_deref() == Some(std::ffi::OsStr::new(vak_tools::broker::WORKER_SUBCOMMAND)) {
        std::process::exit(vak_tools::broker::worker_main().await);
    }
    let cli = Cli::parse();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // User-level secrets always load. The PROJECT .env is only loaded for
    // trusted workspaces: a cloned repository must not be able to inject
    // VAKCODER_*_BASE_URL (credential redirection) or other env on first
    // run.
    if let Some(home) = std::env::var_os("HOME") {
        vak_config::load_env_file(&std::path::PathBuf::from(home).join(".vakcoder/.env"));
    }

    let code = match cli.command {
        None => {
            let trusted = resolve_trust(&cwd, false, true);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            run_tui(cwd, trusted).await
        }
        Some(Command::Tui { trust }) => {
            let trusted = resolve_trust(&cwd, trust, true);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            run_tui(cwd, trusted).await
        }
        Some(Command::Exec {
            prompt,
            model,
            provider,
            max_turns,
            json,
            yes,
            permission_mode,
            worktree,
            session,
            goal,
            criteria,
            trust,
        }) => {
            let trusted = resolve_trust(&cwd, trust, false);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            run_exec(
                cwd,
                prompt,
                model,
                provider,
                max_turns,
                json,
                yes,
                permission_mode,
                worktree,
                session,
                goal,
                criteria,
                trusted,
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
        Some(Command::Memory) => run_memory_list(cwd),
        Some(Command::SkillsReview { action }) => run_skills_review(cwd, action),
        Some(Command::Checkpoints { action }) => run_checkpoints(cwd, action).await,
        Some(Command::Telegram { server, token }) => run_telegram(server, token).await,
        Some(Command::Flow { action }) => run_flow(cwd, action).await,
        Some(Command::Plan {
            task,
            yes,
            worktree,
            trust,
        }) => {
            let trusted = resolve_trust(&cwd, trust, false);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            run_plan(cwd, task, yes, worktree, trusted).await
        }
        Some(Command::Eval {
            report,
            live,
            provider,
            model,
        }) => run_eval(report, live, provider, model).await,
        Some(Command::Serve {
            port,
            gateway,
            trust,
        }) => {
            let trusted = resolve_trust(&cwd, trust, false);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            run_serve(cwd, port, gateway, trusted).await
        }
    };
    std::process::exit(code);
}

// ---------------------------------------------------------------------------
// Workspace trust: a project's .vakcoder/config.toml and .env can grant
// execution power (permission mode, allow rules, hooks, MCP servers, base
// URL redirection). First use of an untrusted workspace demotes those keys
// until the user confirms — per-directory, remembered under ~/.vakcoder.
// ---------------------------------------------------------------------------

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn trust_marker_path(cwd: &std::path::Path) -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".vakcoder/trusted")
            .join(format!("{:016x}", fnv1a(cwd.to_string_lossy().as_bytes())))
    })
}

fn resolve_trust(cwd: &std::path::Path, flag: bool, interactive: bool) -> bool {
    if !vak_config::project_path(cwd).is_file() && !cwd.join(".env").is_file() {
        return true;
    }
    if flag {
        return true;
    }
    if let Some(marker) = trust_marker_path(cwd)
        && marker.is_file()
    {
        return true;
    }
    if interactive && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!();
        eprintln!(
            "This directory ({}) contains a project-level VakCoder",
            cwd.display()
        );
        eprintln!("config (.vakcoder/config.toml) and/or .env that can run commands,");
        eprintln!("auto-approve tools, or redirect API traffic.");
        eprint!("Trust this workspace? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_ok()
            && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
            && let Some(marker) = trust_marker_path(cwd)
            && let Some(parent) = marker.parent()
            && std::fs::create_dir_all(parent).is_ok()
            && std::fs::write(&marker, cwd.to_string_lossy().as_bytes()).is_ok()
        {
            return true;
        }
    } else if !flag {
        eprintln!(
            "note: workspace {} is untrusted; project permission/allow/hooks/mcp/base-url settings are ignored (pass --trust to apply)",
            cwd.display()
        );
    }
    false
}

fn print_config_warnings(core: &Core) {
    for w in &core.config().warnings {
        eprintln!("warning: {w}");
    }
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
        FlowAction::Adopt { from, name, force } => {
            let flows_dir = cwd.join(".vakcoder/flows");
            let out_path = flows_dir.join(format!("{name}.toml"));
            if out_path.exists() && !force {
                eprintln!(
                    "error: {} exists (use --force to overwrite)",
                    out_path.display()
                );
                return 2;
            }
            let adopted = if std::path::Path::new(&from).is_file() {
                // Ledger JSON path.
                let body = std::fs::read_to_string(&from).unwrap_or_default();
                vak_flow::adopt::from_flow_state(&body, &name, &from)
            } else {
                // Session id: extract settled bash commands.
                let core = match Core::new(cwd.clone()) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                };
                let path = vak_session::SessionPath::new_session_file(
                    &core.sessions_home(),
                    core.cwd(),
                    &from,
                );
                let Ok(log) = vak_session::SessionLog::open(path) else {
                    eprintln!("error: session '{from}' not found in this workspace");
                    return 2;
                };
                let cmds = log.settled_bash_commands();
                vak_flow::adopt::from_green_commands(
                    &name,
                    &format!("adopted from session {from}"),
                    &cmds,
                )
            };
            match adopted {
                Ok(a) => {
                    if std::fs::create_dir_all(&flows_dir).is_err() {
                        eprintln!("error: cannot create {}", flows_dir.display());
                        return 2;
                    }
                    if let Err(e) = std::fs::write(&out_path, a.toml) {
                        eprintln!("error: write failed: {e}");
                        return 2;
                    }
                    println!("✓ adopted → {}", out_path.display());
                    for w in a.warnings {
                        println!("  warning: {w}");
                    }
                    println!("  next: vakcoder flow check {name} && vakcoder flow run {name}");
                    0
                }
                Err(e) => {
                    eprintln!("error: adopt failed: {e}");
                    1
                }
            }
        }
        FlowAction::Diff { a, b } => {
            let (ra, rb) = (
                std::fs::read_to_string(&a).unwrap_or_default(),
                std::fs::read_to_string(&b).unwrap_or_default(),
            );
            match vak_flow::adopt::diff_flow_states(&ra, &rb) {
                Ok(report) => {
                    print!("{report}");
                    if report.contains("identical") { 0 } else { 1 }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
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
        FlowAction::Run {
            name,
            resume,
            yes,
            provider,
            model,
            accept_drift,
            trust,
        } => {
            let trusted = resolve_trust(&cwd, trust, false);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            run_flow_exec(
                cwd,
                name,
                resume,
                accept_drift,
                yes,
                provider,
                model,
                trusted,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_flow_exec(
    cwd: PathBuf,
    name: String,
    resume: bool,
    accept_drift: bool,
    yes: bool,
    provider_flag: Option<String>,
    model_flag: Option<String>,
    trusted: bool,
) -> i32 {
    let core = match Core::new_with_trust(cwd.clone(), trusted) {
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

    // Plan preview (doc 27 Phase G): show the shape before any effect.
    if let Ok(layers) = vak_flow::parse::layers(&flow) {
        let rendered: Vec<String> = layers.iter().map(|l| l.join(", ")).collect();
        eprintln!("plan: {}", rendered.join(" | "));
    }

    if let Some(p) = provider_flag {
        core.set_provider(p);
    }
    if let Some(m) = model_flag {
        core.set_model(m);
    }
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

    let engine = match vak_core::build_engine_with(core.config(), &core.extra_allow_snapshot()) {
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

    // Recovery audit (docs/design/27 Phase E): classify the snapshot vs
    // the live flow file BEFORE touching anything. Drift fails closed
    // unless explicitly accepted; the frozen snapshot always wins.
    if resume {
        let snapshot_body = std::fs::read_to_string(&state_path).unwrap_or_default();
        match vak_flow::adopt::recovery_audit(&snapshot_body, Some(&toml_str)) {
            Ok((snapshot, action)) => match action {
                "resume" => println!("[recovery-audit] snapshot={snapshot} action=resume"),
                "repair" if !accept_drift => {
                    eprintln!(
                        "[recovery-audit] snapshot={snapshot} — live flow file drifted from the frozen definition\n  \
                         resume executes the FROZEN copy; pass --accept-drift to acknowledge."
                    );
                    return 2;
                }
                _ => println!("[recovery-audit] snapshot={snapshot} action={action}"),
            },
            Err(e) => {
                eprintln!("error: cannot read run ledger for audit: {e}");
                return 2;
            }
        }
    }

    let mut state = load_state(&state_path, &name, &toml_str);

    let deps = vak_flow::ExecutorDeps {
        provider,
        system_prompt: core.system_prompt(),
        model: core.effective_model(),
        tools: core.agent_tools(),
        read_only_tools: core.agent_read_only_tools(),
        max_turns: core.effective_max_turns(),
        permission: Some(std::sync::Arc::new(engine)),
        mode: match core.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        },
        approver,
        sandbox: core.agent_sandbox(),
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
    // Layer-aware progress strip (doc 27 Phase G): prefix ✓/✗/⊘ events
    // with their topological layer, other lines verbatim.
    let layer_of: std::collections::HashMap<String, usize> = match vak_flow::parse::layers(&flow) {
        Ok(layers) => layers
            .iter()
            .enumerate()
            .flat_map(|(li, l)| l.iter().map(move |id| (id.clone(), li + 1)))
            .collect(),
        Err(_) => Default::default(),
    };
    let total_layers = layer_of.values().copied().max().unwrap_or(0);

    let runner = tokio::spawn(async move { executor.run(&flow, &mut state, cancel, tx).await });
    while let Some(line) = rx.recv().await {
        let marker = line
            .strip_prefix('✓')
            .or_else(|| line.strip_prefix('✗'))
            .or_else(|| line.strip_prefix('⊘'));
        if let Some(rest) = marker {
            let id = rest.trim();
            let layer = layer_of.get(id).copied().unwrap_or(0);
            eprintln!("[L{}/{}] {}", layer, total_layers, line);
        } else {
            eprintln!("{line}");
        }
    }

    match runner.await {
        Ok(outcome) => match outcome {
            vak_flow::FlowOutcome::Completed { outputs } => {
                // Snapshot from the persisted ledger (state moved into the runner).
                if let Ok(body) = std::fs::read_to_string(&state_path)
                    && let Ok(st) = serde_json::from_str::<vak_flow::FlowState>(&body)
                {
                    let snap = vak_flow::graph::graph_snapshot(&st);
                    eprintln!(
                        "── snapshot: {} completed / {} failed / {} skipped · {} layer(s)",
                        snap.completed, snap.failed, snap.skipped, snap.layers_total
                    );
                }
                eprintln!("── flow completed · state {}", state_path.display());
                for (id, out) in outputs {
                    println!("[{id}]\n{out}\n");
                }
                0
            }
            vak_flow::FlowOutcome::Failed { node, reason, .. } => {
                eprintln!("── flow failed at '{node}': {reason}");
                eprintln!("   resume with: VakCoder flow run {name} --resume");
                1
            }
            vak_flow::FlowOutcome::Aborted => {
                eprintln!("── flow aborted · resume with: VakCoder flow run {name} --resume");
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

async fn run_tui(cwd: PathBuf, trusted: bool) -> i32 {
    let core = match Core::new_with_trust(cwd.clone(), trusted) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    print_config_warnings(&core);
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
    worktree: bool,
    resume_session: Option<String>,
    goal: Option<String>,
    criteria: Vec<String>,
    trusted: bool,
) -> i32 {
    let mut effective_cwd = cwd.clone();
    let mut created_worktree: Option<vak_core::worktree::Worktree> = None;
    if worktree {
        match vak_core::worktree::create(&cwd, &format!("exec-{}", timestamp_id())) {
            Ok(wt) => {
                eprintln!("▸ isolated worktree: {} ({})", wt.path.display(), wt.branch);
                effective_cwd = wt.path.clone();
                created_worktree = Some(wt);
            }
            Err(e) => {
                eprintln!("error: worktree isolation failed: {e}");
                return 2;
            }
        }
    }
    let core = match Core::new_with_trust(effective_cwd.clone(), trusted) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    print_config_warnings(&core);
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

    let session = match resume_session {
        Some(sid) => match core.open_session(&sid).await {
            Ok(s) => {
                eprintln!("▸ resuming session {sid}");
                s
            }
            Err(e) => {
                eprintln!("error: cannot open session '{sid}': {e}");
                return 2;
            }
        },
        None => match core.start_session().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        },
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
        let fut = async {
            if let Some(objective) = goal.as_deref() {
                core.run_goal_turn_with(
                    session,
                    &prompt,
                    objective,
                    criteria.clone(),
                    cancel,
                    approver,
                    None,
                    None,
                    tx,
                )
                .await
            } else {
                core.run_turn_with(session, &prompt, cancel, approver, None, None, tx)
                    .await
            }
        };
        fut.await
    });

    let mut total_in: u64 = 0;
    let mut total_out: u64 = 0;
    let mut tool_args: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
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
            AgentEvent::ToolCallStart {
                id,
                name,
                args_json,
            } => {
                tool_args.insert(id, (name.clone(), args_json.clone()));
                eprintln!(
                    "▸ {} {}",
                    name,
                    vak_tui::app::summarize_args(&name, &args_json)
                );
            }
            AgentEvent::ToolCallEnd {
                id,
                name,
                is_error,
                result_preview,
            } => {
                eprintln!("{} {name}", if is_error { "✗" } else { "✓" });
                let stored = tool_args.remove(&id);
                if name == "edit"
                    && !is_error
                    && let Some((_, args)) = &stored
                    && let Some(diff) = vak_tui::app::edit_diff_text(
                        args,
                        &vak_tui::theme::Theme::from_name("plain"),
                        10,
                    )
                {
                    for line in vak_tui::markdown::strip_ansi(&diff).lines() {
                        eprintln!("  {line}");
                    }
                } else if is_error
                    && let Some(prev) = result_preview
                    && !prev.trim().is_empty()
                {
                    let tail: Vec<&str> = prev
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .rev()
                        .take(6)
                        .collect();
                    for line in tail.iter().rev() {
                        eprintln!("  │ {line}");
                    }
                }
            }
            AgentEvent::RetryScheduled {
                attempt,
                delay_ms,
                reason,
            } => {
                eprintln!("⟳ [{attempt}] backing off {delay_ms}ms — {reason}");
            }
            AgentEvent::ContextCompacting { estimated_tokens } => {
                eprintln!("◌ compacting context (~{estimated_tokens} tokens)");
            }
            AgentEvent::ContextCompacted {
                before_tokens,
                after_tokens,
                ..
            } => {
                eprintln!("◌ compacted ~{before_tokens} → ~{after_tokens} tokens");
            }
            AgentEvent::TurnEnd { usage } => {
                total_in += usage.input_tokens;
                total_out += usage.output_tokens;
            }
            _ => {}
        }
    }

    let outcome = match runner.await {
        Ok(Ok((o, _session))) => o,
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
        "\n── {} · Σ tokens in {total_in} / out {total_out} · session {}",
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
        if let Some(wt) = created_worktree {
            let _ = vak_core::worktree::remove(&cwd, &wt);
        }
        return 1;
    }
    if let Some(wt) = created_worktree {
        eprintln!(
            "▸ worktree kept for inspection: {} (branch {}) — remove with git worktree remove",
            wt.path.display(),
            wt.branch
        );
    }
    0
}

fn timestamp_id() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

fn run_config_dump(cwd: PathBuf) {
    match Core::new(cwd.clone()) {
        Ok(core) => {
            println!("# VakCoder effective config");
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
            let f = &core.config().finops;
            println!(
                "finops           = run_cap {} · day_cap {} · overrides {}",
                f.max_run_usd
                    .map(|v| format!("${v:.2}"))
                    .unwrap_or_else(|| "none".into()),
                f.max_day_usd
                    .map(|v| format!("${v:.2}"))
                    .unwrap_or_else(|| "none".into()),
                f.price_overrides.len(),
            );
            println!(
                "goal             = handoff_reset {} · max_audit_blocks {}",
                core.config().goal.handoff_reset,
                core.config().goal.max_audit_blocks,
            );
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
    let dir = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd());
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

async fn run_plan(cwd: PathBuf, task: String, yes: bool, worktree: bool, trusted: bool) -> i32 {
    let mut effective_cwd = cwd.clone();
    if worktree {
        match vak_core::worktree::create(&cwd, &format!("plan-{}", timestamp_id())) {
            Ok(wt) => {
                eprintln!("▸ isolated worktree: {} ({})", wt.path.display(), wt.branch);
                effective_cwd = wt.path.clone();
            }
            Err(e) => {
                eprintln!("error: worktree isolation failed: {e}");
                return 2;
            }
        }
    }
    let core = match Core::new_with_trust(effective_cwd.clone(), trusted) {
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
    let engine = match vak_core::build_engine_with(core.config(), &core.extra_allow_snapshot()) {
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
        tools: core.agent_tools(),
        read_only_tools: core.agent_read_only_tools(),
        max_turns: core.effective_max_turns(),
        permission: Some(std::sync::Arc::new(engine)),
        mode: match core.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        },
        approver,
        sandbox: core.agent_sandbox(),
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

fn builtin_cases() -> Vec<vak_eval::EvalCase> {
    vak_eval::builtin_suite()
        .into_iter()
        .chain(vak_eval::general_suite())
        .collect()
}

async fn run_eval(
    report_path: Option<PathBuf>,
    live: bool,
    provider_flag: Option<String>,
    model_flag: Option<String>,
) -> i32 {
    let mut reports = Vec::new();
    let worker_exe = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            eprintln!("error: tool broker unavailable: {error}");
            return 2;
        }
    };

    if !live {
        for case in builtin_cases() {
            let r = vak_eval::run_case_brokered(&case, worker_exe.clone()).await;
            println!(
                "{:<24} {:>6}  in {:>5} / out {:>4}  {:>5}ms  {}",
                r.task_id,
                if r.passed { "PASS" } else { "FAIL" },
                r.tokens_in,
                r.tokens_out,
                r.duration_ms,
                r.error.as_deref().unwrap_or("")
            );
            reports.push(r);
        }
        // Deterministic context-quality gate (doc 27 Phase C) — no model
        // calls; packet-accounting properties over real compaction.
        let card = match vak_eval::run_context_scorecard() {
            Ok(card) => card,
            Err(e) => {
                eprintln!("context scorecard harness error: {e}");
                return 1;
            }
        };
        println!("{card}");
        if !card.passed() {
            eprintln!("context scorecard FAILED");
            return 1;
        }
    } else {
        let core = match Core::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        if let Some(p) = provider_flag {
            core.set_provider(p);
        }
        if let Some(m) = model_flag {
            core.set_model(m);
        }
        let provider = match core.provider() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        let model = core.effective_model().clone();
        eprintln!("live eval against {} / {model}", core.effective_provider());
        for case in &vak_eval::live_suite() {
            let r = vak_eval::run_case_with_provider_brokered(
                case,
                provider.clone(),
                &model,
                worker_exe.clone(),
            )
            .await;
            println!(
                "{:<24} {:>6}  in {:>5} / out {:>4}  {:>5}ms  {}",
                r.task_id,
                if r.passed { "PASS" } else { "FAIL" },
                r.tokens_in,
                r.tokens_out,
                r.duration_ms,
                r.error.as_deref().unwrap_or("")
            );
            reports.push(r);
        }
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

async fn run_serve(cwd: PathBuf, port: u16, gateway: bool, trusted: bool) -> i32 {
    let core = match Core::new_with_trust(cwd, trusted) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    print_config_warnings(&core);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    match vak_server::serve_with(core, addr, gateway).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

async fn run_telegram(server: String, token_flag: Option<String>) -> i32 {
    // Env-first so the gateway token stays out of `ps`/plist arguments.
    let token = match token_flag {
        Some(t) => t,
        None => vak_config::get_var("VAKCODER_GATEWAY_TOKEN").unwrap_or_else(|| {
            eprintln!("error: gateway token missing — set VAKCODER_GATEWAY_TOKEN in ~/.vakcoder/.env or pass --token");
            String::new()
        }),
    };
    if token.is_empty() {
        return 2;
    }
    // .env-aware lookup so the bot token never has to be exported by hand.
    let Some(bot_token) = vak_config::get_var("TELEGRAM_BOT_TOKEN") else {
        eprintln!("error: TELEGRAM_BOT_TOKEN is not set (put it in .env or ~/.vakcoder/.env)");
        return 2;
    };
    let api_base = vak_config::get_var("TELEGRAM_API_BASE")
        .unwrap_or_else(|| "https://api.telegram.org".to_string());
    // Single-instance guard keyed by bot token: a second local bridge
    // fails fast with the holder's identity instead of flapping 409s.
    let locks_dir = vak_config::get_var("VAKCODER_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".vakcoder")))
        .map(|home| home.join("locks"));
    let bridge = vak_server::telegram::TelegramBridge {
        api_base,
        bot_token: bot_token.clone(),
        gateway_url: server.trim_end_matches('/').to_string(),
        gateway_token: token,
        locks_dir,
    };
    println!(
        "telegram bridge: {} -> {}",
        bridge.api_base, bridge.gateway_url
    );
    match bridge.run().await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
