use clap::Parser;
use futures::StreamExt;
use std::path::PathBuf;

mod admin_open;
mod backup;
mod cli;
mod connect;
mod memory;
mod self_release;
mod tasks;

use cli::{CheckpointAction, Cli, Command, FlowAction, SelfAction, SkillsReviewAction};
use vak_client::{Client, CreateSession, StartRun};
use vak_domain::{Event, PermissionMode, ProjectContext, RunStatus, SandboxMode, SessionContract};

#[tokio::main]
async fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == vak_tools::broker::WORKER_SUBCOMMAND)
    {
        std::process::exit(vak_tools::broker::worker_main().await);
    }
    let cli = Cli::parse();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let code = match cli.command {
        None => serve_tui(cwd).await,
        #[cfg(feature = "tui")]
        Some(Command::Tui { .. }) => serve_tui(cwd).await,
        Some(Command::Exec {
            prompt,
            model,
            provider,
            permission_mode,
            session,
            json,
            ..
        }) => run_exec(cwd, prompt, model, provider, permission_mode, session, json).await,
        Some(Command::Self_ { action }) => match action {
            SelfAction::Install { prefix, no_service } => {
                self_release::run_install(prefix, no_service)
            }
            SelfAction::ServicesSync { names } => self_release::run_services_sync(names),
            SelfAction::Status => self_release::run_status(),
            SelfAction::Uninstall {
                yes,
                purge,
                no_service,
            } => self_release::run_uninstall(yes, purge, no_service),
            SelfAction::Update { url, yes } => self_release::run_update(&url, yes),
        },
        Some(Command::Config { .. }) => config_dump(cwd),
        Some(Command::Sessions) => sessions(cwd).await,
        Some(Command::Memory { action }) => memory::run_memory(cwd, action).await,
        Some(Command::Tasks { action }) => tasks::run_tasks(cwd, action).await,
        Some(Command::Backup { action }) => backup::run_backup(cwd, action).await,
        Some(Command::Checkpoints { action }) => checkpoints(cwd, action).await,
        Some(Command::SkillsReview { action }) => skills(action).await,
        Some(Command::Flow { action }) => flow(cwd, action).await,
        Some(Command::Eval { report, live, .. }) => eval(report, live).await,
        Some(Command::Doctor { .. }) => doctor().await,
        Some(Command::Connect {
            profile,
            url,
            token,
            save,
        }) => connect_cmd(profile, url, token, save).await,
        Some(Command::Serve { port, .. }) => serve(port, cwd).await,
        Some(Command::Admin { print }) => admin_open::run(print),
    };
    std::process::exit(code);
}

async fn client_for() -> Result<Client, String> {
    let resolved = connect::discover(None, None, None)?;
    let client = Client::new(resolved.url, resolved.token).map_err(|e| e.to_string())?;
    client.version().await.map_err(|e| e.to_string())?;
    client.health().await.map_err(|e| e.to_string())?;
    Ok(client)
}

async fn project_for(client: &Client, cwd: &std::path::Path) -> Result<ProjectContext, String> {
    client
        .projects()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|project| {
            std::fs::canonicalize(&project.root).ok() == std::fs::canonicalize(cwd).ok()
        })
        .ok_or_else(|| {
            format!(
                "workspace is not registered with Runtime: {}",
                cwd.display()
            )
        })
}

async fn run_exec(
    cwd: PathBuf,
    prompt: String,
    model: Option<String>,
    provider: Option<String>,
    mode: Option<String>,
    session: Option<String>,
    json: bool,
) -> i32 {
    let client = match client_for().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match project_for(&client, &cwd).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let config = match client.config(&project.id).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error loading config: {e}");
            return 2;
        }
    };
    let provider = provider
        .or_else(|| {
            config
                .values
                .get("provider")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "anthropic".into());
    let model = model
        .or_else(|| {
            config
                .values
                .get("model")
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "default".into());
    let permission_mode = match mode.as_deref().unwrap_or("read-only") {
        "read-only" | "readonly" => PermissionMode::ReadOnly,
        "workspace-write" | "workspacewrite" => PermissionMode::WorkspaceWrite,
        "full-access" | "fullaccess" => PermissionMode::FullAccess,
        value => {
            eprintln!("error: unknown permission mode {value}");
            return 2;
        }
    };
    let session_id = match session {
        Some(value) => match value.parse() {
            Ok(id) => id,
            Err(e) => {
                eprintln!("error: invalid session: {e}");
                return 2;
            }
        },
        None => {
            let contract = SessionContract {
                provider,
                model,
                route_ladder: Vec::new(),
                system_prompt: "You are vakcoder, a coding agent.".into(),
                permission_mode,
                sandbox: SandboxMode::Seatbelt,
                tool_catalogue_revision: "runtime".into(),
                context_limit: 32_000,
                budget_ceiling: None,
            };
            match client
                .create_session(&CreateSession {
                    session_id: None,
                    project_id: project.id.clone(),
                    contract,
                })
                .await
            {
                Ok(s) => s.session_id,
                Err(e) => {
                    eprintln!("error creating session: {e}");
                    return 1;
                }
            }
        }
    };
    match client
        .exec(&StartRun {
            run_id: None,
            session_id,
            project_id: project.id,
            input: prompt,
        })
        .await
    {
        Ok(started) => {
            let run_id = started.run.run_id.clone();
            let mut events = match client.events(&run_id).await {
                Ok(stream) => stream,
                Err(error) => {
                    eprintln!("run {} started but event stream failed: {error}", run_id);
                    return 1;
                }
            };
            let mut output_seen = false;
            while let Some(event) = events.next().await {
                match event {
                    Ok(vak_client::ServerEvent::Domain(Event::RunOutput { delta, .. })) => {
                        output_seen = true;
                        if !json {
                            print!("{delta}");
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                    }
                    Ok(vak_client::ServerEvent::Domain(Event::RunFinished {
                        status,
                        output,
                        ..
                    })) => {
                        if json {
                            println!(
                                "{}",
                                serde_json::json!({
                                    "run_id": run_id,
                                    "status": status,
                                    "output": output
                                })
                            );
                        } else {
                            if !output_seen && !output.is_empty() {
                                print!("{output}");
                            }
                            println!(
                                "\n── {}",
                                if status == RunStatus::Completed {
                                    "completed"
                                } else {
                                    "finished"
                                }
                            );
                        }
                        return if status == RunStatus::Completed { 0 } else { 1 };
                    }
                    Ok(vak_client::ServerEvent::Domain(Event::ApprovalRequested {
                        approval_id,
                        ..
                    })) => {
                        eprintln!("approval required: {approval_id}");
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("run {run_id} event stream failed: {error}");
                        return 1;
                    }
                }
            }
            eprintln!("run {run_id} ended without a terminal event");
            1
        }
        Err(e) => {
            eprintln!("error starting run: {e}");
            1
        }
    }
}

async fn serve_tui(cwd: PathBuf) -> i32 {
    #[cfg(feature = "tui")]
    {
        match connect::discover(None, None, None) {
            Ok(r) => match vak_tui::data::ClientData::connect(
                &r.url,
                &r.token,
                cwd.to_string_lossy().into_owned(),
            )
            .await
            {
                Ok(data) => vak_tui::run(data, vak_tui::UiConfig { cwd }).await,
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            },
            Err(e) => {
                eprintln!("error: {e}");
                2
            }
        }
    }
    #[cfg(not(feature = "tui"))]
    {
        eprintln!("connect to a Runtime server with `vakcoder serve`");
        2
    }
}

async fn serve(port: u16, cwd: PathBuf) -> i32 {
    let token = match vak_config::SecretService::new(vak_config::paths::data_home())
        .get("VAKCODER_GATEWAY_TOKEN")
    {
        Ok(Some(t)) if !t.is_empty() => t,
        _ => match std::env::var("VAKCODER_GATEWAY_TOKEN") {
            Ok(t) if !t.is_empty() => t,
            _ => {
                eprintln!("error: VAKCODER_GATEWAY_TOKEN must be set");
                return 2;
            }
        },
    };
    let worker = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let runtime = match vak_runtime::Runtime::open_configured(
        vak_config::paths::data_home(),
        cwd,
        std::sync::Arc::new(vak_runtime::BrokerToolDispatcher::new(worker)),
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match vak_server::serve(runtime, ([127, 0, 0, 1], port).into(), token, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn config_dump(cwd: PathBuf) -> i32 {
    match vak_config::ConfigService::new(vak_config::paths::data_home(), cwd).load() {
        Ok(s) => {
            println!(
                "revision = {}\nprovider = {:?}\nmodel = {:?}\npermission = {:?}\nsandbox = {:?}",
                s.revision,
                s.config.provider.name,
                s.config.model.name,
                s.config.permission.mode,
                s.config.sandbox.backend
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

async fn sessions(cwd: PathBuf) -> i32 {
    let client = match client_for().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match project_for(&client, &cwd).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match client.sessions(Some(&project.id)).await {
        Ok(rows) => {
            for row in rows {
                println!("{} {:?} {:?}", row.id, row.status, row.created_at);
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

async fn checkpoints(cwd: PathBuf, action: CheckpointAction) -> i32 {
    let client = match client_for().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match project_for(&client, &cwd).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let session = match client
        .sessions(Some(&project.id))
        .await
        .ok()
        .and_then(|s| s.into_iter().max_by_key(|s| s.created_at.clone()))
    {
        Some(s) => s.id,
        None => {
            eprintln!("error: no sessions");
            return 2;
        }
    };
    match action {
        CheckpointAction::List { .. } => match client.checkpoints(&session).await {
            Ok(rows) => {
                for row in rows {
                    println!("{} {}", row.id, row.label);
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        CheckpointAction::Restore { seq, .. } => match client
            .checkpoints(&session)
            .await
            .ok()
            .and_then(|r| r.into_iter().nth(seq as usize))
        {
            Some(row) => match client.restore_checkpoint(&session, &row.id).await {
                Ok(_) => {
                    println!("restored {}", row.id);
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            },
            None => {
                eprintln!("error: checkpoint not found");
                2
            }
        },
    }
}

async fn skills(action: SkillsReviewAction) -> i32 {
    let client = match client_for().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match action {
        SkillsReviewAction::List => match client.skills(None, Some("proposed")).await {
            Ok(rows) => {
                for row in rows {
                    println!("{} {}", row.id, row.name);
                }
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        SkillsReviewAction::Promote { id } => client
            .promote_skill(&id)
            .await
            .map(|_| {
                println!("promoted {id}");
                0
            })
            .unwrap_or_else(|e| {
                eprintln!("error: {e}");
                1
            }),
        SkillsReviewAction::Reject { id } => client
            .reject_skill(&id)
            .await
            .map(|_| {
                println!("rejected {id}");
                0
            })
            .unwrap_or_else(|e| {
                eprintln!("error: {e}");
                1
            }),
    }
}

async fn flow(cwd: PathBuf, action: FlowAction) -> i32 {
    let client = match client_for().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let project = match project_for(&client, &cwd).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match action {
        FlowAction::List => client
            .flows(&project.id)
            .await
            .map(|rows| {
                for row in rows {
                    println!("{} {}", row.name, row.path);
                }
                0
            })
            .unwrap_or_else(|e| {
                eprintln!("error: {e}");
                1
            }),
        FlowAction::Check { name } => client
            .check_flow(&name, &project.id)
            .await
            .map(|row| {
                println!("{} valid", row.name);
                0
            })
            .unwrap_or_else(|e| {
                eprintln!("error: {e}");
                1
            }),
        FlowAction::Run { name, .. } => {
            let session = match client
                .sessions(Some(&project.id))
                .await
                .ok()
                .and_then(|s| s.into_iter().max_by_key(|s| s.created_at.clone()))
            {
                Some(s) => s.id,
                None => {
                    eprintln!("error: no sessions");
                    return 2;
                }
            };
            client
                .run_flow(
                    &name,
                    &vak_client::FlowRunRequest {
                        project_id: project.id,
                        session_id: session,
                        input: format!("run flow {name}"),
                        resume: false,
                    },
                )
                .await
                .map(|r| {
                    println!("run {} started", r.run.run_id);
                    0
                })
                .unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    1
                })
        }
    }
}

async fn eval(report: Option<PathBuf>, live: bool) -> i32 {
    let client = match client_for().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let cases = (0..if live { 3 } else { 8 })
        .map(|i| serde_json::json!({"name": format!("case-{i}")}))
        .collect();
    match client.eval(&vak_client::EvalRequest { cases }).await {
        Ok(result) => {
            if let Some(path) = report
                && let Err(e) = std::fs::write(
                    &path,
                    serde_json::to_vec_pretty(&result).unwrap_or_default(),
                )
            {
                eprintln!("error writing report: {e}");
                return 1;
            }
            println!("{}/{} passed", result.passed, result.total);
            if result.passed == result.total { 0 } else { 1 }
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

async fn doctor() -> i32 {
    let client = match client_for().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    match (
        client.health().await,
        client.version().await,
        client.diagnostics().await,
    ) {
        (Ok(h), Ok(v), Ok(d)) => {
            println!(
                "health: {:?}\nversion: {:?}\ndiagnostics: {}",
                h, v, d.status
            );
            0
        }
        _ => {
            eprintln!("Runtime health checks failed");
            1
        }
    }
}

async fn connect_cmd(
    profile: Option<String>,
    url: Option<String>,
    token: Option<String>,
    save: bool,
) -> i32 {
    match connect::discover(profile.as_deref(), url.as_deref(), token.as_deref()) {
        Ok(resolved) => {
            let client = match Client::new(resolved.url.clone(), resolved.token.clone()) {
                Ok(client) => client,
                Err(error) => {
                    eprintln!("error: {error}");
                    return 1;
                }
            };
            if let Err(error) = client.version().await {
                eprintln!("error: Runtime handshake failed: {error}");
                return 1;
            }
            if let Err(error) = client.health().await {
                eprintln!("error: Runtime handshake failed: {error}");
                return 1;
            }
            println!("connected: {}", resolved.source);
            if save && let Err(e) = connect::save_to_config(&resolved) {
                eprintln!("error saving connection: {e}");
                return 1;
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}
