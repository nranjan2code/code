//! vak-flow: static, validated DAGs of typed nodes executed in dependency
//! layers. Definitions are frozen into each run record; failures follow an
//! explicit required/optional policy; completed nodes can be skipped on
//! resume.

pub mod exec;
pub mod parse;
pub mod types;

pub use exec::{Executor, ExecutorDeps, FlowOutcome};
pub use parse::{ParseError, parse_flow};
pub use types::{FlowDef, FlowState, NodeDef, NodeResult, NodeStatus};
