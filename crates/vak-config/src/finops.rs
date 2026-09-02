//! Spend pricing (docs/design/15-reliability.md). Every dollar figure produced
//! here is an ESTIMATE — providers do not return cost in responses, so
//! rows are labeled `estimated` and never treated as settled fact.

use crate::PriceEntry;
use vak_llm::Usage;

/// Rough per-model pricing (USD per million tokens), substring-matched on
/// lowercase model names. Unknown models are UNKNOWN: callers omit dollar
/// figures rather than guessing at zero.
pub fn usd_per_mtok_heuristic(model: &str) -> Option<(f64, f64)> {
    let m = model.to_ascii_lowercase();
    let has = |s: &str| m.contains(s);
    if has("ox-alpha") || has("opencode") {
        return Some((0.0, 0.0));
    }
    if has("opus") {
        return Some((15.0, 75.0));
    }
    if has("sonnet") {
        return Some((3.0, 15.0));
    }
    if has("haiku") {
        return Some((0.8, 4.0));
    }
    if has("4o-mini") || (has("mini") && has("gpt")) {
        return Some((0.15, 0.6));
    }
    if has("gpt-4o") {
        return Some((2.5, 10.0));
    }
    if has("gpt-4-turbo") {
        return Some((10.0, 30.0));
    }
    if has("o3") {
        return Some((2.0, 8.0));
    }
    if has("gemini") && has("flash") {
        return Some((0.30, 2.5));
    }
    if has("gemini") {
        return Some((1.25, 10.0));
    }
    if has("deepseek") {
        return Some((0.27, 1.1));
    }
    None
}

/// Exact-id overrides win; heuristic substring table is the fallback.
pub fn resolve_usd_per_mtok(
    model: &str,
    overrides: &std::collections::BTreeMap<String, PriceEntry>,
) -> Option<(f64, f64)> {
    if let Some(e) = overrides.get(model) {
        return Some((e.input, e.output));
    }
    usd_per_mtok_heuristic(model)
}

/// Estimated USD for a usage record. Cache-creation tokens bill at the
/// input rate; cache reads at a tenth of it (provider-typical discount).
pub fn estimate_cost_usd(
    model: &str,
    usage: &Usage,
    overrides: &std::collections::BTreeMap<String, PriceEntry>,
) -> Option<f64> {
    let (i, o) = resolve_usd_per_mtok(model, overrides)?;
    let fresh_input = usage.input_tokens as f64;
    let created = usage.cache_creation_input_tokens.unwrap_or(0) as f64;
    let read = usage.cache_read_input_tokens.unwrap_or(0) as f64;
    let out = usage.output_tokens as f64;
    let mtok = 1e6;
    Some((fresh_input + created) / mtok * i + read / mtok * i * 0.1 + out / mtok * o)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn override_beats_heuristic_and_exact_key_wins() {
        let mut o = std::collections::BTreeMap::new();
        o.insert(
            "claude-opus-4-x".to_string(),
            PriceEntry {
                input: 1.5,
                output: 7.5,
            },
        );
        assert_eq!(
            resolve_usd_per_mtok("claude-opus-4-x", &o),
            Some((1.5, 7.5))
        );
        // Unrelated model falls through to the heuristic.
        assert_eq!(
            resolve_usd_per_mtok("claude-sonnet-9", &o),
            Some((3.0, 15.0))
        );
        // Unknown without override stays UNKNOWN.
        assert_eq!(resolve_usd_per_mtok("totally-novel-model", &o), None);
    }

    #[test]
    fn estimate_prices_all_token_classes() {
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_creation_input_tokens: Some(500_000),
            cache_read_input_tokens: Some(2_000_000),
        };
        let usd =
            estimate_cost_usd("claude-sonnet-4", &usage, &Default::default()).expect("priced");
        // input 1M*3 + creation 0.5M*3 + read 2M*3*0.1 + out 0.1M*15
        let expected = 3.0 + 1.5 + 0.6 + 1.5;
        assert!((usd - expected).abs() < 1e-9, "{usd} vs {expected}");
    }

    #[test]
    fn unpriced_model_estimates_nothing() {
        let u = Usage::default();
        assert_eq!(estimate_cost_usd("mystery", &u, &Default::default()), None);
    }
}
