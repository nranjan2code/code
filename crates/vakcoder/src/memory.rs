//! Runtime-owned memory commands.

use std::path::PathBuf;
use vak_client::{Client, MemoryCreate};

pub async fn run_memory(cwd: PathBuf, action: Option<crate::cli::MemoryAction>) -> i32 {
    let client = match crate::connect::discover(None, None, None)
        .map_err(|error| error.to_string())
        .and_then(|resolved| {
            Client::new(resolved.url, resolved.token).map_err(|error| error.to_string())
        }) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("error connecting to Runtime: {error}");
            return 2;
        }
    };
    let project_id = match client.projects().await {
        Ok(projects) => projects
            .into_iter()
            .find(|project| {
                std::fs::canonicalize(&project.root).ok() == std::fs::canonicalize(&cwd).ok()
            })
            .map(|project| project.id),
        Err(error) => {
            eprintln!("error listing Runtime projects: {error}");
            return 2;
        }
    };
    match action.unwrap_or(crate::cli::MemoryAction::List { profile: false }) {
        crate::cli::MemoryAction::List { profile } => match client
            .memory(
                project_id.as_ref(),
                if profile { "profile" } else { "workspace" },
            )
            .await
        {
            Ok(notes) => {
                if notes.is_empty() {
                    println!("no memory notes");
                }
                for note in notes {
                    println!("{} {} [{}]", note.id, note.created_at, note.kind);
                    println!("  {}", note.text.replace('\n', "\n  "));
                }
                0
            }
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        crate::cli::MemoryAction::Add {
            text,
            kind,
            tag,
            profile,
        } => {
            let request = MemoryCreate {
                project_id: if profile { None } else { project_id },
                scope: if profile {
                    "profile".into()
                } else {
                    "workspace".into()
                },
                kind,
                tag,
                text,
            };
            match client.create_memory(&request).await {
                Ok(note) => {
                    println!("added note {} [{}]", note.id, note.kind);
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        crate::cli::MemoryAction::Forget { id, .. } => match client.forget_memory(&id).await {
            Ok(true) => {
                println!("forgot {id}");
                0
            }
            Ok(false) => {
                eprintln!("error: note not found: {id}");
                2
            }
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        crate::cli::MemoryAction::Amend { id, text, .. } => {
            match client.amend_memory(&id, &text).await {
                Ok(_) => {
                    println!("amended {id}");
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
    }
}
