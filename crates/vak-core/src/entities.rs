//! Semantic Entity Knowledge Graph (tri-partite memory architecture).
//!
//! Stores typed domain entities with attributes and cross-entity relations
//! per workspace (and globally), each entity a Document named under the
//! Agent's `entities/<hash>/ENTITIES.jsonl` (`vak_core::documents`).
//!
//! Entities participate in recall via `session_search` and can be inspected
//! or updated across turns.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRelation {
    pub relation: String,
    pub target_entity_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id: String,
    pub name: String,
    pub entity_type: String,
    pub summary: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub relations: Vec<EntityRelation>,
    pub updated_at: DateTime<Utc>,
    /// The conversation and turn this entity was derived from, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<vak_session::trace::DerivedFrom>,
}

pub fn entities_file(home: &Path, cwd: Option<&Path>) -> PathBuf {
    vak_config::scope::AgentScope::new(home).entities_file(cwd)
}

/// Each entity is its own Document under the tier's name, so recording one
/// never rewrites the others (plan M3b slice 3).
fn entity_document(home: &Path, cwd: Option<&Path>, id: &str) -> Result<PathBuf, std::io::Error> {
    if id.is_empty() || id.contains(['/', '\\']) || id == "." || id == ".." {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("entity id {id:?} is not a plain slug"),
        ));
    }
    Ok(entities_file(home, cwd).join(id))
}

fn io_error(error: String) -> std::io::Error {
    std::io::Error::other(error)
}

/// List all entities in the target workspace (or global if cwd is None).
pub fn list_entities(home: &Path, cwd: Option<&Path>) -> Vec<EntityRecord> {
    vak_session::documents::under(&entities_file(home, cwd))
        .iter()
        .filter_map(|path| vak_session::documents::read(path).ok().flatten())
        .filter_map(|raw| serde_json::from_str::<EntityRecord>(&raw).ok())
        .collect()
}

/// Retrieve a specific entity by ID.
pub fn get_entity(home: &Path, cwd: Option<&Path>, id: &str) -> Option<EntityRecord> {
    let path = entity_document(home, cwd, id).ok()?;
    let raw = vak_session::documents::read(&path).ok().flatten()?;
    serde_json::from_str(&raw).ok()
}

/// Search entities by keyword across name, entity_type, summary, attributes, and relations.
pub fn search_entities(home: &Path, cwd: Option<&Path>, query: &str) -> Vec<EntityRecord> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return list_entities(home, cwd);
    }
    list_entities(home, cwd)
        .into_iter()
        .filter(|e| {
            e.name.to_ascii_lowercase().contains(&q)
                || e.entity_type.to_ascii_lowercase().contains(&q)
                || e.summary.to_ascii_lowercase().contains(&q)
                || e.attributes.iter().any(|(k, v)| {
                    k.to_ascii_lowercase().contains(&q) || v.to_ascii_lowercase().contains(&q)
                })
                || e.relations.iter().any(|r| {
                    r.relation.to_ascii_lowercase().contains(&q)
                        || r.target_entity_id.to_ascii_lowercase().contains(&q)
                })
        })
        .collect()
}

/// Records an entity as a new version of its Document.
pub fn upsert_entity(
    home: &Path,
    cwd: Option<&Path>,
    mut record: EntityRecord,
) -> Result<EntityRecord, std::io::Error> {
    let path = entity_document(home, cwd, &record.id)?;
    record.updated_at = Utc::now();
    let json = serde_json::to_string(&record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    vak_session::documents::update(&path, |_| Ok(Some((json.clone(), ())))).map_err(io_error)?;
    Ok(record)
}

/// Delete an entity by ID. Returns true if removed, false if not found.
pub fn delete_entity(home: &Path, cwd: Option<&Path>, id: &str) -> Result<bool, std::io::Error> {
    let Ok(path) = entity_document(home, cwd, id) else {
        return Ok(false);
    };
    vak_session::documents::forget(&path).map_err(io_error)
}

pub struct EntityRecordTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
}

#[async_trait::async_trait]
impl vak_tools::Tool for EntityRecordTool {
    fn name(&self) -> &str {
        "entity_record"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn description(&self) -> &str {
        "Record or update a domain entity in the semantic knowledge graph. \
         Entities represent durable people, places, projects, documents, datasets, systems \
         or concepts with typed attributes and directed relations (e.g. works_with, part_of, \
         owns)."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Unique slug identifier (e.g. 'asha-rao' or 'kitchen-renovation'). If omitted, slug is derived from name."
                },
                "name": {
                    "type": "string",
                    "description": "Human-readable name of the entity"
                },
                "entity_type": {
                    "type": "string",
                    "description": "Category or kind of entity (e.g. 'system', 'service', 'dataset', 'person', 'concept', 'module')"
                },
                "summary": {
                    "type": "string",
                    "description": "Concise summary of the entity's purpose, role, or definition"
                },
                "attributes": {
                    "type": "object",
                    "description": "Key-value map of attributes or metadata"
                },
                "relations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "relation": { "type": "string", "description": "Relationship verb (e.g. 'depends_on', 'hosted_on', 'owns')" },
                            "target": { "type": "string", "description": "Target entity ID" }
                        },
                        "required": ["relation", "target"]
                    },
                    "description": "Outgoing directed relationships to other entities"
                }
            },
            "required": ["name", "entity_type", "summary"]
        })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &vak_tools::ToolContext,
    ) -> vak_tools::ToolOutput {
        let Some(name) = args.get("name").and_then(|v| v.as_str()).map(str::trim) else {
            return vak_tools::ToolOutput::error("missing required argument 'name'");
        };
        let Some(entity_type) = args
            .get("entity_type")
            .and_then(|v| v.as_str())
            .map(str::trim)
        else {
            return vak_tools::ToolOutput::error("missing required argument 'entity_type'");
        };
        let Some(summary) = args.get("summary").and_then(|v| v.as_str()).map(str::trim) else {
            return vak_tools::ToolOutput::error("missing required argument 'summary'");
        };
        if name.is_empty() || entity_type.is_empty() || summary.is_empty() {
            return vak_tools::ToolOutput::error(
                "'name', 'entity_type', and 'summary' must not be empty",
            );
        }

        let id = match args.get("id").and_then(|v| v.as_str()).map(str::trim) {
            Some(custom) if !custom.is_empty() => custom.to_string(),
            _ => {
                let slug: String = name
                    .to_ascii_lowercase()
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .collect();
                let deduped = slug
                    .split('-')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("-");
                if deduped.is_empty() {
                    format!("ent-{}", vak_config::scope::workspace_key(Path::new(name)))
                } else {
                    deduped
                }
            }
        };

        let mut attributes = BTreeMap::new();
        if let Some(obj) = args.get("attributes").and_then(|v| v.as_object()) {
            for (k, v) in obj {
                let val_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                attributes.insert(k.clone(), val_str);
            }
        }

        let mut relations = Vec::new();
        if let Some(arr) = args.get("relations").and_then(|v| v.as_array()) {
            for item in arr {
                if let (Some(rel), Some(tgt)) = (
                    item.get("relation").and_then(|v| v.as_str()),
                    item.get("target").and_then(|v| v.as_str()),
                ) {
                    relations.push(EntityRelation {
                        relation: rel.trim().to_string(),
                        target_entity_id: tgt.trim().to_string(),
                    });
                }
            }
        }

        let record = EntityRecord {
            id: id.clone(),
            name: name.to_string(),
            entity_type: entity_type.to_string(),
            summary: summary.to_string(),
            attributes,
            relations,
            updated_at: Utc::now(),
            derived_from: None,
        };

        match upsert_entity(&self.sessions_home, Some(&self.cwd), record) {
            Ok(saved) => vak_tools::ToolOutput::ok(format!(
                "recorded entity '{id}' ({}) with {} attributes and {} relations",
                saved.entity_type,
                saved.attributes.len(),
                saved.relations.len()
            )),
            Err(e) => vak_tools::ToolOutput::error(format!("could not record entity: {e}")),
        }
    }

    fn claims(&self, _args: &serde_json::Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: vec![],
        }
    }
}

pub struct EntityQueryTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
}

#[async_trait::async_trait]
impl vak_tools::Tool for EntityQueryTool {
    fn name(&self) -> &str {
        "entity_query"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["memory"]
    }

    fn description(&self) -> &str {
        "Query the semantic entity knowledge graph by keyword or entity type. \
         Returns matching entities with their attributes and related connections."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to match against entity names, summaries, attributes, or relations"
                },
                "entity_type": {
                    "type": "string",
                    "description": "Optional filter by entity category (e.g. 'system', 'dataset')"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return (default 10, max 50)"
                }
            }
        })
    }

    async fn execute(
        &self,
        args: &serde_json::Value,
        _ctx: &vak_tools::ToolContext,
    ) -> vak_tools::ToolOutput {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let entity_type_filter = args
            .get("entity_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_ascii_lowercase());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|l| l as usize)
            .unwrap_or(10)
            .min(50);

        let mut results = search_entities(&self.sessions_home, Some(&self.cwd), query);
        if let Some(ref filter) = entity_type_filter {
            results.retain(|e| e.entity_type.to_ascii_lowercase() == *filter);
        }
        results.truncate(limit);

        if results.is_empty() {
            return vak_tools::ToolOutput::ok("no matching entities found in knowledge graph");
        }

        let mut out = format!("Found {} entities:\n\n", results.len());
        for e in results {
            out.push_str(&format!(
                "### [{}] {} (`{}`)\n",
                e.entity_type, e.name, e.id
            ));
            out.push_str(&format!("{}\n", e.summary));
            if !e.attributes.is_empty() {
                out.push_str("Attributes:\n");
                for (k, v) in &e.attributes {
                    out.push_str(&format!("- **{k}**: {v}\n"));
                }
            }
            if !e.relations.is_empty() {
                out.push_str("Relations:\n");
                for r in &e.relations {
                    out.push_str(&format!("- {} -> `{}`\n", r.relation, r.target_entity_id));
                }
            }
            out.push('\n');
        }

        vak_tools::ToolOutput::ok(out.trim_end().to_string())
    }

    fn claims(&self, _args: &serde_json::Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn entity_crud_and_search_roundtrip() {
        vak_config::paths::isolate_home_for_tests();
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let cwd = temp.path().join("cwd");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        let mut attrs = BTreeMap::new();
        attrs.insert("role".into(), "primary_db".into());
        attrs.insert("engine".into(), "postgresql".into());

        let entity = EntityRecord {
            id: "ent-postgres-1".into(),
            name: "Main PostgreSQL Cluster".into(),
            entity_type: "database".into(),
            summary: "Primary transaction database holding customer accounts".into(),
            attributes: attrs,
            relations: vec![EntityRelation {
                relation: "hosted_on".into(),
                target_entity_id: "srv-aws-us-east-1".into(),
            }],
            updated_at: Utc::now(),
            derived_from: None,
        };

        // 1. Upsert
        let saved = upsert_entity(&home, Some(&cwd), entity.clone()).unwrap();
        assert_eq!(saved.id, "ent-postgres-1");

        // 2. Get
        let fetched = get_entity(&home, Some(&cwd), "ent-postgres-1").unwrap();
        assert_eq!(fetched.name, "Main PostgreSQL Cluster");
        assert_eq!(fetched.attributes.get("engine").unwrap(), "postgresql");

        // 3. Search
        let results = search_entities(&home, Some(&cwd), "transaction");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "ent-postgres-1");

        let rel_results = search_entities(&home, Some(&cwd), "aws-us-east-1");
        assert_eq!(rel_results.len(), 1);

        let none_results = search_entities(&home, Some(&cwd), "redis");
        assert!(none_results.is_empty());

        // 4. Update
        let mut updated = fetched;
        updated.summary = "Updated summary".into();
        upsert_entity(&home, Some(&cwd), updated).unwrap();
        let re_fetched = get_entity(&home, Some(&cwd), "ent-postgres-1").unwrap();
        assert_eq!(re_fetched.summary, "Updated summary");

        // 5. Delete
        assert!(delete_entity(&home, Some(&cwd), "ent-postgres-1").unwrap());
        assert!(get_entity(&home, Some(&cwd), "ent-postgres-1").is_none());
        assert!(!delete_entity(&home, Some(&cwd), "ent-postgres-1").unwrap());
    }

    #[tokio::test]
    async fn entity_record_and_query_tools_execute() {
        vak_config::paths::isolate_home_for_tests();
        use vak_tools::Tool;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let cwd = temp.path().join("cwd");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        let record_tool = EntityRecordTool {
            sessions_home: home.clone(),
            cwd: cwd.clone(),
        };
        let query_tool = EntityQueryTool {
            sessions_home: home,
            cwd: cwd.clone(),
        };
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: tokio_util::sync::CancellationToken::new(),
            sandbox: None,
            sandbox_sink: None,
            agent_id: None,
            trace: None,
            new_documents: Vec::new(),
        };

        // Record entity via tool
        let record_args = serde_json::json!({
            "name": "Auth Gateway",
            "entity_type": "service",
            "summary": "Handles JWT authentication and token exchange",
            "attributes": { "port": "8080", "protocol": "https" },
            "relations": [ { "relation": "depends_on", "target": "redis-session-store" } ]
        });
        let rec_out = record_tool.execute(&record_args, &ctx).await;
        assert!(!rec_out.is_error, "{}", rec_out.content);
        assert!(
            rec_out.content.contains("recorded entity 'auth-gateway'"),
            "{}",
            rec_out.content
        );

        // Query entity via tool
        let query_args = serde_json::json!({ "query": "JWT" });
        let q_out = query_tool.execute(&query_args, &ctx).await;
        assert!(!q_out.is_error, "{}", q_out.content);
        assert!(q_out.content.contains("Auth Gateway"), "{}", q_out.content);
        assert!(
            q_out
                .content
                .contains("depends_on -> `redis-session-store`"),
            "{}",
            q_out.content
        );

        // Filter query by entity_type
        let type_args = serde_json::json!({ "entity_type": "service" });
        let type_out = query_tool.execute(&type_args, &ctx).await;
        assert!(!type_out.is_error, "{}", type_out.content);
        assert!(
            type_out.content.contains("Auth Gateway"),
            "{}",
            type_out.content
        );

        let none_args = serde_json::json!({ "entity_type": "database" });
        let none_out = query_tool.execute(&none_args, &ctx).await;
        assert!(!none_out.is_error, "{}", none_out.content);
        assert!(
            none_out.content.contains("no matching entities"),
            "{}",
            none_out.content
        );
    }
}
