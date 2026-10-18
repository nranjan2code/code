//! What `recall` reaches beyond the conversation it runs in
//! (docs/design/23-memory.md): other conversations, saved notes, the
//! person's profile, followed sources and deliverables, in one
//! ranking over the data catalog (plan M6), filtered to the caller's Agent,
//! audience and space before ranking (invariant 37). It is no tool of its
//! own: `recall`'s search returns these hits beside this conversation's
//! turns, so the model never picks a scope (docs/audits/
//! history-lookup-2026-10-16.md). What it returns is an ordinary tool
//! result, so invariant 1 holds by construction.

use std::sync::Arc;

use serde_json::{Value, json};

/// The reach of one conversation's `recall` beyond itself.
pub struct Elsewhere {
    /// The catalog of this data home; `None` when it could not be opened,
    /// which makes every search say so.
    pub catalog: Option<Arc<vak_catalog::Catalog>>,
    /// Who is asking: the Agent and conversation audience, and the sessions
    /// never shown (the trash, and the running conversation, whose turns
    /// `recall` searches on its own).
    pub audience: vak_catalog::Audience,
    /// The workspace's space: its conversations and notes, and anything of
    /// no space (the profile).
    pub space: Option<String>,
    /// Where the trash is kept: read at every search, so a conversation
    /// trashed mid-turn is gone from the next one.
    pub trash: vak_config::scope::SharedScope,
    /// What the record says about a deliverable a hit names: its workspace
    /// path and state (docs/design/90-file-state.md §3.3), by artifact id.
    pub file: Option<FileLookup>,
    /// The card of a turn, by conversation and turn id: a saved note is
    /// read with the record of the turn that wrote it
    /// (docs/design/99-turns-that-do-not-poison.md §4 E).
    pub turn: Option<TurnLookup>,
}

/// An artifact id in, its file and state out (`Elsewhere::file`).
pub type FileLookup = Arc<dyn Fn(&str) -> Result<Option<String>, String> + Send + Sync>;

/// A conversation and a turn id in, that turn's card out (`Elsewhere::turn`).
pub type TurnLookup = Arc<dyn Fn(&str, &str) -> Option<vak_session::TurnCard> + Send + Sync>;

/// Where a hit is from, and the id `forget_memory` takes for a note.
fn hit_label(node: &vak_catalog::Node) -> (String, &'static str) {
    let leaf = || {
        node.locator
            .as_deref()
            .and_then(|path| std::path::Path::new(path).file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    match node.kind.as_str() {
        "memory" if node.status.as_deref() == Some("profile") => {
            (format!("profile/{}", leaf()), "profile")
        }
        "memory" => (format!("memory/{}", leaf()), "saved note"),
        "item" => (node.id.clone(), "followed source"),
        "artifact" => (node.id.clone(), "deliverable"),
        _ => (
            conversation_id(node).unwrap_or_default(),
            "another conversation",
        ),
    }
}

fn conversation_id(node: &vak_catalog::Node) -> Option<String> {
    node.session
        .as_deref()
        .map(|session| session.trim_start_matches("ses_").to_string())
}

impl Elsewhere {
    /// The hits for `query`, most relevant first, each saying where it is
    /// from. Blocking: the caller runs it off the async runtime.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<Value>, String> {
        let catalog = self
            .catalog
            .clone()
            .ok_or("the catalog could not be opened")?;
        let mut audience = self.audience.clone();
        audience
            .exclude_sessions
            .extend(crate::trash::trashed(&self.trash));
        let scope = vak_catalog::Scope {
            space: self.space.clone(),
            kinds: Some(
                ["session", "turn", "call", "memory", "item", "artifact"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
        };
        catalog.catch_up().map_err(|e| e.to_string())?;
        let hits = catalog
            .search(query, &audience, &scope, limit)
            .map_err(|e| e.to_string())?;
        hits.iter().map(|hit| self.hit(hit)).collect()
    }

    fn hit(&self, hit: &vak_catalog::Hit) -> Result<Value, String> {
        let (id, from) = hit_label(&hit.node);
        let mut out = json!({
            "from": from,
            "id": id,
            "date": hit.node.created_at.as_deref().and_then(|at| at.get(..10)).unwrap_or_default(),
            "text": hit.snippet,
        });
        // A turn of another conversation is reopened by this value; the
        // loop allows it only for a conversation an attached artifact came
        // from (plan M8.3b), and says so otherwise.
        if let (Some(conversation), Some(turn)) = (conversation_id(&hit.node), &hit.node.turn) {
            out["open"] = json!(format!("conversation:{conversation}/turn:{turn}"));
        }
        // An item names where it came from, so the answer can cite it.
        if hit.node.kind == "item"
            && let Some(link) = hit.node.locator.as_deref()
        {
            out["link"] = json!(link);
        }
        // A note is read with how the turn that wrote it went: its calls,
        // what it left undelivered, how it ended.
        if hit.node.kind == "memory"
            && let (Some(conversation), Some(turn), Some(lookup)) = (
                conversation_id(&hit.node),
                hit.node.turn.as_deref(),
                self.turn.as_ref(),
            )
        {
            let turn = turn.trim_start_matches("trn_");
            if let Some(card) = lookup(&conversation, turn) {
                out["made_in"] = json!(card.made_in(Some(&conversation)));
            }
        }
        // A deliverable names its file and what the record says of it now,
        // so a later turn can reach it by name.
        if hit.node.kind == "artifact"
            && let Some(lookup) = self.file.as_ref()
            && let Some(file) = lookup(&hit.node.id)?
        {
            out["file"] = json!(file);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// The reach over a catalog of the data home `home`, as Core builds it.
    fn reach_over(
        home: &std::path::Path,
        cwd: &std::path::Path,
        agent: Option<&str>,
        audience: Option<&str>,
    ) -> Elsewhere {
        let catalog = vak_catalog::Catalog::open(&home.join("catalog.db"), home).unwrap();
        Elsewhere {
            catalog: Some(Arc::new(catalog)),
            audience: vak_catalog::Audience {
                agents: agent
                    .map(|agent| vec![vak_session::trace::local::agent(agent).to_string()]),
                audience: audience.map(str::to_string),
                ..Default::default()
            },
            space: Some(vak_session::trace::local::space(cwd).to_string()),
            trash: vak_config::scope::SharedScope::new(home),
            file: None,
            turn: None,
        }
    }

    fn froms(hits: &[Value]) -> Vec<&str> {
        hits.iter().filter_map(|hit| hit["from"].as_str()).collect()
    }

    #[test]
    fn a_profile_note_is_found_as_the_profile_beside_saved_notes() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        crate::memory::append_note(
            home,
            &cwd,
            "decision",
            "deploys",
            "s1",
            "the deploy script lives in scripts/deploy.sh",
        )
        .unwrap();
        crate::memory::append_profile_note(
            home,
            "preference",
            "editor",
            "user prefers vim keybindings everywhere",
            "su",
        )
        .unwrap();
        let reach = reach_over(home, &cwd, None, None);

        let hits = reach.search("deploy script", 8).unwrap();
        assert!(froms(&hits).contains(&"saved note"), "{hits:?}");
        assert!(!froms(&hits).contains(&"profile"), "{hits:?}");

        let hits = reach.search("vim keybindings", 8).unwrap();
        let profile = hits
            .iter()
            .find(|hit| hit["from"] == "profile")
            .expect("profile hit");
        assert!(
            profile["id"].as_str().unwrap().starts_with("profile/"),
            "{hits:?}"
        );
        assert!(
            profile["text"]
                .as_str()
                .unwrap()
                .contains("vim keybindings")
        );
    }

    #[test]
    fn nothing_saved_finds_nothing() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        let reach = reach_over(dir.path(), &cwd, None, None);
        assert!(reach.search("anything at all", 8).unwrap().is_empty());
    }

    #[test]
    fn other_conversations_are_scoped_to_agent_and_audience() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("workspace");
        std::fs::create_dir_all(&cwd).unwrap();
        for (id, agent, audience) in [
            ("one", "researcher", "telegram:one"),
            ("two", "writer", "telegram:two"),
        ] {
            let path = vak_session::SessionPath::new_session_file(&home, &cwd, id);
            let header = vak_session::SessionHeader {
                space: None,
                run: None,
                cause: None,
                agent: Some(vak_session::types::AgentIdentity {
                    id: agent.into(),
                    revision: 1,
                    name: agent.into(),
                    character: "vak".into(),
                    personality: String::new(),
                    animation: "subtle".into(),
                    voice: "default".into(),
                    behaviour: String::new(),
                    responsibilities: String::new(),
                    instructions: String::new(),
                }),
                session_id: id.into(),
                created_at: chrono::Utc::now(),
                cwd: cwd.clone(),
                parent_session_id: None,
                contract_id: None,
                work_item_id: None,
                conversation: Some(vak_session::ConversationContext {
                    conversation_id: id.into(),
                    audience_id: audience.into(),
                    origin: None,
                }),
                contract: vak_session::FrozenContract {
                    app_version: "test".into(),
                    provider: "test".into(),
                    model: "test".into(),
                    route_ladder: Vec::new(),
                    route_objective: String::new(),
                    route_annotations: Vec::new(),
                    system_prompt: String::new(),
                    permission_mode: "read-only".into(),
                    capabilities: Vec::new(),
                    prompt_layers: Vec::new(),
                },
            };
            let mut log = vak_session::SessionLog::create(path, header).unwrap();
            log.append_message(vak_session::MessageRecord {
                message: vak_llm::Message::user_text("private launch plan"),
                meta: None,
            })
            .unwrap();
        }
        let reach = reach_over(&home, &cwd, Some("researcher"), Some("telegram:one"));
        let hits = reach.search("private launch plan", 10).unwrap();
        let ids: Vec<&str> = hits.iter().filter_map(|hit| hit["id"].as_str()).collect();
        assert!(ids.contains(&"one"), "{hits:?}");
        assert!(!ids.contains(&"two"), "{hits:?}");
    }
}
