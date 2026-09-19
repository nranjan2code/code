//! `find_tools`: search the turn's deferred tool catalogue.
//!
//! Part of docs/design/68-context-engine.md §5's tool surface: tools not in
//! the always-visible core set are withheld from the prefix and reachable
//! only by name-and-description search here (or, on Anthropic, by the
//! provider's own tool search). No new dependency — matching is plain
//! case-insensitive substring and token overlap over each candidate's name,
//! description, and JSON-schema argument names/descriptions.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{Tool, ToolContext, ToolOutput};

pub struct FindToolsTool {
    catalogue: Vec<vak_llm::ToolDefinition>,
    /// Every match returned this run is also pushed here, so the caller
    /// (`vak_agent::Agent::tool_definitions`) can keep offering a
    /// discovered tool's full schema for the rest of the turn without the
    /// model needing to call `find_tools` again (docs/design/68 §5).
    discovered: Option<Arc<Mutex<Vec<vak_llm::ToolDefinition>>>>,
}

impl FindToolsTool {
    pub fn new(catalogue: Vec<vak_llm::ToolDefinition>) -> Self {
        FindToolsTool {
            catalogue,
            discovered: None,
        }
    }

    pub fn with_discovered_sink(mut self, sink: Arc<Mutex<Vec<vak_llm::ToolDefinition>>>) -> Self {
        self.discovered = Some(sink);
        self
    }
}

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 50;

#[async_trait]
impl Tool for FindToolsTool {
    fn name(&self) -> &str {
        "find_tools"
    }

    fn description(&self) -> &str {
        "Search tools not currently loaded, by name, description, or argument names/descriptions. Returns full schemas for the best matches, which are then usable for the rest of this turn."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Words to match against tool names, descriptions, and argument names/descriptions."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of matches to return (default 5)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
            return ToolOutput::error(
                r#"{"type":"invalid_arguments","message":"missing required string 'query'"}"#,
            );
        };
        if query.trim().is_empty() {
            return ToolOutput::error(
                r#"{"type":"invalid_arguments","message":"'query' must not be empty"}"#,
            );
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).clamp(1, MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT);
        let matches = rank(&self.catalogue, query, limit);
        if let Some(sink) = &self.discovered {
            let mut discovered = sink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for def in &matches {
                if !discovered.iter().any(|existing| existing.name == def.name) {
                    discovered.push((*def).clone());
                }
            }
        }
        let schemas: Vec<Value> = matches
            .iter()
            .map(|def| {
                json!({
                    "name": def.name,
                    "description": def.description,
                    "input_schema": def.parameters,
                })
            })
            .collect();
        ToolOutput::ok(json!({"matches": schemas}).to_string())
    }
}

fn rank<'a>(
    catalogue: &'a [vak_llm::ToolDefinition],
    query: &str,
    limit: usize,
) -> Vec<&'a vak_llm::ToolDefinition> {
    let query_lower = query.to_ascii_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    let mut scored: Vec<(i64, &vak_llm::ToolDefinition)> = catalogue
        .iter()
        .filter_map(|def| {
            let score = score_tool(def, &query_lower, &tokens);
            (score > 0).then_some((score, def))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, def)| def).collect()
}

fn score_tool(def: &vak_llm::ToolDefinition, query_lower: &str, tokens: &[&str]) -> i64 {
    let name_lower = def.name.to_ascii_lowercase();
    let mut haystack = format!("{} {}", name_lower, def.description.to_ascii_lowercase());
    collect_schema_text(&def.parameters, &mut haystack);

    let mut score = 0i64;
    if name_lower == query_lower {
        score += 100;
    }
    if name_lower.contains(query_lower) {
        score += 20;
    }
    if haystack.contains(query_lower) {
        score += 10;
    }
    for token in tokens {
        if !token.is_empty() && haystack.contains(token) {
            score += 5;
        }
    }
    score
}

fn collect_schema_text(schema: &Value, out: &mut String) {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (name, property) in properties {
        out.push(' ');
        out.push_str(&name.to_ascii_lowercase());
        if let Some(description) = property.get("description").and_then(Value::as_str) {
            out.push(' ');
            out.push_str(&description.to_ascii_lowercase());
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn def(name: &str, description: &str, schema: Value) -> vak_llm::ToolDefinition {
        vak_llm::ToolDefinition::new(name, description, schema)
    }

    fn ctx() -> ToolContext {
        ToolContext::new(PathBuf::from("."))
    }

    #[tokio::test]
    async fn matches_by_name_and_description() {
        let tool = FindToolsTool::new(vec![
            def("webfetch", "Fetch a URL over HTTP.", json!({})),
            def("bash", "Run a shell command.", json!({})),
        ]);
        let out = tool.execute(&json!({"query": "http"}), &ctx()).await;
        assert!(!out.is_error);
        let parsed: Value = serde_json::from_str(&out.content).unwrap();
        let matches = parsed["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "webfetch");
    }

    #[tokio::test]
    async fn matches_by_argument_name_and_description() {
        let tool = FindToolsTool::new(vec![def(
            "weather_lookup",
            "Look something up.",
            json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string", "description": "City to check the forecast for"}
                }
            }),
        )]);
        let out = tool.execute(&json!({"query": "forecast"}), &ctx()).await;
        let parsed: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(parsed["matches"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn returns_full_schema_for_matches() {
        let schema = json!({"type": "object", "properties": {"q": {"type": "string"}}});
        let tool = FindToolsTool::new(vec![def("search", "Search the web.", schema.clone())]);
        let out = tool.execute(&json!({"query": "search"}), &ctx()).await;
        let parsed: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(parsed["matches"][0]["input_schema"], schema);
    }

    #[tokio::test]
    async fn respects_limit_and_ranks_best_matches_first() {
        let tool = FindToolsTool::new(vec![
            def("search_exact", "search", json!({})),
            def("search_other", "search something else", json!({})),
            def("unrelated", "does nothing related", json!({})),
        ]);
        let out = tool
            .execute(&json!({"query": "search", "limit": 1}), &ctx())
            .await;
        let parsed: Value = serde_json::from_str(&out.content).unwrap();
        let matches = parsed["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "search_exact");
    }

    #[tokio::test]
    async fn no_matches_returns_an_empty_list_not_an_error() {
        let tool = FindToolsTool::new(vec![def("bash", "Run a shell command.", json!({}))]);
        let out = tool
            .execute(&json!({"query": "zzz_no_such_thing"}), &ctx())
            .await;
        assert!(!out.is_error);
        let parsed: Value = serde_json::from_str(&out.content).unwrap();
        assert!(parsed["matches"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn matches_are_pushed_into_the_discovered_sink() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let tool = FindToolsTool::new(vec![def("bash", "Run a shell command.", json!({}))])
            .with_discovered_sink(sink.clone());
        tool.execute(&json!({"query": "bash"}), &ctx()).await;
        let discovered = sink.lock().unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].name, "bash");
    }

    #[tokio::test]
    async fn missing_query_is_a_correctable_error() {
        let tool = FindToolsTool::new(vec![]);
        let out = tool.execute(&json!({}), &ctx()).await;
        assert!(out.is_error);
    }
}
