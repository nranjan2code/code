//! `vak sync` (data-architecture plan M9): the remote folder that holds a
//! second copy of what this machine stores, and the key file another
//! machine needs to read it.

use std::path::PathBuf;

use crate::cli::{KeyFileAction, SyncAction};
use vak_core::Core;

/// The passphrase for a key file: `VAK_KEY_PASSPHRASE` when it is set (for
/// a script), else a line typed here.
fn passphrase(again: bool) -> Option<String> {
    if let Some(set) = vak_config::get_var("VAK_KEY_PASSPHRASE") {
        return Some(set);
    }
    let ask = |prompt: &str| {
        eprintln!("{prompt}");
        let mut typed = String::new();
        std::io::stdin().read_line(&mut typed).ok()?;
        Some(typed.trim_end_matches(['\r', '\n']).to_string())
    };
    let first = ask("Passphrase (it is shown as you type):")?;
    if again && ask("The same passphrase again:")? != first {
        eprintln!("error: the two did not match");
        return None;
    }
    Some(first)
}

fn report(what: &str, done: Result<vak_core::sync::SyncReport, vak_core::sync::SyncError>) -> i32 {
    match done {
        Ok(report) => {
            println!(
                "{what}: {} files copied, {} removed; the copy holds {} files (sync {}).",
                report.copied, report.removed, report.files, report.generation
            );
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

pub(crate) fn run_sync(cwd: PathBuf, action: Option<SyncAction>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(core) => core,
        Err(error) => {
            eprintln!("error: {error}");
            return 2;
        }
    };
    match action.unwrap_or(SyncAction::Status) {
        SyncAction::Status => match core.sync_status() {
            Ok(status) => {
                println!("remote folder: {}", status.remote.display());
                if !status.reachable {
                    println!("it cannot be reached right now");
                }
                match status.synced_at {
                    Some(at) => println!(
                        "last synced: {} (sync {})",
                        at.format("%Y-%m-%d %H:%M"),
                        status.generation
                    ),
                    None => println!("never synced"),
                }
                if let Some(remote) = status.remote_generation
                    && remote > status.generation
                {
                    println!("the remote copy is newer (sync {remote})");
                }
                println!("changed here since then: {} files", status.unpushed);
                println!(
                    "{}",
                    match status.role {
                        vak_core::sync::Role::Holder => "this machine holds the work",
                        vak_core::sync::Role::StandingBy if status.released =>
                            "this machine is standing by; the work was handed over and can be taken over",
                        vak_core::sync::Role::StandingBy =>
                            "this machine is standing by; the other machine holds the work",
                        vak_core::sync::Role::Lost =>
                            "the other machine took the work over; pull to stand by again",
                        vak_core::sync::Role::Unset =>
                            "nothing has been synced from this machine yet",
                    }
                );
                0
            }
            Err(error) => {
                eprintln!("{error}");
                1
            }
        },
        SyncAction::Setup { folder } => match core.sync_setup(&folder) {
            Ok(local) => {
                println!("The copy is kept in {}", local.remote.display());
                println!("Nothing has been copied yet. Run `vak sync now`.");
                0
            }
            Err(vak_core::sync::SyncError::Unreachable) => {
                eprintln!("error: {} is not a folder that exists", folder.display());
                1
            }
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        SyncAction::Now => report("Pushed", core.sync_push()),
        SyncAction::Pull { discard } => {
            let done = report("Pulled", core.sync_pull(discard));
            if done == 0 {
                println!("Start Vakyartha again before using it.");
            }
            done
        }
        SyncAction::Handover => {
            let done = report("Handed over", core.sync_handover());
            if done == 0 {
                println!("This machine is standing by. Take over on the other one.");
            }
            done
        }
        SyncAction::Takeover { force, discard } => match core.sync_takeover(force, discard) {
            Ok(None) => {
                println!("This machine already holds the work.");
                0
            }
            Ok(Some(done)) => {
                println!(
                    "This machine holds the work now ({} files brought here).",
                    done.copied
                );
                if done.copied + done.removed > 0 {
                    println!("Start Vakyartha again before using it.");
                }
                0
            }
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        SyncAction::Place { project, folder } => {
            // By id, by the name it was given, or by its folder's name,
            // which is what a project is called until it is named.
            let found: Vec<_> = vak_config::spaces::all()
                .into_iter()
                .filter(|space| {
                    space.id == project
                        || space.name.as_deref() == Some(&project)
                        || space
                            .folder
                            .as_deref()
                            .and_then(std::path::Path::file_name)
                            .is_some_and(|name| name.to_string_lossy() == project)
                })
                .collect();
            let [space] = found.as_slice() else {
                eprintln!(
                    "error: {} projects match '{project}'; name one by its id",
                    found.len()
                );
                return 1;
            };
            match vak_core::workspaces::place_project(&space.id, &folder) {
                Ok(()) => {
                    println!("That project is {} on this machine.", folder.display());
                    0
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    1
                }
            }
        }
        SyncAction::Forget => match core.sync_forget() {
            Ok(()) => {
                println!(
                    "This machine no longer uses a remote folder. The folder was not touched."
                );
                0
            }
            Err(error) => {
                eprintln!("error: {error}");
                1
            }
        },
        SyncAction::Key { action } => match action {
            KeyFileAction::Export { file } => {
                if file.exists() {
                    eprintln!("error: {} already exists", file.display());
                    return 1;
                }
                let Some(passphrase) = passphrase(true) else {
                    return 1;
                };
                match core
                    .export_key_file(&passphrase)
                    .and_then(|text| std::fs::write(&file, text).map_err(|error| error.to_string()))
                {
                    Ok(()) => {
                        println!("Key file written to {}", file.display());
                        println!(
                            "Anyone with this file and its passphrase can read your data. Carry it to the other machine yourself, and delete it once it is imported."
                        );
                        0
                    }
                    Err(error) => {
                        eprintln!("error: {error}");
                        1
                    }
                }
            }
            KeyFileAction::Import { file } => {
                let text = match std::fs::read_to_string(&file) {
                    Ok(text) => text,
                    Err(_) => {
                        eprintln!("error: {} could not be read", file.display());
                        return 1;
                    }
                };
                let Some(passphrase) = passphrase(false) else {
                    return 1;
                };
                match core.import_key_file(&text, &passphrase) {
                    Ok(()) => {
                        println!("This machine now holds your keys. Delete the key file.");
                        0
                    }
                    Err(error) => {
                        eprintln!("error: {error}");
                        1
                    }
                }
            }
        },
    }
}
