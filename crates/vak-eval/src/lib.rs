//! vak-eval: deterministic, in-process eval harness.
//!
//! Each case pairs a scripted model trajectory with a real workspace and a
//! verification command. The agent loop, tools, permissions, and session
//! ledger are the production code paths — only the provider is scripted.
//! Pass/fail plus token/cost metrics land in a JSON report so regressions
//! block releases.

pub mod cases;
pub mod context_gate;
pub mod no_first_class_integrations;
pub mod runner;

pub use cases::{builtin_suite, general_suite, held_out_suite, live_suite};
pub use context_gate::{ContextScorecard, MetricDirection, QualityMetric, run_context_scorecard};
pub use no_first_class_integrations::{Offense, scan_banned_tokens};
pub use runner::{
    EvalCase, EvalComparisonReport, EvalComparisonSuiteReport, EvalReport, EvalSuiteReport,
    ScriptedTurn, compare_case, compare_suite, run_case, run_case_brokered, run_case_with_provider,
    run_case_with_provider_brokered, run_suite,
};
