//! `session_search` — model-visible cross-session recall
//! (docs/design/23-memory.md) over the data catalog (plan M6): past
//! conversations, memory notes, the owner's profile and entities, in one
//! ranking, filtered to the caller's Agent and audience before ranking
//! (invariant 37). Results are ordinary tool results, so invariant 1
//! (model-visible ⇒ logged) holds by construction.

use std::sync::Arc;

use serde_json::Value;

/// Hits returned when the call names no limit.
pub const DEFAULT_LIMIT: usize = 8;

pub struct SessionSearchTool {
    /// The catalog of this data home; `None` when it could not be opened,
    /// which makes every call fail closed.
    pub catalog: Option<Arc<vak_catalog::Catalog>>,
    /// Who is asking: the Agent and conversation audience, and the sessions
    /// never shown (the trash, and the running session, whose content is
    /// already in context).
    pub audience: vak_catalog::Audience,
    /// The workspace's space: its conversations and notes, and anything of
    /// no space (the profile).
    pub space: Option<String>,
    /// Where the trash is kept: read at every call, so a session trashed
    /// mid-turn is gone from the next search.
    pub trash: vak_config::scope::SharedScope,
}

/// How a hit is named to the model: the id `forget_memory` and recall take.
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
        "memory" => (format!("memory/{}", leaf()), "memory"),
        "entity" => (format!("entity/{}", leaf()), "entity"),
        "item" => (node.id.clone(), "from a source"),
        _ => (
            node.session
                .as_deref()
                .map(|session| session.trim_start_matches("ses_").to_string())
                .unwrap_or_default(),
            "conversation",
        ),
    }
}

#[async_trait::async_trait]
impl vak_tools::Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Search past conversations and sessions (user requests and assistant answers), \
         your durable memory notes, the user profile, and items from the sources you \
         follow (feeds, news, saved attachments). Memory hits include \
         an exact memory/<id> or profile/<id> source identifier for forget_memory. \
         Use when the user \
         references earlier work ('that script we wrote', 'the bug from Tuesday') or when \
         prior decisions or stated preferences would help. Returns ranked snippets \
         with the source id and date. Read-only; current conversation is \
         excluded."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords or an exact phrase to look for across past sessions"
                },
                "limit": {
                    "type": "integer",
                    "description": format!("Max hits to return (default {DEFAULT_LIMIT}, max 50)")
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return vak_tools::ToolOutput::error("missing required argument 'query'");
        };
        if query.trim().is_empty() {
            return vak_tools::ToolOutput::error("'query' must not be empty");
        }
        let Some(catalog) = self.catalog.clone() else {
            return vak_tools::ToolOutput::error(
                "session search is unavailable: the catalog could not be opened",
            );
        };
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|l| l as usize)
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, 50);
        let query = query.to_string();
        let mut audience = self.audience.clone();
        audience
            .exclude_sessions
            .extend(crate::trash::trashed(&self.trash));
        let scope = vak_catalog::Scope {
            space: self.space.clone(),
            kinds: Some(
                ["session", "turn", "call", "memory", "entity", "item"]
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
            ),
        };
        let result = tokio::task::spawn_blocking(move || {
            catalog.catch_up()?;
            catalog.search(&query, &audience, &scope, limit)
        })
        .await;
        match result {
            Ok(Ok(hits)) if hits.is_empty() => {
                vak_tools::ToolOutput::ok("No past session matches that query.".to_string())
            }
            Ok(Ok(hits)) => {
                let mut out = String::with_capacity(256 * hits.len());
                out.push_str(&format!("{} hit(s), most relevant first:\n", hits.len()));
                for (i, hit) in hits.iter().enumerate() {
                    let (id, role) = hit_label(&hit.node);
                    let date = hit
                        .node
                        .created_at
                        .as_deref()
                        .and_then(|at| at.get(..10))
                        .unwrap_or_default();
                    out.push_str(&format!(
                        "\n[{}] {id} · {date} · {role} (score {:.2})\n  \"{}\"\n",
                        i + 1,
                        hit.score,
                        hit.snippet
                    ));
                    // An item names where it came from, so the answer can cite it.
                    if hit.node.kind == "item"
                        && let Some(link) = hit.node.locator.as_deref()
                    {
                        out.push_str(&format!("  {link}\n"));
                    }
                }
                vak_tools::ToolOutput::ok(out)
            }
            Ok(Err(e)) => vak_tools::ToolOutput::error(format!("session search failed: {e}")),
            Err(e) => vak_tools::ToolOutput::error(format!("search task failed: {e}")),
        }
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use vak_tools::Tool;

    /// The tool over a catalog of the data home `home`, as Core builds it.
    fn tool_over(
        home: &std::path::Path,
        cwd: &std::path::Path,
        agent: Option<&str>,
        audience: Option<&str>,
    ) -> SessionSearchTool {
        let catalog = vak_catalog::Catalog::open(&home.join("catalog.db"), home).unwrap();
        SessionSearchTool {
            catalog: Some(Arc::new(catalog)),
            audience: vak_catalog::Audience {
                agents: agent
                    .map(|agent| vec![vak_session::trace::local::agent(agent).to_string()]),
                audience: audience.map(str::to_string),
                ..Default::default()
            },
            space: Some(vak_session::trace::local::space(cwd).to_string()),
            trash: vak_config::scope::SharedScope::new(home),
        }
    }

    #[tokio::test]
    async fn profile_tier_recalled_as_profile_role_alongside_memory() {
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

        let tool = tool_over(home, &cwd, None, None);
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            sandbox: None,
            sandbox_sink: None,
            agent_id: None,
            trace: None,
            new_documents: Vec::new(),
            executions: None,
        };

        let out = tool
            .execute(&serde_json::json!({"query": "deploy script"}), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("· memory"), "{}", out.content);
        assert!(!out.content.contains("· profile"), "{}", out.content);

        let out = tool
            .execute(&serde_json::json!({"query": "vim keybindings"}), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.content);
        // Profile-tier hits carry the profile/ id prefix AND role.
        assert!(out.content.contains("profile/"), "{}", out.content);
        assert!(out.content.contains("· profile (score "), "{}", out.content);
        assert!(out.content.contains("vim keybindings"), "{}", out.content);
    }

    #[tokio::test]
    async fn empty_stores_still_answer_cleanly() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        let tool = tool_over(dir.path(), &cwd, None, None);
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            sandbox: None,
            sandbox_sink: None,
            agent_id: None,
            trace: None,
            new_documents: Vec::new(),
            executions: None,
        };
        let out = tool
            .execute(&serde_json::json!({"query": "anything at all"}), &ctx)
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("No past session matches"));
    }

    #[tokio::test]
    async fn transcript_recall_is_scoped_to_agent_and_audience() {
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
        let tool = tool_over(&home, &cwd, Some("researcher"), Some("telegram:one"));
        let ctx = vak_tools::ToolContext {
            cwd: dir.path().join("workspace"),
            cancel: tokio_util::sync::CancellationToken::new(),
            sandbox: None,
            sandbox_sink: None,
            agent_id: None,
            trace: None,
            new_documents: Vec::new(),
            executions: None,
        };
        let out = tool
            .execute(
                &serde_json::json!({"query": "private launch plan", "limit": 10}),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(out.content.contains("one"), "{}", out.content);
        assert!(!out.content.contains("two"), "{}", out.content);
    }

    #[tokio::test]
    async fn entities_recalled_via_session_search() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().join("workspace");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        // Create an entity in this workspace.
        crate::entities::upsert_entity(
            &home,
            Some(&cwd),
            crate::entities::EntityRecord {
                id: "ent-apollo-11".into(),
                name: "Project Apollo".into(),
                entity_type: "mission".into(),
                summary: "Lunar landing mission targeting Sea of Tranquility".into(),
                attributes: Default::default(),
                relations: Default::default(),
                updated_at: chrono::Utc::now(),
                derived_from: None,
            },
        )
        .unwrap();

        let tool = tool_over(&home, &cwd, None, None);
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            sandbox: None,
            sandbox_sink: None,
            agent_id: None,
            trace: None,
            new_documents: Vec::new(),
            executions: None,
        };
        let out = tool
            .execute(
                &serde_json::json!({"query": "Sea of Tranquility", "limit": 5}),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.content);
        assert!(
            out.content.contains("Project Apollo") && out.content.contains("mission"),
            "{}",
            out.content
        );
    }
}
