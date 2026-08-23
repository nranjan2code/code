//! vak-eval: deterministic, in-process eval harness.
//!
//! Each case pairs a scripted model trajectory with a real workspace and a
//! verification command. The agent loop, tools, permissions, and session
//! ledger are the production code paths — only the provider is scripted.
//! Pass/fail plus token/cost metrics land in a JSON report so regressions
//! block releases.

pub mod cases;
pub mod runner;

pub use cases::{builtin_suite, general_suite, live_suite};
pub use runner::{
    EvalCase, EvalReport, ScriptedTurn, run_case, run_case_brokered, run_case_with_provider,
    run_case_with_provider_brokered,
};
