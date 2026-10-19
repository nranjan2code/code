//! The control plane's decisions (docs/design/88-control-plane.md).
//!
//! The model proposes a plan, a call and an answer; the runtime decides at a
//! closed set of decision points, from recorded facts, and every decision is
//! one of the typed values here. Nothing in this crate reads text, holds a
//! session, or calls a provider.

use serde::{Deserialize, Serialize};

pub mod correct;
pub mod provenance;

pub use provenance::{Held, Provenance};

pub use correct::{
    Finding, Ladder, RECOVERY_NOTE, ShownTool, card_ability_not_answered, card_command_not_run,
    card_data_not_found, card_file_missing, card_files_unchanged, card_kind_elsewhere,
    card_replaces_unknown, card_unnamed, card_unplanned, card_without_plan, limit_reached,
    regenerate_arguments, render, skill_not_tool, uncited_source, unknown_tool,
};

/// Who made a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decider {
    Runtime,
    Person,
    Provider,
}

/// A recorded fact a decision was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Evidence {
    /// A ledger entry, by its id.
    Entry(String),
    /// A tool call, by its `tool_use_id`.
    Call(String),
    /// A recorded presentation, by its id.
    Presentation(String),
}

/// The closed set of decision points (doc 88 §3). Adding one is a code
/// change, on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionPoint {
    AdmitTurn,
    FrameStep,
    AcceptCall,
    AcceptAnswer,
    Correct,
    EndTurn,
    Promote,
}

/// One decision: who made it, from what, and what it chose. The point is
/// the choice's own tag, so a record cannot name one point and carry
/// another's output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub decider: Decider,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    pub choice: Choice,
}

impl Decision {
    pub fn by_runtime(choice: Choice) -> Self {
        Self {
            decider: Decider::Runtime,
            evidence: Vec::new(),
            choice,
        }
    }

    pub fn with(mut self, evidence: Evidence) -> Self {
        self.evidence.push(evidence);
        self
    }

    pub fn point(&self) -> DecisionPoint {
        self.choice.point()
    }
}

/// What a decision point chose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "point", rename_all = "snake_case")]
pub enum Choice {
    AdmitTurn(Admission),
    FrameStep(Framing),
    AcceptCall(CallVerdict),
    AcceptAnswer(AnswerVerdict),
    Correct(Correction),
    EndTurn(Ending),
    Promote(Proposal),
}

impl Choice {
    pub fn point(&self) -> DecisionPoint {
        match self {
            Self::AdmitTurn(_) => DecisionPoint::AdmitTurn,
            Self::FrameStep(_) => DecisionPoint::FrameStep,
            Self::AcceptCall(_) => DecisionPoint::AcceptCall,
            Self::AcceptAnswer(_) => DecisionPoint::AcceptAnswer,
            Self::Correct(_) => DecisionPoint::Correct,
            Self::EndTurn(_) => DecisionPoint::EndTurn,
            Self::Promote(_) => DecisionPoint::Promote,
        }
    }
}

/// The turn started, under this Agent, on this model, bounded by the
/// operator's step limit. The route each step was carried on is its
/// receipt's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admission {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub model: String,
    pub max_steps: u32,
}

/// What a step is for. A fact the runtime has, never a reading of the
/// request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Plan,
    Act,
    Answer,
    Repair,
    Compact,
    Judge,
    Summarise,
}

/// How much the model reasons on a step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "depth", rename_all = "snake_case")]
pub enum Reasoning {
    /// The model's own default; nothing asked.
    #[default]
    Default,
    Off,
    Depth(Depth),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

/// Sampling settings; `None` leaves each to the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Sampling {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

/// The settings a request carries for one step: how much the model
/// reasons and how it samples, and whether its tools' arguments are
/// enforced by the wire (docs/design/89-constrained-tool-arguments.md).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct StepSettings {
    pub reasoning: Reasoning,
    #[serde(default)]
    pub sampling: Sampling,
    /// The tools were sent in the wire's strict form, so the provider
    /// enforces their schemas while the model writes the arguments.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strict_tools: bool,
    /// The model's output was asked for in a schema the wire enforces
    /// (structured output).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub schema_output: bool,
}

/// Whether a model takes one setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Yes,
    No,
    /// Nobody reported it and no request has refused it yet: sent, and
    /// learned from the provider's answer.
    #[default]
    Unknown,
}

impl Support {
    pub fn sends(self) -> bool {
        self != Self::No
    }

    /// The narrower of two reports: a `No` from either side wins.
    pub fn meet(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, _) | (_, Self::Yes) => Self::Yes,
            _ => Self::Unknown,
        }
    }
}

/// How a model's reasoning can be set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSupport {
    #[default]
    Unknown,
    /// The model does not reason, or the wire cannot set it.
    Fixed,
    /// Reasoning can be turned on or off, not graded.
    Toggle,
    /// Reasoning takes a depth.
    Graded,
}

/// What a model on a wire can honour (doc 88 §4): what the adapter can pass
/// through, narrowed by what the provider reported and what a request it
/// refused showed. Nothing here names a model or a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub reasoning: ReasoningSupport,
    pub temperature: Support,
    pub top_p: Support,
    pub top_k: Support,
    /// Whether the wire can be told to call one named tool.
    #[serde(default)]
    pub tool_choice: Support,
    /// Whether the wire enforces a tool's schema on the arguments the
    /// model writes (strict tools; docs/design/89 §4.1).
    #[serde(default)]
    pub strict_tools: Support,
    /// Whether the wire enforces a schema on the model's output itself
    /// (structured output; docs/design/89 §4.3).
    #[serde(default)]
    pub schema_output: Support,
    /// Whether the wire hands back a call to a tool the request did not
    /// list. Only where it does can a tool stay off the wire until the
    /// model calls it by name (docs/design/68-context-engine.md §5); where
    /// it does not, or nobody knows, every admitted tool is sent.
    #[serde(default)]
    pub unlisted_calls: Support,
}

impl ModelCapabilities {
    /// The settings this model can honour of those asked: what it cannot
    /// take is left to the model's default, never approximated.
    pub fn honour(&self, asked: StepSettings) -> StepSettings {
        let reasoning = match (self.reasoning, asked.reasoning) {
            (ReasoningSupport::Fixed, _) => Reasoning::Default,
            (ReasoningSupport::Toggle, Reasoning::Depth(_)) => Reasoning::Default,
            (_, reasoning) => reasoning,
        };
        let keep = |support: Support, value| if support.sends() { value } else { None };
        StepSettings {
            strict_tools: asked.strict_tools && self.strict_tools.sends(),
            schema_output: asked.schema_output && self.schema_output.sends(),
            reasoning,
            sampling: Sampling {
                temperature: keep(self.temperature, asked.sampling.temperature),
                top_p: keep(self.top_p, asked.sampling.top_p),
                top_k: if self.top_k.sends() {
                    asked.sampling.top_k
                } else {
                    None
                },
            },
        }
    }

    /// These capabilities with every setting `sent` carried marked as not
    /// taken: what a request refused for its settings, and accepted
    /// without them, showed.
    pub fn refusing(mut self, sent: StepSettings) -> Self {
        if sent.reasoning != Reasoning::Default {
            self.reasoning = ReasoningSupport::Fixed;
        }
        if sent.sampling.temperature.is_some() {
            self.temperature = Support::No;
        }
        if sent.sampling.top_p.is_some() {
            self.top_p = Support::No;
        }
        if sent.sampling.top_k.is_some() {
            self.top_k = Support::No;
        }
        if sent.strict_tools {
            self.strict_tools = Support::No;
        }
        if sent.schema_output {
            self.schema_output = Support::No;
        }
        self
    }

    /// The narrower of two reports, setting by setting.
    pub fn meet(self, other: Self) -> Self {
        use ReasoningSupport::*;
        let reasoning = match (self.reasoning, other.reasoning) {
            (Fixed, _) | (_, Fixed) => Fixed,
            (Toggle, _) | (_, Toggle) => Toggle,
            (Graded, _) | (_, Graded) => Graded,
            _ => Unknown,
        };
        Self {
            reasoning,
            temperature: self.temperature.meet(other.temperature),
            top_p: self.top_p.meet(other.top_p),
            top_k: self.top_k.meet(other.top_k),
            tool_choice: self.tool_choice.meet(other.tool_choice),
            strict_tools: self.strict_tools.meet(other.strict_tools),
            schema_output: self.schema_output.meet(other.schema_output),
            unlisted_calls: self.unlisted_calls.meet(other.unlisted_calls),
        }
    }
}

/// What an adapter sent for one dispatch, and what it knew of the model
/// when it did.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Applied {
    pub settings: StepSettings,
    pub capabilities: ModelCapabilities,
    /// How the step's required tool was put to the provider.
    #[serde(default)]
    pub forced: Forced,
}

/// How a step's required tool (`OutputForm::Tool`) reached the provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Forced {
    /// The step required none.
    #[default]
    NotAsked,
    /// The provider was told to call it.
    Choice,
    /// The wire has no such switch, so only that tool was offered
    /// (docs/design/88-control-plane.md §12.3).
    OnlyOffered,
}

/// Every purpose, in a fixed order.
pub const PURPOSES: [Purpose; 7] = [
    Purpose::Plan,
    Purpose::Act,
    Purpose::Answer,
    Purpose::Repair,
    Purpose::Compact,
    Purpose::Judge,
    Purpose::Summarise,
];

/// A reasoning setting as configuration spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningSetting {
    Default,
    Off,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl From<ReasoningSetting> for Reasoning {
    fn from(setting: ReasoningSetting) -> Self {
        match setting {
            ReasoningSetting::Default => Reasoning::Default,
            ReasoningSetting::Off => Reasoning::Off,
            ReasoningSetting::Low => Reasoning::Depth(Depth::Low),
            ReasoningSetting::Medium => Reasoning::Depth(Depth::Medium),
            ReasoningSetting::High => Reasoning::Depth(Depth::High),
            ReasoningSetting::Xhigh => Reasoning::Depth(Depth::XHigh),
            ReasoningSetting::Max => Reasoning::Depth(Depth::Max),
        }
    }
}

impl From<Reasoning> for ReasoningSetting {
    fn from(reasoning: Reasoning) -> Self {
        match reasoning {
            Reasoning::Default => Self::Default,
            Reasoning::Off => Self::Off,
            Reasoning::Depth(Depth::Low) => Self::Low,
            Reasoning::Depth(Depth::Medium) => Self::Medium,
            Reasoning::Depth(Depth::High) => Self::High,
            Reasoning::Depth(Depth::XHigh) => Self::Xhigh,
            Reasoning::Depth(Depth::Max) => Self::Max,
        }
    }
}

/// One purpose's settings in a configuration layer: a field set there
/// overrides the layer below, an unset one inherits it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningSetting>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

impl StepOverride {
    /// This layer over `lower`: each field this one sets wins.
    pub fn over(self, lower: Self) -> Self {
        Self {
            reasoning: self.reasoning.or(lower.reasoning),
            temperature: self.temperature.or(lower.temperature),
            top_p: self.top_p.or(lower.top_p),
            top_k: self.top_k.or(lower.top_k),
        }
    }

    fn apply(self, settings: StepSettings) -> StepSettings {
        StepSettings {
            strict_tools: settings.strict_tools,
            schema_output: settings.schema_output,
            reasoning: self.reasoning.map_or(settings.reasoning, Reasoning::from),
            sampling: Sampling {
                temperature: self.temperature.or(settings.sampling.temperature),
                top_p: self.top_p.or(settings.sampling.top_p),
                top_k: self.top_k.or(settings.sampling.top_k),
            },
        }
    }
}

/// The operator's limits (`[control.limits]`, doc 88 §5a): the time limits,
/// in seconds, the only numbers that end a wait which is still within its
/// rights to go on, and how many points a computed series may have
/// (docs/design/98 §2). Each is configuration, shown in Settings beside
/// its shipped default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlLimits {
    /// One request of the background capacity probe.
    pub probe_secs: u64,
    /// How long a channel waits for a turn's reply before it answers that
    /// the work is still running.
    pub reply_secs: u64,
    /// A command whose call names no limit of its own.
    pub command_secs: u64,
    /// The longest limit a command's call may name; a call that names more
    /// runs with this.
    pub command_max_secs: u64,
    /// A scheduled script.
    pub script_secs: u64,
    /// A hook that sets no limit of its own.
    pub hook_secs: u64,
    /// A worker's question waiting for a person.
    pub question_secs: u64,
    /// An approval waiting for a person; silence means no.
    pub approval_secs: u64,
    /// The most points a controls card's series may have.
    pub series_points: u64,
}

impl ControlLimits {
    /// The limits a new install runs with.
    pub const fn shipped() -> Self {
        Self {
            probe_secs: 30,
            reply_secs: 240,
            command_secs: 120,
            command_max_secs: 600,
            script_secs: 120,
            hook_secs: 10,
            question_secs: 900,
            approval_secs: 900,
            series_points: 10_000,
        }
    }

    /// These limits with a configuration layer's set fields applied. A
    /// limit of zero would end every wait at once, so it is read as unset.
    pub fn with(self, over: ControlLimitsOverride) -> Self {
        let pick = |set: Option<u64>, lower: u64| set.filter(|secs| *secs > 0).unwrap_or(lower);
        Self {
            probe_secs: pick(over.probe_secs, self.probe_secs),
            reply_secs: pick(over.reply_secs, self.reply_secs),
            command_secs: pick(over.command_secs, self.command_secs),
            command_max_secs: pick(over.command_max_secs, self.command_max_secs),
            script_secs: pick(over.script_secs, self.script_secs),
            hook_secs: pick(over.hook_secs, self.hook_secs),
            question_secs: pick(over.question_secs, self.question_secs),
            approval_secs: pick(over.approval_secs, self.approval_secs),
            series_points: pick(over.series_points, self.series_points),
        }
    }
}

impl Default for ControlLimits {
    fn default() -> Self {
        Self::shipped()
    }
}

/// The limits one configuration layer sets: a field set there
/// overrides the layer below, an unset one inherits it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlLimitsOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_max_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_points: Option<u64>,
}

impl ControlLimitsOverride {
    /// This layer over `lower`: each field this one sets wins.
    pub fn over(self, lower: Self) -> Self {
        Self {
            probe_secs: self.probe_secs.or(lower.probe_secs),
            reply_secs: self.reply_secs.or(lower.reply_secs),
            command_secs: self.command_secs.or(lower.command_secs),
            command_max_secs: self.command_max_secs.or(lower.command_max_secs),
            script_secs: self.script_secs.or(lower.script_secs),
            hook_secs: self.hook_secs.or(lower.hook_secs),
            question_secs: self.question_secs.or(lower.question_secs),
            approval_secs: self.approval_secs.or(lower.approval_secs),
            series_points: self.series_points.or(lower.series_points),
        }
    }
}

/// The settings each purpose's steps are framed with: the shipped
/// defaults, overridden by configuration (doc 88 §4, §12.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepDefaults(std::collections::BTreeMap<Purpose, StepSettings>);

impl Default for StepDefaults {
    fn default() -> Self {
        Self::shipped()
    }
}

impl StepDefaults {
    /// What ships: a step that acts, plans, answers or judges keeps the
    /// model's own reasoning; a repair and a side call that wants a short
    /// structured answer ask for none. Sampling is left to the model.
    pub fn shipped() -> Self {
        Self(
            PURPOSES
                .into_iter()
                .map(|purpose| {
                    let reasoning = match purpose {
                        Purpose::Plan | Purpose::Act | Purpose::Answer | Purpose::Judge => {
                            Reasoning::Default
                        }
                        Purpose::Repair | Purpose::Compact | Purpose::Summarise => Reasoning::Off,
                    };
                    (
                        purpose,
                        // Every step asks for its tools' schemas to be
                        // enforced; a wire that cannot is never sent it
                        // (`ModelCapabilities::honour`).
                        StepSettings {
                            reasoning,
                            sampling: Sampling::default(),
                            strict_tools: true,
                            schema_output: false,
                        },
                    )
                })
                .collect(),
        )
    }

    /// These defaults with one configuration layer applied.
    pub fn with(mut self, layer: &std::collections::BTreeMap<Purpose, StepOverride>) -> Self {
        for (purpose, over) in layer {
            let current = self.get(*purpose);
            self.0.insert(*purpose, over.apply(current));
        }
        self
    }

    pub fn get(&self, purpose: Purpose) -> StepSettings {
        self.0.get(&purpose).copied().unwrap_or_default()
    }
}

/// `FrameStep` (doc 88 §3): the contract a step is dispatched under, from
/// its purpose, the configured defaults and the tools the turn offers.
pub fn frame(purpose: Purpose, defaults: &StepDefaults, tools: Vec<String>) -> StepContract {
    let settings = defaults.get(purpose);
    StepContract {
        purpose,
        reasoning: settings.reasoning,
        sampling: settings.sampling,
        tools,
        deferred: Vec::new(),
        output: OutputForm::Free,
    }
}

/// The form the step's output must take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum OutputForm {
    Free,
    /// The step must call this one tool.
    Tool {
        name: String,
    },
    /// The step must produce output matching the schema with this digest.
    Schema {
        digest: String,
    },
}

/// One step's frame (doc 88 §4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepContract {
    pub purpose: Purpose,
    pub reasoning: Reasoning,
    #[serde(default)]
    pub sampling: Sampling,
    /// The tools the request offers, by name, in request order.
    pub tools: Vec<String>,
    /// Those of `tools` not sent this step (listed by name, loaded once
    /// called), so the record says what the model could call at once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deferred: Vec<String>,
    pub output: OutputForm,
}

/// The contract asked for a step. What each dispatch of it applied is on
/// the step's receipt, attempt by attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Framing {
    pub step: u32,
    pub asked: StepContract,
}

/// Whether a proposed call runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", content = "by", rename_all = "snake_case")]
pub enum CallVerdict {
    Accepted,
    Refused(Refusal),
}

/// What refused a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// The name is no tool this turn offers.
    Unknown,
    /// The arguments do not match the tool's schema.
    Schema,
    /// The tool itself refused these arguments.
    Tool,
    /// The capability was revoked during the turn.
    Revoked,
    /// The same call was repeated in this run.
    Repeated,
    /// A permission rule or mode denied it.
    Policy,
    /// The person asked declined it.
    Person,
    /// No approver could be asked.
    Unattended,
}

/// Where a failed call's fault lies: what would have to change for it to
/// succeed. Named by whoever fails the call, where it fails; never read
/// from an error's wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    /// The call's shape: a name that is no tool, or arguments the tool does
    /// not take. Nothing ran.
    Shape,
    /// What the call names: a file that is not there, text that does not
    /// match, a command that failed. Another call can succeed.
    Target,
    /// The tool, or what it reaches. No change to the call helps.
    Tool,
    /// A rule, a mode or a person refused it.
    Refused,
    /// The run was cancelled.
    Cancelled,
    /// The operator's configured limit for the wait was reached and the
    /// work was stopped there (docs/design/88-control-plane.md §5a): the
    /// call may name a longer one, up to the operator's maximum, or do
    /// less at a time.
    Limit,
}

/// Whether a proposed answer ends the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verdict", content = "by", rename_all = "snake_case")]
pub enum AnswerVerdict {
    Accepted,
    Held(Cause),
}

/// What a correction answers: the check whose verdict failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    StopHook,
    StopGuard,
    Grounding,
    Fence,
    DuplicateCard,
    Presentation,
    CardCompletion,
    CardCompleteness,
    Freshness,
    EmptyStep,
    SteeringDrift,
    TopicMismatch,
    Repair,
    Plan,
    ToolCallRepair,
    /// The answer ends inside an unclosed block.
    Truncated,
    /// Files changed after the last check ran.
    VerificationStale,
    /// The person asked for work to go on until they say done.
    KeepWorking,
    /// A call failed and no later call of its tool succeeded.
    UnresolvedFailure,
    /// Workers the turn started are still running.
    WorkersRunning,
}

/// The rung of the correction ladder (doc 88 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    /// Tell once, with the evidence.
    Tell,
    /// Change the next step's frame.
    Reframe,
    /// End with what exists.
    End,
}

/// What the model is told, and what changes in its frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Correction {
    pub cause: Cause,
    pub rung: Rung,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ending {
    pub end: End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Answered,
    Aborted,
    Failed,
    StepLimit,
}

/// A pattern measured over real turns that a person may want changed in
/// Vak itself: a contract the model kept failing, by model. Only faults
/// that lie in a call's shape, its target, a tool or the runtime's checks
/// are patterns; a person declining or a rule denying is the system
/// working.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pattern {
    /// Calls to this tool were refused before they ran.
    ToolRefused { tool: String, refusal: Refusal },
    /// Calls to this tool ran and failed.
    ToolFailed { tool: String, fault: Fault },
    /// Turns needed this correction.
    Corrected { cause: Cause },
    /// Turns ended at the operator's step limit.
    StepLimit,
}

/// What `Promote` offers for review: a pattern on one model, how often it
/// happened against how often it could have (calls of that tool, or
/// turns), and the turns that show it. Nothing changes itself: a person
/// accepts or dismisses it (docs/design/43-governed-self-evolution.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// Stable across measurements: the same pattern on the same model is
    /// the same proposal, so a review of it holds.
    pub key: String,
    pub model: String,
    pub pattern: Pattern,
    pub count: u64,
    pub of: u64,
    pub turns: Vec<String>,
}

/// A person's answer to a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Review {
    Accepted,
    Dismissed,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_record_names_its_point_by_its_choice() {
        let decision =
            Decision::by_runtime(Choice::AcceptCall(CallVerdict::Refused(Refusal::Schema)))
                .with(Evidence::Call("toolu_1".into()));
        let json = serde_json::to_value(&decision).unwrap();
        assert_eq!(json["choice"]["point"], "accept_call");
        assert_eq!(json["choice"]["by"], "schema");
        assert_eq!(json["evidence"][0]["kind"], "call");
        let back: Decision = serde_json::from_value(json).unwrap();
        assert_eq!(back, decision);
        assert_eq!(back.point(), DecisionPoint::AcceptCall);
    }

    #[test]
    fn every_point_serialises_inside_its_tag() {
        for choice in [
            Choice::AcceptAnswer(AnswerVerdict::Accepted),
            Choice::AcceptAnswer(AnswerVerdict::Held(Cause::Plan)),
            Choice::Correct(Correction {
                cause: Cause::EmptyStep,
                rung: Rung::Tell,
            }),
            Choice::EndTurn(Ending {
                end: End::StepLimit,
            }),
            Choice::AdmitTurn(Admission {
                agent: None,
                model: "m".into(),
                max_steps: 8,
            }),
        ] {
            let decision = Decision::by_runtime(choice);
            let text = serde_json::to_string(&decision).unwrap();
            assert_eq!(serde_json::from_str::<Decision>(&text).unwrap(), decision);
        }
    }

    #[test]
    fn a_model_is_asked_only_what_it_can_honour() {
        let asked = StepSettings {
            strict_tools: false,
            schema_output: false,
            reasoning: Reasoning::Depth(Depth::Low),
            sampling: Sampling {
                temperature: Some(0.2),
                top_p: Some(0.9),
                top_k: Some(20),
            },
        };
        let toggle = ModelCapabilities {
            reasoning: ReasoningSupport::Toggle,
            top_k: Support::No,
            ..Default::default()
        };
        let applied = toggle.honour(asked);
        assert_eq!(applied.reasoning, Reasoning::Default);
        assert_eq!(applied.sampling.temperature, Some(0.2));
        assert_eq!(applied.sampling.top_k, None);
        let off = StepSettings {
            reasoning: Reasoning::Off,
            ..Default::default()
        };
        assert_eq!(toggle.honour(off).reasoning, Reasoning::Off);
        assert_eq!(ModelCapabilities::default().honour(asked), asked);
    }

    #[test]
    fn a_refused_request_narrows_only_what_it_sent() {
        let sent = StepSettings {
            strict_tools: false,
            schema_output: false,
            reasoning: Reasoning::Depth(Depth::High),
            sampling: Sampling::default(),
        };
        let caps = ModelCapabilities::default().refusing(sent);
        assert_eq!(caps.reasoning, ReasoningSupport::Fixed);
        assert_eq!(caps.temperature, Support::Unknown);
        let reported = ModelCapabilities {
            reasoning: ReasoningSupport::Graded,
            temperature: Support::Yes,
            ..Default::default()
        };
        assert_eq!(reported.meet(caps).reasoning, ReasoningSupport::Fixed);
        assert_eq!(reported.meet(caps).temperature, Support::Yes);
    }

    #[test]
    fn configuration_layers_override_field_by_field() {
        use std::collections::BTreeMap;
        let shipped = StepDefaults::shipped();
        assert_eq!(shipped.get(Purpose::Repair).reasoning, Reasoning::Off);
        assert_eq!(shipped.get(Purpose::Act).reasoning, Reasoning::Default);
        let workspace = BTreeMap::from([(
            Purpose::Act,
            StepOverride {
                temperature: Some(0.3),
                ..Default::default()
            },
        )]);
        let route = BTreeMap::from([(
            Purpose::Act,
            StepOverride {
                reasoning: Some(ReasoningSetting::Low),
                ..Default::default()
            },
        )]);
        let defaults = shipped.with(&workspace).with(&route);
        let act = defaults.get(Purpose::Act);
        assert_eq!(act.reasoning, Reasoning::Depth(Depth::Low));
        assert_eq!(act.sampling.temperature, Some(0.3));
        let contract = frame(Purpose::Act, &defaults, vec!["read".into()]);
        assert_eq!(contract.reasoning, act.reasoning);
        assert_eq!(contract.output, OutputForm::Free);
    }

    #[test]
    fn a_framing_round_trips() {
        let decision = Decision::by_runtime(Choice::FrameStep(Framing {
            step: 2,
            asked: StepContract {
                purpose: Purpose::Repair,
                reasoning: Reasoning::Depth(Depth::Low),
                sampling: Sampling::default(),
                tools: vec!["read".into(), "remember".into()],
                deferred: vec!["remember".into()],
                output: OutputForm::Free,
            },
        }));
        let text = serde_json::to_string(&decision).unwrap();
        assert_eq!(serde_json::from_str::<Decision>(&text).unwrap(), decision);
    }

    #[test]
    fn limits_take_the_narrowest_layer_and_ignore_zero() {
        let wide = ControlLimitsOverride {
            command_secs: Some(300),
            hook_secs: Some(20),
            ..Default::default()
        };
        let narrow = ControlLimitsOverride {
            command_secs: Some(45),
            command_max_secs: Some(0),
            approval_secs: Some(0),
            ..Default::default()
        };
        let limits = ControlLimits::shipped().with(narrow.over(wide));
        assert_eq!(limits.command_secs, 45);
        assert_eq!(
            limits.command_max_secs,
            ControlLimits::shipped().command_max_secs
        );
        assert_eq!(limits.hook_secs, 20);
        assert_eq!(limits.approval_secs, ControlLimits::shipped().approval_secs);
        assert_eq!(limits.reply_secs, ControlLimits::shipped().reply_secs);
    }
}
