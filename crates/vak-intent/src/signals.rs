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
//! one or more axis values, and [`Votes::winner`] turns the tally into a
//! reading. A request that produces no signals lands at low confidence and
//! falls back to the orienting engagement, which is exactly the desired
//! behaviour for input this lexicon has never seen.
//!
//! # What may vote
//!
//! Words vote only from the part of the message that is a request. Two
//! things are set aside first:
//!
//! * **Pasted material** — fenced code, and runs of lines that are not prose
//!   (log lines, CSV rows, stack frames, indented code). A log line that says
//!   "worker deploy job 12 for customer account" is data, not an instruction
//!   to deploy anything; before this rule a 300-line log produced 301
//!   "parts", an irreversible reading and a 26 KB note.
//! * **Statements** — a clause that starts with its subject ("the deploy
//!   failed", "it crashes on empty input") describes the situation; its
//!   nouns are not verbs. Such a clause keeps its stakes, evidence and
//!   recency words as context, but casts no act vote.
//!
//! What remains is read by clause role: an imperative ("fix the parser"), a
//! question ("did the email send?"), a request phrased as a wish ("I want
//! the parser refactored"), or social talk ("thanks!").

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::axes::{Act, Attendance, Clarity, Evidence, Horizon, Modality, Stakes};

/// Where an observation came from. Recorded so an explain view can group by
/// cause, and so a bad lexicon entry is traceable to its category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalKind {
    /// A word or phrase in the request.
    Lexical,
    /// Shape of the text: code fences, paths, URLs, pasted material.
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
    Worker,
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
            Surface::Worker => "worker",
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
            "worker" | "child" | "subagent" => Some(Surface::Worker),
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
            Surface::Worker => Attendance::Unattended,
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
}

/// Facts about the conversation so far.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HistoryFacts {
    /// What the previous turn was read as, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_act: Option<Act>,
    /// How many messages the conversation held before this request; `0`
    /// means this is its first.
    pub turn_index: usize,
    /// Threads from earlier turns that are still open, for strand lineage.
    #[serde(default)]
    pub open_threads: Vec<crate::strand::ThreadFact>,
}

/// Everything tier 1 is allowed to look at.
#[derive(Debug, Clone, Default)]
pub struct Request<'a> {
    pub text: &'a str,
    /// A unique id the host mints for this turn (a UUIDv7). Strand ids are
    /// `{turn_id}.{index}`, so a thread — and the commitment keyed by it — is
    /// unique across turns, sessions and workspaces. Empty only in tests and
    /// previews, where a positional id stands in.
    pub turn_id: &'a str,
    pub surface: Surface,
    pub attachments: &'a [Attachment],
    pub workspace: WorkspaceFacts,
    pub history: HistoryFacts,
    /// Attendance the host knows for certain, overriding the surface's guess.
    pub attendance_override: Option<Attendance>,
    /// Set when the request arrived under an explicit `/goal fix` or
    /// `/goal replace` command. The resolver never infers these.
    pub lineage_hint: Option<crate::strand::LineageHint>,
}

// ------------------------------------------------------------- lexicon ---

/// Verbs that indicate an act. One verb may vote for several acts; the tally
/// across all signals decides.
///
/// Kept deliberately small and general. This is a *prior*, not a classifier:
/// its job is to be right often enough on plain requests that the paid tiers
/// stay rare, and to abstain rather than guess on anything unusual.
const ACT_VERBS: &[(&str, Act, f64)] = &[
    // Answer. The wh-words are here because a question's head is one.
    ("what", Act::Answer, 0.8),
    ("why", Act::Answer, 0.8),
    ("how", Act::Answer, 0.8),
    ("explain", Act::Answer, 0.9),
    ("describe", Act::Answer, 0.7),
    ("summarize", Act::Answer, 0.7),
    ("summarise", Act::Answer, 0.7),
    ("tell", Act::Answer, 0.5),
    // Conversational delivery verbs describe the answer the user wants in
    // this chat; they do not imply a workspace artifact. Treating
    // "present it as a card" as Author made the stop gate demand a file or
    // edit receipt for an ordinary answer.
    ("give", Act::Answer, 0.8),
    ("present", Act::Answer, 0.7),
    // "Show three lighthouses as a table" asks for an answer drawn on
    // screen, like "present"; read as Locate it set the exploratory
    // stance ("map the landscape") and the turn went to fetch web pages
    // for facts it was only asked to show.
    ("show", Act::Answer, 0.7),
    ("translate", Act::Answer, 0.7),
    ("define", Act::Answer, 0.7),
    ("clarify", Act::Answer, 0.6),
    // Locate
    ("find", Act::Locate, 0.9),
    ("search", Act::Locate, 0.9),
    ("where", Act::Locate, 0.7),
    ("locate", Act::Locate, 1.0),
    ("list", Act::Locate, 0.7),
    ("grep", Act::Locate, 1.0),
    ("look", Act::Locate, 0.5),
    // Browsing reads the web; it does not act on it.
    ("browse", Act::Locate, 0.6),
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
    ("calculate", Act::Analyze, 0.8),
    ("compute", Act::Analyze, 0.7),
    ("measure", Act::Analyze, 0.7),
    ("estimate", Act::Analyze, 0.7),
    ("inspect", Act::Analyze, 0.7),
    ("examine", Act::Analyze, 0.8),
    // Author
    ("write", Act::Author, 0.8),
    ("draft", Act::Author, 0.9),
    ("create", Act::Author, 0.7),
    ("generate", Act::Author, 0.7),
    // General creation language, without assuming a domain or file type.
    ("make", Act::Author, 0.7),
    ("build", Act::Author, 0.7),
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
    ("modify", Act::Modify, 0.9),
    ("append", Act::Modify, 0.9),
    ("prepend", Act::Modify, 0.9),
    ("insert", Act::Modify, 0.8),
    ("replace", Act::Modify, 0.8),
    ("rewrite", Act::Modify, 0.8),
    ("reformat", Act::Modify, 0.9),
    ("convert", Act::Modify, 0.7),
    ("tidy", Act::Modify, 0.7),
    ("optimize", Act::Modify, 0.8),
    ("optimise", Act::Modify, 0.8),
    ("upgrade", Act::Modify, 0.8),
    ("downgrade", Act::Modify, 0.8),
    ("revert", Act::Modify, 0.9),
    ("rebase", Act::Modify, 0.9),
    ("merge", Act::Modify, 0.7),
    // Commits and moves are local and undoable; both are also nouns
    // ("the last commit"), so they carry less weight than a pure verb.
    ("commit", Act::Modify, 0.6),
    ("move", Act::Modify, 0.6),
    ("reorder", Act::Modify, 0.8),
    ("rearrange", Act::Modify, 0.8),
    ("amend", Act::Modify, 0.8),
    ("revise", Act::Modify, 0.8),
    ("tweak", Act::Modify, 0.8),
    ("uncomment", Act::Modify, 0.9),
    ("enable", Act::Modify, 0.6),
    ("disable", Act::Modify, 0.6),
    // Operate: reaching outside the workspace. Several of these are common
    // nouns too ("a blog post", "the release plan"), so they carry less
    // weight than an unambiguous verb and rely on the head-position bonus
    // when they really are the action.
    ("deploy", Act::Operate, 0.8),
    ("release", Act::Operate, 0.6),
    ("publish", Act::Operate, 0.9),
    ("send", Act::Operate, 0.9),
    ("email", Act::Operate, 0.8),
    ("post", Act::Operate, 0.5),
    ("install", Act::Operate, 0.8),
    ("restart", Act::Operate, 0.8),
    ("schedule", Act::Operate, 0.7),
    ("click", Act::Operate, 0.8),
    ("pay", Act::Operate, 0.9),
    ("notify", Act::Operate, 0.9),
    ("remind", Act::Operate, 0.9),
    ("alert", Act::Operate, 0.8),
    // Pushing, uploading and buying reach someone else's system and cannot
    // be quietly taken back. "Run" and "execute" are deliberately absent:
    // "run the tests" is a check, and reading it as an operation would make
    // it irreversible.
    ("push", Act::Operate, 0.6),
    ("upload", Act::Operate, 0.8),
    ("submit", Act::Operate, 0.7),
    ("purchase", Act::Operate, 0.9),
    ("buy", Act::Operate, 0.8),
    // Verify. Watching and monitoring observe; they change nothing, so they
    // are checks rather than operations on the world.
    ("test", Act::Verify, 0.8),
    ("verify", Act::Verify, 1.0),
    ("check", Act::Verify, 0.8),
    ("validate", Act::Verify, 0.9),
    ("confirm", Act::Verify, 0.7),
    ("audit", Act::Verify, 0.8),
    ("reproduce", Act::Verify, 0.8),
    ("watch", Act::Verify, 0.7),
    ("monitor", Act::Verify, 0.8),
    ("compile", Act::Verify, 0.7),
    ("lint", Act::Verify, 0.8),
    ("double-check", Act::Verify, 0.85),
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

/// Greetings and thanks. Not verbs, so they never take the imperative bonus:
/// "hey, what did we decide yesterday?" is a question with a greeting in
/// front of it, and reading it as `converse` gave it minimal context.
const SOCIAL_WORDS: &[(&str, f64)] = &[
    ("hi", 1.0),
    ("hello", 1.0),
    ("hey", 1.0),
    ("hiya", 1.0),
    ("yo", 0.8),
    ("thanks", 1.0),
    ("thank", 0.8),
    ("thx", 0.8),
    ("cheers", 0.8),
    ("bye", 1.0),
    ("goodbye", 1.0),
];

/// Whole social phrases. Their words are consumed, so the `how` in "how are
/// you" does not also vote for a factual answer.
const SOCIAL_PHRASES: &[(&str, f64)] = &[
    ("how are you", 1.3),
    ("how is it going", 1.3),
    ("how s it going", 1.3),
    ("what s up", 1.3),
    ("good morning", 1.3),
    ("good afternoon", 1.3),
    ("good evening", 1.3),
    ("good night", 1.3),
    ("nice to meet you", 1.3),
    ("nice to see you", 1.3),
    ("long time no see", 1.3),
    ("see you later", 1.3),
    ("see you soon", 1.3),
    ("thank you", 1.0),
];

/// Acknowledgements stripped from the front of a clause. They carry no
/// request of their own; alone they are social.
const ACKNOWLEDGEMENTS: &[&str] = &["ok", "okay", "cool", "great", "sure", "alright", "right"];

/// Polite and conversational openers stripped so the operational verb lands
/// in head position.
const PREAMBLES: &[&str] = &[
    "could you please",
    "can you please",
    "would you please",
    "would you mind",
    "could you",
    "can you",
    "would you",
    "will you",
    "can u",
    "can we",
    "could we",
    "please",
    "pls",
    "plz",
    "kindly",
    "just",
    "go ahead and",
    "help me to",
    "help me",
    "i would like you to",
    "i d like you to",
    "i want you to",
    "i need you to",
    "i want to",
    "i need to",
    "we need to",
    "let s",
    "lets",
];

/// Sequencing words at the front of a clause. Structure, not verbs.
const LEAD_FILLERS: &[&str] = &[
    "first", "firstly", "second", "secondly", "third", "thirdly", "next", "now", "then", "finally",
    "lastly", "also", "and", "so", "actually", "well", "hmm", "oh", "btw",
];

/// Openers that make a statement a request: "I want the parser refactored",
/// "what I need is for you to refactor the module". Their words are
/// consumed: the `what` of a pseudo-cleft does not ask a question.
const DESIRE_PREFIXES: &[&str] = &[
    "what i need",
    "what we need",
    "what i want",
    "what we want",
    "what i d like",
    "all i need",
    "i want",
    "i d like",
    "i would like",
    "we want",
    "we d like",
    "we would like",
    "i need",
    "we need",
    "i wish",
];

/// Who the requester is. "Alert me", "send me the report": delivering a
/// result to the person asking is an answer, not an irreversible operation
/// on someone else.
const REQUESTER: &[&str] = &["me", "us", "myself", "ourselves"];

/// A question's head: wh-words vote through [`ACT_VERBS`]; these auxiliaries
/// open a question when a subject follows ("did the email send", "can I
/// deploy on Friday", "is it done").
const AUX_WORDS: &[&str] = &[
    "is", "are", "was", "were", "am", "does", "did", "has", "have", "had", "can", "could", "will",
    "would", "should", "shall", "may", "might", "isn", "aren", "wasn", "weren", "doesn", "didn",
    "hasn", "haven", "hadn", "couldn", "wouldn", "shouldn", "won", "do",
];
const WH_WORDS: &[&str] = &[
    "what", "why", "how", "where", "when", "which", "who", "whom", "whose",
];

/// Words that follow an auxiliary in a question. `do` asks only before a
/// person ("do you know"), because "do this every day" is an instruction.
const QUESTION_SUBJECTS: &[&str] = &[
    "i",
    "you",
    "we",
    "they",
    "he",
    "she",
    "it",
    "this",
    "that",
    "these",
    "those",
    "the",
    "there",
    "my",
    "our",
    "your",
    "their",
    "his",
    "her",
    "its",
    "any",
    "anyone",
    "anything",
    "anybody",
    "everyone",
    "everything",
    "someone",
    "something",
    "all",
    "each",
    "every",
];
const PERSONS: &[&str] = &["i", "you", "we", "they", "he", "she"];

/// Verbs that, in second place, make the first word a subject: "build fails
/// on CI", "deploy failed", "value moved here". A statement describes the
/// situation; its first word is not an instruction even when it could be a
/// verb somewhere else.
const STATEMENT_VERBS: &[&str] = &[
    "is", "are", "was", "were", "has", "have", "had", "does", "did", "doesn", "didn", "isn",
    "wasn", "won", "can", "cannot", "should", "will", "would", "fails", "failed", "failing",
    "occurs", "occurred", "happens", "happened", "crashes", "crashed", "crashing", "breaks",
    "broke", "broken", "works", "worked", "returns", "returned", "throws", "threw", "shows",
    "showed", "says", "said", "seems", "seemed", "looks", "looked", "keeps", "kept", "stopped",
    "started", "hangs", "hung", "errors", "errored", "times", "timed", "panics", "panicked",
    "moved", "passes", "passed", "runs", "ran", "went", "gets", "got",
];

/// A clause that starts with one of these is a statement unless a verb heads
/// one of its sub-clauses: "it crashes on empty input", "the deploy failed".
const SUBJECT_STARTERS: &[&str] = &[
    "i",
    "we",
    "you",
    "they",
    "he",
    "she",
    "it",
    "this",
    "that",
    "these",
    "those",
    "the",
    "a",
    "an",
    "my",
    "our",
    "your",
    "their",
    "his",
    "her",
    "its",
    "there",
    "here",
    "some",
    "all",
    "each",
    "every",
    "no",
    "none",
    "one",
    "someone",
    "something",
    "everyone",
    "everything",
    "nothing",
    "nobody",
    "anyone",
    "anything",
];

/// Words that raise stakes when the act touches something. They name the
/// target environment or the nature of the operation, never a topic:
/// "customer", "payment", "live" and "everyone" read "rename the Customer
/// struct" and "fix the payment form validation" as irreversible, capped
/// approval at `ask`, and told the model to confirm an edit.
const STAKES_WORDS: &[(&str, Stakes, f64)] = &[
    ("production", Stakes::Irreversible, 1.0),
    ("prod", Stakes::Irreversible, 0.8),
    ("irreversible", Stakes::Irreversible, 1.0),
    ("irreversibly", Stakes::Irreversible, 1.0),
    ("permanently", Stakes::Irreversible, 0.9),
    ("force push", Stakes::Irreversible, 0.9),
    // The flag spellings of the same commands: `git push --force`,
    // `git push -f`, `git reset --hard` discard history or uncommitted work.
    ("push force", Stakes::Irreversible, 0.9),
    ("push f", Stakes::Irreversible, 0.9),
    ("reset hard", Stakes::Irreversible, 0.9),
    ("rm rf", Stakes::Irreversible, 1.0),
    ("drop table", Stakes::Irreversible, 0.9),
    ("drop database", Stakes::Irreversible, 1.0),
    // Launching: "we go live in an hour". Bare `live` is a topic word or a
    // recency word ([`LIVE_WORD`]), never stakes.
    ("go live", Stakes::Irreversible, 0.8),
    ("going live", Stakes::Irreversible, 0.8),
];

/// Words that raise the evidence standard.
const EVIDENCE_WORDS: &[(&str, Evidence, f64)] = &[
    ("cite", Evidence::Cited, 1.0),
    ("cites", Evidence::Cited, 0.9),
    ("citation", Evidence::Cited, 1.0),
    ("citations", Evidence::Cited, 1.0),
    // Matched exactly (see `EXACT_EVIDENCE_WORDS`): inflection would fold
    // "source code" into it.
    ("sources", Evidence::Cited, 0.8),
    ("evidence", Evidence::Cited, 0.6),
    ("prove", Evidence::Verified, 0.8),
    ("proof", Evidence::Verified, 0.7),
    ("passing", Evidence::Verified, 0.7),
    ("sign off", Evidence::Audited, 0.9),
    ("signed off", Evidence::Audited, 0.9),
    ("acceptance test", Evidence::Audited, 0.8),
    ("acceptance testing", Evidence::Audited, 0.8),
];

/// Evidence words whose inflections mean something else. `sources` must not
/// match `source` — "the source code" is not a citation request.
const EXACT_EVIDENCE_WORDS: &[&str] = &["sources"];

/// "Make sure" and "ensure" demand machine-checkable proof only when the
/// thing to be sure of is checkable: "make sure the tests pass" is a
/// predicate, "make sure it rhymes" is a style instruction, and reading the
/// latter as `verified` made the stop gate demand a shell receipt for a poem.
const ASSURANCE_PHRASES: &[(&str, f64)] = &[("make sure", 0.8), ("ensure", 0.7)];
const CHECKABLE_WORDS: &[&str] = &[
    "test",
    "tests",
    "testing",
    "pass",
    "passes",
    "passing",
    "build",
    "builds",
    "compile",
    "compiles",
    "compiling",
    "lint",
    "ci",
    "green",
    "works",
    "working",
    "run",
    "runs",
];

/// `live` is two words. As an adjective or adverb ("a live score", "is it
/// live") it means *current*, and is a recency word like "right now": a
/// request for a fact with it asks for a value observed this turn. As a verb
/// ("we live in the city", "my kids live with me") it means *reside*, and
/// says nothing about time: measured live, "we live in the city" in a
/// weekend-planning request set `live-data`, the freshness check refused the
/// plan card, and the person got no plan at all. "Go live" — launching —
/// is a stakes phrase, not a recency word ([`STAKES_WORDS`]).
///
/// The verb reading is recognised from its neighbours, never from a topic:
/// a subject or auxiliary right before it, or — unless a copula or "go"
/// right before it makes it the adjective — a residence preposition right
/// after it. Only the bare form `live` is ever read in the current sense;
/// `lives`, `lived` and `living` are always the verb.
const LIVE_WORD: &str = "live";
/// A word before `live` that makes it the verb "reside".
const RESIDE_SUBJECTS: &[&str] = &[
    "i", "we", "you", "they", "he", "she", "who", "people", "both", "all", "to", "can", "could",
    "would", "will", "might", "should", "must", "not", "never", "t", "d", "ll",
];
/// A word after `live` that makes it "reside", unless [`LIVE_COPULAS`]
/// precedes it.
const RESIDE_PREPOSITIONS: &[&str] = &[
    "in", "near", "with", "nearby", "abroad", "alone", "together", "close", "downtown", "outside",
];
/// A word before `live` that keeps it the adjective ("is live in prod",
/// "go live in an hour").
const LIVE_COPULAS: &[&str] = &[
    "is", "are", "was", "were", "be", "been", "being", "s", "re", "go", "goes", "going", "went",
    "gone", "now",
];

/// Whether some occurrence of `live` in `tokens` means *current*, not
/// *reside*.
fn live_means_current(tokens: &[String]) -> bool {
    tokens.iter().enumerate().any(|(i, token)| {
        if token != LIVE_WORD {
            return false;
        }
        let prev = i.checked_sub(1).map(|p| tokens[p].as_str());
        let next = tokens.get(i + 1).map(String::as_str);
        let copula = prev.is_some_and(|w| LIVE_COPULAS.contains(&w));
        let subject = prev.is_some_and(|w| RESIDE_SUBJECTS.contains(&w));
        let preposition = next.is_some_and(|w| RESIDE_PREPOSITIONS.contains(&w));
        copula || !(subject || preposition)
    })
}

/// Temporal deixis: the request asks for a value as it stands *now*, which
/// no model knows from training and which must therefore be observed on this
/// turn (`live-data` domain, docs/design/68-context-engine.md §7). References
/// to time, never to a topic. Whether it applies also depends on the act —
/// only a request for a fact asks for a current value — which the resolver
/// decides once the act is known.
const RECENCY_PHRASES: &[(&str, f64)] = &[
    ("right now", 1.0),
    ("currently", 0.9),
    ("current", 0.8),
    ("as of today", 1.0),
    ("as of now", 1.0),
    ("today", 0.6),
    ("tonight", 0.7),
    ("this morning", 0.8),
    ("this week", 0.5),
    ("latest", 0.7),
    ("recent", 0.6),
    ("recently", 0.6),
    ("real time", 0.8),
    ("at the moment", 0.9),
    ("up to date", 0.7),
    // Only in the sense of *current* ([`live_means_current`]).
    (LIVE_WORD, 0.5),
];

/// Nouns that make a recency word local rather than live: "the current
/// directory", "the latest changes", "live reload". The workspace, the
/// conversation and the running session are observed with local tools and
/// hold no value a model could carry over stale from training.
const LOCAL_NOUNS: &[&str] = &[
    "directory",
    "dir",
    "folder",
    "file",
    "files",
    "path",
    "branch",
    "commit",
    "commits",
    "diff",
    "changes",
    "change",
    "working",
    "workspace",
    "project",
    "repo",
    "repository",
    "codebase",
    "code",
    "implementation",
    "function",
    "method",
    "class",
    "module",
    "line",
    "lines",
    "cursor",
    "selection",
    "tab",
    "window",
    "page",
    "screen",
    "session",
    "sessions",
    "conversation",
    "conversations",
    "chat",
    "chats",
    "message",
    "messages",
    "thread",
    "threads",
    "context",
    "task",
    "plan",
    "step",
    "turn",
    "user",
    "config",
    "configuration",
    "settings",
    "setup",
    "build",
    "test",
    "tests",
    "reload",
    "preview",
    "server",
    "share",
    "coding",
    "edit",
    "editing",
    "demo",
    "mode",
    "state",
    // The agent's own state is read with its own tools, not retrieved from
    // the world: "what are you holding right now", "your current plan".
    "you",
    "your",
    "yours",
    "yourself",
    "commitment",
    "commitments",
    "tasks",
    "plans",
    "inbox",
    "memory",
    "notes",
    "reminders",
];

/// The runtime tells the model the time and date on every turn, so asking
/// for them needs no retrieval.
const TIME_WORDS: &[&str] = &[
    "time", "date", "day", "weekday", "clock", "timezone", "hour", "year", "month",
];

/// A recency adjective can describe a value in the world ("latest weather")
/// or a prior conversational answer ("the latest weather answer you
/// reported"). The latter is a history lookup and must not trigger the
/// current-value freshness gate. These are structural cues: require both a
/// response noun and an explicit reference to what the assistant said before
/// suppressing live-data intent, so ordinary requests for the latest news or
/// weather still require retrieval.
fn refers_to_prior_response(tokens: &Tokens) -> bool {
    let has_response_noun = ["answer", "response", "reply", "message", "turn"]
        .iter()
        .any(|word| tokens.words.iter().any(|token| token == word));
    let has_past_reference = [
        "earlier", "previous", "prior", "last", "before", "said", "report", "told", "answered",
    ]
    .iter()
    .any(|word| tokens.words.iter().any(|token| token == word));
    has_response_noun && has_past_reference
}

/// Phrases implying the work outlives this turn. Sequencing words ("then",
/// "after that") are not here: they separate the parts of one request, which
/// is what strands are for, not a claim that the work spans sessions.
const HORIZON_PHRASES: &[(&str, Horizon, f64)] = &[
    ("every day", Horizon::Durable, 1.0),
    ("every night", Horizon::Durable, 1.0),
    ("every morning", Horizon::Durable, 1.0),
    ("every evening", Horizon::Durable, 1.0),
    ("every month", Horizon::Durable, 1.0),
    ("every week", Horizon::Durable, 1.0),
    ("every hour", Horizon::Durable, 1.0),
    ("each day", Horizon::Durable, 1.0),
    ("each week", Horizon::Durable, 1.0),
    ("each month", Horizon::Durable, 1.0),
    ("every monday", Horizon::Durable, 1.0),
    ("every tuesday", Horizon::Durable, 1.0),
    ("every wednesday", Horizon::Durable, 1.0),
    ("every thursday", Horizon::Durable, 1.0),
    ("every friday", Horizon::Durable, 1.0),
    ("every saturday", Horizon::Durable, 1.0),
    ("every sunday", Horizon::Durable, 1.0),
    ("every weekday", Horizon::Durable, 1.0),
    ("every weekend", Horizon::Durable, 1.0),
    ("nightly", Horizon::Durable, 0.9),
    ("monthly", Horizon::Durable, 0.9),
    ("daily", Horizon::Durable, 0.9),
    ("weekly", Horizon::Durable, 0.9),
    ("hourly", Horizon::Durable, 0.9),
    ("continuously", Horizon::Durable, 0.9),
    ("whenever", Horizon::Durable, 0.7),
    ("keep watching", Horizon::Durable, 1.0),
    ("keep an eye", Horizon::Durable, 0.9),
    ("from now on", Horizon::Durable, 0.9),
    ("ongoing", Horizon::Durable, 0.8),
    ("until", Horizon::Durable, 0.4),
    ("over the next", Horizon::Durable, 0.7),
];

/// Recurrence words that are adjectives after a determiner ("the nightly
/// job") and adverbs otherwise ("check it nightly"). Only the adverb votes.
const RECURRENCE_ADJECTIVES: &[&str] = &["nightly", "daily", "weekly", "monthly", "hourly"];
const DETERMINERS: &[&str] = &[
    "the", "a", "an", "this", "that", "our", "my", "your", "its", "their", "each", "of",
];

/// Deictic markers: the request points at something it does not contain.
pub(crate) const DEICTIC_WORDS: &[&str] = &[
    "this", "that", "it", "these", "those", "here", "there", "again", "same",
];

/// Pronouns that leave a request with no object of its own when they end
/// it: "fix it", "deploy that". "Write a function that parses dates" uses
/// `that` as a relative pronoun and points at nothing.
const BARE_PRONOUNS: &[&str] = &["it", "this", "that", "these", "those"];

/// Words before a verb that still leave it heading its clause.
const HEAD_PREDECESSORS: &[&str] = &[
    "and", "then", "also", "plus", "next", "after", "or", "now", "please", "first", "finally", "so",
];

const IMPERATIVE_BONUS: f64 = 1.6;

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
    /// out the incidental 0.3-weight hints so a stray word cannot promote a
    /// one-liner to durable multi-day work.
    pub(crate) const ESCALATION_FLOOR: f64 = 0.5;

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
            .filter(|(_, weight)| {
                // The winner is always retained. Others must clear the
                // absolute floor, and either sit within the band of the
                // winner or carry strong independent signal.
                *weight >= best
                    || (*weight >= Self::ESCALATION_FLOOR
                        && (*weight >= best * (1.0 - band) || *weight >= 1.0))
            })
            .map(|(value, _)| value)
            .collect()
    }

    /// Winner and a [0,1] confidence.
    ///
    /// Two rules, because the axes are not all the same shape:
    ///
    /// * **Categorical** (`Act`, `Clarity`): argmax with confidence from the
    ///   margin over the runner-up.
    /// * **Ordered** (`Stakes`, `Horizon`, `Evidence`): the highest level with
    ///   real support wins, and lower levels corroborate rather than compete.
    ///   Taking the maximum is also the safe direction on every ordered axis
    ///   here — more caution, a stricter proof standard, a longer horizon.
    pub fn winner(&self) -> Option<(T, f64)> {
        let ranked = self.ranked();
        let (best, best_score) = ranked.first().copied()?;
        if best_score <= 0.0 {
            return None;
        }
        if best.axis_rank().is_some() {
            // Everything below the escalation floor abstains: a vote too weak
            // to escalate on its own is not evidence of the level it names.
            let (highest, weight) = ranked
                .iter()
                .filter(|(_, weight)| *weight >= Self::ESCALATION_FLOOR)
                .max_by_key(|(value, _)| value.axis_rank().unwrap_or(0))
                .copied()?;
            let mass = (weight / 1.5).min(1.0);
            return Some((highest, (0.7 + mass * 0.3).clamp(0.0, 1.0)));
        }
        let runner_up = ranked.get(1).map(|(_, s)| *s).unwrap_or(0.0);
        // How much better the winner is, as a fraction of itself — not its
        // share of the total. Share-of-total punishes any second signal
        // regardless of how weak.
        let margin = 1.0 - (runner_up / best_score).min(1.0);
        // A lone weak signal should not read as certainty either, so the
        // margin is damped by how much evidence there was at all.
        let mass = (best_score / 1.5).min(1.0);
        Some((best, (margin * 0.7 + mass * 0.3).clamp(0.0, 1.0)))
    }
}

/// Everything tier 1 concluded about one part of a request, before the
/// resolver turns it into a reading.
#[derive(Debug, Clone, Default)]
pub struct Extraction {
    pub signals: Vec<Signal>,
    pub act: Votes<Act>,
    pub horizon: Votes<Horizon>,
    /// Stakes stated by the request's own words. Applied only to effectful
    /// acts: a question about production has no blast radius.
    pub stakes_from_words: Votes<Stakes>,
    /// Stakes implied by the *environment* rather than by the request.
    ///
    /// Kept separate because it is only relevant to acts that actually touch
    /// something: a dirty working tree makes an edit riskier, and says nothing
    /// whatsoever about the risk of saying hello.
    pub stakes_from_environment: Votes<Stakes>,
    pub evidence: Votes<Evidence>,
    pub clarity: Votes<Clarity>,
    pub input_modalities: Vec<Modality>,
    pub output_modalities: Vec<Modality>,
    pub attendance: Attendance,
    pub domains: Vec<String>,
    /// Domains implied by the *environment* rather than by the request's own
    /// words. A git repository says nothing about the subject of a question
    /// asked inside it, so these apply only to effectful acts.
    pub domains_from_environment: Vec<String>,
    /// The strongest temporal reference to a current value, if any. The
    /// resolver turns it into the `live-data` domain only when the act asks
    /// for a fact; "refactor the current implementation" wants no retrieval.
    pub recency: Option<(String, f64)>,
    /// The part points at something it does not contain ("it", "that").
    pub deictic: bool,
}

// ------------------------------------------------------------- tokens ---

/// Split into lowercase alphanumeric words, preserving order.
fn words(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (i, ch) in chars.iter().copied().enumerate() {
        // Keep an internal hyphen with its compound. Splitting
        // "latency-check" into two tokens made the noun `check` look like an
        // imperative verb and caused the stop gate to launch unrelated tools.
        // Compound verbs that are meaningful actions are listed explicitly.
        let internal_hyphen = ch == '-'
            && i > 0
            && i + 1 < chars.len()
            && chars[i - 1].is_ascii_alphanumeric()
            && chars[i + 1].is_ascii_alphanumeric();
        if ch.is_ascii_alphanumeric() || internal_hyphen {
            current.push(ch);
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Crude English de-inflection, enough to match a lexicon of bare verbs.
///
/// Not a stemmer and not trying to be one: it only strips the suffixes that
/// make a request's actual verb invisible to an exact-match lexicon.
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

/// A lexicon word with its de-inflected forms, computed once.
struct Target {
    word: &'static str,
    stems: Vec<&'static str>,
}

impl Target {
    fn new(word: &'static str) -> Self {
        Target {
            word,
            stems: stems(word),
        }
    }
}

/// A clause's tokens with their de-inflected forms, computed once, so
/// matching the whole lexicon against a clause allocates nothing per pair.
struct Tokens {
    words: Vec<String>,
}

impl Tokens {
    fn new(text: &str) -> Self {
        Tokens { words: words(text) }
    }

    fn len(&self) -> usize {
        self.words.len()
    }

    fn get(&self, index: usize) -> Option<&str> {
        self.words.get(index).map(String::as_str)
    }

    /// Whether the token at `index` is `target`, allowing inflection and
    /// silent-`e` alterations ("writing" → "write", "deploying" → "deploy").
    fn matches(&self, index: usize, target: &Target) -> bool {
        let Some(token) = self.get(index) else {
            return false;
        };
        if token == target.word {
            return true;
        }
        let token_stems = stems(token);
        if token_stems.contains(&target.word) {
            return true;
        }
        if let Some(without_e) = target.word.strip_suffix('e')
            && token_stems.contains(&without_e)
        {
            return true;
        }
        for target_stem in &target.stems {
            if token == *target_stem || token_stems.contains(target_stem) {
                return true;
            }
            if token.strip_suffix('e') == Some(*target_stem) {
                return true;
            }
        }
        false
    }

    /// First position matching `target`, skipping consumed tokens.
    fn position(&self, target: &Target, consumed: &[bool]) -> Option<usize> {
        (0..self.len())
            .find(|&i| !consumed.get(i).copied().unwrap_or(false) && self.matches(i, target))
    }

    /// Whether `phrase` (whole words) occurs, and where.
    fn phrase_at(&self, phrase: &[&str]) -> Option<usize> {
        if phrase.is_empty() || phrase.len() > self.len() {
            return None;
        }
        (0..=self.len() - phrase.len())
            .find(|&start| (0..phrase.len()).all(|i| self.words[start + i] == phrase[i]))
    }
}

static VERB_TARGETS: LazyLock<Vec<(Target, Act, f64)>> = LazyLock::new(|| {
    ACT_VERBS
        .iter()
        .map(|(word, act, weight)| (Target::new(word), *act, *weight))
        .collect()
});

fn is_lexicon_verb(tokens: &Tokens, index: usize) -> bool {
    VERB_TARGETS
        .iter()
        .any(|(target, _, _)| tokens.matches(index, target))
}

fn split_phrase(phrase: &'static str) -> Vec<&'static str> {
    phrase.split(' ').collect()
}

// ------------------------------------------------------------ clauses ---

/// What a clause is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClauseKind {
    /// A verb heads it: "fix the parser", "every day, check the bill".
    Imperative,
    /// Asks something: a wh-word or an auxiliary before a subject, or a
    /// trailing question mark.
    Question,
    /// A request phrased as a wish ("I want the parser refactored"), or one
    /// led by a verb the lexicon does not know ("sync the files daily").
    Request,
    /// Greetings and thanks and nothing else.
    Social,
    /// Describes the situation: "it crashes on empty input".
    Statement,
}

impl ClauseKind {
    /// Whether this clause asks for work, and so may stand as a strand.
    pub(crate) fn is_work(self) -> bool {
        matches!(
            self,
            ClauseKind::Imperative | ClauseKind::Question | ClauseKind::Request
        )
    }
}

/// One clause, read once.
pub(crate) struct ClauseRead {
    pub text: String,
    pub boundary: crate::strand::Boundary,
    pub kind: ClauseKind,
    tokens: Tokens,
    /// Tokens a social phrase already accounted for.
    consumed: Vec<bool>,
    /// Converse weight from greetings stripped off the front.
    greeting: f64,
    /// Signals for what was stripped, for the explain view.
    stripped: Vec<String>,
    question_mark: bool,
}

impl ClauseRead {
    /// Whether this clause asks for work of its own, and so starts a part: a
    /// question, or an instruction or wish that names something to do.
    /// Everything else is context for the part beside it.
    pub(crate) fn starts_part(&self) -> bool {
        match self.kind {
            ClauseKind::Question => true,
            ClauseKind::Imperative | ClauseKind::Request => self.has_act_votes(),
            ClauseKind::Social | ClauseKind::Statement => false,
        }
    }

    /// Whether this clause votes for any act at all.
    fn has_act_votes(&self) -> bool {
        match self.kind {
            ClauseKind::Statement | ClauseKind::Social => {
                (1..self.tokens.len()).any(|i| self.is_head(i) && is_lexicon_verb(&self.tokens, i))
            }
            _ => (0..self.tokens.len())
                .any(|i| !self.consumed[i] && is_lexicon_verb(&self.tokens, i)),
        }
    }

    /// Whether the token at `index` heads its (sub-)clause: first word, or
    /// right after a conjunction or sequencing word, or after punctuation.
    fn is_head(&self, index: usize) -> bool {
        if index == 0 {
            return true;
        }
        if let Some(prev) = self.tokens.get(index - 1)
            && HEAD_PREDECESSORS.contains(&prev)
        {
            return true;
        }
        if index > 1
            && self.tokens.get(index - 2) == Some("and")
            && self.tokens.get(index - 1) == Some("then")
        {
            return true;
        }
        let Some(target) = self.tokens.get(index) else {
            return false;
        };
        let lower = self.text.to_ascii_lowercase();
        for (at, _) in lower.match_indices(target) {
            let before = &lower[..at];
            let after = &lower[at + target.len()..];
            let word_start =
                before.is_empty() || before.ends_with(|c: char| !c.is_ascii_alphanumeric());
            let word_end =
                after.is_empty() || after.starts_with(|c: char| !c.is_ascii_alphanumeric());
            if word_start && word_end && before.trim_end().ends_with([',', ';', ':', '.', '\n']) {
                return true;
            }
        }
        false
    }
}

fn starts_with_words(tokens: &[String], phrase: &str) -> Option<usize> {
    let needle: Vec<&str> = phrase.split(' ').collect();
    (tokens.len() >= needle.len() && needle.iter().zip(tokens).all(|(a, b)| a == b))
        .then_some(needle.len())
}

/// Drop `count` leading words from `text`, keeping the original spelling of
/// what remains.
fn drop_leading_words(text: &str, count: usize) -> &str {
    let mut seen = 0usize;
    let mut in_word = false;
    for (i, c) in text.char_indices() {
        let is_word = c.is_ascii_alphanumeric();
        if is_word && !in_word {
            if seen == count {
                return &text[i..];
            }
            seen += 1;
        }
        in_word = is_word;
    }
    ""
}

/// Read one clause: strip what leads it, then decide its kind.
pub(crate) fn read_clause(
    text: &str,
    boundary: crate::strand::Boundary,
    question_mark: bool,
) -> ClauseRead {
    let mut rest = text.trim();
    let mut greeting = 0.0f64;
    let mut stripped = Vec::new();
    let mut acknowledged = false;
    // Greetings, acknowledgements, polite openers and sequencing words, in
    // any order, until none applies.
    loop {
        let tokens = words(rest);
        let Some(first) = tokens.first() else {
            break;
        };
        if let Some((phrase, weight)) = SOCIAL_PHRASES
            .iter()
            .find(|(phrase, _)| starts_with_words(&tokens, phrase).is_some())
            .filter(|(phrase, _)| starts_with_words(&tokens, phrase) != Some(tokens.len()))
        {
            // A social phrase leading a longer clause is a greeting.
            if let Some(n) = starts_with_words(&tokens, phrase) {
                greeting = greeting.max(*weight);
                stripped.push(format!("greeting `{phrase}`"));
                rest = drop_leading_words(rest, n);
                continue;
            }
        }
        if tokens.len() > 1
            && let Some((word, weight)) = SOCIAL_WORDS.iter().find(|(word, _)| word == first)
        {
            greeting = greeting.max(*weight);
            stripped.push(format!("greeting `{word}`"));
            rest = drop_leading_words(rest, 1);
            continue;
        }
        if tokens.len() > 1 && ACKNOWLEDGEMENTS.contains(&first.as_str()) {
            acknowledged = true;
            stripped.push(format!("acknowledgement `{first}`"));
            rest = drop_leading_words(rest, 1);
            continue;
        }
        if let Some(n) = PREAMBLES
            .iter()
            .filter_map(|preamble| starts_with_words(&tokens, preamble))
            .max()
            .filter(|n| *n < tokens.len())
        {
            stripped.push(format!("opener `{}`", tokens[..n].join(" ")));
            rest = drop_leading_words(rest, n);
            continue;
        }
        if tokens.len() > 1 && LEAD_FILLERS.contains(&first.as_str()) {
            rest = drop_leading_words(rest, 1);
            continue;
        }
        break;
    }
    let rest = rest.trim_start_matches(|c: char| !c.is_ascii_alphanumeric() && c.is_ascii());
    let tokens = Tokens::new(rest);
    let mut consumed = vec![false; tokens.len()];
    let mut social_phrase = false;
    for (phrase, _) in SOCIAL_PHRASES {
        let words: Vec<&str> = split_phrase(phrase);
        if let Some(at) = tokens.phrase_at(&words) {
            social_phrase = true;
            for flag in consumed.iter_mut().skip(at).take(words.len()) {
                *flag = true;
            }
        }
    }
    let lone_social = tokens.len() == 1
        && SOCIAL_WORDS
            .iter()
            .any(|(word, _)| tokens.get(0) == Some(*word));
    // A wish is a request, and the words that say so are not verbs of
    // their own: the `what` of "what I need is…" asks nothing.
    let desire = DESIRE_PREFIXES
        .iter()
        .filter_map(|prefix| starts_with_words(&tokens.words, prefix))
        .max();
    if let Some(length) = desire {
        for flag in consumed.iter_mut().take(length) {
            *flag = true;
        }
    }
    let mut read = ClauseRead {
        text: rest.to_string(),
        boundary,
        kind: ClauseKind::Statement,
        tokens,
        consumed,
        greeting,
        stripped,
        question_mark,
    };
    read.kind = classify(
        &read,
        social_phrase,
        lone_social,
        acknowledged,
        desire.is_some(),
    );
    read
}

fn classify(
    read: &ClauseRead,
    social_phrase: bool,
    lone_social: bool,
    acknowledged: bool,
    desire: bool,
) -> ClauseKind {
    let tokens = &read.tokens;
    if tokens.len() == 0 {
        return if read.greeting > 0.0 || acknowledged {
            ClauseKind::Social
        } else {
            ClauseKind::Statement
        };
    }
    if lone_social || (social_phrase && read.consumed.iter().all(|c| *c)) {
        return ClauseKind::Social;
    }
    if desire {
        return ClauseKind::Request;
    }
    let first = tokens.get(0).unwrap_or("");
    let second = tokens.get(1).unwrap_or("");
    let asks = WH_WORDS.contains(&first)
        || (first == "do" && PERSONS.contains(&second))
        || (first != "do" && AUX_WORDS.contains(&first) && QUESTION_SUBJECTS.contains(&second))
        || read.question_mark;
    if asks && !(social_phrase && read.consumed.first().copied().unwrap_or(false)) {
        return ClauseKind::Question;
    }
    if social_phrase && read.consumed.first().copied().unwrap_or(false) {
        return ClauseKind::Social;
    }
    // A verb heading a later sub-clause ("the build is broken, fix it")
    // makes the clause an instruction whatever its first words say.
    if (1..tokens.len()).any(|i| read.is_head(i) && is_lexicon_verb(tokens, i)) {
        return ClauseKind::Imperative;
    }
    // "Build fails on CI", "the deploy failed", "move occurs because…": a
    // subject, then its verb. The first word is the subject even when it
    // could be a verb somewhere else.
    if STATEMENT_VERBS.contains(&second) || first.chars().any(|c| c.is_ascii_digit()) {
        return ClauseKind::Statement;
    }
    if read.is_head(0) && is_lexicon_verb(tokens, 0) {
        return ClauseKind::Imperative;
    }
    if SUBJECT_STARTERS.contains(&first) {
        return ClauseKind::Statement;
    }
    ClauseKind::Request
}

// ------------------------------------------------------------ material ---

/// A request with its pasted material set aside.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Prepared {
    /// The text that may vote.
    pub instruction: String,
    /// Fenced code blocks removed.
    pub fenced_blocks: usize,
    /// Lines of unfenced pasted material removed.
    pub pasted_lines: usize,
}

fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && digits <= 3
        && (trimmed[digits..].starts_with(". ") || trimmed[digits..].starts_with(") "))
}

/// Whether a line reads as something a person wrote to the agent, rather
/// than something pasted for it to look at.
fn reads_as_request(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    // Indented lines are code, stack frames and tracebacks.
    if line.starts_with('\t') || line.starts_with("    ") {
        return false;
    }
    if is_list_item(line) {
        return true;
    }
    let tokens = Tokens::new(trimmed);
    if let Some(first) = tokens.get(0)
        && (is_lexicon_verb(&tokens, 0)
            || WH_WORDS.contains(&first)
            || AUX_WORDS.contains(&first)
            || SOCIAL_WORDS.iter().any(|(word, _)| *word == first)
            || PREAMBLES
                .iter()
                .any(|preamble| preamble.split(' ').next() == Some(first)))
    {
        return true;
    }
    let visible: Vec<char> = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if visible.is_empty() {
        return true;
    }
    let prose = visible
        .iter()
        .filter(|c| {
            c.is_alphabetic()
                || matches!(
                    c,
                    ',' | '.' | '\'' | '?' | '!' | ':' | ';' | '-' | '"' | '(' | ')'
                )
        })
        .count();
    let letters = visible.iter().filter(|c| c.is_alphabetic()).count();
    let prose_like = prose as f64 / visible.len() as f64 >= 0.9
        && letters as f64 / visible.len() as f64 >= 0.7
        && tokens.len() >= 2;
    let ends_like_prose = trimmed.ends_with(['.', '?', '!', ':']);
    prose_like || (ends_like_prose && letters as f64 / visible.len() as f64 >= 0.6)
}

/// Set pasted material aside: fenced blocks, and runs of two or more lines
/// that do not read as a request (or one very long one). What remains is the
/// request itself.
pub(crate) fn prepare(raw: &str) -> Prepared {
    fn flush<'a>(run: &mut Vec<&'a str>, kept: &mut Vec<&'a str>, prepared: &mut Prepared) {
        let material = run.len() >= 2 || run.iter().any(|line| line.len() >= 160);
        if material {
            prepared.pasted_lines += run.len();
        } else {
            kept.append(run);
        }
        run.clear();
    }
    let text = clean_request_text(raw);
    let mut prepared = Prepared::default();
    let mut kept: Vec<&str> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if !in_fence {
                flush(&mut run, &mut kept, &mut prepared);
                prepared.fenced_blocks += 1;
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if reads_as_request(line) {
            flush(&mut run, &mut kept, &mut prepared);
            kept.push(line);
        } else {
            run.push(line);
        }
    }
    flush(&mut run, &mut kept, &mut prepared);
    prepared.instruction = kept.join("\n");
    prepared
}

/// Strip prompt scaffolding and runner control blocks before extracting
/// intent. Delegates to [`crate::control`], which owns the tag vocabulary.
pub(crate) fn clean_request_text(raw: &str) -> String {
    crate::control::strip_control_blocks(raw).trim().to_string()
}

// ------------------------------------------------------------- digest ---

/// A digest of every table tier 1 reads, so a lexicon change that forgets
/// to bump [`crate::RESOLVER_VERSION`] fails a test rather than silently
/// invalidating every ledger row that claims `reproducible: true`.
///
/// Scoring constants are deliberately part of it too: a changed weight is
/// as much a new resolver as a new word.
pub fn lexicon_digest() -> String {
    use sha2::{Digest, Sha256};
    let mut out = String::new();
    for (word, act, weight) in ACT_VERBS {
        out.push_str(&format!("act:{word}:{}:{weight}\n", act.as_str()));
    }
    for (word, weight) in SOCIAL_WORDS {
        out.push_str(&format!("social:{word}:{weight}\n"));
    }
    for (phrase, weight) in SOCIAL_PHRASES {
        out.push_str(&format!("social-phrase:{phrase}:{weight}\n"));
    }
    let lists: [(&str, &[&str]); 15] = [
        ("ack", ACKNOWLEDGEMENTS),
        ("preamble", PREAMBLES),
        ("filler", LEAD_FILLERS),
        ("desire", DESIRE_PREFIXES),
        ("requester", REQUESTER),
        ("statement-verb", STATEMENT_VERBS),
        ("aux", AUX_WORDS),
        ("wh", WH_WORDS),
        ("question-subject", QUESTION_SUBJECTS),
        ("person", PERSONS),
        ("subject", SUBJECT_STARTERS),
        ("checkable", CHECKABLE_WORDS),
        ("local", LOCAL_NOUNS),
        ("time", TIME_WORDS),
        ("head-predecessor", HEAD_PREDECESSORS),
    ];
    for (name, list) in lists {
        for word in list {
            out.push_str(&format!("{name}:{word}\n"));
        }
    }
    for (word, stakes, weight) in STAKES_WORDS {
        out.push_str(&format!("stakes:{word}:{}:{weight}\n", stakes.as_str()));
    }
    for (word, evidence, weight) in EVIDENCE_WORDS {
        out.push_str(&format!("evidence:{word}:{}:{weight}\n", evidence.as_str()));
    }
    for word in EXACT_EVIDENCE_WORDS {
        out.push_str(&format!("evidence-exact:{word}\n"));
    }
    for (phrase, weight) in ASSURANCE_PHRASES {
        out.push_str(&format!("assurance:{phrase}:{weight}\n"));
    }
    for (phrase, weight) in RECENCY_PHRASES {
        out.push_str(&format!("recency:{phrase}:{weight}\n"));
    }
    for (phrase, horizon, weight) in HORIZON_PHRASES {
        out.push_str(&format!("horizon:{phrase}:{}:{weight}\n", horizon.as_str()));
    }
    for word in RESIDE_SUBJECTS {
        out.push_str(&format!("live-reside-subject:{word}\n"));
    }
    for word in RESIDE_PREPOSITIONS {
        out.push_str(&format!("live-reside-preposition:{word}\n"));
    }
    for word in LIVE_COPULAS {
        out.push_str(&format!("live-copula:{word}\n"));
    }
    for word in DEICTIC_WORDS {
        out.push_str(&format!("deictic:{word}\n"));
    }
    for word in BARE_PRONOUNS {
        out.push_str(&format!("bare:{word}\n"));
    }
    for word in RECURRENCE_ADJECTIVES {
        out.push_str(&format!("recurrence-adjective:{word}\n"));
    }
    for word in DETERMINERS {
        out.push_str(&format!("determiner:{word}\n"));
    }
    out.push_str(&format!("floor:{}\n", Votes::<Act>::ESCALATION_FLOOR));
    out.push_str(&format!("imperative-bonus:{IMPERATIVE_BONUS}\n"));
    crate::strand::segmentation_fingerprint(&mut out);
    format!("{:x}", Sha256::digest(out.as_bytes()))
}

// --------------------------------------------------------- extraction ---

/// Tier 1 over a whole request read as one part. A pure function of
/// `request`; the resolver reads requests part by part through
/// [`extract_part`].
pub fn extract(request: &Request<'_>) -> Extraction {
    let prepared = prepare(request.text);
    let clauses: Vec<ClauseRead> = crate::strand::segment(&prepared.instruction)
        .into_iter()
        .map(|clause| read_clause(&clause.text, clause.boundary, clause.question))
        .collect();
    let all: Vec<&ClauseRead> = clauses.iter().collect();
    extract_part(
        request,
        &all,
        &prepared,
        list_items(&prepared.instruction),
        false,
    )
}

/// How many enumerated items the request holds.
pub(crate) fn list_items(instruction: &str) -> usize {
    instruction
        .lines()
        .filter(|line| is_list_item(line))
        .count()
}

/// Tier 1 over one part of a request: the clauses that make it up, and the
/// pasted material and enumeration of the request it belongs to.
pub(crate) fn extract_part(
    request: &Request<'_>,
    clauses: &[&ClauseRead],
    prepared: &Prepared,
    enumerated: usize,
    earlier_parts: bool,
) -> Extraction {
    let mut out = Extraction {
        attendance: request
            .attendance_override
            .unwrap_or_else(|| request.surface.implied_attendance()),
        ..Extraction::default()
    };
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

    let part_words: Vec<String> = clauses
        .iter()
        .flat_map(|clause| clause.tokens.words.iter().cloned())
        .collect();
    let checkable = part_words
        .iter()
        .any(|word| CHECKABLE_WORDS.contains(&word.as_str()));
    let mut asks = false;
    let mut work = false;

    for clause in clauses {
        for stripped in &clause.stripped {
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                "stripped",
                0.0,
                format!("{stripped} set aside"),
            ));
        }
        if clause.greeting > 0.0 {
            out.act.add(Act::Converse, clause.greeting);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                "social:greeting",
                clause.greeting,
                "a greeting ⇒ converse".to_string(),
            ));
        }
        work |= clause.kind.is_work();
        read_acts(clause, &mut out);
        if clause.kind == ClauseKind::Question {
            asks = true;
        }
        read_stakes_and_evidence(clause, checkable, &mut out);
        read_recency(clause, &mut out);
        if matches!(clause.kind, ClauseKind::Imperative | ClauseKind::Request) {
            read_horizon(clause, &mut out);
        }
        read_structure(clause, &mut out);
    }
    if asks {
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "question",
            0.0,
            "the part asks a question",
        ));
    }

    // A greeting and nothing else is one reply.
    if !work
        && out
            .act
            .ranked()
            .iter()
            .all(|(act, _)| *act == Act::Converse)
        && !out.act.is_empty()
    {
        out.horizon.add(Horizon::Immediate, 0.6);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "social-only",
            0.6,
            "social talk and nothing else ⇒ immediate",
        ));
    }

    // --- material ---------------------------------------------------------
    if prepared.fenced_blocks > 0 {
        out.act.add(Act::Modify, 0.3);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "code-fence",
            0.3,
            format!("{} fenced block(s) set aside", prepared.fenced_blocks),
        ));
    }
    if prepared.pasted_lines > 0 {
        out.input_modalities.push(Modality::Data);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "pasted-material",
            0.0,
            format!(
                "{} pasted line(s) read as material, not instructions",
                prepared.pasted_lines
            ),
        ));
    }
    // Enumerated steps are the honest multi-step marker: a list the user
    // wrote themselves. Multi-step is a session, not a durable obligation.
    if enumerated >= 2 && work {
        out.horizon.add(Horizon::Session, 0.9);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "enumerated-steps",
            0.9,
            format!("{enumerated} enumerated items ⇒ session"),
        ));
    }

    // --- deixis -----------------------------------------------------------
    let deictic: Vec<&str> = DEICTIC_WORDS
        .iter()
        .copied()
        .filter(|w| part_words.iter().any(|t| t == w))
        .collect();
    out.deictic = !deictic.is_empty();
    if !deictic.is_empty() {
        let bare = clauses.iter().any(|clause| {
            clause.kind.is_work()
                && clause
                    .tokens
                    .words
                    .last()
                    .is_some_and(|last| BARE_PRONOUNS.contains(&last.as_str()))
        });
        // Pointing at something is only ambiguous when there is nothing to
        // point at: the first part of a first message with no attachment.
        // Mid-conversation, after an earlier part of the same message ("explain
        // the parser, then refactor it"), or with an attached file, it is
        // ordinary and clear.
        if request.history.turn_index == 0 && request.attachments.is_empty() && !earlier_parts {
            if bare {
                out.clarity.add(Clarity::Ambiguous, 0.7);
                out.signals.push(Signal::new(
                    SignalKind::Deictic,
                    "unresolved-reference",
                    0.7,
                    format!("`{}` with no prior context", deictic.join("`, `")),
                ));
            }
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

    // --- workspace --------------------------------------------------------
    if request.workspace.is_repo {
        out.domains_from_environment.push("engineering".into());
        out.signals.push(Signal::new(
            SignalKind::Workspace,
            "git-repo",
            0.3,
            "workspace is a git repository",
        ));
    }
    if request.workspace.has_uncommitted_changes {
        out.stakes_from_environment.add(Stakes::Reversible, 0.5);
        out.signals.push(Signal::new(
            SignalKind::Workspace,
            "dirty-tree",
            0.5,
            "uncommitted changes present",
        ));
    }

    // --- session ----------------------------------------------------------
    if out.act.is_empty()
        && part_words
            .iter()
            .any(|word| DEICTIC_WORDS.contains(&word.as_str()))
        && let Some(previous) = request.history.previous_act
    {
        // The preceding act can resolve an explicit pointer such as "that
        // again", but it cannot classify a fresh directive by itself. People
        // change subject every turn; an act prior without a reference made a
        // short format request inherit verification work from the prior turn.
        out.act.add(previous, 0.25);
        out.signals.push(Signal::new(
            SignalKind::Session,
            format!("continues:{}", previous.as_str()),
            0.25,
            format!("previous turn read as {}", previous.as_str()),
        ));
    }

    // --- attachments ------------------------------------------------------
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

fn read_acts(clause: &ClauseRead, out: &mut Extraction) {
    let tokens = &clause.tokens;
    for (target, act, weight) in VERB_TARGETS.iter() {
        let Some(position) = tokens.position(target, &clause.consumed) else {
            continue;
        };
        let head = clause.is_head(position);
        let weight = match clause.kind {
            // A statement's first word is its subject, and its other nouns
            // are not verbs; only a verb heading a later sub-clause ("it
            // crashes, fix it") asks for anything.
            ClauseKind::Statement if !head || position == 0 => continue,
            ClauseKind::Social if !head => continue,
            // In a question the head asks; other verbs are what it asks
            // about ("did the email send?") and count for half.
            ClauseKind::Question if !head => weight * 0.5,
            _ if head => weight * IMPERATIVE_BONUS,
            _ => *weight,
        };
        // "Alert me", "send me the report": delivering the result to the
        // person asking is an answer, not an operation on someone else.
        let to_requester = *act == Act::Operate
            && tokens
                .get(position + 1)
                .is_some_and(|next| REQUESTER.contains(&next));
        let (act, weight) = if to_requester {
            (Act::Answer, weight * 0.5)
        } else {
            (*act, weight)
        };
        out.act.add(act, weight);
        out.signals.push(Signal::new(
            SignalKind::Lexical,
            format!("verb:{}", target.word),
            weight,
            if to_requester {
                format!("`{} me` delivers to the requester ⇒ answer", target.word)
            } else if position == 0 {
                format!("`{}` (leading verb) ⇒ {}", target.word, act.as_str())
            } else if head {
                format!("`{}` (clause head verb) ⇒ {}", target.word, act.as_str())
            } else {
                format!("`{}` ⇒ {}", target.word, act.as_str())
            },
        ));
    }
    if clause.kind == ClauseKind::Question {
        let first = tokens.get(0).unwrap_or("");
        if !WH_WORDS.contains(&first) {
            let weight = if AUX_WORDS.contains(&first) {
                0.8 * IMPERATIVE_BONUS
            } else {
                0.8
            };
            out.act.add(Act::Answer, weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                "question:head",
                weight,
                if AUX_WORDS.contains(&first) {
                    format!("`{first}` opens a question ⇒ answer")
                } else {
                    "a question mark ⇒ answer".to_string()
                },
            ));
        }
    }
    for (phrase, weight) in SOCIAL_PHRASES {
        if tokens.phrase_at(&split_phrase(phrase)).is_some() {
            out.act.add(Act::Converse, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("social:{}", phrase.replace(' ', "-")),
                *weight,
                format!("`{phrase}` ⇒ converse"),
            ));
        }
    }
    if clause.kind == ClauseKind::Social
        && let Some(first) = tokens.get(0)
        && let Some((word, weight)) = SOCIAL_WORDS.iter().find(|(word, _)| *word == first)
    {
        out.act.add(Act::Converse, *weight);
        out.signals.push(Signal::new(
            SignalKind::Lexical,
            format!("social:{word}"),
            *weight,
            format!("`{word}` ⇒ converse"),
        ));
    }
}

fn read_stakes_and_evidence(clause: &ClauseRead, checkable: bool, out: &mut Extraction) {
    let tokens = &clause.tokens;
    for (phrase, stakes, weight) in STAKES_WORDS {
        let words = split_phrase(phrase);
        let hit = if words.len() > 1 {
            tokens.phrase_at(&words).is_some()
        } else {
            tokens.position(&Target::new(phrase), &[]).is_some()
        };
        if hit {
            out.stakes_from_words.add(*stakes, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("stakes:{}", phrase.replace(' ', "-")),
                *weight,
                format!("`{phrase}` ⇒ {}", stakes.as_str()),
            ));
        }
    }
    for (phrase, evidence, weight) in EVIDENCE_WORDS {
        let words = split_phrase(phrase);
        let hit = if words.len() > 1 {
            tokens.phrase_at(&words).is_some()
        } else if EXACT_EVIDENCE_WORDS.contains(phrase) {
            tokens.words.iter().any(|t| t == phrase)
        } else {
            tokens.position(&Target::new(phrase), &[]).is_some()
        };
        if hit {
            out.evidence.add(*evidence, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("evidence:{}", phrase.replace(' ', "-")),
                *weight,
                format!("`{phrase}` ⇒ {}", evidence.as_str()),
            ));
        }
    }
    for (phrase, weight) in ASSURANCE_PHRASES {
        let words = split_phrase(phrase);
        let hit = if words.len() > 1 {
            tokens.phrase_at(&words).is_some()
        } else {
            tokens.position(&Target::new(phrase), &[]).is_some()
        };
        if hit && checkable {
            out.evidence.add(Evidence::Verified, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("evidence:{}", phrase.replace(' ', "-")),
                *weight,
                format!("`{phrase}` something checkable ⇒ verified"),
            ));
        }
    }
}

fn read_recency(clause: &ClauseRead, out: &mut Extraction) {
    let tokens = &clause.tokens;
    if refers_to_prior_response(tokens) {
        return;
    }
    let only_live_is_current =
        live_means_current(&tokens.words) && tokens.words.iter().any(|word| word == LIVE_WORD);
    if TIME_WORDS
        .iter()
        .any(|word| tokens.words.iter().any(|t| t == word))
        && !only_live_is_current
    {
        return;
    }
    for (phrase, weight) in RECENCY_PHRASES {
        let words = split_phrase(phrase);
        let Some(at) = tokens.phrase_at(&words) else {
            continue;
        };
        // "We live in the city" is not a question about time.
        if *phrase == LIVE_WORD && !live_means_current(&tokens.words) {
            continue;
        }
        let next = tokens.get(at + words.len()).unwrap_or("");
        // "today's": the possessive splits into `today` `s`; look past it.
        let next = if next == "s" {
            tokens.get(at + words.len() + 1).unwrap_or("")
        } else {
            next
        };
        // The thing that is current: the noun after the word ("the current
        // directory"), just before it ("is my branch up to date"), or what a
        // wh-question asks about ("what tasks are scheduled right now").
        let before = (at.saturating_sub(2)..at).filter_map(|i| tokens.get(i));
        let asked_about = tokens
            .get(0)
            .filter(|first| matches!(*first, "what" | "which"))
            .and_then(|_| tokens.get(1));
        if LOCAL_NOUNS.contains(&next)
            || before.into_iter().any(|word| LOCAL_NOUNS.contains(&word))
            || asked_about.is_some_and(|word| LOCAL_NOUNS.contains(&word))
        {
            continue;
        }
        if out.recency.as_ref().is_none_or(|(_, best)| *weight > *best) {
            out.recency = Some((phrase.to_string(), *weight));
        }
    }
    if let Some((phrase, weight)) = out.recency.clone()
        && !out
            .signals
            .iter()
            .any(|s| s.name == format!("recency:{}", phrase.replace(' ', "-")))
    {
        out.signals.push(Signal::new(
            SignalKind::Lexical,
            format!("recency:{}", phrase.replace(' ', "-")),
            weight,
            format!("`{phrase}` ⇒ a current value, if the part asks for a fact"),
        ));
    }
}

fn read_horizon(clause: &ClauseRead, out: &mut Extraction) {
    let tokens = &clause.tokens;
    for (phrase, horizon, weight) in HORIZON_PHRASES {
        // A recurrence adjective names a thing, not a schedule: "the nightly
        // job", "our daily report". Only the adverb says the work recurs.
        if RECURRENCE_ADJECTIVES.contains(phrase)
            && tokens
                .words
                .windows(2)
                .any(|pair| DETERMINERS.contains(&pair[0].as_str()) && pair[1] == *phrase)
            && !tokens
                .words
                .windows(2)
                .any(|pair| !DETERMINERS.contains(&pair[0].as_str()) && pair[1] == *phrase)
            && tokens.get(0) != Some(phrase)
        {
            continue;
        }
        if tokens.phrase_at(&split_phrase(phrase)).is_some() {
            out.horizon.add(*horizon, *weight);
            out.signals.push(Signal::new(
                SignalKind::Lexical,
                format!("horizon:{}", phrase.replace(' ', "-")),
                *weight,
                format!("`{phrase}` ⇒ {}", horizon.as_str()),
            ));
        }
    }
}

fn read_structure(clause: &ClauseRead, out: &mut Extraction) {
    let text = &clause.text;
    let lower = text.to_ascii_lowercase();
    if lower.contains("http://") || lower.contains("https://") {
        out.act.add(Act::Analyze, 0.3);
        out.signals.push(Signal::new(
            SignalKind::Structural,
            "url",
            0.3,
            "a URL to fetch",
        ));
    }
    let path_like = text
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn request<'a>(text: &'a str) -> Request<'a> {
        Request {
            text,
            ..Request::default()
        }
    }

    #[test]
    fn temporal_deixis_is_recorded_and_local_nouns_are_not() {
        let now = extract(&request("what is the current price of copper"));
        assert_eq!(now.recency.as_ref().map(|r| r.0.as_str()), Some("current"));
        // "Recent" asks for the world as it stands now, unless what is
        // recent is local: invented headlines once filled a "recent news"
        // card built from nothing retrieved.
        let news = extract(&request(
            "Show a news card with three recent technology headlines.",
        ));
        assert_eq!(news.recency.as_ref().map(|r| r.0.as_str()), Some("recent"));
        assert!(
            extract(&request("Summarise the recent changes in this branch."))
                .recency
                .is_none()
        );
        assert!(now.signals.iter().any(|s| s.name.starts_with("recency:")));
        // The evidence standard is left to the request's own words.
        assert_ne!(now.evidence.winner().map(|w| w.0), Some(Evidence::Cited));

        for local in [
            "what's in the current directory",
            "update the README to reflect the latest changes",
            "fix the live reload bug",
            "what time is it right now",
            "what's today's date",
        ] {
            assert!(extract(&request(local)).recency.is_none(), "{local}");
        }
        let recalled = extract(&request(
            "What temperature did you report in the latest weather answer? Include its time.",
        ));
        assert!(recalled.recency.is_none(), "{:?}", recalled.recency);
        assert!(
            extract(&request("What is the latest weather in Noida?"))
                .recency
                .is_some(),
            "a live weather request must still require a current observation"
        );
        assert!(
            extract(&request("explain how copper is refined"))
                .recency
                .is_none()
        );
    }

    #[test]
    fn live_as_reside_is_neither_current_nor_irreversible() {
        for text in [
            "Help me plan Saturday with the kids. They are 6 and 9, we live in the city, and I would like a couple of options for the afternoon.",
            "deploy the site to wherever I live near",
            "update the doc: my son lives with his grandparents",
            "rewrite the letter, we have lived in Pune for years",
            "fix the budget for living costs",
            "where do you live",
            "change the address, my kids live in the suburbs",
        ] {
            let x = extract(&request(text));
            assert!(x.recency.is_none(), "{text}: {:?}", x.recency);
            assert!(
                !x.signals
                    .iter()
                    .any(|s| s.name.starts_with("stakes:") && s.name.contains("live")),
                "{text}"
            );
        }
    }

    /// `live` in the sense of *current* is a recency word — whether it asks
    /// for live data is then the act's question (a request for a fact) — and
    /// "go live" is a launch, which raises stakes for the work that does it.
    #[test]
    fn live_as_current_still_counts() {
        for text in [
            "what is the live score of the match",
            "show me the live price",
            "is it live",
            "find a live stream of the launch",
            "deploy it, the site is live in production",
            "we go live in an hour, deploy the fix",
        ] {
            let x = extract(&request(text));
            assert!(
                x.recency
                    .as_ref()
                    .is_some_and(|(phrase, _)| phrase == "live"),
                "{text}: {:?}",
                x.recency
            );
        }
        let launch = extract(&request("we go live in an hour, deploy the fix"));
        assert!(
            launch.signals.iter().any(|s| s.name == "stakes:go-live"),
            "{:?}",
            launch.signals
        );
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
    fn a_greeting_in_front_of_a_question_does_not_take_it_over() {
        let extraction = extract(&request(
            "hey, what did we decide about the schema yesterday?",
        ));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Answer));
        let extraction = extract(&request("hey can you check the logs"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Verify));
        let extraction = extract(&request("hello there, how are you doing today"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Converse));
    }

    #[test]
    fn hyphenated_output_literal_is_not_misread_as_a_verify_command() {
        let extraction = extract(&request(
            "Reply with exactly latency-check and no other text.",
        ));
        assert_ne!(extraction.act.winner().map(|w| w.0), Some(Act::Verify));

        let explicit = extract(&request("Please double-check this translation."));
        assert_eq!(explicit.act.winner().map(|w| w.0), Some(Act::Verify));
    }

    #[test]
    fn previous_act_only_resolves_explicit_continuity_not_a_new_topic() {
        let mut fresh = request("Reply with exactly latency-check and no other text.");
        fresh.history.turn_index = 8;
        fresh.history.previous_act = Some(Act::Verify);
        assert_ne!(extract(&fresh).act.winner().map(|w| w.0), Some(Act::Verify));

        let mut follow_up = request("Do that again.");
        follow_up.history.turn_index = 8;
        follow_up.history.previous_act = Some(Act::Verify);
        assert_eq!(
            extract(&follow_up).act.winner().map(|w| w.0),
            Some(Act::Verify)
        );
    }

    #[test]
    fn production_words_raise_stakes_whatever_the_verb() {
        let extraction = extract(&request("deploy the service to production"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Operate));
        assert_eq!(
            extraction.stakes_from_words.winner().map(|w| w.0),
            Some(Stakes::Irreversible)
        );
    }

    #[test]
    fn topic_nouns_do_not_raise_stakes() {
        for text in [
            "rename the Customer struct to Client",
            "fix the payment form validation",
            "make the banner visible to everyone",
            "the query is expensive, optimize it",
        ] {
            assert!(
                extract(&request(text)).stakes_from_words.winner().is_none(),
                "{text}"
            );
        }
    }

    #[test]
    fn unknown_input_abstains_rather_than_guessing() {
        let extraction = extract(&request("zorble the frobnicator"));
        assert!(extraction.act.is_empty(), "lexicon invented a reading");
    }

    #[test]
    fn a_statement_casts_no_act_vote() {
        let extraction = extract(&request("the deploy script is broken"));
        assert!(extraction.act.is_empty(), "{:?}", extraction.act.ranked());
        let extraction = extract(&request("it crashes on empty input, fix it"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Modify));
    }

    #[test]
    fn a_question_about_an_effect_is_a_question() {
        let extraction = extract(&request("Did the email send?"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Answer));
        let extraction = extract(&request("is the deploy broken"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Answer));
        // A request that is phrased as a question is still a request.
        let extraction = extract(&request("can you deploy the service?"));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Operate));
    }

    #[test]
    fn conjunctions_alone_do_not_imply_a_long_horizon() {
        let extraction = extract(&request("explain what this and that mean"));
        let horizon = extraction.horizon.winner().map(|w| w.0);
        assert_ne!(horizon, Some(Horizon::Durable));
        assert_ne!(horizon, Some(Horizon::Session));
    }

    #[test]
    fn recurrence_phrases_imply_a_durable_horizon_for_requests_only() {
        let extraction = extract(&request("check the cloud bill every day and alert me"));
        assert_eq!(
            extraction.horizon.winner().map(|w| w.0),
            Some(Horizon::Durable)
        );
        for question in [
            "what happens whenever I press ctrl-c in the REPL?",
            "why does the test fail continuously on CI?",
        ] {
            assert_ne!(
                extract(&request(question)).horizon.winner().map(|w| w.0),
                Some(Horizon::Durable),
                "{question}"
            );
        }
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
    fn only_a_bare_pronoun_is_ambiguous_and_only_without_context() {
        let cold = extract(&request("fix this"));
        assert_eq!(cold.clarity.winner().map(|w| w.0), Some(Clarity::Ambiguous));

        // `that` as a relative pronoun points at nothing.
        let relative = extract(&request("Write a function that parses ISO dates"));
        assert_ne!(
            relative.clarity.winner().map(|w| w.0),
            Some(Clarity::Ambiguous)
        );

        let mut warm = request("fix this");
        warm.history.turn_index = 3;
        warm.history.previous_act = Some(Act::Locate);
        assert_eq!(
            extract(&warm).clarity.winner().map(|w| w.0),
            Some(Clarity::Clear)
        );
    }

    #[test]
    fn pasted_material_is_set_aside() {
        let mut log = String::from("why is this service failing? here is the log:\n");
        for i in 0..40 {
            log.push_str(&format!(
                "2026-09-26T10:00:{i:02}Z INFO worker deploy job {i} for customer account\n"
            ));
        }
        let prepared = prepare(&log);
        assert_eq!(prepared.pasted_lines, 40);
        assert!(!prepared.instruction.contains("worker deploy"));
        let extraction = extract(&request(&log));
        assert_eq!(extraction.act.winner().map(|w| w.0), Some(Act::Answer));
        assert!(extraction.input_modalities.contains(&Modality::Data));

        let fenced = "what does this do?\n```rust\nfn deploy() { send(); }\n```";
        let prepared = prepare(fenced);
        assert_eq!(prepared.fenced_blocks, 1);
        assert_eq!(prepared.instruction, "what does this do?");
    }

    #[test]
    fn hard_wrapped_prose_is_kept() {
        let text = "please update the config so that\nthe retries are bounded and the timeout\nis configurable per provider";
        let prepared = prepare(text);
        assert_eq!(prepared.pasted_lines, 0, "{prepared:?}");
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

    /// On an ordered axis a weaker level *corroborates* a stronger one.
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
        assert!(confidence >= alone_confidence);
    }

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

    #[test]
    fn weak_hints_do_not_escalate_an_ordered_axis() {
        let mut horizon: Votes<Horizon> = Votes::default();
        horizon.add(Horizon::Immediate, 0.6);
        horizon.add(Horizon::Durable, 0.3);
        assert_eq!(horizon.winner().map(|w| w.0), Some(Horizon::Immediate));
    }

    #[test]
    fn assurance_needs_something_checkable() {
        assert_eq!(
            extract(&request("make sure the tests pass"))
                .evidence
                .winner()
                .map(|w| w.0),
            Some(Evidence::Verified)
        );
        assert!(
            extract(&request("write a poem, make sure it rhymes"))
                .evidence
                .winner()
                .is_none()
        );
    }
}
