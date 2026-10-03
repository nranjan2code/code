//! vak-tools: built-in agent tools behind one trait.
//!
//! Contract: tools never panic and never return Err; failures are
//! ToolOutput::error values fed back to the model for self-correction.

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

pub mod artifact;
pub mod bash;
pub mod broker;
pub mod context;
pub mod contract;
pub mod doc_read;
pub mod edit;
pub mod find_tools;
pub mod glob;
pub mod grep;
#[cfg(target_os = "linux")]
pub mod landlock;
pub mod mail_calendar;
pub mod office_apply;
mod office_pdf;
pub mod read;
pub mod recall;
pub mod retired;
pub mod sandbox;
pub mod sandbox_events;
pub mod webbrowse;
pub mod webfetch;
pub mod window;
pub mod write;

use async_trait::async_trait;
use serde_json::Value;

pub use context::ToolContext;
pub use contract::validate_input;
pub use find_tools::FindToolsTool;
pub use recall::{RecallRequest, RecallTool, apply_chars, apply_range, parse_recall_args};
pub use sandbox_events::{SandboxEvent, SandboxEventSink};
pub use webbrowse::WebBrowseTool;
pub use webfetch::WebFetchTool;
pub use window::{LINE_WINDOW_CHARS, RESULT_WINDOW_CHARS, bounded, window};

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

/// A validated card, ready to become a `Presentation` ledger entry
/// (docs/design/68-context-engine.md §10): the canonical payload plus the
/// schema-driven title and identity digest, and the skill that owns its type.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationCard {
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub schema_version: u32,
    /// Canonical (key-sorted) payload.
    pub payload: Value,
    pub title: String,
    pub identity_digest: String,
}

/// What a tool returns. `content` is the whole result: a tool never shortens
/// its own output, because the calling loop records it in full and decides
/// how much a request shows (`window`).
#[derive(Debug, Clone, Default)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    /// The cards a run this call delegated to showed (a `task` worker's).
    /// The calling loop records each as a presentation of this call, so the
    /// user sees it and the delegating agent can recall it. In-process only:
    /// a brokered worker's reply cannot fill it.
    pub delegated: Option<DelegatedCards>,
    /// The MCP server that answered this call, when one did. In-process
    /// only, like `delegated`; the calling loop records it as a
    /// `CallEffect` (docs/design/85-turn-graph.md, G0).
    pub mcp_source: Option<vak_session::types::McpSource>,
}

pub use vak_session::types::McpSource;

/// Whether a call reads or writes the file it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAccess {
    Read,
    Write,
}

/// The cards a delegated run validated and showed, and the session that ran
/// it, whose ledger holds each card's original call.
#[derive(Debug, Clone, PartialEq)]
pub struct DelegatedCards {
    pub session_id: String,
    pub cards: Vec<PresentationCard>,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: false,
            delegated: None,
            mcp_source: None,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: true,
            delegated: None,
            mcp_source: None,
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

/// Workspace-relative directory where files a person sends (on a channel or
/// dropped in a client) are saved, so every file tool reaches them under
/// invariant 10 (docs/design/72, "File in").
pub const INBOX_DIR: &str = "inbox";

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

    /// Whether a successful call *is* the answer being presented to the user
    /// (it shows a card) rather than a step toward one. The agent loop reads
    /// this to know a card was emitted; it does not infer it from the name.
    fn presents_cards(&self) -> bool {
        false
    }

    /// The capability domains this tool serves (`filesystem`, `web`, …; the
    /// vocabulary is `vak_core::capability::Domain`). The tool classifies
    /// itself so the harness never keeps a name table. Empty means
    /// undeclared, and an undeclared tool is always loaded: deferring is a
    /// context saving, never a policy, so an unknown tool fails open.
    fn serves(&self) -> &'static [&'static str] {
        &[]
    }

    /// Loaded into every turn's schemas regardless of the turn's reading —
    /// the orientation and discovery primitives a model needs to operate at
    /// all. Everything else is loaded when the reading predicts it and is
    /// otherwise one `find_tools` call away.
    fn always_loaded(&self) -> bool {
        false
    }

    /// The workspace path a successful call with these arguments delivers
    /// for the person to review through its own card (an Office draft with
    /// Review). The answer need not present it again: the presentation check
    /// stands down for the run, an identical call is answered with the first
    /// one's result, and a card previewing that path is withheld. Unlike
    /// `presents_cards`, this has no bearing on permission.
    fn delivered_file(&self, _args: &Value) -> Option<String> {
        None
    }

    /// The workspace file a call with `args` reads or writes, as the path
    /// argument names it. The calling loop records the file's digest after a
    /// successful call, so a later turn can tell it touched the same
    /// content. The tool declares this; the loop keeps no table of names.
    fn file_access(&self, _args: &Value) -> Option<(FileAccess, String)> {
        None
    }

    /// Whether a successful call produces the requested user-facing
    /// artifact. The runtime counts this only from the matching successful
    /// result, so a proposal or failed write is never completion evidence.
    fn produces_artifact(&self, args: &Value) -> bool {
        self.delivered_file(args).is_some()
    }

    /// A reason this call cannot succeed, decided from its arguments alone
    /// before permission is evaluated, so a person is never asked to approve
    /// a call that would only be refused (a text edit of a Word file). It
    /// can only refuse: `None` means "evaluate as usual", never "allowed".
    fn refusal(&self, _args: &Value) -> Option<String> {
        None
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
        std::sync::Arc::new(doc_read::DocReadTool),
        std::sync::Arc::new(office_apply::OfficeApplyTool),
    ]
}

/// Read/glob/grep/doc_read subset for explore-style workers.
pub fn read_only_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(read::ReadTool),
        std::sync::Arc::new(glob::GlobTool),
        std::sync::Arc::new(grep::GrepTool),
        std::sync::Arc::new(doc_read::DocReadTool),
    ]
}

pub fn brokered_default_tools(worker_exe: std::path::PathBuf) -> Vec<std::sync::Arc<dyn Tool>> {
    brokered_tools(worker_exe, &[])
}

/// The default tools behind the broker, each told which Office files are
/// new to the workspace a task copy was made from (see
/// [`ToolContext::new_documents`]).
pub fn brokered_tools(
    worker_exe: std::path::PathBuf,
    new_documents: &[String],
) -> Vec<std::sync::Arc<dyn Tool>> {
    default_tools()
        .into_iter()
        .map(|tool| {
            std::sync::Arc::new(broker::BrokeredTool::new(
                tool,
                worker_exe.clone(),
                new_documents.to_vec(),
            )) as std::sync::Arc<dyn Tool>
        })
        .collect()
}

pub fn brokered_read_only_tools(worker_exe: std::path::PathBuf) -> Vec<std::sync::Arc<dyn Tool>> {
    read_only_tools()
        .into_iter()
        .map(|tool| {
            std::sync::Arc::new(broker::BrokeredTool::new(
                tool,
                worker_exe.clone(),
                Vec::new(),
            )) as std::sync::Arc<dyn Tool>
        })
        .collect()
}

pub fn definitions(tools: &[std::sync::Arc<dyn Tool>]) -> Vec<vak_llm::ToolDefinition> {
    tools
        .iter()
        .map(|t| vak_llm::ToolDefinition::new(t.name(), t.description(), t.schema()))
        .collect()
}

/// Canonical tool name resolution for aliases and common model hallucinations.
/// Also remaps retired tool names (e.g. `python_eval`) to their replacement
/// (`bash`), so a model that still reaches for a retired name is guided to
/// the correct tool rather than producing an `unknown_capability` failure.
pub fn canonical_tool_name(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.eq_ignore_ascii_case("session-search") {
        return "session_search";
    }
    if trimmed.eq_ignore_ascii_case("propose-skill") {
        return "propose_skill";
    }
    if trimmed.eq_ignore_ascii_case("read-file") || trimmed.eq_ignore_ascii_case("read_file") {
        return "read";
    }
    if trimmed.eq_ignore_ascii_case("write-file") || trimmed.eq_ignore_ascii_case("write_file") {
        return "write";
    }
    if trimmed.eq_ignore_ascii_case("edit-file") || trimmed.eq_ignore_ascii_case("edit_file") {
        return "edit";
    }
    if trimmed.eq_ignore_ascii_case("shell")
        || trimmed.eq_ignore_ascii_case("sh")
        || trimmed.eq_ignore_ascii_case("sandbox")
        || trimmed.eq_ignore_ascii_case("sandbox_exec")
        || trimmed.eq_ignore_ascii_case("terminal")
        || trimmed.eq_ignore_ascii_case("exec")
    {
        return "bash";
    }
    // Retired tool names: redirect to their replacement (currently `bash`).
    for tool in retired::RETIRED_TOOLS {
        if tool.name.eq_ignore_ascii_case(trimmed) {
            return tool
                .replacement
                .split(" — ")
                .next()
                .unwrap_or("bash")
                .trim();
        }
    }
    name
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

    #[test]
    fn canonical_tool_name_resolves_common_hallucinations_and_aliases() {
        assert_eq!(canonical_tool_name("read_file"), "read");
        assert_eq!(canonical_tool_name("write_file"), "write");
        assert_eq!(canonical_tool_name("edit_file"), "edit");
        assert_eq!(canonical_tool_name("session-search"), "session_search");
        assert_eq!(canonical_tool_name("unknown_tool"), "unknown_tool");
    }

    #[test]
    fn canonical_tool_name_redirects_retired_tools_to_bash() {
        // python_eval and react_preview were retired in 3.0.21; the model
        // may still reach for them. They must resolve to `bash`.
        assert_eq!(canonical_tool_name("python_eval"), "bash");
        assert_eq!(canonical_tool_name("react_preview"), "bash");
        assert_eq!(canonical_tool_name("PYTHON_EVAL"), "bash");
    }
}
