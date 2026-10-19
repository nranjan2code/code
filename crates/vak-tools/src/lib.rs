//! vak-tools: built-in agent tools behind one trait.
//!
//! Contract: tools never panic and never return Err; failures are
//! ToolOutput::error values fed back to the model for self-correction.

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

pub mod arguments;
pub mod artifact;
pub mod bash;
pub mod broker;
pub mod capability;
pub mod context;
pub mod contract;
pub mod diff;
pub mod doc_read;
pub mod drafts;
pub mod edit;
pub mod egress;
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

pub use arguments::{JsonFault, json_fault, read_arguments, unreadable};
pub use context::ToolContext;
pub use contract::validate_input;
pub use plan::{PlanTool, parse_plan_args};
pub use recall::{RecallRequest, RecallTool, apply_chars, apply_range, parse_recall_args};
pub use sandbox_events::{SandboxEvent, SandboxEventSink};
pub use webbrowse::Renderer;
pub use webfetch::WebFetchTool;
pub use window::{LINE_WINDOW_CHARS, RESULT_WINDOW_CHARS, bounded, window};

pub use vak_control::Fault;

/// A validated card, ready to become a `Presentation` ledger entry
/// (docs/design/68-context-engine.md §10): the canonical payload plus the
/// schema-driven title and identity digest, and the skill that owns its type.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationCard {
    /// The source entry id in the child session. Present only while a
    /// foreground worker's cards are being transferred to its parent.
    pub presentation_id: Option<String>,
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub schema_version: u32,
    /// Canonical (key-sorted) payload.
    pub payload: Value,
    pub title: String,
    pub identity_digest: String,
    /// The card block that built it, when its kind is a pack's
    /// (docs/design/91-presentation-capabilities.md §2).
    pub block: Option<String>,
    /// Exact ability invocation whose result supplied this card's data.
    pub data_call_id: Option<String>,
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
    pub delegated: Vec<DelegatedCards>,
    /// The MCP server that answered this call, when one did. In-process
    /// only, like `delegated`; the calling loop records it as a
    /// `CallEffect` (docs/design/85-turn-graph.md, G0).
    pub mcp_source: Option<vak_session::types::McpSource>,
    /// The connected provider account the content came from, when it did:
    /// the result is then kept under that account's key (plan M7a-b).
    pub account: Option<String>,
    /// What this result holds beyond what its tool declares
    /// (`Tool::provenance`): a delegated worker's result holds what the
    /// worker held. The calling loop joins the two.
    pub provenance: vak_control::Provenance,
    /// A process this call started ran: its exit code (none when it was
    /// stopped) and how long it ran, in milliseconds. The calling loop
    /// records it as `CallEffect::Ran`, failed or not.
    pub ran: Option<(Option<i32>, u64)>,
}

pub use vak_session::types::McpSource;

/// What results of a tool of this reach hold, when it declares nothing
/// more: reading the workspace, Vak's own records or a command's output is
/// the owner's data; the network and an integration server answer in
/// someone else's words.
pub fn provenance_of_reach(reach: vak_permission::Reach) -> vak_control::Provenance {
    use vak_control::Provenance;
    use vak_permission::Reach;
    match reach {
        Reach::ReadPath | Reach::ReadStore | Reach::Command | Reach::Learning | Reach::Schedule => {
            Provenance::PRIVATE
        }
        Reach::Network | Reach::Integration => Provenance::UNTRUSTED,
        Reach::Display | Reach::Coordination | Reach::WritePath | Reach::Other => Provenance::NONE,
    }
}

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
            delegated: Vec::new(),
            mcp_source: None,
            account: None,
            provenance: vak_control::Provenance::NONE,
            ran: None,
        }
    }

    /// The same output, naming the connected account it came from.
    /// Says a process this call started ran and how it ended.
    pub fn ran(mut self, exit_code: Option<i32>, duration_ms: u64) -> Self {
        self.ran = Some((exit_code, duration_ms));
        self
    }

    pub fn from_account(mut self, account: &str) -> Self {
        self.account = Some(account.to_string());
        self
    }

    pub fn error(fault: Fault, content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            fault: Some(fault),
            delegated: Vec::new(),
            mcp_source: None,
            account: None,
            provenance: vak_control::Provenance::NONE,
            ran: None,
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

/// What a turn read before a call made a version (docs/design/90-file-state.md
/// §5.1): a workspace file or draft by the path the call gave, with its
/// digest when it was in the workspace, or a page by its address.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadInput {
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        digest: Option<String>,
    },
    Page {
        url: String,
    },
}

/// A file a call produces (docs/design/82-library.md §3): every claim is
/// completion evidence, and a declared one is a Library entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactClaim {
    pub path: String,
    pub declared: bool,
    pub title: Option<String>,
    pub summary: Option<String>,
    /// The call wrote a draft of `path` for review, not the workspace file:
    /// the bytes are the call's own draft
    /// (`.vak/scratch/<agent>/<call>/<path>`, docs/design/90-file-state.md).
    pub drafted: bool,
    /// The draft the call started from, by its name, when it revised one:
    /// the new version's parent.
    pub based_on: Option<String>,
    /// The few words the call gave to name this version (its `change`
    /// argument), shown as its title in the file's history.
    pub change: Option<String>,
    /// What the turn read before this call, filled by the loop from the
    /// turn's recorded reads; a tool leaves it empty.
    pub read_first: Vec<ReadInput>,
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

/// See [`Tool::changes_made`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangesMade {
    /// The argument listing the changed files to show, as the calls that
    /// wrote them named them; every changed file when it is left out.
    pub paths: &'static str,
    /// The argument the loop fills: one record per file, with its
    /// `filename`, `hunks`, `additions` and `deletions`.
    pub files: &'static str,
}

/// See [`Tool::output_of_command`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandOutput {
    /// The argument holding the command exactly as a call ran it.
    pub command: &'static str,
    /// The argument the loop fills with that call's result, for a card
    /// that shows it (a test report shows only how the run ended).
    pub output: Option<&'static str>,
    /// The arguments the loop fills with how the process ended: its exit
    /// code (left out when it was stopped) and how long it ran.
    pub exit_code: &'static str,
    pub duration_ms: &'static str,
}

/// What a card shows that only a call in the conversation can have
/// produced (docs/design/68-context-engine.md §5). A card tool that declares
/// one is loaded once the conversation holds such a call, and promoted in
/// place at the step one appears, as a tool called by name is; until then it
/// is listed under "More tools". The facts are other tools' own records, so
/// a tool added at runtime that runs, writes, retrieves or answers data
/// brings its card with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Grounds {
    /// A process a call started ran (`CallEffect::Ran`).
    Ran,
    /// A call wrote a workspace file (`CallEffect::FileWrite`).
    Wrote,
    /// A call fetched a page or reached an integration (`CallEffect::Fetched`,
    /// `CallEffect::Mcp`).
    Retrieved,
    /// A call of a tool that answers data (`Tool::answers_data`) succeeded.
    Answered,
}

/// The facts `log`'s conversation holds, on the branch it is on. `answering`
/// names the tools that declare `Tool::answers_data`.
pub fn grounds_held(
    log: &vak_session::SessionLog,
    answering: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<Grounds> {
    use vak_session::types::CallEffect;
    let mut held = std::collections::BTreeSet::new();
    let mut failed = std::collections::HashSet::new();
    let mut answered = Vec::new();
    for entry in log.chain_to_root() {
        match &entry.payload {
            vak_session::EntryPayload::CallEffect(record) => match &record.effect {
                CallEffect::Ran { .. } => {
                    held.insert(Grounds::Ran);
                }
                CallEffect::FileWrite { .. } => {
                    held.insert(Grounds::Wrote);
                }
                CallEffect::Fetched { .. } | CallEffect::Mcp(_) => {
                    held.insert(Grounds::Retrieved);
                }
                CallEffect::Failed { .. } => {
                    failed.insert(record.tool_use_id.clone());
                }
                _ => {}
            },
            vak_session::EntryPayload::Message(record) => {
                for block in &record.message.content {
                    if let vak_llm::ContentBlock::ToolUse { id, name, .. } = block
                        && answering.contains(name)
                    {
                        answered.push(id.clone());
                    }
                }
            }
            _ => {}
        }
    }
    if answered.iter().any(|id| !failed.contains(id)) {
        held.insert(Grounds::Answered);
    }
    held
}

/// For a card tool that shows what one of the Agent's abilities answered
/// (docs/design/93 R5): the argument naming the ability, the argument
/// saying where the records are in its answer, and the arguments the loop
/// fills from them (`Tool::fill_from_data`), so a card's rows or points are
/// the answer's own, never values the model copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallData {
    pub from: &'static str,
    pub call_id: &'static str,
    pub list: &'static str,
    pub filled: &'static [&'static str],
}

impl CommandOutput {
    /// Every argument the loop fills.
    pub fn filled(&self) -> Vec<&'static str> {
        self.output
            .into_iter()
            .chain([self.exit_code, self.duration_ms])
            .collect()
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput;

    /// What this call needs from the world while it runs. The default is
    /// unknown: schedulers serialize it with other potentially mutating calls.
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

    /// The fields of a card this tool shows that may follow the values
    /// another card of the answer publishes: paths into the stored card,
    /// `*` standing for every index of a list (docs/design/98 §2).
    fn followable_fields(&self) -> Vec<Vec<String>> {
        Vec::new()
    }

    /// The form a card this tool shows is drawn as (`StructuredOutput.block`),
    /// for a tool that presents cards of one form. The client draws an empty
    /// shell of it while a call is written.
    fn card_block(&self) -> Option<&str> {
        None
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
    fn serves(&self) -> Vec<&str> {
        Vec::new()
    }

    /// What this tool reaches and what a permission rule about it matches
    /// (`vak_permission::Declared`). The engine's mode defaults read this,
    /// never the tool's name. A tool that declares nothing is an unknown:
    /// read-only mode denies it and workspace-write asks.
    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::default()
    }

    /// What this tool's results hold: the owner's data, words someone else
    /// wrote, both or neither (docs/design/08-permissions.md "Provenance").
    /// Read from its reach unless it declares otherwise.
    fn provenance(&self) -> vak_control::Provenance {
        provenance_of_reach(self.permission().reach)
    }

    /// Whether a call to this tool can send what the turn holds off this
    /// machine. Read from its reach unless it declares otherwise; a shell
    /// command is one only where the sandbox leaves it the network, which
    /// the calling loop decides.
    fn leaves_machine(&self) -> bool {
        matches!(
            self.permission().reach,
            vak_permission::Reach::Network | vak_permission::Reach::Integration
        )
    }

    /// What a call reaches, in the owner's words, from its declaration:
    /// an approval for it says this rather than the tool's name alone.
    /// Empty when the permission engine's own wording says enough.
    fn reaches(&self) -> Vec<String> {
        Vec::new()
    }

    /// For a card tool that shows what a command printed: the argument
    /// naming the command, as a call ran it, and the argument the loop
    /// fills with that call's recorded result, so the card shows what the
    /// command really printed, never words the model wrote for it. A card
    /// naming a command no call in the conversation ran is refused.
    fn output_of_command(&self) -> Option<CommandOutput> {
        None
    }

    /// For a card tool that shows the files this turn changed: the
    /// argument naming which of them to show, and the argument the loop
    /// fills with each one's change, taken from the file as it was before
    /// the turn first wrote it and as it is now, never from lines the model
    /// wrote for it. A card when no file changed is refused.
    fn changes_made(&self) -> Option<ChangesMade> {
        None
    }

    /// For a card tool that shows what an ability answered: which
    /// arguments name it and where its records are, and which the loop
    /// fills (`CallData`). A card naming an ability no call in the
    /// conversation answered is refused.
    fn data_of_call(&self) -> Option<CallData> {
        None
    }

    /// For a card tool: what only a call in the conversation can have
    /// produced for it to show (`Grounds`). None loads it from the first
    /// call, like every other card.
    fn grounded_in(&self) -> Option<Grounds> {
        None
    }

    /// Fills this card's arguments from `records`, the list an ability's
    /// answer holds where the card said; a refusal says what to fix.
    fn fill_from_data(
        &self,
        _args: &mut serde_json::Value,
        _records: &[serde_json::Value],
    ) -> Result<(), String> {
        Ok(())
    }

    /// Whether this tool's answer is data a card may take its rows or
    /// points from (`data_of_call`): an ability's program answers JSON.
    fn answers_data(&self) -> bool {
        false
    }

    /// The code-owned prompt contract this tool's use needs, shown only
    /// while a tool that declares it is admitted.
    fn prompt_contract(&self) -> Option<PromptContract> {
        None
    }

    /// Loaded into every turn's schemas: the tools a model works with on
    /// most turns (reading, searching and changing files, running commands,
    /// fetching a page, planning, discovery), and the ones a request in
    /// plain conversation names (remembering what the person says, finding
    /// it again). Every other tool that declares
    /// a domain is listed by name and loaded once called, and then stays
    /// loaded for the session (docs/design/88-control-plane.md §12.1).
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

    /// The web address a call with `args` fetches. The calling loop records
    /// it after a successful call, so a version made later in the turn
    /// names the page it read (docs/design/90-file-state.md §5.1).
    fn fetched(&self, _args: &Value) -> Option<String> {
        None
    }

    /// The pattern of workspace paths a call with these arguments lists
    /// (a glob), relative to the workspace: the loop also lists recorded
    /// files matching it that are not in the folder yet
    /// (docs/design/90-file-state.md §3.3).
    fn lists_paths(&self, _args: &Value) -> Option<String> {
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
            drafted: true,
            ..Default::default()
        })
    }

    /// The shell command a call with these arguments runs, for a tool that
    /// runs one. The calling loop counts it as execution, keeps it for the
    /// regression checks and screens it for a draft copied past review; it
    /// reads this declaration and never a tool's name.
    fn command(&self, _args: &Value) -> Option<String> {
        None
    }

    /// Whether a successful call looks something up for the request. Every
    /// tool does unless it says otherwise: one that shows a card is the
    /// answer, and one that only finds other tools or records a plan has
    /// looked at nothing yet.
    fn looks_up(&self) -> bool {
        !self.presents_cards()
    }

    /// Whether a successful call retrieves source material. This is a receipt
    /// of retrieval, never a verdict on relevance, freshness or claim support.
    /// Plans, displays, discovery of tools and commands alone are not sources.
    fn retrieves_evidence(&self, args: &Value) -> bool {
        self.looks_up()
            && (self.fetched(args).is_some()
                || matches!(self.file_access(args), Some((FileAccess::Read, _)))
                || self.serves().contains(&"source-discovery")
                || self.permission().reach == vak_permission::Reach::Integration)
    }

    /// The deliverables a call with these arguments declares, for the tool
    /// a model plans its answer with
    /// (docs/design/86-model-plans-runtime-checks.md §2.1).
    fn planned(&self, _args: &Value) -> Option<Vec<vak_intent::Deliverable>> {
        None
    }

    /// The argument, in milliseconds, that bounds how long one call may
    /// run. A call that leaves it out runs under the operator's limit
    /// (`[control.limits] command_secs`), which the calling loop fills in
    /// from this declaration (docs/design/88-control-plane.md §5a).
    fn time_limit_argument(&self) -> Option<&'static str> {
        None
    }

    /// The schema this call's arguments follow: the tool's own, unless the
    /// tool knows a narrower one for this call (an integration broker knows
    /// the discovered schema of the tool a call names). What regenerates an
    /// unreadable call writes it against this
    /// (docs/design/89-constrained-tool-arguments.md §4.3).
    fn call_schema(&self, _args: &Value) -> Value {
        self.schema()
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
    /// Conflicts with every potentially mutating call (whole-world scope).
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
    /// conflict when one contains the other. An unclaimed call has an
    /// unknown effect set, so it conflicts with any call that is not proven
    /// read-only. May over-serialize; never under-serializes.
    pub fn conflicts(&self, other: &ResourceClaims) -> bool {
        // Preserve the explicit read-only escape hatch: these calls are
        // proven not to change the workspace and may run alongside any call.
        if self.read_only || other.read_only {
            return false;
        }
        // An absent footprint says nothing about effects. Treat it as
        // potentially overlapping every call that may have effects,
        // including an exclusive call.
        if self.is_unclaimed() || other.is_unclaimed() {
            return true;
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

/// The tool in `tools` that a recorded call named `name` ran, by its exact
/// name, as the agent loop matches a call. Readers of
/// A call of `tool` that runs `command` within `limit_ms`, in the argument
/// names the tool declares (its command subject and its time-limit
/// argument), or `None` when the tool runs no command.
pub fn command_call(tool: &dyn Tool, command: &str, limit_ms: u64) -> Option<Value> {
    let vak_permission::Subject::Command(argument) = tool.permission().subject else {
        return None;
    };
    let mut call = serde_json::Map::new();
    call.insert(argument.to_string(), Value::from(command));
    if let Some(limit) = tool.time_limit_argument() {
        call.insert(limit.to_string(), Value::from(limit_ms));
    }
    Some(Value::Object(call))
}

/// A code-owned section of the system prompt a tool's use needs
/// (docs/design/07-prompt.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptContract {
    /// How Office files and PDFs are read and changed: drafts for review.
    Document,
    /// What the execution sandbox allows a command.
    Sandbox,
}

/// recorded calls use this and then the tool's declarations, never the name.
pub fn tool_for_call<'a>(
    tools: &'a [std::sync::Arc<dyn Tool>],
    name: &str,
) -> Option<&'a dyn Tool> {
    tools
        .iter()
        .find(|tool| tool.name() == name)
        .map(|tool| tool.as_ref())
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
