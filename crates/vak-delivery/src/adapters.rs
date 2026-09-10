//! Provider-shape adapters: the one place external tool output is allowed to
//! be reinterpreted for rendering.
//!
//! `structured_outputs_from_text` (see `skills.rs`) covers tools we can ask
//! to speak our contract. Most tools are not ours to ask — a weather API, a
//! ticketing system, a third-party MCP server ships whatever shape it ships,
//! and nothing here can make the rest of the world adopt `semantic_type` /
//! `payload`. Rewriting a tool's actual output to fit our envelope would
//! also reach past our own boundary: that output is the same value the
//! ledger records, the same value a future turn's model sees, the same
//! value any other consumer reads — none of which asked for our rendering
//! concerns.
//!
//! So the boundary sits at render composition, not at the tool: an adapter
//! reads the *stored, untouched* result and, only for shapes it explicitly
//! recognizes, produces a `StructuredOutput` for that one rendering pass. It
//! never writes back to the ledger, never changes what the tool returned,
//! and — like every other structured candidate — never skips
//! `SkillRegistry::validate` before anything is trusted enough to render.
//!
//! An adapter is deliberately narrow: it matches one provider's actual,
//! known response shape and returns `None` for anything else. There is no
//! adapter here that pattern-matches on field names in general ("has a
//! `temp` key, must be weather") — that would be exactly the guessing this
//! module exists to avoid. Recognizing a *specific, known* schema precisely
//! is not guessing; inferring meaning from incidental field names is.
//!
//! Adding support for a new provider means writing one adapter and
//! registering it — never touching another adapter, never adding a branch
//! to the render pipeline itself, and never touching the tool that produced
//! the data.

use serde_json::Value;

use crate::skills::StructuredOutput;

/// Recognizes one external provider's specific, known response shape and
/// projects it into a `StructuredOutput` candidate — still subject to
/// `SkillRegistry::validate` before anything renders from it.
pub trait ResultAdapter: Send + Sync {
    /// Stable identifier for diagnostics (never shown as if it were a
    /// tool name the pipeline "knows" — it names the adapter, not a tool).
    fn id(&self) -> &'static str;

    /// Return `Some` only when `raw` matches this adapter's specific known
    /// shape. Any other shape — including one that merely looks similar —
    /// must return `None` rather than guess.
    fn adapt(&self, raw: &Value) -> Option<StructuredOutput>;
}

struct WeatherApiCurrentAdapter;

impl ResultAdapter for WeatherApiCurrentAdapter {
    fn id(&self) -> &'static str {
        "weatherapi.current"
    }

    fn adapt(&self, raw: &Value) -> Option<StructuredOutput> {
        let current = raw.get("current")?;
        let temperature = current.get("temp_c")?.as_f64()?;
        Some(StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: crate::PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            payload: serde_json::json!({
                "label": "Temperature",
                "value": temperature,
                "unit": "C",
            }),
        })
    }
}

/// An ordered, purely additive set of adapters. Order only matters as a
/// tie-break when two adapters both recognize the same input, which a
/// well-scoped adapter should make rare.
#[derive(Default)]
pub struct AdapterRegistry {
    adapters: Vec<Box<dyn ResultAdapter>>,
}

impl AdapterRegistry {
    pub fn register(&mut self, adapter: Box<dyn ResultAdapter>) {
        self.adapters.push(adapter);
    }

    /// Tries every registered adapter against `raw`, returning the first
    /// match. Registering a new adapter cannot change what an existing one
    /// matches — each is asked independently and in isolation.
    pub fn try_adapt(&self, raw: &Value) -> Option<StructuredOutput> {
        self.adapters.iter().find_map(|adapter| adapter.adapt(raw))
    }

    #[cfg(test)]
    fn ids(&self) -> Vec<&'static str> {
        self.adapters.iter().map(|adapter| adapter.id()).collect()
    }
}

/// A tool result's full path to becoming a render candidate: try the
/// self-declared contract first (`structured_outputs_from_text` — the tool
/// already speaks our envelope, fenced or bare), and only if that finds
/// nothing, try parsing the result as JSON and asking the adapter registry
/// whether it recognizes the shape. Either way, nothing is returned that
/// hasn't also passed `SkillRegistry::validate` for `surface` — an adapter
/// gets no more trust than a tool declaring its own type would.
pub fn structured_outputs_from_tool_result(
    text: &str,
    surface: &str,
    adapters: &AdapterRegistry,
) -> Vec<StructuredOutput> {
    structured_outputs_from_tool_result_with(
        text,
        surface,
        adapters,
        &crate::skills::built_in_skill_registry(),
    )
}

pub fn structured_outputs_from_tool_result_with(
    text: &str,
    surface: &str,
    adapters: &AdapterRegistry,
    skills: &crate::skills::SkillRegistry,
) -> Vec<StructuredOutput> {
    let declared = crate::skills::structured_outputs_from_text(text);
    if !declared.is_empty() {
        return declared;
    }
    let Ok(raw) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(candidate) = adapters.try_adapt(&raw) else {
        return Vec::new();
    };
    if skills.validate(&candidate, surface, &[]).is_ok() {
        vec![candidate]
    } else {
        Vec::new()
    }
}

pub fn built_in_adapters() -> AdapterRegistry {
    let mut registry = AdapterRegistry::default();
    registry.register(Box::new(WeatherApiCurrentAdapter));
    registry
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use serde_json::json;

    /// A stand-in for a real third-party shape, kept in the test module and
    /// named accordingly so it can never be mistaken for a shipped adapter.
    struct FixtureOpenMeteoLikeAdapter;

    impl ResultAdapter for FixtureOpenMeteoLikeAdapter {
        fn id(&self) -> &'static str {
            "fixture.open_meteo_like"
        }

        fn adapt(&self, raw: &Value) -> Option<StructuredOutput> {
            let current = raw.get("current_weather")?;
            let temperature = current.get("temperature")?.as_f64()?;
            Some(StructuredOutput {
                semantic_type: "metric".into(),
                schema_version: crate::PRESENTATION_SCHEMA_VERSION,
                skill_id: "core".into(),
                skill_version: "1.0.0".into(),
                payload: json!({
                    "label": "Temperature",
                    "value": temperature,
                    "unit": "C",
                }),
            })
        }
    }

    #[test]
    fn an_adapter_only_matches_its_own_known_shape() {
        let mut registry = AdapterRegistry::default();
        registry.register(Box::new(FixtureOpenMeteoLikeAdapter));

        let matched = registry
            .try_adapt(&json!({"current_weather": {"temperature": 21.5, "windspeed": 4.0}}))
            .expect("recognized shape should adapt");
        assert_eq!(matched.semantic_type, "metric");
        assert_eq!(matched.payload["value"], 21.5);

        // A shape that merely has some overlapping keys, or none at all,
        // must not be coerced into rendering as something it isn't.
        assert!(registry.try_adapt(&json!({"temp": 21.5})).is_none());
        assert!(registry.try_adapt(&json!({"unrelated": true})).is_none());
    }

    #[test]
    fn adapter_output_still_goes_through_normal_validation() {
        // The adapter itself can be wrong or stale; nothing here should let
        // its output skip the same schema check every other candidate goes
        // through before it is trusted enough to render.
        let mut registry = AdapterRegistry::default();
        registry.register(Box::new(FixtureOpenMeteoLikeAdapter));
        let candidate = registry
            .try_adapt(&json!({"current_weather": {"temperature": 21.5}}))
            .expect("recognized shape should adapt");
        let registry_check = crate::skills::built_in_skill_registry();
        assert!(registry_check.validate(&candidate, "desktop", &[]).is_ok());
    }

    #[test]
    fn built_in_adapters_register_real_provider_shapes() {
        assert_eq!(built_in_adapters().ids(), vec!["weatherapi.current"]);
    }

    #[test]
    fn a_self_declared_result_never_needs_an_adapter() {
        let mut adapters = AdapterRegistry::default();
        adapters.register(Box::new(FixtureOpenMeteoLikeAdapter));
        let outputs = structured_outputs_from_tool_result(
            r#"{"semantic_type":"metric","payload":{"label":"Temperature","value":25,"unit":"C"}}"#,
            "desktop",
            &adapters,
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].semantic_type, "metric");
    }

    #[test]
    fn an_unrecognized_provider_shape_renders_as_nothing_rather_than_a_guess() {
        let adapters = AdapterRegistry::default();
        let outputs = structured_outputs_from_tool_result(
            r#"{"current_weather": {"temperature": 21.5}}"#,
            "desktop",
            &adapters,
        );
        assert!(outputs.is_empty());
    }

    #[test]
    fn a_recognized_provider_shape_adapts_and_validates() {
        let mut adapters = AdapterRegistry::default();
        adapters.register(Box::new(FixtureOpenMeteoLikeAdapter));
        let outputs = structured_outputs_from_tool_result(
            r#"{"current_weather": {"temperature": 21.5, "windspeed": 4.0}}"#,
            "desktop",
            &adapters,
        );
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].semantic_type, "metric");
        assert_eq!(outputs[0].payload["value"], 21.5);
    }

    #[test]
    fn weatherapi_shape_becomes_specialist_output() {
        let adapters = built_in_adapters();
        let weather = structured_outputs_from_tool_result(
            r#"{"current":{"temp_c":31.5,"condition":{"text":"Sunny"}}}"#,
            "desktop",
            &adapters,
        );
        assert_eq!(weather[0].semantic_type, "metric");
    }
}
