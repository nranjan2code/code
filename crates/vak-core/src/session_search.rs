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
        // Curated memory participates in recall and outranks transcripts
        // (docs/design/26-learning.md). The global profile tier joins the
        // same extras ranking so user-level memories follow them across
        // projects (docs/design/29-personal-os.md P1).
        let notes = crate::memory::list_notes(&home, &cwd);
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
                }
            })
            .collect();
        let profile_ids: std::collections::HashSet<String> =
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
                    });
                    id
                })
                .collect();
        let result = tokio::task::spawn_blocking(move || {
            search_extended(&home, &cwd, &query, limit, Some(&exclude), &extras)
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
                    if profile_ids.contains(&h.session_id) {
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
        };
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
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
        };
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
        };
        let out = tool
            .execute(&serde_json::json!({"query": "anything at all"}), &ctx)
            .await;
        assert!(!out.is_error);
        assert!(out.content.contains("No past session matches"));
    }
}
