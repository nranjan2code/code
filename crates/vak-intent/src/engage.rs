//! Deriving an engagement from a reading and the authority in force.
//!
//! An [`Engagement`] has two halves and the split is load-bearing:
//!
//! * [`Limits`] carries authority implications, forms a meet semilattice, and
//!   is only ever composed downward. Invariant 1 lives there.
//! * [`Posture`] carries selections that imply no authority — which renderer,
//!   how chatty, whether to state an assumption. Getting one wrong is a
//!   quality bug, not a safety bug.
//!
//! Keeping them apart means the safety review only has to read one small type.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::authority::{Authority, Autonomy, GateFallback};
use crate::axes::{Act, Attendance, Clarity, EpistemicStance, Evidence, Horizon, Modality, Stakes};
use crate::limits::Limits;
use crate::reading::Reading;

/// How a human is kept in the loop for this work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HilMode {
    /// Block and ask now. Someone is here and the decision cannot wait.
    Interrupt,
    /// Proceed inside the granted envelope; escalate outside it.
    Envelope,
    /// Do it, show the diff, offer a reversal. Only for work a checkpoint can
    /// undo — asking permission for something instantly reversible spends
    /// the user's attention for nothing.
    Review,
    /// Nobody can answer now, so suspend the commitment and put the question
    /// in the inbox rather than failing the work outright.
    Defer,
}

impl HilMode {
    /// How much a human is pulled in, least first. `meet` takes the larger.
    pub fn caution_rank(self) -> u8 {
        match self {
            HilMode::Review => 0,
            HilMode::Envelope => 1,
            HilMode::Defer => 2,
            HilMode::Interrupt => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            HilMode::Interrupt => "interrupt",
            HilMode::Envelope => "envelope",
            HilMode::Review => "review",
            HilMode::Defer => "defer",
        }
    }
}

/// What to do about a request that is not fully specified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClarifyPolicy {
    /// Nothing missing; get on with it.
    Proceed,
    /// Choose a reading, say which one out loud, and continue. The right
    /// default for low-stakes ambiguity: a question costs more than a stated
    /// assumption that turns out wrong and is corrected.
    StateAssumption,
    /// Stop and ask. Reserved for ambiguity that could cause harm.
    Ask,
}

impl ClarifyPolicy {
    pub fn caution_rank(self) -> u8 {
        match self {
            ClarifyPolicy::Proceed => 0,
            ClarifyPolicy::StateAssumption => 1,
            ClarifyPolicy::Ask => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ClarifyPolicy::Proceed => "proceed",
            ClarifyPolicy::StateAssumption => "state-assumption",
            ClarifyPolicy::Ask => "ask",
        }
    }
}

/// The shape the result should take when rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputShape {
    /// Ordinary prose.
    Prose,
    /// A claim with the sources that back it.
    Sources,
    /// A ranked list of locations.
    Findings,
    /// A change set.
    Diff,
    /// Pass/fail per check.
    Matrix,
    /// Rows and columns.
    Table,
    /// A produced file or document.
    Artifact,
    /// What was done, for after-the-fact reading.
    Report,
}

impl OutputShape {
    pub fn as_str(self) -> &'static str {
        match self {
            OutputShape::Prose => "prose",
            OutputShape::Sources => "sources",
            OutputShape::Findings => "findings",
            OutputShape::Diff => "diff",
            OutputShape::Matrix => "matrix",
            OutputShape::Table => "table",
            OutputShape::Artifact => "artifact",
            OutputShape::Report => "report",
        }
    }
}

/// When results reach the human.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cadence {
    /// Stream as it happens.
    Live,
    /// One delivery when the unit of work finishes.
    OnCompletion,
    /// Roll up into the next digest. What overnight work should do rather
    /// than sending forty notifications nobody reads.
    Digest,
}

impl Cadence {
    /// Least batched first. Composition takes the *least* batched, because
    /// a held packet is the one that can be lost.
    pub fn immediacy_rank(self) -> u8 {
        match self {
            Cadence::Digest => 0,
            Cadence::OnCompletion => 1,
            Cadence::Live => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Cadence::Live => "live",
            Cadence::OnCompletion => "on-completion",
            Cadence::Digest => "digest",
        }
    }
}

/// How hard a delivery may push for attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Urgency {
    /// Break through: this needs somebody now.
    Interrupt,
    /// Normal notification.
    Notify,
    /// Silent; it will be read when the human looks.
    Quiet,
}

impl Urgency {
    pub fn rank(self) -> u8 {
        match self {
            Urgency::Quiet => 0,
            Urgency::Notify => 1,
            Urgency::Interrupt => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Urgency::Interrupt => "interrupt",
            Urgency::Notify => "notify",
            Urgency::Quiet => "quiet",
        }
    }
}

/// How results should be delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryPosture {
    pub shape: OutputShape,
    pub cadence: Cadence,
    pub urgency: Urgency,
}

/// Facts about this turn's difficulty, for the route ladder's demand scoring.
///
/// These are the fields `vak_llm::DemandInput` has always had and that
/// `plan_route_ladder` has always passed as zeros, which is why objective
/// selection has been effectively constant. Supplying them honestly is what
/// turns the existing router on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemandHint {
    pub reasoning_required: bool,
    pub evidence_required: bool,
    pub structured_output: bool,
    /// The whole turn is one reply from what is already known: every part
    /// of it is conversation, or an answer expected in one reply, read
    /// confidently. The only thing it is used for is asking a provider for
    /// less reasoning than its default; nothing ever asks for more on a
    /// reading (invariant 32), and false changes nothing.
    #[serde(default)]
    pub direct_reply: bool,
}

/// When the loop may stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StopProfile {
    /// One message is a complete answer.
    #[default]
    Message,
    /// Stopping is fine once something was looked at.
    Inspection,
    /// Stopping requires an observable effect. An effectful act that produced
    /// no effect did not finish, whatever the model says about it.
    Effect,
    /// Stopping requires a criterion to have been evaluated.
    Verification,
}

impl StopProfile {
    /// Strictest last. Composition takes the strictest.
    pub fn rank(self) -> u8 {
        match self {
            StopProfile::Message => 0,
            StopProfile::Inspection => 1,
            StopProfile::Effect => 2,
            StopProfile::Verification => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            StopProfile::Message => "message",
            StopProfile::Inspection => "inspection",
            StopProfile::Effect => "effect",
            StopProfile::Verification => "verification",
        }
    }
}

/// How much history and recall this turn wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextProfile {
    /// Just the conversation. No retrieval, no workspace scan.
    Minimal,
    /// Conversation plus recall of prior sessions and memory.
    Recall,
    /// Recall plus the workspace delta since the run started.
    Working,
    /// Everything, rendered from the commitment ledger.
    Full,
}

impl ContextProfile {
    /// Widest last. Composition takes the widest, since a strand that needs
    /// the workspace delta needs it whatever its neighbours need.
    pub fn rank(self) -> u8 {
        match self {
            ContextProfile::Minimal => 0,
            ContextProfile::Recall => 1,
            ContextProfile::Working => 2,
            ContextProfile::Full => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ContextProfile::Minimal => "minimal",
            ContextProfile::Recall => "recall",
            ContextProfile::Working => "working",
            ContextProfile::Full => "full",
        }
    }
}

/// The non-authority half of an engagement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Posture {
    /// Run under managed-work admission rather than a direct turn.
    pub managed: bool,
    /// Open a durable commitment for this work.
    pub open_commitment: bool,
    /// Take a checkpoint before the first effect.
    pub checkpoint_before_effect: bool,
    pub hil: HilMode,
    pub gate_fallback: GateFallback,
    pub clarify: ClarifyPolicy,
    pub delivery: DeliveryPosture,
    pub demand: DemandHint,
    pub stop: StopProfile,
    pub context: ContextProfile,
    /// The epistemic cognitive stance for this turn.
    #[serde(default)]
    pub epistemic_stance: EpistemicStance,
    /// The code-owned block this engagement contributes to the prompt.
    /// `None` when there is nothing worth spending tokens to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What the runtime will do about a reading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Engagement {
    pub limits: Limits,
    pub posture: Posture,
}

impl Engagement {
    /// The engagement that changes nothing — vak's pre-kernel behaviour.
    pub fn general() -> Self {
        Engagement {
            limits: Limits::unrestricted(),
            posture: Posture {
                managed: false,
                open_commitment: false,
                checkpoint_before_effect: false,
                hil: HilMode::Interrupt,
                gate_fallback: GateFallback::Deny,
                clarify: ClarifyPolicy::Proceed,
                delivery: DeliveryPosture {
                    shape: OutputShape::Prose,
                    cadence: Cadence::Live,
                    urgency: Urgency::Notify,
                },
                demand: DemandHint {
                    reasoning_required: false,
                    evidence_required: false,
                    structured_output: false,
                    direct_reply: false,
                },
                stop: StopProfile::Message,
                context: ContextProfile::Recall,
                epistemic_stance: EpistemicStance::DirectAnswer,
                note: None,
            },
        }
    }

    /// The engagement for a request the kernel could not read confidently.
    ///
    /// Posture is the general one, but the tool surface is narrowed to the
    /// orientation floor (docs/design/68-context-engine.md, Principle 6:
    /// "when a decision cannot be made confidently, send less and give the
    /// model a way to ask for more"). This is an *explicit* decision rather
    /// than an ambiguous top element: `Limits::unrestricted` keeps meaning
    /// "everything", which is what a disabled kernel produces.
    pub fn orienting() -> Self {
        let mut engagement = Engagement::general();
        engagement.limits.required_domains =
            crate::limits::DomainSet::only(FLOOR_DOMAINS.iter().copied());
        engagement
    }

    /// Compose with another engagement, narrowing only: limits meet, and the
    /// posture takes the more cautious of each selection ([`Posture::meet`]).
    pub fn meet(&self, other: &Engagement) -> Engagement {
        Engagement {
            limits: self.limits.meet(&other.limits),
            posture: self.posture.meet(&other.posture),
        }
    }

    /// The engagement for a turn made of several strands.
    ///
    /// Everything authority-bearing meets: the strictest strand governs
    /// approval, permission, spend, closure evidence, modalities, and the
    /// posture. The domain requirement is the turn's *capacity* rather than
    /// its authority and takes the **union** — a turn that is "search the web,
    /// then run the tests" needs both toolsets. It is still at most the
    /// unrestricted baseline, which is the invariant that matters.
    pub fn compose(strands: &[Engagement]) -> Engagement {
        let Some((first, rest)) = strands.split_first() else {
            return Engagement::general();
        };
        let mut out = first.clone();
        let mut domains = first.limits.required_domains.clone();
        for next in rest {
            domains = domains.union(&next.limits.required_domains);
            out = out.meet(next);
        }
        out.limits.required_domains = domains;
        out
    }
}

impl Posture {
    /// The more cautious of two postures, field by field, under an explicit
    /// per-field order, so the operation is commutative and never less
    /// cautious than either operand. The delivery shape and the stance have
    /// no order and are `self`'s. Notes are kept from both sides: a
    /// model-visible instruction one strand needed is not dropped because
    /// another had none.
    pub fn meet(&self, other: &Posture) -> Posture {
        let a = self;
        let b = other;
        {
            Posture {
                managed: a.managed || b.managed,
                open_commitment: a.open_commitment || b.open_commitment,
                checkpoint_before_effect: a.checkpoint_before_effect || b.checkpoint_before_effect,
                hil: if b.hil.caution_rank() > a.hil.caution_rank() {
                    b.hil
                } else {
                    a.hil
                },
                gate_fallback: if a.gate_fallback == GateFallback::Deny
                    || b.gate_fallback == GateFallback::Deny
                {
                    GateFallback::Deny
                } else {
                    GateFallback::Defer
                },
                clarify: if b.clarify.caution_rank() > a.clarify.caution_rank() {
                    b.clarify
                } else {
                    a.clarify
                },
                delivery: DeliveryPosture {
                    // The primary strand's shape; shapes have no order.
                    shape: a.delivery.shape,
                    cadence: if b.delivery.cadence.immediacy_rank()
                        > a.delivery.cadence.immediacy_rank()
                    {
                        b.delivery.cadence
                    } else {
                        a.delivery.cadence
                    },
                    urgency: if b.delivery.urgency.rank() > a.delivery.urgency.rank() {
                        b.delivery.urgency
                    } else {
                        a.delivery.urgency
                    },
                },
                demand: DemandHint {
                    reasoning_required: a.demand.reasoning_required || b.demand.reasoning_required,
                    evidence_required: a.demand.evidence_required || b.demand.evidence_required,
                    structured_output: a.demand.structured_output || b.demand.structured_output,
                    direct_reply: a.demand.direct_reply && b.demand.direct_reply,
                },
                stop: if b.stop.rank() > a.stop.rank() {
                    b.stop
                } else {
                    a.stop
                },
                context: if b.context.rank() > a.context.rank() {
                    b.context
                } else {
                    a.context
                },
                // The primary strand's stance. Stances have no order either;
                // the strand-aware note carries the rest.
                epistemic_stance: a.epistemic_stance,
                note: match (&a.note, &b.note) {
                    (None, None) => None,
                    (Some(n), None) | (None, Some(n)) => Some(n.clone()),
                    (Some(x), Some(y)) if x == y => Some(x.clone()),
                    (Some(x), Some(y)) => Some(format!("{x}\n{y}")),
                },
            }
        }
    }
}

// ------------------------------------------------------ capability slice ---

/// Kinds of work this act plausibly needs, as domain names.
///
/// This replaces a table that listed built-in *tool names* per act
/// (`Act::Answer => ["webfetch", "mcp"]`). That shape could not answer the
/// only question that matters — which of the capabilities this user actually
/// installed could serve this request — because installed capabilities were
/// never in it. Its failure mode was silent: a live-data question about a
/// specific place read as `Answer`, sliced to six file tools, and came back
/// "I do not have access to real-time information" with a configured,
/// connected search server sitting right there. Naming the server in the
/// next turn did not help either, because `Locate` had no way to reach one.
///
/// Domains are matched against what each capability declares it serves, so
/// adding an integration never edits this function. A capability that
/// declares nothing is never narrowed away, which keeps the common case
/// working with no configuration at all.
fn act_domains(act: Act) -> &'static [&'static str] {
    match act {
        // A greeting needs nothing beyond the orientation floor. This is
        // where slicing pays for itself most obviously.
        Act::Converse => &[],
        // The acts that exist to produce a fact must be able to go and get
        // one. Lookup is read-only, and a missing capability costs a failed
        // task while an extra one costs a little context — so this trade is
        // the one this table has always claimed to want to make.
        Act::Answer => &["live-data", "web"],
        Act::Locate => &["live-data", "web", "vcs"],
        Act::Analyze => &["live-data", "web", "code-exec", "vcs"],
        Act::Author => &["documents", "web"],
        Act::Modify => &["documents", "code-exec", "vcs"],
        // Operating reaches outside the workspace and legitimately needs the
        // broad set; the narrowing that matters for this act is the approval
        // floor, not the toolbox.
        Act::Operate => &[
            "code-exec",
            "web",
            "live-data",
            "messaging",
            "documents",
            "orchestration",
        ],
        Act::Verify => &["code-exec", "observability"],
        Act::Orchestrate => &["orchestration", "code-exec"],
        Act::Govern => &["memory", "orchestration", "documents", "observability"],
    }
}

/// The domains every turn gets regardless of act: enough to look at what is
/// in front of it and recall what it already knows.
///
/// An agent that cannot look at anything cannot correct a misread of its own
/// task, so this floor is what makes slicing safe to attempt at all.
pub const FLOOR_DOMAINS: &[&str] = &["filesystem", "memory"];

/// Every domain a capability can declare it serves — the shared vocabulary
/// readings are matched against (`vak_core::capability::Domain` mirrors it,
/// and a test there keeps the two equal). A tier-2/3 classifier is told to
/// choose from exactly these, because a free-form subject tag ("weather")
/// matches no capability and would key behaviour on a topic.
pub const DOMAIN_VOCABULARY: &[&str] = &[
    "live-data",
    "web",
    "filesystem",
    "code-exec",
    "memory",
    "messaging",
    "documents",
    "orchestration",
    "vcs",
    "observability",
];

// ----------------------------------------------------------- derivation ---

/// Turn a reading plus the authority in force into an engagement.
///
/// `slice_capabilities` is separate from the reading's own confidence because
/// the two failure modes are not symmetric: raising an approval floor on a
/// misread is a small annoyance, while removing a tool the task needed looks
/// to the user like the agent is broken. Callers therefore gate slicing on a
/// higher bar, and low confidence still tightens risk.
pub fn derive(reading: &Reading, authority: &Authority, slice_capabilities: bool) -> Engagement {
    let mut limits = Limits::unrestricted();

    // --- capabilities --------------------------------------------------
    if slice_capabilities {
        // Union over every act the reading covers. A request that is
        // genuinely both a modification and a verification needs both
        // toolsets, and resolving the tie by argmax would silently remove
        // half of what it needs.
        let mut domains: BTreeSet<String> = FLOOR_DOMAINS
            .iter()
            .map(|domain| (*domain).to_string())
            .collect();
        for act in reading.acts() {
            domains.extend(act_domains(act).iter().map(|d| (*d).to_string()));
        }
        // Evidence the reading demands has to come from somewhere. A turn
        // required to cite cannot satisfy that from memory, so requiring
        // citation implies the ability to reach a source — whatever the act
        // was read as. This is the general form of the live-data failure: it
        // was never specific to `Answer`.
        if reading.evidence.rank() >= Evidence::Cited.rank() {
            domains.insert("live-data".into());
            domains.insert("web".into());
        }
        limits.required_domains = crate::limits::DomainSet::only(domains);
    } else {
        // Not confident enough to say what the turn needs: send the
        // orientation floor and let `find_tools` reach the rest (design 68,
        // Principle 6). Stated explicitly so `DomainSet::All` keeps its one
        // meaning — "everything", the disabled-kernel behaviour.
        limits.required_domains = crate::limits::DomainSet::only(FLOOR_DOMAINS.iter().copied());
    }

    // --- modality ------------------------------------------------------
    // A leg that cannot see is not a valid fallback for a vision turn.
    limits.required_modalities = reading.required_modalities();

    // A reading never caps the route ladder, the turn count or the workers.
    // Those caps only ever removed capacity from requests the reader got
    // wrong — a 20-character "fix the failing test" lost its fallback legs
    // and gave its workers two steps — and a reading decides what is loaded,
    // never what is possible (invariant 32).

    // --- closure -------------------------------------------------------
    limits.min_satisfaction = reading.evidence.min_satisfaction();

    // --- posture -------------------------------------------------------
    let hil = derive_hil(reading, authority);
    let delivery = derive_delivery(reading);
    let posture = Posture {
        // Multi-step work runs under a plan; only work that outlives the
        // session earns a durable commitment.
        managed: reading.horizon.rank() >= Horizon::Session.rank(),
        open_commitment: reading.horizon.opens_commitment(),
        // Any act the reading covers, not just the primary one. "migrate …
        // and verify … before deploying" resolves `verify` as primary, and
        // gating on that alone skipped the checkpoint for a turn that plainly
        // modifies and deploys. `Author` is deliberately not one of the acts
        // `requires_execution` covers (see `Act::requires_execution`): most
        // authoring never touches the workspace, and only the request text —
        // which this `Reading`-only derivation cannot see — says whether it
        // names a file. `OutcomeSpec::requires_execution` covers that case
        // once the objective text is known; a caller deciding whether to
        // checkpoint a file-saving authoring turn needs that check too.
        checkpoint_before_effect: reading.acts().iter().any(|act| act.requires_execution())
            && reading.stakes.wants_checkpoint(),
        hil,
        gate_fallback: GateFallback::Deny,
        clarify: derive_clarify(reading),
        delivery,
        demand: DemandHint {
            reasoning_required: matches!(
                reading.act,
                Act::Analyze | Act::Author | Act::Modify | Act::Orchestrate
            ) || reading.horizon.rank() >= Horizon::Session.rank(),
            evidence_required: reading.evidence.rank() >= Evidence::Cited.rank(),
            structured_output: matches!(
                delivery.shape,
                OutputShape::Matrix | OutputShape::Table | OutputShape::Diff
            ),
            direct_reply: slice_capabilities
                && reading.horizon == Horizon::Immediate
                && reading.evidence.rank() < Evidence::Cited.rank()
                && reading
                    .acts()
                    .iter()
                    .all(|act| matches!(act, Act::Converse | Act::Answer)),
        },
        stop: derive_stop(reading),
        context: derive_context(reading),
        epistemic_stance: derive_epistemic_stance(reading),
        note: derive_note(reading, hil),
    };

    let mut engagement = Engagement { limits, posture };
    apply_authority(&mut engagement, reading, authority);
    engagement
}

/// Apply the authority in force to an engagement: the approval ceiling for
/// the reading's stakes, and what happens to a gate nobody can answer.
///
/// One place for every path — a confident reading, a provisional one, and a
/// part the free tiers could not read at all — because authority does not
/// depend on how well the request was read: `manual` means "ask" whether or
/// not the reader understood the words.
///
/// A grant on a commitment is applied separately, once the strands serving
/// it are known (`apply_envelopes`), and covers actions one at a time at the
/// approval gate — an engagement is computed before anyone knows which paths
/// a turn will touch.
pub fn apply_authority(engagement: &mut Engagement, reading: &Reading, authority: &Authority) {
    engagement.limits.approval_ceiling = authority.approval_ceiling(reading.stakes);
    engagement.posture.gate_fallback = authority.gate_fallback(reading.horizon);
}

/// Derive the appropriate epistemic cognitive stance from the reading.
///
/// Ensures the model adopts the right operational and intellectual posture
/// across any domain of work without domain-specific stereotyping.
pub fn derive_epistemic_stance(reading: &Reading) -> EpistemicStance {
    match reading.act {
        Act::Converse => EpistemicStance::Conversational,
        Act::Answer => {
            if reading.evidence.rank() >= Evidence::Cited.rank() {
                EpistemicStance::Analytical
            } else {
                EpistemicStance::DirectAnswer
            }
        }
        Act::Locate => EpistemicStance::Exploratory,
        Act::Analyze => EpistemicStance::Analytical,
        Act::Author => EpistemicStance::Generative,
        Act::Modify | Act::Operate | Act::Govern => EpistemicStance::Operational,
        Act::Verify => EpistemicStance::Diagnostic,
        Act::Orchestrate => {
            if reading.acts().iter().any(|act| act.is_effectful()) {
                EpistemicStance::Operational
            } else {
                EpistemicStance::Exploratory
            }
        }
    }
}

fn derive_hil(reading: &Reading, authority: &Authority) -> HilMode {
    let needs_a_human = reading.stakes.rank() >= Stakes::Costly.rank();
    // Nobody to ask and somewhere to park the question: wait rather than
    // fail. This covers irreversible work too — an irreversible step with
    // nobody present must reach the inbox, not raise a gate that can only
    // time out. "Interrupt" is what happens when someone is here to be
    // interrupted.
    if needs_a_human && authority.gate_fallback(reading.horizon) == GateFallback::Defer {
        return HilMode::Defer;
    }
    // Irreversible stakes always interrupt whatever was delegated.
    if reading.stakes == Stakes::Irreversible {
        return HilMode::Interrupt;
    }
    // Manual autonomy is "propose only": nothing is done first and shown
    // afterwards, so `Review` is never the right mode for it.
    if authority.autonomy == Autonomy::Manual {
        return HilMode::Interrupt;
    }
    // `Envelope` is set by `apply_envelopes` on a strand whose commitment
    // carries a live grant; delegation without one is not a boundary.
    if authority.autonomy == Autonomy::Autonomous {
        return HilMode::Review;
    }
    // Reversible work under a checkpoint is better reviewed than pre-approved.
    if reading.stakes.rank() <= Stakes::Reversible.rank()
        && reading.acts().iter().any(|act| act.is_effectful())
    {
        return HilMode::Review;
    }
    HilMode::Interrupt
}

fn derive_clarify(reading: &Reading) -> ClarifyPolicy {
    // Ambiguity is only worth interrupting for when being wrong would hurt.
    let costly = reading.stakes.rank() >= Stakes::Costly.rank();
    match reading.clarity {
        Clarity::Clear => ClarifyPolicy::Proceed,
        Clarity::Underspecified | Clarity::Ambiguous if costly => ClarifyPolicy::Ask,
        Clarity::Underspecified | Clarity::Ambiguous => ClarifyPolicy::StateAssumption,
    }
}

fn derive_delivery(reading: &Reading) -> DeliveryPosture {
    let shape = match reading.act {
        Act::Converse | Act::Answer => {
            if reading.evidence.rank() >= Evidence::Cited.rank() {
                OutputShape::Sources
            } else {
                OutputShape::Prose
            }
        }
        Act::Locate => OutputShape::Findings,
        Act::Analyze => {
            if reading.evidence.rank() >= Evidence::Cited.rank() {
                OutputShape::Sources
            } else {
                OutputShape::Prose
            }
        }
        Act::Author => OutputShape::Artifact,
        Act::Modify => OutputShape::Diff,
        Act::Operate | Act::Govern => OutputShape::Report,
        Act::Verify => OutputShape::Matrix,
        Act::Orchestrate => OutputShape::Report,
    };
    let cadence = match reading.attendance {
        Attendance::Interactive => Cadence::Live,
        Attendance::Supervised => Cadence::OnCompletion,
        // Overnight work sends one roll-up, not a stream nobody is reading.
        Attendance::Unattended => Cadence::Digest,
    };
    let urgency = match (reading.stakes, reading.attendance) {
        (Stakes::Irreversible, _) => Urgency::Interrupt,
        (Stakes::Costly, Attendance::Unattended) => Urgency::Notify,
        (_, Attendance::Unattended) => Urgency::Quiet,
        _ => Urgency::Notify,
    };
    DeliveryPosture {
        shape,
        cadence,
        urgency,
    }
}

fn derive_stop(reading: &Reading) -> StopProfile {
    if reading.evidence.rank() >= Evidence::Verified.rank() {
        return StopProfile::Verification;
    }
    // The *primary* act, not every contender: a contender is a noun that
    // happens to be a verb somewhere ("what do we know about deploys?"
    // carries an `operate` contender), and gating the stop on it demanded
    // an effect from a question. A request that genuinely has an effectful
    // part is a strand of its own, and the composite stop is the strictest
    // strand's. If the act changes something, an episode that changed
    // nothing did not finish, whatever the model says about it.
    if reading.act.is_effectful() {
        return StopProfile::Effect;
    }
    match reading.act {
        Act::Converse | Act::Answer => StopProfile::Message,
        _ => StopProfile::Inspection,
    }
}

fn derive_context(reading: &Reading) -> ContextProfile {
    if reading.horizon.opens_commitment() {
        return ContextProfile::Full;
    }
    if reading
        .acts()
        .iter()
        .any(|act| matches!(act, Act::Modify | Act::Verify | Act::Operate))
    {
        return ContextProfile::Working;
    }
    match reading.act {
        // A greeting does not need last Tuesday's session searched.
        Act::Converse => ContextProfile::Minimal,
        _ => ContextProfile::Recall,
    }
}

/// The code-owned prompt block, when there is something worth saying.
///
/// Deliberately terse and factual. This is not a place to re-explain the
/// agent's job; it states the few decisions the model cannot otherwise see,
/// and stays silent when there are none.
fn derive_note(reading: &Reading, hil: HilMode) -> Option<String> {
    let mut lines = Vec::new();
    match derive_clarify(reading) {
        ClarifyPolicy::StateAssumption => lines.push(
            "This request is under-specified. Choose the most reasonable reading, \
             state the assumption you made in one sentence, and proceed."
                .to_string(),
        ),
        ClarifyPolicy::Ask => lines.push(
            "This request is ambiguous and the stakes are high enough that guessing \
             is not acceptable. Ask one specific question before acting."
                .to_string(),
        ),
        ClarifyPolicy::Proceed => {}
    }
    if reading.evidence.rank() >= Evidence::Verified.rank() {
        lines.push(format!(
            "Completion requires {} evidence: the runtime, not you, decides whether \
             this is done. Produce a checkable result and do not claim success \
             without one.",
            reading.evidence.min_satisfaction().as_str()
        ));
    } else if reading.evidence == Evidence::Cited {
        lines.push("Cite the sources behind any factual claim.".to_string());
    }
    if hil == HilMode::Defer {
        lines.push(
            "Nobody is available to answer right now. If you need a decision, say so \
             plainly and stop; the question will be queued rather than guessed."
                .to_string(),
        );
    }
    if reading.stakes == Stakes::Irreversible {
        lines.push(if hil == HilMode::Defer {
            "At least one step here cannot be undone and nobody is available to \
             confirm it. Do everything up to that step, then stop and say what \
             needs confirming."
                .to_string()
        } else {
            "At least one step here cannot be undone. Confirm before that step, \
             not after."
                .to_string()
        });
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Modalities a reading needs, exposed for the ladder filter.
pub fn required_modalities(reading: &Reading) -> BTreeSet<Modality> {
    reading.required_modalities()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::axes::{Act, Attendance, Clarity, Evidence, Horizon, Stakes};

    fn reading(act: Act, horizon: Horizon, stakes: Stakes, evidence: Evidence) -> Reading {
        Reading {
            act,
            horizon,
            stakes,
            evidence,
            clarity: Clarity::Clear,
            attendance: Attendance::Interactive,
            confidence: 0.9,
            ..Reading::general()
        }
    }

    /// Invariant 1 across the entire reachable space of readings. If this ever
    /// fails, some derivation is granting rather than restricting.
    #[test]
    fn no_derived_engagement_ever_widens_the_baseline() {
        let baseline = Limits::unrestricted();
        for act in Act::ALL {
            for horizon in Horizon::ALL {
                for stakes in Stakes::ALL {
                    for evidence in Evidence::ALL {
                        for clarity in Clarity::ALL {
                            for attendance in Attendance::ALL {
                                for autonomy in crate::Autonomy::ALL {
                                    for slice in [true, false] {
                                        let mut r = reading(act, horizon, stakes, evidence);
                                        r.clarity = clarity;
                                        r.attendance = attendance;
                                        let authority = Authority {
                                            autonomy,
                                            attendance,
                                        };
                                        let engagement = derive(&r, &authority, slice);
                                        assert!(
                                            engagement.limits.is_at_most(&baseline),
                                            "widened: {act:?}/{horizon:?}/{stakes:?}/\
                                             {evidence:?}/{autonomy:?} slice={slice}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_greeting_gets_no_tools_and_minimal_context() {
        let r = reading(
            Act::Converse,
            Horizon::Immediate,
            Stakes::Inert,
            Evidence::None,
        );
        let engagement = derive(&r, &Authority::default(), true);
        // A greeting asks for nothing beyond the floor that lets it look at
        // what is in front of it.
        assert_eq!(
            engagement.limits.required_domains,
            crate::limits::DomainSet::only(FLOOR_DOMAINS.iter().copied())
        );
        assert_eq!(engagement.posture.context, ContextProfile::Minimal);
        assert!(!engagement.posture.open_commitment);
        assert_eq!(engagement.posture.note, None);
    }

    #[test]
    fn every_act_keeps_the_orientation_floor() {
        for act in Act::ALL {
            let r = reading(act, Horizon::Turn, Stakes::Reversible, Evidence::None);
            let engagement = derive(&r, &Authority::default(), true);
            for domain in FLOOR_DOMAINS {
                assert!(
                    engagement.limits.required_domains.contains(domain),
                    "{act:?} lost `{domain}` and cannot orient itself"
                );
            }
        }
    }

    #[test]
    fn asking_and_searching_can_still_reach_a_live_source() {
        // Regression: `answer` and `locate` were sliced to the orientation
        // floor and `webfetch`. A configured search MCP server (Tavily) was
        // therefore unreachable for exactly the two acts that ask for a
        // fact, and "how is the weather today" came back as "I do not have
        // access to real-time information" with the tool sitting right
        // there, connected and admitted.
        for act in [Act::Answer, Act::Locate] {
            let r = reading(act, Horizon::Turn, Stakes::Inert, Evidence::None);
            let engagement = derive(&r, &Authority::default(), true);
            assert!(
                engagement.limits.required_domains.contains("live-data"),
                "{act:?} cannot reach a live source"
            );
        }
    }

    #[test]
    fn required_citation_implies_reaching_a_source_whatever_the_act() {
        // The general form of the weather failure: it was never specific to
        // `Answer`. A turn obliged to cite cannot satisfy that from memory.
        let r = reading(Act::Verify, Horizon::Turn, Stakes::Inert, Evidence::Cited);
        let engagement = derive(&r, &Authority::default(), true);
        assert!(engagement.limits.required_domains.contains("live-data"));
    }

    #[test]
    fn domain_requirements_narrow_by_intersection() {
        // More domains admits more capabilities, so composing two limits
        // must take the intersection or `meet` would widen.
        let wide = Limits {
            required_domains: crate::limits::DomainSet::only(["web", "live-data", "code-exec"]),
            ..Limits::unrestricted()
        };
        let narrow = Limits {
            required_domains: crate::limits::DomainSet::only(["web"]),
            ..Limits::unrestricted()
        };
        let met = wide.meet(&narrow);
        assert_eq!(
            met.required_domains,
            crate::limits::DomainSet::only(["web"])
        );
        assert!(met.is_at_most(&wide));
        assert!(!wide.is_at_most(&narrow), "widening must not validate");
    }

    #[test]
    fn durable_work_is_managed_opens_a_commitment_and_renders_full_context() {
        let r = reading(
            Act::Modify,
            Horizon::Durable,
            Stakes::Reversible,
            Evidence::Verified,
        );
        let engagement = derive(&r, &Authority::default(), true);
        assert!(engagement.posture.managed);
        assert!(engagement.posture.open_commitment);
        assert_eq!(engagement.posture.context, ContextProfile::Full);
        assert_eq!(engagement.posture.stop, StopProfile::Verification);
        assert_eq!(
            engagement.limits.min_satisfaction,
            crate::Satisfaction::Observed
        );
    }

    #[test]
    fn audited_work_cannot_be_closed_by_the_model_alone() {
        let r = reading(
            Act::Modify,
            Horizon::Session,
            Stakes::Reversible,
            Evidence::Audited,
        );
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(
            engagement.limits.min_satisfaction,
            crate::Satisfaction::Attested
        );
        assert!(!crate::Satisfaction::Asserted.satisfies(engagement.limits.min_satisfaction));
        assert!(!crate::Satisfaction::Cited.satisfies(engagement.limits.min_satisfaction));
    }

    #[test]
    fn low_stakes_ambiguity_states_an_assumption_and_high_stakes_asks() {
        let mut low = reading(Act::Answer, Horizon::Turn, Stakes::Inert, Evidence::None);
        low.clarity = Clarity::Ambiguous;
        assert_eq!(
            derive(&low, &Authority::default(), true).posture.clarify,
            ClarifyPolicy::StateAssumption
        );

        let mut high = reading(
            Act::Operate,
            Horizon::Turn,
            Stakes::Irreversible,
            Evidence::None,
        );
        high.clarity = Clarity::Ambiguous;
        assert_eq!(
            derive(&high, &Authority::default(), true).posture.clarify,
            ClarifyPolicy::Ask
        );
    }

    #[test]
    fn unattended_durable_costly_work_defers_rather_than_interrupting() {
        let r = reading(
            Act::Operate,
            Horizon::Durable,
            Stakes::Costly,
            Evidence::None,
        );
        let authority = Authority {
            autonomy: Autonomy::Delegated,
            attendance: Attendance::Unattended,
        };
        let engagement = derive(&r, &authority, true);
        assert_eq!(engagement.posture.hil, HilMode::Defer);
        assert_eq!(engagement.posture.gate_fallback, GateFallback::Defer);
    }

    #[test]
    fn unattended_work_rolls_up_instead_of_pinging() {
        let mut r = reading(Act::Verify, Horizon::Durable, Stakes::Inert, Evidence::None);
        r.attendance = Attendance::Unattended;
        let delivery = derive(&r, &Authority::default(), true).posture.delivery;
        assert_eq!(delivery.cadence, Cadence::Digest);
        assert_eq!(delivery.urgency, Urgency::Quiet);
    }

    #[test]
    fn irreversible_work_always_interrupts_and_says_so_in_the_prompt() {
        let r = reading(
            Act::Operate,
            Horizon::Turn,
            Stakes::Irreversible,
            Evidence::None,
        );
        let engagement = derive(&r, &Authority::default(), true);
        assert_eq!(engagement.posture.hil, HilMode::Interrupt);
        assert_eq!(engagement.posture.delivery.urgency, Urgency::Interrupt);
        assert!(
            engagement
                .posture
                .note
                .as_deref()
                .is_some_and(|n| n.contains("cannot be undone"))
        );
    }

    #[test]
    fn demand_is_populated_rather_than_left_at_zero() {
        let r = reading(
            Act::Analyze,
            Horizon::Session,
            Stakes::Inert,
            Evidence::Cited,
        );
        let demand = derive(&r, &Authority::default(), true).posture.demand;
        assert!(demand.reasoning_required);
        assert!(demand.evidence_required);
    }

    /// Only a turn that is conversation, or an answer in one reply, read
    /// confidently, is a direct reply; one part that is anything more
    /// makes the whole turn not one.
    #[test]
    fn a_direct_reply_is_conversation_or_an_answer_in_one_reply() {
        let direct = |act, horizon, evidence, confident| {
            let r = reading(act, horizon, Stakes::Inert, evidence);
            derive(&r, &Authority::default(), confident)
        };
        let chat = direct(Act::Converse, Horizon::Immediate, Evidence::None, true);
        let answer = direct(Act::Answer, Horizon::Immediate, Evidence::None, true);
        assert!(chat.posture.demand.direct_reply);
        assert!(answer.posture.demand.direct_reply);
        for other in [
            direct(Act::Answer, Horizon::Immediate, Evidence::None, false),
            direct(Act::Answer, Horizon::Turn, Evidence::None, true),
            direct(Act::Answer, Horizon::Immediate, Evidence::Cited, true),
            direct(Act::Analyze, Horizon::Immediate, Evidence::None, true),
            direct(Act::Author, Horizon::Immediate, Evidence::None, true),
        ] {
            assert!(!other.posture.demand.direct_reply);
            assert!(!chat.meet(&other).posture.demand.direct_reply);
        }
        assert!(chat.meet(&answer).posture.demand.direct_reply);
        assert!(!Engagement::general().posture.demand.direct_reply);
        assert!(!Engagement::orienting().posture.demand.direct_reply);
    }

    /// A reading whose contenders include an effectful act must checkpoint;
    /// the stop rule follows the primary act, and a compound request with a
    /// genuinely effectful part carries it as a strand of its own.
    #[test]
    fn any_effectful_contender_forces_a_checkpoint_but_not_an_effect_stop() {
        let mut r = reading(
            Act::Verify,
            Horizon::Session,
            Stakes::Reversible,
            Evidence::None,
        );
        r.alternate_acts.insert(Act::Modify);
        let engagement = derive(&r, &Authority::default(), true);
        assert!(engagement.posture.checkpoint_before_effect);
        assert_eq!(engagement.posture.stop, StopProfile::Inspection);

        // And a reading with no effectful act does neither.
        let inert = reading(
            Act::Answer,
            Horizon::Turn,
            Stakes::Reversible,
            Evidence::None,
        );
        let engagement = derive(&inert, &Authority::default(), true);
        assert!(!engagement.posture.checkpoint_before_effect);
        assert_eq!(engagement.posture.stop, StopProfile::Message);
    }

    #[test]
    fn meet_of_two_engagements_never_widens_either() {
        let a = derive(
            &reading(
                Act::Modify,
                Horizon::Session,
                Stakes::Costly,
                Evidence::Verified,
            ),
            &Authority::default(),
            true,
        );
        let b = derive(
            &reading(Act::Answer, Horizon::Turn, Stakes::Inert, Evidence::None),
            &Authority::default(),
            true,
        );
        let met = a.meet(&b);
        assert!(met.limits.is_at_most(&a.limits));
        assert!(met.limits.is_at_most(&b.limits));
    }
}
