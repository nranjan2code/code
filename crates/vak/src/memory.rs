//! `vak memory` (docs/design/29-personal-os.md P1): list/add/forget/
/// amend over the workspace MEMORY.md tier and the global USER.md profile
/// tier, wired straight onto `vak_core::memory`. Ids are printed so they
/// can be copied into forget/amend.
use std::path::{Path, PathBuf};

use vak_core::Core;
use vak_core::memory::{self, NoteBlock};

pub fn run_memory(cwd: PathBuf, action: Option<crate::cli::MemoryAction>) -> i32 {
    let core = match Core::new(cwd.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let home = core.sessions_home();
    let workspace = core.cwd().clone();
    match action.unwrap_or(crate::cli::MemoryAction::List { profile: false }) {
        crate::cli::MemoryAction::Clean { older_than_secs } => {
            let report =
                memory::cleanup_artifacts(&home, std::time::Duration::from_secs(older_than_secs));
            println!(
                "cleaned {} locks, {} temp files, {} empty workspace directories",
                report.removed_locks, report.removed_temps, report.removed_empty_dirs
            );
            0
        }
        crate::cli::MemoryAction::List { profile } => {
            list(&home, &workspace, profile);
            0
        }
        crate::cli::MemoryAction::Consolidate => {
            match core.consolidate_memory() {
                Ok(report) => {
                    println!("Memory consolidation complete:");
                    println!("  Examined notes:        {}", report.total_notes_examined);
                    println!("  Promoted invariants:   {}", report.promoted_invariants.len());
                    for inv in &report.promoted_invariants {
                        println!("    + Invariant: \"{inv}\"");
                    }
                    println!("  Detected conflicts:    {}", report.detected_conflicts.len());
                    for conf in &report.detected_conflicts {
                        println!("    ! Conflict: {conf}");
                    }
                    println!("  Distilled entities:    {}", report.distilled_entities.len());
                    for ent in &report.distilled_entities {
                        println!("    * Entity: {ent}");
                    }
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        crate::cli::MemoryAction::Add {
            text,
            kind,
            tag,
            profile,
        } => match add(&home, &workspace, profile, &kind, &tag, &text) {
            Ok(note) => {
                println!(
                    "added note {} [{}] to {}",
                    note.id,
                    note.kind,
                    tier(profile)
                );
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        crate::cli::MemoryAction::Forget { id, profile } => {
            match memory::forget_note(&store_path(&home, &workspace, profile), &id) {
                Ok(bytes) => {
                    println!("forgot {id} ({bytes} bytes removed from {})", tier(profile));
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        crate::cli::MemoryAction::Amend { id, text, profile } => {
            match memory::amend_note(&store_path(&home, &workspace, profile), &id, &text) {
                Ok(()) => {
                    println!("amended {id} in {}", tier(profile));
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

fn tier(profile: bool) -> &'static str {
    if profile {
        "profile (USER.md)"
    } else {
        "workspace (MEMORY.md)"
    }
}

fn store_path(home: &Path, workspace: &Path, profile: bool) -> PathBuf {
    if profile {
        memory::profile_path(home)
    } else {
        home.join("memory")
            .join(memory::hash_cwd(workspace))
            .join("MEMORY.md")
    }
}

fn notes_for(home: &Path, workspace: &Path, profile: bool) -> Vec<NoteBlock> {
    if profile {
        memory::list_profile_notes(home)
    } else {
        memory::list_notes(home, workspace)
    }
}

fn list(home: &Path, workspace: &Path, profile: bool) {
    let notes = notes_for(home, workspace, profile);
    if notes.is_empty() {
        println!("no memory notes in {}", tier(profile));
        return;
    }
    for n in &notes {
        println!(
            "{} {} [{}]{} session={}",
            n.id,
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
}

fn add(
    home: &Path,
    workspace: &Path,
    profile: bool,
    kind: &str,
    tag: &str,
    text: &str,
) -> Result<NoteBlock, String> {
    // CLI-written notes carry a synthetic session id; provenance grammar
    // stays identical to tool-written notes.
    if profile {
        memory::append_profile_note(home, kind, tag, text, "cli")
    } else {
        memory::append_note(home, workspace, kind, tag, "cli", text)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn store_paths_split_tiers() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let ws = dir.path().join("ws");
        assert_eq!(
            store_path(home, &ws, true),
            home.join("memory/user/USER.md")
        );
        assert_eq!(
            store_path(home, &ws, false),
            home.join("memory")
                .join(memory::hash_cwd(&ws))
                .join("MEMORY.md")
        );
    }

    #[test]
    fn add_list_roundtrip_both_tiers() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let ws = dir.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();

        let note = add(
            home,
            &ws,
            false,
            "decision",
            "deploy",
            "pause before rollbacks",
        )
        .unwrap();
        let profile_note = add(home, &ws, true, "preference", "", "vim bindings").unwrap();

        assert!(note.id.len() == 16);
        assert_eq!(note.session_id, "cli");
        let ws_notes = notes_for(home, &ws, false);
        assert_eq!(ws_notes.len(), 1);
        assert_eq!(ws_notes[0].text, "pause before rollbacks");
        let profile_notes = notes_for(home, &ws, true);
        assert_eq!(profile_notes.len(), 1);
        assert_eq!(profile_notes[0].id, profile_note.id);

        // forget by printed id removes exactly that block.
        let path = store_path(home, &ws, false);
        memory::forget_note(&path, &note.id).unwrap();
        assert!(notes_for(home, &ws, false).is_empty());
    }

    #[test]
    fn empty_add_is_a_value_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = add(dir.path(), dir.path(), false, "note", "", "   ").unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn empty_listing_renders_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        list(dir.path(), &ws, false);
        list(dir.path(), &ws, true);
    }
}
