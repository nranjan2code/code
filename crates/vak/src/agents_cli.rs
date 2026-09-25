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

            match agents::save(root, &existing, vak_core::trust::is_trusted(root)) {
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
    }
}
