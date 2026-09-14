//! CLI handlers for `vak entities` (knowledge graph inspection & management).

use std::path::PathBuf;

use vak_core::Core;
use vak_core::entities;

pub fn run_entities(cwd: PathBuf, action: Option<crate::cli::EntitiesAction>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let home = core.sessions_home();
    let workspace = core.cwd().clone();

    match action.unwrap_or(crate::cli::EntitiesAction::List { global: false }) {
        crate::cli::EntitiesAction::List { global } => {
            let cwd_opt = if global { None } else { Some(workspace.as_path()) };
            let list = entities::list_entities(&home, cwd_opt);
            if list.is_empty() {
                println!(
                    "No entities recorded in {} knowledge graph.",
                    if global { "global" } else { "workspace" }
                );
                return 0;
            }
            println!(
                "Entities in {} knowledge graph ({} total):\n",
                if global { "global" } else { "workspace" },
                list.len()
            );
            for e in list {
                println!("- [{}] {} (`{}`): {}", e.entity_type, e.name, e.id, e.summary);
                if !e.attributes.is_empty() {
                    let attrs: Vec<_> =
                        e.attributes.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    println!("    attrs: {}", attrs.join(", "));
                }
                if !e.relations.is_empty() {
                    let rels: Vec<_> = e
                        .relations
                        .iter()
                        .map(|r| format!("{} -> {}", r.relation, r.target_entity_id))
                        .collect();
                    println!("    relations: {}", rels.join(", "));
                }
            }
            0
        }
        crate::cli::EntitiesAction::Search {
            query,
            entity_type,
            global,
        } => {
            let cwd_opt = if global { None } else { Some(workspace.as_path()) };
            let mut results = entities::search_entities(&home, cwd_opt, &query);
            if let Some(filter) = entity_type {
                results.retain(|e| e.entity_type.eq_ignore_ascii_case(&filter));
            }
            if results.is_empty() {
                println!("No matching entities found.");
                return 0;
            }
            println!("Found {} matching entities:\n", results.len());
            for e in results {
                println!("- [{}] {} (`{}`): {}", e.entity_type, e.name, e.id, e.summary);
            }
            0
        }
        crate::cli::EntitiesAction::Get { id, global } => {
            let cwd_opt = if global { None } else { Some(workspace.as_path()) };
            match entities::get_entity(&home, cwd_opt, &id) {
                Some(e) => {
                    println!("ID:          {}", e.id);
                    println!("Name:        {}", e.name);
                    println!("Type:        {}", e.entity_type);
                    println!("Summary:     {}", e.summary);
                    println!("Updated:     {}", e.updated_at);
                    if !e.attributes.is_empty() {
                        println!("Attributes:");
                        for (k, v) in e.attributes {
                            println!("  {k}: {v}");
                        }
                    }
                    if !e.relations.is_empty() {
                        println!("Relations:");
                        for r in e.relations {
                            println!("  {} -> {}", r.relation, r.target_entity_id);
                        }
                    }
                    0
                }
                None => {
                    eprintln!("entity '{id}' not found");
                    1
                }
            }
        }
        crate::cli::EntitiesAction::Delete { id, global } => {
            let cwd_opt = if global { None } else { Some(workspace.as_path()) };
            match entities::delete_entity(&home, cwd_opt, &id) {
                Ok(true) => {
                    println!("deleted entity '{id}'");
                    0
                }
                Ok(false) => {
                    eprintln!("entity '{id}' not found");
                    1
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
    }
}
