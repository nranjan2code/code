//! CLI handlers for `vak agents` (managing specialist agents & domain templates).

use std::path::PathBuf;

use vak_server::agents;

use crate::cli::AgentsAction;

pub fn run_agents(cwd: PathBuf, action: Option<AgentsAction>) -> i32 {
    let shared = vak_config::paths::default_workspace();

    match action.unwrap_or(AgentsAction::List { global: false }) {
        AgentsAction::List { global } => {
            let root = if global {
                shared.as_path()
            } else {
                cwd.as_path()
            };
            let list = match agents::load(root) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("error reading agents: {e}");
                    return 1;
                }
            };
            if list.is_empty() {
                println!(
                    "No agents configured in {} scope.",
                    if global { "global" } else { "workspace" }
                );
                println!(
                    "Hint: run 'vak agents templates' or 'vak agents init --template <name> --id <id>' to create one."
                );
                return 0;
            }
            println!(
                "Configured Agents in {} scope ({} total):\n",
                if global { "global" } else { "workspace" },
                list.len()
            );
            for a in list {
                println!("- `{}` ({}): {}", a.id, a.name, a.behaviour);
                if !a.personality.is_empty() {
                    println!("    personality: {}", a.personality);
                }
                if !a.responsibilities.is_empty() {
                    println!("    focus: {}", a.responsibilities);
                }
            }
            0
        }
        AgentsAction::Templates => {
            let templates = agents::builtin_templates();
            println!(
                "Available Universal Domain Specialist Templates ({} total):\n",
                templates.len()
            );
            for t in templates {
                println!("- `{}` — {} [Domain: {}]", t.template_id, t.name, t.domain);
                println!("    {}", t.description);
                println!(
                    "    Character: {} | Voice: {} | Animation: {}",
                    t.character, t.voice, t.animation
                );
                println!("    Focus: {}", t.responsibilities);
                println!();
            }
            println!("To instantiate an agent from a template:");
            println!(
                "  vak agents init --template <template_id> --id <unique_agent_id> [--name <display_name>] [--global]"
            );
            0
        }
        AgentsAction::Init {
            template,
            id,
            name,
            global,
        } => {
            let Some(tmpl) = agents::find_template(&template) else {
                eprintln!("error: unknown template '{template}'");
                eprintln!("Run 'vak agents templates' to view available archetypes.");
                return 1;
            };

            let root = if global {
                shared.as_path()
            } else {
                cwd.as_path()
            };
            let mut existing = match agents::load(root) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("error reading agents: {e}");
                    return 1;
                }
            };

            if existing.iter().any(|a| a.id == id) {
                eprintln!(
                    "error: agent with id '{id}' already exists in {} scope",
                    if global { "global" } else { "workspace" }
                );
                return 1;
            }

            let new_agent = tmpl.to_agent_definition(&id, name.as_deref());
            existing.push(new_agent.clone());

            match agents::save(root, &existing) {
                Ok(_) => {
                    println!(
                        "Created agent '{}' ({}) in {} scope.",
                        new_agent.id,
                        new_agent.name,
                        if global { "global" } else { "workspace" }
                    );
                    0
                }
                Err(e) => {
                    eprintln!("error saving agent: {e}");
                    1
                }
            }
        }
        AgentsAction::Runs { id, limit, global } => {
            let root = if global {
                shared.as_path()
            } else {
                cwd.as_path()
            };
            let runs = match agents::list_runs(root, Some(&id), limit) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error reading agent runs: {e}");
                    return 1;
                }
            };
            if runs.is_empty() {
                println!(
                    "No past runs recorded for agent '{id}' in {} scope.",
                    if global { "global" } else { "workspace" }
                );
                return 0;
            }
            println!("Recent runs for agent '{}' ({} total):\n", id, runs.len());
            for r in runs {
                println!("- Run [{}] — Status: {}", r.run_id, r.status);
                println!("    Started: {}", r.started_at);
                if let Some(comp) = &r.completed_at {
                    println!("    Completed: {comp}");
                }
                println!("    Prompt: {}", r.prompt);
                if let Some(sum) = &r.summary {
                    println!("    Summary: {sum}");
                }
                if let Some(err) = &r.error {
                    println!("    Error: {err}");
                }
                println!();
            }
            0
        }
        AgentsAction::Schedule {
            id,
            cron,
            prompt,
            global,
        } => {
            let root = if global {
                shared.as_path()
            } else {
                cwd.as_path()
            };
            let schedule = agents::AgentSchedule {
                cron_or_interval: cron.clone(),
                prompt: prompt.clone(),
                enabled: true,
                last_run_at: None,
                last_status: None,
            };
            match agents::update_schedule(root, &id, Some(schedule)) {
                Ok(agent) => {
                    println!(
                        "Scheduled agent '{}' ({}) with frequency: '{}'",
                        agent.id, agent.name, cron
                    );
                    println!("Prompt: '{}'", prompt);
                    0
                }
                Err(e) => {
                    eprintln!("error updating agent schedule: {e}");
                    1
                }
            }
        }
    }
}
