//! vak-tools: built-in agent tools behind one trait.
//!
//! Contract: tools never panic and never return Err; failures are
//! ToolOutput::error values fed back to the model for self-correction.

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

pub mod arguments;
pub mod artifact;
pub mod bash;
pub mod broker;
pub mod context;
pub mod contract;
pub mod doc_read;
pub mod drafts;
pub mod edit;
pub mod find_tools;
pub mod glob;
pub mod grep;
#[cfg(target_os = "linux")]
pub mod landlock;
pub mod mail_calendar;
pub mod office_apply;
mod office_pdf;
pub mod plan;
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

pub use arguments::read_arguments;
pub use context::ToolContext;
pub use contract::validate_input;
pub use find_tools::FindToolsTool;
pub use plan::{PlanTool, parse_plan_args};
pub use recall::{RecallRequest, RecallTool, apply_chars, apply_range, parse_recall_args};
pub use sandbox_events::{SandboxEvent, SandboxEventSink};
pub use webbrowse::WebBrowseTool;
pub use webfetch::WebFetchTool;
pub use window::{LINE_WINDOW_CHARS, RESULT_WINDOW_CHARS, bounded, window};

pub use vak_control::Fault;

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
    /// Where the fault lies when the call failed, named where it failed
    /// (docs/design/88-control-plane.md §8). `None` is a success.
    pub fault: Option<Fault>,
    /// The cards a run this call delegated to showed (a `task` worker's).
    /// The calling loop records each as a presentation of this call, so the
    /// user sees it and the delegating agent can recall it. In-process only:
    /// a brokered worker's reply cannot fill it.
    pub delegated: Option<DelegatedCards>,
    /// The MCP server that answered this call, when one did. In-process
    /// only, like `delegated`; the calling loop records it as a
    /// `CallEffect` (docs/design/85-turn-graph.md, G0).
    pub mcp_source: Option<vak_session::types::McpSource>,
    /// The connected provider account the content came from, when it did:
    /// the result is then kept under that account's key (plan M7a-b).
    pub account: Option<String>,
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
            fault: None,
            delegated: None,
            mcp_source: None,
            account: None,
        }
    }

    /// The same output, naming the connected account it came from.
    pub fn from_account(mut self, account: &str) -> Self {
        self.account = Some(account.to_string());
        self
    }

    pub fn error(fault: Fault, content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            fault: Some(fault),
            delegated: None,
            mcp_source: None,
            account: None,
        }
    }

    pub fn is_error(&self) -> bool {
        self.fault.is_some()
    }
}

/// Workspace-relative directory where files a person sends (on a channel or
/// dropped in a client) are saved, so every file tool reaches them under
/// invariant 10 (docs/design/72, "File in").
pub const INBOX_DIR: &str = "inbox";

/// A file a call produces (docs/design/82-library.md §3): every claim is
/// completion evidence, and a declared one is a Library entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClaim {
    pub path: String,
    pub declared: bool,
    pub title: Option<String>,
    pub summary: Option<String>,
}

/// Where the agent loop reports every successful call's claim (plan M8):
/// `vak_core::artifacts::CallSink` records a declared one as an artifact
/// and a version, and an undeclared one as a version only of a file that
/// is already an artifact, under the run's trace key.
pub trait ArtifactSink: Send + Sync {
    fn declared(
        &self,
        claim: &ArtifactClaim,
        session: Option<&str>,
        call: &str,
        trace: Option<&vak_session::trace::TraceKey>,
    );
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

    /// Whether a call with these arguments may change the workspace. The
    /// runtime checkpoints the workspace before the first such call of a
    /// turn is run, so the declaration errs towards yes: by default any call
    /// that claims the world or a path it does not only read.
    fn changes_workspace(&self, args: &Value) -> bool {
        let claims = self.claims(args);
        claims.exclusive || (!claims.read_only && !claims.paths.is_empty())
    }

    /// Whether a successful call *is* the answer being presented to the user
    /// (it shows a card) rather than a step toward one. The agent loop reads
    /// this to know a card was emitted; it does not infer it from the name.
    fn presents_cards(&self) -> bool {
        false
    }

    /// The kinds of card this tool shows (`semantic_type`s), for a tool that
    /// presents cards: the answer document offers each as a kind of block
    /// (docs/design/87-composed-answers.md §2). By default the ones its
    /// schema lists; a tool whose one kind its schema leaves out names it.
    fn card_kinds(&self) -> Vec<String> {
        self.schema()["properties"]["semantic_type"]["enum"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    /// The capability domains this tool serves (`filesystem`, `web`, …; the
    /// vocabulary is `vak_core::capability::Domain`). The tool classifies
    /// itself so the harness never keeps a name table. Empty means
    /// undeclared, and an undeclared tool is always loaded: deferring is a
    /// context saving, never a policy, so an unknown tool fails open.
    fn serves(&self) -> &'static [&'static str] {
        &[]
    }

    /// Loaded into every turn's schemas: the tools a model works with on
    /// most turns (reading, searching and changing files, running commands,
    /// fetching a page, planning, discovery). Every other tool that declares
    /// a domain is one `find_tools` call away until a turn loads it, and then
    /// stays loaded for the session (docs/design/88-control-plane.md §12.1).
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

    /// The workspace file a successful call with these arguments produces,
    /// and whether the call declares it a deliverable for the person
    /// (docs/design/82-library.md §3). The runtime reads this only from the
    /// matching successful result, so a proposal or a failed write is
    /// neither completion evidence nor a Library entry.
    fn artifact(&self, args: &Value) -> Option<ArtifactClaim> {
        self.delivered_file(args).map(|path| ArtifactClaim {
            path,
            declared: true,
            title: None,
            summary: None,
        })
    }

    /// The argument, in milliseconds, that bounds how long one call may
    /// run. A call that leaves it out runs under the operator's limit
    /// (`[control.limits] command_secs`), which the calling loop fills in
    /// from this declaration (docs/design/88-control-plane.md §5a).
    fn time_limit_argument(&self) -> Option<&'static str> {
        None
    }

    /// Arguments this tool fills in itself when a call leaves them out,
    /// applied before the call is validated: a card tool whose name settles
    /// its type fills `semantic_type`, which its schema asks for first
    /// (docs/design/30-render-architecture.md §30.1, rule 5). It only adds
    /// a missing field and never changes one the call wrote.
    fn fill_defaults(&self, _args: &mut Value) {}

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
