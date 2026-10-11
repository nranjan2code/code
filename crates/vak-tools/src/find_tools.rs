//! `find_tools`: search the turn's deferred tool catalogue.
//!
//! Part of docs/design/68-context-engine.md §5's tool surface: tools not in
//! the always-visible core set are withheld from the prefix and reachable
//! only by name-and-description search here (or, on Anthropic, by the
//! provider's own tool search). No new dependency — matching is plain
//! case-insensitive substring and token overlap over each candidate's name,
//! description, and JSON-schema argument names, descriptions, and enum values.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{Tool, ToolContext, ToolOutput};

pub struct FindToolsTool {
    catalogue: Vec<vak_llm::ToolDefinition>,
    /// Search-only vocabulary supplied by the capability registry (currently
    /// domain labels such as `live-data`). It is deliberately separate from
    /// the model-visible tool schema so discovery metadata cannot change the
    /// callable contract.
    keywords: HashMap<String, Vec<String>>,
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
            keywords: HashMap::new(),
            discovered: None,
        }
    }

    pub fn with_keywords(mut self, keywords: HashMap<String, Vec<String>>) -> Self {
        self.keywords = keywords;
        self
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

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Search all admitted tools, including already-loaded broker tools, by name, capability domain, description, argument names/descriptions, or supported choice values. Returns full schemas for the best matches; deferred matches become usable for the rest of this turn."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Words to match against tool names, descriptions, argument names/descriptions, and supported choice values."
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
        let matches = rank(&self.catalogue, &self.keywords, query, limit);
        let mut new_match_count = 0usize;
        if let Some(sink) = &self.discovered {
            let mut discovered = sink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for def in &matches {
                if !discovered.iter().any(|existing| existing.name == def.name) {
                    discovered.push((*def).clone());
                    new_match_count += 1;
                }
            }
        } else {
            new_match_count = matches.len();
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
        let mut result = json!({"matches": schemas, "new_match_count": new_match_count});
        if new_match_count == 0 && !matches.is_empty() {
            result["note"] = json!(
                "No new tool names matched beyond tools already returned this turn. The full admitted catalogue has been searched; use the best available match or state the limitation instead of repeating discovery with reworded queries."
            );
        }
        ToolOutput::ok(result.to_string())
    }
}

fn rank<'a>(
    catalogue: &'a [vak_llm::ToolDefinition],
    keywords: &HashMap<String, Vec<String>>,
    query: &str,
    limit: usize,
) -> Vec<&'a vak_llm::ToolDefinition> {
    let query_lower = query.to_ascii_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    let mut scored: Vec<(i64, &vak_llm::ToolDefinition)> = catalogue
        .iter()
        .filter_map(|def| {
            let score = score_tool(
                def,
                keywords
                    .get(&def.name)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                &query_lower,
                &tokens,
            );
            (score > 0).then_some((score, def))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.into_iter().take(limit).map(|(_, def)| def).collect()
}

fn score_tool(
    def: &vak_llm::ToolDefinition,
    keywords: &[String],
    query_lower: &str,
    tokens: &[&str],
) -> i64 {
    let name_lower = def.name.to_ascii_lowercase();
    let mut haystack = format!("{} {}", name_lower, def.description.to_ascii_lowercase());
    collect_schema_text(&def.parameters, &mut haystack);
    for keyword in keywords {
        haystack.push(' ');
        haystack.push_str(&keyword.to_ascii_lowercase());
    }

    let mut score = 0i64;
    if name_lower == query_lower {
        score += 100;
    }
    // A supported choice whose full value equals the query is a stronger
    // signal than a longer enum value that merely contains it (for example,
    // `board` should select the board card ahead of `social.x.takeaway_board`).
    if contains_exact_enum(&def.parameters, query_lower) {
        score += 60;
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
        // A query word that is a whole part of the tool's own name says more
        // than the same letters inside another tool's prose ("diff" in
        // "differ" ranked the comparison block above the diff block).
        if !token.is_empty() && name_lower.split('_').any(|part| part == *token) {
            score += 15;
        }
    }
    score
}

fn contains_exact_enum(value: &Value, query_lower: &str) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(|values| {
                    values.iter().any(|value| {
                        value
                            .as_str()
                            .is_some_and(|value| value.eq_ignore_ascii_case(query_lower))
                    })
                })
                || object
                    .values()
                    .any(|child| contains_exact_enum(child, query_lower))
        }
        Value::Array(values) => values
            .iter()
            .any(|child| contains_exact_enum(child, query_lower)),
        _ => false,
    }
}

fn collect_schema_text(schema: &Value, out: &mut String) {
    let Some(object) = schema.as_object() else {
        return;
    };

    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            out.push(' ');
            out.push_str(&name.to_ascii_lowercase());
            if let Some(description) = property.get("description").and_then(Value::as_str) {
                out.push(' ');
                out.push_str(&description.to_ascii_lowercase());
            }
            collect_schema_text(property, out);
        }
    }

    // Array-item properties are part of the arguments the model must fill,
    // and can contain the only useful discovery vocabulary for a card (for
    // example chart-point axes or timeline-step details). Walk nested object
    // schemas as well as top-level properties; enum values remain searchable
    // at every depth.
    if let Some(items) = object.get("items") {
        collect_schema_text(items, out);
    }
    collect_enum_text(schema, out);
}

fn collect_enum_text(value: &Value, out: &mut String) {
    match value {
        Value::Object(object) => {
            if let Some(values) = object.get("enum").and_then(Value::as_array) {
                for value in values {
                    if let Some(value) = value.as_str() {
                        out.push(' ');
                        out.push_str(&value.to_ascii_lowercase());
                    }
                }
            }
            for child in object.values() {
                collect_enum_text(child, out);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_enum_text(child, out);
            }
        }
        _ => {}
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
    async fn matches_nested_array_item_argument_descriptions() {
        let tool = FindToolsTool::new(vec![def(
            "emit_chart_card",
            "Emit a data card.",
            json!({
                "type": "object",
                "properties": {
                    "points": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "x": {"type": "string", "description": "Month shown on the horizontal axis"},
                                "y": {"type": "number", "description": "Revenue shown on the vertical axis"}
                            }
                        }
                    }
                }
            }),
        )]);

        for query in ["horizontal axis", "revenue vertical"] {
            let out = tool.execute(&json!({"query": query}), &ctx()).await;
            let parsed: Value = serde_json::from_str(&out.content).unwrap();
            assert_eq!(
                parsed["matches"][0]["name"], "emit_chart_card",
                "query: {query}"
            );
        }
    }

    #[tokio::test]
    async fn matches_supported_presentation_types_from_schema_enums() {
        let tool = FindToolsTool::new(vec![
            def(
                "emit_metric_card",
                "Emit a measurement card.",
                json!({
                    "type":"object",
                    "properties":{"semantic_type":{"type":"string","enum":["metric","telemetry.metric","weather"]}}
                }),
            ),
            def(
                "emit_timeline_card",
                "Emit an ordered plan card.",
                json!({
                    "type":"object",
                    "properties":{"semantic_type":{"type":"string","enum":["timeline","care_plan"]}}
                }),
            ),
        ]);

        for (query, expected) in [
            ("weather", "emit_metric_card"),
            ("houseplant care plan", "emit_timeline_card"),
        ] {
            let out = tool.execute(&json!({"query":query}), &ctx()).await;
            let parsed: Value = serde_json::from_str(&out.content).unwrap();
            assert_eq!(parsed["matches"][0]["name"], expected, "query: {query}");
            assert!(
                parsed["matches"][0]["input_schema"]["properties"]["semantic_type"]["enum"]
                    .is_array()
            );
        }
    }

    #[tokio::test]
    async fn repeated_search_results_explicitly_signal_that_discovery_did_not_expand() {
        let discovered = Arc::new(Mutex::new(Vec::new()));
        let tool = FindToolsTool::new(vec![def(
            "webfetch",
            "Fetch a known URL over HTTP.",
            json!({"type":"object","properties":{"url":{"type":"string"}}}),
        )])
        .with_discovered_sink(discovered);

        let first = tool.execute(&json!({"query":"webfetch"}), &ctx()).await;
        let first: Value = serde_json::from_str(&first.content).unwrap();
        assert_eq!(first["new_match_count"], 1);
        assert!(first.get("note").is_none());

        let repeat = tool
            .execute(&json!({"query":"http fetch URL"}), &ctx())
            .await;
        let repeat: Value = serde_json::from_str(&repeat.content).unwrap();
        assert_eq!(repeat["new_match_count"], 0);
        assert!(
            repeat["note"]
                .as_str()
                .unwrap()
                .contains("instead of repeating discovery")
        );
        assert_eq!(repeat["matches"][0]["name"], "webfetch");
    }

    #[tokio::test]
    async fn matches_loaded_broker_by_capability_domain() {
        let tool = FindToolsTool::new(vec![def(
            "mcp",
            "Call tools exposed by configured servers.",
            json!({}),
        )])
        .with_keywords(HashMap::from([(
            "mcp".into(),
            vec!["live-data".into(), "web".into()],
        )]));
        let out = tool
            .execute(&json!({"query": "live-data weather"}), &ctx())
            .await;
        let parsed: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(parsed["matches"][0]["name"], "mcp");
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
