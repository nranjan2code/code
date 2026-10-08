//! The trash and the archive (docs/design/74 §2.4, plan M7a-e): a
//! conversation moved to the trash is hidden everywhere, from the session
//! lists, every search a person or the model can run, its transcript and
//! export, and digests, and can be restored until its window ends; an
//! erased one is hidden for good. This module is the one place any of
//! those readers asks. The state is one ref per conversation
//! (`vak_session::conversation_state`), keyed by session id for the whole
//! data home, so a single mutation and its bulk form share one boundary
//! check (AGENTS.md invariant 22) in the server.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use vak_session::conversation_state::{self, ConversationState};

fn tenant(shared: &vak_config::scope::SharedScope) -> PathBuf {
    vak_config::paths::tenant_home_at(shared.root(), vak_config::paths::LOCAL_TENANT)
}

/// Every conversation with a state.
pub fn states(shared: &vak_config::scope::SharedScope) -> Vec<(String, ConversationState)> {
    conversation_state::all(&tenant(shared)).unwrap_or_default()
}

/// Every session id that is hidden: in the trash, or erased.
pub fn trashed(shared: &vak_config::scope::SharedScope) -> HashSet<String> {
    states(shared)
        .into_iter()
        .filter_map(|(id, state)| state.hidden().then_some(id))
        .collect()
}

pub fn is_trashed(shared: &vak_config::scope::SharedScope, session_id: &str) -> bool {
    conversation_state::get(&tenant(shared), session_id).is_ok_and(|state| state.hidden())
}

/// Whether each conversation with a state is archived. One in the trash
/// was archived first and still reads as archived.
pub fn archived(shared: &vak_config::scope::SharedScope) -> HashMap<String, bool> {
    states(shared)
        .into_iter()
        .map(|(id, state)| (id, state.archived || state.hidden()))
        .collect()
}

/// Archives a conversation or brings it back to the default list.
pub fn set_archived(
    shared: &vak_config::scope::SharedScope,
    session_id: &str,
    archived: bool,
) -> std::io::Result<()> {
    conversation_state::update(&tenant(shared), session_id, |state| {
        state.archived = archived
    })
    .map(|_| ())
    .map_err(std::io::Error::other)
}

/// What a cross-session search must skip: the trash and, when there is one,
/// the session doing the searching.
pub fn search_exclusions(
    shared: &vak_config::scope::SharedScope,
    current: Option<&str>,
) -> HashSet<String> {
    let mut excluded = trashed(shared);
    excluded.extend(current.map(str::to_string));
    excluded
}

/// Moves sessions into or out of the trash. Moving one in starts its
/// window now; one already there keeps the time it went in.
pub fn set(
    shared: &vak_config::scope::SharedScope,
    session_ids: &[String],
    trashed: bool,
) -> std::io::Result<()> {
    let now = chrono::Utc::now();
    for id in session_ids {
        conversation_state::update(&tenant(shared), id, |state| {
            state.trashed_at = match (trashed, state.trashed_at) {
                (true, already) => Some(already.unwrap_or(now)),
                (false, _) => None,
            };
        })
        .map_err(std::io::Error::other)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_trash_window_starts_once_and_the_archive_survives_a_restore() {
        let home = tempfile::tempdir().unwrap();
        let shared = vak_config::scope::SharedScope::new(home.path());
        set_archived(&shared, "s9", true).unwrap();
        set(&shared, &["s9".into()], true).unwrap();
        let entered = states(&shared)[0].1.trashed_at.expect("a time it went in");
        // Trashing what is already there does not restart its window.
        set(&shared, &["s9".into()], true).unwrap();
        assert_eq!(states(&shared)[0].1.trashed_at, Some(entered));
        assert_eq!(archived(&shared).get("s9"), Some(&true));
        // Restored, it is archived again, as it was before.
        set(&shared, &["s9".into()], false).unwrap();
        assert!(!is_trashed(&shared, "s9"));
        assert_eq!(archived(&shared).get("s9"), Some(&true));
        assert!(
            !home.path().join("deleted.json").exists()
                && !home.path().join("archive.json").exists(),
            "no sidecar file is written"
        );
    }

    #[test]
    fn a_session_moves_into_and_out_of_the_trash() {
        let home = tempfile::tempdir().unwrap();
        assert!(!is_trashed(
            &vak_config::scope::SharedScope::new(home.path()),
            "s1"
        ));
        set(
            &vak_config::scope::SharedScope::new(home.path()),
            &["s1".into(), "s2".into()],
            true,
        )
        .unwrap();
        assert!(is_trashed(
            &vak_config::scope::SharedScope::new(home.path()),
            "s1"
        ));
        assert_eq!(
            trashed(&vak_config::scope::SharedScope::new(home.path())).len(),
            2
        );
        set(
            &vak_config::scope::SharedScope::new(home.path()),
            &["s1".into()],
            false,
        )
        .unwrap();
        assert!(!is_trashed(
            &vak_config::scope::SharedScope::new(home.path()),
            "s1"
        ));
        let excluded = search_exclusions(
            &vak_config::scope::SharedScope::new(home.path()),
            Some("current"),
        );
        assert!(excluded.contains("s2") && excluded.contains("current"));
        assert!(!excluded.contains("s1"));
    }
}
