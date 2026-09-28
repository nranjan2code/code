//! Extensible, capability-aware presentation skills.
//!
//! Skills contribute typed data and renderer metadata. They never provide
//! executable desktop code; a surface chooses a trusted renderer or falls
//! back to the item's text representation.
//!
//! Skills are **data**, not code. A plugin-contributed skill ships a
//! `PresentationSkillManifest` (semantic types it provides + per-surface
//! renderer bindings) that is serialized into the `DeliveryJob.skill_registry`
//! field and sent to the isolated worker. The worker validates ```vak fences
//! against that registry without needing filesystem or network access.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PRESENTATION_SKILL_API: &str = "presentation.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationRecipe {
    pub id: String,
    pub version: String,
    /// Capability that owns this recipe when it is contributed by a plugin.
    /// Built-in recipes leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_skill: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub match_signals: Vec<String>,
    pub primary: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    #[serde(default)]
    pub fallback: BTreeMap<String, String>,
    /// Surfaces on which this composition is eligible. This is deliberately
    /// separate from renderer bindings; a recipe never chooses a renderer.
    #[serde(default)]
    pub surfaces: Vec<String>,
    #[serde(default)]
    pub default_recipe: bool,
    #[serde(default)]
    pub requires_typed_output: bool,
    #[serde(default)]
    pub typed_output_types: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationDecision {
    pub recipe_id: String,
    pub recipe_version: String,
    #[serde(default)]
    pub priority: i32,
    pub matched_signals: Vec<String>,
    pub renderer: String,
    pub disposition: DecisionDisposition,
    #[serde(default)]
    pub requires_typed_output: bool,
    #[serde(default)]
    pub typed_output_types: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDisposition {
    Unresolved,
    Native,
    Sandboxed,
    Fallback,
    Rejected,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationPlan {
    pub recipe: Option<PresentationDecision>,
    pub accepted: Vec<StructuredOutput>,
    pub rejected: Vec<PlanDiagnostic>,
    /// Renderer resolution is per structured output, never per recipe. A
    /// document may contain several semantic types with different renderers.
    #[serde(default)]
    pub renderers: Vec<RendererDecision>,
    /// Requirements contributed by the skills that actually own accepted
    /// outputs. Unvalidated or rejected plugin declarations never appear.
    #[serde(default)]
    pub outcome_requirements: Vec<OutcomeRequirementDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererDecision {
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub renderer: String,
    pub disposition: DecisionDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiagnostic {
    pub semantic_type: String,
    pub disposition: DecisionDisposition,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct PresentationPlanner {
    pub skills: SkillRegistry,
    pub recipes: RecipeCatalog,
}

impl PresentationPlan {
    /// Merge requirements from accepted typed outputs into the shared intent
    /// contract. Rejected outputs and plugin authority are never propagated.
    pub fn merge_outcome_requirements(&self, outcome: &mut vak_intent::OutcomeSpec) -> Vec<String> {
        let mut rejected = Vec::new();
        for declaration in &self.outcome_requirements {
            if let Err(reason) = outcome.merge_declared_requirement(
                declaration.id.clone(),
                &declaration.kind,
                declaration.description.clone(),
                &declaration.importance,
                Some("typed-output".into()),
            ) {
                rejected.push(format!("{}: {reason}", declaration.id));
            }
        }
        rejected
    }
}

impl PresentationPlanner {
    pub fn plan(
        &self,
        signals: &[String],
        surface: &str,
        capabilities: &[String],
        candidates: &[StructuredOutput],
    ) -> PresentationPlan {
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        let mut renderers = Vec::new();
        for candidate in candidates {
            match self.skills.validate(candidate, surface, capabilities) {
                Ok(binding) => {
                    accepted.push(candidate.clone());
                    renderers.push(RendererDecision {
                        semantic_type: candidate.semantic_type.clone(),
                        skill_id: candidate.skill_id.clone(),
                        skill_version: candidate.skill_version.clone(),
                        renderer: binding.renderer.clone(),
                        disposition: renderer_disposition(&binding.renderer),
                        diagnostic: None,
                    });
                }
                Err(error) => rejected.push(PlanDiagnostic {
                    semantic_type: candidate.semantic_type.clone(),
                    disposition: DecisionDisposition::Fallback,
                    reason: error.to_string(),
                }),
            }
        }
        let available: Vec<String> = accepted
            .iter()
            .map(|candidate| candidate.semantic_type.clone())
            .collect();
        let mut outcome_requirements = Vec::new();
        for semantic_type in &available {
            for declaration in self.skills.outcome_requirements_for_type(semantic_type) {
                if !outcome_requirements
                    .iter()
                    .any(|existing: &OutcomeRequirementDeclaration| existing.id == declaration.id)
                {
                    outcome_requirements.push(declaration.clone());
                }
            }
        }
        let mut recipe = self.recipes.choose_for_types(signals, surface, &available);
        if let Some(decision) = recipe.as_mut() {
            decision.renderer = renderer_summary(&renderers);
            decision.disposition = renderer_summary_disposition(&renderers);
        }
        PresentationPlan {
            recipe,
            accepted,
            rejected,
            renderers,
            outcome_requirements,
        }
    }
}

fn renderer_disposition(renderer: &str) -> DecisionDisposition {
    if renderer.starts_with("native:") {
        DecisionDisposition::Native
    } else if renderer.starts_with("sandbox:") {
        DecisionDisposition::Sandboxed
    } else {
        DecisionDisposition::Fallback
    }
}

fn renderer_summary(renderers: &[RendererDecision]) -> String {
    let mut ids = renderers
        .iter()
        .map(|decision| decision.renderer.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    match ids.as_slice() {
        [] => "markdown:native".into(),
        [renderer] => (*renderer).into(),
        _ => "mixed".into(),
    }
}

fn renderer_summary_disposition(renderers: &[RendererDecision]) -> DecisionDisposition {
    let Some(first) = renderers.first().map(|decision| decision.disposition) else {
        return DecisionDisposition::Native;
    };
    if renderers
        .iter()
        .all(|decision| decision.disposition == first)
    {
        first
    } else {
        DecisionDisposition::Mixed
    }
}

impl std::fmt::Display for SkillError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidManifest(reason) => write!(f, "invalid manifest: {reason}"),
            Self::UnknownType(kind) => write!(f, "unknown semantic type: {kind}"),
            Self::InvalidPayload(reason) => write!(f, "invalid payload: {reason}"),
            Self::MissingCapability(capability) => write!(f, "missing capability: {capability}"),
        }
    }
}

/// What a set of signals says an answer should be presented as. See
/// [`RecipeCatalog::intended_outputs`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntendedPresentation {
    pub recipe_id: String,
    pub matched_signals: Vec<String>,
    pub primary_types: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeCatalog {
    pub recipes: Vec<PresentationRecipe>,
}

impl RecipeCatalog {
    pub fn register(&mut self, recipe: PresentationRecipe) -> Result<(), SkillError> {
        if recipe.id.trim().is_empty()
            || recipe.version.trim().is_empty()
            || recipe.primary.is_empty()
        {
            return Err(SkillError::InvalidManifest(
                "recipe id, version, and primary are required".into(),
            ));
        }
        self.recipes.push(recipe);
        Ok(())
    }

    pub fn remove_revoked_skills(&mut self, revoked: &std::collections::BTreeSet<String>) {
        self.recipes.retain(|recipe| {
            recipe
                .owner_skill
                .as_ref()
                .is_none_or(|owner| !revoked.contains(owner))
        });
    }

    /// Merge plugin-contributed recipe definitions. Plugins declare
    /// presentation files in their manifest `components.presentation` list.
    /// Each file is a JSON-encoded `PresentationRecipe`. Unknown files are
    /// silently skipped.
    pub fn merge_recipe_files(&mut self, files: &[(String, Vec<u8>)]) {
        for (_path, bytes) in files {
            if let Ok(json) = std::str::from_utf8(bytes)
                && let Ok(recipe) = serde_json::from_str::<PresentationRecipe>(json)
            {
                let _ = self.register(recipe);
            }
        }
    }

    /// The semantic types the best-matching non-default recipe composes for
    /// these signals, whether or not any output of those types exists yet —
    /// the answer to "what should this have been presented as?". `choose_*`
    /// answers a different question (which recipe fits the outputs already
    /// produced), so it cannot be used to notice a missing card.
    pub fn intended_outputs(
        &self,
        signals: &[String],
        surface: &str,
    ) -> Option<IntendedPresentation> {
        self.recipes
            .iter()
            .filter(|recipe| !recipe.default_recipe && !recipe.match_signals.is_empty())
            .filter(|recipe| {
                recipe
                    .match_signals
                    .iter()
                    .all(|signal| signals.iter().any(|candidate| candidate == signal))
            })
            .filter(|recipe| recipe.surfaces.iter().any(|item| item == surface))
            .max_by(|left, right| {
                (left.priority, left.match_signals.len(), left.id.as_str()).cmp(&(
                    right.priority,
                    right.match_signals.len(),
                    right.id.as_str(),
                ))
            })
            .map(|recipe| IntendedPresentation {
                recipe_id: recipe.id.clone(),
                matched_signals: recipe.match_signals.clone(),
                primary_types: recipe.primary.clone(),
            })
    }

    pub fn choose(&self, signals: &[String], surface: &str) -> Option<PresentationDecision> {
        self.choose_for_types(signals, surface, &[])
    }

    pub fn choose_for_types(
        &self,
        signals: &[String],
        surface: &str,
        available_types: &[String],
    ) -> Option<PresentationDecision> {
        self.recipes
            .iter()
            .filter_map(|recipe| {
                let matched_signals: Vec<String> = recipe
                    .match_signals
                    .iter()
                    .filter(|signal| signals.iter().any(|candidate| candidate == *signal))
                    .cloned()
                    .collect();
                if matched_signals.len() != recipe.match_signals.len()
                    && !recipe.match_signals.is_empty()
                {
                    return None;
                }
                let supported_surface = recipe.surfaces.iter().any(|item| item == surface)
                    || (recipe.surfaces.is_empty()
                        && recipe.fallback.keys().any(|item| item == surface));
                if !supported_surface {
                    return None;
                }
                if recipe.requires_typed_output
                    && !recipe.typed_output_types.iter().any(|required| {
                        available_types.iter().any(|candidate| {
                            type_matches(required, std::slice::from_ref(candidate))
                        })
                    })
                {
                    return None;
                }
                if !recipe.default_recipe
                    && !available_types.is_empty()
                    && !recipe
                        .primary
                        .iter()
                        .all(|required| type_matches(required, available_types))
                {
                    return None;
                }
                Some(PresentationDecision {
                    recipe_id: recipe.id.clone(),
                    recipe_version: recipe.version.clone(),
                    priority: recipe.priority,
                    matched_signals,
                    // Renderer selection is performed by PresentationPlanner
                    // from validated skill bindings. Recipes only select
                    // composition intent.
                    renderer: "unresolved".into(),
                    disposition: DecisionDisposition::Unresolved,
                    requires_typed_output: recipe.requires_typed_output,
                    typed_output_types: recipe.typed_output_types.clone(),
                })
            })
            .max_by(|left, right| {
                (
                    left.priority,
                    left.matched_signals.len(),
                    left.recipe_id.as_str(),
                    left.recipe_version.as_str(),
                )
                    .cmp(&(
                        right.priority,
                        right.matched_signals.len(),
                        right.recipe_id.as_str(),
                        right.recipe_version.as_str(),
                    ))
            })
            .filter(|decision| {
                !(decision.recipe_id == "answer.basic"
                    && decision.matched_signals.is_empty()
                    && !signals.is_empty())
            })
    }
}

fn type_matches(required: &str, available: &[String]) -> bool {
    let base = required.split('.').next().unwrap_or(required);
    available.iter().any(|candidate| {
        candidate == required
            || candidate == base
            || (required == "source.card" && candidate == "link.preview")
            || (required == "outcome" && candidate == "research.synthesis")
    })
}

/// Built-in recipes are deliberately small and composable. Domain skills can
/// register more specific recipes; the generic answer recipe remains the
/// deterministic last-resort composition.
pub fn built_in_recipes() -> RecipeCatalog {
    let mut catalog = RecipeCatalog::default();
    for recipe in [
        (
            "answer.basic",
            vec![],
            vec!["outcome"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "answer.research",
            // Citing several sources is not by itself a research synthesis:
            // a one-number answer (such as a current stock price) can cite
            // multiple sources and should stay prose or use a metric card.
            vec!["citations", "multiple_sources", "synthesis"],
            vec!["research.synthesis"],
            vec!["desktop", "terminal"],
        ),
        (
            "research.synthesis",
            vec!["research", "synthesis", "takeaways"],
            vec!["research.synthesis"],
            vec!["desktop", "terminal"],
        ),
        (
            "weather.forecast",
            vec!["temperature", "forecast"],
            vec!["metric"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "travel.itinerary",
            vec!["travel"],
            vec!["itinerary"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "news.synthesis",
            vec!["news", "research"],
            vec!["research.synthesis"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "coding.change_summary",
            vec!["files_changed"],
            vec!["outcome"],
            vec!["desktop", "terminal"],
        ),
        (
            "coding.diff_inspector",
            vec!["diff", "files_changed"],
            vec!["coding.diff"],
            vec!["desktop", "terminal"],
        ),
        (
            "coding.test_report",
            vec!["tests", "pass_fail"],
            vec!["test.report"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "terminal.session",
            vec!["terminal", "command_exec"],
            vec!["terminal.view"],
            vec!["desktop", "terminal"],
        ),
        (
            "data.multi_chart",
            vec!["chart", "telemetry"],
            vec!["chart"],
            vec!["desktop", "terminal"],
        ),
        (
            "data.spreadsheet_grid",
            // A markdown table is structurally unambiguous on its own. The
            // former second signal (`table_data`, keywords like "dataset" or
            // "MRR") made a plain table of index levels or prices — no such
            // word — match nothing, so it was never recognised as a grid.
            vec!["tabular"],
            vec!["data.grid"],
            vec!["desktop", "terminal"],
        ),
        (
            "lifestyle.culinary_recipe",
            vec!["recipe", "ingredients"],
            vec!["recipe.card"],
            vec!["desktop"],
        ),
        (
            "workflow.approval",
            vec!["approval", "action"],
            vec!["approval"],
            vec!["desktop", "telegram"],
        ),
        (
            "artifact.collection",
            vec!["artifact"],
            vec!["artifact.collection"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "ui.preview",
            vec![],
            vec!["ui.preview"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "plan.timeline",
            vec!["plan", "timeline"],
            vec!["plan.timeline"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.map",
            vec!["location", "coordinates", "route"],
            vec!["map"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.calendar",
            vec!["event", "meeting", "availability"],
            vec!["calendar"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.board",
            vec!["board", "kanban", "column"],
            vec!["board"],
            vec!["desktop", "terminal"],
        ),
        (
            "universal.entity",
            vec!["person", "company", "place", "product"],
            vec!["entity"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.evidence",
            vec!["source", "citation", "provenance"],
            vec!["evidence"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.document",
            vec!["document", "pdf", "report"],
            vec!["document"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.graph",
            vec!["graph", "network", "dependency"],
            vec!["graph"],
            vec!["desktop", "terminal"],
        ),
        (
            "universal.form",
            vec!["form", "input", "survey"],
            vec!["form"],
            vec!["desktop", "terminal"],
        ),
        (
            "universal.transaction",
            vec!["invoice", "payment", "booking"],
            vec!["transaction"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.alert",
            vec!["alert", "warning", "incident"],
            vec!["alert"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.conversation",
            vec!["conversation", "thread", "message"],
            vec!["conversation"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "universal.simulation",
            vec!["forecast", "scenario", "what_if"],
            vec!["simulation"],
            vec!["desktop", "terminal"],
        ),
    ] {
        let requires_typed_output = matches!(recipe.0, "coding.test_report" | "ui.preview");
        let typed_output_types: Vec<String> = match recipe.0 {
            "coding.test_report" => vec!["test.report".into()],
            "ui.preview" => vec!["ui.preview".into()],
            _ => Vec::new(),
        };
        let _ = catalog.register(PresentationRecipe {
            id: recipe.0.into(),
            version: "1.0.0".into(),
            owner_skill: None,
            priority: 0,
            match_signals: recipe.1.into_iter().map(String::from).collect(),
            primary: recipe.2.into_iter().map(String::from).collect(),
            optional: Vec::new(),
            fallback: BTreeMap::new(),
            surfaces: recipe.3.into_iter().map(String::from).collect(),
            default_recipe: recipe.0 == "answer.basic",
            requires_typed_output,
            typed_output_types,
        });
    }
    catalog
}

pub fn built_in_skill_registry() -> SkillRegistry {
    let mut registry = SkillRegistry::default();
    let mut renderers = BTreeMap::new();
    renderers.insert(
        "desktop".into(),
        RendererBinding {
            renderer: "native:structured".into(),
            interactive: false,
            requires: vec![],
        },
    );
    renderers.insert(
        "terminal".into(),
        RendererBinding {
            renderer: "builtin:generic".into(),
            interactive: false,
            requires: vec![],
        },
    );
    renderers.insert(
        "telegram".into(),
        RendererBinding {
            renderer: "builtin:compact".into(),
            interactive: false,
            requires: vec![],
        },
    );
    let _ = registry.register(PresentationSkillManifest {
        id: "core".into(),
        version: "1.0.0".into(),
        api: PRESENTATION_SKILL_API.into(),
        provides: vec![
            "link.preview",
            "metric",
            "chart",
            // `chart` synonyms `emit_chart_card` accepts; the client has
            // renderers for all three (PresentationRenderer.tsx), and a
            // type the registry rejects degrades a valid card to fallback
            // text.
            "trend",
            "timeseries",
            "bar_chart",
            // Same gap, same shape: the client (PresentationRenderer.tsx
            // STRUCTURED_RENDERERS) has had renderers for these ten since
            // commits 85408ebc and 1d34cd6b ("support all 62+ outcome
            // render types") but this registry was never updated alongside
            // them, so every one of them was rejected by
            // SkillRegistry::validate() with UnknownType before ever
            // reaching that client code — confirmed by diffing
            // built_in_semantic_types() against STRUCTURED_RENDERERS' keys
            // while building the emit_*_card tool-calling path (2026-09-18,
            // see docs/design/67-presentation-renderer-guide.md Step 0).
            // None of them need a dedicated validate_payload() arm: each
            // shares its shape with an already-registered sibling
            // (metric_chart/comparison_chart/telemetry.chart with chart;
            // telemetry.metric/weather with metric; decision_matrix/
            // criteria_matrix/tradeoff_analysis with table;
            // lifestyle.recipe/lifestyle.culinary_recipe with recipe.card),
            // so the shared, permissive fallthrough in validate_payload
            // already covers them.
            "metric_chart",
            "comparison_chart",
            "telemetry.chart",
            "telemetry.metric",
            "weather",
            "decision_matrix",
            "criteria_matrix",
            "tradeoff_analysis",
            "lifestyle.recipe",
            "lifestyle.culinary_recipe",
            "media.image",
            "media.video",
            "media.audio",
            "research.synthesis",
            "itinerary",
            "coding.diff",
            "test.report",
            "terminal.view",
            "data.grid",
            "recipe.card",
            "ui.preview",
            "plan.timeline",
            "overview",
            "checklist",
            "schedule",
            "comparison",
            "decision",
            "budget",
            "collection",
            "detail",
            "steps",
            "summary",
            "notes",
            "agenda",
            "follow_up",
            "reminder",
            "comparison_table",
            "pros_cons",
            "scorecard",
            "milestones",
            "progress",
            "status",
            "inventory",
            "shopping_list",
            "meal_plan",
            "lesson",
            "reading_list",
            "habit_plan",
            "project_plan",
            "meeting_notes",
            "contact_log",
            "finance_summary",
            "invoice_summary",
            "recipe_summary",
            "travel_options",
            "home_project",
            "care_plan",
            "event_plan",
            "media_list",
            "research_brief",
            "faq",
            "timeline",
            "table",
            "dataframe",
            "recipe",
            "news",
            "coding.deployment",
            "coding.incident",
            "coding.benchmark",
            "coding.architecture",
            "coding.dependencies",
            "coding.release",
            "coding.search",
            "map",
            "route_map",
            "calendar",
            "availability",
            "board",
            "entity",
            "search_results",
            "evidence",
            "decision_analysis",
            "document",
            "graph",
            "form",
            "action",
            "transaction",
            "alert",
            "conversation",
            "progress_dashboard",
            "simulation",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        renderers,
        schema: None,
        outcome_requirements: Vec::new(),
    });
    registry
}

/// Every `semantic_type` the built-in registry will accept, sorted and
/// de-duplicated. This is the compiled-in vocabulary a model may legally
/// emit in a ```vak block; prompt building reads it so the advertised
/// catalogue cannot drift from what the server actually validates.
pub fn built_in_semantic_types() -> Vec<String> {
    let mut types: Vec<String> = built_in_skill_registry()
        .skills
        .values()
        .flat_map(|skill| skill.provides.iter().cloned())
        .collect();
    types.sort();
    types.dedup();
    types
}

pub fn signals_from_text(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let checks = [
        (
            "citations",
            &["http://", "https://", "source:", "sources:"][..],
        ),
        (
            "multiple_sources",
            &["sources:", "according to", "references"][..],
        ),
        (
            "research",
            &[
                "research",
                "key takeaways",
                "findings",
                "research synthesis",
                "verified sources",
            ][..],
        ),
        (
            "synthesis",
            &["synthesis", "key takeaways", "findings", "verified sources"][..],
        ),
        (
            "takeaways",
            &[
                "key takeaways",
                "takeaways",
                "findings",
                "research synthesis",
            ][..],
        ),
        ("temperature", &["temperature", "°c", "°f"][..]),
        ("forecast", &["forecast", "humidity", "wind speed"][..]),
        (
            "travel",
            &["travel", "trip", "itinerary", "flight", "hotel", "vacation"][..],
        ),
        (
            "news",
            &[
                "news",
                "latest",
                "headlines",
                "what happened",
                "current events",
            ][..],
        ),
        (
            "files_changed",
            &["files changed", "modified:", "created:"][..],
        ),
        ("tests", &["tests", "test suite", "test report"][..]),
        ("pass_fail", &["passed", "failed", "failures"][..]),
        (
            "benchmark",
            &[
                "benchmark",
                "req/sec",
                "req/s",
                "p99",
                "throughput",
                "concurrency",
            ][..],
        ),
        (
            "chart",
            &["chart", "plot", "graph", "trend", "time series"][..],
        ),
        (
            "telemetry",
            &[
                "telemetry",
                "metrics",
                "kpi",
                "latency",
                "req/sec",
                "req/s",
                "p99",
                "throughput",
            ][..],
        ),
        (
            "table_data",
            &[
                "table",
                "dataset",
                "mrr",
                "active orgs",
                "revenue breakdown",
            ][..],
        ),
        (
            "recipe",
            &["recipe", "servings", "cook time", "prep time", "baste"][..],
        ),
        (
            "ingredients",
            &[
                "ingredients",
                "tbsp",
                "tsp",
                "fillet",
                "tablespoon",
                "teaspoon",
                "cups",
            ][..],
        ),
        ("artifact", &["artifact", "download", "generated file"][..]),
        ("approval", &["approval", "approve", "permission"][..]),
        ("action", &["allow once", "deny", "run this"][..]),
        (
            "terminal",
            &["docker ps", "kubectl", "exit 0", "exit 1", "command line"][..],
        ),
        (
            "command_exec",
            &["$ ", "user@", "exit 0", "exit 1", "stdout:", "stderr:"][..],
        ),
        (
            "docker",
            &["docker", "container", "microservices", "ports"][..],
        ),
    ];
    let mut signals: Vec<String> = checks
        .iter()
        .filter_map(|(signal, needles)| {
            needles
                .iter()
                .any(|needle| signal_text_hit(&lower, needle))
                .then_some((*signal).into())
        })
        .collect();
    signals.extend(structural_signals(text).into_iter().map(String::from));
    signals
}

/// Signals about the *structure* of a piece of Markdown, read from the parsed
/// document rather than from characters in the raw text: a table is a table
/// block, a diff is a code block in diff form. (These used to be substring
/// hits on `| ---`, a fenced ```` ```diff ```` marker and a literal tab, which
/// miss an indented table, match a table inside a code sample, and cannot tell
/// a real tab-separated block from one stray tab.)
fn structural_signals(text: &str) -> Vec<&'static str> {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
    let mut signals = Vec::new();
    let mut code: Option<(bool, String)> = None; // (declared diff/patch, body)
    for event in Parser::new_ext(text, Options::ENABLE_TABLES) {
        match event {
            Event::Start(Tag::Table(_)) => signals.push("tabular"),
            Event::Start(Tag::CodeBlock(kind)) => {
                let declared_diff = matches!(&kind, CodeBlockKind::Fenced(info)
                    if matches!(info.split_whitespace().next(), Some("diff" | "patch")));
                code = Some((declared_diff, String::new()));
            }
            Event::Text(chunk) => {
                if let Some((_, body)) = code.as_mut() {
                    body.push_str(&chunk);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((declared_diff, body)) = code.take() {
                    let first = body.lines().next().unwrap_or_default();
                    if declared_diff || first.starts_with("diff --git") || first.starts_with("@@ ")
                    {
                        signals.push("diff");
                    }
                    if is_tab_separated(&body) {
                        signals.push("tabular");
                    }
                }
            }
            _ => {}
        }
    }
    if is_tab_separated(text) {
        signals.push("tabular");
    }
    // An unfenced unified diff is still a diff: a `diff --git` header line or a
    // hunk header (`@@ -1,2 +1,3 @@`) at the start of a line.
    if text
        .lines()
        .any(|line| line.starts_with("diff --git ") || is_hunk_header(line))
    {
        signals.push("diff");
    }
    signals
}

/// `@@ -a[,b] +c[,d] @@` at the start of a line.
fn is_hunk_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("@@ -") else {
        return false;
    };
    let mut parts = rest.splitn(2, " +");
    let (old, new) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    let range = |text: &str| {
        let text = text.split(" @@").next().unwrap_or_default();
        !text.is_empty()
            && text
                .split(',')
                .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    };
    range(old) && range(new) && line.contains(" @@")
}

/// Two or more consecutive lines that each split into the same number (at
/// least two) of tab-separated fields — a pasted spreadsheet range — as
/// opposed to a stray tab in prose.
fn is_tab_separated(text: &str) -> bool {
    let mut previous: Option<usize> = None;
    let mut run = 0;
    for line in text.lines() {
        let fields = line.split('\t').count();
        if fields >= 2 && previous.is_none_or(|p| p == fields) {
            run += 1;
            previous = Some(fields);
            if run >= 2 {
                return true;
            }
        } else {
            run = usize::from(fields >= 2);
            previous = (fields >= 2).then_some(fields);
        }
    }
    false
}

/// Match standalone lexical terms without allowing incidental substrings in
/// ordinary prose. Compound phrases retain substring matching because their
/// spaces provide the boundary. Needles that themselves contain non-word
/// boundary characters (e.g. `https://`, `source:`, `°c`) fall back to
/// substring matching, since tokenization would strip those characters and
/// make a match impossible. Presentation signals are hints only; typed
/// result provenance remains the authority for specialized recipes.
fn signal_text_hit(lower: &str, needle: &str) -> bool {
    if needle.chars().any(char::is_whitespace)
        || needle.chars().any(|c| !c.is_alphanumeric() && c != '_')
    {
        return lower.contains(needle);
    }
    lower
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .any(|token| token == needle)
}

#[derive(Debug, Clone, Default)]
pub struct SignalContext<'a> {
    pub text: &'a str,
    pub tool_name: Option<&'a str>,
    pub tool_input: Option<&'a serde_json::Value>,
    pub tool_output: Option<&'a str>,
    pub is_error: bool,
    /// Domain names the calling capability declares it serves (e.g. `"web"`,
    /// `"live-data"`), as recorded by `vak_core::capability::domain::Domain`.
    /// Drives presentation signals that depend on external retrieval; a
    /// tool's name or identity is never matched for this.
    pub domains: &'a [&'a str],
}

pub fn signals_from_context(ctx: &SignalContext<'_>) -> Vec<String> {
    let mut signals = signals_from_text(ctx.text);
    if let Some(tool) = ctx.tool_name {
        match tool {
            "bash" => {
                signals.push("terminal".into());
                signals.push("command_exec".into());
                if let Some(input) = ctx.tool_input {
                    let cmd = input
                        .get("command")
                        .or_else(|| input.get("cmd"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let lower = cmd.to_ascii_lowercase();
                    if lower.contains("test")
                        || lower.contains("pytest")
                        || lower.contains("cargo test")
                        || lower.contains("npm test")
                    {
                        signals.push("tests".into());
                        signals.push("pass_fail".into());
                    }
                    if lower.contains("docker") || lower.contains("kubectl") {
                        signals.push("docker".into());
                        signals.push("terminal".into());
                    }
                    if lower.contains("diff") || lower.contains("git diff") {
                        signals.push("diff".into());
                        signals.push("files_changed".into());
                    }
                }
            }
            "edit" | "write" | "apply_patch" => {
                signals.push("diff".into());
                signals.push("files_changed".into());
            }
            _ => {}
        }
    }
    // Retrieval-flavoured presentation signals come from what the calling
    // capability declares it serves, never from the tool's name: a vendor
    // renamed or replaced still serves `web`/`live-data`, and an
    // undeclared capability contributes nothing here rather than being
    // guessed at.
    if ctx
        .domains
        .iter()
        .any(|domain| matches!(*domain, "web" | "live-data"))
    {
        signals.push("citations".into());
        signals.push("multiple_sources".into());
        signals.push("research".into());
        signals.push("synthesis".into());
        signals.push("takeaways".into());
    }
    if let Some(output) = ctx.tool_output {
        let lower = output.to_ascii_lowercase();
        if lower.contains("test result:")
            || lower.contains("tests passed")
            || lower.contains("failures:")
        {
            signals.push("tests".into());
            signals.push("pass_fail".into());
        }
        if lower.contains("diff --git") || lower.contains("@@ ") {
            signals.push("diff".into());
            signals.push("files_changed".into());
        }
    }
    signals.sort();
    signals.dedup();
    signals
}

pub fn link_previews_from_text(text: &str) -> Vec<StructuredOutput> {
    let mut seen = std::collections::BTreeSet::new();
    text.split_whitespace()
        .filter_map(|token| {
            let url = token.trim_matches(|character: char| "()[]{}<>.,;\"'".contains(character));
            (url.starts_with("https://") || url.starts_with("http://")).then(|| url.to_string())
        })
        .filter(|url| seen.insert(url.clone()))
        .take(12)
        .map(|url| StructuredOutput {
            semantic_type: "link.preview".into(),
            schema_version: crate::PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: serde_json::json!({"url": url, "title": "Open source link"}),
        })
        .collect()
}

/// Extract explicitly typed `vak` fragments from model/tool text. This is
/// intentionally domain-neutral: the registry, not this parser, decides what
/// semantic types exist and whether a surface may render them.
///
/// A tool result is usually not Markdown at all — it's just the JSON its
/// implementation returned. Requiring a ```vak fence around it would force
/// every tool wrapper to know about Markdown just to be found here, which is
/// its own kind of special-casing. So the *whole* text is tried as one bare
/// `{"semantic_type": ..., "payload": ...}` envelope first — the identical
/// two-field contract a fenced fragment uses, just without the fence. Any
/// tool, or any future one, opts in the same way a model does: declare what
/// it produced. Nothing here inspects a tool's name or its payload's field
/// names to guess a type.
pub fn structured_outputs_from_text(text: &str) -> Vec<StructuredOutput> {
    structured_outputs_from_text_with(text, &built_in_skill_registry())
}

/// Locate byte spans `(start, end)` of candidate JSON objects containing
/// `"semantic_type"` within arbitrary text.
pub(crate) fn find_semantic_json_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let marker = b"\"semantic_type\"";
    let marker_len = marker.len();

    let mut search_start = 0;
    while search_start + marker_len <= len {
        let Some(rel_pos) = text[search_start..].find("\"semantic_type\"") else {
            break;
        };
        let marker_pos = search_start + rel_pos;

        let mut candidate = None;
        for i in (0..=marker_pos).rev() {
            if bytes[i] == b'{' {
                let mut depth = 0;
                let mut in_str = false;
                let mut escape = false;
                let mut closed_at = None;
                #[allow(clippy::needless_range_loop)]
                for j in i..len {
                    let b = bytes[j];
                    if escape {
                        escape = false;
                        continue;
                    }
                    if b == b'\\' && in_str {
                        escape = true;
                        continue;
                    }
                    if b == b'"' {
                        in_str = !in_str;
                        continue;
                    }
                    if !in_str {
                        if b == b'{' {
                            depth += 1;
                        } else if b == b'}' {
                            depth -= 1;
                            if depth == 0 {
                                closed_at = Some(j);
                                break;
                            }
                        }
                    }
                }
                if let Some(end_idx) = closed_at
                    && end_idx >= marker_pos
                {
                    candidate = Some((i, end_idx + 1));
                }
            }
        }

        if let Some((start, end)) = candidate {
            if !spans.iter().any(|(s, e)| *s == start && *e == end) {
                spans.push((start, end));
            }
            search_start = end;
        } else {
            search_start = marker_pos + marker_len;
        }
    }
    spans
}

/// Like [`structured_outputs_from_text`] but validates against a
/// plugin-extended skill registry. The Core passes its merged registry
/// (builtins + plugin skills) so that plugin-declared semantic types are
/// recognized in model/tool text.
pub fn structured_outputs_from_text_with(
    text: &str,
    skills: &SkillRegistry,
) -> Vec<StructuredOutput> {
    if let Ok(output) = parse_fragment_with(text.trim(), skills) {
        return vec![output];
    }
    let mut outputs = Vec::new();
    let mut remainder = text;
    while let Some(start) = remainder.find("```vak") {
        let after = &remainder[start + 6..];
        let body = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        let Some(end) = body.find("```") else { break };
        if let Ok(output) = parse_fragment_with(&body[..end], skills)
            && !outputs
                .iter()
                .any(|existing: &StructuredOutput| existing == &output)
        {
            outputs.push(output);
        }
        remainder = &body[end + 3..];
    }
    for (start, end) in find_semantic_json_spans(text) {
        let candidate = &text[start..end];
        if let Ok(output) = parse_fragment_with(candidate, skills)
            && !outputs
                .iter()
                .any(|existing: &StructuredOutput| existing == &output)
        {
            outputs.push(output);
        }
    }
    outputs
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationSkillManifest {
    pub id: String,
    pub version: String,
    pub api: String,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub renderers: BTreeMap<String, RendererBinding>,
    /// Optional JSON Schema for payload validation. When present, payloads
    /// are validated against it. When absent (plugin skills that don't
    /// declare one), the payload shape is accepted as-is — the plugin's own
    /// renderer owns correctness of its data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    /// Declarative requirements that a consumer may merge into its shared
    /// outcome contract when this skill owns the produced semantic type.
    #[serde(default)]
    pub outcome_requirements: Vec<OutcomeRequirementDeclaration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeRequirementDeclaration {
    pub id: String,
    pub kind: String,
    pub description: String,
    #[serde(default)]
    pub importance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererBinding {
    pub renderer: String,
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuredOutput {
    pub semantic_type: String,
    pub schema_version: u16,
    pub skill_id: String,
    pub skill_version: String,
    pub payload: Value,
}

/// The only model-authored rich block envelope. The host supplies versions
/// and renderer identity; models supply data, never executable UI or receipts.
///
/// Uses the built-in skill registry for validation. For plugin-contributed
/// semantic types, use [`parse_fragment_with`].
pub fn parse_fragment(source: &str) -> Result<StructuredOutput, SkillError> {
    parse_fragment_with(source, &built_in_skill_registry())
}

/// Like [`parse_fragment`] but validates against an arbitrary (possibly
/// plugin-extended) skill registry. The worker receives the merged registry
/// through the `DeliveryJob.skill_registry` field.
pub fn parse_fragment_with(
    source: &str,
    skills: &SkillRegistry,
) -> Result<StructuredOutput, SkillError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fragment {
        semantic_type: String,
        payload: Value,
    }
    let fragment: Fragment = match serde_json::from_str(source) {
        Ok(f) => f,
        Err(err) => {
            if let Ok(value) = toml::from_str::<serde_json::Value>(source) {
                if let (Some(st), Some(p)) = (value.get("semantic_type"), value.get("payload")) {
                    Fragment {
                        semantic_type: st.as_str().unwrap_or("metric").to_string(),
                        payload: p.clone(),
                    }
                } else {
                    return Err(SkillError::InvalidPayload(err.to_string()));
                }
            } else {
                return Err(SkillError::InvalidPayload(err.to_string()));
            }
        }
    };
    // Resolve the owning skill by the semantic type it declares, rather than
    // assuming "core" — plugin-contributed types are registered under their
    // own skill id and must validate against their own manifest.
    let (skill_id, skill_version) = skills
        .find_by_type(&fragment.semantic_type)
        .map(|(id, version)| (id.to_string(), version.to_string()))
        .unwrap_or_else(|| ("core".to_string(), "1.0.0".to_string()));
    let output = StructuredOutput {
        semantic_type: fragment.semantic_type,
        payload: fragment.payload,
        schema_version: crate::PRESENTATION_SCHEMA_VERSION,
        skill_id,
        skill_version,
    };
    skills.validate(&output, "desktop", &[])?;
    Ok(output)
}

/// Deterministic text projection of validated data. It preserves every supplied
/// field in an inspectable data appendix rather than trusting a second summary.
pub fn structured_markdown(output: &StructuredOutput) -> String {
    let p = &output.payload;
    let title = p["title"].as_str().unwrap_or(&output.semantic_type);
    let mut lines = vec![format!("### {title}")];
    match output.semantic_type.as_str() {
        "metric" => {
            if p.get("label").is_some() && p.get("value").is_some() {
                lines.push(format!(
                    "{}: {} {}",
                    p["label"].as_str().unwrap_or_default(),
                    p["value"]
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| p["value"].to_string()),
                    p["unit"].as_str().unwrap_or_default()
                ));
            } else if let Some(obj) = p.as_object() {
                for (k, v) in obj {
                    if k == "title" {
                        continue;
                    }
                    let val_str = v
                        .as_str()
                        .map(String::from)
                        .unwrap_or_else(|| v.to_string());
                    lines.push(format!("{k}: {val_str}"));
                }
            }
        }
        "link.preview" => lines.push(format!(
            "{}\n{}",
            title,
            p["url"].as_str().unwrap_or_default()
        )),
        "chart" => lines.push(p["accessible_summary"].as_str().unwrap_or_default().into()),
        "terminal.view" => lines.push(format!(
            "Command: {}\nExit: {}",
            p["command"].as_str().unwrap_or("not supplied"),
            p.get("exit_code")
                .map(Value::to_string)
                .unwrap_or_else(|| "not supplied".into())
        )),
        "plan.timeline" => {
            if let Some(items) = p["items"].as_array() {
                for item in items.iter().take(100) {
                    let label = item["label"].as_str().unwrap_or("Step");
                    let label = match item["time"].as_str() {
                        Some(time) => format!("{label} ({time})"),
                        None => label.to_string(),
                    };
                    let detail = item["detail"].as_str().unwrap_or_default();
                    if detail.is_empty() {
                        lines.push(format!("- {label}"));
                    } else {
                        lines.push(format!("- {label}: {detail}"));
                    }
                    for option in item["options"].as_array().into_iter().flatten().take(20) {
                        let name = option["label"].as_str().unwrap_or("Option");
                        let facts = option["facts"]
                            .as_array()
                            .map(|facts| {
                                facts
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default();
                        let text = [
                            option["detail"].as_str().unwrap_or_default(),
                            facts.as_str(),
                        ]
                        .into_iter()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                        if text.is_empty() {
                            lines.push(format!("  - Option: {name}"));
                        } else {
                            lines.push(format!("  - Option: {name}: {text}"));
                        }
                    }
                }
            }
        }
        "ui.preview" => lines.push(format!(
            "Preview: {}\nFile: {}",
            title,
            p["artifact_path"].as_str().unwrap_or_default()
        )),
        _ => {}
    }
    let json = serde_json::to_string_pretty(p).unwrap_or_else(|_| p.to_string());
    let fence = "`".repeat(
        json.split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            .max(2)
            + 1,
    );
    lines.push(format!("{fence}json\n{json}\n{fence}"));
    lines.join("\n\n")
}

/// Replace validated rich fences in-place with their deterministic text
/// projection. Ordinary Markdown and invalid/incomplete fences stay exact.
///
/// Uses the built-in skill registry. For plugin-contributed semantic types,
/// use [`project_structured_fences_with`].
pub fn project_structured_fences(source: &str) -> String {
    project_structured_fences_with(source, &built_in_skill_registry())
}

/// Like [`project_structured_fences`] but validates against an arbitrary
/// (possibly plugin-extended) skill registry. Called by `render_content`
/// with the `DeliveryJob.skill_registry` field (the Core's merged registry)
/// when available, falling back to builtins in the worker.
pub fn project_structured_fences_with(source: &str, skills: &SkillRegistry) -> String {
    use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};
    let mut replacements = Vec::new();
    let mut pending: Option<(usize, String)> = None;
    for (event, range) in Parser::new(source).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                if language.as_ref() == "vak" =>
            {
                pending = Some((range.start, String::new()));
            }
            Event::Text(text) => {
                if let Some((_, content)) = &mut pending {
                    content.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, content)) = pending.take()
                    && let Ok(output) = parse_fragment_with(&content, skills)
                {
                    replacements.push((start..range.end, structured_markdown(&output)));
                }
            }
            _ => {}
        }
    }
    let mut result = source.to_string();
    for (range, replacement) in replacements.into_iter().rev() {
        result.replace_range(range, &replacement);
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub url: String,
    pub canonical_url: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_url: Option<String>,
    pub icon_url: Option<String>,
    pub media_type: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub label: String,
    pub value: Value,
    pub unit: Option<String>,
    pub change: Option<f64>,
    pub trend: Option<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaOutput {
    pub source: String,
    pub media_type: String,
    pub alt: String,
    pub thumbnail: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartOutput {
    pub chart_type: String,
    pub title: Option<String>,
    pub x_label: Option<String>,
    pub y_label: Option<String>,
    pub series: Vec<ChartSeries>,
    pub accessible_summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSeries {
    pub name: String,
    pub points: Vec<ChartPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartPoint {
    pub x: Value,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillError {
    InvalidManifest(String),
    UnknownType(String),
    InvalidPayload(String),
    MissingCapability(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRegistry {
    pub skills: BTreeMap<String, PresentationSkillManifest>,
}

impl SkillRegistry {
    pub fn register(&mut self, manifest: PresentationSkillManifest) -> Result<(), SkillError> {
        if manifest.id.trim().is_empty() || manifest.version.trim().is_empty() {
            return Err(SkillError::InvalidManifest(
                "id and version are required".into(),
            ));
        }
        if manifest.api != PRESENTATION_SKILL_API {
            return Err(SkillError::InvalidManifest(format!(
                "unsupported API {}",
                manifest.api
            )));
        }
        if manifest.provides.iter().any(|kind| kind.trim().is_empty()) {
            return Err(SkillError::InvalidManifest("empty semantic type".into()));
        }
        if manifest.outcome_requirements.iter().any(|requirement| {
            requirement.id.trim().is_empty()
                || requirement.kind.trim().is_empty()
                || requirement.description.trim().is_empty()
        }) {
            return Err(SkillError::InvalidManifest(
                "outcome requirements need id, kind, and description".into(),
            ));
        }
        if self.skills.iter().any(|(id, existing)| {
            id != &manifest.id
                && manifest
                    .provides
                    .iter()
                    .any(|kind| existing.provides.iter().any(|owned| owned == kind))
        }) {
            return Err(SkillError::InvalidManifest(
                "semantic type is already owned by another skill".into(),
            ));
        }
        self.skills.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    /// Return plugin-declared requirements for a validated semantic type.
    /// Declarations are data only; the runtime still evaluates them and
    /// applies its own permission and evidence rules.
    pub fn outcome_requirements_for_type(
        &self,
        semantic_type: &str,
    ) -> Vec<&OutcomeRequirementDeclaration> {
        self.skills
            .values()
            .filter(|skill| skill.provides.iter().any(|kind| kind == semantic_type))
            .flat_map(|skill| skill.outcome_requirements.iter())
            .collect()
    }

    /// Merge plugin-contributed skill manifests into this registry.
    /// Plugins declare presentation files in their manifest `components.presentation`
    /// list. Each file is a JSON-encoded `PresentationSkillManifest` or
    /// `PresentationRecipe`. Unknown files are silently skipped (a plugin
    /// manifest can declare files for other systems).
    pub fn merge_plugin_files(&mut self, files: &[(String, Vec<u8>)]) {
        for (_path, bytes) in files {
            if let Ok(json) = std::str::from_utf8(bytes)
                && let Ok(manifest) = serde_json::from_str::<PresentationSkillManifest>(json)
            {
                let _ = self.register(manifest);
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<&PresentationSkillManifest> {
        self.skills.get(id)
    }

    /// Find the skill that declares a given semantic type. Returns the skill's
    /// `(id, version)` so `parse_fragment_with` can attribute an output to
    /// its real owner rather than the hardcoded `"core"` fallback.
    pub fn find_by_type(&self, semantic_type: &str) -> Option<(&String, &String)> {
        self.skills
            .values()
            .find(|skill| skill.provides.iter().any(|kind| kind == semantic_type))
            .map(|skill| (&skill.id, &skill.version))
    }

    pub fn validate(
        &self,
        output: &StructuredOutput,
        surface: &str,
        capabilities: &[String],
    ) -> Result<&RendererBinding, SkillError> {
        let skill = self
            .get(&output.skill_id)
            .ok_or_else(|| SkillError::UnknownType(output.skill_id.clone()))?;
        if output.schema_version != crate::PRESENTATION_SCHEMA_VERSION
            || output.skill_version != skill.version
        {
            return Err(SkillError::InvalidPayload(
                "unsupported structured output version".into(),
            ));
        }
        if !skill
            .provides
            .iter()
            .any(|kind| kind == &output.semantic_type)
        {
            return Err(SkillError::UnknownType(output.semantic_type.clone()));
        }
        let renderer = skill
            .renderers
            .get(surface)
            .ok_or_else(|| SkillError::MissingCapability(surface.into()))?;
        for required in &renderer.requires {
            if !capabilities.iter().any(|capability| capability == required) {
                return Err(SkillError::MissingCapability(required.clone()));
            }
        }
        validate_payload(
            &output.semantic_type,
            &output.payload,
            skill.schema.as_ref(),
        )?;
        Ok(renderer)
    }
}

/// Validate a payload value against a minimal JSON Schema subset
/// (`type`, `required`). This avoids pulling in a schema crate; plugin
/// manifests that need richer validation declare their own renderer.
fn validate_against_schema(payload: &Value, schema: &Value) -> Result<(), SkillError> {
    let Some(schema_obj) = schema.as_object() else {
        return Err(SkillError::InvalidPayload(
            "plugin schema must be a JSON object".into(),
        ));
    };
    if let Some(expected) = schema_obj.get("type").and_then(|t| t.as_str()) {
        let matches = match expected {
            "object" => payload.is_object(),
            "array" => payload.is_array(),
            "string" => payload.is_string(),
            "number" => payload.is_number(),
            "boolean" => payload.is_boolean(),
            "null" => payload.is_null(),
            _ => true,
        };
        if !matches {
            return Err(SkillError::InvalidPayload(format!(
                "payload is not a {expected}"
            )));
        }
    }
    if let Some(required) = schema_obj.get("required").and_then(|r| r.as_array()) {
        let Some(obj) = payload.as_object() else {
            return Err(SkillError::InvalidPayload(
                "payload must be an object for required check".into(),
            ));
        };
        for key in required.iter().flat_map(|k| k.as_str()) {
            if !obj.contains_key(key) {
                return Err(SkillError::InvalidPayload(format!(
                    "missing required field: {key}"
                )));
            }
        }
    }
    Ok(())
}

fn validate_payload(
    semantic_type: &str,
    payload: &Value,
    schema: Option<&Value>,
) -> Result<(), SkillError> {
    if payload.to_string().len() > 1_000_000 {
        return Err(SkillError::InvalidPayload("payload exceeds 1 MB".into()));
    }
    let object = payload.as_object();
    if let Some(object) = object {
        validate_context(object)?;
    }
    // Plugin-declared schema takes precedence: if the skill ships a JSON
    // Schema, validate against it. If not, fall through to the built-in
    // type-by-type validators for known core types.
    if let Some(schema) = schema {
        return validate_against_schema(payload, schema);
    }
    let object =
        object.ok_or_else(|| SkillError::InvalidPayload("payload must be an object".into()))?;
    let strings = |value: &Value, keys: &[&str]| keys.iter().all(|key| value[*key].is_string());
    fn array<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
        value[key].as_array().filter(|items| items.len() <= 10_000)
    }
    let scalar = |value: &Value| {
        value.is_null() || value.is_string() || value.is_number() || value.is_boolean()
    };
    let nonnegative = |value: &Value| value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0);
    let valid = match semantic_type {
        "link.preview" => strings(payload, &["url", "title"]),
        "metric" => {
            (strings(payload, &["label"])
                && object.contains_key("value")
                && scalar(&payload["value"]))
                || (object.len() >= 2 && object.values().any(scalar))
        }
        "media.image" | "media.video" | "media.audio" => {
            strings(payload, &["source", "media_type", "alt"])
        }
        "chart" => {
            strings(payload, &["accessible_summary"])
                && matches!(
                    payload["chart_type"].as_str(),
                    Some("line" | "bar" | "area")
                )
                && array(payload, "series").is_some_and(|series| {
                    series.iter().all(|s| {
                        strings(s, &["name"])
                            && array(s, "points").is_some_and(|points| {
                                points.iter().all(|p| {
                                    (p["x"].is_number() || p["x"].is_string())
                                        && p.get("y").is_some_and(|y| {
                                            y.is_null() || y.as_f64().is_some_and(f64::is_finite)
                                        })
                                })
                            })
                    })
                })
        }
        "research.synthesis" => array(payload, "sources").is_some_and(|sources| {
            sources.iter().all(|s| strings(s, &["title", "url"]))
                && array(payload, "takeaways").is_some_and(|items| {
                    items.iter().all(|item| {
                        strings(item, &["text"])
                            && item.get("citation_indices").is_some_and(|indices| {
                                indices.as_array().is_some_and(|indices| {
                                    !indices.is_empty()
                                        && indices.iter().all(|i| {
                                            i.as_u64()
                                                .is_some_and(|i| i > 0 && i <= sources.len() as u64)
                                        })
                                })
                            })
                    })
                })
        }),
        "itinerary" => array(payload, "items").is_some_and(|items| {
            items.iter().all(|item| {
                strings(item, &["title"]) && item.get("detail").is_none_or(Value::is_string)
            })
        }),
        "coding.diff" => array(payload, "files").is_some_and(|files| {
            files.iter().all(|file| {
                strings(file, &["filename", "hunks"])
                    && file["additions"].is_u64()
                    && file["deletions"].is_u64()
            })
        }),
        "test.report" => array(payload, "tests").is_some_and(|tests| {
            tests.iter().all(|test| {
                strings(test, &["name"])
                    && matches!(
                        test["status"].as_str(),
                        Some("passed" | "failed" | "skipped")
                    )
                    && test.get("duration_ms").is_none_or(&nonnegative)
            }) && ["total", "passed", "failed", "skipped"].iter().all(|key| {
                payload.get(*key).is_none_or(|count| {
                    count.as_u64().is_some_and(|count| {
                        count
                            == if *key == "total" {
                                tests.len()
                            } else {
                                tests
                                    .iter()
                                    .filter(|test| test["status"].as_str() == Some(*key))
                                    .count()
                            } as u64
                    })
                })
            })
        }),
        "terminal.view" => {
            strings(payload, &["output"])
                && payload.get("command").is_none_or(Value::is_string)
                && payload.get("exit_code").is_none_or(Value::is_i64)
                && payload.get("duration_ms").is_none_or(&nonnegative)
        }
        "data.grid" => array(payload, "columns").is_some_and(|columns| {
            let keys: std::collections::BTreeSet<_> =
                columns.iter().filter_map(|c| c["key"].as_str()).collect();
            keys.len() == columns.len()
                && columns.iter().all(|c| strings(c, &["key", "label"]))
                && array(payload, "rows").is_some_and(|rows| {
                    rows.iter().all(|row| {
                        row.as_object().is_some_and(|row| {
                            row.iter()
                                .all(|(key, value)| keys.contains(key.as_str()) && scalar(value))
                        })
                    })
                })
        }),
        "recipe.card" => {
            strings(payload, &["title"])
                && payload
                    .get("servings")
                    .is_none_or(|v| v.as_u64().is_some_and(|v| v > 0 && v <= 10_000))
                && array(payload, "ingredients").is_some_and(|items| {
                    items.iter().all(|item| {
                        item.is_string()
                            || (strings(item, &["name"])
                                && item.get("amount").is_none_or(&nonnegative)
                                && item.get("unit").is_none_or(Value::is_string))
                    })
                })
                && array(payload, "steps").is_some_and(|steps| {
                    steps.iter().all(|step| {
                        step.is_string()
                            || (strings(step, &["text"])
                                && step.get("timer_seconds").is_none_or(|v| {
                                    v.as_u64().is_some_and(|v| v > 0 && v <= 86_400)
                                }))
                    })
                })
        }
        // The list renderer only reads an array under one of these keys, and
        // only labels an item from one of these fields. A payload that keys
        // its options as, say, `options: [{name, score, pros, cons}]` is not
        // an error anywhere — it simply renders as a raw JSON dump, which is
        // the failure this validator exists to make loud.
        "decision" | "decision_analysis" => [
            "choices",
            "items",
            "steps",
            "milestones",
            "slots",
            "agenda",
            "tasks",
            "questions",
            "qa",
            "entries",
        ]
        .iter()
        .find_map(|key| array(payload, key))
        .is_some_and(|items| {
            items.iter().all(|item| {
                item.is_string()
                    || item.is_number()
                    || [
                        "label", "title", "name", "question", "task", "text", "choice", "activity",
                    ]
                    .iter()
                    .any(|field| item[*field].is_string())
            })
        }),
        "plan.timeline" => {
            strings(payload, &["title"])
                && array(payload, "items").is_some_and(|items| {
                    items.iter().all(|item| {
                        strings(item, &["label"])
                            && item.get("detail").is_none_or(Value::is_string)
                            && item.get("status").is_none_or(Value::is_string)
                            && item.get("time").is_none_or(Value::is_string)
                            && item.get("options").is_none_or(|options| {
                                options.as_array().is_some_and(|options| {
                                    options.iter().all(|option| {
                                        strings(option, &["label"])
                                            && option.get("detail").is_none_or(Value::is_string)
                                            && option.get("facts").is_none_or(|facts| {
                                                facts.as_array().is_some_and(|facts| {
                                                    facts.iter().all(Value::is_string)
                                                })
                                            })
                                    })
                                })
                            })
                    })
                })
        }
        _ => return Ok(()),
    };
    if valid {
        Ok(())
    } else {
        Err(SkillError::InvalidPayload(format!(
            "invalid {semantic_type} payload shape or values"
        )))
    }
}

/// Validate the optional, semantic context shared by typed outputs. The
/// context is data-driven so temporal, coding, research, and data skills can
/// use the same evidence boundary without the renderer inferring meaning from
/// incidental payload field names.
fn validate_context(object: &serde_json::Map<String, Value>) -> Result<(), SkillError> {
    let Some(context) = object.get("context") else {
        return Ok(());
    };
    let Some(context) = context.as_object() else {
        return Err(SkillError::InvalidPayload(
            "context must be an object".into(),
        ));
    };
    for key in ["domain", "as_of", "comparison_basis"] {
        if context.get(key).is_some_and(|value| !value.is_string()) {
            return Err(SkillError::InvalidPayload(format!(
                "context.{key} must be a string"
            )));
        }
    }
    if let Some(period) = context.get("period") {
        let Some(period) = period.as_object() else {
            return Err(SkillError::InvalidPayload(
                "context.period must be an object".into(),
            ));
        };
        for key in ["start", "end", "timezone"] {
            if !period.get(key).is_some_and(Value::is_string) {
                return Err(SkillError::InvalidPayload(format!(
                    "context.period.{key} is required"
                )));
            }
        }
    }
    if let Some(evidence) = context.get("evidence") {
        let Some(evidence) = evidence.as_array() else {
            return Err(SkillError::InvalidPayload(
                "context.evidence must be an array".into(),
            ));
        };
        if evidence.iter().any(|item| {
            !item.is_object()
                || !item.get("id").is_some_and(Value::is_string)
                || !item.get("kind").is_some_and(Value::is_string)
        }) {
            return Err(SkillError::InvalidPayload(
                "each context.evidence item requires string id and kind".into(),
            ));
        }
    }
    if context.get("comparison_basis").is_some() && context.get("period").is_none() {
        return Err(SkillError::InvalidPayload(
            "context.comparison_basis requires context.period".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn registry_validates_payload_and_capabilities() {
        let mut registry = SkillRegistry::default();
        let mut renderers = BTreeMap::new();
        renderers.insert(
            "desktop".into(),
            RendererBinding {
                renderer: "native:chart".into(),
                interactive: true,
                requires: vec!["charts".into()],
            },
        );
        assert!(
            registry
                .register(PresentationSkillManifest {
                    id: "core".into(),
                    version: "1.0.0".into(),
                    api: PRESENTATION_SKILL_API.into(),
                    provides: vec!["chart".into()],
                    renderers,
                    schema: None,
                    outcome_requirements: Vec::new(),
                })
                .is_ok()
        );
        let output = StructuredOutput {
            semantic_type: "chart".into(),
            schema_version: crate::PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: serde_json::json!({"chart_type":"line","series":[],"accessible_summary":"No data"}),
        };
        assert!(
            registry
                .validate(&output, "desktop", &["charts".into()])
                .is_ok()
        );
        assert!(matches!(
            registry.validate(&output, "desktop", &[]),
            Err(SkillError::MissingCapability(_))
        ));
    }

    #[test]
    fn every_seed_semantic_type_is_accepted_by_the_registry() {
        // A seed presentation that accepts a semantic type the delivery
        // registry does not declare can never be reached: `validate` rejects
        // the output as `UnknownType` before the compiler ever sees it. This
        // is the drift that left `table` unroutable while a `table` seed and
        // a `table` prompt example both existed.
        let accepted = built_in_semantic_types();
        for record in vak_presentation::seeds::built_in_seed_pack() {
            for semantic_type in &record.spec.accepts {
                assert!(
                    accepted.contains(semantic_type),
                    "seed {} accepts `{semantic_type}`, which no skill in the built-in \
                     registry provides — the output would be rejected as UnknownType",
                    record.spec.id
                );
            }
        }
    }

    #[test]
    fn desktop_renderer_vocabulary_matches_the_delivery_registry() {
        // The delivery registry is the admission boundary and the desktop
        // registry is the rendering boundary. A type present on only one side
        // either gets rejected before delivery or reaches the client without a
        // renderer. Keep this check close to the authoritative server list so
        // `cargo test -p vak-delivery` catches either direction of drift.
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vak-client-ui/src/components/PresentationRenderer.tsx"
        ));
        let registry = source
            .split("const STRUCTURED_RENDERERS:")
            .nth(1)
            .and_then(|tail| tail.split("export const structuredRendererTypes").next())
            .unwrap_or_default();
        assert!(
            !registry.is_empty(),
            "could not locate STRUCTURED_RENDERERS in PresentationRenderer.tsx"
        );

        let mut rendered: Vec<String> = registry
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let rest = line.strip_prefix('"')?;
                let (semantic_type, rest) = rest.split_once('"')?;
                rest.trim_start()
                    .starts_with(':')
                    .then(|| semantic_type.to_owned())
            })
            .collect();
        rendered.sort();
        rendered.dedup();

        let accepted = built_in_semantic_types();
        let server_only: Vec<_> = accepted
            .iter()
            .filter(|semantic_type| !rendered.contains(semantic_type))
            .collect();
        let client_only: Vec<_> = rendered
            .iter()
            .filter(|semantic_type| !accepted.contains(semantic_type))
            .collect();
        assert!(
            server_only.is_empty() && client_only.is_empty(),
            "presentation vocabulary drift: server-only={server_only:?}, client-only={client_only:?}"
        );
    }

    #[test]
    fn registry_rejects_ambiguous_semantic_type_ownership() {
        let mut registry = built_in_skill_registry();
        let result = registry.register(PresentationSkillManifest {
            id: "other-skill".into(),
            version: "1.0.0".into(),
            api: PRESENTATION_SKILL_API.into(),
            provides: vec!["data.grid".into()],
            renderers: BTreeMap::new(),
            schema: None,
            outcome_requirements: Vec::new(),
        });
        assert!(
            matches!(result, Err(SkillError::InvalidManifest(reason)) if reason.contains("already owned"))
        );
    }

    #[test]
    fn recipe_selection_is_deterministic_and_surface_aware() {
        let mut catalog = RecipeCatalog::default();
        let mut fallback = BTreeMap::new();
        fallback.insert("desktop".into(), "native:weather".into());
        assert!(
            catalog
                .register(PresentationRecipe {
                    id: "weather.forecast".into(),
                    version: "1.0.0".into(),
                    owner_skill: None,
                    priority: 0,
                    match_signals: vec!["temperature".into(), "forecast".into()],
                    primary: vec!["metric.group".into()],
                    optional: vec!["chart.line".into()],
                    fallback,
                    surfaces: Vec::new(),
                    default_recipe: false,
                    requires_typed_output: false,
                    typed_output_types: Vec::new(),
                })
                .is_ok()
        );
        let decision = catalog.choose(&["temperature".into(), "forecast".into()], "desktop");
        assert!(decision.is_some());
        if let Some(decision) = decision {
            assert_eq!(decision.recipe_id, "weather.forecast");
            assert_eq!(decision.matched_signals.len(), 2);
            assert_eq!(decision.disposition, DecisionDisposition::Unresolved);
        }
    }

    #[test]
    fn revoked_recipe_owner_is_removed_without_affecting_builtins() {
        let mut catalog = RecipeCatalog::default();
        catalog
            .register(PresentationRecipe {
                id: "plugin.report".into(),
                version: "1.0.0".into(),
                owner_skill: Some("plugin.report_skill".into()),
                priority: 10,
                match_signals: vec!["report".into()],
                primary: vec!["plugin.report".into()],
                optional: Vec::new(),
                fallback: BTreeMap::from([(
                    String::from("desktop"),
                    String::from("markdown:native"),
                )]),
                surfaces: vec!["desktop".into()],
                default_recipe: false,
                requires_typed_output: false,
                typed_output_types: Vec::new(),
            })
            .expect("valid recipe");
        catalog
            .register(PresentationRecipe {
                id: "builtin.report".into(),
                version: "1.0.0".into(),
                owner_skill: None,
                priority: 0,
                match_signals: vec!["report".into()],
                primary: vec!["report".into()],
                optional: Vec::new(),
                fallback: BTreeMap::from([(
                    String::from("desktop"),
                    String::from("markdown:native"),
                )]),
                surfaces: vec!["desktop".into()],
                default_recipe: false,
                requires_typed_output: false,
                typed_output_types: Vec::new(),
            })
            .expect("valid recipe");

        catalog.remove_revoked_skills(&std::collections::BTreeSet::from([String::from(
            "plugin.report_skill",
        )]));

        assert_eq!(catalog.recipes.len(), 1);
        assert_eq!(catalog.recipes[0].id, "builtin.report");
    }

    #[test]
    fn planner_keeps_rejected_items_auditable() {
        let mut skills = SkillRegistry::default();
        let mut renderers = BTreeMap::new();
        renderers.insert(
            "desktop".into(),
            RendererBinding {
                renderer: "native:chart".into(),
                interactive: false,
                requires: vec!["charts".into()],
            },
        );
        assert!(
            skills
                .register(PresentationSkillManifest {
                    id: "core".into(),
                    version: "1.0.0".into(),
                    api: PRESENTATION_SKILL_API.into(),
                    provides: vec!["chart".into()],
                    renderers,
                    schema: None,
                    outcome_requirements: Vec::new(),
                })
                .is_ok()
        );
        let mut recipes = RecipeCatalog::default();
        assert!(
            recipes
                .register(PresentationRecipe {
                    id: "answer.basic".into(),
                    version: "1.0.0".into(),
                    owner_skill: None,
                    priority: 0,
                    match_signals: vec![],
                    primary: vec!["chart".into()],
                    optional: vec![],
                    fallback: BTreeMap::new(),
                    surfaces: vec!["desktop".into()],
                    default_recipe: false,
                    requires_typed_output: false,
                    typed_output_types: Vec::new(),
                })
                .is_ok()
        );
        let planner = PresentationPlanner { skills, recipes };
        let plan = planner.plan(&[], "desktop", &[], &[StructuredOutput {
            semantic_type: "chart".into(), schema_version: crate::PRESENTATION_SCHEMA_VERSION, skill_id: "core".into(), skill_version: "1.0.0".into(),
            payload: serde_json::json!({"chart_type":"line","series":[],"accessible_summary":"No data"}),
        }]);
        assert!(plan.accepted.is_empty());
        assert_eq!(plan.rejected.len(), 1);
    }

    #[test]
    fn planner_reports_surface_renderer_bindings_not_recipe_defaults() {
        let output = StructuredOutput {
            semantic_type: "data.grid".into(),
            schema_version: crate::PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: serde_json::json!({
                "columns": [{"key": "region", "label": "Region"}],
                "rows": [{"region": "APAC"}]
            }),
        };
        let planner = PresentationPlanner {
            skills: built_in_skill_registry(),
            recipes: built_in_recipes(),
        };

        let desktop = planner.plan(
            &["table_data".into(), "tabular".into()],
            "desktop",
            &[],
            std::slice::from_ref(&output),
        );
        let desktop_recipe = desktop.recipe.expect("desktop recipe");
        assert_eq!(desktop_recipe.recipe_id, "data.spreadsheet_grid");
        assert_eq!(desktop_recipe.renderer, "native:structured");
        assert_eq!(desktop_recipe.disposition, DecisionDisposition::Native);
        assert_eq!(desktop.renderers.len(), 1);
        assert_eq!(desktop.renderers[0].renderer, "native:structured");

        let terminal = planner.plan(
            &["table_data".into(), "tabular".into()],
            "terminal",
            &[],
            std::slice::from_ref(&output),
        );
        let terminal_recipe = terminal.recipe.expect("terminal recipe");
        assert_eq!(terminal_recipe.renderer, "builtin:generic");
        assert_eq!(terminal_recipe.disposition, DecisionDisposition::Fallback);
        assert_eq!(terminal.renderers[0].renderer, "builtin:generic");
    }

    #[test]
    fn link_extractor_deduplicates_and_limits_urls() {
        let items = link_previews_from_text(
            "See https://example.com/a, https://example.com/a and http://example.org",
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].semantic_type, "link.preview");
    }

    #[test]
    fn structured_extraction_is_domain_neutral_and_strict() {
        let text = "before\n```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"Temperature\",\"value\":25,\"unit\":\"C\"}}\n```\nafter";
        let outputs = structured_outputs_from_text(text);
        assert_eq!(outputs.len(), 1);
        assert!(structured_outputs_from_text("```vak\nnot-json\n```").is_empty());

        let unfenced = "Vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"Temperature\",\"value\":25,\"unit\":\"C\"}}\n\nProse.";
        let unfenced_outputs = structured_outputs_from_text(unfenced);
        assert_eq!(unfenced_outputs.len(), 1);
        assert_eq!(unfenced_outputs[0].semantic_type, "metric");
    }

    #[test]
    fn semantic_context_requires_complete_periods_and_evidence_shape() {
        let valid = r#"{"semantic_type":"metric","payload":{"label":"Close","value":10,"context":{"domain":"market","as_of":"2026-09-04T16:00:00+05:30","period":{"start":"2026-08-31","end":"2026-09-04","timezone":"Asia/Kolkata"},"comparison_basis":"prior_close_to_period_close","evidence":[{"id":"source-1","kind":"cited"}]}}}"#;
        assert_eq!(structured_outputs_from_text(valid).len(), 1);

        let missing_period_end = r#"{"semantic_type":"metric","payload":{"label":"Close","value":10,"context":{"comparison_basis":"weekly" ,"period":{"start":"2026-08-31","timezone":"Asia/Kolkata"}}}}"#;
        assert!(structured_outputs_from_text(missing_period_end).is_empty());

        let invalid_evidence = r#"{"semantic_type":"metric","payload":{"label":"Close","value":10,"context":{"evidence":[{"id":"source-1"}]}}}"#;
        assert!(structured_outputs_from_text(invalid_evidence).is_empty());
    }

    /// A tool result is almost never Markdown — it's a bare JSON envelope.
    /// One structural rule ("is the whole text a valid envelope?") has to
    /// serve every persona's tools without knowing any of them by name:
    /// a general user's weather lookup, a developer's CI runner, a knowledge
    /// worker's research aggregator, and a data analyst's dataset query all
    /// go through the identical unnamed code path.
    #[test]
    fn bare_json_tool_results_are_recognized_without_a_fence_or_any_tool_name() {
        // General user — a weather tool's raw JSON reply.
        let weather = structured_outputs_from_text(
            r#"{"semantic_type":"metric","payload":{"label":"Temperature","value":25,"unit":"C"}}"#,
        );
        assert_eq!(weather.len(), 1);
        assert_eq!(weather[0].semantic_type, "metric");

        // Developer — a CI/test runner's raw JSON reply.
        let ci = structured_outputs_from_text(
            r#"{"semantic_type":"test.report","payload":{"tests":[{"name":"it_compiles","status":"passed"}],"total":1,"passed":1,"failed":0,"skipped":0}}"#,
        );
        assert_eq!(ci.len(), 1);
        assert_eq!(ci[0].semantic_type, "test.report");

        // Knowledge worker — a research/aggregation tool's raw JSON reply.
        let research = structured_outputs_from_text(
            r#"{"semantic_type":"research.synthesis","payload":{"sources":[{"title":"Report","url":"https://example.com"}],"takeaways":[{"text":"Adoption is rising","citation_indices":[1]}]}}"#,
        );
        assert_eq!(research.len(), 1);
        assert_eq!(research[0].semantic_type, "research.synthesis");

        // Data analyst — a query/BI tool's raw JSON reply.
        let grid = structured_outputs_from_text(
            r#"{"semantic_type":"data.grid","payload":{"columns":[{"key":"region","label":"Region"}],"rows":[{"region":"APAC"}]}}"#,
        );
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0].semantic_type, "data.grid");

        // Data/telemetry consumer — a metrics tool's raw JSON reply.
        let chart = structured_outputs_from_text(
            r#"{"semantic_type":"chart","payload":{"chart_type":"line","series":[{"name":"p99","points":[{"x":1,"y":42.0}]}],"accessible_summary":"p99 latency over time"}}"#,
        );
        assert_eq!(chart.len(), 1);
        assert_eq!(chart[0].semantic_type, "chart");
        let timeline = structured_outputs_from_text(
            r#"{"semantic_type":"plan.timeline","payload":{"title":"Weekend trip","items":[{"label":"Travel","detail":"Train to Jaipur"}]}}"#,
        );
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].semantic_type, "plan.timeline");

        // A tool that just returns plain prose, or JSON with no declared
        // semantic_type, must not have a type guessed for it.
        assert!(structured_outputs_from_text("It is 25C and sunny in Austin.").is_empty());
        assert!(structured_outputs_from_text(r#"{"temp": 25, "condition": "sunny"}"#).is_empty());
        assert!(
            structured_outputs_from_text(r#"{"semantic_type":"metric","payload":{"label":"x"}}"#)
                .is_empty()
        );
    }

    /// A plan step may carry the alternatives the person chooses between;
    /// they are typed, validated, and spelled out in the text form a channel
    /// gets.
    #[test]
    fn plan_step_options_are_typed_and_reach_the_text_form() {
        let plan = structured_outputs_from_text(
            r#"{"semantic_type":"plan.timeline","payload":{"title":"Saturday","items":[{"label":"Morning","time":"9:00–12:00","detail":"Pancakes, then the park"},{"label":"Afternoon","detail":"Pick one","options":[{"label":"Science museum","detail":"Hands-on exhibits","facts":["20 min away","2 hours"]},{"label":"Botanic garden"}]}]}}"#,
        );
        assert_eq!(plan.len(), 1, "a step with options is a valid plan");
        let text = structured_markdown(&plan[0]);
        assert!(text.contains("- Morning (9:00–12:00): Pancakes, then the park"));
        assert!(
            text.contains("  - Option: Science museum: Hands-on exhibits · 20 min away, 2 hours")
        );
        assert!(text.contains("  - Option: Botanic garden"));

        for malformed in [
            r#"{"label":"Afternoon","options":"museum or garden"}"#,
            r#"{"label":"Afternoon","options":[{"detail":"no label"}]}"#,
            r#"{"label":"Afternoon","options":[{"label":"Museum","facts":"far"}]}"#,
            r#"{"label":"Afternoon","time":3}"#,
        ] {
            let source = format!(
                r#"{{"semantic_type":"plan.timeline","payload":{{"title":"Saturday","items":[{malformed}]}}}}"#
            );
            assert!(
                structured_outputs_from_text(&source).is_empty(),
                "rejected: {malformed}"
            );
        }
    }

    #[test]
    fn universal_recipes_selection_from_signals_and_context() {
        let catalog = built_in_recipes();

        // 1. Research synthesis
        let research_text = "Key takeaways from our findings:\n1. Rust is fast.\nSources:\nhttps://example.com/rust";
        let sigs = signals_from_text(research_text);
        let decision = catalog
            .choose(&sigs, "desktop")
            .expect("research synthesis decision");
        assert_eq!(decision.recipe_id, "research.synthesis");

        // 2. Diff inspector
        let diff_text = "Files changed:\n```diff\n@@ -1,2 +1,3 @@\n+added\n```";
        let sigs = signals_from_text(diff_text);
        let decision = catalog
            .choose(&sigs, "desktop")
            .expect("diff inspector decision");
        assert_eq!(decision.recipe_id, "coding.diff_inspector");

        // 3. Test report
        let test_text = "Test suite executed: 42 passed, 0 failures.";
        let sigs = signals_from_text(test_text);
        assert!(catalog.choose(&sigs, "desktop").is_none());

        // 4. Terminal session from bash tool context
        let cmd = serde_json::json!({"command": "docker ps -a"});
        let ctx = SignalContext {
            text: "Container status: exit 0",
            tool_name: Some("bash"),
            tool_input: Some(&cmd),
            tool_output: Some("CONTAINER ID IMAGE STATUS"),
            is_error: false,
            domains: &[],
        };
        let sigs = signals_from_context(&ctx);
        let decision = catalog.choose(&sigs, "desktop").expect("terminal decision");
        assert_eq!(decision.recipe_id, "terminal.session");

        // 5. Culinary recipe
        let recipe_text =
            "Recipe for Salmon:\nPrep time: 10m\nIngredients:\n- 2 fillets\n- 2 tbsp olive oil";
        let sigs = signals_from_text(recipe_text);
        let decision = catalog.choose(&sigs, "desktop").expect("recipe decision");
        assert_eq!(decision.recipe_id, "lifestyle.culinary_recipe");

        // 6. Data spreadsheet grid
        let table_text = "Dataset breakdown:\n| MRR | Growth |\n| --- | --- |\n| $10k | +20% |";
        let sigs = signals_from_text(table_text);
        let decision = catalog.choose(&sigs, "desktop").expect("grid decision");
        assert_eq!(decision.recipe_id, "data.spreadsheet_grid");

        // Incidental substrings in news prose must not select a coding report.
        let news = "Protests continued while the market surpassed expectations.";
        let sigs = signals_from_text(news);
        assert!(!sigs.iter().any(|signal| signal == "tests"));
        assert!(!sigs.iter().any(|signal| signal == "pass_fail"));
        assert_ne!(
            catalog.choose(&sigs, "desktop").map(|d| d.recipe_id),
            Some("coding.test_report".into())
        );

        // 7. Multi chart
        let chart_text =
            "Telemetry metrics chart showing p99 latency trend and req/sec throughput.";
        let sigs = signals_from_text(chart_text);
        let decision = catalog.choose(&sigs, "desktop").expect("chart decision");
        assert_eq!(decision.recipe_id, "data.multi_chart");
    }

    #[test]
    fn accepted_plugin_output_surfaces_declared_outcome_requirements() {
        let mut skills = SkillRegistry::default();
        let mut renderers = BTreeMap::new();
        renderers.insert(
            "desktop".into(),
            RendererBinding {
                renderer: "native:custom".into(),
                interactive: false,
                requires: Vec::new(),
            },
        );
        assert!(
            skills
                .register(PresentationSkillManifest {
                    id: "planning".into(),
                    version: "1.0.0".into(),
                    api: PRESENTATION_SKILL_API.into(),
                    provides: vec!["plan".into()],
                    renderers,
                    schema: None,
                    outcome_requirements: vec![OutcomeRequirementDeclaration {
                        id: "plan-next-steps".into(),
                        kind: "constraint".into(),
                        description: "include next steps".into(),
                        importance: "must".into(),
                    }],
                })
                .is_ok()
        );
        let planner = PresentationPlanner {
            skills,
            recipes: RecipeCatalog::default(),
        };
        let plan = planner.plan(
            &[],
            "desktop",
            &[],
            &[StructuredOutput {
                semantic_type: "plan".into(),
                schema_version: crate::PRESENTATION_SCHEMA_VERSION,
                skill_id: "planning".into(),
                skill_version: "1.0.0".into(),
                payload: serde_json::json!({"steps": []}),
            }],
        );
        assert_eq!(plan.outcome_requirements.len(), 1);
        assert_eq!(plan.outcome_requirements[0].id, "plan-next-steps");
        let mut outcome = vak_intent::OutcomeSpec::from_reading(
            "make a plan",
            &vak_intent::Reading::general(),
            1,
        );
        assert!(plan.merge_outcome_requirements(&mut outcome).is_empty());
        assert!(
            outcome
                .requirements
                .iter()
                .any(|requirement| requirement.id == "plan-next-steps")
        );
    }
}

#[cfg(test)]
mod structural_signal_tests {
    use super::signals_from_text;

    fn has(text: &str, signal: &str) -> bool {
        signals_from_text(text).iter().any(|s| s == signal)
    }

    #[test]
    fn a_real_markdown_table_is_tabular_even_when_indented_or_unspaced() {
        assert!(has(
            "| Index | Change |\n|---|---|\n| Nifty | -0.22% |\n",
            "tabular"
        ));
        assert!(has(
            "Results:\n\n  | a | b |\n  | - | - |\n  | 1 | 2 |\n",
            "tabular"
        ));
    }

    #[test]
    fn table_syntax_inside_a_code_sample_is_not_a_table() {
        let text = "Markdown tables look like this:\n\n```\n| a | b |\n|---|---|\n```\n";
        assert!(
            !has(text, "tabular"),
            "a table shown as source is not a table"
        );
        assert!(!has("use `|---|` as the separator row", "tabular"));
    }

    #[test]
    fn tab_separated_rows_are_tabular_but_one_stray_tab_is_not() {
        assert!(has("name\tqty\napple\t3\npear\t5\n", "tabular"));
        assert!(!has("He paused,\tthen left.", "tabular"));
    }

    #[test]
    fn diffs_are_recognised_by_structure() {
        assert!(has("```diff\n- old\n+ new\n```\n", "diff"));
        assert!(has("diff --git a/x b/x\n--- a/x\n+++ b/x\n", "diff"));
        assert!(has("@@ -1,2 +1,3 @@ fn main\n line\n", "diff"));
        assert!(
            !has("the @@ sign and diff of opinions", "diff"),
            "prose mentioning them is not a diff"
        );
    }
}
