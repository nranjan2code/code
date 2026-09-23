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
    ("recipe", "recipe"),
    ("news", "news"),
    ("ui-preview", "ui.preview"),
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

const UNIVERSAL: &[(&str, &str)] = &[
    ("map", "map"),
    ("route-map", "route_map"),
    ("calendar", "calendar"),
    ("availability", "availability"),
    ("board", "board"),
    ("entity", "entity"),
    ("search-results", "search_results"),
    ("evidence", "evidence"),
    ("decision-analysis", "decision_analysis"),
    ("document", "document"),
    ("table", "table"),
    ("dataframe", "dataframe"),
    ("graph", "graph"),
    ("form", "form"),
    ("action", "action"),
    ("transaction", "transaction"),
    ("alert", "alert"),
    ("conversation", "conversation"),
    ("progress-dashboard", "progress_dashboard"),
    ("simulation", "simulation"),
];

fn binding(path: &str) -> SpecValue {
    SpecValue::Binding(Binding {
        path: path.into(),
        required: false,
        empty: EmptyValue::EmptyText,
    })
}

fn required_binding(path: &str) -> SpecValue {
    SpecValue::Binding(Binding {
        path: path.into(),
        required: true,
        empty: EmptyValue::Omit,
    })
}

fn row(primitive: Primitive, props: &[(&str, &str)]) -> SpecNode {
    SpecNode {
        primitive,
        props: props
            .iter()
            .map(|(key, path)| ((*key).into(), binding(path)))
            .collect(),
        children: Vec::new(),
        each: None,
        item: None,
    }
}

fn list(primitive: Primitive, path: &str, item: SpecNode) -> SpecNode {
    SpecNode {
        primitive,
        props: BTreeMap::new(),
        children: Vec::new(),
        each: Some(Binding {
            path: path.into(),
            required: true,
            empty: EmptyValue::Omit,
        }),
        item: Some(Box::new(item)),
    }
}

#[allow(clippy::manual_unwrap_or_default)]
fn seed(id: &str, accepts: &str) -> StoredPresentation {
    let root_primitive = match accepts {
        "itinerary" | "schedule" | "timeline" | "milestones" | "incident-timeline" => {
            Primitive::Timeline
        }
        "map" | "route_map" => Primitive::Map,
        "calendar" | "availability" => Primitive::Calendar,
        "board" => Primitive::Board,
        "entity" => Primitive::Entity,
        "evidence" => Primitive::Evidence,
        "graph" => Primitive::Graph,
        "form" => Primitive::Form,
        "action" => Primitive::Row,
        "transaction" | "table" | "dataframe" => Primitive::Table,
        "document" => Primitive::Section,
        "alert" => Primitive::Alert,
        "conversation" => Primitive::Stack,
        "simulation" => Primitive::Chart,
        "checklist" | "shopping_list" | "reading_list" | "habit_plan" => Primitive::Checklist,
        "decision" | "decision_analysis" | "comparison" | "comparison_table" | "pros_cons"
        | "scorecard" => Primitive::Comparison,
        "budget" | "finance_summary" | "invoice_summary" | "inventory" => Primitive::Table,
        "steps" | "lesson" | "event_plan" | "care_plan" => Primitive::Steps,
        "progress" | "status" => Primitive::Progress,
        "metric" | "benchmark" => Primitive::Metric,
        "recipe" | "recipe_summary" | "lifestyle.recipe" => Primitive::Recipe,
        "research_brief" | "news" => Primitive::Research,
        "preview" | "ui.preview" => Primitive::UiPreview,
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
    let certified = matches!(id, "timeline" | "table" | "recipe" | "research-brief");
    let mut root = SpecNode {
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
    };
    let accepts = match id {
        "timeline" => vec![accepts.into(), "plan.timeline".into()],
        "recipe" => vec![accepts.into(), "recipe.card".into()],
        "research-brief" => vec![accepts.into(), "research.synthesis".into()],
        _ => vec![accepts.into()],
    };
    match id {
        "timeline" => {
            root.props.remove("items");
            root.children.clear();
            root.each = Some(Binding {
                path: "$.items".into(),
                required: true,
                empty: EmptyValue::Omit,
            });
            root.item = Some(Box::new(row(
                Primitive::Section,
                &[
                    ("label", "$.label"),
                    ("detail", "$.detail"),
                    ("status", "$.status"),
                ],
            )));
        }
        "table" => {
            root.props.remove("items");
            root.props
                .insert("columns".into(), required_binding("$.columns"));
            root.children.clear();
            root.each = Some(Binding {
                path: "$.rows".into(),
                required: true,
                empty: EmptyValue::Omit,
            });
            root.item = Some(Box::new(row(Primitive::Row, &[("*", "$")])));
        }
        "recipe" => {
            root.props.remove("items");
            root.props.insert("servings".into(), binding("$.servings"));
            root.props
                .insert("cook_time_minutes".into(), binding("$.cook_time_minutes"));
            root.children = vec![
                list(
                    Primitive::IngredientList,
                    "$.ingredients",
                    row(
                        Primitive::Row,
                        &[
                            ("name", "$.name"),
                            ("amount", "$.amount"),
                            ("unit", "$.unit"),
                        ],
                    ),
                ),
                list(
                    Primitive::StepList,
                    "$.steps",
                    row(
                        Primitive::Section,
                        &[("text", "$.text"), ("timer_seconds", "$.timer_seconds")],
                    ),
                ),
            ];
        }
        "research-brief" => {
            root.props.remove("items");
            root.children = vec![
                list(
                    Primitive::TakeawayList,
                    "$.takeaways",
                    row(
                        Primitive::Section,
                        &[
                            ("text", "$.text"),
                            ("citation_indices", "$.citation_indices"),
                        ],
                    ),
                ),
                list(
                    Primitive::SourceList,
                    "$.sources",
                    row(
                        Primitive::Section,
                        &[
                            ("title", "$.title"),
                            ("url", "$.url"),
                            ("source_name", "$.source_name"),
                        ],
                    ),
                ),
            ];
        }
        _ => {}
    }
    let spec = PresentationSpec {
        schema_version: super::SPEC_SCHEMA_VERSION,
        id: format!("seed.{id}"),
        // Seed definitions are immutable revisions. Bump this whenever the
        // declarative starter shape changes so an older persisted seed cannot
        // collide with the new digest at the same (id, revision) key.
        revision: 4,
        accepts,
        root,
        fallback: FallbackSpec::default(),
        accessibility: AccessibilitySpec {
            summary: Some(Binding {
                path: "$.summary".into(),
                required: false,
                empty: EmptyValue::EmptyText,
            }),
        },
        metadata: BTreeMap::from([
            (String::from("seed"), String::from("true")),
            (String::from("certified"), certified.to_string()),
        ]),
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
            generation: Some("seed-4-live-cards".into()),
        },
        enabled: false,
    }
}

/// The built-in starter pack contains domain experiences plus universal
/// interaction shapes. They are disabled previews and may be activated
/// explicitly.
pub fn built_in_seed_pack() -> Vec<StoredPresentation> {
    EVERYDAY
        .iter()
        .chain(CODING.iter())
        .chain(UNIVERSAL.iter())
        .map(|(id, accepts)| seed(id, accepts))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompileInput, CompiledPresentation, compile};

    #[test]
    fn seed_pack_is_rich_disabled_and_validated_by_host_types() {
        let pack = built_in_seed_pack();
        assert_eq!(pack.len(), 75);
        assert!(pack.iter().all(|record| !record.enabled));
        assert!(
            pack.iter()
                .any(|record| record.spec.accepts == ["coding.diff"])
        );
        assert!(
            pack.iter().all(|record| !record.digest.is_empty()),
            "{:?}",
            pack.iter()
                .filter(|record| record.digest.is_empty())
                .map(|record| &record.spec.id)
                .collect::<Vec<_>>()
        );
        let mut primitives = Vec::new();
        for record in &pack {
            if !primitives.contains(&record.spec.root.primitive) {
                primitives.push(record.spec.root.primitive);
            }
        }
        assert!(primitives.len() >= 6);
    }

    #[test]
    fn seeds_reach_the_newly_added_primitives() {
        let pack = built_in_seed_pack();
        for expected in [Primitive::Recipe, Primitive::Research, Primitive::UiPreview] {
            assert!(
                pack.iter()
                    .any(|record| record.spec.root.primitive == expected),
                "no seed reaches {expected:?}"
            );
        }
    }

    #[test]
    fn new_primitives_compile_rich_with_their_own_payloads() {
        // The enum additions are reachable end to end, not decorative: each
        // compiles through the same generic pipeline as every other seed.
        let cases = [
            (
                "recipe",
                serde_json::json!({
                    "title": "Weeknight dal",
                    "summary": "A fast lentil dal",
                    "ingredients": [{"name":"Lentils","amount":1,"unit":"cup"}],
                    "steps": [{"text":"Rinse lentils"},{"text":"Simmer 20 minutes"}],
                }),
            ),
            (
                "news",
                serde_json::json!({
                    "title": "Grid storage in 2026",
                    "summary": "Three takeaways with sources",
                    "items": ["Costs fell", "Deployment doubled"],
                }),
            ),
            (
                "ui.preview",
                serde_json::json!({
                    "title": "Settings panel",
                    "summary": "Sandboxed preview of the authored markup",
                    "items": ["Light", "Dark"],
                }),
            ),
        ];
        let pack = built_in_seed_pack();
        for (semantic_type, payload) in cases {
            let found = pack.iter().find(|record| {
                record
                    .spec
                    .accepts
                    .iter()
                    .any(|accepted| accepted == semantic_type)
            });
            assert!(found.is_some(), "no seed accepts {semantic_type}");
            let Some(record) = found else { continue };
            let result = compile(
                &record.spec,
                &CompileInput {
                    semantic_type: semantic_type.into(),
                    payload,
                    fallback_text: "Example result".into(),
                },
            );
            assert!(
                matches!(&result, CompiledPresentation::Rich(tree)
                    if tree.root.primitive == record.spec.root.primitive),
                "{semantic_type} did not compile rich through its own primitive: {result:?}"
            );
        }
    }

    #[test]
    fn every_seed_compiles_with_minimal_generic_payload() {
        // Keep the starter pack honest: a newly added seed must be consumable
        // by the same generic compiler used for user and plugin definitions.
        // This intentionally does not assert a domain-specific renderer.
        for record in built_in_seed_pack() {
            assert!(
                !record.spec.accepts.is_empty(),
                "seed accepts one semantic type"
            );
            let semantic_type = &record.spec.accepts[0];
            let result = compile(
                &record.spec,
                &CompileInput {
                    semantic_type: semantic_type.clone(),
                    payload: serde_json::json!({
                        "title": "Example",
                        "subtitle": "A reusable starter",
                        "summary": "A concise example result",
                        "items": [{"label":"One"},{"label":"Two"}],
                        "columns": [{"key":"label","label":"Label"}],
                        "rows": [{"label":"One"}],
                        "ingredients": [{"name":"Oats","amount":1,"unit":"cup"}],
                        "steps": [{"text":"Cook the oats"}],
                        "takeaways": [{"text":"One takeaway"}],
                        "sources": [{"title":"Source","url":"https://example.com"}],
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

    #[test]
    fn certified_live_cards_keep_their_real_collections() {
        let cases = [
            (
                "plan.timeline",
                serde_json::json!({"title":"Trip", "items":[{"label":"Leave","detail":"At eight"},{"label":"Arrive","detail":"At nine"}]}),
                2,
            ),
            (
                "table",
                serde_json::json!({"title":"Options", "columns":[{"key":"name","label":"Name"}], "rows":[{"name":"Train"},{"name":"Bus"}]}),
                2,
            ),
            (
                "recipe.card",
                serde_json::json!({"title":"Dal", "ingredients":[{"name":"Lentils","amount":1}], "steps":[{"text":"Rinse"}]}),
                2,
            ),
            (
                "research.synthesis",
                serde_json::json!({"title":"Brief", "takeaways":[{"text":"Costs fell"}], "sources":[{"title":"Report","url":"https://example.com"}]}),
                2,
            ),
        ];
        let pack = built_in_seed_pack();
        for (semantic_type, payload, child_count) in cases {
            let record = pack
                .iter()
                .find(|record| record.spec.accepts.iter().any(|kind| kind == semantic_type))
                .expect("certified seed");
            assert_eq!(
                record.spec.metadata.get("certified").map(String::as_str),
                Some("true")
            );
            let CompiledPresentation::Rich(tree) = compile(
                &record.spec,
                &CompileInput {
                    semantic_type: semantic_type.into(),
                    payload,
                    fallback_text: "Fallback".into(),
                },
            ) else {
                panic!("{semantic_type} must compile rich");
            };
            assert_eq!(tree.root.children.len(), child_count, "{semantic_type}");
            match semantic_type {
                "plan.timeline" => assert_eq!(tree.root.children[0].props["label"], "Leave"),
                "table" => assert_eq!(tree.root.children[0].props["name"], "Train"),
                "recipe.card" => {
                    assert_eq!(tree.root.children[0].children[0].props["name"], "Lentils")
                }
                "research.synthesis" => assert_eq!(
                    tree.root.children[0].children[0].props["text"],
                    "Costs fell"
                ),
                _ => unreachable!(),
            }
        }
    }
}
