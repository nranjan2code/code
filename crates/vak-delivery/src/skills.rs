//! Extensible, capability-aware presentation skills.
//!
//! Skills contribute typed data and renderer metadata. They never provide
//! executable desktop code; a surface chooses a trusted renderer or falls
//! back to the item's text representation.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PRESENTATION_SKILL_API: &str = "presentation.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationRecipe {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub match_signals: Vec<String>,
    pub primary: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    #[serde(default)]
    pub fallback: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationDecision {
    pub recipe_id: String,
    pub recipe_version: String,
    pub matched_signals: Vec<String>,
    pub renderer: String,
    pub disposition: DecisionDisposition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionDisposition {
    Native,
    Sandboxed,
    Fallback,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationPlan {
    pub recipe: Option<PresentationDecision>,
    pub accepted: Vec<StructuredOutput>,
    pub rejected: Vec<PlanDiagnostic>,
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

impl PresentationPlanner {
    pub fn plan(
        &self,
        signals: &[String],
        surface: &str,
        capabilities: &[String],
        candidates: &[StructuredOutput],
    ) -> PresentationPlan {
        let recipe = self.recipes.choose(signals, surface);
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        for candidate in candidates {
            match self.skills.validate(candidate, surface, capabilities) {
                Ok(_) => accepted.push(candidate.clone()),
                Err(error) => rejected.push(PlanDiagnostic {
                    semantic_type: candidate.semantic_type.clone(),
                    disposition: DecisionDisposition::Fallback,
                    reason: error.to_string(),
                }),
            }
        }
        PresentationPlan {
            recipe,
            accepted,
            rejected,
        }
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

#[derive(Debug, Clone, Default)]
pub struct RecipeCatalog {
    recipes: Vec<PresentationRecipe>,
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

    pub fn choose(&self, signals: &[String], surface: &str) -> Option<PresentationDecision> {
        self.recipes
            .iter()
            .filter_map(|recipe| {
                let matched_signals: Vec<String> = recipe
                    .match_signals
                    .iter()
                    .filter(|signal| signals.iter().any(|candidate| candidate == *signal))
                    .cloned()
                    .collect();
                if matched_signals.is_empty() && !recipe.match_signals.is_empty() {
                    return None;
                }
                let renderer = recipe
                    .fallback
                    .get(surface)
                    .cloned()
                    .unwrap_or_else(|| "builtin:generic".into());
                Some(PresentationDecision {
                    recipe_id: recipe.id.clone(),
                    recipe_version: recipe.version.clone(),
                    matched_signals,
                    renderer,
                    disposition: if recipe.fallback.contains_key(surface) {
                        DecisionDisposition::Native
                    } else {
                        DecisionDisposition::Fallback
                    },
                })
            })
            .max_by_key(|decision| decision.matched_signals.len())
    }
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
            vec!["citations", "multiple_sources"],
            vec!["outcome", "source.card"],
            vec!["desktop", "terminal"],
        ),
        (
            "research.synthesis",
            vec!["research", "synthesis", "takeaways"],
            vec!["research.card", "citations"],
            vec!["desktop", "terminal"],
        ),
        (
            "weather.forecast",
            vec!["temperature", "forecast"],
            vec!["metric.group", "chart.line"],
            vec!["desktop", "terminal", "telegram"],
        ),
        (
            "coding.change_summary",
            vec!["files_changed"],
            vec!["outcome", "artifact.collection"],
            vec!["desktop", "terminal"],
        ),
        (
            "coding.diff_inspector",
            vec!["diff", "files_changed"],
            vec!["coding.diff", "artifact.collection"],
            vec!["desktop", "terminal"],
        ),
        (
            "coding.test_report",
            vec!["tests", "pass_fail"],
            vec!["status", "table"],
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
            vec!["chart", "metric.group"],
            vec!["desktop", "terminal"],
        ),
        (
            "data.spreadsheet_grid",
            vec!["table_data", "tabular"],
            vec!["data.grid", "table"],
            vec!["desktop", "terminal"],
        ),
        (
            "lifestyle.culinary_recipe",
            vec!["recipe", "ingredients"],
            vec!["recipe.card", "timer"],
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
    ] {
        let fallback = recipe
            .3
            .into_iter()
            .map(|surface| (surface.into(), "builtin:generic".into()))
            .collect();
        let _ = catalog.register(PresentationRecipe {
            id: recipe.0.into(),
            version: "1.0.0".into(),
            match_signals: recipe.1.into_iter().map(String::from).collect(),
            primary: recipe.2.into_iter().map(String::from).collect(),
            optional: Vec::new(),
            fallback,
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
            "media.image",
            "media.video",
            "media.audio",
            "research.synthesis",
            "coding.diff",
            "test.report",
            "terminal.view",
            "data.grid",
            "recipe.card",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        renderers,
    });
    registry
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
        ("diff", &["diff --", "```diff", "@@ "][..]),
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
        ("tabular", &["| ---", "|---", "| :---", "|:---", "\t"][..]),
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
    checks
        .iter()
        .filter_map(|(signal, needles)| {
            needles
                .iter()
                .any(|needle| lower.contains(needle))
                .then_some((*signal).into())
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct SignalContext<'a> {
    pub text: &'a str,
    pub tool_name: Option<&'a str>,
    pub tool_input: Option<&'a serde_json::Value>,
    pub tool_output: Option<&'a str>,
    pub is_error: bool,
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
            "websearch" | "tavily" => {
                signals.push("citations".into());
                signals.push("multiple_sources".into());
                signals.push("research".into());
                signals.push("synthesis".into());
                signals.push("takeaways".into());
            }
            _ => {}
        }
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
            schema_version: 1,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: serde_json::json!({"url": url, "title": "Open source link"}),
        })
        .collect()
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

#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    skills: BTreeMap<String, PresentationSkillManifest>,
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
        self.skills.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&PresentationSkillManifest> {
        self.skills.get(id)
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
        validate_payload(&output.semantic_type, &output.payload)?;
        Ok(renderer)
    }
}

fn validate_payload(semantic_type: &str, payload: &Value) -> Result<(), SkillError> {
    let object = payload
        .as_object()
        .ok_or_else(|| SkillError::InvalidPayload("payload must be an object".into()))?;
    let required = match semantic_type {
        "link.preview" => &["url", "title"][..],
        "metric" => &["label", "value"][..],
        "chart" => &["chart_type", "series", "accessible_summary"][..],
        "media.image" | "media.video" | "media.audio" => &["source", "media_type", "alt"][..],
        _ => return Err(SkillError::UnknownType(semantic_type.into())),
    };
    if let Some(missing) = required.iter().find(|field| !object.contains_key(**field)) {
        return Err(SkillError::InvalidPayload(format!(
            "missing field {missing}"
        )));
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
                })
                .is_ok()
        );
        let output = StructuredOutput {
            semantic_type: "chart".into(),
            schema_version: 1,
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
    fn recipe_selection_is_deterministic_and_surface_aware() {
        let mut catalog = RecipeCatalog::default();
        let mut fallback = BTreeMap::new();
        fallback.insert("desktop".into(), "native:weather".into());
        assert!(
            catalog
                .register(PresentationRecipe {
                    id: "weather.forecast".into(),
                    version: "1.0.0".into(),
                    match_signals: vec!["temperature".into(), "forecast".into()],
                    primary: vec!["metric.group".into()],
                    optional: vec!["chart.line".into()],
                    fallback,
                })
                .is_ok()
        );
        let decision = catalog.choose(&["temperature".into(), "forecast".into()], "desktop");
        assert!(decision.is_some());
        if let Some(decision) = decision {
            assert_eq!(decision.recipe_id, "weather.forecast");
            assert_eq!(decision.matched_signals.len(), 2);
            assert_eq!(decision.disposition, DecisionDisposition::Native);
        }
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
                })
                .is_ok()
        );
        let mut recipes = RecipeCatalog::default();
        assert!(
            recipes
                .register(PresentationRecipe {
                    id: "answer.basic".into(),
                    version: "1.0.0".into(),
                    match_signals: vec![],
                    primary: vec!["chart".into()],
                    optional: vec![],
                    fallback: BTreeMap::new(),
                })
                .is_ok()
        );
        let planner = PresentationPlanner { skills, recipes };
        let plan = planner.plan(&[], "desktop", &[], &[StructuredOutput {
            semantic_type: "chart".into(), schema_version: 1, skill_id: "core".into(), skill_version: "1.0.0".into(),
            payload: serde_json::json!({"chart_type":"line","series":[],"accessible_summary":"No data"}),
        }]);
        assert!(plan.accepted.is_empty());
        assert_eq!(plan.rejected.len(), 1);
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
        let decision = catalog
            .choose(&sigs, "desktop")
            .expect("test report decision");
        assert_eq!(decision.recipe_id, "coding.test_report");

        // 4. Terminal session from bash tool context
        let cmd = serde_json::json!({"command": "docker ps -a"});
        let ctx = SignalContext {
            text: "Container status: exit 0",
            tool_name: Some("bash"),
            tool_input: Some(&cmd),
            tool_output: Some("CONTAINER ID IMAGE STATUS"),
            is_error: false,
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

        // 7. Multi chart
        let chart_text =
            "Telemetry metrics chart showing p99 latency trend and req/sec throughput.";
        let sigs = signals_from_text(chart_text);
        let decision = catalog.choose(&sigs, "desktop").expect("chart decision");
        assert_eq!(decision.recipe_id, "data.multi_chart");
    }
}
