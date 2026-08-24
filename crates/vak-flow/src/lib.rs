//! vak-flow: static, validated DAGs of typed nodes executed in dependency
//! layers. Definitions are frozen into each run record; failures follow an
//! explicit required/optional policy; completed nodes can be skipped on
//! resume.

pub mod adopt;
pub mod exec;
pub mod parse;
pub mod planner;
pub mod types;

pub use exec::{Executor, ExecutorDeps, FlowOutcome};
pub use parse::{ParseError, parse_flow};
pub use planner::{
    PLANNER_SYSTEM, PlanOutcome, ToolCatalogEntry, build_planner_prompt, extract_toml,
    plan_and_run, sanitize_basic_string_newlines,
};
pub use types::{FlowDef, FlowState, NodeDef, NodeResult, NodeStatus};
