//! The trash (docs/plans/data-architecture-plan.md, M0): a session moved to
//! the trash is hidden everywhere, from the session lists, every search a
//! person or the model can run, its transcript and export, and digests,
//! and can be restored. Erasure, which removes it for good, is later work
//! (M7). This module is the one place any of those readers asks.
//!
//! The set is keyed by session id in the shared data home, like the archive,
//! so a single mutation and its bulk form share one boundary check
//! (AGENTS.md invariant 22) in the server.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn path(shared_home: &Path) -> PathBuf {
    shared_home.join("deleted.json")
}

fn read(shared_home: &Path) -> HashMap<String, bool> {
    std::fs::read_to_string(path(shared_home))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Every session id in the trash.
pub fn trashed(shared_home: &Path) -> HashSet<String> {
    read(shared_home)
        .into_iter()
        .filter_map(|(id, trashed)| trashed.then_some(id))
        .collect()
}

pub fn is_trashed(shared_home: &Path, session_id: &str) -> bool {
    read(shared_home).get(session_id).copied().unwrap_or(false)
}

/// What a cross-session search must skip: the trash and, when there is one,
/// the session doing the searching.
pub fn search_exclusions(shared_home: &Path, current: Option<&str>) -> HashSet<String> {
    let mut excluded = trashed(shared_home);
    excluded.extend(current.map(str::to_string));
    excluded
}

/// Moves sessions into or out of the trash, atomically and one writer at a
/// time within the process.
pub fn set(shared_home: &Path, session_ids: &[String], trashed: bool) -> std::io::Result<()> {
    static WRITE: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = WRITE
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut map = read(shared_home);
    for id in session_ids {
        if trashed {
            map.insert(id.clone(), true);
        } else {
            map.remove(id);
        }
    }
    std::fs::create_dir_all(shared_home)?;
    let temp = path(shared_home).with_extension("json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_string(&map).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(temp, path(shared_home))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_session_moves_into_and_out_of_the_trash() {
        let home = tempfile::tempdir().unwrap();
        assert!(!is_trashed(home.path(), "s1"));
        set(home.path(), &["s1".into(), "s2".into()], true).unwrap();
        assert!(is_trashed(home.path(), "s1"));
        assert_eq!(trashed(home.path()).len(), 2);
        set(home.path(), &["s1".into()], false).unwrap();
        assert!(!is_trashed(home.path(), "s1"));
        let excluded = search_exclusions(home.path(), Some("current"));
        assert!(excluded.contains("s2") && excluded.contains("current"));
        assert!(!excluded.contains("s1"));
    }
}
