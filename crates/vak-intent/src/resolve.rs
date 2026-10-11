//! The resolution cascade.
//!
//! Tiers run cheapest first and stop as soon as confidence clears the bar:
//!
//! | Tier | Cost | Reproducible |
//! |---|---|---|
//! | [`Tier::Declared`] — a caller said so | free | yes |
//! | [`Tier::Signals`] — deterministic extraction | free | yes |
//! | [`Tier::LocalModel`] / [`Tier::CloudModel`] | metered | no |
//! | [`Tier::General`] — nothing reached the bar | free | yes |
//!
//! A request is resolved as a list of [`Strand`]s (see [`crate::strand`]):
//! pasted material is set aside, the rest is segmented into clauses, each
//! clause that asks for work starts a part, and the clauses around it that
//! only describe the situation join it as context. The turn's engagement is
//! [`Engagement::compose`] over the strands. Everything that wants one answer
//! for the turn reads the composite [`Reading`].
//!
//! # Why this module does not dispatch
//!
//! Escalating to a model is a provider dispatch, and in vak a dispatch means a
//! work receipt, a spend-gate admission, a watchdog and a cancellation token.
//! All of that machinery lives in `vak-core`, so this module decides
//! **whether** a paid tier is warranted and hands back a
//! [`Resolution::Escalate`] carrying the partial reading; the host builds the
//! prompt with [`classification_prompt`], performs the call, parses the answer
//! with [`parse_classifications`], and folds it back in with
//! [`apply_classification`].

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::authority::{Authority, Autonomy, Envelope};
use crate::axes::{Act, Attendance, Clarity, Evidence, Horizon, Modality, Stakes};
use crate::engage::{Engagement, HilMode, derive};
use crate::reading::{Confidences, Intent, Provenance, Reading, Tier};
use crate::signals::{ClauseRead, Extraction, Request, Signal, SignalKind};
use crate::strand::{Boundary, Lineage, LineageHint, Strand, StrandRelation, ThreadFact};

/// Bumped whenever the lexicon, the scoring, or the segmentation changes, so
/// a ledger entry can be read against the rules that actually produced it.
///
/// History: 1 — the original kernel. 2 — word-boundary phrase matching,
/// sub-floor ordered votes abstain, lexical stakes gated on effectful acts,
/// strands. 3 — conversational delivery verbs resolve as Answer rather than
/// workspace authoring. 4 — two lines of work took this number before they
/// met, so a ledger row that says 4 was written by one of them: on `main`,
/// `live` as the verb "reside" ("we live in the city") stopped reading as
/// current data or irreversible stakes; on the intent-accuracy branch, the
/// tier-1 reader was rewritten (clauses read by role, pasted material set
/// aside, topic nouns no longer raise stakes, recency asks for live data only
/// beside a request for a fact and never about something local or the
/// agent's own state, assurance needs something checkable, no horizon from
/// sequencing words or length, strand ids from a host-minted turn id, stakes
/// words count when the verb is unknown and a weak part keeps them, the flag
/// spellings of destructive git commands and common instruction verbs are
/// read). 5 — the two together: `live` counts as a recency word only in the
/// sense of *current*, and "go live" is a stakes phrase. 6 — irreversible
/// stakes words can raise caution when another action verb is present. 7 —
/// preserve internal hyphens during lexical reading and recognize the explicit
/// compound action "double-check", so a noun such as "latency-check" no
/// longer becomes a verification command. 8 — the previous turn's act can
/// resolve an explicit deictic follow-up, but never classifies a new directive
/// on its own. 9 — a strand that names a second open thread with at least
/// `MERGE_MIN_OVERLAP` shared words merges it (`Lineage::Continues::merges`);
/// the lexicon is unchanged. 10 — question words, auxiliaries and pronouns
/// are not keywords, so sharing "what" no longer continues a thread. 11 —
/// "show" is a delivery verb (Answer) like "give" and "present", not a
/// search (Locate). 12 — "recent" and "recently" are temporal deixis: a
/// card of "three recent headlines" built from nothing retrieved carried
/// invented ones; plural conversation nouns ("recent conversations") stay
/// local. 13 — a greeting followed only by a word for when ("how are you
/// today?") is social and asks for no current value; the lexicon is
/// unchanged. 14 — a nearby determiner makes `schedule` a noun rather than
/// an external-operation verb (`keep the three-hour schedule`); explicit
/// scheduling requests still read as `Operate`. 15 — deixis in a later part
/// of the same message binds to that message's earlier part, not to an older
/// conversation thread. 16 — inferred continuation requires two shared
/// content words, the existing merge threshold. 17 — local pronouns in a
/// self-contained request do not identify older work, and presentation
/// language does not establish topic overlap. 18 — a dependent or sequential
/// part inherits its relationship from the preceding part in this message;
/// it cannot independently bind to an unrelated older thread. 19 — a
/// sentence that only shapes how that preceding answer is presented remains
/// part of the same request. The test
/// 20 — a lexical continuation must cover at least half of the older thread's
/// topic words, so a few broad nouns in a detailed new request cannot attach
/// it to a different scenario. 21 — colon-led, comma-separated timed items
/// are schedule labels rather than imperative actions, and `list` without a
/// direct object is read as a noun. 22 — comma-separated timed phases after
/// a context phrase receive the same reading without a colon, so a phase
/// label that resembles a workspace command does not widen an in-chat plan.
/// The test
/// `lexicon_digest_matches_resolver_version` pins the tables to this number so
/// a change to either without the other fails CI.
pub const RESOLVER_VERSION: u32 = 22;

/// Thresholds and switches for the cascade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolverConfig {
    /// Master switch. Off resolves everything to the general engagement,
    /// which is vak's pre-kernel behaviour: every admitted tool loaded,
    /// nothing narrowed.
    pub enabled: bool,
    /// Below this, a part's reading is too weak to use: it keeps only the
    /// risk its words stated and gets the general engagement.
    pub provisional_confidence: f64,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        ResolverConfig {
            enabled: true,
            provisional_confidence: 0.45,
        }
    }
}

/// A caller stating the reading outright — tier 0.
///
/// Every field is optional; whatever is set overrides the corresponding axis
/// on every strand and whatever is not falls through to the signal tier.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Declared {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<Act>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<Horizon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stakes: Option<Stakes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarity: Option<Clarity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attendance: Option<Attendance>,
    #[serde(default)]
    pub domains: BTreeSet<String>,
}

impl Declared {
    pub fn is_empty(&self) -> bool {
        self.act.is_none()
            && self.horizon.is_none()
            && self.stakes.is_none()
            && self.evidence.is_none()
            && self.clarity.is_none()
            && self.attendance.is_none()
            && self.domains.is_empty()
    }

    /// How much of the reading was stated rather than inferred, in [0,1].
    fn coverage(&self) -> f64 {
        let stated = [
            self.act.is_some(),
            self.horizon.is_some(),
            self.stakes.is_some(),
            self.evidence.is_some(),
        ]
        .into_iter()
        .filter(|x| *x)
        .count();
        stated as f64 / 4.0
    }
}

/// Group clauses into parts. A clause that asks for work starts a part;
/// clauses that only describe the situation, and social talk, are context —
/// before the first part they lead it, after it they follow the part they
/// came after. A request with no work clause at all is one part.
fn group_parts(clauses: &[ClauseRead]) -> Vec<Vec<usize>> {
    let mut parts: Vec<Vec<usize>> = Vec::new();
    let mut leading: Vec<usize> = Vec::new();
    for (index, clause) in clauses.iter().enumerate() {
        if clause.starts_part() {
            let mut part = std::mem::take(&mut leading);
            part.push(index);
            parts.push(part);
        } else if let Some(last) = parts.last_mut() {
            last.push(index);
        } else {
            leading.push(index);
        }
    }
    if parts.is_empty() {
        parts.push(leading);
    }
    parts
}

/// Run tiers 0 and 1.
///
/// Pure: identical inputs always produce an identical result — nothing here
/// reads a clock — which is what makes a recorded decision reconstructable.
/// Time enters only where a grant's liveness is judged (`apply_envelopes`),
/// as a parameter.
pub fn resolve(
    request: &Request<'_>,
    declared: &Declared,
    authority: &Authority,
    config: &ResolverConfig,
) -> Intent {
    if !config.enabled {
        return Intent::general(RESOLVER_VERSION);
    }

    // --- segmentation ----------------------------------------------------
    let prepared = crate::signals::prepare(request.text);
    let clauses: Vec<ClauseRead> = crate::strand::segment(&prepared.instruction)
        .into_iter()
        .map(|clause| crate::signals::read_clause(&clause.text, clause.boundary, clause.question))
        .collect();
    let enumerated = crate::signals::list_items(&prepared.instruction);
    let parts = group_parts(&clauses);

    // --- per-strand readings ---------------------------------------------
    let turn = request.history.turn_index;
    let mut strands: Vec<Strand> = Vec::new();
    let mut signals: Vec<Signal> = Vec::new();
    let mut weakest_axes: Vec<&'static str> = Vec::new();
    let mut all_weak = true;
    if !declared.is_empty() {
        signals.push(Signal {
            kind: SignalKind::Declared,
            name: "declared".into(),
            weight: 1.0,
            detail: format!(
                "caller stated {:.0}% of the axes",
                declared.coverage() * 100.0
            ),
        });
    }
    let multi = parts.len() > 1;
    for (index, part) in parts.iter().enumerate() {
        let reads: Vec<&ClauseRead> = part.iter().map(|&i| &clauses[i]).collect();
        let extraction =
            crate::signals::extract_part(request, &reads, &prepared, enumerated, index > 0);
        let text = reads
            .iter()
            .map(|clause| clause.text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let boundary = if index == 0 {
            Boundary::Start
        } else {
            reads
                .iter()
                .find(|clause| clause.starts_part())
                .map(|clause| clause.boundary)
                .unwrap_or(Boundary::Addition)
        };
        let strand_id = if request.turn_id.is_empty() {
            format!("t{turn}.{index}")
        } else {
            format!("{}.{index}", request.turn_id)
        };
        for signal in &extraction.signals {
            let mut signal = signal.clone();
            if multi {
                signal.detail = format!("[part {}] {}", index + 1, signal.detail);
            }
            signals.push(signal);
        }
        let (reading, weakest) = assemble(&extraction, declared);
        weakest_axes.push(weakest);

        let weak = reading.confidence < config.provisional_confidence;
        if !weak {
            all_weak = false;
        }
        let (reading, engagement) = if weak {
            // Nothing reached the bar for this part. Keep the facts that are
            // observations rather than inferences — attendance is a surface
            // fact, modalities come from attachments — and any risk the text
            // raised: stakes above the ordinary and an evidence standard are
            // only ever read from words the request actually contains, and a
            // reading too weak to narrow capability is never too weak to
            // raise caution. The part gets the orienting engagement, made at
            // least as careful as that risk demands; what a human delegated
            // still applies, because authority does not depend on how well
            // the request was read.
            let general = Reading::general();
            let kept = Reading {
                attendance: reading.attendance,
                input_modalities: reading.input_modalities.clone(),
                output_modalities: reading.output_modalities.clone(),
                stakes: if reading.stakes.rank() > general.stakes.rank() {
                    reading.stakes
                } else {
                    general.stakes
                },
                evidence: if reading.evidence.rank() > general.evidence.rank() {
                    reading.evidence
                } else {
                    general.evidence
                },
                confidence: reading.confidence,
                axis_confidence: reading.axis_confidence,
                ..general
            };
            let engagement = Engagement::general().meet(&derive(&kept, authority));
            (kept, engagement)
        } else {
            let engagement = derive(&reading, authority);
            (reading, engagement)
        };

        let relation = match boundary {
            Boundary::Start => StrandRelation::Independent,
            Boundary::Sequence => match strands.last() {
                Some(previous) => StrandRelation::Sequential {
                    after: previous.strand_id.clone(),
                },
                None => StrandRelation::Independent,
            },
            Boundary::Addition | Boundary::Sentence => match strands.last() {
                Some(previous) if extraction.deictic => StrandRelation::Dependent {
                    on: previous.strand_id.clone(),
                },
                _ => StrandRelation::Independent,
            },
        };
        // Deixis inside a compound request points to an earlier part of this
        // same message before it can point to a thread from an older turn.
        // `relation` records that local dependency; allowing `lineage_for` to
        // search session threads here made "A box has 31 crayons ...? Show it
        // clearly" continue whichever old answer thread happened to be open.
        // Explicit `/goal fix` and `/goal replace` remain authoritative.
        let same_message_parent = match &relation {
            StrandRelation::Sequential { after } | StrandRelation::Dependent { on: after } => {
                strands.iter().find(|previous| previous.strand_id == *after)
            }
            StrandRelation::Independent => None,
        };
        let local_lineage = same_message_parent.map(|parent| match &parent.lineage {
            // A local child of work already attached to an older thread stays
            // with that thread. A child of a new or replaced strand must not
            // reach back to the replaced or merely recent thread by pronoun.
            Lineage::Continues { .. } | Lineage::Corrects { .. } => parent.lineage.clone(),
            Lineage::New | Lineage::Replaces { .. } => Lineage::New,
        });
        let lineage = if request.lineage_hint.is_none() {
            local_lineage.unwrap_or_else(|| {
                lineage_for(
                    &reading,
                    extraction.deictic && index == 0,
                    &text,
                    &request.history.open_threads,
                    request.lineage_hint,
                )
            })
        } else {
            lineage_for(
                &reading,
                extraction.deictic && index == 0,
                &text,
                &request.history.open_threads,
                request.lineage_hint,
            )
        };
        let thread_id = lineage
            .continued_thread()
            .map(str::to_string)
            .unwrap_or_else(|| strand_id.clone());
        strands.push(Strand {
            strand_id,
            thread_id,
            text,
            reading,
            relation,
            lineage,
            engagement,
        });
    }

    // --- composite -------------------------------------------------------
    let reading = composite_reading(&strands);
    let engagement = recompose(&strands);

    let tier = if declared.coverage() >= 1.0 {
        Tier::Declared
    } else if all_weak {
        Tier::General
    } else {
        Tier::Signals
    };
    let mut provenance = Provenance::new(tier, RESOLVER_VERSION, signals);
    let weakest = weakest_axes
        .iter()
        .zip(strands.iter())
        .min_by(|(_, a), (_, b)| {
            a.reading
                .confidence
                .partial_cmp(&b.reading.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(axis, _)| *axis)
        .unwrap_or("act");

    if all_weak {
        provenance.escalation_note = Some(format!(
            "confidence {:.2} below floor {:.2}, weakest on {weakest}; general engagement applied",
            reading.confidence, config.provisional_confidence
        ));
    }

    Intent {
        reading,
        strands,
        engagement,
        provenance,
    }
}

/// The turn's engagement from its strands: [`Engagement::compose`], plus a
/// model-visible note only when it carries relationship or per-part guidance.
fn recompose(strands: &[Strand]) -> Engagement {
    let mut engagement = Engagement::compose(
        &strands
            .iter()
            .map(|strand| strand.engagement.clone())
            .collect::<Vec<_>>(),
    );
    if strands.len() > 1 {
        let note = strand_note(strands);
        if !note.is_empty() {
            engagement.posture.note = Some(note);
        }
    }
    engagement
}

/// Which thread, if any, a strand belongs to.
///
/// An explicit hint always wins and always names a thread — the most
/// specific match, or the most recent open thread when nothing matches.
/// Otherwise a strand continues a thread when it shares the act and either
/// shares at least two topic words covering half the older thread, or is a
/// short deictic follow-up. A long,
/// self-contained request may contain local words like "that" or "it"; they
/// do not identify an older thread. Generic presentation preferences do not
/// establish topic overlap. A reading
/// that matches nothing starts a thread of its own; a wrong `New` costs a
/// duplicate thread, a wrong `Continues` merges unrelated work, so the tie
/// goes to `New`.
/// Content words a clause must share with another open thread before it is
/// read as merging that thread into the one it continues. The overlap must
/// also cover half the other thread's subject. A single shared word is too
/// weak to establish either a merge or a continuation: common
/// planning words linked otherwise unrelated scenarios and pulled their full
/// histories into later requests. Short follow-ups such as "refactor it"
/// can still identify a continuation on their own.
const MERGE_MIN_OVERLAP: usize = 2;

/// A continuation must account for enough of the older thread's subject to
/// distinguish it from a new request that happens to reuse a few broad words.
/// Measure against the stored thread so concise follow-ups need not repeat
/// every current detail.
fn enough_topic_overlap(shared: usize, thread: &ThreadFact) -> bool {
    shared >= MERGE_MIN_OVERLAP && shared.saturating_mul(2) >= thread.keywords.len()
}

const PRESENTATION_WORDS: &[&str] = &[
    "easy", "scan", "share", "simple", "clear", "short", "brief", "concise", "readable",
];

fn topic_keywords(text: &str) -> BTreeSet<String> {
    crate::strand::keywords(text)
        .into_iter()
        .filter(|word| !PRESENTATION_WORDS.contains(&word.as_str()))
        .collect()
}

fn lineage_for(
    reading: &Reading,
    deictic: bool,
    text: &str,
    open_threads: &[ThreadFact],
    hint: Option<LineageHint>,
) -> Lineage {
    let keywords = topic_keywords(text);
    let short_deictic_follow_up = deictic && crate::strand::keywords(text).len() <= 3;
    let overlap = |thread: &ThreadFact| thread.keywords.intersection(&keywords).count();
    let candidates: Vec<&ThreadFact> = open_threads
        .iter()
        .filter(|thread| reading.acts().contains(&thread.act) || deictic)
        .collect();
    let best = candidates
        .iter()
        .copied()
        .max_by_key(|thread| (overlap(thread), reading.acts().contains(&thread.act)))
        .filter(|thread| {
            enough_topic_overlap(overlap(thread), thread)
                || (short_deictic_follow_up && reading.acts().contains(&thread.act))
        });
    match hint {
        Some(LineageHint::Corrects) => {
            let thread = best.or(open_threads.last());
            return thread
                .map(|t| Lineage::Corrects {
                    thread_id: t.thread_id.clone(),
                })
                .unwrap_or(Lineage::New);
        }
        Some(LineageHint::Replaces) => {
            let thread = best.or(open_threads.last());
            return thread
                .map(|t| Lineage::Replaces {
                    thread_id: t.thread_id.clone(),
                })
                .unwrap_or(Lineage::New);
        }
        None => {}
    }
    best.map(|t| Lineage::Continues {
        thread_id: t.thread_id.clone(),
        merges: candidates
            .iter()
            .filter(|other| {
                other.thread_id != t.thread_id && enough_topic_overlap(overlap(other), other)
            })
            .map(|other| other.thread_id.clone())
            .collect(),
    })
    .unwrap_or(Lineage::New)
}

/// The one reading for the turn: the most consequential strand, widened by
/// the others.
fn composite_reading(strands: &[Strand]) -> Reading {
    let Some(primary) = strands.iter().rev().max_by_key(|strand| {
        (
            strand.reading.stakes.rank(),
            strand.reading.acts().iter().any(|act| act.is_effectful()),
            !matches!(strand.relation, StrandRelation::Dependent { .. }),
            (strand.reading.confidence * 1000.0) as u32,
        )
    }) else {
        return Reading::general();
    };
    let mut out = primary.reading.clone();
    let mut confidence = Confidences {
        act: f64::INFINITY,
        horizon: f64::INFINITY,
        stakes: f64::INFINITY,
        evidence: f64::INFINITY,
    };
    for strand in strands {
        let r = &strand.reading;
        for act in r.acts() {
            if act != out.act {
                out.alternate_acts.insert(act);
            }
        }
        if r.horizon.rank() > out.horizon.rank() {
            out.horizon = r.horizon;
        }
        if r.stakes.rank() > out.stakes.rank() {
            out.stakes = r.stakes;
        }
        if r.evidence.rank() > out.evidence.rank() {
            out.evidence = r.evidence;
        }
        if r.clarity.rank() > out.clarity.rank() {
            out.clarity = r.clarity;
        }
        out.input_modalities
            .extend(r.input_modalities.iter().copied());
        out.output_modalities
            .extend(r.output_modalities.iter().copied());
        out.domains.extend(r.domains.iter().cloned());
        let c = r.axis_confidence;
        confidence.act = confidence.act.min(c.act);
        confidence.horizon = confidence.horizon.min(c.horizon);
        confidence.stakes = confidence.stakes.min(c.stakes);
        confidence.evidence = confidence.evidence.min(c.evidence);
    }
    if !confidence.act.is_finite() {
        confidence = Confidences::default();
    }
    out.axis_confidence = confidence;
    out.confidence = confidence.overall();
    out
}

/// The model-visible note for a multi-strand turn: guidance that holds for
/// every part of the request, unlabelled. Nothing here names a part by
/// number or says one part uses another or continues earlier work: the
/// model was never shown the parts, and "Part 3 uses the result of part 2"
/// sent a model looking for work that did not exist (found live 2026-10-11,
/// docs/design/86-model-plans-runtime-checks.md §1). Relations and lineage
/// stay in the intent record for audit; what the request asks for is the
/// model's own plan. Parts are never quoted either: this note rides in the
/// per-turn tail, and restating the request there reads as the user asking
/// again (docs/design/68-context-engine.md §6).
fn strand_note(strands: &[Strand]) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let Some((first, rest)) = strands.split_first() else {
        return String::new();
    };
    let Some(note) = &first.engagement.posture.note else {
        return String::new();
    };
    for line in note.lines() {
        let shared = rest.iter().all(|strand| {
            strand
                .engagement
                .posture
                .note
                .as_deref()
                .is_some_and(|other| other.lines().any(|candidate| candidate == line))
        });
        if shared && !lines.contains(&line) {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Acts that ask for a fact, and so may ask for its current value.
fn asks_for_a_fact(act: Act) -> bool {
    matches!(act, Act::Answer | Act::Locate | Act::Analyze)
}

/// Turn one part's votes into a reading, honouring anything the caller
/// declared.
///
/// Returns the reading and the name of the axis that scored worst, which is
/// what an escalation prompt should focus a model's attention on.
fn assemble(extraction: &Extraction, declared: &Declared) -> (Reading, &'static str) {
    let mut confidences: Vec<(&'static str, f64)> = Vec::new();

    // A declared axis is certain by construction; an inferred one carries the
    // vote margin. Defaults are chosen to be the *safe* value on risk axes and
    // the *ordinary* value elsewhere.
    let (act, act_confidence) = match declared.act {
        Some(act) => (act, 1.0),
        None => match extraction.act.winner() {
            Some((act, confidence)) => (act, confidence),
            None => (Act::Answer, 0.0),
        },
    };
    // Confidence in a capability *slice* is confidence that the slice covers
    // the request — not confidence about which single act won. Acts within
    // half the winner's weight join the slice, and confidence is how much of
    // the total act evidence that set accounts for, damped by how much
    // evidence there was at all.
    const ACT_BAND: f64 = 0.5;
    let mut alternate_acts: BTreeSet<Act> = BTreeSet::new();
    let mut act_confidence = act_confidence;
    if declared.act.is_none() {
        let ranked = extraction.act.ranked();
        let contenders = extraction.act.contenders(ACT_BAND);
        if !contenders.is_empty() {
            alternate_acts.extend(contenders.iter().copied().filter(|a| *a != act));
            let total: f64 = ranked.iter().map(|(_, weight)| weight).sum();
            let covered: f64 = ranked
                .iter()
                .filter(|(value, _)| contenders.contains(value))
                .map(|(_, weight)| weight)
                .sum();
            let coverage = if total > 0.0 { covered / total } else { 0.0 };
            let best = ranked.first().map(|(_, w)| *w).unwrap_or(0.0);
            let mass = (best / 1.5).min(1.0);
            act_confidence = (coverage * 0.6 + mass * 0.4).clamp(0.0, 1.0);
        }
    }
    confidences.push(("act", act_confidence));

    let (horizon, horizon_confidence) = match declared.horizon {
        Some(horizon) => (horizon, 1.0),
        None => match extraction.horizon.winner() {
            Some((horizon, confidence)) => (horizon, confidence),
            // Genuinely uncertain: a request with no recurrence or enumeration
            // markers could be a one-liner or a week of work, and nothing in
            // the text distinguishes them.
            None => (Horizon::Turn, 0.5),
        },
    };
    confidences.push(("horizon", horizon_confidence));

    // Stakes default upward from the act rather than to a fixed value: an
    // unrecognised request to `deploy` should not read as inert just because
    // no stakes word appeared next to it. Both the request's own stakes words
    // and the environment's apply to acts that touch something — and to a
    // request whose verb was not recognised at all, because "force push to
    // the production branch" is no less dangerous for using a verb the
    // reader does not know. Only a request recognised as asking, finding or
    // analysing may mention production without being about to change it.
    let act_unknown = declared.act.is_none() && extraction.act.winner().is_none();
    let may_carry_action_stakes = matches!(
        act,
        Act::Author | Act::Modify | Act::Operate | Act::Verify | Act::Orchestrate | Act::Govern
    );
    let mut stakes_votes: crate::signals::Votes<Stakes> = crate::signals::Votes::default();
    if may_carry_action_stakes || act_unknown {
        for (value, weight) in extraction.stakes_from_words.ranked() {
            stakes_votes.add(value, weight);
        }
        for (value, weight) in extraction.stakes_from_environment.ranked() {
            stakes_votes.add(value, weight);
        }
    }
    let (stakes, stakes_confidence) = match declared.stakes {
        Some(stakes) => (stakes, 1.0),
        None => match stakes_votes.winner() {
            Some((stakes, confidence)) => {
                let floor = implied_stakes(act);
                if floor.rank() > stakes.rank() {
                    (floor, confidence.min(0.6))
                } else {
                    (stakes, confidence)
                }
            }
            // No stakes language at all. The act's own floor is then the
            // answer, and it is only as trustworthy as the act reading that
            // produced it.
            None => (implied_stakes(act), act_confidence.max(0.5)),
        },
    };
    confidences.push(("stakes", stakes_confidence));

    // Absence is informative here, unlike on the other axes: a request with no
    // citation, verification or sign-off language genuinely does have no
    // special evidentiary standard.
    let (evidence, evidence_confidence) = match declared.evidence {
        Some(evidence) => (evidence, 1.0),
        None => match extraction.evidence.winner() {
            Some((evidence, confidence)) => (evidence, confidence),
            None => (Evidence::None, 0.85),
        },
    };
    confidences.push(("evidence", evidence_confidence));

    let (clarity, _) = match declared.clarity {
        Some(clarity) => (clarity, 1.0),
        None => match extraction.clarity.winner() {
            Some((clarity, confidence)) => (clarity, confidence),
            None => (Clarity::Clear, 0.5),
        },
    };

    let attendance = declared.attendance.unwrap_or(extraction.attendance);

    let mut domains: BTreeSet<String> = extraction.domains.iter().cloned().collect();
    // Environment-derived domains (a git repository) apply only to effectful
    // acts: a repository mid-edit does not make answering a question an
    // engineering task.
    if act.is_effectful() {
        domains.extend(extraction.domains_from_environment.iter().cloned());
    }
    // A temporal reference asks for a value as it stands now only beside a
    // request for a fact. "Refactor the current implementation" and "hello,
    // how are you today" ask for none, and reading them as live-data made the
    // loop demand a retrieval and replace the answer when none came.
    if extraction.recency.is_some()
        && std::iter::once(act)
            .chain(alternate_acts.iter().copied())
            .any(asks_for_a_fact)
    {
        domains.insert("live-data".into());
    }
    domains.extend(declared.domains.iter().cloned());

    let axis_confidence = Confidences {
        act: act_confidence,
        horizon: horizon_confidence,
        stakes: stakes_confidence,
        evidence: evidence_confidence,
    };
    // The overall figure is the weakest axis, not an average: averaging would
    // let a confident `act` hide a coin flip on `stakes`. It gates only the
    // all-or-nothing fallback to the orienting engagement; each projection
    // gates on the specific axis it depends on.
    let confidence = axis_confidence.overall();
    let weakest = confidences
        .iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(name, _)| *name)
        .unwrap_or("act");

    let input_modalities: BTreeSet<Modality> =
        extraction.input_modalities.iter().copied().collect();
    let output_modalities: BTreeSet<Modality> =
        extraction.output_modalities.iter().copied().collect();

    (
        Reading {
            act,
            horizon,
            stakes,
            evidence,
            clarity,
            input_modalities,
            output_modalities,
            attendance,
            alternate_acts,
            domains,
            confidence: if confidence.is_finite() {
                confidence
            } else {
                0.0
            },
            axis_confidence,
        },
        weakest,
    )
}

/// The lowest stakes an act can honestly carry when nothing else is known.
///
/// This is a floor, never a cap: a stakes word in the request can raise it,
/// and nothing here lowers what the request itself implies.
pub fn implied_stakes(act: Act) -> Stakes {
    match act {
        Act::Converse | Act::Answer | Act::Locate | Act::Analyze => Stakes::Inert,
        // Governing the agent's own configuration, memory and skills changes
        // state the agent owns and can change back; the permission engine and
        // the privileged-config rules are what guard it, not a confirmation
        // prompt on "remember that I prefer tabs".
        Act::Author | Act::Modify | Act::Verify | Act::Orchestrate | Act::Govern => {
            Stakes::Reversible
        }
        // Reaching outside the workspace is never assumed to be cheap.
        Act::Operate => Stakes::Irreversible,
    }
}

// ----------------------------------------------------------- envelopes ---

/// Narrow an intent by the grants on the commitments its strands serve.
///
/// `envelopes` maps a strand id to the envelope of the commitment that strand
/// is working on. A live envelope lowers the strand's permission ceiling and
/// spend ceiling — it never raises either — and marks a delegated strand as
/// working inside its envelope, unless the strand is irreversible or waiting
/// on a person. What the envelope *pre-authorizes* is decided per action at
/// the approval gate (`Envelope::covers`), not here.
pub fn apply_envelopes(
    intent: Intent,
    envelopes: &BTreeMap<String, Envelope>,
    autonomy: Autonomy,
    now: chrono::DateTime<chrono::Utc>,
) -> Intent {
    let mut strands = intent.strands.clone();
    let mut changed = false;
    for strand in &mut strands {
        let Some(envelope) = envelopes
            .get(&strand.strand_id)
            .filter(|envelope| envelope.is_live(now))
        else {
            continue;
        };
        changed = true;
        let limits = &mut strand.engagement.limits;
        limits.permission_ceiling = limits.permission_ceiling.meet(envelope.permission_ceiling);
        let cap = envelope
            .spend_limit_usd
            .filter(|cap| cap.is_finite() && *cap >= 0.0);
        limits.spend_ceiling_usd = match (limits.spend_ceiling_usd, cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if autonomy == Autonomy::Delegated
            && strand.reading.stakes != Stakes::Irreversible
            && strand.engagement.posture.hil != HilMode::Defer
        {
            strand.engagement.posture.hil = HilMode::Envelope;
        }
    }
    if !changed {
        return intent;
    }
    let engagement = recompose(&strands);
    Intent {
        reading: intent.reading,
        strands,
        engagement,
        provenance: intent.provenance,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {

    use super::*;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn request<'a>(text: &'a str) -> Request<'a> {
        Request {
            text,
            ..Request::default()
        }
    }

    fn resolve_text(text: &str) -> Intent {
        resolve(
            &request(text),
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        )
    }

    /// `RESOLVER_VERSION` and the lexicon move together. When this fails:
    /// bump `RESOLVER_VERSION`, add a line to its history, and replace the
    /// pinned digest with the one in the message.
    #[test]
    fn lexicon_digest_matches_resolver_version() {
        const PINNED: (u32, &str) = (
            22,
            "0053b8ab40fb608ff7145771fc6f38c557612cbd3314f50c8c8be55a7db4f49c",
        );
        let digest = crate::signals::lexicon_digest();
        assert_eq!(
            (RESOLVER_VERSION, digest.as_str()),
            PINNED,
            "the tier-1 lexicon changed: bump RESOLVER_VERSION and pin digest {digest}"
        );
    }

    #[test]
    fn resolution_is_deterministic() {
        let a = resolve_text("refactor the parser");
        let b = resolve_text("refactor the parser");
        assert_eq!(a, b);
    }

    #[test]
    fn a_fully_declared_reading_is_tier_zero_and_reproducible() {
        let declared = Declared {
            act: Some(Act::Modify),
            horizon: Some(Horizon::Session),
            stakes: Some(Stakes::Reversible),
            evidence: Some(Evidence::Verified),
            ..Declared::default()
        };
        let resolution = resolve(
            &request("whatever"),
            &declared,
            &Authority::default(),
            &ResolverConfig::default(),
        );
        let intent = resolution;
        assert_eq!(intent.provenance.tier, Tier::Declared);
        assert!(intent.provenance.reproducible);
        assert_eq!(intent.reading.act, Act::Modify);
        assert_eq!(intent.reading.evidence, Evidence::Verified);
    }

    #[test]
    fn confidence_tracks_the_weakest_axis() {
        let resolution = resolve_text("refactor the parser");
        let intent = resolution;
        assert!(intent.reading.confidence <= 1.0);
        let declared = Declared {
            act: Some(Act::Modify),
            horizon: Some(Horizon::Turn),
            stakes: Some(Stakes::Reversible),
            evidence: Some(Evidence::None),
            ..Declared::default()
        };
        let full = resolve(
            &request("refactor the parser"),
            &declared,
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert!(full.reading.confidence > intent.reading.confidence);
    }

    /// What a human delegated applies whether or not the request was read:
    /// `manual` asks at every level, even for a part nobody understood.
    #[test]
    fn authority_governs_a_part_the_free_tiers_could_not_read() {
        let manual = Authority {
            autonomy: Autonomy::Manual,
            ..Authority::default()
        };
        let intent = resolve(
            &request("zorble the frobnicator"),
            &Declared::default(),
            &manual,
            &ResolverConfig::default(),
        );
        assert_eq!(intent.provenance.tier, Tier::General);
        assert_eq!(
            intent.engagement.limits.approval_ceiling,
            crate::ApprovalCeiling::Ask
        );
    }

    #[test]
    fn operate_is_never_assumed_cheap_and_governing_is_reversible() {
        assert_eq!(implied_stakes(Act::Operate), Stakes::Irreversible);
        assert_eq!(implied_stakes(Act::Govern), Stakes::Reversible);
        assert_eq!(implied_stakes(Act::Answer), Stakes::Inert);
    }

    // ----------------------------------------------------------- strands ---

    #[test]
    fn independent_new_outputs_do_not_add_a_count_only_model_note() {
        let intent = resolve_text(
            "For a 50-minute science club session with 12 students, use this schedule: welcome 10 minutes, build a simple model 25 minutes, clean up 15 minutes. Budget is $40: materials $28 and safety supplies $12. Give me a practical plan with a short checklist and the exact budget split, easy to share.",
        );
        assert!(intent.strands.len() > 1);
        assert!(intent.model_visible().is_none());
    }

    #[test]
    fn list_remains_a_verb_when_it_has_a_direct_object() {
        assert_eq!(
            resolve_text("List the supplies and their prices")
                .reading
                .act,
            Act::Locate
        );
    }

    #[test]
    fn timed_labels_do_not_hide_a_direct_action_outside_the_schedule() {
        let intent = resolve_text(
            "Fix the projector before the 30-minute workshop: introduction 5 minutes, painting 20 minutes, cleanup 5 minutes.",
        );
        assert!(intent.reading.acts().contains(&Act::Modify));
    }

    #[test]
    fn same_message_presentation_refinement_does_not_continue_an_unrelated_open_thread() {
        let mut request =
            request("A box has 31 crayons and 8 are missing. How many remain? Show it clearly.");
        request.history.open_threads.push(ThreadFact {
            thread_id: "older-garden-thread".into(),
            act: Act::Answer,
            domains: BTreeSet::new(),
            keywords: ["garden".into(), "plan".into()].into(),
        });
        let intent = resolve(
            &request,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );

        assert_eq!(intent.strands.len(), 1);
        assert_eq!(intent.strands[0].lineage, Lineage::New);
        assert!(intent.model_visible().is_none());
    }

    #[test]
    fn a_sequenced_same_message_step_keeps_a_real_parent_thread() {
        let mut request = request("How many garden plants remain? Then show it clearly.");
        request.history.open_threads.push(ThreadFact {
            thread_id: "active-garden-thread".into(),
            act: Act::Answer,
            domains: BTreeSet::new(),
            keywords: ["garden".into(), "plants".into()].into(),
        });
        let intent = resolve(
            &request,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );

        assert_eq!(intent.strands.len(), 2);
        assert!(matches!(
            intent.strands[0].lineage,
            Lineage::Continues { ref thread_id, .. } if thread_id == "active-garden-thread"
        ));
        assert_eq!(
            intent.strands[1].lineage, intent.strands[0].lineage,
            "a local dependent step must remain attached to its parent work"
        );
    }

    #[test]
    fn explicit_addition_remains_a_distinct_part_after_sentence_refinement() {
        let intent = resolve_text("How many crayons remain? Also show it clearly.");
        assert_eq!(intent.strands.len(), 2);
        assert!(matches!(
            intent.strands[1].relation,
            StrandRelation::Dependent { .. }
        ));
    }

    /// Strand ids come from the host's turn id, so a thread — and the
    /// commitment keyed by it — never collides with another turn's.
    #[test]
    fn strand_ids_come_from_the_turn_id() {
        let mut req = request("fix the login bug. Also check whether the nightly job ran");
        req.turn_id = "0199a1b2-7c3d-7e4f-8a9b-0c1d2e3f4a5b";
        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert_eq!(
            intent.strands[0].strand_id,
            "0199a1b2-7c3d-7e4f-8a9b-0c1d2e3f4a5b.0"
        );
        assert_eq!(
            intent.strands[1].thread_id,
            "0199a1b2-7c3d-7e4f-8a9b-0c1d2e3f4a5b.1"
        );
    }

    #[test]
    fn conversational_card_delivery_is_an_answer_not_workspace_authoring() {
        for text in [
            "Give me the latest India news and present it as a card",
            "What is the current weather in Delhi? Show it as a card",
        ] {
            let intent = resolve_text(text);
            assert_eq!(intent.reading.act, Act::Answer, "{text}");
            assert!(
                intent
                    .strands
                    .iter()
                    .all(|strand| strand.reading.act != Act::Author),
                "{text}: {:?}",
                intent.strands
            );
            assert!(intent.reading.domains.contains("live-data"), "{text}");
        }
    }

    #[test]
    fn unrelated_strands_are_independent_threads() {
        let intent = resolve_text("fix the login bug. Also check whether the nightly job ran");
        assert_eq!(intent.strands.len(), 2);
        assert_eq!(intent.strands[1].relation, StrandRelation::Independent);
        assert_ne!(intent.strands[0].thread_id, intent.strands[1].thread_id);
        assert!(matches!(intent.strands[0].lineage, Lineage::New));
    }

    #[test]
    fn a_strand_continues_an_open_thread_it_points_at() {
        let mut req = request("now refactor it");
        req.history.turn_index = 2;
        req.history.open_threads = vec![ThreadFact {
            thread_id: "s0.0".into(),
            act: Act::Modify,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords("fix the parser"),
        }];
        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert_eq!(
            intent.strands[0].lineage,
            Lineage::Continues {
                thread_id: "s0.0".into(),
                merges: Vec::new(),
            }
        );
        assert_eq!(intent.strands[0].thread_id, "s0.0");
    }

    #[test]
    fn two_shared_content_words_continue_without_a_pronoun() {
        let mut req = request("Refactor the parser module");
        req.history.turn_index = 2;
        req.history.open_threads = vec![ThreadFact {
            thread_id: "parser-work".into(),
            act: Act::Modify,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords("refactor the parser module"),
        }];

        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );

        assert_eq!(
            intent.strands[0].lineage,
            Lineage::Continues {
                thread_id: "parser-work".into(),
                merges: Vec::new(),
            }
        );
    }

    #[test]
    fn a_detailed_new_plan_does_not_attach_to_an_older_club_plan_by_common_nouns() {
        let text = "For a 24-minute book club, use this schedule: welcome 4 minutes, reading 16 minutes, wrap-up 4 minutes. Budget: tea $10 and fruit $8, total $18. Make it easy to share.";
        let mut req = request(text);
        req.history.turn_index = 112;
        req.history.previous_act = Some(Act::Answer);
        req.history.open_threads = vec![ThreadFact {
            thread_id: "older-art-club-plan".into(),
            act: Act::Answer,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords(
                "Plan a 35-minute art club for 8 kids: setup, painting, and cleanup. Budget $28 for materials and paint; include a supply checklist.",
            ),
        }];

        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );

        assert!(
            intent
                .strands
                .iter()
                .all(|strand| matches!(strand.lineage, Lineage::New)),
            "{:?}",
            intent
                .strands
                .iter()
                .map(|strand| (&strand.text, &strand.lineage))
                .collect::<Vec<_>>()
        );
        assert!(
            intent
                .model_visible()
                .is_none_or(|note| !note.contains("continues earlier work")),
            "{:?}",
            intent.model_visible()
        );
    }

    #[test]
    fn a_later_part_pronoun_stays_with_its_own_request() {
        let mut req = request("Write a short event plan. Also make it easy to share.");
        req.history.turn_index = 2;
        req.history.open_threads = vec![ThreadFact {
            thread_id: "older-plan".into(),
            act: Act::Author,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords("prepare a release timeline"),
        }];

        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );

        assert_eq!(intent.strands.len(), 2);
        assert_eq!(
            intent.strands[1].relation,
            StrandRelation::Dependent {
                on: intent.strands[0].strand_id.clone(),
            }
        );
        assert_eq!(intent.strands[1].lineage, Lineage::New);
    }

    #[test]
    fn one_shared_planning_word_does_not_continue_an_unrelated_thread() {
        let mut req = request("Write a short book-club schedule.");
        req.history.turn_index = 2;
        req.history.open_threads = vec![ThreadFact {
            thread_id: "older-event".into(),
            act: Act::Author,
            domains: BTreeSet::new(),
            keywords: BTreeSet::from(["schedule".into()]),
        }];

        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );

        assert_eq!(intent.strands.len(), 1);
        assert_eq!(intent.strands[0].lineage, Lineage::New);
    }

    #[test]
    fn local_deixis_and_presentation_words_do_not_link_a_new_scenario() {
        let mut req = request(
            "A few of us are meeting tomorrow to plan a community garden. There will be 6 neighbors, we have about an hour and a half, and I can spend up to $45 on seed packets, drinks and plant labels. Could you suggest an easy plan for the time, a small shopping list, and a way to use the money that adds up to $45? I'd like to share it in our group chat.",
        );
        req.history.turn_index = 20;
        req.history.open_threads = vec![ThreadFact {
            thread_id: "older-event-style".into(),
            act: Act::Author,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords("Make it easy to scan and share"),
        }];

        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );

        assert_eq!(intent.strands[0].lineage, Lineage::New);
    }

    /// A clause that names two open threads continues the closer one and
    /// merges the other; one shared word is not enough to merge.
    #[test]
    fn strand_continues_two_threads() {
        let thread = |id: &str, text: &str| ThreadFact {
            thread_id: id.into(),
            act: Act::Modify,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords(text),
        };
        let mut req = request("merge the parser refactor with the flaky integration tests cleanup");
        req.history.turn_index = 4;
        req.history.open_threads = vec![
            thread("s0.0", "refactor the parser module"),
            thread("s1.0", "cleanup of the flaky integration tests"),
            thread("s2.0", "cleanup the downloads folder"),
        ];
        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        let Lineage::Continues { thread_id, merges } = &intent.strands[0].lineage else {
            panic!("{:?}", intent.strands[0].lineage);
        };
        let mut threads: Vec<&str> = merges.iter().map(String::as_str).collect();
        threads.push(thread_id);
        threads.sort_unstable();
        assert_eq!(
            threads,
            vec!["s0.0", "s1.0"],
            "{:?}",
            intent.strands[0].lineage
        );
    }

    /// The live false link of 2026-10-03: a question about a file shared
    /// only the word "what" with an arithmetic question, and continued its
    /// thread.
    #[test]
    fn a_shared_question_word_does_not_continue_a_thread() {
        let text = "Kindly show me exactly what status.txt currently contains, word for word, nothing more.";
        let act = resolve_text(text).reading.act;
        let mut req = request(text);
        req.history.turn_index = 2;
        req.history.open_threads = vec![ThreadFact {
            thread_id: "s1.0".into(),
            act,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords("What is 17 times 23?"),
        }];
        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert_eq!(
            intent.strands[0].lineage,
            Lineage::New,
            "{:?}",
            intent.strands[0].lineage
        );
    }

    /// Corrections and replacements are never inferred from text, and a
    /// replacement starts a thread of its own.
    #[test]
    fn corrections_only_come_from_an_explicit_hint() {
        let thread = ThreadFact {
            thread_id: "s0.0".into(),
            act: Act::Modify,
            domains: BTreeSet::new(),
            keywords: crate::strand::keywords("refactor the parser"),
        };
        let mut req = request("actually refactor the parser to use a state machine instead");
        req.history.turn_index = 1;
        req.history.open_threads = vec![thread.clone()];
        let inferred = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert!(matches!(
            inferred.strands[0].lineage,
            Lineage::Continues { .. }
        ));

        req.lineage_hint = Some(LineageHint::Replaces);
        let explicit = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert_eq!(
            explicit.strands[0].lineage,
            Lineage::Replaces {
                thread_id: "s0.0".into()
            }
        );
        assert_eq!(explicit.strands[0].thread_id, explicit.strands[0].strand_id);
    }

    /// A greeting beside real work is context for it, not a part of its own.
    #[test]
    fn a_greeting_beside_real_work_is_not_a_part() {
        let intent = resolve_text("hi! then refactor the parser");
        assert_eq!(intent.strands.len(), 1, "{:?}", intent.strands);
        assert_eq!(intent.reading.act, Act::Modify);
        assert!(intent.engagement.posture.note.is_none());
    }

    #[test]
    fn the_strictest_strand_governs_authority() {
        let intent = resolve_text("summarise the changelog, then deploy the service to production");
        assert_eq!(
            intent.engagement.limits.approval_ceiling,
            crate::ApprovalCeiling::Ask
        );
        assert_eq!(intent.reading.stakes, Stakes::Irreversible);
        assert_eq!(intent.engagement.posture.stop, crate::StopProfile::Effect);
    }

    /// Regression corpus for the lexical false positives two reviews found.
    #[test]
    fn everyday_requests_do_not_grow_horizons_stakes_or_evidence() {
        let cases: &[(&str, Horizon, Stakes, Evidence)] = &[
            (
                "fix the authentication bug in the login handler",
                Horizon::Turn,
                Stakes::Reversible,
                Evidence::None,
            ),
            (
                "wait until the build finishes and tell me",
                Horizon::Turn,
                Stakes::Inert,
                Evidence::None,
            ),
            (
                "where does this config live",
                Horizon::Turn,
                Stakes::Inert,
                Evidence::None,
            ),
            (
                "explain the customer model in this codebase",
                Horizon::Turn,
                Stakes::Inert,
                Evidence::None,
            ),
            (
                "how does our production deploy work?",
                Horizon::Turn,
                Stakes::Inert,
                Evidence::None,
            ),
            (
                "look at the source code for the parser",
                Horizon::Turn,
                Stakes::Inert,
                Evidence::None,
            ),
            (
                "rename the Customer struct to Client",
                Horizon::Turn,
                Stakes::Reversible,
                Evidence::None,
            ),
            (
                "fix the payment form validation",
                Horizon::Turn,
                Stakes::Reversible,
                Evidence::None,
            ),
            (
                "remember that I prefer tabs over spaces",
                Horizon::Turn,
                Stakes::Reversible,
                Evidence::None,
            ),
            (
                "what happens whenever I press ctrl-c in the REPL?",
                Horizon::Turn,
                Stakes::Inert,
                Evidence::None,
            ),
            (
                "explain step by step how the borrow checker works",
                Horizon::Turn,
                Stakes::Inert,
                Evidence::None,
            ),
            (
                "write a poem about the sea and make sure it rhymes",
                Horizon::Turn,
                Stakes::Reversible,
                Evidence::None,
            ),
        ];
        for (text, horizon, stakes, evidence) in cases {
            let intent = resolve_text(text);
            assert_eq!(intent.reading.horizon, *horizon, "{text}");
            assert_eq!(intent.reading.stakes, *stakes, "{text}");
            assert_eq!(intent.reading.evidence, *evidence, "{text}");
            assert!(!intent.engagement.posture.open_commitment, "{text}");
            assert_ne!(
                intent.engagement.limits.approval_ceiling,
                crate::ApprovalCeiling::Ask,
                "{text}"
            );
        }
        // A recurrence adjective names a thing, not a schedule.
        let adjective = resolve_text("check whether the nightly job ran");
        assert_ne!(adjective.reading.horizon, Horizon::Durable);
        assert!(!adjective.engagement.posture.open_commitment);
        let adverb = resolve_text("check the cloud bill nightly and alert me");
        assert_eq!(adverb.reading.horizon, Horizon::Durable);
        // And the ones that should still fire, still do.
        let deploy = resolve_text("deploy the service to production");
        assert_eq!(deploy.reading.stakes, Stakes::Irreversible);
        let nightly = resolve_text("check the cloud bill every day and alert me");
        assert_eq!(nightly.reading.horizon, Horizon::Durable);
        assert!(nightly.engagement.posture.open_commitment);
        let cited = resolve_text("summarise the paper and cite your sources");
        assert_eq!(cited.reading.evidence, Evidence::Cited);
    }

    /// Temporal words ask for a live value only beside a request for a fact.
    #[test]
    fn live_data_follows_a_request_for_a_fact() {
        for text in [
            "what is the current price of copper",
            "what's the weather in Delhi today",
            "give me the latest headlines",
        ] {
            let intent = resolve_text(text);
            assert!(intent.reading.domains.contains("live-data"), "{text}");
        }
        for text in [
            "Refactor the current implementation of the parser to use a state machine",
            "update the README to reflect the latest changes",
            "fix the live reload bug in the dev server",
            "hello there, how are you doing today",
            "hi, how are you today?",
            "good morning! how are you doing right now?",
            "what time is it right now",
        ] {
            let intent = resolve_text(text);
            assert!(!intent.reading.domains.contains("live-data"), "{text}");
        }
    }

    /// A question about an effect is a question: no effect is demanded.
    #[test]
    fn a_question_about_an_effect_demands_no_effect() {
        let intent = resolve_text("Did the email send?");
        assert_eq!(intent.reading.act, Act::Answer);
        assert_eq!(intent.engagement.posture.stop, crate::StopProfile::Message);
        assert_ne!(intent.reading.stakes, Stakes::Irreversible);
    }

    /// Pasted material is read as material: one question about a log is one
    /// part, whatever the log's lines say.
    #[test]
    fn a_pasted_log_is_material_not_parts() {
        let mut text = String::from("why is this service failing? here is the log:\n");
        for i in 0..300 {
            let verb = ["check", "update", "send", "delete", "deploy"][i % 5];
            text.push_str(&format!(
                "2026-09-26T10:{:02}:{:02}Z INFO worker {verb} job {i} for customer account\n",
                i / 60,
                i % 60
            ));
        }
        let intent = resolve_text(&text);
        assert_eq!(intent.strands.len(), 1, "{}", intent.strands.len());
        assert_eq!(intent.reading.act, Act::Answer);
        assert_ne!(intent.reading.stakes, Stakes::Irreversible);
        assert!(intent.engagement.posture.note.is_none());
    }

    /// Resolution is linear in the size of the request. A 50 KB paste took
    /// 105 s in a release build when every fold re-extracted the text.
    #[test]
    fn a_large_paste_resolves_quickly() {
        let mut text = String::from("analyze this data:\n");
        for i in 0..2_000 {
            text.push_str(&format!(
                "{i},{},{},north,widget-{}\n",
                i * 3,
                i % 7,
                i % 13
            ));
        }
        let started = std::time::Instant::now();
        let intent = resolve_text(&text);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(intent.reading.act, Act::Analyze);
    }

    /// Any number of real parts is read; the note names none of them.
    #[test]
    fn many_parts_are_all_read_and_the_note_names_no_part() {
        let mut text = String::from("do these:\n");
        for i in 0..20 {
            text.push_str(&format!("{}. fix the parser case {i}\n", i + 1));
        }
        let intent = resolve_text(&text);
        assert_eq!(intent.strands.len(), 20);
        let note = intent
            .engagement
            .posture
            .note
            .as_deref()
            .unwrap_or_default();
        assert!(!note.to_lowercase().contains("part "), "{note}");
        // No part is labelled with the runtime's own act names.
        for act in Act::ALL {
            assert!(!note.contains(&format!(": {}", act.as_str())), "{note}");
        }
    }

    /// A below-floor reading keeps what it *observed*: an attached image is a
    /// fact, and the ledger must not record a text-only turn.
    #[test]
    fn the_orienting_fallback_keeps_observed_modalities() {
        let attachments = vec![crate::Attachment {
            modality: Modality::Image,
            name: "x.png".into(),
        }];
        let mut req = request("zorble the frobnicator");
        req.attachments = &attachments;
        let config = ResolverConfig {
            provisional_confidence: 0.6,
            ..ResolverConfig::default()
        };
        let intent = resolve(&req, &Declared::default(), &Authority::default(), &config);
        assert_eq!(intent.provenance.tier, Tier::General);
        assert!(intent.reading.input_modalities.contains(&Modality::Image));
        assert!(
            intent
                .engagement
                .limits
                .required_modalities
                .contains(&Modality::Image)
        );
    }

    #[test]
    fn an_envelope_narrows_the_strands_it_covers() {
        let mut req = request("refactor the parser");
        req.turn_id = "turn";
        let intent = resolve(
            &req,
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        let envelope = Envelope {
            envelope_id: "env".into(),
            granted_by: "operator".into(),
            granted_at: now(),
            expires_at: None,
            spend_limit_usd: Some(2.0),
            path_scope: Vec::new(),
            tool_scope: Vec::new(),
            permission_ceiling: crate::PermissionCeiling::WorkspaceWrite,
            escalation: crate::Escalation::WaitIndefinitely,
            revoked_at: None,
        };
        let envelopes = BTreeMap::from([("turn.0".to_string(), envelope.clone())]);
        let narrowed = apply_envelopes(intent.clone(), &envelopes, Autonomy::Delegated, now());
        assert_eq!(
            narrowed.engagement.limits.permission_ceiling,
            crate::PermissionCeiling::WorkspaceWrite
        );
        assert_eq!(narrowed.engagement.limits.spend_ceiling_usd, Some(2.0));
        assert_eq!(narrowed.engagement.posture.hil, HilMode::Envelope);
        assert!(
            narrowed
                .engagement
                .limits
                .is_at_most(&intent.engagement.limits)
        );

        // A revoked grant narrows nothing and grants nothing.
        let mut revoked = envelope;
        revoked.revoked_at = Some(now());
        let untouched = apply_envelopes(
            intent.clone(),
            &BTreeMap::from([("turn.0".to_string(), revoked)]),
            Autonomy::Delegated,
            now(),
        );
        assert_eq!(untouched, intent);
    }
}
