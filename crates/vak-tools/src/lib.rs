//! vak-tools: built-in agent tools behind one trait.
//!
//! Contract: tools never panic and never return Err; failures are
//! ToolOutput::error values fed back to the model for self-correction.

pub mod bash;
pub mod broker;
pub mod context;
pub mod contract;
pub mod edit;
pub mod glob;
pub mod grep;
#[cfg(target_os = "linux")]
pub mod landlock;
pub mod python_runner;
pub mod react_runner;
pub mod read;
pub mod sandbox;
pub mod webbrowse;
pub mod webfetch;
pub mod write;

use async_trait::async_trait;
use serde_json::Value;

pub use context::{OutputLimits, ToolContext};
pub use contract::validate_input;
pub use python_runner::PythonTool;
pub use react_runner::ReactPreviewTool;
pub use webbrowse::WebBrowseTool;
pub use webfetch::WebFetchTool;

/// Machine classification of a tool failure, so the agent loop and the output
/// gate can reason about it instead of re-running keyword matches against
/// free-text errors. `ToolErrorKind::classify` is the single authority that
/// drives both the recovery hint and the repair budget — there is no parallel
/// classification in the agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorKind {
    /// The call shape was wrong (missing/invalid argument, schema mismatch,
    /// unknown capability, oversized payload, or partial/malformed transport
    /// data). Plausibly repairable by re-issuing with corrected arguments.
    Correctable,
    /// A transient transport fault; the runtime may retry, the model cannot
    /// repair it by editing arguments.
    Transient,
    /// The tool is not admitted this turn (not in the frozen capability set).
    NotAdmitted,
    /// Policy / permission / capability-surface denial.
    Denied,
    /// Authentication / authorization failure.
    Auth,
    /// Provider rate limiting.
    RateLimited,
    /// User-initiated cancellation.
    Cancelled,
    /// Unclassified.
    Unknown,
}

impl ToolErrorKind {
    /// Classify a tool-error content string. Order matters: non-repairable
    /// categories are matched before `Correctable`, so an auth error that
    /// happens to contain "invalid" is never misread as a repairable
    /// argument fault. This subsumes and extends the agent's old keyword
    /// list — including the previously-uncovered payload/size faults (e.g.
    /// a body exceeding the fetch byte cap) so they get classified instead
    /// of falling through to `Unknown`.
    pub fn classify(error: &str) -> Self {
        let lower = error.to_ascii_lowercase();
        if lower.contains("cancel") || lower.contains("aborted") {
            return Self::Cancelled;
        }
        if lower.contains("rate limit") || lower.contains("429") || lower.contains("retry-after") {
            return Self::RateLimited;
        }
        if lower.contains("unauthorized")
            || lower.contains("forbidden")
            || lower.contains("credential")
            || lower.contains("token")
            || lower.contains("authentication")
            || lower.contains("unauthenticated")
        {
            return Self::Auth;
        }
        if lower.contains("denied")
            || lower.contains("revoked")
            || lower.contains("not allowed")
            || lower.contains("permission")
            || lower.contains("approval")
            || lower.contains("policy")
        {
            return Self::Denied;
        }
        if lower.contains("not admitted") {
            return Self::NotAdmitted;
        }
        // Correctable: the caller can plausibly fix this by re-issuing with
        // corrected arguments. Covers the original hint set plus the
        // payload/size and generic argument faults that used to be missed.
        // `unknown_capability` is correctable — the model can switch to an
        // admitted broker name instead.
        if lower.contains("unknown_capability")
            || lower.contains("invalid")
            || lower.contains("missing")
            || lower.contains("parameter")
            || lower.contains("argument")
            || lower.contains("expected")
            || lower.contains("not a valid")
            || lower.contains("schema")
            || lower.contains("malformed")
            || lower.contains("truncated")
            || lower.contains("partial")
            || lower.contains("parse")
            || lower.contains("exceeds")
            || lower.contains("too large")
            || lower.contains("payload")
            || lower.contains("byte")
            || lower.contains("cap")
            || lower.contains("size")
            || lower.contains("input too")
            || lower.contains("timed out")
            || lower.contains("connection closed")
            || lower.contains("spawn failed")
            || lower.contains("protocol error")
            || lower.contains("tool task failed")
        {
            return Self::Correctable;
        }
        Self::Unknown
    }

    /// Whether the failure class carries a model recovery hint today.
    /// `Transient`, `Auth`, `RateLimited`, `Denied`, `Cancelled`,
    /// `NotAdmitted`, and `Unknown` deliberately do **not** — matching the
    /// original contract that only requires an external state change or a
    /// model-side argument fix.
    pub fn is_correctable(self) -> bool {
        matches!(self, Self::Correctable)
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: true,
        }
    }

    /// Classify this output's failure kind, or `Unknown` for a success.
    pub fn classify(&self) -> ToolErrorKind {
        if !self.is_error {
            return ToolErrorKind::Unknown;
        }
        ToolErrorKind::classify(&self.content)
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput;

    /// What this call needs from the world while it runs. The default is
    /// unclaimed: schedulers may run it alongside anything.
    fn claims(&self, _args: &Value) -> ResourceClaims {
        ResourceClaims::default()
    }
}

/// Declared concurrent-execution constraints for one tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceClaims {
    /// Conflicts with every other claimed call (unknown or whole-world scope).
    pub exclusive: bool,
    /// Never conflicts; safe to fan out freely.
    pub read_only: bool,
    /// Path scopes the call will touch (globs; `src/**` style).
    pub paths: Vec<String>,
}

impl ResourceClaims {
    pub fn is_unclaimed(&self) -> bool {
        !self.exclusive && !self.read_only && self.paths.is_empty()
    }

    /// Conservative prefix test after normalization: two write scopes
    /// conflict when one contains the other. May over-serialize; never
    /// under-serializes.
    pub fn conflicts(&self, other: &ResourceClaims) -> bool {
        if self.is_unclaimed() || other.is_unclaimed() || self.read_only || other.read_only {
            return false;
        }
        if self.exclusive || other.exclusive {
            return true;
        }
        for a in &self.paths {
            let na = normalize_scope(a);
            for b in &other.paths {
                let nb = normalize_scope(b);
                if na.starts_with(&nb) || nb.starts_with(&na) {
                    return true;
                }
            }
        }
        false
    }
}

fn normalize_scope(scope: &str) -> String {
    let mut s = scope.trim().trim_end_matches(['/', '*']).to_string();
    while s.ends_with('/') {
        s.pop();
    }
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

pub fn default_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(read::ReadTool),
        std::sync::Arc::new(write::WriteTool),
        std::sync::Arc::new(edit::EditTool),
        std::sync::Arc::new(bash::BashTool),
        std::sync::Arc::new(glob::GlobTool),
        std::sync::Arc::new(grep::GrepTool),
    ]
}

/// Read/glob/grep subset for explore-style subagents.
pub fn read_only_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(read::ReadTool),
        std::sync::Arc::new(glob::GlobTool),
        std::sync::Arc::new(grep::GrepTool),
    ]
}

pub fn brokered_default_tools(worker_exe: std::path::PathBuf) -> Vec<std::sync::Arc<dyn Tool>> {
    default_tools()
        .into_iter()
        .map(|tool| {
            std::sync::Arc::new(broker::BrokeredTool::new(tool, worker_exe.clone()))
                as std::sync::Arc<dyn Tool>
        })
        .collect()
}

pub fn brokered_read_only_tools(worker_exe: std::path::PathBuf) -> Vec<std::sync::Arc<dyn Tool>> {
    read_only_tools()
        .into_iter()
        .map(|tool| {
            std::sync::Arc::new(broker::BrokeredTool::new(tool, worker_exe.clone()))
                as std::sync::Arc<dyn Tool>
        })
        .collect()
}

pub fn definitions(tools: &[std::sync::Arc<dyn Tool>]) -> Vec<vak_llm::ToolDefinition> {
    tools
        .iter()
        .map(|t| vak_llm::ToolDefinition::new(t.name(), t.description(), t.schema()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_correctable_covers_arg_shape_and_payload_faults() {
        // The previously-uncovered webfetch byte-cap case must now classify
        // as correctable instead of falling through to Unknown (which meant
        // it got no recovery hint at all in the agent).
        assert_eq!(
            ToolErrorKind::classify("response body exceeds the 524288 byte cap"),
            ToolErrorKind::Correctable,
        );
        assert_eq!(
            ToolErrorKind::classify("missing required parameter: server"),
            ToolErrorKind::Correctable,
        );
        assert_eq!(
            ToolErrorKind::classify("invalid tool arguments: expected object"),
            ToolErrorKind::Correctable,
        );
        assert_eq!(
            ToolErrorKind::classify(r#"{"type":"unknown_capability","name":"tavily_search"}"#),
            ToolErrorKind::Correctable,
        );
        assert_eq!(
            ToolErrorKind::classify("mcp protocol error: invalid argument"),
            ToolErrorKind::Correctable,
        );
    }

    #[test]
    fn classify_non_repairable_classes_get_no_hint() {
        assert_eq!(
            ToolErrorKind::classify("cancelled"),
            ToolErrorKind::Cancelled
        );
        assert_eq!(
            ToolErrorKind::classify("429 rate limit exceeded"),
            ToolErrorKind::RateLimited,
        );
        assert_eq!(
            ToolErrorKind::classify("capability denied by channel policy"),
            ToolErrorKind::Denied,
        );
        assert_eq!(
            ToolErrorKind::classify("unauthorized: bad token"),
            ToolErrorKind::Auth,
        );
        assert_eq!(
            ToolErrorKind::classify("tool not admitted on this turn"),
            ToolErrorKind::NotAdmitted,
        );
        assert!(!ToolErrorKind::classify("cancelled").is_correctable());
        assert!(!ToolErrorKind::classify("429 rate limit").is_correctable());
        assert!(ToolErrorKind::classify("missing required parameter: server").is_correctable());
    }

    #[test]
    fn success_classifies_unknown() {
        let ok = ToolOutput::ok("done");
        assert_eq!(ok.classify(), ToolErrorKind::Unknown);
        assert!(!ok.classify().is_correctable());
    }
}
