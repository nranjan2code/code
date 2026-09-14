//! `session_search` — model-visible cross-session recall
//! (docs/design/23-memory.md). Injected by `Core::run_turn_with` next to
//! the task and MCP tools; results are ordinary tool results, so invariant
//! 1 (model-visible ⇒ logged) holds by construction.

use std::path::PathBuf;

use serde_json::Value;

use vak_session::{DEFAULT_LIMIT, ExternalDoc, search_extended};

pub struct SessionSearchTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
    /// Usually the running session: its content is already in context.
    pub exclude_session_id: String,
    /// Optional audience scope. When set, transcript hits must carry the same
    /// frozen Agent and conversation audience in their header; an unscoped or
    /// legacy ledger is rejected rather than treated as shared data.
    pub agent_id: Option<String>,
    pub audience_id: Option<String>,
}

fn session_in_scope(
    home: &std::path::Path,
    cwd: &std::path::Path,
    session_id: &str,
    agent_id: Option<&str>,
    audience_id: Option<&str>,
) -> bool {
    if agent_id.is_none() && audience_id.is_none() {
        return true;
    }
    let path = vak_session::SessionPath::new_session_file(home, cwd, session_id);
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::BufRead;
    let Some(Ok(line)) = std::io::BufReader::new(file).lines().next() else {
        return false;
    };
    let Ok(entry) = serde_json::from_str::<vak_session::Entry>(&line) else {
        return false;
    };
    let vak_session::EntryPayload::Header(header) = entry.payload else {
        return false;
    };
    let agent_matches = agent_id.is_none_or(|wanted| {
        header
            .agent
            .as_ref()
            .is_some_and(|agent| agent.id == wanted)
    });
    let audience_matches = audience_id.is_none_or(|wanted| {
        header
            .conversation
            .as_ref()
            .is_some_and(|context| context.audience_id == wanted)
    });
    agent_matches && audience_matches
}

fn tag_suffix(tag: &str) -> String {
    if tag.is_empty() {
        String::new()
    } else {
        format!(" {tag}")
    }
}

#[async_trait::async_trait]
impl vak_tools::Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search PAST sessions of this workspace (other conversations, their \
         user requests and assistant answers) plus your durable memory notes \
         and the global user profile. Use when the user references earlier \
         work ('that script we wrote', 'the bug from Tuesday') or when prior \
         decisions or stated preferences would help. Returns ranked snippets \
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

    async fn execute(&self, args: &Value, ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(query) = args.get("query").and_then(Value::as_str) else {
            return vak_tools::ToolOutput::error("missing required argument 'query'");
        };
        if query.trim().is_empty() {
            return vak_tools::ToolOutput::error("'query' must not be empty");
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|l| l as usize)
            .unwrap_or(DEFAULT_LIMIT);

        let home = self.sessions_home.clone();
        let cwd = self.cwd.clone();
        let query = query.to_string();
        let exclude = self.exclude_session_id.clone();
        let agent_id = self.agent_id.clone();
        let audience_id = self.audience_id.clone();
        // Curated memory participates in recall and outranks transcripts
        // (docs/design/26-learning.md). The global profile tier joins the
        // same extras ranking so user-level memories follow them across
        // projects (docs/design/29-personal-os.md P1).
        let notes = crate::memory::list_notes(&home, &cwd)
            .into_iter()
            .filter(|note| {
                session_in_scope(
                    &home,
                    &cwd,
                    &note.session_id,
                    agent_id.as_deref(),
                    audience_id.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        let mut extras: Vec<ExternalDoc> = notes
            .iter()
            .map(|n| {
                let key = if n.tag.is_empty() {
                    n.kind.clone()
                } else {
                    n.tag.clone()
                };
                ExternalDoc {
                    id: key,
                    text: format!("[{}{}] {}", n.kind, tag_suffix(&n.tag), n.text),
                    ts: Some(n.ts),
                    role: Some("memory".into()),
                }
            })
            .collect();
        // A remote audience does not inherit the local user's global profile
        // merely because it selected the same Agent. Account linking must
        // explicitly grant that scope; local sessions (including the new
        // explicit `audience_id = "local"` admission) may receive these entries.
        let profile_note_ids: std::collections::HashSet<String> = if audience_id
            .as_deref()
            .is_none_or(|audience| audience == "local")
        {
            crate::memory::list_profile_notes(&home)
                .iter()
                .map(|n| {
                    let key = if n.tag.is_empty() {
                        n.kind.clone()
                    } else {
                        n.tag.clone()
                    };
                    let id = format!("profile/{key}");
                    extras.push(ExternalDoc {
                        id: id.clone(),
                        text: format!("[{}{}] {}", n.kind, tag_suffix(&n.tag), n.text),
                        ts: Some(n.ts),
                        role: Some("profile".into()),
                    });
                    id
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };
        for entity in crate::entities::list_entities(&home, Some(&cwd)) {
            extras.push(ExternalDoc {
                id: format!("entity/{}", entity.id),
                text: format!(
                    "[entity:{}] {} — {}{}",
                    entity.entity_type,
                    entity.name,
                    entity.summary,
                    if entity.attributes.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " ({})",
                            entity
                                .attributes
                                .iter()
                                .map(|(k, v)| format!("{k}: {v}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                ),
                ts: Some(entity.updated_at),
                role: Some("entity".into()),
            });
        }
        let result = tokio::task::spawn_blocking(move || {
            let mut hits = search_extended(
                &home,
                &cwd,
                &query,
                // Filter after a larger ranked window so an unrelated Agent's
                // hits cannot consume the caller's small result limit.
                limit.clamp(DEFAULT_LIMIT, 50).saturating_mul(2).min(50),
                Some(&exclude),
                &extras,
            )?;
            if agent_id.is_some() || audience_id.is_some() {
                hits.retain(|hit| {
                    hit.entry_id.is_empty()
                        || session_in_scope(
                            &home,
                            &cwd,
                            &hit.session_id,
                            agent_id.as_deref(),
                            audience_id.as_deref(),
                        )
                });
                hits.truncate(limit.clamp(1, 50));
            }
            Ok::<Vec<vak_session::SessionHit>, vak_session::SearchError>(hits)
        })
        .await;

        match result {
            Ok(Ok(hits)) if hits.is_empty() => {
                vak_tools::ToolOutput::ok("No past session matches that query.".to_string())
            }
            Ok(Ok(mut hits)) => {
                // search_extended labels every curated extra "memory";
                // re-tag the profile-tier subset so surfaces can tell
                // global profile recall apart from workspace memory.
                for h in &mut hits {
                    if profile_note_ids.contains(&h.session_id) {
                        h.role = "profile".into();
                    }
                }
                let mut out = String::with_capacity(256 * hits.len());
                out.push_str(&format!("{} hit(s), most relevant first:\n", hits.len()));
                for (i, h) in hits.iter().enumerate() {
                    out.push_str(&format!(
                        "\n[{}] {} · {} · {} (score {:.2})\n  \"{}\"\n",
                        i + 1,
                        h.session_id,
                        h.ts.format("%Y-%m-%d"),
                        h.role,
                        h.score,
                        h.snippet
                    ));
                }
                vak_tools::ToolOutput::ok(ctx.truncate_output(out))
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

    #[tokio::test]
    async fn profile_tier_recalled_as_profile_role_alongside_memory() {
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

        let tool = SessionSearchTool {
            sessions_home: home.to_path_buf(),
            cwd: cwd.clone(),
            exclude_session_id: "current".into(),
            agent_id: None,
            audience_id: None,
        };
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
            sandbox_sink: None,
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
        assert!(out.content.contains("profile/editor · "), "{}", out.content);
        assert!(out.content.contains("· profile (score "), "{}", out.content);
        assert!(out.content.contains("vim keybindings"), "{}", out.content);
    }

    #[tokio::test]
    async fn empty_stores_still_answer_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        let tool = SessionSearchTool {
            sessions_home: dir.path().to_path_buf(),
            cwd: cwd.clone(),
            exclude_session_id: String::new(),
            agent_id: None,
            audience_id: None,
        };
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
            sandbox_sink: None,
        };
        let out = tool
            .execute(&serde_json::json!({"query": "anything at all"}), &ctx)
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("No past session matches"));
    }

    #[tokio::test]
    async fn transcript_recall_is_scoped_to_agent_and_audience() {
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
                agent: Some(vak_session::types::AgentIdentity {
                    id: agent.into(),
                    revision: 1,
                    name: agent.into(),
                    personality: String::new(),
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
        let tool = SessionSearchTool {
            sessions_home: home,
            cwd,
            exclude_session_id: String::new(),
            agent_id: Some("researcher".into()),
            audience_id: Some("telegram:one".into()),
        };
        let ctx = vak_tools::ToolContext {
            cwd: dir.path().join("workspace"),
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
            sandbox_sink: None,
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
            },
        )
        .unwrap();

        let tool = SessionSearchTool {
            sessions_home: home,
            cwd: cwd.clone(),
            exclude_session_id: String::new(),
            agent_id: None,
            audience_id: None,
        };
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
            sandbox_sink: None,
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
