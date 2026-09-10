//! Declarative starter definitions. These are content-only seeds: the runtime
//! still validates, compiles, activates, and falls back through its generic
//! pipeline. Adding a seed never adds a renderer branch.

use super::{
    AccessibilitySpec, Binding, EmptyValue, FallbackSpec, LibraryScope, PresentationOrigin,
    PresentationSpec, Primitive, SpecNode, SpecValue, StoredPresentation, digest,
};
use std::collections::BTreeMap;

const EVERYDAY: &[(&str, &str)] = &[
    ("overview", "overview"),
    ("checklist", "checklist"),
    ("schedule", "schedule"),
    ("comparison", "comparison"),
    ("decision", "decision"),
    ("budget", "budget"),
    ("collection", "collection"),
    ("detail", "detail"),
    ("steps", "steps"),
    ("summary", "summary"),
    ("notes", "notes"),
    ("agenda", "agenda"),
    ("follow-up", "follow_up"),
    ("reminder", "reminder"),
    ("comparison-table", "comparison_table"),
    ("pros-cons", "pros_cons"),
    ("scorecard", "scorecard"),
    ("milestones", "milestones"),
    ("progress", "progress"),
    ("status", "status"),
    ("inventory", "inventory"),
    ("shopping-list", "shopping_list"),
    ("meal-plan", "meal_plan"),
    ("itinerary", "itinerary"),
    ("lesson", "lesson"),
    ("reading-list", "reading_list"),
    ("habit-plan", "habit_plan"),
    ("project-plan", "project_plan"),
    ("meeting-notes", "meeting_notes"),
    ("contact-log", "contact_log"),
    ("finance-summary", "finance_summary"),
    ("invoice-summary", "invoice_summary"),
    ("recipe-summary", "recipe_summary"),
    ("travel-options", "travel_options"),
    ("home-project", "home_project"),
    ("care-plan", "care_plan"),
    ("event-plan", "event_plan"),
    ("media-list", "media_list"),
    ("research-brief", "research_brief"),
    ("faq", "faq"),
    ("timeline", "timeline"),
    ("metric", "metric"),
];

const CODING: &[(&str, &str)] = &[
    ("diff-review", "coding.diff"),
    ("test-results", "test.report"),
    ("terminal-session", "terminal.view"),
    ("deployment-receipt", "coding.deployment"),
    ("incident-timeline", "coding.incident"),
    ("benchmark", "coding.benchmark"),
    ("architecture-review", "coding.architecture"),
    ("dependency-review", "coding.dependencies"),
    ("release-notes", "coding.release"),
    ("code-search", "coding.search"),
];

fn binding(path: &str) -> SpecValue {
    SpecValue::Binding(Binding {
        path: path.into(),
        required: false,
        empty: EmptyValue::EmptyText,
    })
}

#[allow(clippy::manual_unwrap_or_default)]
fn seed(id: &str, accepts: &str) -> StoredPresentation {
    let root_primitive = match accepts {
        "itinerary" | "schedule" | "timeline" | "milestones" | "incident-timeline" => {
            Primitive::Timeline
        }
        "checklist" | "shopping_list" | "reading_list" | "habit_plan" => Primitive::Checklist,
        "comparison" | "comparison_table" | "pros_cons" | "scorecard" => Primitive::Comparison,
        "budget" | "finance_summary" | "invoice_summary" | "inventory" => Primitive::Table,
        "steps" | "lesson" | "event_plan" | "care_plan" => Primitive::Steps,
        "progress" | "status" => Primitive::Progress,
        "metric" | "benchmark" => Primitive::Metric,
        "coding.diff" => Primitive::Diff,
        "test.report" => Primitive::TestMatrix,
        "terminal.view" => Primitive::Terminal,
        _ => {
            let primitives = [
                Primitive::Section,
                Primitive::Stack,
                Primitive::Row,
                Primitive::Timeline,
                Primitive::Checklist,
                Primitive::Table,
                Primitive::Comparison,
                Primitive::Steps,
                Primitive::Progress,
                Primitive::KeyValue,
                Primitive::Disclosure,
            ];
            primitives[id.bytes().map(usize::from).sum::<usize>() % primitives.len()]
        }
    };
    let mut props = BTreeMap::new();
    props.insert("title".into(), binding("$.title"));
    props.insert("subtitle".into(), binding("$.subtitle"));
    props.insert("items".into(), binding("$.items"));
    let mut text_props = BTreeMap::new();
    text_props.insert("text".into(), binding("$.summary"));
    let spec = PresentationSpec {
        schema_version: super::SPEC_SCHEMA_VERSION,
        id: format!("seed.{id}"),
        revision: 1,
        accepts: vec![accepts.into()],
        root: SpecNode {
            primitive: root_primitive,
            props,
            children: vec![SpecNode {
                primitive: Primitive::Text,
                props: text_props,
                children: Vec::new(),
                each: None,
                item: None,
            }],
            each: None,
            item: None,
        },
        fallback: FallbackSpec::default(),
        accessibility: AccessibilitySpec {
            summary: Some(Binding {
                path: "$.summary".into(),
                required: false,
                empty: EmptyValue::EmptyText,
            }),
        },
        metadata: BTreeMap::from([(String::from("seed"), String::from("true"))]),
    };
    StoredPresentation {
        digest: match digest(&spec) {
            Ok(value) => value,
            Err(_) => String::new(),
        },
        spec,
        origin: PresentationOrigin {
            scope: LibraryScope::Workspace,
            owner: "builtin".into(),
            plugin_id: None,
            generation: Some("seed-1".into()),
        },
        enabled: false,
    }
}

/// The built-in starter pack: 42 everyday definitions plus 10 coding-flow
/// definitions. They are disabled previews and may be activated explicitly.
pub fn built_in_seed_pack() -> Vec<StoredPresentation> {
    EVERYDAY
        .iter()
        .chain(CODING.iter())
        .map(|(id, accepts)| seed(id, accepts))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile, CompileInput, CompiledPresentation};

    #[test]
    fn seed_pack_is_rich_disabled_and_validated_by_host_types() {
        let pack = built_in_seed_pack();
        assert_eq!(pack.len(), 52);
        assert!(pack.iter().all(|record| !record.enabled));
        assert!(
            pack.iter()
                .any(|record| record.spec.accepts == ["coding.diff"])
        );
        assert!(pack.iter().all(|record| !record.digest.is_empty()));
        let mut primitives = Vec::new();
        for record in &pack {
            if !primitives.contains(&record.spec.root.primitive) {
                primitives.push(record.spec.root.primitive);
            }
        }
        assert!(primitives.len() >= 6);
    }

    #[test]
    fn every_seed_compiles_with_minimal_generic_payload() {
        // Keep the starter pack honest: a newly added seed must be consumable
        // by the same generic compiler used for user and plugin definitions.
        // This intentionally does not assert a domain-specific renderer.
        for record in built_in_seed_pack() {
            let semantic_type = record
                .spec
                .accepts
                .first()
                .expect("seed accepts one semantic type");
            let result = compile(
                &record.spec,
                &CompileInput {
                    semantic_type: semantic_type.clone(),
                    payload: serde_json::json!({
                        "title": "Example",
                        "subtitle": "A reusable starter",
                        "summary": "A concise example result",
                        "items": ["One", "Two"],
                    }),
                    fallback_text: "Example result".into(),
                },
            );
            assert!(
                matches!(result, CompiledPresentation::Rich(_)),
                "seed {} failed generic compilation: {:?}",
                record.spec.id,
                result
            );
        }
    }
}
