//! Autonomous Memory Consolidation & Invariant Distillation.
//!
//! Self-supervised pass over workspace episodic notes that:
//! 1. Identifies recurring procedural patterns and promotes them to immutable invariants.
//! 2. Detects contradictory preferences or decisions across sessions.
//! 3. Distills structured entity records from repeated domain facts.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::entities::{self, EntityRecord};
use crate::memory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsolidationReport {
    pub total_notes_examined: usize,
    pub promoted_invariants: Vec<String>,
    pub detected_conflicts: Vec<String>,
    pub distilled_entities: Vec<String>,
}

/// Analyze episodic notes for this workspace and consolidate them.
#[allow(clippy::collapsible_if)]
pub fn consolidate_memory(home: &Path, cwd: &Path) -> Result<ConsolidationReport, String> {
    let notes = memory::list_notes(home, cwd);
    let total_notes_examined = notes.len();

    let mut promoted_invariants = Vec::new();
    let mut detected_conflicts = Vec::new();
    let mut distilled_entities = Vec::new();

    let existing_invariants: HashSet<String> = notes
        .iter()
        .filter(|n| n.kind == "invariant")
        .map(|n| n.text.trim().to_ascii_lowercase())
        .collect();

    // 1. Procedural Invariant Promotion
    // Group procedural notes by normalized text -> list of session_ids
    let mut procedural_map: HashMap<String, (Vec<String>, String, String)> = HashMap::new();
    for note in &notes {
        if note.kind == "procedural" {
            let norm = note.text.trim().to_ascii_lowercase();
            let entry = procedural_map
                .entry(norm)
                .or_insert_with(|| (Vec::new(), note.tag.clone(), note.text.clone()));
            if !entry.0.contains(&note.session_id) {
                entry.0.push(note.session_id.clone());
            }
        }
    }

    for (norm, (sessions, tag, orig_text)) in procedural_map {
        let is_modal = norm.contains("always")
            || norm.contains("never")
            || norm.contains("must")
            || norm.contains("invariant");
        let is_multi_session = sessions.len() >= 2;

        if (is_modal || is_multi_session) && !existing_invariants.contains(&norm) {
            match memory::append_note(home, cwd, "invariant", &tag, "consolidation", &orig_text) {
                Ok(_) => promoted_invariants.push(orig_text),
                Err(e) => return Err(format!("failed to promote invariant: {e}")),
            }
        }
    }

    // 2. Contradiction / Conflict Detection
    // Group notes by tag
    let mut by_tag: HashMap<String, Vec<&memory::NoteBlock>> = HashMap::new();
    for note in &notes {
        if !note.tag.is_empty() {
            by_tag.entry(note.tag.clone()).or_default().push(note);
        }
    }

    for (tag, tag_notes) in by_tag {
        for i in 0..tag_notes.len() {
            for j in (i + 1)..tag_notes.len() {
                let text_a = tag_notes[i].text.to_ascii_lowercase();
                let text_b = tag_notes[j].text.to_ascii_lowercase();
                if is_contradiction(&text_a, &text_b) {
                    detected_conflicts.push(format!(
                        "Tag '{tag}': \"{}\" vs \"{}\"",
                        tag_notes[i].text.trim(),
                        tag_notes[j].text.trim()
                    ));
                }
            }
        }
    }

    // 3. Entity Distillation from structured facts
    let existing_entities = entities::list_entities(home, Some(cwd));
    let existing_entity_names: HashSet<String> = existing_entities
        .iter()
        .map(|e| e.name.to_ascii_lowercase())
        .collect();

    for note in &notes {
        if note.kind == "fact" {
            if let Some((name, entity_type, summary)) = extract_entity_pattern(&note.text) {
                if !existing_entity_names.contains(&name.to_ascii_lowercase()) {
                    let mut attrs = BTreeMap::new();
                    if !note.tag.is_empty() {
                        attrs.insert("tag".into(), note.tag.clone());
                    }
                    attrs.insert("source_session".into(), note.session_id.clone());

                    let slug: String = name
                        .to_ascii_lowercase()
                        .chars()
                        .map(|c| if c.is_alphanumeric() { c } else { '-' })
                        .collect();
                    let record = EntityRecord {
                        id: slug,
                        name: name.clone(),
                        entity_type,
                        summary,
                        attributes: attrs,
                        relations: Vec::new(),
                        updated_at: Utc::now(),
                        derived_from: None,
                    };

                    if entities::upsert_entity(home, Some(cwd), record).is_ok() {
                        distilled_entities.push(name);
                    }
                }
            }
        }
    }

    Ok(ConsolidationReport {
        total_notes_examined,
        promoted_invariants,
        detected_conflicts,
        distilled_entities,
    })
}

fn is_contradiction(a: &str, b: &str) -> bool {
    let opposing_pairs = [
        ("tabs", "spaces"),
        ("enable", "disable"),
        ("enabled", "disabled"),
        ("use", "do not use"),
        ("allow", "deny"),
        ("strict", "lenient"),
        ("always", "never"),
        ("async", "sync"),
    ];

    for (pos, neg) in opposing_pairs {
        if (a.contains(pos) && b.contains(neg)) || (a.contains(neg) && b.contains(pos)) {
            return true;
        }
    }
    false
}

fn extract_entity_pattern(text: &str) -> Option<(String, String, String)> {
    let lower = text.to_ascii_lowercase();
    let type_markers = [
        (" service is ", "service"),
        (" database is ", "database"),
        (" cluster is ", "cluster"),
        (" api is ", "api"),
        (" dataset is ", "dataset"),
        (" module is ", "module"),
        (" pipeline is ", "pipeline"),
    ];

    for (marker, entity_type) in type_markers {
        if let Some(pos) = lower.find(marker) {
            let is_idx = pos + marker.find(" is ").unwrap_or(0);
            let name_part = text[..is_idx].trim();
            let rest = text[is_idx + 4..].trim();
            if !name_part.is_empty()
                && !rest.is_empty()
                && name_part.split_whitespace().count() <= 6
            {
                let name = name_part
                    .trim_start_matches("The ")
                    .trim_start_matches("the ")
                    .trim();
                return Some((name.to_string(), entity_type.to_string(), rest.to_string()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn memory_consolidation_promotes_invariants_and_detects_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let cwd = temp.path().join("cwd");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        // 1. Invariant candidate: modal word "always"
        memory::append_note(
            &home,
            &cwd,
            "procedural",
            "db",
            "sess-1",
            "always verify database migration rollback plan",
        )
        .unwrap();

        // 2. Conflicting preference notes under same tag "formatting"
        memory::append_note(
            &home,
            &cwd,
            "preference",
            "formatting",
            "sess-1",
            "use tabs for indentation",
        )
        .unwrap();
        memory::append_note(
            &home,
            &cwd,
            "preference",
            "formatting",
            "sess-2",
            "use spaces for indentation",
        )
        .unwrap();

        // 3. Structured fact describing an entity
        memory::append_note(
            &home,
            &cwd,
            "fact",
            "infra",
            "sess-3",
            "The Payments API is the external service processing card payments",
        )
        .unwrap();

        let report = consolidate_memory(&home, &cwd).unwrap();
        assert_eq!(report.total_notes_examined, 4);

        // Verify invariant promotion
        assert_eq!(report.promoted_invariants.len(), 1);
        assert!(
            report.promoted_invariants[0]
                .contains("always verify database migration rollback plan")
        );

        // Verify conflict detection
        assert_eq!(report.detected_conflicts.len(), 1);
        assert!(report.detected_conflicts[0].contains("formatting"));

        // Verify entity distillation
        assert_eq!(report.distilled_entities.len(), 1);
        assert_eq!(report.distilled_entities[0], "Payments API");

        // Verify distilled entity exists in entity store
        let fetched = entities::get_entity(&home, Some(&cwd), "payments-api").unwrap();
        assert_eq!(fetched.name, "Payments API");
        assert_eq!(fetched.entity_type, "api");
    }
}
