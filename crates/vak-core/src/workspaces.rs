//! Which workspaces a person wants to *see*, as distinct from which ones
//! exist: a view over the space registry (`vak_config::spaces`), which
//! records when each space was last opened and whether it was forgotten.
//!
//! # Forgetting is not deleting
//!
//! Forgetting a workspace removes it from the lists a surface shows. It
//! does not touch its session ledgers, memory, checkpoints, receipts or
//! commitments, nor the project's `.vak/config.toml`, secret scope or trust
//! decision. So opening it again restores everything. A space is hidden
//! by its id, from the admin console's Projects (`spaces::set_forgotten`);
//! erasing what a space holds is erasure's, never this.
//!
//! # One store, every surface
//!
//! The desktop, the browser client and the admin console all read this
//! one record (AGENTS.md invariant 30).

use std::path::{Path, PathBuf};

/// How many recently opened workspaces a surface offers.
const MAX_KNOWN: usize = 24;

fn io(error: String) -> std::io::Error {
    std::io::Error::other(error)
}

/// Record that `path` was opened: shown first, and shown again if it was
/// forgotten. Re-opening is the un-forget; there is no separate verb.
pub fn remember(path: &Path) -> std::io::Result<()> {
    vak_config::spaces::opened(path).map(drop).map_err(io)
}

/// Whether `path`'s space has been explicitly forgotten.
pub fn is_forgotten(path: &Path) -> bool {
    vak_config::spaces::bound_space(path).is_some_and(|id| {
        vak_config::spaces::all()
            .into_iter()
            .any(|space| space.id == id && space.forgotten)
    })
}

/// The workspaces a surface should offer: the recently opened ones, newest
/// first, then `discovered` (a shell's cwd, the gateway's workspace), minus
/// anything forgotten and any folder that no longer exists.
pub fn visible(discovered: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let spaces = vak_config::spaces::all();
    let forgotten = |path: &Path| {
        vak_config::spaces::bound_space(path)
            .is_some_and(|id| spaces.iter().any(|space| space.id == id && space.forgotten))
    };
    let recent = spaces
        .iter()
        .filter(|space| !space.forgotten && space.last_opened.is_some())
        .filter_map(|space| space.folder.clone())
        .take(MAX_KNOWN);
    let mut out: Vec<PathBuf> = Vec::new();
    for path in recent.chain(discovered) {
        let path = path.canonicalize().unwrap_or(path);
        if out.contains(&path) || !path.is_dir() || forgotten(&path) {
            continue;
        }
        out.push(path);
    }
    out
}

/// Whether any Agent holds a conversation for the space `space` in the
/// data home `home`.
pub fn space_has_conversations(home: &std::path::Path, space: &str) -> bool {
    std::fs::read_dir(home.join("agents"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|agent| {
            std::fs::read_dir(agent.path().join("sessions").join(space))
                .is_ok_and(|mut sessions| sessions.next().is_some())
        })
}

/// Says that the project `space`, made on another machine, is the folder
/// `folder` on this one (data-architecture plan M9).
pub fn place_project(space: &str, folder: &std::path::Path) -> Result<(), String> {
    let home = vak_config::paths::data_home();
    vak_config::spaces::attach(space, folder, |other| space_has_conversations(&home, other))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// One shared home for the test binary; each test asserts only about its
    /// own uniquely named workspace, which is its own space.
    fn workspace(name: &str) -> PathBuf {
        let home = vak_config::paths::isolate_home_for_tests();
        let ws = home.join(name);
        std::fs::create_dir_all(&ws).unwrap();
        ws.canonicalize().unwrap()
    }

    /// Hides the space of `ws`, as the admin console does, by its id.
    fn forget(ws: &Path) -> std::io::Result<()> {
        let id = vak_config::spaces::bind(ws).map_err(io)?;
        vak_config::spaces::set_forgotten(&id, true).map_err(io)
    }

    #[test]
    fn forgetting_hides_it_and_reopening_brings_it_back() {
        let ws = workspace("forget-roundtrip");

        remember(&ws).unwrap();
        assert!(visible([]).contains(&ws));

        forget(&ws).unwrap();
        assert!(!visible([]).contains(&ws), "forgotten but still shown");
        assert!(is_forgotten(&ws));

        // Re-opening is the un-forget. No separate verb to discover.
        remember(&ws).unwrap();
        assert!(visible([]).contains(&ws));
        assert!(!is_forgotten(&ws));
    }

    /// The whole safety argument: forgetting is a view decision, so nothing
    /// a later re-add would need may be destroyed by it.
    #[test]
    fn forgetting_touches_no_workspace_content() {
        let ws = workspace("forget-keeps-content");
        std::fs::create_dir_all(ws.join(".vak")).unwrap();
        std::fs::write(ws.join(".vak/config.toml"), "model = \"kept\"\n").unwrap();

        remember(&ws).unwrap();
        forget(&ws).unwrap();

        assert!(ws.is_dir(), "the workspace directory was removed");
        assert_eq!(
            std::fs::read_to_string(ws.join(".vak/config.toml")).unwrap(),
            "model = \"kept\"\n",
            "forgetting a workspace must not touch its settings"
        );
    }

    /// A forgotten workspace that something else rediscovers (a shell's
    /// cwd) must stay hidden — otherwise "remove" lasts until the next
    /// refresh, which is not removal.
    #[test]
    fn a_rediscovered_workspace_stays_forgotten() {
        let ws = workspace("forget-rediscovered");
        forget(&ws).unwrap();
        assert!(
            !visible([ws.clone()]).contains(&ws),
            "rediscovery resurrected a forgotten workspace"
        );
    }

    /// A folder that no longer exists is not worth offering.
    #[test]
    fn a_missing_directory_is_not_offered() {
        let ws = workspace("forget-missing");
        remember(&ws).unwrap();
        std::fs::remove_dir_all(&ws).unwrap();
        assert!(!visible([]).contains(&ws));
    }
}
