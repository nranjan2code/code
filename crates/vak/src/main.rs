// The CLI's stdout and stderr are the person's.
#![allow(clippy::disallowed_macros)]
use std::io::Write as _;
use std::path::PathBuf;

use clap::Parser;
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, TurnOutcome};
use vak_core::Core;
use vak_llm::stream::StreamEvent;

mod agents_cli;
mod backup;
mod cli;
mod data;
mod digest;
mod doctor;
mod effects;
mod entities_cli;
mod format;
mod inbox;
mod install;
mod intent;
mod memory;
mod office;
mod plugins;
mod prompts;
mod question_prompt;
mod runs;
mod setup;
mod sync;
mod triggers;
mod update_check;

use cli::{CheckpointAction, Cli, Command, FlowAction, SkillsAction, SkillsReviewAction};

fn run_export(cwd: PathBuf, session_id: String, html: bool, out: Option<PathBuf>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    if vak_core::trash::is_trashed(&core.shared_scope(), &session_id) {
        eprintln!("error: session '{session_id}' is in the trash");
        return 1;
    }
    let home = core.scope().into_root();
    let path = vak_session::SessionPath::new_session_file(&home, core.cwd(), &session_id);
    let log = match vak_session::SessionLog::open_read_only(path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: could not open session '{session_id}': {e}");
            return 1;
        }
    };
    let msgs = log.derive_conversation();
    let md = vak_core::transcript_md::render_markdown(&msgs);
    let content = if html {
        vak_presentation::transcode_to_html(&format!("Session {session_id}"), &md)
    } else {
        md
    };

    if let Some(dest) = out {
        if let Err(e) = std::fs::write(&dest, content) {
            eprintln!("error writing to {}: {e}", dest.display());
            return 1;
        }
        println!("Exported session to {}", dest.display());
    } else {
        print!("{content}");
    }
    0
}

fn run_skills_review(cwd: PathBuf, action: SkillsReviewAction) -> i32 {
    let Some(core) = Core::new(cwd).ok() else {
        return 2;
    };
    match action {
        SkillsReviewAction::List => {
            let proposals = vak_core::learning::list_proposals(&core.scope(), core.cwd());
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
            match vak_core::learning::promote(&core.scope(), core.cwd(), &id) {
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
            match vak_core::learning::reject(&core.scope(), core.cwd(), &id) {
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

fn run_skills(cwd: PathBuf, action: SkillsAction) -> i32 {
    match action {
        SkillsAction::Validate { path, json } => {
            let mut files = Vec::new();
            let roots = path.map_or_else(
                || {
                    vec![
                        vak_config::scope::WorkspaceScope::new(&cwd).skills(),
                        vak_config::paths::data_home().join("skills"),
                    ]
                },
                |path| vec![path],
            );
            for root in &roots {
                collect_skill_files(root, &mut files);
            }
            files.sort();
            files.dedup();
            if files.is_empty() {
                let locations = roots
                    .iter()
                    .map(|root| root.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                eprintln!("no SKILL.md files found below {locations}");
                return 1;
            }
            let mut failed = false;
            let mut reports = Vec::new();
            for file in files {
                match vak_core::skills::validate(&file) {
                    Ok((skill, warnings)) => reports.push(serde_json::json!({
                        "path": file,
                        "name": skill.name,
                        "valid": true,
                        "warnings": warnings,
                    })),
                    Err(error) => {
                        failed = true;
                        reports.push(serde_json::json!({
                            "path": file,
                            "valid": false,
                            "error": error,
                        }));
                    }
                }
            }
            if json {
                println!("{}", serde_json::Value::Array(reports));
            } else {
                for report in &reports {
                    let path = report["path"].as_str().unwrap_or("unknown");
                    if report["valid"] == true {
                        println!("ok {} ({})", report["name"], path);
                        for warning in report["warnings"].as_array().into_iter().flatten() {
                            println!("warning: {}", warning.as_str().unwrap_or("unknown"));
                        }
                    } else {
                        println!("invalid {}: {}", path, report["error"]);
                    }
                }
            }
            i32::from(failed)
        }
    }
}

fn collect_skill_files(root: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    if root.is_file() {
        if root.file_name().is_some_and(|name| name == "SKILL.md") {
            files.push(root.to_path_buf());
        }
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        collect_skill_files(&entry.path(), files);
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
            match vak_core::checkpoints::list(&core.scope(), &sid) {
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
            match vak_core::checkpoints::load(&core.scope(), &session, seq) {
                Ok(cp) => match core
                    .objects()
                    .map_err(std::io::Error::other)
                    .and_then(|objects| {
                        vak_core::checkpoints::restore(core.cwd(), objects.as_ref(), &cp)
                    }) {
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
    let mut session_dirs = Vec::new();
    let direct = core.scope().sessions_dir(core.cwd());
    if direct.exists() {
        session_dirs.push(direct);
    }
    let shared = core.shared_scope().into_root();
    if let Ok(agents) = std::fs::read_dir(shared.join("agents")) {
        for agent in agents.flatten() {
            let s = vak_session::SessionPath::sessions_dir(&agent.path(), core.cwd());
            if s.exists() && !session_dirs.contains(&s) {
                session_dirs.push(s);
            }
        }
    }
    let trashed = vak_core::trash::trashed(&vak_config::scope::SharedScope::new(&shared));
    let mut rows: Vec<_> = Vec::new();
    for dir in session_dirs {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let maybe_m = (name.ends_with(".jsonl")
                    && !trashed.contains(name.trim_end_matches(".jsonl")))
                .then(|| e.metadata().ok().and_then(|meta| meta.modified().ok()))
                .flatten();
                if let Some(m) = maybe_m {
                    rows.push((m, name));
                }
            }
        }
    }
    rows.sort_by_key(|(m, _)| std::cmp::Reverse(*m));
    rows.first()
        .map(|(_, name)| name.trim_end_matches(".jsonl").to_string())
}
/// Every turn this binary runs is read in a terminal, so the system prompt
/// says so (docs/design/07-prompt.md). `run_serve` is the exception and
/// stamps its own surface.
/// Stamp the CLI surface and, with it, whether this invocation's approver
/// can answer a gate. These one-shot paths install `AutoApprove` under
/// `--yes` and `AutoDeny` otherwise, and the prompt is composed by
/// `start_session()` further down — so the flag has to be on the `Core`
/// before that, or the prompt advertises capabilities the run will refuse.
fn with_cli_surface(core: Core, approver_answerable: bool) -> Core {
    core.with_surface(vak_core::Surface::Cli)
        .with_approver_answerable(approver_answerable)
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
    if internal.as_deref()
        == Some(std::ffi::OsStr::new(
            vak_tools::broker::PERSISTENT_WORKER_SUBCOMMAND,
        ))
    {
        std::process::exit(vak_tools::broker::persistent_worker_main().await);
    }
    if internal.as_deref()
        == Some(std::ffi::OsStr::new(
            vak_delivery::worker::WORKER_SUBCOMMAND,
        ))
    {
        std::process::exit(vak_delivery::worker::run_stdio());
    }
    let cli = Cli::parse();
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd = cli
        .workspace
        .as_ref()
        .map(|w| {
            if w.is_absolute() {
                w.clone()
            } else {
                current_dir.join(w)
            }
        })
        .unwrap_or(current_dir);

    // User-level secrets always load. The PROJECT secret scope is only
    // loaded for trusted workspaces: a cloned repository must not be able
    // to inject VAK_*_BASE_URL (credential redirection) or other env on
    // first run.
    if let Some(env_path) = vak_config::user_env_path() {
        vak_config::load_env_file(&env_path);
    }

    // One subscriber per process: content-free JSON lines in the service's
    // log (plan M5). The full-screen terminal draws its own screen, so it
    // reports nothing on stderr.
    let service = match &cli.command {
        Some(Command::Serve { .. }) => Some("server"),
        Some(Command::Telegram { .. }) => Some("telegram"),
        Some(Command::Discord { .. }) => Some("discord"),
        Some(Command::Slack { .. }) => Some("slack"),
        Some(Command::Term { .. }) => None,
        _ => Some("cli"),
    };
    if let Some(service) = service {
        vak_telemetry::init(service);
    }

    let code = match cli.command {
        None => {
            use clap::CommandFactory as _;
            cli::Cli::command().print_help().ok();
            println!();
            println!(
                "vak is a headless CLI/automation runtime. Try `vak exec \"<prompt>\"` \
                 or `vak plan` — for an interactive GUI use the vak-desktop app."
            );
            0
        }
        Some(Command::Term {
            server,
            token,
            session,
        }) => {
            let server = server.unwrap_or_else(|| {
                format!("http://127.0.0.1:{}", vak_ops::OpsConfig::detect().port)
            });
            let token = token
                .filter(|t| !t.trim().is_empty())
                .or_else(|| vak_config::get_var("VAK_GATEWAY_TOKEN"));
            let opts = vak_terminal::TerminalOptions {
                session_id: session,
                server_url: Some(server),
                token,
                workspace_cwd: Some(cwd),
            };
            match vak_terminal::run_terminal(opts).await {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("vak term: {e}");
                    1
                }
            }
        }
        Some(Command::Exec {
            prompt,
            agent,
            model,
            provider,
            max_turns,
            json,
            yes,
            permission_mode,
            write_paths,
            worktree,
            session,
            managed,
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
                agent,
                model,
                provider,
                max_turns,
                json,
                yes,
                permission_mode,
                write_paths,
                worktree,
                session,
                managed,
                goal,
                criteria,
                trusted,
            )
            .await
        }
        Some(Command::Config { action }) => {
            // Each of these resolves workspace trust for itself, because a
            // reader must see the same layers a run would: an untrusted
            // project's `permission_mode` and allow rules are stripped by
            // the loader, so reporting them would describe a policy no run
            // uses.
            match action {
                None | Some(cli::ConfigAction::Dump) => {
                    run_config_dump(cwd);
                    0
                }
                Some(cli::ConfigAction::Permissions) => run_config_permissions(cwd),
                Some(cli::ConfigAction::SetMode { mode, scope }) => {
                    run_config_set_mode(cwd, &mode, scope)
                }
                Some(cli::ConfigAction::SetApproval { mode, scope }) => {
                    run_config_set_approval(cwd, &mode, scope)
                }
            }
        }
        Some(Command::Prompts { action }) => {
            // Reading and previewing must show what a real run would see, so
            // trust is resolved exactly as `exec` resolves it. An untrusted
            // workspace's own identity stays hidden here too.
            let trusted = resolve_trust(&cwd, false, false);
            prompts::run(cwd, action, trusted)
        }
        Some(Command::Sessions) => {
            run_sessions_list(cwd);
            0
        }
        Some(Command::Self_ { action }) => match action {
            cli::SelfAction::State { verify } => setup::run_state(verify),
            cli::SelfAction::Install { prefix, force } => install::run_install(prefix, force),
            cli::SelfAction::Reinstall { prefix, yes } => install::run_reinstall(prefix, yes),
            cli::SelfAction::Verify { prefix } => install::run_verify(prefix),
            cli::SelfAction::ServicesSync { prefix, names } => {
                install::run_services_sync(prefix, names)
            }
            cli::SelfAction::Status { prefix } => install::run_status(prefix),
            cli::SelfAction::Uninstall { prefix, yes, purge } => {
                install::run_uninstall(prefix, yes, purge)
            }
            cli::SelfAction::Update {
                prefix,
                url,
                yes,
                dry_run,
            } => match resolve_update_url(url) {
                Some(u) => install::run_update(prefix, &u, yes, dry_run),
                None => {
                    eprintln!(
                        "error: no release feed URL — pass `--url` or set `[update] url` in config"
                    );
                    2
                }
            },
        },
        Some(Command::Memory { action }) => memory::run_memory(cwd, action),
        Some(Command::Entities { action }) => entities_cli::run_entities(cwd, action),
        Some(Command::Agents { action }) => agents_cli::run_agents(cwd, action),
        Some(Command::Export {
            session_id,
            html,
            out,
        }) => run_export(cwd, session_id, html, out),
        Some(Command::SkillsReview { action }) => run_skills_review(cwd, action),
        Some(Command::Skills { action }) => run_skills(cwd, action),
        Some(Command::Plugins { action }) => plugins::run_plugins(cwd, action),
        Some(Command::Intent { action }) => intent::run_intent(cwd, action),
        Some(Command::Office { action }) => office::run_office(action).await,
        Some(Command::Commit { action }) => intent::run_commit(cwd, action),
        Some(Command::Grant {
            id,
            paths,
            tools,
            spend_usd,
            hours,
            permission,
            on_silence,
            after_hours,
        }) => intent::run_grant(
            cwd,
            id,
            paths,
            tools,
            spend_usd,
            hours,
            permission,
            on_silence,
            after_hours,
        ),
        Some(Command::Revoke { id }) => intent::run_revoke(cwd, id),
        Some(Command::Checkpoints { action }) => run_checkpoints(cwd, action).await,
        Some(Command::Telegram {
            server,
            token,
            bot_id,
        }) => run_telegram(server, token, bot_id).await,
        Some(Command::Discord {
            server,
            token,
            bot_id,
        }) => run_discord(server, token, bot_id).await,
        Some(Command::Slack {
            server,
            token,
            bot_id,
        }) => run_slack(server, token, bot_id).await,
        Some(Command::Flow { action }) => run_flow(cwd, action).await,
        Some(Command::Setup {
            action,
            no_browser,
            print_url,
            terminal,
            non_interactive,
        }) => match action {
            Some(cli::SetupAction::Status { json, prefix }) => setup::run_status(cwd, prefix, json),
            Some(cli::SetupAction::Seed) => setup::run_seed(),
            None if terminal || non_interactive => setup::run_terminal(cwd, non_interactive).await,
            None => setup::run_wizard(cwd, !no_browser && !print_url, print_url).await,
        },
        Some(Command::Doctor { trust, repair }) => {
            let trusted = resolve_trust(&cwd, trust, false);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            doctor::run_doctor(cwd, trusted, repair)
        }
        Some(Command::Backup { action }) => backup::run_backup(cwd, action),
        Some(Command::Digest { days }) => digest::run_digest(cwd, days),
        Some(Command::Triggers { action }) => triggers::run_triggers(cwd, action),
        Some(Command::Inbox { action }) => inbox::run_inbox(cwd, action),
        Some(Command::Data { action }) => data::run_data(cwd, action),
        Some(Command::Sync { action }) => sync::run_sync(cwd, action),
        Some(Command::Runs { action }) => runs::run_runs(cwd, action),
        Some(Command::Effects { action }) => effects::run_effects(cwd, action),
        Some(Command::Plan {
            task,
            yes,
            permission_mode,
            write_paths,
            worktree,
            trust,
        }) => {
            let trusted = resolve_trust(&cwd, trust, false);
            if trusted {
                vak_config::load_env_file(std::path::Path::new(".env"));
            }
            run_plan(
                cwd,
                task,
                yes,
                permission_mode,
                write_paths,
                worktree,
                trusted,
            )
            .await
        }
        Some(Command::Eval {
            report,
            generated,
            seed,
            offset,
            live,
            provider,
            model,
        }) => run_eval(report, generated, seed, offset, live, provider, model).await,
        Some(Command::Open {
            surface,
            port,
            print,
        }) => run_open(surface, port, print),
        Some(Command::Serve {
            port,
            host,
            gateway,
            trust,
        }) => {
            let serve_cwd = if gateway {
                vak_config::paths::gateway_workspace()
            } else {
                cwd
            };
            let trusted = resolve_trust(&serve_cwd, trust, false);
            if trusted {
                vak_config::load_env_file(&serve_cwd.join(".env"));
            }
            run_serve(serve_cwd, port, host, gateway, trusted).await
        }
    };
    std::process::exit(code);
}

// ---------------------------------------------------------------------------
// Workspace trust: a project's .vak/config.toml and secret scope can grant
// execution power (permission mode, allow rules, hooks, MCP servers, base
// URL redirection). First use of an untrusted workspace demotes those keys
// until the user confirms — per-directory, remembered under ~/.vak.
// ---------------------------------------------------------------------------

/// Release feed URL for `self update`: the flag when given, otherwise the
/// `[update] url` already used by the startup update check, so the two
/// paths can never point at different feeds.
///
/// Loaded untrusted on purpose. The feed names the binary that replaces this
/// one and supplies its own artifact checksums, so a project config must not
/// be able to redirect it; `load_with_trust` drops `[update] url` from an
/// untrusted project layer and the user's global value still applies.
fn resolve_update_url(flag: Option<String>) -> Option<String> {
    if let Some(u) = flag.filter(|u| !u.trim().is_empty()) {
        return Some(u);
    }
    let cwd = std::env::current_dir().ok()?;
    let trusted = trust_marker_path(&cwd).is_some_and(|marker| marker.exists());
    vak_config::load_with_trust(&cwd, trusted).ok()?.update.url
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn trust_marker_path(cwd: &std::path::Path) -> Option<PathBuf> {
    Some(
        vak_config::paths::data_home()
            .join("trusted")
            .join(format!("{:016x}", fnv1a(cwd.to_string_lossy().as_bytes()))),
    )
}

fn resolve_trust(cwd: &std::path::Path, flag: bool, interactive: bool) -> bool {
    if !vak_config::project_path(cwd).is_file()
        && !vak_config::credentials::scope_has_any(&cwd.join(".env"))
    {
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
            "This directory ({}) contains a project-level Vakyartha",
            cwd.display()
        );
        eprintln!("config (.vak/config.toml) and/or secrets that can run commands,");
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

pub(crate) fn print_config_warnings(core: &Core) {
    for w in &core.config().warnings {
        eprintln!("warning: {w}");
    }
}

fn flow_dirs(cwd: &std::path::Path) -> Vec<PathBuf> {
    vec![
        vak_config::scope::WorkspaceScope::new(cwd).flows(),
        vak_config::paths::data_home().join("flows"),
    ]
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
                println!("no flows found (.vak/flows/*.toml)");
                return 0;
            }
            for (name, path) in flows {
                println!("{name}  {}", path.display());
            }
            0
        }
        FlowAction::Adopt { from, name, force } => {
            let flows_dir = vak_config::scope::WorkspaceScope::new(&cwd).flows();
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
                    &core.scope().into_root(),
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
                    println!("  next: vak flow check {name} && vak flow run {name}");
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
    let core = match Core::new_with_trust(cwd.clone(), trusted).map(|c| with_cli_surface(c, yes)) {
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

    // Plan preview (docs/design/10-flows.md): show the shape before any effect.
    if let Ok(layers) = vak_flow::parse::layers(&flow) {
        let rendered: Vec<String> = layers.iter().map(|l| l.join(", ")).collect();
        eprintln!("plan: {}", rendered.join(" | "));
    }

    if provider_flag.is_some() || model_flag.is_some() {
        let route = core.effective_route();
        core.set_route(
            provider_flag.unwrap_or(route.provider),
            model_flag.unwrap_or(route.model),
        );
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

    let engine = match core.build_permission_engine(&core.extra_allow_snapshot()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // Each execution is its own run, and its checkpoint is keyed by it. A
    // resume is a new run, the next attempt, continuing from the newest
    // run of this flow's checkpoint.
    let flow_runs = core.scope().flow_runs();
    let trace = core.mint_trace(None);
    let (state_path, attempt) = if resume {
        match core.runs().of_flow(&name) {
            Ok(previous) => match previous.first() {
                Some(prior) => (
                    flow_runs.join(format!("{}.json", prior.id)),
                    prior.attempt + 1,
                ),
                None => {
                    eprintln!("error: no previous run to resume");
                    return 2;
                }
            },
            Err(e) => {
                eprintln!("error: run records unreadable: {e}");
                return 2;
            }
        }
    } else {
        (flow_runs.join(format!("{}.json", trace.run)), 1)
    };
    let checkpoint_path = flow_runs.join(format!("{}.json", trace.run));

    // Recovery audit (docs/design/10-flows.md): classify the snapshot vs
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

    let prepared = core.prepare_turn().await;
    let mut outcome =
        vak_intent::OutcomeSpec::from_reading(name.clone(), &vak_intent::Reading::default(), 0);
    outcome.max_turns = Some(core.effective_max_turns());
    let objects = match core.objects() {
        Ok(objects) => objects,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let deps = vak_flow::ExecutorDeps {
        objects,
        prompt_layers: Vec::new(),
        provider_route: core.effective_provider().to_string(),
        provider,
        system_prompt: prepared.system_prompt,
        node_prompt: Some(core.flow_node_prompt(core.capability_descriptors())),
        model: core.effective_model(),
        tools: prepared.tools,
        read_only_tools: prepared.read_only_tools,
        max_turns: core.effective_max_turns(),
        outcome: Some(outcome),
        max_retries: 0,
        retry_base_backoff_ms: 100,
        request_timeout: Some(std::time::Duration::from_secs(600)),
        circuit_breaker: None,
        run_retry_attempts: 0,
        run_retry_base_backoff_ms: 1000,
        dispatch_ceiling: 1,
        spend_gate: None,
        permission: Some(std::sync::Arc::new(engine)),
        mode: match core.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        },
        approval_mode: match core.effective_approval_mode() {
            vak_config::ApprovalMode::Ask => vak_agent::ApprovalMode::Ask,
            vak_config::ApprovalMode::ApproveSafe => vak_agent::ApprovalMode::ApproveSafe,
            vak_config::ApprovalMode::AutoApprove => vak_agent::ApprovalMode::AutoApprove,
        },
        approver,
        sandbox: core.agent_sandbox(),
        cwd: core.cwd().clone(),
        sessions_home: core.scope().into_root().clone(),
        parent_session_id,
        state_path: checkpoint_path.clone(),
        agent_identity: core.agent_identity().cloned(),
        conversation_context: core.conversation_context().cloned(),
        run: Some(vak_flow::FlowRun {
            runs: core.runs(),
            trace,
            attempt,
        }),
        work: None,
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
    // Layer-aware progress strip (docs/design/10-flows.md): prefix ✓/✗/⊘ events
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
                if let Ok(body) = std::fs::read_to_string(&checkpoint_path)
                    && let Ok(st) = serde_json::from_str::<vak_flow::FlowState>(&body)
                {
                    let snap = vak_flow::graph::graph_snapshot(&st);
                    eprintln!(
                        "── snapshot: {} completed / {} failed / {} skipped · {} layer(s)",
                        snap.completed, snap.failed, snap.skipped, snap.layers_total
                    );
                }
                eprintln!("── flow completed · state {}", checkpoint_path.display());
                for (id, out) in outputs {
                    println!("[{id}]\n{out}\n");
                }
                0
            }
            vak_flow::FlowOutcome::Failed { node, reason, .. } => {
                eprintln!("── flow failed at '{node}': {reason}");
                eprintln!("   resume with: vak flow run {name} --resume");
                1
            }
            vak_flow::FlowOutcome::Aborted => {
                eprintln!("── flow aborted · resume with: vak flow run {name} --resume");
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
        outcome: None,
        nodes: Default::default(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_exec(
    cwd: PathBuf,
    prompt: String,
    agent: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    max_turns: usize,
    _json: bool,
    yes: bool,
    permission_mode: Option<String>,
    write_paths: Vec<PathBuf>,
    worktree: bool,
    resume_session: Option<String>,
    managed: bool,
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
    let mut core = match Core::new_with_trust(effective_cwd.clone(), trusted)
        .map(|c| with_cli_surface(c, yes))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    if let Some(agent_id) = &agent
        && agent_id != "vak"
    {
        let profiles = match vak_server::agents::effective(&core) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: failed to load agents: {e}");
                return 2;
            }
        };
        let Some(profile) = profiles
            .into_iter()
            .find(|p| p.id == *agent_id || p.name.eq_ignore_ascii_case(agent_id))
        else {
            eprintln!(
                "error: agent '{agent_id}' not found. Run 'vak agents list' to view configured agents."
            );
            return 2;
        };
        if !profile.is_admissible() {
            eprintln!(
                "error: agent '{agent_id}' is {:?} and cannot execute runs",
                profile.lifecycle
            );
            return 2;
        }
        eprintln!("▸ agent: {} ({})", profile.name, profile.id);
        core = core.with_agent_identity(Some(profile.identity()));
    }
    print_config_warnings(&core);
    update_check::maybe_check_update(core.config());
    if provider.is_some() || model.is_some() {
        let route = core.effective_route();
        core.set_route(
            provider.unwrap_or(route.provider),
            model.unwrap_or(route.model),
        );
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
                if let Some(agent_id) = &agent
                    && let Some(h_agent) = s.header().and_then(|h| h.agent.as_ref())
                    && h_agent.id != *agent_id
                    && *agent_id != "vak"
                {
                    eprintln!(
                        "error: session '{sid}' was created for agent '{}', but you requested agent '{agent_id}'",
                        h_agent.id
                    );
                    return 2;
                }
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

    // Gates are auto-approved or auto-denied as before. A worker's question
    // is different: only a person can answer it, so with a terminal in front
    // of one it is asked there (docs/design/84-worker-questions-and-control.md).
    let gates: std::sync::Arc<dyn vak_agent::Approver> = if yes {
        std::sync::Arc::new(vak_agent::AutoApprove)
    } else {
        std::sync::Arc::new(vak_agent::AutoDeny)
    };
    let approver: Option<std::sync::Arc<dyn vak_agent::Approver>> = Some(std::sync::Arc::new(
        question_prompt::CliApprover::new(gates),
    ));
    let permission = if write_paths.is_empty() {
        None
    } else {
        match core.build_permission_engine(&core.extra_allow_snapshot()) {
            Ok(engine) => Some(std::sync::Arc::new(
                engine.restrict_write_paths(core.cwd(), &write_paths),
            )),
            Err(error) => {
                eprintln!("error: {error}");
                return 2;
            }
        }
    };

    // The runner consumes `core`; keep a handle for the post-turn
    // reflection seam.
    let reflection_core = core.clone();
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
                    permission,
                    None,
                    tx,
                )
                .await
            } else if managed {
                core.run_managed_turn_with(session, &prompt, cancel, approver, permission, None, tx)
                    .await
            } else {
                core.run_turn_with(session, &prompt, cancel, approver, permission, None, tx)
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
                eprintln!("▸ {} {}", name, format::summarize_args(&name, &args_json));
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
                    && let Some(diff) = format::edit_diff_text(args, 10)
                {
                    for line in format::strip_ansi(&diff).lines() {
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
            AgentEvent::RouteFallback {
                to_provider,
                to_model,
            } => {
                eprintln!("⤵ route fallback → {to_provider}/{to_model} (frozen ladder leg)");
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
                total_in += usage.prompt_tokens();
                total_out += usage.output_tokens;
            }
            AgentEvent::Sandbox(sb_ev) => {
                use vak_tools::sandbox_events::{SandboxEvent, fold_carriage_returns};
                match sb_ev {
                    SandboxEvent::ExecutionStarted {
                        tool,
                        code_preview,
                        scratch_dir,
                        ..
                    } => {
                        let first_line = code_preview.lines().next().unwrap_or("").trim();
                        let display_cmd = if first_line.len() > 60 {
                            format!("{}…", &first_line[..60])
                        } else {
                            first_line.to_string()
                        };
                        eprintln!("  ┌─ [sandbox:{tool}] {display_cmd}");
                        if !scratch_dir.is_empty() {
                            eprintln!("  │  scratch: {scratch_dir}");
                        }
                    }
                    SandboxEvent::Stdout { chunk, .. } => {
                        let folded = fold_carriage_returns(&chunk);
                        for line in folded.lines() {
                            if !line.trim().is_empty() {
                                eprintln!("  │  {line}");
                            }
                        }
                    }
                    SandboxEvent::Stderr { chunk, .. } => {
                        let folded = fold_carriage_returns(&chunk);
                        for line in folded.lines() {
                            if !line.trim().is_empty() {
                                eprintln!("  │! {line}");
                            }
                        }
                    }
                    SandboxEvent::OutputTruncated { .. } => {
                        eprintln!("  │! output truncated after the configured capture limit");
                    }
                    SandboxEvent::PackageInstalled { packages, .. } => {
                        eprintln!("  │  📦 packages: {}", packages.join(", "));
                    }
                    SandboxEvent::ArtifactGenerated {
                        path,
                        mime_type,
                        size_bytes,
                        ..
                    } => {
                        eprintln!("  │  📄 artifact: {path} ({size_bytes} B, {mime_type})");
                    }
                    SandboxEvent::ProcessTelemetry {
                        elapsed_ms,
                        memory_bytes,
                        ..
                    } => {
                        if memory_bytes > 0 {
                            let mb = memory_bytes as f64 / (1024.0 * 1024.0);
                            eprintln!("  │  ⏱ {}ms | RSS: {:.1}MB", elapsed_ms, mb);
                        }
                    }
                    SandboxEvent::ExecutionFinished {
                        exit_code,
                        duration_ms,
                        artifacts,
                        ..
                    } => {
                        let status_sym = if exit_code == 0 { "✓" } else { "✗" };
                        let mut summary = format!(
                            "  └─ {status_sym} finished in {duration_ms}ms (exit: {exit_code})"
                        );
                        if !artifacts.is_empty() {
                            summary.push_str(&format!(" [{} artifact(s)]", artifacts.len()));
                        }
                        eprintln!("{summary}");
                    }
                }
            }
            _ => {}
        }
    }

    let (outcome, session) = match runner.await {
        Ok(Ok((o, session))) => (o, Some(session)),
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
    // Post-turn reflection seam (docs/design/29 P1): the final assistant
    // text is already committed to the ledger, so no extra tail is passed.
    // Bounded so a stuck auxiliary stream cannot hang the exit.
    if let Some(session) = session.as_ref() {
        let pass = tokio::time::timeout(
            REFLECTION_CALL_TIMEOUT,
            reflection_core.reflect_after_turn(session, ""),
        )
        .await;
        if let Ok(outcome) = pass
            && let Some(line) = exec_reflection_line(&outcome)
        {
            eprintln!("{line}");
        }
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
    uuid::Uuid::now_v7().to_string()
}

/// Upper bound on one background reflection pass so a stuck auxiliary
/// stream cannot hang process exit.
const REFLECTION_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Footnote line for a post-turn reflection outcome: one dim line when the
/// pass persisted something or was skipped for a reason worth acting on
/// (budget), silent otherwise.
fn exec_reflection_line(outcome: &vak_core::reflection::ReflectionOutcome) -> Option<String> {
    match outcome {
        vak_core::reflection::ReflectionOutcome::Reflected { notes_added, .. } => {
            Some(format!("· reflected: {notes_added} note(s)"))
        }
        vak_core::reflection::ReflectionOutcome::Skipped { reason: "budget" } => {
            Some("· reflection skipped: budget cap reached".to_string())
        }
        _ => None,
    }
}

/// `vak config permissions` — the whole permission answer in one place.
///
/// The CLI could previously only dump the merged config, which prints the
/// mode but not what the engine actually evaluates. An operator debugging a
/// refusal had no terminal command that showed the rules, the approval mode,
/// and which capabilities the composed policy will refuse on this surface.
fn run_config_permissions(cwd: PathBuf) -> i32 {
    // Trust matters here: an untrusted project's `allow` rules are stripped
    // by the loader, so reading with the wrong trust would print a rule set
    // no run would ever use.
    let trusted = vak_core::trust::is_trusted(&cwd);
    let core = match Core::new_with_trust(cwd, trusted) {
        Ok(core) => core,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let (allow, ask, deny) = core.effective_permission_rules();
    println!("permission_mode  = {:?}", core.effective_permission_mode());
    println!(
        "approval_mode    = {}",
        core.effective_approval_mode().as_str()
    );
    println!("sandbox          = {}", core.effective_sandbox_name());
    println!("project_trusted  = {trusted}");
    for (label, list) in [("deny", &deny), ("ask", &ask), ("allow", &allow)] {
        if list.is_empty() {
            println!("{label:<16} = (none)");
        } else {
            println!("{label:<16} = {}", list.join(", "));
        }
    }
    let learned = core.extra_allow_snapshot();
    if !learned.is_empty() {
        println!("learned          = {}", learned.join(", "));
    }
    // The composed answer, not the configured one: this is the same
    // computation the system prompt and the tool registry read.
    let standings = core.capability_standings();
    if standings.is_empty() {
        println!("\ncapabilities     = (none configured)");
        return 0;
    }
    println!("\ncapabilities");
    for standing in &standings {
        let mark = match standing.reach {
            vak_core::reach::Reach::Open => "open",
            vak_core::reach::Reach::Gated => "gated",
            vak_core::reach::Reach::Blocked => "BLOCKED",
        };
        println!("  {mark:<8} {}", standing.label);
        if !standing.reason.is_empty() {
            println!("           {}", standing.reason);
        }
        if !standing.remedy.is_empty() {
            println!("           fix: {}", standing.remedy);
        }
    }
    0
}

/// Where a `vak config set-*` write lands. Mirrors the two scopes every
/// other layered setting already uses.
fn scope_config_path(scope: cli::PromptScope, cwd: &std::path::Path) -> Option<PathBuf> {
    match scope {
        cli::PromptScope::User => vak_config::global_path(),
        cli::PromptScope::Project => Some(vak_config::project_path(cwd)),
    }
}

fn run_config_set_mode(cwd: PathBuf, mode: &str, scope: cli::PromptScope) -> i32 {
    let Some(parsed) = vak_config::PermissionMode::deserialize_str(mode) else {
        eprintln!("error: unknown mode '{mode}' (read-only | workspace-write | full-access)");
        return 2;
    };
    let Some(path) = scope_config_path(scope, &cwd) else {
        eprintln!("error: user home unavailable");
        return 2;
    };
    if let Err(e) =
        vak_config::persist_preferences_to(path, None, None, None, Some(parsed), None, None)
    {
        eprintln!("error: {e}");
        return 2;
    }
    println!("permission_mode = {mode} ({} layer)", scope_label(scope));
    // A running server keeps its own copy; say so rather than implying the
    // change reached every surface already.
    println!("A running `vak serve` picks this up on its next config refresh.");
    0
}

fn run_config_set_approval(cwd: PathBuf, mode: &str, scope: cli::PromptScope) -> i32 {
    let Some(parsed) = vak_config::ApprovalMode::parse(mode) else {
        eprintln!("error: unknown mode '{mode}' (ask | approve-safe | auto-approve)");
        return 2;
    };
    let Some(path) = scope_config_path(scope, &cwd) else {
        eprintln!("error: user home unavailable");
        return 2;
    };
    if let Err(e) =
        vak_config::persist_preferences_to(path, None, None, None, None, Some(parsed), None)
    {
        eprintln!("error: {e}");
        return 2;
    }
    println!("approval_mode = {mode} ({} layer)", scope_label(scope));
    println!("A running `vak serve` picks this up on its next config refresh.");
    0
}

fn scope_label(scope: cli::PromptScope) -> &'static str {
    match scope {
        cli::PromptScope::User => "Shared",
        cli::PromptScope::Project => "project",
    }
}

fn run_config_dump(cwd: PathBuf) {
    match Core::new(cwd.clone()) {
        Ok(core) => {
            println!("# Vakyartha effective config");
            println!("version          = {}", vak_core::APP_VERSION);
            println!("cwd              = {}", core.cwd().display());
            println!("provider         = {}", core.effective_provider());
            println!("model            = {}", core.effective_model());
            println!(
                "max_tokens       = {}",
                core.config()
                    .max_tokens
                    .map_or_else(|| "the model's own".to_string(), |cap| cap.to_string())
            );
            println!("max_turns        = {}", core.effective_max_turns());
            println!("permission_mode  = {:?}", core.effective_permission_mode());
            println!(
                "approval_mode    = {}",
                core.effective_approval_mode().as_str()
            );
            println!("sandbox          = {}", core.effective_sandbox_name());
            println!("sessions_home    = {}", core.scope().into_root().display());
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
            let r = core.effective_route_settings();
            let same_model: Vec<String> = r
                .same_model
                .iter()
                .map(|group| {
                    group
                        .iter()
                        .map(vak_llm::ModelRef::spelling)
                        .collect::<Vec<_>>()
                        .join(" = ")
                })
                .collect();
            println!(
                "route            = objective {} · fallback_models [{}] · same_model [{}] · max_fallbacks {} · quality_hints [{}]",
                r.objective,
                r.fallback_models.join(", "),
                same_model.join("; "),
                r.max_fallbacks,
                r.quality_hints.join(", "),
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
    let mut session_dirs = Vec::new();
    let direct = core.scope().sessions_dir(core.cwd());
    if direct.exists() {
        session_dirs.push(direct.clone());
    }
    let shared = core.shared_scope().into_root();
    if let Ok(agents) = std::fs::read_dir(shared.join("agents")) {
        for agent in agents.flatten() {
            let s = vak_session::SessionPath::sessions_dir(&agent.path(), core.cwd());
            if s.exists() && !session_dirs.contains(&s) {
                session_dirs.push(s);
            }
        }
    }
    let trashed = vak_core::trash::trashed(&vak_config::scope::SharedScope::new(&shared));
    // A session is a ledger directory of record segments, named by its id.
    let mut rows: Vec<(std::time::SystemTime, String)> = Vec::new();
    for dir in &session_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(header) = vak_session::SessionLog::read_header(&path) else {
                continue;
            };
            let id = header.session_id;
            if trashed.contains(&id) || rows.iter().any(|(_, known)| known == &id) {
                continue;
            }
            let modified = std::fs::read_dir(&path)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|file| file.metadata().ok()?.modified().ok())
                .max()
                .unwrap_or(std::time::UNIX_EPOCH);
            rows.push((modified, id));
        }
    }
    rows.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    if rows.is_empty() {
        println!("no sessions yet ({})", direct.display());
        return;
    }
    for (modified, id) in rows {
        let at: chrono::DateTime<chrono::Local> = modified.into();
        println!("{id}  {}", at.format("%Y-%m-%d %H:%M"));
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_plan(
    cwd: PathBuf,
    task: String,
    yes: bool,
    permission_mode: Option<String>,
    write_paths: Vec<PathBuf>,
    worktree: bool,
    trusted: bool,
) -> i32 {
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
    let core = match Core::new_with_trust(effective_cwd.clone(), trusted)
        .map(|c| with_cli_surface(c, yes))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    // Applied before the session is started, so the frozen prompt describes
    // the boundary this run will actually have.
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
    let engine = match core.build_permission_engine(&core.extra_allow_snapshot()) {
        // Same execution contract `exec --write-path` carries: direct file
        // mutations outside the declared paths are refused before rules or
        // permission mode are consulted. A planner generates its own write
        // targets, which is precisely why it needs this.
        Ok(e) if !write_paths.is_empty() => e.restrict_write_paths(core.cwd(), &write_paths),
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let prepared = core.prepare_turn().await;
    let mut outcome =
        vak_intent::OutcomeSpec::from_reading(task.clone(), &vak_intent::Reading::default(), 0);
    outcome.max_turns = Some(core.effective_max_turns());
    let objects = match core.objects() {
        Ok(objects) => objects,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    // The plan is a run; each attempt it makes is a run of its own,
    // caused by this one (vak_flow::plan_and_run).
    let plan_trace = core.mint_trace(None);
    if let Err(e) = core.runs().open_in(
        &plan_trace,
        None,
        None,
        1,
        None,
        Some(vak_session::runs::RunWork::Plan),
    ) {
        eprintln!("error: the plan's run could not be recorded: {e}");
        return 2;
    }
    let deps = vak_flow::ExecutorDeps {
        objects,
        prompt_layers: Vec::new(),
        provider_route: core.effective_provider().to_string(),
        provider,
        system_prompt: prepared.system_prompt,
        node_prompt: Some(core.flow_node_prompt(core.capability_descriptors())),
        model: core.effective_model(),
        tools: prepared.tools,
        read_only_tools: prepared.read_only_tools,
        max_turns: core.effective_max_turns(),
        outcome: Some(outcome),
        max_retries: 0,
        retry_base_backoff_ms: 100,
        request_timeout: Some(std::time::Duration::from_secs(600)),
        circuit_breaker: None,
        run_retry_attempts: 0,
        run_retry_base_backoff_ms: 1000,
        dispatch_ceiling: 1,
        spend_gate: None,
        permission: Some(std::sync::Arc::new(engine)),
        mode: match core.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        },
        approval_mode: match core.effective_approval_mode() {
            vak_config::ApprovalMode::Ask => vak_agent::ApprovalMode::Ask,
            vak_config::ApprovalMode::ApproveSafe => vak_agent::ApprovalMode::ApproveSafe,
            vak_config::ApprovalMode::AutoApprove => vak_agent::ApprovalMode::AutoApprove,
        },
        approver,
        sandbox: core.agent_sandbox(),
        cwd: core.cwd().clone(),
        sessions_home: core.scope().into_root().clone(),
        parent_session_id,
        state_path: core
            .scope()
            .flow_runs()
            .join(format!("{}.json", plan_trace.run)),
        agent_identity: core.agent_identity().cloned(),
        conversation_context: core.conversation_context().cloned(),
        run: Some(vak_flow::FlowRun {
            runs: core.runs(),
            trace: plan_trace.clone(),
            attempt: 1,
        }),
        work: None,
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
    let plan_span = vak_session::runs::span(&plan_trace);
    let runner = tokio::spawn(tracing::Instrument::instrument(
        async move { vak_flow::plan_and_run(std::sync::Arc::new(deps), &task, cancel, tx).await },
        plan_span,
    ));

    while let Some(line) = rx.recv().await {
        eprintln!("{line}");
    }

    let finished = runner.await;
    let settled = match &finished {
        Ok(vak_flow::PlanOutcome::Completed { .. }) => vak_session::runs::RunOutcome::Completed,
        Ok(vak_flow::PlanOutcome::PlanningFailed { reason }) => {
            vak_session::runs::RunOutcome::Failed {
                reason: format!("planning failed: {reason}"),
            }
        }
        Ok(vak_flow::PlanOutcome::Failed { node, reason }) => {
            vak_session::runs::RunOutcome::Failed {
                reason: format!("{node}: {reason}"),
            }
        }
        Ok(vak_flow::PlanOutcome::Aborted) => vak_session::runs::RunOutcome::Cancelled,
        Err(e) => vak_session::runs::RunOutcome::Failed {
            reason: format!("the planner crashed: {e}"),
        },
    };
    if let Err(e) = core.runs().settle(plan_trace.run, settled, None) {
        eprintln!("warning: the plan's run did not settle: {e}");
    }
    match finished {
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
    generated: usize,
    scenario_seed: u64,
    scenario_offset: u64,
    live: bool,
    provider_flag: Option<String>,
    model_flag: Option<String>,
) -> i32 {
    if live && generated > 0 {
        eprintln!("error: --generated is deterministic and cannot be combined with --live");
        return 2;
    }
    if scenario_offset.checked_add(generated as u64).is_none() {
        eprintln!("error: --offset plus --generated exceeds the scenario index range");
        return 2;
    }
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
        for index in scenario_offset..(scenario_offset + generated as u64) {
            let case = vak_eval::generated_scenario(scenario_seed, index);
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
            if reports.len() % 100 == 0 {
                eprintln!("eval progress: {} cases", reports.len());
            }
        }
        // Deterministic context-engine gate (docs/design/68-context-engine.md
        // "Verification") — no model calls; planner, projection and
        // two-model replay properties over a fixture ledger.
        let card = match vak_eval::run_context_engine_scorecard() {
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
        if provider_flag.is_some() || model_flag.is_some() {
            let route = core.effective_route();
            core.set_route(
                provider_flag.unwrap_or(route.provider),
                model_flag.unwrap_or(route.model),
            );
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
            "scenario_batch": {
                "kind": if live { "live-model" } else { "scripted-deterministic" },
                "seed": scenario_seed,
                "offset": scenario_offset,
                "generated": generated,
            },
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

/// Open a running server's web surface, already signed in.
///
/// The point is that nobody should have to go and find this machine's
/// access token to reach a server on this machine. The token is read from
/// the pinned Shared secret scope and handed over as a one-shot `?token=`,
/// which the client immediately exchanges for a session cookie and erases
/// from the address bar.
///
/// LOOPBACK ONLY, and not by convention: the server refuses `?token=` from
/// any non-loopback host (invariant 34), so this URL authenticates nothing
/// if it leaves the machine. That is why the convenience is safe to offer
/// at all.
fn run_open(surface: cli::OpenSurface, port: Option<u16>, print_only: bool) -> i32 {
    let path = match surface {
        cli::OpenSurface::App => "/app",
        cli::OpenSurface::Admin => "/admin",
    };
    let port = port.unwrap_or_else(|| vak_ops::OpsConfig::detect().port);
    let base = format!("http://127.0.0.1:{port}{path}");

    // No token pinned is not an error: the server then mints one per boot,
    // and the sign-in screen is the honest answer.
    let url = match vak_config::get_var("VAK_GATEWAY_TOKEN").filter(|t| !t.trim().is_empty()) {
        Some(token) => format!(
            "{base}?token={}",
            percent_encoding::utf8_percent_encode(token.trim(), percent_encoding::NON_ALPHANUMERIC)
        ),
        None => {
            eprintln!(
                "note: no VAK_GATEWAY_TOKEN pinned, so this link cannot sign you in.\n\
                 Run `vak self services-sync` to pin one."
            );
            base.clone()
        }
    };

    if print_only {
        println!("{url}");
        return 0;
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(opener).arg(&url).spawn() {
        Ok(_) => {
            // The token is deliberately NOT echoed here; the browser has it.
            println!("opening {base}");
            0
        }
        Err(e) => {
            eprintln!("could not launch a browser ({e}); open this yourself:\n{url}");
            1
        }
    }
}

/// Whether this process is running inside a container.
///
/// `/.dockerenv` is Docker's own marker; `VAK_CONTAINER` is set by our
/// image so the check also holds under runtimes that do not create it
/// (Podman, containerd). This is a USABILITY guard, not a privilege
/// boundary — anyone who can set the variable can already pass any flag
/// they like — so detecting it loosely is fine.
fn in_container() -> bool {
    std::path::Path::new("/.dockerenv").exists()
        || std::env::var("VAK_CONTAINER").is_ok_and(|v| v == "1")
}

async fn run_serve(
    cwd: PathBuf,
    port: u16,
    host: Option<String>,
    gateway: bool,
    trusted: bool,
) -> i32 {
    // Not `Cli`: this process serves API clients and, with `--gateway`, chat
    // channels. The gateway re-stamps each inbound message with its own
    // channel (`vak-server/src/gateway.rs`); this is the fallback for a
    // plain HTTP caller.
    let core = match Core::new_with_trust(cwd, trusted)
        .map(|c| c.with_surface(vak_core::Surface::Server))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    print_config_warnings(&core);

    // Where to listen. `--host` wins over `[server] bind`, and both default
    // to loopback — so an existing install keeps behaving exactly as it did.
    let server = core.config().server.clone();
    let bind = host.unwrap_or_else(|| server.bind.clone());
    let publicly = !matches!(bind.as_str(), "127.0.0.1" | "::1" | "localhost");

    // Refuse, rather than warn. Binding a shell-capable agent to a
    // reachable interface with no `trusted_hosts` means every request from
    // the network is rejected 421 anyway — so the server would appear to
    // start and then answer nothing, which is the worst of both outcomes.
    // Failing here says exactly which setting is missing while the operator
    // is still looking at the terminal.
    //
    // EXCEPT in a container, where 0.0.0.0 is the only address that can be
    // reached at all and the access control is `docker run -p` — an
    // explicit, deliberate act by the operator, which is exactly what this
    // refusal exists to require. Refusing here would mean every container
    // needs `trusted_hosts` before it can serve its own published port, so
    // the guard would be worked around rather than obeyed.
    //
    // What does NOT relax is the `Host` check itself (invariant 34): DNS
    // rebinding is defended identically inside a container, because that
    // attack does not care where the process runs.
    if publicly && server.trusted_hosts.is_empty() && in_container() {
        eprintln!(
            "note: binding {bind} inside a container; reachability is governed by\n\
             the published port. Add [server] trusted_hosts to serve a real hostname."
        );
    } else if publicly && server.trusted_hosts.is_empty() {
        eprintln!(
            "error: refusing to bind {bind} with no [server] trusted_hosts.\n\
             \n\
             A non-loopback bind exposes this agent — including its tools — to\n\
             whoever can reach the port. Name the hostnames you will actually\n\
             use in .vak/config.toml (or the user config):\n\
             \n\
             [server]\n\
             bind = \"{bind}\"\n\
             trusted_hosts = [\"vak.example.com\"]\n\
             public_url = \"https://vak.example.com\"   # enables Secure cookies\n\
             \n\
             If you only need remote access for yourself, an SSH tunnel needs\n\
             none of this: ssh -N -L {port}:127.0.0.1:{port} <host>"
        );
        return 2;
    }

    let ip: std::net::IpAddr = match bind.as_str() {
        "localhost" => std::net::IpAddr::from([127, 0, 0, 1]),
        other => match other.parse() {
            Ok(ip) => ip,
            Err(_) => {
                eprintln!("error: [server] bind is not an IP address: {other}");
                return 2;
            }
        },
    };
    let addr = std::net::SocketAddr::new(ip, port);
    if publicly {
        eprintln!(
            "listening on {addr} — reachable beyond this machine. Trusted hosts: {}",
            server.trusted_hosts.join(", ")
        );
    }
    match vak_server::serve_with(core, addr, gateway).await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// How to tell someone to set a secret, now that there is no file to name
/// — every bridge's "where do I put this token" hint reads the same.
/// Resolved through `vak_config::credentials` (OS keychain or the
/// encrypted-file fallback), so the actionable advice is the Settings UI,
/// `PUT /config/key`, or an environment variable — never a path to edit
/// by hand.
fn env_hint() -> &'static str {
    "the Settings UI, PUT /config/key, or an environment variable"
}

/// Env-first gateway token so it stays out of `ps`/plist arguments,
/// shared by every bridge subcommand.
fn bridge_gateway_token(token_flag: Option<String>) -> Option<String> {
    match token_flag {
        Some(t) if !t.trim().is_empty() => Some(t),
        _ => match vak_config::get_var("VAK_GATEWAY_TOKEN") {
            Some(t) if !t.trim().is_empty() => Some(t),
            _ => {
                eprintln!(
                    "error: gateway token missing — set VAK_GATEWAY_TOKEN via {}, or pass --token",
                    env_hint()
                );
                None
            }
        },
    }
}

/// Credential-store-aware bot-token lookup so a bridge credential never
/// has to be exported by hand.
fn bridge_bot_token(env_var: &str) -> Option<String> {
    match vak_config::get_var(env_var) {
        Some(t) if !t.trim().is_empty() => Some(t),
        _ => {
            eprintln!("error: {env_var} is not set — set it via {}", env_hint());
            None
        }
    }
}

/// Resolve which env var a bridge should read its token from: the legacy
/// single per-surface slot by default, or — when `--bot-id` names a bot
/// created in the admin console's Bots list (multi-bot-per-channel,
/// docs/design/34) — that bot's own env var, so a second `vak telegram
/// --bot-id ...` process can run alongside the first with a different
/// token. `None` only when `--bot-id` was given but no such bot exists.
fn bridge_token_env_var(legacy_env_var: &str, bot_id: Option<&str>) -> Option<String> {
    match bot_id {
        None => Some(legacy_env_var.to_string()),
        Some(id) => {
            let sessions_home = vak_config::paths::data_home();
            match vak_server::gateway::bot_token_env_for_id(&sessions_home, id) {
                Some(env_var) => Some(env_var),
                None => {
                    eprintln!(
                        "error: no bot '{id}' — create it first in the admin console's Bots list"
                    );
                    None
                }
            }
        }
    }
}

/// `vak discord` (docs/design/34 Phase 3) — same flag shape as
/// `vak telegram`, same env-first credential handling.
async fn run_discord(server: String, token_flag: Option<String>, bot_id: Option<String>) -> i32 {
    let Some(env_var) = bridge_token_env_var("DISCORD_BOT_TOKEN", bot_id.as_deref()) else {
        return 2;
    };
    let (Some(token), Some(bot_token)) =
        (bridge_gateway_token(token_flag), bridge_bot_token(&env_var))
    else {
        return 2;
    };
    let bridge =
        vak_server::surfaces::discord::DiscordBridge::from_env(server, token, bot_token, bot_id);
    println!(
        "discord bridge: {} -> {} ({} channel(s))",
        bridge.api_base,
        bridge.gateway_url,
        bridge.channel_ids.len()
    );
    match bridge.run().await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `vak slack` (docs/design/34 Phase 3).
async fn run_slack(server: String, token_flag: Option<String>, bot_id: Option<String>) -> i32 {
    let Some(env_var) = bridge_token_env_var("SLACK_BOT_TOKEN", bot_id.as_deref()) else {
        return 2;
    };
    let (Some(token), Some(bot_token)) =
        (bridge_gateway_token(token_flag), bridge_bot_token(&env_var))
    else {
        return 2;
    };
    let bridge =
        vak_server::surfaces::slack::SlackBridge::from_env(server, token, bot_token, bot_id);
    println!(
        "slack bridge: {} -> {} ({} channel(s))",
        bridge.api_base,
        bridge.gateway_url,
        bridge.channel_ids.len()
    );
    match bridge.run().await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

async fn run_telegram(server: String, token_flag: Option<String>, bot_id: Option<String>) -> i32 {
    let Some(env_var) = bridge_token_env_var("TELEGRAM_BOT_TOKEN", bot_id.as_deref()) else {
        return 2;
    };
    let Some(token) = bridge_gateway_token(token_flag) else {
        return 2;
    };
    let Some(bot_token) = bridge_bot_token(&env_var) else {
        return 2;
    };
    let api_base = vak_config::get_var("TELEGRAM_API_BASE")
        .unwrap_or_else(|| "https://api.telegram.org".to_string());
    let bridge = vak_server::surfaces::telegram::TelegramBridge {
        token_env: env_var.clone(),
        api_base,
        bot_token: bot_token.clone(),
        gateway_url: server.trim_end_matches('/').to_string(),
        gateway_token: token,
        cursor: vak_server::surfaces::PollCursor::for_bot("telegram", bot_id.as_deref()),
        bot_id,
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

#[cfg(test)]
mod reflection_line_tests {
    use super::exec_reflection_line;

    #[test]
    fn reflected_outcomes_get_a_footnote_even_at_zero_notes() {
        let line = exec_reflection_line(&vak_core::reflection::ReflectionOutcome::Reflected {
            notes_added: 2,
            skills_proposed: false,
        });
        assert_eq!(line.as_deref(), Some("· reflected: 2 note(s)"));
        assert_eq!(
            exec_reflection_line(&vak_core::reflection::ReflectionOutcome::Reflected {
                notes_added: 0,
                skills_proposed: true,
            })
            .as_deref(),
            Some("· reflected: 0 note(s)")
        );
    }

    #[test]
    fn budget_skips_are_actionable_and_other_skips_silent() {
        assert_eq!(
            exec_reflection_line(&vak_core::reflection::ReflectionOutcome::Skipped {
                reason: "budget"
            })
            .as_deref(),
            Some("· reflection skipped: budget cap reached")
        );
        for quiet in [
            "reflection-disabled",
            "already-in-flight",
            "reflect-call-failed",
        ] {
            assert_eq!(
                exec_reflection_line(&vak_core::reflection::ReflectionOutcome::Skipped {
                    reason: quiet
                }),
                None,
                "{quiet} must stay silent"
            );
        }
    }
}
