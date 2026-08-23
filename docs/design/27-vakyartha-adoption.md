# 27 — Vakyartha adoption

Result of a deep study of the sibling project **vakyartha** v5.30.128
(local-first agent workbench: Tauri 2 + Next.js over a Rust execution
kernel, `/Users/nisheethranjan/Projects/vakyartha`), August 2026. Goal:
extract mechanisms with genuine leverage for a Rust coding-agent harness;
reject domain-specific weight. This continues an existing lineage —
`17-context.md` was inspired by vakyartha's prompt-budget protocol and the
planner's repair→validate loop came from the same source (see
`00-roadmap.md`, research decisions).

Vakyartha's strongest cross-cutting discipline, worth copying everywhere:

> **Facts remove, predictions reorder. Absent is UNKNOWN, never zero.**
> Gates act on recorded facts; predictions only rank. Missing data stays
> unknown — it is never treated as zero/bad/good.

## North star

Every phase here serves one competitive axis: **long-horizon reliability**.
The evidence is convergent — our own recorded meta-lesson (harness-only
changes moved Terminal-Bench 2.0 by 10–14pp, `06-research-notes.md`); the
Horizon Gap survey ("harness engineering, not the underlying model, is
often the binding constraint"); METR's task-horizon doubling every ~4–7
months and accelerating; LoopsBench showing the field shifting from
harness engineering to loop engineering with the best frontier
configuration resolving just 25% of dependency-DAG tasks; and Anthropic's
own harness posts (evaluator separation, done-contracts, reset-with-
handoff). Vakyartha's imports are chosen only where they serve that axis;
Phase H sources directly from the same public evidence.

## What we do NOT re-import

| Vakyartha mechanism | Verdict | Reason |
|---|---|---|
| Reliability stack (retries, watchdogs, breakers, HIL gates) | have equivalents | `15-reliability.md`, `22-gateway.md` |
| Append-only ledgers + git-as-history | have equivalents | `02-sessions.md`, checkpoints |
| Static flows + planner→repair→validate | already ours | `10-flows.md`, `11-planner.md` (adopted earlier) |
| Cost budgets/schedules as raw concepts | table stakes | we adopt the *discipline* (Phase D), not the concept |
| Micro Apps visual layer | **reject** | violates simplicity-as-core thesis (`AGENTS.md`) |
| Knowledge repo / OCR / hand-rolled HNSW file format | **reject** | different domain. Adopt one rule: embedding substrate identity = provider+model+dims+normalization, never dimension alone |
| AFM / Apple-specific lanes | **reject** | platform lock-in |
| Fixed %-budget context slices (40/20/20/20) | **reject** | their own weakest mechanism; blunt instrument |
| Plan-learning receipts | **reject** | aspirational, thin stamping heuristic |

## Confidence & falsification

This doc originally presented all phases with roughly uniform confidence;
that overstated certainty. Corrected standing, per phase:

| Phase | Standing confidence | Falsifier / trigger | Status (2026-08-23) |
|---|---|---|---|
| A receipts + ceiling | enabler; medium standalone | none needed if B–E land | accepted as foundation |
| B frozen ladder | conditional on multi-provider + unattended pain | a provider outage killing a live session | **fired** — user-attested incident |
| C context gate | insurance until context loss observed | a long-horizon task demonstrably forgetting earlier state | **fired** — user-attested incident |
| D FinOps | high (gap exists today: unattended gateway spend is uncapped) | n/a — gap ships now | confirmed by runaway/vague-spend report |
| E flows adopt/diff | product bet on replay appetite | a completed run manually rerun/adapted by hand | **fired** — user-attested |
| F process | hygiene; low risk; compounding | n/a | ongoing |
| G projection (conditional) | cosmetic until the data layer exists | getting lost during a real flow run; remote flow-control demand | parked behind A/E |
| H loop engineering | evidence-strongest tier: 2026 public frontier convergence + attested context loss | a frontier harness ships audited completion/goal mode and ours lags; or dogfood proves a mechanism dead weight → strip it | adopted 2026-08-24 |

All four demand triggers were attested on 2026-08-23, which promotes B, C,
D, E from speculative to demand-backed. Standing rule going forward —
vakyartha's own methodology: **the first fixture/scenario for each phase
comes from the actual incident**, not a synthetic one (Phase C's first
recall fixture = the forgetting case; Phase F scenarios start from real
regressions). If a future revision of this doc cannot cite incidents for a
phase, that phase reverts to deferred.

## Phase A — Work receipts + dispatch ceiling

Status: **landed** — `vak_llm::work` types, `StepLedger` threaded through
the reliability helper, `receipt` session entries (projection-neutral),
ceiling default `(max_retries+1) × (run_retry_attempts+1)`; see roadmap
row for the gate evidence.

Every provider call becomes an auditable typed event. Today attempts are
implicit (retry counters, breaker state machine); after this they are
ledger entries.

**Types** (new `work.rs` in vak-llm; aggregated in vak-agent):

```rust
pub struct WorkRequest {   // built internally per step; not user-facing
    pub purpose: Purpose,            // classify | plan | execute | verify | summarize
    pub contract_ref: ContractId,    // points at the frozen header
}
pub struct DispatchAttempt {
    pub ordinal: u32,
    pub reason: AttemptReason,       // initial | route-fallback | schema-repair | endurance-retry | last-resort
    pub domain: FailureDomain,       // account | provider | model | request | network | deadline | unknown
    pub settlement: Settlement,      // ok | failed | cancelled | unknown   (unknown = fail-closed against paid fallback)
    pub latency_ms: u64,
    pub usage: Option<Usage>,
}
pub struct WorkReceipt {    // success path
    pub request: WorkRequest,
    pub ladder: Vec<(ProviderId, ModelId)>,  // frozen route walked (empty = single-model legacy)
    pub winning_index: usize,
    pub attempts: Vec<DispatchAttempt>,
}
pub struct WorkFailureReceipt { request, attempts, typed_reason }
```

**Dispatch ceiling**: shared per-step budget enforced before every paid
dispatch; exhaustion is typed and fail-closed (never retried through,
never breaker-tripping). Single-ladder default codifies today's exact
worst case, `(max_retries + 1) * (run_retry_attempts + 1)` (=28 at
defaults), so Phase A changes zero resilience behavior while making the
cap explicit and configurable; the frozen ladder (Phase B) tightens the
formula to `ladder.len() + repair_allowance` once multi-candidate routes
exist. The breaker/endurance machinery consumes `FailureDomain`s instead
of string-matching errors (blind domains trip the breaker as today;
informed transience keeps feeding endurance only).

**Attempt vocabulary**: `initial | retry | endurance-retry` ship now;
`route-fallback | schema-repair | last-resort` arrive with Phase B's
ladder walk. Attempts are one row per actual provider dispatch — never
scheduling bookkeeping. Settlements follow the epistemics rule: pre-
dispatch rejections are `failed`; transport/mid-stream failures that may
have consumed compute are `unknown`; user aborts are `cancelled`.

**Ledger** (invariant 1): new session entry kind `"receipt"` written once
per completed step (success or failure). `derive_messages()` ignores
receipts for prompting — they are audit, not model-visible input.

Resolves the drift flagged in review: README advertised vak-flow
"receipts" and `02-sessions.md` listed them under Later; both are updated
with this phase.

**Tests**: receipt entry round-trips through projection untouched;
ceiling exhaustion fails closed mid-ladder; `unknown` settlement blocks
paid fallback; breaker classification unit matrix per `FailureDomain`.

## Phase B — Frozen ladder routing

Today `FrozenContract` pins exactly one provider/model; losing that
provider loses the turn (invariant 7 forbids mid-contract switching).
Resolution adopted (user-confirmed): **freeze the ordered ladder INTO the
contract at admission**. Walking a frozen ladder is not switching — it is
the contract executing.

Mechanics:

1. At session/turn admission, `Core::admit_route()` builds the candidate
   set from configured providers + `discover_models()` (invariant 9),
   applies constraints (key present, capability fits purpose, budget head
   exists), then orders it with a **versioned pure function**
   `order_ladder_v1(set, evidence)` — cheap-first by default, quality-band
   promotion for planning/verification purposes. The whole ladder is
   written into the frozen header.
2. Dispatch walks the ladder top-down on typed failures (Phase A
   domains). Success commits; the receipt records the full walk.
3. **Evidence ledger** (`$VAKCODER_HOME/routing-evidence.jsonl`):
   append-only `(provider, model) -> {success, failure, UNKNOWN}` outcome
   counts + p50 latency, TTL-decayed, cleared by success. Billing without
   a rated outcome contributes **unknown**, never zero ("being billed
   proves nothing about answer quality"). Ordering shrinks toward neutral
   priors proportional to sample size.

Invariant 7 rewording when this lands: *"Transient provider failures
retry within the frozen route ladder committed at admission; the ladder is
part of the contract, so walking it never changes the contract. Never
switch providers outside the frozen ladder."* Update `AGENTS.md` +
`15-reliability.md` in the same commit.

**Tests**: admission freezes identical ladders for identical inputs
(pure function); ladder walk stops on first success; evidence JSONL
corruption degrades to empty-with-warning (never misranks); single-model
config produces the exact legacy behavior (ladder length 1).

## Phase C — Context packet accounting + deterministic gate

Status: **landed** — partitions on plans/entries (serde-back-compat),
counts on `ContextCompacted`, `run_context_scorecard()` printed by every
offline eval.

Retires the deferred bullets in `17-context.md` ("Deliberately not built").

1. **Partition accounting**: every compaction and every projected packet
   emits `{selected_entry_ids, dropped_entry_ids}` partitioning all
   entries strictly before the current turn. Surfaced on
   `ContextCompacted` events and stored in the compaction entry.
2. **Scorecard gate** in vak-eval (deterministic fixtures, no live model,
   target ≤100ms): given a fixture session + query, assert
   - **recall**: required turns present in `selected`
   - **precision**: noise fixtures absent from `selected`
   - **evidence coverage**: anchors cited by the expected answer appear
     in the surviving packet; forbidden material does not leak
   - **quality-beats-fit**: a packet missing required evidence fails even
     if it fits the budget
3. Every fixed context bug becomes an anonymized fixture (vakyartha
   practice). Metrics print as a pass/fail scorecard in eval output.

**Tests**: partition covers the full prefix (sum invariant); scorecard
catches seeded regressions (drop-a-required-turn fixture fails).

## Phase D — FinOps (spend is a safety property)

Status: **landed** with two honest deviations — (1) the one-time
cap-raise exists on `CoreSpendGate::raise_once()` but has no UI surface
yet (the Ask currently resolves approve/deny only); (2) unit lanes for
non-token media costs are reserved, not tracked — unpriced models record
`usd: null` (UNKNOWN), never fake zero.

Unattended gateway surfaces currently spend with no cap. Budget admission
is therefore security work, aligned with `24-agent-security.md`.

1. **Cost ledger** `$VAKCODER_HOME/cost-log.jsonl`: every settled
   dispatch appends `{provider, model, usage, usd, source: actual|estimated,
   attribution: {session_id, task_id?, delivery_surface?}, ts}`.
   Attribution joins by durable ids only; unattributed rows stay
   labeled `Unattributed`.
2. **Pricing leaves the TUI heuristic** (`vak-tui/src/pricing.rs`
   substring table): move to `[finops] price_overrides` config + discovered
   catalog prices where providers publish them; TUI reads the same source.
   Estimates always labeled.
3. **Pre-dispatch budget admission**: `[finops] max_run_usd`,
   `max_day_usd`. Enforced before each model call from the ledger; a run
   exceeding its cap gets a **typed budget Ask** whose options are finite
   (raise-cap-once / finish-read-only / abort) — dismissal = abort.
   Unattended turns auto-deny per gateway rules (silence means no).
4. Unit lanes preserved: token costs and non-token units (e.g. image
   counts) stay separate record kinds, never merged into fake totals.

**Tests**: ledger append/settle ordering; cap refusal mid-run with typed
error + receipt; day-window math across restarts; unattended auto-deny.

## Phase E — Runs → reusable flows (replay)

Convert proven work into governed reruns. Substrate already exists:
flow-run state ledgers freeze definition TOML, planner attempts are
audited, best-of-N proves keep=merge promotion.

1. **Builder**: `flows adopt --from <session>` (and HTTP endpoint)
   inspects a completed session/plan ledger, extracts the settled step
   sequence into `.vakcoder/flows/<name>.toml` with parameterized inputs.
   Provider/model come **only** from receipts (Phase A), never caller hints.
2. **Frozen snapshot per run**: flow runs already store
   `definition_toml`; make explicit that resume/repair executes against
   the stored snapshot, never the live file (test-enforced).
3. **Deterministic run-vs-run diff**: `flows diff <runA> <runB>` — pure
   comparison of node statuses, outputs, usage deltas. No model in the loop.
4. **Recovery audit**: on `--resume`, classify
   `{snapshot: frozen|live|missing, coverage: complete|partial}` → typed
   action `resume | repair | rebuild`. Fail closed on mismatch.

Deliberately excluded from vakyartha's version: subgraph reuse, approval
resume choreography — our flow model has no interactive mid-flow approvals
yet; revisit only if demanded.

**Tests**: adopt→run reproduces outputs on mock provider; diff of
identical runs is empty; snapshot-tamper detection fails closed.

## Phase F — Process adoption (no product code)

From vakyartha's engineering practice, adapted:

1. **Scenario harness** (`scripts/scenarios/`): drive the real binary end
   to end against the offline mock provider — exactly one stubbed
   boundary. Each scenario asserts universal invariants (abort preserves
   partial output; timeout-deny resolves nothing; breaker opens after N
   blind failures; compaction keeps recent turns verbatim; approval
   forwarding verdict vocabulary). An unmatched mock expectation is a hard
   failure — "a green pipeline that never ran" must be impossible.
   Consolidates what today lives in scattered integration tests.
2. **Two-file contract rule**: rules live in `AGENTS.md`; the *why*
   (citing shipped regressions) lives beside them; both change in the same
   commit. Superseded rationale gets deleted.
3. **Gate-to-subsystem table** in `AGENTS.md`: each subsystem maps to the
   exact command that proves it. Prefer property tests over curated
   fixtures for semantic gates.
4. **CI checks doc citations**: referenced `crates/…` paths in design
   docs must resolve; stale citations fail CI.

## Phase G (conditional) — Run-graph projection & flow surfaces

Landscape evidence (2026-08): flows-as-first-class-artifacts are winning —
n8n Agents ships define-once reuse across chat/schedules/workflow nodes,
and a dashboard ecosystem consumes coding agents' own transcripts
externally (plan strips, mermaid viewers, subagent trees). Generative-UI
agents independently validate the model-authors-semantics /
renderer-owns-presentation split (scope 4 below ≈ vakyartha's AnswerDraft).
Counter-evidence bounds the ambition: OpenAI deprecated Agent Builder — its
flagship visual canvas — eight months after launch (shutdown 2026-11-30),
migrating users to code-first SDKs. Conclusion: projection-as-data is the
durable layer; canvas-as-core is an anti-goal.

Scope, each gated on its trigger:

1. **Run-graph snapshot as data** — typed
   `{layers: [{node_id, status, started_at, duration_ms, error}]}`
   projected from existing flow-run ledgers + planner attempt audits;
   emitted as an SSE event during runs and served at
   `GET /flows/{run}/graph`. Follows invariant 4 (delta + snapshot);
   zero rendering opinions. *Trigger*: getting lost in a real flow run.
2. **TUI affordances** — one line per active layer
   (`L2/4 ▶ research-a · ✔ fetch · ✘ parse · skipped 2`) plus a
   completion/resume summary; planner prints its validated shape with the
   same layer printer before executing. Rides along with (1); reuses
   `flow check`'s printer.
3. **Flow surfaces beyond CLI** — server endpoints + desktop controls for
   flows, making our SSE consumable by our own UIs and third-party
   dashboards alike. *Trigger*: remote/desktop flow-control demand.
4. **Typed tool outputs → rendered tables/metrics** —
   `ToolOutput{content: String}` gains an optional typed payload;
   TUI/desktop render tables/metrics/charts natively. Model authors
   semantics, deterministic renderer owns presentation. *Trigger*:
   repeated structured-output demand.

Anti-goals: no interactive node canvas in core/TUI, no animation, no SVG
timelines in a terminal — Agent Builder's deprecation is the cited
evidence that authoring canvases rot faster than the data layer beneath
them. Rich views belong to desktop or external consumers of the snapshot
API.

## Phase H — Loop engineering (the long-horizon kernel)

Status: **core landed** — goal mode, audited completion (deterministic
`verify:` criteria + judge dispatches receipted as `Verify` work),
regression obligations, and reset-with-handoff are live behind
`[goal]` config / `exec --goal --criteria`. Honest deviations: planner
done-contracts (mechanism 5) land with the next flows touch; the judge
sees a transcript digest, not yet a filesystem/checkpoint diff —
checkpoint-delta auditing is the natural next slice.

Not a vakyartha import: sourced from the same public frontier evidence as
the North star section, and the answer to "better than any other harness."
Gap audit confirmed 2026-08-24: no goal mode, no independent auditor, no
task-state record outside the transcript, no reset-with-handoff, no
regression obligations. Every mechanism below builds on existing
strengths — stop gate, append-only ledgers, subagents, run-start
checkpoints, exit-code-verified evals.

1. **Goal mode** — a durable `goal` ledger entry
   `{objective, acceptance_criteria[], status}` surviving across turns
   and sessions. After every turn the loop evaluates goal progress,
   generalizing the existing stop gate machinery, and emits typed
   outcomes `execute | blocked | ask | done`; only `done` ends the run.
   Surfaced in the TUI status row and server events.
2. **Audited completion (MEA-lite)** — a `done` claim spawns a read-only
   verifier subagent that judges the workspace delta against the
   acceptance criteria using run-start checkpoints (before/after already
   captured). Completion is recorded from the audit receipt, never from
   self-report. Audit failure returns findings to the executor. The
   auditor holds read-only tools only — it can never mutate state to make
   its own report true.
3. **Regression obligations** — commands/tests proven green during a run
   become ledger-recorded obligations. The stop gate's verify stage
   re-runs them before any completion claim; a regression reopens the
   goal instead of ending the run.
4. **Reset-with-handoff** — on the still-over-after-compaction fail-closed
   branch (`17-context.md`), write a structured handoff entry
   `{objective_state, decisions[], open_items[], obligations[]}` and
   continue from a fresh segment seeded by it. Complements compaction,
   never replaces it; append-only safe (invariant 2). Anthropic found
   resets with structured handoffs beat compaction alone ("context
   anxiety").
5. **Done-contracts** — planner nodes carry acceptance criteria up front;
   flow verify nodes and goal-mode checks consume them. Vakyartha's
   co-planning idea lands here, now evidence-backed independently of
   vakyartha (Anthropic's sprint contracts).

Design discipline — **strippable scaffolding**: each mechanism is
config-gated and excluded from the minimal eval profile (our baseline
stays minimal per the DeepSeek-Harness lesson). As models improve,
Anthropic's practice applies: re-examine and delete what stops being
load-bearing rather than accumulating ceremony.

**Tests**: goal-mode e2e (mock provider): criteria unmet → loop
continues; met → audited `done`. Auditor cannot write (permission engine
denies); workspace tampered vs criteria → audit fails and findings reach
the executor. Obligation regression reopens the goal. Compaction-still-
over produces a handoff entry + fresh-segment continuation that completes
the objective. Planner contract validation rejects criterion-less nodes
while goal mode is active.

## Parking lot (post-adoption, demand-driven)

- Skill intent-discovery: embed turn intent against skill descriptions,
  promote above threshold, fall closed to name-match; usage telemetry
  (data already logged via FrozenContract skills list); graduation ladder
  remember → propose_skill → scheduled task → adopted flow.
- Supply-chain hardening for desktop releases: SBOM generation, signing/
  Mach-O verification script guarded by meta-tests.
- Capacity exhaustion as first-class Ask type (fold into Phase D initially).

## Sequencing

Demand status per the Confidence & falsification section: B, C, D, E are
demand-backed (attested incidents), not speculative; H is evidence-backed
by the public frontier convergence; A is their shared foundation. Order
follows the North star — long-horizon reliability first.

| Order | Phase | Depends on | Size |
|---|---|---|---|
| 1 | A receipts + ceiling | — | M |
| 2 | C context gate | A (events) | M |
| 3 | D FinOps | A (usage in receipts) | M |
| 4 | H loop engineering | A + C (receipts, context discipline) | L |
| 5 | B frozen ladder | A | L |
| 6 | E flows adopt/diff | A | M |
| conditional | G run-graph projection & flow surfaces | A; rides with E | S slices |
| ongoing | F process | none | S slices |

A first: everything else records into receipts. C and D can proceed in
parallel after A; H follows once receipts exist because audits and
obligations are ledger entries — it is the differentiating phase. B is
demand-backed resilience plumbing, necessary but not differentiating; its
pure-function core (admission set + versioned ordering) should land
behind tests before any UI/config surface.

## Open questions

- Receipt granularity: one entry per step (chosen) vs per attempt?
  Per-step keeps the ledger compact; per-attempt aids forensics —
  compromise chosen: attempts array inside the step-level receipt.
- Evidence ledger retention window (default proposal: 30 days TTL).
- Whether `max_day_usd` belongs to finops config or privileged
  `[gateway]` section (proposal: both readable, only `[gateway]` writable
  by user, matching the privileged-config pattern).
