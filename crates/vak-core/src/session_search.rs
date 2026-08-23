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

#[async_trait::async_trait]
impl vak_tools::Tool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search PAST sessions of this workspace (other conversations, their \
         user requests and assistant answers). Use when the user references \
         earlier work ('that script we wrote', 'the bug from Tuesday') or \
         when prior decisions would help. Returns ranked snippets with the \
         session id and date. Read-only; current conversation is excluded."
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
        // (docs/design/26-learning.md).
        let notes = crate::memory::list_notes(&home, &cwd);
        let extras: Vec<ExternalDoc> = notes
            .iter()
            .map(|n| ExternalDoc {
                id: if n.tag.is_empty() {
                    n.kind.clone()
                } else {
                    n.tag.clone()
                },
                text: format!(
                    "[{}{}] {}",
                    n.kind,
                    if n.tag.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", n.tag)
                    },
                    n.text
                ),
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
            Ok(Ok(hits)) => {
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
