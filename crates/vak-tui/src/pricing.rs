//! Cost estimates for UI display. Pricing lives in vak-config::finops so
//! the TUI, the FinOps ledger, and budget admission all read one source;
//! unknown models are UNKNOWN and the UI omits dollar figures rather than
//! guessing. Every figure shown here is an estimate.

pub fn usd_per_mtok(model: &str) -> Option<(f64, f64)> {
    vak_config::finops::usd_per_mtok_heuristic(model)
}

pub fn session_cost(model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    vak_config::finops::estimate_cost_usd(
        model,
        &vak_llm::Usage {
            input_tokens,
            output_tokens,
            ..Default::default()
        },
        &Default::default(),
    )
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
