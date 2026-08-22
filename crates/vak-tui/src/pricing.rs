/// Rough per-model pricing (USD per million tokens) for cost estimates.
/// Substring-matched on lowercase model names; unknown models return None
/// and the UI omits dollar figures rather than guessing.
pub fn usd_per_mtok(model: &str) -> Option<(f64, f64)> {
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

pub fn session_cost(model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    usd_per_mtok(model).map(|(i, o)| input_tokens as f64 / 1e6 * i + output_tokens as f64 / 1e6 * o)
}

pub fn format_cost(usd: f64) -> String {
    if usd <= 0.0 {
        "$0".to_string()
    } else if usd < 0.0001 {
        "<$0.0001".to_string()
    } else {
        format!("${usd:.4}")
    }
}
