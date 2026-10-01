//! The seven axes of a reading.
//!
//! These are deliberately **behavioural and orthogonal**, not a subject-matter
//! taxonomy. vak runs everything from "hi" to a multi-month programme, and a
//! domain label ("research", "coding") says nothing about what the runtime
//! should *do*. Each axis below drives one distinct subsystem, and no axis
//! collapses into another:
//!
//! * a long task can be closely watched and a ten-second automation
//!   unattended, so `Horizon` is not `Attendance`;
//! * work can be irreversible yet need only an assertion, or inert yet need
//!   citations, so `Stakes` is not `Evidence`.
//!
//! Subject matter survives only as an open-vocabulary `domains` tag on
//! [`crate::Reading`], used for skill affinity and telemetry and **never** for
//! control flow.
//!
//! Every ordered axis spells its `rank` out explicitly rather than deriving it
//! from declaration order. `vak_config::PermissionMode::rank` sets that
//! precedent for exactly the reason it applies here: the narrowing lattice in
//! [`crate::Limits`] depends on the ranking being right, and reordering the
//! variants some later day must not silently invert a safety decision.

use serde::{Deserialize, Serialize};

/// The shape of the work — what the runtime must actually do.
///
/// The `Author` / `Modify` / `Operate` split is the safety-relevant one and is
/// genuinely universal: it means the same thing for prose, a spreadsheet, and
/// a production deploy. Authoring produces new content, modifying changes
/// existing state, operating reaches outside the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Act {
    /// Social or conversational. No effect, no retrieval.
    Converse,
    /// Recall or explain from knowledge already in context.
    Answer,
    /// Find something: search, grep, retrieve, enumerate.
    Locate,
    /// Reason over gathered material and produce a judgement.
    Analyze,
    /// Produce new artifact content — prose, code, config, a plan.
    Author,
    /// Change existing state that the workspace owns.
    Modify,
    /// Act on the world outside the workspace: deploy, send, schedule,
    /// install, control a service, drive a screen.
    Operate,
    /// Check a claim: run tests, audit, reproduce.
    Verify,
    /// Decompose and delegate across workers or flows.
    Orchestrate,
    /// Meta-work on the agent itself: config, permissions, memory, skills.
    Govern,
}

impl Act {
    pub const ALL: [Act; 10] = [
        Act::Converse,
        Act::Answer,
        Act::Locate,
        Act::Analyze,
        Act::Author,
        Act::Modify,
        Act::Operate,
        Act::Verify,
        Act::Orchestrate,
        Act::Govern,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Act::Converse => "converse",
            Act::Answer => "answer",
            Act::Locate => "locate",
            Act::Analyze => "analyze",
            Act::Author => "author",
            Act::Modify => "modify",
            Act::Operate => "operate",
            Act::Verify => "verify",
            Act::Orchestrate => "orchestrate",
            Act::Govern => "govern",
        }
    }

    pub fn parse(value: &str) -> Option<Act> {
        Act::ALL
            .into_iter()
            .find(|act| act.as_str() == value.trim().to_ascii_lowercase())
    }

    /// Whether this act, by its nature, changes something outside the
    /// conversation. Used to decide whether an episode that produced no
    /// effect can honestly claim completion.
    pub fn is_effectful(self) -> bool {
        matches!(self, Act::Modify | Act::Operate | Act::Govern)
    }

    /// Whether this act, by its nature, demands a runtime execution or
    /// file-modification receipt (a bash call or a file write) before it can
    /// honestly claim to be done — the same set as [`Act::is_effectful`].
    /// `Verify` is absent: checking a pasted paragraph, a translation or a
    /// claim is proven by a source, a computation or a careful reading as
    /// often as by a command, so a check the request *demands* be proven is
    /// the evidence axis's job (`StopProfile::Verification`), not the act's.
    /// `Author` is absent for the same reason: producing prose, code, or a
    /// plan is proven by the response itself. A request that *names* a file
    /// deliverable still needs one — that is `OutcomeSpec::requires_execution`,
    /// which has the request text and this does not. `Orchestrate` is also
    /// absent: delegating is proven by a tool dispatch, so it belongs under
    /// `requires_tool` instead.
    pub fn requires_execution(self) -> bool {
        matches!(self, Act::Modify | Act::Operate | Act::Govern)
    }

    /// Whether this act requires inspection, search, or enumeration tools.
    pub fn requires_inspection(self) -> bool {
        matches!(self, Act::Locate)
    }

    /// Whether this act requires any tool invocation at all. `Orchestrate`
    /// joins here rather than in `requires_execution`: it needs proof that a
    /// worker or flow was actually dispatched, but that proof is a tool
    /// call, not specifically a shell command or a file write. `Verify`
    /// joins here too; the stop gate still accepts a substantive direct
    /// answer when the material to check came with the request.
    pub fn requires_tool(self) -> bool {
        self.requires_execution()
            || self.requires_inspection()
            || matches!(self, Act::Orchestrate | Act::Verify)
    }
}

/// How long the work lives. Promotion to a durable commitment happens at
/// `Session` and above; below that an intent resolves and dies inside the
/// turn, so a simple prompt pays nothing for this machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Horizon {
    /// One reply. No tools expected.
    Immediate,
    /// A handful of tool calls inside one turn.
    Turn,
    /// Multi-step work that wants a plan inside this session.
    Session,
    /// Spans sessions, restarts, and potentially months.
    Durable,
}

impl Horizon {
    pub const ALL: [Horizon; 4] = [
        Horizon::Immediate,
        Horizon::Turn,
        Horizon::Session,
        Horizon::Durable,
    ];

    /// Explicit total order; see the module note on why this is not derived.
    pub fn rank(self) -> u8 {
        match self {
            Horizon::Immediate => 0,
            Horizon::Turn => 1,
            Horizon::Session => 2,
            Horizon::Durable => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Horizon::Immediate => "immediate",
            Horizon::Turn => "turn",
            Horizon::Session => "session",
            Horizon::Durable => "durable",
        }
    }

    pub fn parse(value: &str) -> Option<Horizon> {
        Horizon::ALL
            .into_iter()
            .find(|h| h.as_str() == value.trim().to_ascii_lowercase())
    }

    /// Work at this horizon earns a durable commitment of its own.
    ///
    /// Only work that outlives the session: multi-step work inside one
    /// session runs under its plan (the managed work contract), and opening a
    /// month-long obligation for it left the ledger full of commitments
    /// nobody would ever close.
    pub fn opens_commitment(self) -> bool {
        self == Horizon::Durable
    }
}

/// Blast radius if the work goes wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stakes {
    /// No effect at all; nothing to undo.
    Inert,
    /// Workspace state, recoverable from a checkpoint or version control.
    Reversible,
    /// Spends money, quota, or meaningful time.
    Costly,
    /// Cannot be undone by this runtime: external sends, deletes, deploys,
    /// payments, anything a third party observes.
    Irreversible,
}

impl Stakes {
    pub const ALL: [Stakes; 4] = [
        Stakes::Inert,
        Stakes::Reversible,
        Stakes::Costly,
        Stakes::Irreversible,
    ];

    pub fn rank(self) -> u8 {
        match self {
            Stakes::Inert => 0,
            Stakes::Reversible => 1,
            Stakes::Costly => 2,
            Stakes::Irreversible => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Stakes::Inert => "inert",
            Stakes::Reversible => "reversible",
            Stakes::Costly => "costly",
            Stakes::Irreversible => "irreversible",
        }
    }

    pub fn parse(value: &str) -> Option<Stakes> {
        Stakes::ALL
            .into_iter()
            .find(|s| s.as_str() == value.trim().to_ascii_lowercase())
    }

    /// Take a checkpoint before the first effect, so `Review`-mode
    /// human-in-the-loop has something to offer a reversal against.
    pub fn wants_checkpoint(self) -> bool {
        self.rank() >= Stakes::Reversible.rank()
    }
}

/// The standard of proof the result must meet. This axis is the one that
/// answers "how do we know it is done": it sets the **minimum**
/// [`Satisfaction`] strength at which a commitment may close `Fulfilled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Evidence {
    /// The agent's word is enough.
    None,
    /// Claims must name their sources.
    Cited,
    /// A machine-checkable predicate must pass.
    Verified,
    /// An independent party must confirm.
    Audited,
}

impl Evidence {
    pub const ALL: [Evidence; 4] = [
        Evidence::None,
        Evidence::Cited,
        Evidence::Verified,
        Evidence::Audited,
    ];

    pub fn rank(self) -> u8 {
        match self {
            Evidence::None => 0,
            Evidence::Cited => 1,
            Evidence::Verified => 2,
            Evidence::Audited => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Evidence::None => "none",
            Evidence::Cited => "cited",
            Evidence::Verified => "verified",
            Evidence::Audited => "audited",
        }
    }

    pub fn parse(value: &str) -> Option<Evidence> {
        Evidence::ALL
            .into_iter()
            .find(|e| e.as_str() == value.trim().to_ascii_lowercase())
    }

    /// The weakest satisfaction strength that may close work held to this
    /// standard. This mapping is the closure invariant in one place.
    pub fn min_satisfaction(self) -> Satisfaction {
        match self {
            Evidence::None => Satisfaction::Asserted,
            Evidence::Cited => Satisfaction::Cited,
            Evidence::Verified => Satisfaction::Observed,
            Evidence::Audited => Satisfaction::Attested,
        }
    }
}

/// How strongly a completion claim is backed. Ordered weakest to strongest.
///
/// The distinction that matters: `Asserted` and `Cited` come from the model,
/// while `Observed` and `Attested` come from outside it. The runtime evaluates
/// `Observed` predicates itself and the model never marks one passed — the
/// same separation of powers as permission-before-dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Satisfaction {
    /// The model says so. Transcript only.
    #[default]
    Asserted,
    /// The model says so and names sources that can be followed.
    Cited,
    /// A predicate the runtime evaluated against the world passed.
    Observed,
    /// An independent party confirmed: a human, an auditor with no write
    /// access to what it audits, or an external receipt.
    Attested,
}

impl Satisfaction {
    pub const ALL: [Satisfaction; 4] = [
        Satisfaction::Asserted,
        Satisfaction::Cited,
        Satisfaction::Observed,
        Satisfaction::Attested,
    ];

    pub fn rank(self) -> u8 {
        match self {
            Satisfaction::Asserted => 0,
            Satisfaction::Cited => 1,
            Satisfaction::Observed => 2,
            Satisfaction::Attested => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Satisfaction::Asserted => "asserted",
            Satisfaction::Cited => "cited",
            Satisfaction::Observed => "observed",
            Satisfaction::Attested => "attested",
        }
    }

    pub fn parse(value: &str) -> Option<Satisfaction> {
        Satisfaction::ALL
            .into_iter()
            .find(|s| s.as_str() == value.trim().to_ascii_lowercase())
    }

    /// Whether evidence of this strength discharges a requirement for
    /// `required`. The whole closure invariant reduces to this call.
    pub fn satisfies(self, required: Satisfaction) -> bool {
        self.rank() >= required.rank()
    }

    /// Evidence produced by the model itself rather than observed from
    /// outside it. Kept explicit because "the model checked its own work" is
    /// the failure mode this lattice exists to make visible.
    pub fn is_self_reported(self) -> bool {
        matches!(self, Satisfaction::Asserted | Satisfaction::Cited)
    }
}

/// How well specified the request is.
///
/// Interaction rule, applied in [`crate::engage`]: ambiguity is only worth
/// interrupting for when the stakes are high. `Ambiguous + Inert` proceeds on
/// a stated assumption; `Ambiguous + Irreversible` asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Clarity {
    /// One plausible reading, all required parameters present or inferable.
    Clear,
    /// One plausible reading, but something needed is missing.
    Underspecified,
    /// Two or more plausible readings that imply different work.
    Ambiguous,
}

impl Clarity {
    pub const ALL: [Clarity; 3] = [Clarity::Clear, Clarity::Underspecified, Clarity::Ambiguous];

    pub fn rank(self) -> u8 {
        match self {
            Clarity::Clear => 0,
            Clarity::Underspecified => 1,
            Clarity::Ambiguous => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Clarity::Clear => "clear",
            Clarity::Underspecified => "underspecified",
            Clarity::Ambiguous => "ambiguous",
        }
    }

    pub fn parse(value: &str) -> Option<Clarity> {
        Clarity::ALL
            .into_iter()
            .find(|c| c.as_str() == value.trim().to_ascii_lowercase())
    }
}

/// A channel of input or output.
///
/// Modality is a **hard constraint on dispatch**, not a preference: a leg that
/// cannot see is not a valid fallback for a vision turn. Silently dropping an
/// image because the serving model is text-only is precisely the "everything
/// worked as designed and the outcome was a lie" failure that `vak_core::reach`
/// exists to prevent, so an unsatisfiable modality is a typed error rather than
/// a quiet degradation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    /// Live screen control: reading pixels and driving a pointer/keyboard.
    Screen,
    /// Structured records — tabular, JSON, a query result set.
    Data,
    /// An open-ended feed consumed or produced incrementally.
    Stream,
}

impl Modality {
    pub const ALL: [Modality; 7] = [
        Modality::Text,
        Modality::Image,
        Modality::Audio,
        Modality::Video,
        Modality::Screen,
        Modality::Data,
        Modality::Stream,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Modality::Text => "text",
            Modality::Image => "image",
            Modality::Audio => "audio",
            Modality::Video => "video",
            Modality::Screen => "screen",
            Modality::Data => "data",
            Modality::Stream => "stream",
        }
    }

    pub fn parse(value: &str) -> Option<Modality> {
        Modality::ALL
            .into_iter()
            .find(|m| m.as_str() == value.trim().to_ascii_lowercase())
    }

    /// Whether serving this modality needs a capability beyond plain text
    /// completion. `Text` and `Data` ride the ordinary text interface; the
    /// rest constrain which legs may serve the turn.
    pub fn needs_declared_support(self) -> bool {
        !matches!(self, Modality::Text | Modality::Data)
    }
}

/// Whether a human is available to answer, as **observed by the runtime**.
///
/// This is a fact about the environment, not a grant. It is deliberately
/// separate from [`crate::Autonomy`], which is a fact about what a human has
/// delegated. Conflating the two is why agents nag when you wanted autonomy
/// and barrel ahead when nobody is watching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Attendance {
    /// Someone is here now and will answer within seconds.
    ///
    /// The default, matching `Core`'s existing optimistic `approver_answerable`
    /// and its `reconcile_answerability` correction: assume a human is
    /// reachable, then let the surface that actually serves the run say
    /// otherwise. Assuming nobody is there would make every gate an immediate
    /// denial on surfaces that had simply not answered yet.
    #[default]
    Interactive,
    /// Someone is reachable and will answer within minutes or hours.
    Supervised,
    /// Nobody will answer. Gates raised here are not questions, they are
    /// denials, unless the work can suspend and wait.
    Unattended,
}

impl Attendance {
    pub const ALL: [Attendance; 3] = [
        Attendance::Interactive,
        Attendance::Supervised,
        Attendance::Unattended,
    ];

    /// Ranked by how much supervision is available, most first.
    ///
    /// An ordering for reporting and for choosing the more cautious of two
    /// observations — deliberately *not* a cap on [`crate::Autonomy`]:
    /// delegation and oversight are independent facts, and their
    /// interaction lives in [`crate::Authority::gate_fallback`].
    pub fn rank(self) -> u8 {
        match self {
            Attendance::Unattended => 0,
            Attendance::Supervised => 1,
            Attendance::Interactive => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Attendance::Interactive => "interactive",
            Attendance::Supervised => "supervised",
            Attendance::Unattended => "unattended",
        }
    }

    pub fn parse(value: &str) -> Option<Attendance> {
        Attendance::ALL
            .into_iter()
            .find(|a| a.as_str() == value.trim().to_ascii_lowercase())
    }

    /// Whether a raised gate can actually reach somebody in time to matter.
    pub fn can_answer_now(self) -> bool {
        matches!(self, Attendance::Interactive)
    }
}

/// The epistemic cognitive stance for this turn — how the model postures its reasoning,
/// evidence evaluation, and communication across any domain of work.
///
/// While `Act` describes *what* action is being requested, `EpistemicStance` defines
/// *how to think, inquire, and communicate*. This is completely domain-neutral:
/// it governs financial modeling, legal research, scientific synthesis, creative writing,
/// system operations, data analysis, and software engineering with equal fidelity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EpistemicStance {
    /// Social, relational, or conversational. Low cognitive ceremony, direct and friendly.
    Conversational,
    /// Direct explanation or recall from existing knowledge. Clear and concise.
    #[default]
    DirectAnswer,
    /// Critical evaluation, synthesis, and deduction. Claims are separated from verified
    /// facts, counter-hypotheses are evaluated, and assertions require citations.
    Analytical,
    /// Breadth-first mapping, discovery, and search. Systematically surveys options,
    /// trade-offs, and unknowns without premature closure.
    Exploratory,
    /// High-density synthesis, creation, or composition (prose, plans, designs, code, briefs).
    /// Focuses on tone alignment, structural elegance, and publication-ready finish.
    Generative,
    /// State-changing action, execution, or environment mutation. Emphasizes inspecting
    /// state first, minimal blast radius, invariant verification, and rollback awareness.
    Operational,
    /// Root-cause investigation, discrepancy audit, or debugging. Isolates underlying causes
    /// before proposing repairs; tests hypotheses against observed evidence.
    Diagnostic,
}

impl EpistemicStance {
    pub const ALL: [EpistemicStance; 7] = [
        EpistemicStance::Conversational,
        EpistemicStance::DirectAnswer,
        EpistemicStance::Analytical,
        EpistemicStance::Exploratory,
        EpistemicStance::Generative,
        EpistemicStance::Operational,
        EpistemicStance::Diagnostic,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EpistemicStance::Conversational => "conversational",
            EpistemicStance::DirectAnswer => "direct-answer",
            EpistemicStance::Analytical => "analytical",
            EpistemicStance::Exploratory => "exploratory",
            EpistemicStance::Generative => "generative",
            EpistemicStance::Operational => "operational",
            EpistemicStance::Diagnostic => "diagnostic",
        }
    }

    pub fn parse(value: &str) -> Option<EpistemicStance> {
        EpistemicStance::ALL
            .into_iter()
            .find(|s| s.as_str() == value.trim().to_ascii_lowercase())
    }

    /// Concise, universal operating guidelines for this stance.
    /// Free of domain-specific jargon; applies equally across all disciplines.
    pub fn guideline_prompt(self) -> &'static str {
        match self {
            EpistemicStance::Conversational => {
                "Engage directly and naturally. Keep replies concise and conversational; \
                 do not force structured workflows or unprompted actions when simple dialogue is requested."
            }
            EpistemicStance::DirectAnswer => {
                "Provide a clear, direct answer to the question. Avoid unnecessary meta-commentary, \
                 unsolicited execution plans, or unwarranted tool calls when knowledge in context suffices."
            }
            // Citation is demanded by the intent note when the request asks
            // for it, not by the stance: measured live, "cite sources for
            // every factual assertion" made a small local model deliberate
            // and emit nothing on requests that asked for no sources.
            EpistemicStance::Analytical => {
                "Scrutinize claims objectively. Separate verified facts from inferences and \
                 evaluate counter-arguments or alternative explanations before concluding."
            }
            EpistemicStance::Exploratory => {
                "Map the landscape systematically. Prioritize breadth, surface key trade-offs, \
                 and report gaps, uncertainties, or negative findings honestly without forcing premature conclusions."
            }
            EpistemicStance::Generative => {
                "Focus on polished, high-density craftsmanship. Align voice and tone to the audience, \
                 eliminate filler and boilerplate, and deliver complete, publication-ready outputs."
            }
            EpistemicStance::Operational => {
                "Inspect existing state before acting. Minimize blast radius, verify outcomes \
                 immediately after each step, and ensure changes can be safely understood or reversed."
            }
            EpistemicStance::Diagnostic => {
                "Isolate root causes before applying fixes. Form explicit hypotheses, test them \
                 against observed evidence, and verify edge cases before concluding an issue is resolved."
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_total_and_strictly_increasing() {
        for pair in Horizon::ALL.windows(2) {
            assert!(pair[0].rank() < pair[1].rank());
        }
        for pair in Stakes::ALL.windows(2) {
            assert!(pair[0].rank() < pair[1].rank());
        }
        for pair in Evidence::ALL.windows(2) {
            assert!(pair[0].rank() < pair[1].rank());
        }
        for pair in Satisfaction::ALL.windows(2) {
            assert!(pair[0].rank() < pair[1].rank());
        }
        for pair in Clarity::ALL.windows(2) {
            assert!(pair[0].rank() < pair[1].rank());
        }
    }

    #[test]
    fn every_axis_round_trips_through_its_wire_name() {
        for act in Act::ALL {
            assert_eq!(Act::parse(act.as_str()), Some(act));
        }
        for horizon in Horizon::ALL {
            assert_eq!(Horizon::parse(horizon.as_str()), Some(horizon));
        }
        for stakes in Stakes::ALL {
            assert_eq!(Stakes::parse(stakes.as_str()), Some(stakes));
        }
        for evidence in Evidence::ALL {
            assert_eq!(Evidence::parse(evidence.as_str()), Some(evidence));
        }
        for clarity in Clarity::ALL {
            assert_eq!(Clarity::parse(clarity.as_str()), Some(clarity));
        }
        for modality in Modality::ALL {
            assert_eq!(Modality::parse(modality.as_str()), Some(modality));
        }
        for attendance in Attendance::ALL {
            assert_eq!(Attendance::parse(attendance.as_str()), Some(attendance));
        }
        for satisfaction in Satisfaction::ALL {
            assert_eq!(
                Satisfaction::parse(satisfaction.as_str()),
                Some(satisfaction)
            );
        }
        for stance in EpistemicStance::ALL {
            assert_eq!(EpistemicStance::parse(stance.as_str()), Some(stance));
            assert!(!stance.guideline_prompt().is_empty());
        }
    }

    /// The closure invariant reduced to its smallest statement: self-reported
    /// evidence can never discharge a requirement for observed or attested
    /// proof. If this ever passes, a model can close its own audit.
    #[test]
    fn self_reported_evidence_never_satisfies_external_requirements() {
        for weak in [Satisfaction::Asserted, Satisfaction::Cited] {
            assert!(weak.is_self_reported());
            assert!(!weak.satisfies(Satisfaction::Observed));
            assert!(!weak.satisfies(Satisfaction::Attested));
        }
        assert!(Satisfaction::Observed.satisfies(Satisfaction::Observed));
        assert!(Satisfaction::Attested.satisfies(Satisfaction::Observed));
        assert!(!Satisfaction::Observed.satisfies(Satisfaction::Attested));
    }

    #[test]
    fn evidence_maps_onto_the_satisfaction_lattice_monotonically() {
        for pair in Evidence::ALL.windows(2) {
            assert!(pair[0].min_satisfaction().rank() < pair[1].min_satisfaction().rank());
        }
    }
}
