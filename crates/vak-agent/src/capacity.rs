//! Measured per-model capacity (docs/design/68-context-engine.md §1).
//!
//! Everything here is pure: types, planning math, and the horizon-ladder
//! search state machine. No network call is made from this module — the
//! bind-time probe is driven by `vak-core`, which owns provider clients and
//! the cancellation token, and folds each rung's result back through
//! `Ladder::report`. Feedback math is likewise pure: `vak-agent`'s turn loop
//! calls `CapacityProfile::observe_usage` / `observe_instruction_failure`
//! after a real dispatch and records the result to the ledger itself.
//!
//! "Measure, never assume" (design doc principle 1) means the only
//! constants in this file are search parameters and confidence levels, each
//! named and doc-commented below — nothing here caps a request.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// EWMA smoothing for `tokens_per_char` and `prefill_tps` feedback (§1
/// "Feedback"). Low enough that one unusually short or long turn cannot
/// swing the estimate, high enough that a real regime change (a different
/// model behind the same route) is folded in within a handful of turns.
const FEEDBACK_EWMA_ALPHA: f64 = 0.3;

/// Tokens-per-char used only before any real usage has been observed
/// (`Ewma::samples == 0`). The instant the provider reports usage even
/// once, the measured value replaces it — this is a bootstrap value, not a
/// cap or a long-lived default.
const UNCALIBRATED_TOKENS_PER_CHAR: f64 = 0.25;

/// A request at or above this fraction of the current horizon that also
/// failed an explicit instruction is treated as evidence the horizon is
/// wrong rather than noise (§1 "Horizon tightening"). Below this fraction a
/// failure is presumed unrelated to context size.
const HORIZON_FAILURE_FRACTION: f64 = 0.8;

/// Confidence assigned to a horizon lowered by feedback (as opposed to a
/// fresh probe): high enough to act on immediately, low enough that the
/// scheduled re-probe still wins once it runs.
const HORIZON_FEEDBACK_CONFIDENCE: f64 = 0.6;

/// Confidence assigned to an unverified hosted-model horizon that starts
/// from `declared_window` because the operator has not opted into full
/// hosted probing (§1 "Cost control for hosted models").
const HOSTED_UNPROBED_CONFIDENCE: f64 = 0.3;

/// Confidence assigned to a horizon the ladder actually converged on.
const PROBED_CONFIDENCE: f64 = 0.9;

/// The horizon ladder's binary search (between the last accepted rung and
/// the first failing one) stops once the remaining gap is under this
/// fraction of the failing size — tight enough that the reported horizon is
/// useful, loose enough to bound probe cost to a handful of rungs (§1).
const LADDER_SEARCH_STOP_FRACTION: f64 = 0.25;

/// Named starting rungs for the horizon ladder (§1): "4k, 8k, 16k, 32k,
/// 64k, … up to declared_window × 0.9". Growth continues geometrically
/// past the last named rung, doubling, until the cap.
const LADDER_START_RUNGS: [u64; 5] = [4_000, 8_000, 16_000, 32_000, 64_000];

/// Ladder rungs never probe past this fraction of the declared window —
/// beyond it a provider is expected to reject outright on every run,
/// wasting a rung on a foregone conclusion.
const LADDER_MAX_FRACTION: f64 = 0.9;

/// Local-model profiles are cheap to re-probe (docs/design/68 §1: "the
/// probe is free apart from time") and the weights behind a name can change
/// under an operator's feet (an `ollama pull` swaps them), so the TTL is
/// short.
pub const LOCAL_PROFILE_TTL: Duration = Duration::from_secs(24 * 3600);

/// Hosted profiles are slower/costlier to re-probe in full and the
/// underlying model changes far less often once pinned to a version, so the
/// TTL is long.
pub const HOSTED_PROFILE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Identifies one measured capacity profile. Quantisation is part of the
/// identity because two quantisations of the same model id can have very
/// different real horizons despite sharing a declared window; it is `None`
/// when the provider does not expose it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileKey {
    pub provider: String,
    pub model: String,
    pub quantisation: Option<String>,
}

/// Exponentially-weighted moving average, with the sample count needed to
/// tell "never observed" apart from "observed and stable at zero".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ewma {
    pub value: f64,
    pub alpha: f64,
    pub samples: u32,
}

impl Ewma {
    pub fn new(alpha: f64) -> Self {
        Ewma {
            value: 0.0,
            alpha,
            samples: 0,
        }
    }

    /// Folds one observation in. The first observation replaces the
    /// (otherwise meaningless) zero seed outright rather than being
    /// blended into it, so one real sample is fully trusted immediately.
    pub fn observe(&mut self, sample: f64) {
        if self.samples == 0 {
            self.value = sample;
        } else {
            self.value = self.alpha * sample + (1.0 - self.alpha) * self.value;
        }
        self.samples = self.samples.saturating_add(1);
    }
}

/// The largest prompt size at which the model is known to still follow an
/// explicit instruction (§1). `confidence` and `last_confirmed` make it
/// possible to tell a freshly probed horizon from an assumed or
/// feedback-lowered one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Horizon {
    pub tokens: u64,
    pub confidence: f64,
    pub last_confirmed: SystemTime,
}

/// What is known about a provider's prefix-cache behaviour for this model
/// (§1 "Cache rung").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheBehaviour {
    Unknown,
    None,
    PrefixStable,
    ProviderReported,
}

/// One rung of the horizon ladder: a prompt size sent to the provider,
/// whether the provider accepted the request at all, and — when accepted —
/// whether the model followed the probe instruction embedded at that size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rung {
    pub tokens: u64,
    pub accepted: bool,
    pub followed_instruction: Option<bool>,
    pub prefill_ms: Option<u64>,
}

/// Audit trail for a `CapacityProfile`: when it was probed, which rungs
/// were tried, any other signals that fed the decision, and a digest of the
/// provider metadata it was built from (so a metadata change is detectable
/// even inside the TTL).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeProvenance {
    pub probed_at: SystemTime,
    pub rungs: Vec<Rung>,
    pub signals: Vec<String>,
    pub metadata_digest: String,
}

/// A ledger-recorded, per-`(provider, model, quantisation)` measurement of
/// what a model can actually do — the only input to context budgeting
/// (docs/design/68-context-engine.md §1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityProfile {
    /// Provider metadata (existing `model_context()`), before any probing.
    pub declared_window: u64,
    /// The largest prompt the provider has actually accepted.
    pub verified_window: Option<u64>,
    /// The largest prompt at which the model still followed a tool
    /// instruction — the number budgeting actually uses.
    pub instruction_horizon: Horizon,
    /// `usage.input_tokens / chars_sent`, fed by every turn's receipt.
    pub tokens_per_char: Ewma,
    /// Input tokens per second of prefill, cache-miss turns only.
    pub prefill_tps: Ewma,
    pub cache: CacheBehaviour,
    /// Provider max output, or the largest completion observed.
    pub output_reserve: u64,
    pub provenance: ProbeProvenance,
    /// Set when feedback lowered the horizon (§1 "schedules a re-probe");
    /// the next bind of this key re-runs the ladder instead of trusting the
    /// lowered estimate indefinitely.
    #[serde(default)]
    pub needs_reprobe: bool,
}

impl CapacityProfile {
    /// A profile built from provider metadata alone, with no probe run —
    /// the hosted-and-not-opted-in path (§1 "Cost control for hosted
    /// models"): the horizon starts at the declared window with low
    /// confidence and is tightened only by feedback or a later full probe.
    pub fn from_metadata_only(
        declared_window: u64,
        output_reserve: u64,
        metadata_digest: String,
        probed_at: SystemTime,
    ) -> Self {
        CapacityProfile {
            declared_window,
            verified_window: None,
            instruction_horizon: Horizon {
                tokens: declared_window,
                confidence: HOSTED_UNPROBED_CONFIDENCE,
                last_confirmed: probed_at,
            },
            tokens_per_char: Ewma::new(FEEDBACK_EWMA_ALPHA),
            prefill_tps: Ewma::new(FEEDBACK_EWMA_ALPHA),
            cache: CacheBehaviour::Unknown,
            output_reserve,
            provenance: ProbeProvenance {
                probed_at,
                rungs: Vec::new(),
                signals: vec!["metadata-only: hosted probing not opted in".into()],
                metadata_digest,
            },
            needs_reprobe: false,
        }
    }

    /// A profile built from a converged horizon ladder (§1 "Horizon
    /// ladder").
    pub fn from_probe(
        declared_window: u64,
        verified_window: Option<u64>,
        horizon: Horizon,
        cache: CacheBehaviour,
        output_reserve: u64,
        provenance: ProbeProvenance,
    ) -> Self {
        CapacityProfile {
            declared_window,
            verified_window,
            instruction_horizon: horizon,
            tokens_per_char: Ewma::new(FEEDBACK_EWMA_ALPHA),
            prefill_tps: Ewma::new(FEEDBACK_EWMA_ALPHA),
            cache,
            output_reserve,
            provenance,
            needs_reprobe: false,
        }
    }

    /// Remaining input budget for a request: the horizon minus the
    /// measured stable prefix, the tail, the output reserve, and any
    /// reserve already committed by the current turn. Saturates to zero
    /// rather than underflowing — a caller over budget gets "nothing left",
    /// never a wrapped huge number.
    pub fn budget(&self, prefix_tokens: u64, tail_tokens: u64, current_turn_reserve: u64) -> u64 {
        self.instruction_horizon
            .tokens
            .saturating_sub(prefix_tokens)
            .saturating_sub(tail_tokens)
            .saturating_sub(self.output_reserve)
            .saturating_sub(current_turn_reserve)
    }

    /// Whether `tokens_per_char` reflects a real observation rather than
    /// the uncalibrated bootstrap value.
    pub fn is_calibrated(&self) -> bool {
        self.tokens_per_char.samples > 0
    }

    /// Estimated token count for `chars`, using the measured tokens/char
    /// once calibrated and the fixed bootstrap value only before that.
    pub fn estimate_tokens(&self, chars: u64) -> u64 {
        let tpc = if self.is_calibrated() {
            self.tokens_per_char.value
        } else {
            UNCALIBRATED_TOKENS_PER_CHAR
        };
        (chars as f64 * tpc).round() as u64
    }

    /// Folds one turn's real usage into the profile (§1 "Feedback").
    /// `cache_miss` gates the prefill measurement: prefill throughput is
    /// only meaningful when the provider actually re-evaluated the prefix.
    pub fn observe_usage(
        &mut self,
        chars_sent: u64,
        usage: &vak_llm::Usage,
        first_token_latency_ms: Option<u64>,
        cache_miss: bool,
    ) {
        if chars_sent > 0 && usage.input_tokens > 0 {
            self.tokens_per_char
                .observe(usage.input_tokens as f64 / chars_sent as f64);
        }
        if cache_miss && usage.input_tokens > 0 {
            // Prefer the provider's own reported prefill latency (Ollama
            // fills `usage.prefill_ms`) over the caller's wall-clock
            // estimate, since it excludes model-load and network time.
            if let Some(ms) = usage.prefill_ms.or(first_token_latency_ms)
                && ms > 0
            {
                self.prefill_tps
                    .observe(usage.input_tokens as f64 / (ms as f64 / 1000.0));
            }
        }
    }

    /// Records that a request of `request_tokens` failed an explicit
    /// instruction (required card not emitted, required tool not called, a
    /// stop-policy block) the runtime already classifies. Only requests
    /// near the current horizon count as evidence about the horizon itself
    /// (§1 "Horizon tightening"); this never raises the horizon; a re-probe
    /// is the only path that can.
    pub fn observe_instruction_failure(&mut self, request_tokens: u64) {
        let threshold = HORIZON_FAILURE_FRACTION * self.instruction_horizon.tokens as f64;
        if (request_tokens as f64) < threshold {
            return;
        }
        let lowered = self.instruction_horizon.tokens.min(request_tokens);
        self.instruction_horizon = Horizon {
            tokens: lowered,
            confidence: HORIZON_FEEDBACK_CONFIDENCE,
            last_confirmed: SystemTime::now(),
        };
        self.needs_reprobe = true;
    }

    /// Whether this profile should be re-probed before being trusted again:
    /// past its TTL (24h local / 7d hosted), or contradicted metadata.
    /// `current_metadata_digest` is compared by the caller because only it
    /// knows the freshly fetched metadata; passing `None` skips that check.
    pub fn is_stale(&self, now: SystemTime, local: bool) -> bool {
        let ttl = if local {
            LOCAL_PROFILE_TTL
        } else {
            HOSTED_PROFILE_TTL
        };
        match now.duration_since(self.provenance.probed_at) {
            Ok(elapsed) => elapsed > ttl,
            // Clock moved backwards since the probe: treat as fresh rather
            // than fabricating staleness from an impossible duration.
            Err(_) => false,
        }
    }
}

/// The horizon-ladder state machine (§1 "Horizon ladder"). Pure: the caller
/// drives it by asking `next_rung()`, sending exactly that request, and
/// folding the outcome back with `report()`. No network call happens here.
#[derive(Debug, Clone)]
pub struct Ladder {
    rungs: Vec<u64>,
    next_index: usize,
    last_pass: Option<u64>,
    first_fail: Option<u64>,
    searching: bool,
    result: Option<Horizon>,
    verified_window: Option<u64>,
}

impl Ladder {
    /// Builds the rung sequence: the named starting sizes filtered to (and
    /// extended geometrically past, by doubling) `declared_window * 0.9`.
    pub fn new(declared_window: u64) -> Self {
        let cap = (declared_window as f64 * LADDER_MAX_FRACTION) as u64;
        let mut rungs: Vec<u64> = LADDER_START_RUNGS
            .into_iter()
            .filter(|&r| r <= cap)
            .collect();
        let mut next = LADDER_START_RUNGS
            .last()
            .copied()
            .unwrap_or(4_000)
            .saturating_mul(2);
        while next <= cap && next > 0 {
            rungs.push(next);
            next = next.saturating_mul(2);
        }
        if rungs.is_empty() && cap > 0 {
            rungs.push(cap);
        }
        Ladder {
            rungs,
            next_index: 0,
            last_pass: None,
            first_fail: None,
            searching: false,
            result: None,
            verified_window: None,
        }
    }

    /// The next prompt size to probe, or `None` once the ladder has
    /// converged on a horizon.
    pub fn next_rung(&self) -> Option<u64> {
        if self.result.is_some() {
            return None;
        }
        if self.searching {
            let lo = self.last_pass.unwrap_or(0);
            let hi = self.first_fail?;
            if hi <= lo {
                return None;
            }
            let mid = lo + (hi - lo) / 2;
            if mid <= lo { None } else { Some(mid) }
        } else {
            self.rungs.get(self.next_index).copied()
        }
    }

    /// Folds one rung's outcome back in. `accepted` is false for a
    /// provider-level rejection (400/413 style context error); `followed`
    /// is meaningless when `accepted` is false.
    pub fn report(&mut self, tokens: u64, accepted: bool, followed: bool) {
        if !accepted {
            self.verified_window = Some(match self.verified_window {
                Some(v) => v.min(tokens.saturating_sub(1)),
                None => tokens.saturating_sub(1),
            });
            self.first_fail = Some(match self.first_fail {
                Some(f) => f.min(tokens),
                None => tokens,
            });
            self.searching = true;
        } else if followed {
            self.last_pass = Some(match self.last_pass {
                Some(p) => p.max(tokens),
                None => tokens,
            });
            if !self.searching {
                self.next_index += 1;
            }
        } else {
            // Accepted but the model did not follow the instruction: this
            // rung IS the instruction-horizon boundary, same as a rejection
            // for search purposes.
            self.first_fail = Some(match self.first_fail {
                Some(f) => f.min(tokens),
                None => tokens,
            });
            self.searching = true;
        }
        self.converge();
    }

    fn converge(&mut self) {
        if self.result.is_some() {
            return;
        }
        if let (Some(lo), Some(hi)) = (self.last_pass, self.first_fail) {
            if hi <= lo {
                self.result = Some(self.horizon_at(lo));
                return;
            }
            let gap = (hi - lo) as f64 / hi as f64;
            if gap < LADDER_SEARCH_STOP_FRACTION {
                self.result = Some(self.horizon_at(lo));
            }
        } else if !self.searching && self.next_index >= self.rungs.len() {
            // Every named rung passed with no failure ever observed: the
            // horizon is at least the last rung tried.
            if let Some(lo) = self.last_pass {
                self.result = Some(self.horizon_at(lo));
            }
        }
    }

    fn horizon_at(&self, tokens: u64) -> Horizon {
        Horizon {
            tokens,
            confidence: PROBED_CONFIDENCE,
            last_confirmed: SystemTime::now(),
        }
    }

    /// The converged horizon, once `next_rung()` has returned `None`.
    pub fn result(&self) -> Option<Horizon> {
        self.result.clone()
    }

    /// The largest prompt size the provider is known to have accepted at
    /// all (independent of instruction-following), when a rejection was
    /// observed.
    pub fn verified_window(&self) -> Option<u64> {
        self.verified_window
    }
}

/// Builds a probe request of roughly `tokens_target` tokens: alternating
/// user/assistant filler messages of varied, non-repeating text (so no
/// adjacent-message prefix-cache shortcut can mask the real prefill cost),
/// ending with an instruction to call `probe_ack`. `tokens_per_char` should
/// come from the profile being probed when calibrated, so rung sizes land
/// close to their target on real providers.
pub fn probe_request(
    tokens_target: u64,
    tokens_per_char: f64,
    model: &str,
) -> vak_llm::ChatRequest {
    let tpc = if tokens_per_char > 0.0 {
        tokens_per_char
    } else {
        UNCALIBRATED_TOKENS_PER_CHAR
    };
    let chars_target = (tokens_target as f64 / tpc) as u64;
    let mut messages = Vec::new();
    let mut chars_written: u64 = 0;
    let mut seed: u64 = 0;
    while chars_written < chars_target {
        let text = probe_filler_text(seed);
        chars_written += text.len() as u64;
        messages.push(if seed.is_multiple_of(2) {
            vak_llm::Message::user_text(text)
        } else {
            vak_llm::Message::assistant(vec![vak_llm::ContentBlock::text(text)])
        });
        seed += 1;
    }
    messages.push(vak_llm::Message::user_text(
        "Call the probe_ack tool now with {\"ok\": true} as its only argument. \
         Do not answer in prose and do not call any other tool.",
    ));
    let mut request = vak_llm::ChatRequest::new(model);
    request.messages = messages;
    request.tools = vec![vak_llm::ToolDefinition::probe_ack()];
    request.max_tokens = 32;
    request
}

/// Deterministic, non-repeating filler text for one probe message. Seeded
/// by message index so a probe run is reproducible, and varied so the
/// provider cannot shortcut prefill via a repeated-content optimisation.
fn probe_filler_text(seed: u64) -> String {
    const WORDS: [&str; 16] = [
        "ridge",
        "cobalt",
        "ferry",
        "lantern",
        "quartz",
        "meridian",
        "otter",
        "glacier",
        "ember",
        "thicket",
        "vellum",
        "cinder",
        "harbor",
        "tundra",
        "opal",
        "switchback",
    ];
    let mut s = String::new();
    for j in 0..40u64 {
        let idx = ((seed.wrapping_mul(31).wrapping_add(j.wrapping_mul(17))) as usize) % WORDS.len();
        if j > 0 {
            s.push(' ');
        }
        s.push_str(WORDS[idx]);
    }
    s
}

/// Whether a probe response followed the instruction: a `ToolUse` block
/// named `probe_ack` anywhere in the response.
pub fn followed(response: &vak_llm::AssistantMessage) -> bool {
    response
        .content
        .iter()
        .any(|b| matches!(b, vak_llm::ContentBlock::ToolUse { name, .. } if name == "probe_ack"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn budget_saturates_instead_of_underflowing() {
        let profile = flat_profile_with_reserve(1_000, 0);
        assert_eq!(profile.budget(200, 100, 0), 700);
        // prefix + tail + reserve exceed the horizon: saturates to 0, never wraps.
        assert_eq!(profile.budget(900, 200, 0), 0);
    }

    #[test]
    fn ewma_first_sample_replaces_zero_seed() {
        let mut ewma = Ewma::new(0.3);
        ewma.observe(10.0);
        assert_eq!(ewma.value, 10.0);
        assert_eq!(ewma.samples, 1);
        ewma.observe(20.0);
        assert!((ewma.value - (0.3 * 20.0 + 0.7 * 10.0)).abs() < 1e-9);
        assert_eq!(ewma.samples, 2);
    }

    #[test]
    fn estimate_tokens_uses_bootstrap_until_calibrated() {
        let profile = flat_profile(10_000);
        assert_eq!(profile.estimate_tokens(400), 100); // 0.25 tokens/char
        let mut calibrated = profile;
        calibrated.tokens_per_char.observe(0.5);
        assert_eq!(calibrated.estimate_tokens(400), 200);
    }

    #[test]
    fn observe_usage_updates_tokens_per_char_and_prefill_on_cache_miss_only() {
        let mut profile = flat_profile(10_000);
        let usage = vak_llm::Usage {
            input_tokens: 1_000,
            output_tokens: 10,
            prefill_ms: Some(2_000),
            ..Default::default()
        };
        profile.observe_usage(2_000, &usage, None, true);
        assert_eq!(profile.tokens_per_char.samples, 1);
        assert!((profile.tokens_per_char.value - 0.5).abs() < 1e-9);
        assert_eq!(profile.prefill_tps.samples, 1);
        assert!((profile.prefill_tps.value - 500.0).abs() < 1e-9); // 1000 tok / 2s

        let mut cache_hit_profile = flat_profile(10_000);
        cache_hit_profile.observe_usage(2_000, &usage, None, false);
        assert_eq!(cache_hit_profile.prefill_tps.samples, 0);
    }

    #[test]
    fn horizon_never_widens_on_feedback() {
        let mut profile = flat_profile(10_000);
        profile.observe_instruction_failure(9_000); // >= 0.8 * 10_000
        assert_eq!(profile.instruction_horizon.tokens, 9_000);
        assert!(profile.needs_reprobe);
        assert_eq!(profile.instruction_horizon.confidence, 0.6);

        // A later "failure" reported at a larger size than the (already
        // lowered) horizon must not raise it back up.
        profile.observe_instruction_failure(9_500);
        assert_eq!(profile.instruction_horizon.tokens, 9_000);
    }

    #[test]
    fn observe_instruction_failure_ignores_requests_far_below_horizon() {
        let mut profile = flat_profile(10_000);
        profile.observe_instruction_failure(1_000); // well under 0.8 * horizon
        assert_eq!(profile.instruction_horizon.tokens, 10_000);
        assert!(!profile.needs_reprobe);
    }

    #[test]
    fn is_stale_respects_local_vs_hosted_ttl() {
        let profile = flat_profile(10_000);
        let just_over_a_day = profile.provenance.probed_at + Duration::from_secs(25 * 3600);
        assert!(profile.is_stale(just_over_a_day, true));
        assert!(!profile.is_stale(just_over_a_day, false));
    }

    #[test]
    fn ladder_converges_via_binary_search_within_tolerance() {
        // Real horizon is 20_000; every rung <= 20_000 passes, every rung
        // above fails the instruction.
        let mut ladder = Ladder::new(200_000);
        let real_horizon = 20_000u64;
        let mut iterations = 0;
        while let Some(rung) = ladder.next_rung() {
            iterations += 1;
            assert!(iterations < 100, "ladder did not converge");
            let followed = rung <= real_horizon;
            ladder.report(rung, true, followed);
        }
        let horizon = ladder.result().expect("ladder should converge");
        assert!(horizon.tokens <= real_horizon);
        // within the 25% search-stop tolerance of the true horizon
        assert!(horizon.tokens as f64 >= real_horizon as f64 * 0.75);
    }

    #[test]
    fn ladder_records_verified_window_on_provider_rejection() {
        let mut ladder = Ladder::new(200_000);
        // First rung is accepted and followed.
        let first = ladder.next_rung().expect("first rung");
        ladder.report(first, true, true);
        // Next rung is rejected outright by the provider.
        let second = ladder.next_rung().expect("second rung");
        ladder.report(second, false, false);
        assert_eq!(ladder.verified_window(), Some(second - 1));
    }

    #[test]
    fn probe_request_shape_alternates_roles_and_ends_with_probe_ack_instruction() {
        let req = probe_request(1_000, 0.25, "test-model");
        assert_eq!(req.max_tokens, 32);
        assert_eq!(req.tools.len(), 1);
        assert_eq!(req.tools[0].name, "probe_ack");
        assert!(req.cache.is_none());
        let last = req
            .messages
            .last()
            .expect("at least the instruction message");
        assert!(last.text_content().contains("probe_ack"));
        // Alternates starting with user.
        assert_eq!(req.messages[0].role, vak_llm::Role::User);
        if req.messages.len() > 2 {
            assert_eq!(req.messages[1].role, vak_llm::Role::Assistant);
        }
        // Roughly sized: within a generous tolerance of the token target
        // translated through the given tokens/char.
        let total_chars: usize = req.messages.iter().map(|m| m.text_content().len()).sum();
        assert!(total_chars as f64 >= 1_000.0 / 0.25 * 0.5);
    }

    #[test]
    fn followed_detects_probe_ack_tool_use_only() {
        let with_ack = vak_llm::AssistantMessage {
            content: vec![vak_llm::ContentBlock::ToolUse {
                id: "1".into(),
                name: "probe_ack".into(),
                input: serde_json::json!({"ok": true}),
            }],
            stop_reason: vak_llm::StopReason::ToolUse,
            usage: vak_llm::Usage::default(),
            model: "m".into(),
            response_id: None,
        };
        assert!(followed(&with_ack));

        let prose_only = vak_llm::AssistantMessage {
            content: vec![vak_llm::ContentBlock::text("sure, here you go")],
            stop_reason: vak_llm::StopReason::EndTurn,
            usage: vak_llm::Usage::default(),
            model: "m".into(),
            response_id: None,
        };
        assert!(!followed(&prose_only));
    }

    fn flat_profile(horizon_tokens: u64) -> CapacityProfile {
        flat_profile_with_reserve(horizon_tokens, 1_024)
    }

    fn flat_profile_with_reserve(horizon_tokens: u64, output_reserve: u64) -> CapacityProfile {
        CapacityProfile::from_probe(
            horizon_tokens,
            None,
            Horizon {
                tokens: horizon_tokens,
                confidence: PROBED_CONFIDENCE,
                last_confirmed: SystemTime::now(),
            },
            CacheBehaviour::Unknown,
            output_reserve,
            ProbeProvenance {
                probed_at: SystemTime::now(),
                rungs: Vec::new(),
                signals: Vec::new(),
                metadata_digest: "digest".into(),
            },
        )
    }
}
