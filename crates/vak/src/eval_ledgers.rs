//! `vak eval --ledgers` (docs/design/88-control-plane.md, C7): what the
//! decisions in this install's ledgers measure, by model, and the patterns
//! offered for review. Reads and changes nothing.

use std::path::PathBuf;

use vak_core::Core;

pub(crate) fn run(cwd: PathBuf, json: bool) -> i32 {
    let report = match Core::new(cwd).and_then(|core| vak_core::measure::report(&core)) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
        return 0;
    }
    println!(
        "{} turns in {} sessions ({} unreadable)",
        report["turns"], report["sessions"], report["unreadable"]
    );
    println!(
        "\n{:<28} {:>6} {:>8} {:>6} {:>10} {:>7} {:>8} {:>7} {:>8} {:>7}",
        "model",
        "turns",
        "answered",
        "failed",
        "step limit",
        "calls",
        "refused",
        "faults",
        "corrects",
        "steps"
    );
    for model in report["models"].as_array().into_iter().flatten() {
        println!(
            "{:<28} {:>6} {:>8} {:>6} {:>10} {:>7} {:>8} {:>7} {:>8} {:>7}",
            model["model"].as_str().unwrap_or(""),
            model["turns"].as_u64().unwrap_or(0),
            model["answered"].as_u64().unwrap_or(0),
            model["failed"].as_u64().unwrap_or(0),
            model["step_limit"].as_u64().unwrap_or(0),
            model["calls"].as_u64().unwrap_or(0),
            model["refused_calls"].as_u64().unwrap_or(0),
            model["failed_calls"].as_u64().unwrap_or(0),
            model["corrections"].as_u64().unwrap_or(0),
            model["median_steps"]
                .as_f64()
                .map_or("-".into(), |s| format!("{s:.1}")),
        );
    }
    let proposals = report["proposals"].as_array().cloned().unwrap_or_default();
    println!("\n{} patterns offered for review", proposals.len());
    for proposal in proposals {
        println!(
            "  {:>4} of {:<5} {}",
            proposal["count"].as_u64().unwrap_or(0),
            proposal["of"].as_u64().unwrap_or(0),
            proposal["key"].as_str().unwrap_or("")
        );
    }
    0
}
