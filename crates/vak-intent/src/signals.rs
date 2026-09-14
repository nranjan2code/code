//! Deterministic signal extraction — resolver tier 1.
//!
//! Everything here is a pure function of [`Request`]. That is not an
//! aesthetic preference: a decision that narrowed a turn's capabilities has to
//! be reconstructable from the ledger months later, and the only way to
//! promise that is to make the decision depend on nothing but recorded inputs.
//! Tiers 2 and 3 may call a model and are explicitly marked non-reproducible;
//! this tier is not.
//!
//! Signals *vote*, they do not decide. Each observation contributes weight to
//! one or more axis values, the resolver takes the argmax per axis, and
//! confidence comes from the margin between the winner and the runner-up. A
//! request that produces no signals therefore lands at low confidence and
//! falls back to the general engagement, which is exactly the desired
//! behaviour for input this lexicon has never seen.

use serde::{Deserialize, Serialize};

use crate::axes::{Act, Attendance, Clarity, Evidence, Horizon, Modality, Stakes};

/// Where an observation came from. Recorded so an explain view can group by
/// cause, and so a bad lexicon entry is traceable to its category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalKind {
    /// A word or phrase in the request.
    Lexical,
    /// Shape of the text: length, code fences, paths, URLs.
    Structural,
    /// Reference to something not in the request ("this file", "that error").
    Deictic,
    /// Facts about the workspace the request landed in.
    Workspace,
    /// Facts about the conversation so far.
    Session,
    /// Which surface the request arrived on.
    Surface,
    /// Attached media or documents.
    Attachment,
    /// Stated outright by a caller; tier 0.
    Declared,
}

impl SignalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SignalKind::Lexical => "lexical",
            SignalKind::Structural => "structural",
            SignalKind::Deictic => "deictic",
            SignalKind::Workspace => "workspace",
            SignalKind::Session => "session",
            SignalKind::Surface => "surface",
            SignalKind::Attachment => "attachment",
            SignalKind::Declared => "declared",
        }
    }
}

/// One typed observation with the weight it carried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub kind: SignalKind,
    /// Stable identifier, e.g. `verb:refactor` or `structural:code-fence`.
    /// Stable because misread analysis groups by it across months of ledgers.
    pub name: String,
    pub weight: f64,
    /// What was actually seen, for a human reading an explain view.
    pub detail: String,
}

impl Signal {
    fn new(
        kind: SignalKind,
        name: impl Into<String>,
        weight: f64,
        detail: impl Into<String>,
    ) -> Self {
        Signal {
            kind,
            name: name.into(),
            weight,
            detail: detail.into(),
        }
    }
}

/// Which surface a request arrived on.
///
/// This is the strongest available evidence about [`Attendance`], because it
/// is a fact about the deployment rather than a guess about the person: a cron
/// tick genuinely has nobody watching it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    #[default]
    Cli,
    Desktop,
    Server,
    /// A chat gateway: Telegram, Discord, Slack.
    Chat,
    /// A scheduled task or watchdog firing.
    Cron,
    /// A proactive check-in the runtime initiated.
    Heartbeat,
    /// A parent agent dispatching a child.
    Subagent,
}

impl Surface {
    pub fn as_str(self) -> &'static str {
        match self {
            Surface::Cli => "cli",
            Surface::Desktop => "desktop",
            Surface::Server => "server",
            Surface::Chat => "chat",
            Surface::Cron => "cron",
            Surface::Heartbeat => "heartbeat",
            Surface::Subagent => "subagent",
        }
    }

    pub fn parse(value: &str) -> Option<Surface> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cli" | "terminal" => Some(Surface::Cli),
            "desktop" | "tauri" => Some(Surface::Desktop),
            "server" | "http" | "api" => Some(Surface::Server),
            "chat" | "telegram" | "discord" | "slack" | "gateway" => Some(Surface::Chat),
            "cron" | "task" | "schedule" | "watchdog" => Some(Surface::Cron),
            "heartbeat" => Some(Surface::Heartbeat),
            "subagent" | "child" => Some(Surface::Subagent),
            _ => None,
        }
    }

    /// Attendance implied by the surface alone, before anything the request
    /// says. A caller that knows better overrides it on [`Request`].
    pub fn implied_attendance(self) -> Attendance {
        match self {
            Surface::Cli | Surface::Desktop => Attendance::Interactive,
            // A chat message was typed by somebody, but they may well have put
            // the phone down; treat them as reachable, not present.
            Surface::Chat | Surface::Server => Attendance::Supervised,
            Surface::Cron | Surface::Heartbeat => Attendance::Unattended,
            // A child inherits its parent's attendance; this is only the
            // fallback when the parent said nothing.
            Surface::Subagent => Attendance::Unattended,
        }
    }
}

/// An attached input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub modality: Modality,
    pub name: String,
}

/// Facts about the workspace the request landed in.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFacts {
    pub is_repo: bool,
    /// Uncommitted changes present. Raises stakes: work here is recoverable
    /// only if a checkpoint is taken, because git alone will not save it.
    pub has_uncommitted_changes: bool,
    /// Paths touched recently, used to resolve deictic references.
    #[serde(default)]
    pub recent_paths: Vec<String>,
}

/// Facts about the conversation so far.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryFacts {
    /// What the previous turn was read as, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_act: Option<Act>,
    /// Index of this turn within the session; 0 is the first.
    pub turn_index: usize,
    /// A commitment is already open and this request may belong to it.
    pub commitment_open: bool,
}

/// Everything tier 1 is allowed to look at.
#[derive(Debug, Clone, Default)]
pub struct Request<'a> {
    pub text: &'a str,
    pub surface: Surface,
    pub attachments: &'a [Attachment],
    pub workspace: WorkspaceFacts,
    pub history: HistoryFacts,
    /// Attendance the host knows for certain, overriding the surface's guess.
    pub attendance_override: Option<Attendance>,
}

// ------------------------------------------------------------- lexicon ---

/// Verbs that indicate an act. One verb may vote for several acts; the
/// argmax across all signals decides.
///
/// Kept deliberately small and general. This is a *prior*, not a classifier:
/// its job is to be right often enough on plain requests that the paid tiers
/// stay rare, and to abstain rather than guess on anything unusual.
const ACT_VERBS: &[(&str, Act, f64)] = &[
    // Converse
    ("hi", Act::Converse, 1.0),
    ("hello", Act::Converse, 1.0),
    ("hey", Act::Converse, 1.0),
    ("thanks", Act::Converse, 1.0),
    ("thank", Act::Converse, 0.8),
    ("bye", Act::Converse, 1.0),
    // Answer
    ("what", Act::Answer, 0.8),
    ("why", Act::Answer, 0.8),
    ("how", Act::Answer, 0.8),
    ("explain", Act::Answer, 0.9),
    ("describe", Act::Answer, 0.7),
    ("summarize", Act::Answer, 0.7),
    ("summarise", Act::Answer, 0.7),
    ("tell", Act::Answer, 0.5),
    // Locate
    ("find", Act::Locate, 0.9),
    ("search", Act::Locate, 0.9),
    ("where", Act::Locate, 0.7),
    ("locate", Act::Locate, 1.0),
    ("list", Act::Locate, 0.7),
    ("grep", Act::Locate, 1.0),
    ("look", Act::Locate, 0.5),
    // Analyze
    ("analyze", Act::Analyze, 1.0),
    ("analyse", Act::Analyze, 1.0),
    ("research", Act::Analyze, 0.9),
    ("investigate", Act::Analyze, 0.9),
    ("compare", Act::Analyze, 0.8),
    ("evaluate", Act::Analyze, 0.8),
    ("assess", Act::Analyze, 0.8),
    ("diagnose", Act::Analyze, 0.8),
    ("review", Act::Analyze, 0.7),
    // Author
    ("write", Act::Author, 0.8),
    ("draft", Act::Author, 0.9),
    ("create", Act::Author, 0.7),
    ("generate", Act::Author, 0.7),
    ("design", Act::Author, 0.7),
    ("compose", Act::Author, 0.8),
    ("plan", Act::Author, 0.5),
    ("add", Act::Author, 0.5),
    // Modify
    ("fix", Act::Modify, 0.9),
    ("refactor", Act::Modify, 1.0),
    ("update", Act::Modify, 0.8),
    ("change", Act::Modify, 0.8),
    ("edit", Act::Modify, 0.9),
    ("rename", Act::Modify, 0.9),
    ("remove", Act::Modify, 0.8),
    ("delete", Act::Modify, 0.7),
    ("implement", Act::Modify, 0.7),
    ("migrate", Act::Modify, 0.8),
    ("patch", Act::Modify, 0.8),
    ("debug", Act::Modify, 0.7),
    // Operate. Several of these are common nouns too ("a blog post", "the
    // release plan"), so they carry less weight than an unambiguous verb and
    // rely on the leading-position bonus when they really are the action.
    ("deploy", Act::Operate, 0.8),
    ("release", Act::Operate, 0.6),
    ("publish", Act::Operate, 0.9),
    ("send", Act::Operate, 0.9),
    ("email", Act::Operate, 0.8),
    ("post", Act::Operate, 0.5),
    ("install", Act::Operate, 0.8),
    ("restart", Act::Operate, 0.8),
    ("schedule", Act::Operate, 0.7),
    ("watch", Act::Operate, 0.7),
    ("monitor", Act::Operate, 0.8),
    ("click", Act::Operate, 0.8),
    ("open", Act::Operate, 0.3),
    ("browse", Act::Operate, 0.6),
    ("pay", Act::Operate, 0.9),
    // Verify
    ("test", Act::Verify, 0.8),
    ("verify", Act::Verify, 1.0),
    ("check", Act::Verify, 0.8),
    ("validate", Act::Verify, 0.9),
    ("confirm", Act::Verify, 0.7),
    ("audit", Act::Verify, 0.8),
    ("reproduce", Act::Verify, 0.8),
    // Orchestrate
    ("orchestrate", Act::Orchestrate, 1.0),
    ("coordinate", Act::Orchestrate, 0.8),
    ("delegate", Act::Orchestrate, 0.9),
    ("parallel", Act::Orchestrate, 0.6),
    // Govern
    ("configure", Act::Govern, 0.8),
    ("remember", Act::Govern, 0.8),
    ("forget", Act::Govern, 0.8),
    ("permission", Act::Govern, 0.7),
    ("setting", Act::Govern, 0.6),
    ("settings", Act::Govern, 0.6),
];

/// Words that raise stakes regardless of the act.
const STAKES_WORDS: &[(&str, Stakes, f64)] = &[
    ("production", Stakes::Irreversible, 1.0),
    ("prod", Stakes::Irreversible, 0.8),
    ("live", Stakes::Irreversible, 0.5),
    ("customer", Stakes::Irreversible, 0.7),
    ("customers", Stakes::Irreversible, 0.7),
    ("payment", Stakes::Irreversible, 0.9),
    ("invoice", Stakes::Irreversible, 0.7),
    ("irreversible", Stakes::Irreversible, 1.0),
    ("permanently", Stakes::Irreversible, 0.9),
    ("everyone", Stakes::Irreversible, 0.6),
    ("expensive", Stakes::Costly, 0.7),
    ("budget", Stakes::Costly, 0.6),
    ("quota", Stakes::Costly, 0.6),
];

/// Words that raise the evidence standard.
const EVIDENCE_WORDS: &[(&str, Evidence, f64)] = &[
    ("cite", Evidence::Cited, 1.0),
    ("cites", Evidence::Cited, 0.9),
    ("citation", Evidence::Cited, 1.0),
    ("sources", Evidence::Cited, 0.8),
    ("source", Evidence::Cited, 0.4),
    ("evidence", Evidence::Cited, 0.6),
    ("prove", Evidence::Verified, 0.8),
    ("proof", Evidence::Verified, 0.7),
    ("make sure", Evidence::Verified, 0.8),
    ("ensure", Evidence::Verified, 0.7),
    ("passing", Evidence::Verified, 0.7),
    ("green", Evidence::Verified, 0.4),
    ("audited", Evidence::Audited, 1.0),
    ("sign off", Evidence::Audited, 0.9),
    ("acceptance", Evidence::Audited, 0.8),
];

/// Phrases implying the work outlives this turn.
const HORIZON_PHRASES: &[(&str, Horizon, f64)] = &[
    ("every day", Horizon::Durable, 1.0),
    ("every night", Horizon::Durable, 1.0),
    ("every morning", Horizon::Durable, 1.0),
    ("every month", Horizon::Durable, 1.0),
    ("each day", Horizon::Durable, 1.0),
    ("each week", Horizon::Durable, 1.0),
    ("nightly", Horizon::Durable, 0.9),
    ("monthly", Horizon::Durable, 0.9),
    ("continuously", Horizon::Durable, 0.9),
    ("whenever", Horizon::Durable, 0.7),
    ("every week", Horizon::Durable, 1.0),
    ("every hour", Horizon::Durable, 1.0),
    ("daily", Horizon::Durable, 0.9),
    ("weekly", Horizon::Durable, 0.9),
    ("hourly", Horizon::Durable, 0.9),
    ("keep watching", Horizon::Durable, 1.0),
    ("keep an eye", Horizon::Durable, 0.9),
    ("from now on", Horizon::Durable, 0.9),
    ("ongoing", Horizon::Durable, 0.8),
    ("until", Horizon::Durable, 0.4),
    ("over the next", Horizon::Durable, 0.7),
    ("step by step", Horizon::Session, 0.7),
    ("then", Horizon::Session, 0.3),
    ("after that", Horizon::Session, 0.6),
    ("and then", Horizon::Session, 0.6),
    ("first", Horizon::Session, 0.3),
    ("finally", Horizon::Session, 0.4),
];

/// Deictic markers: the request points at something it does not contain.
const DEICTIC_WORDS: &[&str] = &[
    "this", "that", "it", "these", "those", "here", "there", "again", "same",
];

// ------------------------------------------------------------ scoring ---

/// An axis value that can name itself, so vote tallies can break ties
/// deterministically without depending on the enum's declaration order.
///
/// `Ord` would have been shorter and is deliberately not used: the axes spell
/// their `rank` out precisely so that reordering variants cannot change a
/// safety decision, and a derived `Ord` would quietly reintroduce exactly that
/// coupling here.
pub trait AxisValue: Copy + PartialEq {
    fn axis_name(self) -> &'static str;

    /// Position on an ordered axis, or `None` for a categorical one.
    ///
    /// This distinction is load-bearing. `Act` is categorical: `modify` and
    /// `answer` are rival explanations, and evidence for one really is evidence
    /// against the other. `Stakes` is *ordered*: a signal saying "reversible"
    /// does not argue against "irreversible", it agrees with it more weakly.
    /// Scoring an ordered axis by argmax over rivals made corroborating
    /// evidence *reduce* confidence — a dirty working tree voting `reversible`
    /// dragged "deploy to production" below the acceptance floor and dropped
    /// it to the general engagement, losing the irreversible reading entirely.
    fn axis_rank(self) -> Option<u8> {
        None
    }
}

impl AxisValue for Act {
    fn axis_name(self) -> &'static str {
        self.as_str()
    }
}
impl AxisValue for Horizon {
    fn axis_name(self) -> &'static str {
        self.as_str()
    }

    fn axis_rank(self) -> Option<u8> {
        Some(self.rank())
    }
}
impl AxisValue for Stakes {
    fn axis_name(self) -> &'static str {
        self.as_str()
    }

    fn axis_rank(self) -> Option<u8> {
        Some(self.rank())
    }
}
impl AxisValue for Evidence {
    fn axis_name(self) -> &'static str {
        self.as_str()
    }

    fn axis_rank(self) -> Option<u8> {
        Some(self.rank())
    }
}
impl AxisValue for Clarity {
    fn axis_name(self) -> &'static str {
        self.as_str()
    }
}

/// Weighted votes for one axis.
#[derive(Debug, Clone)]
pub struct Votes<T: AxisValue> {
    tally: Vec<(T, f64)>,
}

impl<T: AxisValue> Default for Votes<T> {
    fn default() -> Self {
        Votes { tally: Vec::new() }
    }
}

impl<T: AxisValue> Votes<T> {
    pub fn add(&mut self, value: T, weight: f64) {
        match self.tally.iter_mut().find(|(v, _)| *v == value) {
            Some((_, score)) => *score += weight,
            None => self.tally.push((value, weight)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tally.is_empty()
    }

    /// Every value that received weight, strongest first. Ties break by name
    /// so the ordering is total and stable across runs.
    pub fn ranked(&self) -> Vec<(T, f64)> {
        let mut ranked = self.tally.clone();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.axis_name().cmp(b.0.axis_name()))
        });
        ranked
    }

    /// Minimum weight before a signal may escalate an ordered axis. Filters
    /// out the incidental 0.3-weight hints so a stray conjunction cannot
    /// promote a one-liner to durable multi-day work.
    const ESCALATION_FLOOR: f64 = 0.5;

    /// Every value scoring within `band` of the winner, strongest first.
    ///
    /// Used for the act axis, where two readings being close is often not
    /// ambiguity to resolve but a request that genuinely spans both: "fix the
    /// failing test" is a `modify` and a `verify`, and picking one loses the
    /// other's tools.
    pub fn contenders(&self, band: f64) -> Vec<T> {
        let ranked = self.ranked();
        let Some((_, best)) = ranked.first().copied() else {
            return Vec::new();
        };
        if best <= 0.0 {
            return Vec::new();
        }
        ranked
            .into_iter()
            .filter(|(_, weight)| *weight >= best * (1.0 - band))
            .map(|(value, _)| value)
            .collect()
    }

    /// Winner and a [0,1] confidence.
    ///
    /// Two rules, because the axes are not all the same shape:
    ///
    /// * **Categorical** (`Act`, `Clarity`): argmax with confidence from the
    ///   margin over the runner-up. "Two readings scored 5.0 each" is genuine
    ///   ambiguity worth escalating; "one scored 0.6 and nothing else scored"
    ///   is a weak but unambiguous read.
    /// * **Ordered** (`Stakes`, `Horizon`, `Evidence`): the highest level with
    ///   real support wins, and lower levels corroborate rather than compete.
    ///   Confidence comes from that level's own weight. Taking the maximum is
    ///   also the safe direction on every ordered axis here — more caution,
    ///   a stricter proof standard, a longer horizon.
    pub fn winner(&self) -> Option<(T, f64)> {
        let ranked = self.ranked();
        let (best, best_score) = ranked.first().copied()?;
        if best_score <= 0.0 {
            return None;
        }
        if best.axis_rank().is_some() {
            let (highest, weight) = ranked
                .iter()
                .filter(|(_, weight)| *weight >= Self::ESCALATION_FLOOR)
                .max_by_key(|(value, _)| value.axis_rank().unwrap_or(0))
                .copied()
                // Everything was below the escalation floor: fall back to the
                // strongest signal rather than abstaining, since weak evidence
                // is still evidence.
                .unwrap_or((best, best_score));
            let mass = (weight / 1.5).min(1.0);
            return Some((highest, (0.7 + mass * 0.3).clamp(0.0, 1.0)));
        }
        let runner_up = ranked.get(1).map(|(_, s)| *s).unwrap_or(0.0);
        // How much better the winner is, as a fraction of itself — not its
        // share of the total. Share-of-total punishes any second signal
        // regardless of how weak: "research how to…" scored `analyze` at 0.9
        // against `answer` at 0.4 and still read as near-ambiguous, because
        // 0.5/1.3 is only 0.38. A 2.25x lead is not a coin flip.
        let margin = 1.0 - (runner_up / best_score).min(1.0);
        // A lone weak signal should not read as certainty either, so the
        // margin is damped by how much evidence there was at all.
        let mass = (best_score / 1.5).min(1.0);
        Some((best, (margin * 0.7 + mass * 0.3).clamp(0.0, 1.0)))
    }
}

/// Everything tier 1 concluded, before the resolver turns it into a reading.
#[derive(Debug, Clone, Default)]
pub struct Extraction {
    pub signals: Vec<Signal>,
    pub act: Votes<Act>,
    pub horizon: Votes<Horizon>,
    pub stakes: Votes<Stakes>,
    /// Stakes implied by the *environment* rather than by the request.
    ///
    /// Kept separate because it is only relevant to acts that actually touch
    /// something: a dirty working tree makes an edit riskier, and says nothing
    /// whatsoever about the risk of saying hello. Folding it in unconditionally
    /// made every greeting in a work-in-progress repository read as
    /// `reversible`.
    pub stakes_from_environment: Votes<Stakes>,
    pub evidence: Votes<Evidence>,
    pub clarity: Votes<Clarity>,
    pub input_modalities: Vec<Modality>,
    pub output_modalities: Vec<Modality>,
    pub attendance: Attendance,
    pub domains: Vec<String>,
}

/// Split into lowercase alphanumeric words, preserving order.
fn words(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// Crude English de-inflection, enough to match a lexicon of bare verbs.
///
/// Not a stemmer and not trying to be one: it only strips the three suffixes
/// that make a request's actual verb invisible to an exact-match lexicon.
/// Without it "before deploying to production" contributed no act signal at
/// all, because the lexicon has `deploy` and the text has `deploying`.
fn stems(word: &str) -> Vec<&str> {
    let mut out = vec![word];
    for suffix in ["ing", "ed", "es", "s"] {
        if let Some(stem) = word.strip_suffix(suffix)
            && stem.len() >= 3
        {
            out.push(stem);
            // Doubled consonant before -ing/-ed: "running" -> "runn" -> "run".
            if matches!(suffix, "ing" | "ed")
                && let Some(trimmed) = stem.strip_suffix(stem.chars().last().unwrap_or(' '))
                && trimmed.len() >= 3
                && stem.chars().rev().nth(1) == stem.chars().last()
            {
                out.push(trimmed);
            }
        }
    }
    out
}

/// Whether any token in the request matches `word`, allowing inflection.
fn token_position(tokens: &[String], word: &str) -> Option<usize> {
    tokens.iter().position(|token| stems(token).contains(&word))
}

/// Strip prompt scaffolding and runner control blocks before extracting intent.
fn clean_request_text(raw: &str) -> String {
    let mut text = raw.to_string();
    let tags = [
        "conversation_thread",
        "context_summary",
        "intent",
        "work_contract",
        "managed_work",
        "context_packet",
        "system_reminder",
        "runtime_guidance",
        "scratchpad",
    ];
    for tag in tags {
        let open_pattern = format!("<{tag}");
        let close_pattern = format!("</{tag}>");
        while let Some(start) = text.find(&open_pattern) {
            if let Some(end_offset) = text[start..].find(&close_pattern) {
                let end = start + end_offset + close_pattern.len();
                text.replace_range(start..end, "");
            } else {
                text.truncate(start);
                break;
            }
        }
    }
    while let Some(start) = text.find("[Scheduled-run context:") {
        if let Some(end_offset) = text[start..].find(']') {
            let end = start + end_offset + 1;
            text.replace_range(start..end, "");
        } else {
            text.truncate(start);
            break;
        }
    }
    text
}

/// Tier 1. A pure function of `request`.
pub fn extract(request: &Request<'_>) -> Extraction {
    let mut out = Extraction {
        attendance: request
            .attendance_override
            .unwrap_or_else(|| request.surface.implied_attendance()),
        ..Extraction::default()
    };
    let cleaned_text = clean_request_text(request.text);
    let lower = cleaned_text.to_ascii_lowercase();
    let tokens = words(&cleaned_text);

    out.signals.push(Signal::new(
        SignalKind::Surface,
        format!("surface:{}", request.surface.as_str()),
        0.0,
        format!(
            "arrived on {} ⇒ {} by default",
            request.surface.as_str(),
            out.attendance.as_str()
        ),
    ));

    // --- lexical -------------------------------------------------------
    // English requests are overwhelmingly imperative, so the leading token is
    // far more likely to be the actual verb than a later one. Without this,
    // "write a blog post" scores `author` and `operate` equally — because
    // "post" is a verb somewhere, just not here.
    const IMPERATIVE_BONUS: f64 = 1.6;
    for (word, act, weight) in ACT_VERBS {
        let Some(position) = token_position(&tokens, word) else {
            continue;
        };
        let leading = position == 0;
        let weight = if leading {
            weight * IMPERATIVE_BONUS
        } else {
            *weight
        };
        out.act.add(*act, weight);
        out.signals.push(Signal::new(
            SignalKind::Lexical,
            format!("verb:{word}"),
            weight,
            if leading {
                format!("`{word}` (leading verb) ⇒ {}", act.as_str())
            } else {
                format!("`{word}` ⇒ {}", act.as_str())
            },
        ));
    }
    for (word, stakes, weight) in STAKES_WORDS {
        if token_position(&tokens, word).is_some() {
            out.stakes.add(*stakes, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("stakes:{word}"),
                *weight,
                format!("`{word}` ⇒ {}", stakes.as_str()),
            ));
        }
    }
    for (phrase, evidence, weight) in EVIDENCE_WORDS {
        let hit = if phrase.contains(' ') {
            lower.contains(phrase)
        } else {
            tokens.iter().any(|t| t == phrase)
        };
        if hit {
            out.evidence.add(*evidence, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("evidence:{phrase}"),
                *weight,
                format!("`{phrase}` ⇒ {}", evidence.as_str()),
            ));
        }
    }
    for (phrase, horizon, weight) in HORIZON_PHRASES {
        if lower.contains(phrase) {
            out.horizon.add(*horizon, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("horizon:{}", phrase.replace(' ', "-")),
                *weight,
                format!("`{phrase}` ⇒ {}", horizon.as_str()),
            ));
        }
    }

    // --- structural ----------------------------------------------------
    let length = cleaned_text.trim().len();
    if length <= 24 && !tokens.is_empty() {
        out.horizon.add(Horizon::Immediate, 0.6);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "length:short",
            0.6,
            format!("{length} chars ⇒ immediate"),
        ));
    } else if length >= 400 {
        out.horizon.add(Horizon::Session, 0.7);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "length:long",
            0.7,
            format!("{length} chars ⇒ session"),
        ));
    }
    if cleaned_text.contains("```") {
        out.act.add(Act::Modify, 0.3);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "code-fence",
            0.3,
            "fenced code present",
        ));
    }
    if lower.contains("http://") || lower.contains("https://") {
        out.act.add(Act::Analyze, 0.3);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "url",
            0.3,
            "a URL to fetch",
        ));
    }
    let path_like = cleaned_text
        .split_whitespace()
        .any(|w| w.contains('/') && w.contains('.') && !w.contains("://"));
    if path_like {
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "path-mention",
            0.4,
            "a file path is named",
        ));
        out.act.add(Act::Modify, 0.2);
        out.clarity.add(Clarity::Clear, 0.5);
    }
    // Enumerated steps are the honest multi-step marker: a list the user
    // wrote themselves, not two stray conjunctions.
    let step_markers = lower.matches("\n- ").count()
        + lower.matches("\n1.").count()
        + lower.matches("\n2.").count();
    if step_markers >= 2 {
        out.horizon.add(Horizon::Session, 0.9);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "enumerated-steps",
            0.9,
            format!("{step_markers} enumerated items"),
        ));
    }

    // --- deictic -------------------------------------------------------
    let deictic: Vec<&str> = DEICTIC_WORDS
        .iter()
        .copied()
        .filter(|w| tokens.iter().any(|t| t == w))
        .collect();
    if !deictic.is_empty() {
        // Pointing at something is only ambiguous when there is no history to
        // point at. Mid-conversation it is ordinary and clear.
        if request.history.turn_index == 0 && request.workspace.recent_paths.is_empty() {
            out.clarity.add(Clarity::Ambiguous, 0.7);
            out.signals.push(Signal::new(
                SignalKind::Deictic,
                "unresolved-reference",
                0.7,
                format!("`{}` with no prior context", deictic.join("`, `")),
            ));
        } else {
            out.clarity.add(Clarity::Clear, 0.4);
            out.signals.push(Signal::new(
                SignalKind::Deictic,
                "resolved-reference",
                0.4,
                format!("`{}` resolvable from context", deictic.join("`, `")),
            ));
        }
    }

    // --- workspace -----------------------------------------------------
    if request.workspace.is_repo {
        out.domains.push("engineering".into());
        out.signals.push(Signal::new(
            SignalKind::Workspace,
            "git-repo",
            0.3,
            "workspace is a git repository",
        ));
    }
    if request.workspace.has_uncommitted_changes {
        // Uncommitted work is not recoverable from git alone, so a modifying
        // turn here is riskier than the same turn on a clean tree. Applied
        // only to effectful acts; see `stakes_from_environment`.
        out.stakes_from_environment.add(Stakes::Reversible, 0.5);
        out.signals.push(Signal::new(
            SignalKind::Workspace,
            "dirty-tree",
            0.5,
            "uncommitted changes present",
        ));
    }

    // --- session -------------------------------------------------------
    if let Some(previous) = request.history.previous_act {
        // Continuity is a real prior, but a weak one: people change subject.
        out.act.add(previous, 0.25);
        out.signals.push(Signal::new(
            SignalKind::Session,
            format!("continues:{}", previous.as_str()),
            0.25,
            format!("previous turn read as {}", previous.as_str()),
        ));
    }
    if request.history.commitment_open {
        out.horizon.add(Horizon::Session, 0.4);
        out.signals.push(Signal::new(
            SignalKind::Session,
            "commitment-open",
            0.4,
            "a commitment is already open here",
        ));
    }

    // --- attachments ---------------------------------------------------
    for attachment in request.attachments {
        out.input_modalities.push(attachment.modality);
        out.signals.push(Signal::new(
            SignalKind::Attachment,
            format!("input:{}", attachment.modality.as_str()),
            1.0,
            format!(
                "attached {} `{}`",
                attachment.modality.as_str(),
                attachment.name
            ),
        ));
        if attachment.modality != Modality::Text {
            out.act.add(Act::Analyze, 0.4);
        }
    }
    if !out.input_modalities.contains(&Modality::Text) {
        out.input_modalities.push(Modality::Text);
    }
    out.output_modalities.push(Modality::Text);

    out.input_modalities.sort();
    out.input_modalities.dedup();
    out.output_modalities.sort();
    out.output_modalities.dedup();
    out.domains.sort();
    out.domains.dedup();
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn request<'a>(text: &'a str) -> Request<'a> {
        Request {
            text,
            surface: Surface::Cli,
            attachments: &[],
            workspace: WorkspaceFacts::default(),
            history: HistoryFacts::default(),
            attendance_override: None,
        }
    }

    #[test]
    fn extraction_is_a_pure_function_of_its_input() {
        let a = extract(&request("refactor the parser and run the tests"));
        let b = extract(&request("refactor the parser and run the tests"));
        assert_eq!(a.signals, b.signals);
        assert_eq!(a.act.winner().map(|w| w.0), b.act.winner().map(|w| w.0));
    }

    #[test]
    fn a_greeting_reads_as_converse_and_immediate() {
        let extraction = extract(&request("hi"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Converse));
        assert_eq!(
            extraction.horizon.winner().map(|w| w.0),
            Some(Horizon::Immediate)
        );
    }

    #[test]
    fn production_words_raise_stakes_whatever_the_verb() {
        let extraction = extract(&request("deploy the service to production"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Operate));
        assert_eq!(
            extraction.stakes.winner().map(|w| w.0),
            Some(Stakes::Irreversible)
        );
    }

    #[test]
    fn unknown_input_abstains_rather_than_guessing() {
        let extraction = extract(&request("zorble the frobnicator"));
        assert!(extraction.act.is_empty(), "lexicon invented a reading");
    }

    /// The `is_managed_work_request` heuristic this replaces fired on any two
    /// of `and`/`then`/`first`. A sentence that merely contains conjunctions
    /// is not multi-day work, and reading it as such is what made the old
    /// gate useless.
    #[test]
    fn conjunctions_alone_do_not_imply_a_long_horizon() {
        let extraction = extract(&request("explain what this and that mean"));
        let horizon = extraction.horizon.winner().map(|w| w.0);
        assert_ne!(horizon, Some(Horizon::Durable));
        assert_ne!(horizon, Some(Horizon::Session));
    }

    #[test]
    fn recurrence_phrases_imply_a_durable_horizon() {
        let extraction = extract(&request("check the cloud bill every day and alert me"));
        assert_eq!(
            extraction.horizon.winner().map(|w| w.0),
            Some(Horizon::Durable)
        );
    }

    #[test]
    fn a_cron_surface_is_unattended_and_a_cli_is_interactive() {
        let mut cron = request("run the nightly sweep");
        cron.surface = Surface::Cron;
        assert_eq!(extract(&cron).attendance, Attendance::Unattended);
        assert_eq!(extract(&request("hi")).attendance, Attendance::Interactive);
    }

    #[test]
    fn an_explicit_attendance_override_beats_the_surface_guess() {
        let mut req = request("do the thing");
        req.surface = Surface::Cron;
        req.attendance_override = Some(Attendance::Interactive);
        assert_eq!(extract(&req).attendance, Attendance::Interactive);
    }

    #[test]
    fn attachments_become_required_input_modalities() {
        let attachments = vec![Attachment {
            modality: Modality::Image,
            name: "screenshot.png".into(),
        }];
        let mut req = request("what is wrong here");
        req.attachments = &attachments;
        let extraction = extract(&req);
        assert!(extraction.input_modalities.contains(&Modality::Image));
        assert!(extraction.input_modalities.contains(&Modality::Text));
    }

    #[test]
    fn a_bare_pronoun_is_ambiguous_only_without_context() {
        let cold = extract(&request("fix this"));
        assert_eq!(cold.clarity.winner().map(|w| w.0), Some(Clarity::Ambiguous));

        let mut warm = request("fix this");
        warm.history.turn_index = 3;
        warm.history.previous_act = Some(Act::Locate);
        assert_eq!(
            extract(&warm).clarity.winner().map(|w| w.0),
            Some(Clarity::Clear)
        );
    }

    #[test]
    fn categorical_confidence_reflects_margin_not_just_score() {
        let mut tied: Votes<Act> = Votes::default();
        tied.add(Act::Answer, 2.0);
        tied.add(Act::Modify, 2.0);
        let (_, tied_confidence) = tied.winner().unwrap();

        let mut clear: Votes<Act> = Votes::default();
        clear.add(Act::Modify, 2.0);
        let (_, clear_confidence) = clear.winner().unwrap();

        assert!(tied_confidence < clear_confidence);
    }

    /// On an ordered axis a weaker level *corroborates* a stronger one. This
    /// is the bug that dropped "deploy the billing service to production" to
    /// the general engagement: a dirty working tree voted `reversible`, which
    /// was scored as a rival to `irreversible` and dragged the whole reading
    /// below the acceptance floor.
    #[test]
    fn a_lower_level_never_argues_against_a_higher_one() {
        let mut alone: Votes<Stakes> = Votes::default();
        alone.add(Stakes::Irreversible, 1.0);
        let (_, alone_confidence) = alone.winner().unwrap();

        let mut corroborated: Votes<Stakes> = Votes::default();
        corroborated.add(Stakes::Irreversible, 1.0);
        corroborated.add(Stakes::Reversible, 0.5);
        let (value, confidence) = corroborated.winner().unwrap();

        assert_eq!(value, Stakes::Irreversible);
        assert!(
            confidence >= alone_confidence,
            "agreement at a lower level reduced confidence: {confidence} < {alone_confidence}"
        );
    }

    /// Ordered axes take the highest supported level, because on every one of
    /// them that is the cautious direction.
    #[test]
    fn ordered_axes_take_the_highest_supported_level() {
        let mut stakes: Votes<Stakes> = Votes::default();
        stakes.add(Stakes::Inert, 3.0);
        stakes.add(Stakes::Irreversible, 0.9);
        assert_eq!(stakes.winner().map(|w| w.0), Some(Stakes::Irreversible));

        let mut evidence: Votes<Evidence> = Votes::default();
        evidence.add(Evidence::Cited, 2.0);
        evidence.add(Evidence::Audited, 0.8);
        assert_eq!(evidence.winner().map(|w| w.0), Some(Evidence::Audited));
    }

    /// A stray weak hint must not promote work to a longer horizon; that is
    /// how the keyword heuristic this replaces went wrong.
    #[test]
    fn weak_hints_do_not_escalate_an_ordered_axis() {
        let mut horizon: Votes<Horizon> = Votes::default();
        horizon.add(Horizon::Immediate, 0.6);
        horizon.add(Horizon::Durable, 0.3);
        assert_eq!(horizon.winner().map(|w| w.0), Some(Horizon::Immediate));
    }
}
