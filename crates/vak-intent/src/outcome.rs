//! The outcome contract carried by a turn and, when needed, durable work —
//! and the control plane that can change it while it runs.
//!
//! # Control plane
//!
//! A running turn can be steered, paused, cancelled, re-planned or approved.
//! **Authority for any of that comes from the channel, never from the
//! text.** The transport stamps every request with a [`ControlSource`] —
//! a human on a surface, an agent in the same process, or an external
//! system — and [`evaluate_intervention`] decides from the source and the
//! kind alone. A body cannot claim to be a person.
//!
//! Text carries control only in one narrow form: an explicit [`Command`] from
//! a human — a leading slash command, or a whole message that is exactly one
//! of the short words `stop`, `cancel`, `pause`, `resume`, `status`. Anything
//! else a human types while a run is busy is steering text and reaches the
//! model between steps. "Stop using semicolons in the output" is a steer;
//! before this module it cancelled the run.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::axes::Act;

/// Who is asking. Set by the transport that received the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlSource {
    /// A person on a surface the runtime serves: CLI, desktop, web, a chat
    /// gateway. `principal` is whatever identity the surface has (a chat
    /// sender, a login), for the audit row.
    Human {
        surface: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        principal: Option<String>,
    },
    /// Another agent in this runtime — a parent steering a worker, a worker
    /// reporting to its parent, the commitment upkeep tick.
    Agent {
        session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_session_id: Option<String>,
    },
    /// An external system with no human behind it: an HTTP client that did
    /// not authenticate as a person, a cron trigger, a webhook.
    System { origin: String },
}

impl ControlSource {
    pub fn is_human(&self) -> bool {
        matches!(self, ControlSource::Human { .. })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ControlSource::Human { .. } => "human",
            ControlSource::Agent { .. } => "agent",
            ControlSource::System { .. } => "system",
        }
    }
}

/// A request that arrives after execution has begun. It is classified before
/// it can affect the plan; free-form text is never treated as an authority
/// change by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    Status,
    Steer,
    AddRequirement,
    RemoveRequirement,
    Reprioritize,
    Replan,
    Pause,
    Resume,
    Cancel,
    Approve,
    Reject,
}

impl InterventionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Steer => "steer",
            Self::AddRequirement => "add_requirement",
            Self::RemoveRequirement => "remove_requirement",
            Self::Reprioritize => "reprioritize",
            Self::Replan => "replan",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Cancel => "cancel",
            Self::Approve => "approve",
            Self::Reject => "reject",
        }
    }
}

/// An explicit command a human typed. The only way text becomes control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    Status,
    Pause,
    Resume,
    Cancel,
    Replan {
        text: String,
    },
    AddRequirement {
        text: String,
    },
    RemoveRequirement {
        text: String,
    },
    Reprioritize {
        text: String,
    },
    /// `/goal replace …`: the active goal is superseded by `text`.
    GoalReplace {
        text: String,
    },
    /// `/goal fix …`: the active goal is amended by `text`.
    GoalFix {
        text: String,
    },
    /// `/until-done …`: work on `text` and keep going past each apparent
    /// finish until the person says `done` or `stop`. The one way to hold
    /// completion for the person: a phrase in free text ("keep going until
    /// the tests pass") names a condition, not a hold.
    UntilDone {
        text: String,
    },
    /// `/approve <gate>` — matched to a raised gate by id, never by prose.
    Approve {
        gate_id: String,
    },
    Reject {
        gate_id: String,
    },
}

impl Command {
    pub fn intervention_kind(&self) -> InterventionKind {
        match self {
            Command::Status => InterventionKind::Status,
            Command::Pause => InterventionKind::Pause,
            Command::Resume => InterventionKind::Resume,
            Command::Cancel => InterventionKind::Cancel,
            Command::Replan { .. } => InterventionKind::Replan,
            Command::AddRequirement { .. } => InterventionKind::AddRequirement,
            Command::RemoveRequirement { .. } => InterventionKind::RemoveRequirement,
            Command::Reprioritize { .. } => InterventionKind::Reprioritize,
            // Goal edits ride the same re-plan path.
            Command::GoalReplace { .. } | Command::GoalFix { .. } => InterventionKind::Replan,
            // Sent into a running turn, the hold's text is steering.
            Command::UntilDone { .. } => InterventionKind::Steer,
            Command::Approve { .. } => InterventionKind::Approve,
            Command::Reject { .. } => InterventionKind::Reject,
        }
    }

    /// The text the command carries, for the parts that become a request.
    pub fn text(&self) -> Option<&str> {
        match self {
            Command::Replan { text }
            | Command::AddRequirement { text }
            | Command::RemoveRequirement { text }
            | Command::Reprioritize { text }
            | Command::GoalReplace { text }
            | Command::GoalFix { text }
            | Command::UntilDone { text } => Some(text),
            _ => None,
        }
    }
}

/// Recognise an explicit command in a human message.
///
/// Slash commands are matched on the first token, case-insensitively.
/// Bare words are matched only when the *whole* message, less trailing
/// punctuation, is exactly one of `stop`, `cancel`, `pause`, `resume`,
/// `status` — so "stop" and "Stop!" cancel, and "stop using semicolons"
/// is steering text. Everything else is `None`.
pub fn parse_command(text: &str) -> Option<Command> {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('/') {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let verb = parts.next()?.to_ascii_lowercase();
        let arg = parts.next().map(str::trim).unwrap_or("").to_string();
        let needs_arg = |arg: &str| (!arg.is_empty()).then(|| arg.to_string());
        return match verb.as_str() {
            "status" => Some(Command::Status),
            "pause" | "hold" => Some(Command::Pause),
            "resume" | "continue" => Some(Command::Resume),
            "stop" | "cancel" | "abort" => Some(Command::Cancel),
            "replan" => needs_arg(&arg).map(|text| Command::Replan { text }),
            "add" => needs_arg(&arg).map(|text| Command::AddRequirement { text }),
            "drop" | "remove" => needs_arg(&arg).map(|text| Command::RemoveRequirement { text }),
            "prioritize" | "prioritise" | "reprioritize" => {
                needs_arg(&arg).map(|text| Command::Reprioritize { text })
            }
            "goal" => {
                let mut sub = arg.splitn(2, char::is_whitespace);
                let which = sub.next().unwrap_or("").to_ascii_lowercase();
                let text = sub.next().map(str::trim).unwrap_or("");
                match (which.as_str(), needs_arg(text)) {
                    ("replace", Some(text)) => Some(Command::GoalReplace { text }),
                    ("fix", Some(text)) => Some(Command::GoalFix { text }),
                    _ => None,
                }
            }
            "until-done" | "until_done" | "untildone" => {
                needs_arg(&arg).map(|text| Command::UntilDone { text })
            }
            "approve" => needs_arg(&arg).map(|gate_id| Command::Approve { gate_id }),
            "reject" => needs_arg(&arg).map(|gate_id| Command::Reject { gate_id }),
            _ => None,
        };
    }
    let bare = trimmed
        .trim_end_matches(['.', '!', '?'])
        .trim()
        .to_ascii_lowercase();
    match bare.as_str() {
        "status" => Some(Command::Status),
        "pause" => Some(Command::Pause),
        "resume" => Some(Command::Resume),
        "stop" | "cancel" => Some(Command::Cancel),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionDecision {
    Accepted,
    Queued,
    RequiresHuman,
    Rejected,
}

impl InterventionDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Queued => "queued",
            Self::RequiresHuman => "requires_human",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionRequest {
    pub request_id: String,
    pub kind: InterventionKind,
    pub text: String,
    pub source: ControlSource,
    pub target_revision: Option<u64>,
    /// The session the intervention is aimed at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_session_id: Option<String>,
    /// The session that dispatched the target, from the target's own header.
    /// An agent may control a session only when it is that parent: "own
    /// children only" is a fact about the target, not about the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_parent_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterventionEvaluation {
    pub request: InterventionRequest,
    pub decision: InterventionDecision,
    pub reason: String,
    pub creates_revision: bool,
}

/// Evaluate the control-plane handling of an intervention.
///
/// | kind | human | agent | system |
/// |---|---|---|---|
/// | status | accepted | accepted | accepted |
/// | resume | accepted | own subtree | accepted |
/// | pause / cancel | accepted | own children only | rejected |
/// | steer | queued | queued (typed message) | rejected |
/// | replan / add / drop / prioritize | queued, new revision | requires human | rejected |
/// | approve / reject | accepted | rejected | rejected |
///
/// This deliberately does not inspect or grant permissions; effectful changes
/// still go through the permission engine and approval gates.
pub fn evaluate_intervention(request: InterventionRequest) -> InterventionEvaluation {
    use InterventionDecision as D;
    use InterventionKind as K;
    // Its own child: the target names this agent as the session that
    // dispatched it. Any other session — a sibling, another agent's run, a
    // person's — is not the agent's to stop.
    let own_subtree = match &request.source {
        ControlSource::Agent { session_id, .. } => {
            request.target_parent_session_id.as_deref() == Some(session_id.as_str())
                && request
                    .target_session_id
                    .as_deref()
                    .is_some_and(|target| target != session_id)
        }
        _ => false,
    };
    let (decision, reason, creates_revision): (D, &str, bool) =
        match (&request.kind, &request.source) {
            (K::Status, _) => (D::Accepted, "status is observational", false),
            (K::Resume, ControlSource::Human { .. } | ControlSource::System { .. }) => (
                D::Accepted,
                "resume continues at the next safe boundary",
                false,
            ),
            (K::Resume, ControlSource::Agent { .. }) if own_subtree => {
                (D::Accepted, "an agent may resume work it dispatched", false)
            }
            (K::Pause, ControlSource::Human { .. }) => {
                (D::Accepted, "pause preserves partial work", false)
            }
            (K::Cancel, ControlSource::Human { .. }) => {
                (D::Accepted, "cancellation is fail-safe", false)
            }
            (K::Pause | K::Cancel, ControlSource::Agent { .. }) if own_subtree => (
                D::Accepted,
                "an agent may pause or cancel work it dispatched",
                false,
            ),
            (K::Pause | K::Cancel | K::Resume, ControlSource::Agent { .. }) => (
                D::Rejected,
                "an agent may only control its own children",
                false,
            ),
            (K::Pause | K::Cancel, ControlSource::System { .. }) => (
                D::Rejected,
                "an external system cannot stop a human's run",
                false,
            ),
            (K::Steer, ControlSource::Human { .. } | ControlSource::Agent { .. }) => (
                D::Queued,
                "steering queued at the next safe boundary",
                false,
            ),
            (K::Steer, ControlSource::System { .. }) => {
                (D::Rejected, "an external system cannot steer a run", false)
            }
            (
                K::Replan | K::Reprioritize | K::AddRequirement | K::RemoveRequirement,
                ControlSource::Human { .. },
            ) => (
                D::Queued,
                "scope change is queued for a new plan revision",
                true,
            ),
            (
                K::Replan | K::Reprioritize | K::AddRequirement | K::RemoveRequirement,
                ControlSource::Agent { .. },
            ) => (
                D::RequiresHuman,
                "scope changes proposed by an agent require human review",
                false,
            ),
            (
                K::Replan | K::Reprioritize | K::AddRequirement | K::RemoveRequirement,
                ControlSource::System { .. },
            ) => (D::Rejected, "an external system cannot change scope", false),
            (K::Approve | K::Reject, ControlSource::Human { .. }) => {
                (D::Accepted, "human control-plane decision recorded", false)
            }
            (K::Approve | K::Reject, _) => (D::Rejected, "only a human can resolve a gate", false),
        };
    InterventionEvaluation {
        request,
        decision,
        reason: reason.into(),
        creates_revision,
    }
}

use crate::{Evidence, Reading};

/// Whether a requirement came from the request or was inferred by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementOrigin {
    Explicit,
    Inferred,
    /// Declared by the model in its plan for the turn
    /// (docs/design/86-model-plans-runtime-checks.md §2.1).
    Declared,
}

/// How a declared deliverable reaches the person, and so which receipt shows
/// it was produced (docs/design/86-model-plans-runtime-checks.md §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliverableForm {
    /// A presentation card recorded this turn.
    Card,
    /// The turn's final text.
    Text,
    /// A file delivered for review.
    File,
    /// A change made in the workspace or outside Vak.
    Change,
    /// Something run to prove a result, after the last change.
    Check,
}

impl DeliverableForm {
    pub const ALL: [DeliverableForm; 5] = [
        DeliverableForm::Card,
        DeliverableForm::Text,
        DeliverableForm::File,
        DeliverableForm::Change,
        DeliverableForm::Check,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DeliverableForm::Card => "card",
            DeliverableForm::Text => "text",
            DeliverableForm::File => "file",
            DeliverableForm::Change => "change",
            DeliverableForm::Check => "check",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|form| form.as_str().eq_ignore_ascii_case(text.trim()))
    }
}

/// How strongly a requirement affects completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementImportance {
    Must,
    Prefer,
}

/// The kind of result a requirement concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    Deliverable,
    Evidence,
    Constraint,
    Integrity,
}

/// A checkable expectation attached to one outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeRequirement {
    pub id: String,
    pub kind: RequirementKind,
    pub description: String,
    pub origin: RequirementOrigin,
    pub importance: RequirementImportance,
    #[serde(default)]
    pub target: Option<String>,
    /// For a deliverable the model declared, how it is delivered.
    #[serde(default)]
    pub form: Option<DeliverableForm>,
}

/// The user's requested result as understood at turn admission.
///
/// This is an interpretation record, not an authority grant. It may guide
/// execution and presentation, but permissions and evidence strength remain
/// owned by their existing runtime boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeSpec {
    pub schema_version: u16,
    /// Monotonic contract revision within a session. Revision zero is the
    /// admission baseline; revisions are append-only and never rewritten.
    #[serde(default)]
    pub revision: u64,
    pub objective: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub requirements: Vec<OutcomeRequirement>,
    pub resolver_version: u32,
    #[serde(default)]
    pub evidence_max_age_secs: Option<i64>,
    /// Maximum model turns admitted for this outcome, when the engagement
    /// resolver derived a cap.
    #[serde(default)]
    pub max_turns: Option<usize>,
    /// The primary act of every part of the request, typed. What the stop
    /// gate reasons from — never the requirement descriptions, which are
    /// prose for people and, for merged requirements, prose from extensions.
    /// Contender acts are not here: a contender is a noun that is a verb
    /// somewhere ("deploys" in a question), and gating completion on it
    /// demanded an execution receipt from an answer.
    #[serde(default)]
    pub acts: BTreeSet<Act>,
    /// When the loop may stop, from the engagement. The stop gate reads this
    /// first and the acts second.
    #[serde(default)]
    pub stop: crate::StopProfile,
}

/// One deliverable the model said it will produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deliverable {
    pub name: String,
    pub form: DeliverableForm,
}

/// Runtime status of the primary deliverable. Produced output is not itself
/// proof that every requirement was satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeStatus {
    Produced,
    Failed,
    Cancelled,
    Unknown,
}

/// Aggregate runtime verdict. This is derived from runtime evidence and
/// requirement evaluations; model prose cannot set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionVerdict {
    Complete,
    Partial,
    Unknown,
    Failed,
    Cancelled,
}

/// Runtime verdict for one requirement. `Unknown` is intentionally available
/// when a structural check cannot establish semantic correctness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Met,
    Unmet,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    None,
    Fresh,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceReceipt {
    pub id: String,
    pub kind: String,
    pub producer: String,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    /// When the underlying source was published or last materially updated.
    /// This is distinct from retrieval/observation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_published_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When the fact or event actually occurred, if different from retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_at: Option<chrono::DateTime<chrono::Utc>>,
    pub state: EvidenceState,
}

impl EvidenceReceipt {
    /// Timestamp appropriate for freshness checks. Requirement evaluators can
    /// choose publication/effective time explicitly; observation remains the
    /// safe default for legacy receipts.
    pub fn freshness_timestamp(&self) -> chrono::DateTime<chrono::Utc> {
        self.source_published_at
            .or(self.effective_at)
            .unwrap_or(self.observed_at)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementEvaluation {
    pub requirement_id: String,
    pub status: RequirementStatus,
    pub reason: String,
}

impl OutcomeSpec {
    /// A named file deliverable cannot be established by prose alone. This
    /// conservative signal only affects outcome assessment; it grants no tool.
    ///
    /// Only a request with a part that produces something — authoring,
    /// modifying, operating — can owe a file: "explain how to write a
    /// README.md" names a file and asks for an explanation, and demanding a
    /// write for it made the stop gate send a correct answer back for a
    /// file nobody asked for.
    pub fn saved_file_target(&self) -> Option<String> {
        if !self
            .acts
            .iter()
            .any(|act| matches!(act, Act::Author | Act::Modify | Act::Operate))
        {
            return None;
        }
        let request = self.objective.to_ascii_lowercase();
        let asks_to_write = [
            "create ",
            "write ",
            "save ",
            "generate ",
            "make ",
            "build ",
            "export ",
        ]
        .iter()
        .any(|verb| request.contains(verb));
        if !asks_to_write {
            return None;
        }
        request.split_whitespace().find_map(|word| {
            let token = word.trim_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '.' && c != '_' && c != '-' && c != '/'
            });
            let is_file = [
                ".html", ".htm", ".md", ".txt", ".json", ".csv", ".pdf", ".docx", ".pptx", ".xlsx",
                ".svg", ".png", ".js", ".ts", ".tsx", ".rs", ".py", ".css", ".sql",
            ]
            .iter()
            .any(|extension| token.ends_with(extension) && token.len() > extension.len());
            is_file.then(|| token.to_string())
        })
    }

    pub fn expects_saved_file(&self) -> bool {
        self.saved_file_target().is_some()
    }

    /// Whether a text file is an explicitly requested output of an authoring
    /// turn. A plan, list, or other authored answer is delivered in the
    /// conversation unless the request names a file or directly asks to save
    /// one. Effectful acts such as modifying an existing workspace file keep
    /// their normal write path.
    pub fn permits_authored_text_file(&self) -> bool {
        if self.expects_saved_file() || self.acts.iter().any(|act| act.requires_execution()) {
            return true;
        }
        if !self.acts.contains(&Act::Author) {
            return true;
        }

        let words: Vec<String> = self
            .objective
            .to_ascii_lowercase()
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_string)
            .collect();
        let output_verbs = ["write", "create", "make", "generate", "save", "export"];
        let file_targets = ["file", "filename", "disk", "workspace"];
        let negation = ["not", "never", "avoid", "dont", "don", "no"];
        words.iter().enumerate().any(|(at, word)| {
            output_verbs.contains(&word.as_str())
                && !words[at.saturating_sub(2)..at]
                    .iter()
                    .any(|previous| negation.contains(&previous.as_str()))
                && words
                    .iter()
                    .skip(at + 1)
                    .take(6)
                    .any(|next| file_targets.contains(&next.as_str()))
        })
    }

    /// Build the conservative baseline contract for an ordinary turn.
    ///
    /// The request text is preserved as the objective; inferred requirements
    /// never grant tools or claim that evidence exists.
    pub fn from_reading(
        objective: impl Into<String>,
        reading: &Reading,
        resolver_version: u32,
    ) -> Self {
        let mut requirements = vec![OutcomeRequirement {
            id: "deliverable-1".into(),
            kind: RequirementKind::Deliverable,
            description: format!("produce an {} result", reading.act.as_str()),
            origin: RequirementOrigin::Inferred,
            importance: RequirementImportance::Must,
            target: Some("primary".into()),
            form: None,
        }];
        if reading.evidence != Evidence::None {
            requirements.push(OutcomeRequirement {
                id: "evidence-1".into(),
                kind: RequirementKind::Evidence,
                description: format!("meet the {} evidence standard", reading.evidence.as_str()),
                origin: RequirementOrigin::Inferred,
                importance: RequirementImportance::Must,
                target: Some("primary".into()),
                form: None,
            });
        }
        OutcomeSpec {
            schema_version: 1,
            revision: 0,
            objective: objective.into(),
            assumptions: Vec::new(),
            requirements,
            resolver_version,
            evidence_max_age_secs: match reading.evidence {
                Evidence::None => None,
                Evidence::Cited => Some(86_400),
                Evidence::Verified | Evidence::Audited => Some(3_600),
            },
            max_turns: None,
            acts: BTreeSet::from([reading.act]),
            stop: crate::StopProfile::default(),
        }
    }

    /// The baseline contract with the engagement's own stop rule and the
    /// primary act of every part. A reading never sets `max_turns`: the turn
    /// budget is the operator's, whatever the request looked like.
    pub fn from_intent(objective: impl Into<String>, intent: &crate::Intent) -> Self {
        let mut spec = Self::from_reading(
            objective,
            &intent.reading,
            intent.provenance.resolver_version,
        );
        spec.stop = intent.engagement.posture.stop;
        spec.acts
            .extend(intent.strands.iter().map(|strand| strand.reading.act));
        spec
    }

    /// Whether this outcome requires execution or file modifications.
    ///
    /// True when an act genuinely needs one (`Act::requires_execution`:
    /// `Modify`/`Operate`/`Govern`/`Verify`), or when the request names a
    /// file deliverable outright (`expects_saved_file`) — the only case
    /// where authoring content also demands a receipt, since `Act` alone has
    /// no view of the request text.
    pub fn requires_execution(&self) -> bool {
        self.expects_saved_file() || self.acts.iter().any(|act| act.requires_execution())
    }

    /// Whether the requested primary output is a file-backed Office artifact.
    /// A bare Author act also covers prose answers such as letters and plans;
    /// require a file receipt only when the request names a file or an Office
    /// format. This lets "make an Excel document" require a workbook without
    /// making every authored sentence produce a file.
    pub fn expects_artifact(&self) -> bool {
        if self.expects_saved_file() {
            return true;
        }
        let objective = self.objective.to_ascii_lowercase();
        let requests_office_file = [
            "spreadsheet",
            "workbook",
            "excel",
            "xlsx",
            "word document",
            "word file",
            "docx",
            "document",
            "powerpoint",
            "power point",
            "slide deck",
            "slides",
            "pptx",
            "pdf",
        ]
        .iter()
        .any(|term| objective.contains(term));
        requests_office_file
            && self
                .acts
                .iter()
                .any(|act| matches!(act, Act::Author | Act::Modify | Act::Operate))
    }

    /// Whether this outcome requires inspection, search, or enumeration.
    pub fn requires_inspection(&self) -> bool {
        self.acts.iter().any(|act| act.requires_inspection())
    }

    /// Whether this outcome requires real tool execution or evidence receipts.
    pub fn requires_tool(&self) -> bool {
        self.expects_saved_file()
            || self.acts.iter().any(|act| act.requires_tool())
            || self
                .requirements
                .iter()
                .any(|r| r.kind == RequirementKind::Evidence)
    }

    /// The act to name in logs and nudges: the most demanding one.
    pub fn deliverable_act(&self) -> Option<&str> {
        self.acts
            .iter()
            .max_by_key(|act| {
                (
                    act.is_effectful(),
                    act.requires_execution(),
                    act.requires_inspection(),
                )
            })
            .map(|act| act.as_str())
    }

    /// Merge an extension-provided requirement without allowing it to alter
    /// authority. Invalid declarations are rejected at the contract boundary.
    pub fn merge_declared_requirement(
        &mut self,
        id: impl Into<String>,
        kind: &str,
        description: impl Into<String>,
        importance: &str,
        target: Option<String>,
    ) -> Result<(), String> {
        let kind = match kind {
            "deliverable" => RequirementKind::Deliverable,
            "evidence" => RequirementKind::Evidence,
            "constraint" => RequirementKind::Constraint,
            "integrity" => RequirementKind::Integrity,
            other => return Err(format!("unsupported outcome requirement kind: {other}")),
        };
        let importance = match importance {
            "" | "prefer" => RequirementImportance::Prefer,
            "must" => RequirementImportance::Must,
            other => {
                return Err(format!(
                    "unsupported outcome requirement importance: {other}"
                ));
            }
        };
        let id = id.into();
        let description = description.into();
        if id.trim().is_empty() || description.trim().is_empty() {
            return Err("outcome requirement id and description are required".into());
        }
        if self
            .requirements
            .iter()
            .any(|requirement| requirement.id == id)
        {
            return Err(format!("duplicate outcome requirement id: {id}"));
        }
        self.requirements.push(OutcomeRequirement {
            id,
            kind,
            description,
            origin: RequirementOrigin::Inferred,
            importance,
            target,
            form: None,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::{Act, Reading};

    #[test]
    fn named_saved_file_requires_evidence_but_an_inline_plan_does_not() {
        let authoring = Reading {
            act: Act::Author,
            ..Reading::general()
        };
        let file = OutcomeSpec::from_reading(
            "Create an invitation in this workspace as invitation.html",
            &authoring,
            1,
        );
        assert!(file.expects_saved_file());
        let plan = OutcomeSpec::from_reading("Create a two-day lunch plan", &authoring, 1);
        assert!(!plan.expects_saved_file());
        assert!(!plan.expects_artifact());
        let note = OutcomeSpec::from_reading("Write a short note to my neighbour", &authoring, 1);
        assert!(!note.expects_artifact());
        let native_cards = OutcomeSpec::from_reading(
            "Make a practical plan and use suitable native presentation cards",
            &authoring,
            1,
        );
        assert!(!native_cards.expects_artifact());
        for request in [
            "Make an Excel document",
            "Create a spreadsheet dashboard",
            "Write a Word document",
            "Build a PowerPoint presentation",
            "Create a slide deck for the team",
            "Generate a PDF report",
        ] {
            let output = OutcomeSpec::from_reading(request, &authoring, 1);
            assert!(output.expects_artifact(), "{request}");
        }
        let inspection = OutcomeSpec::from_reading("Explain README.md", &Reading::general(), 1);
        assert!(!inspection.expects_saved_file());
        assert!(!inspection.expects_artifact());
        // Naming a file inside a question is not asking for one.
        let explanation =
            OutcomeSpec::from_reading("explain how to write a README.md", &Reading::general(), 1);
        assert!(!explanation.expects_saved_file());
        assert!(!explanation.requires_execution());
    }

    #[test]
    fn authored_text_file_permission_tracks_the_requested_outcome() {
        let authoring = Reading {
            act: Act::Author,
            ..Reading::general()
        };
        let plan =
            OutcomeSpec::from_reading("Make an easy-to-share plan and supply list", &authoring, 1);
        assert!(!plan.permits_authored_text_file());
        let declined_file =
            OutcomeSpec::from_reading("Do not create a file for this", &authoring, 1);
        assert!(!declined_file.permits_authored_text_file());

        for request in [
            "Write a file with the plan",
            "Save this to a file",
            "Create a file for the supply list",
            "Save the notes in the workspace",
            "Write the plan to workshop.md",
        ] {
            assert!(
                OutcomeSpec::from_reading(request, &authoring, 1).permits_authored_text_file(),
                "{request}"
            );
        }

        let modification = OutcomeSpec::from_reading(
            "Update the existing notes",
            &Reading {
                act: Act::Modify,
                ..Reading::general()
            },
            1,
        );
        assert!(modification.permits_authored_text_file());
    }

    #[test]
    fn baseline_contract_preserves_objective_and_adds_only_inferred_requirements() {
        let reading = Reading {
            act: Act::Answer,
            evidence: Evidence::Cited,
            ..Reading::general()
        };
        let spec = OutcomeSpec::from_reading("current events in India", &reading, 1);
        assert_eq!(spec.objective, "current events in India");
        assert!(spec.requirements.iter().any(|requirement| {
            requirement.kind == RequirementKind::Evidence
                && requirement.origin == RequirementOrigin::Inferred
                && requirement.importance == RequirementImportance::Must
        }));
        assert!(
            spec.requirements
                .iter()
                .all(|requirement| { requirement.origin == RequirementOrigin::Inferred })
        );
    }

    #[test]
    fn extension_requirements_are_data_driven_and_narrowing_only() {
        let mut spec = OutcomeSpec::from_reading("make a plan", &Reading::general(), 1);
        let result = spec.merge_declared_requirement(
            "plan-structure",
            "constraint",
            "include assumptions and next steps",
            "must",
            Some("primary".into()),
        );
        assert!(result.is_ok());
        assert_eq!(
            spec.requirements.last().map(|item| item.origin),
            Some(RequirementOrigin::Inferred)
        );
        assert!(
            spec.merge_declared_requirement("", "constraint", "x", "must", None)
                .is_err()
        );
        assert!(
            spec.merge_declared_requirement("bad", "grant", "x", "must", None)
                .is_err()
        );
    }

    #[test]
    fn only_explicit_commands_are_control() {
        assert_eq!(parse_command("/status"), Some(Command::Status));
        assert_eq!(parse_command("/stop"), Some(Command::Cancel));
        assert_eq!(parse_command("Stop!"), Some(Command::Cancel));
        assert_eq!(parse_command("pause"), Some(Command::Pause));
        assert_eq!(
            parse_command("/replan around the new constraint"),
            Some(Command::Replan {
                text: "around the new constraint".into()
            })
        );
        assert_eq!(
            parse_command("/goal replace ship the index only"),
            Some(Command::GoalReplace {
                text: "ship the index only".into()
            })
        );
        assert_eq!(
            parse_command("/until-done tighten the essay"),
            Some(Command::UntilDone {
                text: "tighten the essay".into()
            })
        );
        assert_eq!(parse_command("/until-done"), None);
        assert_eq!(
            parse_command("/approve gate-7"),
            Some(Command::Approve {
                gate_id: "gate-7".into()
            })
        );
        // Natural language is steering, whatever word it starts with.
        for text in [
            "stop using semicolons in the output",
            "pause the music service before deploying",
            "also include a CSV",
            "status of the migration please",
            "replace the deprecated API call",
            "use a shorter answer",
        ] {
            assert_eq!(parse_command(text), None, "{text}");
        }
        // A slash command without its argument is not a command either.
        assert_eq!(parse_command("/replan"), None);
        assert_eq!(parse_command("/goal replace"), None);
    }

    fn intervention(kind: InterventionKind, source: ControlSource) -> InterventionRequest {
        InterventionRequest {
            request_id: "i-1".into(),
            kind,
            text: String::new(),
            source,
            target_revision: Some(1),
            target_session_id: Some("child".into()),
            target_parent_session_id: Some("parent".into()),
        }
    }

    fn human() -> ControlSource {
        ControlSource::Human {
            surface: "desktop".into(),
            principal: None,
        }
    }

    fn agent(session_id: &str) -> ControlSource {
        ControlSource::Agent {
            session_id: session_id.into(),
            parent_session_id: None,
        }
    }

    fn system() -> ControlSource {
        ControlSource::System {
            origin: "webhook".into(),
        }
    }

    /// The authority matrix: humans may do anything, agents only their own
    /// children, systems only observe.
    #[test]
    fn intervention_authority_comes_from_the_source() {
        use InterventionDecision as D;
        use InterventionKind as K;
        let decide = |kind: K, source: ControlSource| {
            evaluate_intervention(intervention(kind, source)).decision
        };

        assert_eq!(decide(K::Cancel, human()), D::Accepted);
        assert_eq!(decide(K::Replan, human()), D::Queued);
        assert_eq!(decide(K::Approve, human()), D::Accepted);
        assert!(evaluate_intervention(intervention(K::Replan, human())).creates_revision);

        // An agent controlling a child it dispatched.
        assert_eq!(decide(K::Cancel, agent("parent")), D::Accepted);
        // An agent trying to control the session it lives in — or one that
        // is not its child — is refused.
        assert_eq!(decide(K::Cancel, agent("child")), D::Rejected);
        assert_eq!(decide(K::Cancel, agent("sibling")), D::Rejected);
        assert_eq!(decide(K::Pause, agent("stranger")), D::Rejected);
        assert_eq!(decide(K::Resume, agent("stranger")), D::Rejected);
        // Without the target's parentage there is nothing to prove, so the
        // answer is no.
        let mut unknown_parent = intervention(K::Cancel, agent("parent"));
        unknown_parent.target_parent_session_id = None;
        assert_eq!(evaluate_intervention(unknown_parent).decision, D::Rejected);
        assert_eq!(decide(K::Replan, agent("parent")), D::RequiresHuman);
        assert_eq!(decide(K::Approve, agent("parent")), D::Rejected);

        assert_eq!(decide(K::Status, system()), D::Accepted);
        assert_eq!(decide(K::Resume, system()), D::Accepted);
        assert_eq!(decide(K::Cancel, system()), D::Rejected);
        assert_eq!(decide(K::Steer, system()), D::Rejected);
        assert_eq!(decide(K::Approve, system()), D::Rejected);
    }
}
