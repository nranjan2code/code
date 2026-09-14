//! Semantic Entity Knowledge Graph (tri-partite memory architecture).
//!
//! Stores typed domain entities with attributes and cross-entity relations
//! per workspace in `<sessions_home>/entities/<hash>/ENTITIES.jsonl`
//! (and global entities in `<sessions_home>/entities/global/ENTITIES.jsonl`).
//!
//! Entities participate in recall via `session_search` and can be inspected
//! or updated across turns.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::memory::hash_cwd;

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
}

pub fn entities_file(home: &Path, cwd: Option<&Path>) -> PathBuf {
    let sub = match cwd {
        Some(dir) => hash_cwd(dir),
        None => "global".to_string(),
    };
    home.join("entities").join(sub).join("ENTITIES.jsonl")
}

/// List all entities in the target workspace (or global if cwd is None).
pub fn list_entities(home: &Path, cwd: Option<&Path>) -> Vec<EntityRecord> {
    let path = entities_file(home, cwd);
    let Ok(file) = File::open(&path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(record) = serde_json::from_str::<EntityRecord>(trimmed) {
            records.push(record);
        }
    }
    records
}

/// Retrieve a specific entity by ID.
pub fn get_entity(home: &Path, cwd: Option<&Path>, id: &str) -> Option<EntityRecord> {
    list_entities(home, cwd).into_iter().find(|e| e.id == id)
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

/// Upsert an entity record: updates in-place if matching ID exists, or appends.
pub fn upsert_entity(
    home: &Path,
    cwd: Option<&Path>,
    mut record: EntityRecord,
) -> Result<EntityRecord, std::io::Error> {
    let path = entities_file(home, cwd);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    record.updated_at = Utc::now();
    let mut all = list_entities(home, cwd);
    if let Some(pos) = all.iter().position(|e| e.id == record.id) {
        all[pos] = record.clone();
    } else {
        all.push(record.clone());
    }

    // Atomic overwrite via temp file
    let tmp_path = path.with_extension("tmp");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        for entry in &all {
            let json = serde_json::to_string(entry)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            writeln!(file, "{json}")?;
        }
        file.flush()?;
    }
    std::fs::rename(&tmp_path, &path)?;
    Ok(record)
}

/// Delete an entity by ID. Returns true if removed, false if not found.
pub fn delete_entity(home: &Path, cwd: Option<&Path>, id: &str) -> Result<bool, std::io::Error> {
    let path = entities_file(home, cwd);
    if !path.is_file() {
        return Ok(false);
    }
    let all = list_entities(home, cwd);
    let orig_len = all.len();
    let filtered: Vec<_> = all.into_iter().filter(|e| e.id != id).collect();
    if filtered.len() == orig_len {
        return Ok(false);
    }

    let tmp_path = path.with_extension("tmp");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        for entry in &filtered {
            let json = serde_json::to_string(entry)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            writeln!(file, "{json}")?;
        }
        file.flush()?;
    }
    std::fs::rename(&tmp_path, &path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_crud_and_search_roundtrip() {
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
}
