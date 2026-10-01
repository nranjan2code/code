# Reliable work execution — implementation plan

Status: in progress; E0 audit is incomplete and E2 has a first recovery slice.
Date: 2026-09-30.
Reviewed source: `0bfcaa39e341cef46360513e9d6f037c0f5f2493`.
Design review: [reliable-work-execution-review.md](reliable-work-execution-review.md).

## Objective and acceptance

Complete general-purpose work at the existing evidence and quality standard,
using fewer unnecessary provider attempts and smaller justified requests, while
preserving conversation recall, artifacts and resumable progress.

The acceptance gate is joint: quality, correctness, memory, latency and resource
use. Reducing tokens by omitting a required fact, lowering evidence requirements,
abandoning work early, or mislabelling a partial result is a regression.

This is an extension of designs 15, 42, 47, 50, 52 and 68. It uses existing work
ownership and storage, and does not depend on starting M1 of the data refactor.

## Decisions to settle in stage E0

1. Root ownership: use the existing admitted request/episode and its source ledger
   identity; distinguish it from the session and from a durable commitment. Do
   not introduce the future data-architecture `Run` model in this change.
2. Policy: finite outer resource limits come from operator/user policy and current
   admission. Calibrate defaults against measured workloads. Learned estimates
   may allocate within those limits, never silently raise them.
3. Identity: define which provider observations prove shared account capacity,
   and how region, model group and endpoint-specific limits narrow it. Unknown
   sharing stays explicit. Reuse existing credential fingerprint/identity seams.
4. Recovery: distinguish temporary throttling, request too large for a refill
   window, exhausted quota, overload and uncertain settlement. Agree how each
   enters the existing error, ladder, endurance and outcome contracts.
5. Scope: account coordination is shared across all Core instances in a service
   process. Independent processes must not claim globally enforced limits without
   a shared authority. Audit current gateway/bus ownership before choosing any
   additional cross-process mechanism.
6. Resumption: rederive spent resources, completed evidence and outstanding work
   from existing durable ledgers. Unknown in-flight settlements stay uncertain;
   effects require reconciliation before retry.

## E0 — audit, baseline and contract

**Work**

- Match the deployed version/source to the current tree before drawing a causal
  conclusion about the exact implementation.
- Inventory provider calls for ordinary execution, intent resolution, planning,
  verification, compaction, reflection, workers, flows, evals, gateway and desktop.
  Include background discovery/probing: their costs retain their own ownership
  and still participate in shared provider capacity admission.
- Audit rate-limit normalization, headers, error codes, cache-tier accounting,
  request estimates, token usage, tool durations, approval waits and final outcome.
- Trace why this request began with a large working set and how it grew. Record
  metadata and causal references; keep content out of logs.
- Create a deterministic, content-free fixture that reproduces the incident's
  TPM constraints, context growth and retry pattern. It must not reproduce keys,
  private account identifiers or unrelated conversation text.
- Specify ownership, reservation, settlement, uncertainty, deadline and completion
  contracts. Map them onto existing structs and consumers before adding types.
- Establish a diverse baseline: conversation, factual lookup, writing, document
  creation, data analysis, code changes, verification and operation. Include
  ambiguous requests, long-history references and mixed deliverables.

**Exit gate**: the consumer inventory has an owner and accounting path for every
dispatch; all E0 decisions are explicit; the fixture distinguishes rate waiting,
tool waiting and model work. Baseline measures joint quality and resource use.

## E1 — shared accounting and exact settlement

**Work**

- Extend the existing spend/admission seam with a root work resource account.
  Children receive a share of the parent allowance; parent and child execution
  consume one aggregate account, with atomic reservations under concurrency.
- Track provider dispatches, measured tokens by tier, estimated/settled money
  when priced, elapsed work time and recoveries. Retain step receipts and attach
  them to their root ownership. Do not count receipt usage and assistant-message
  usage twice.
- Give each admission reservation a stable identity. Settle or release that exact
  reservation once on success, definitive rejection, cancellation or uncertainty.
  Unknown charge is recorded separately from known usage and estimated exposure.
- Preserve operator day and session/commitment caps as independent enclosing
  limits. Dollar pricing may be unknown; token/time/dispatch admission must still
  work, and unknown dollars must stay unknown.
- Restore accounting on restart from durable evidence. No new message, worker,
  fallback, compaction or approval can silently reset accumulated consumption.
- Remove superseded reservation/accounting paths in the same change.

**Likely files**: `vak-core/src/finops.rs`, Core spend-gate ownership,
`vak-agent` spend trait and dispatch paths, `vak-llm/src/work.rs`, session receipt
types/projections, worker/flow/planner/eval consumers identified in E0.

**Exit gate**: concurrent children cannot overspend a shared allowance; every
reservation has exactly one terminal disposition; failures with missing usage
remain explicit; restart/retry cannot reset consumption; all inventoried calls
participate or have a documented, separately bounded owner.

## E2 — coordinate provider capacity and recovery

Depends on E1.

**Work**

- Normalize published provider rate-limit observations and retry timing into
  provider-independent typed hints in VAK-owned HTTP adapters. Preserve error
  codes; distinguish exhausted quota from retryable throttling. Handle supported
  duration/date forms without an English-message keyword policy in the agent loop.
- Add shared account capacity admission before dispatch. Combine request demand,
  observed remaining capacity/reset time and uncertain estimates. Queue work
  cancelably when capacity will refill; do not issue paid model calls to poll it.
- Carry cooldown/admission state across confirmed aliases of one account.
  Walking the admitted ladder must not evade a shared capacity limit. An eligible
  independent leg still follows existing route and model-identity policy.
- If request demand exceeds the known refill-window capacity, waiting alone is
  not recovery. Return a typed decision for a logged context replan or an eligible
  alternate; never replay the same oversized request indefinitely.
- Preserve the distinction between informed transience and blind failures in
  invariant 7. A provider-capacity scheduler is not a replacement circuit breaker.
- Derive attempt watchdogs from admitted remaining time and observed latency,
  within policy bounds; nested retries cannot reset the root deadline. Separate
  service work, provider queueing and human approval waits in accounting.

**Exit gate**: a modeled TPM window results in coordinated waiting and useful
completion rather than repeated rejection; parallel sessions and protocol aliases
do not race one shared capacity bucket; unknown-limit behavior remains bounded;
cancel/revocation interrupts waits; quota exhaustion is not transient endurance.

## E3 — context justified by work, with intact recall

Can follow E1 independently of E2; both must land before E4's combined gate.

**Work**

- Extend `vak-context` planning to use the request's required references,
  constraints and evidence as well as the capacity profile. Model capacity is an
  upper bound, not a target to fill.
- Prefer the smallest context that preserves the required dependencies, with
  uncertainty-driven recall when relevance is unclear. Measure coverage of named
  references and constraints; no domain-specific source-count or word-count rule.
- Audit tool-result ingestion into the current evidence/digest path. Preserve full
  material, provenance and expansion handles, using registered structured or
  content-aware representations rather than raw truncation.
- Preserve whole-turn fidelity, within-turn frozen request planning and stable
  cache prefixes. Any explicit replan/handoff is logged and reproducible; do not
  mutate an exact retry invisibly. A redesign of open-turn representation requires
  an explicit doc 68 decision and its own no-loss gate.
- Keep durable memory separate from the model's temporary working set. References
  excluded from a request remain recallable through existing trash-aware paths.
  Budget decisions never delete history, evidence or remembered preferences.

**Likely files**: `vak-context/src/planner.rs`, capacity/assembly, session turn
index and evidence/digest/recall paths, Core context posture, agent tool-result
recording and explicit replan transitions.

**Exit gate**: exact reference/constraint recall is retained in long histories;
unrelated historical material does not fill a large model window; evidence
expansion restores the original material; no-cut and request-reconstruction
invariants pass; request reduction does not lower outcome quality.

## E4 — progress, completion reserve and truthful recovery

Depends on E1–E3.

**Work**

- Consume existing `OutcomeSpec`, requirements and scoped evidence in continuation
  decisions. Progress can be a supported claim, artifact revision, check result,
  resolved dependency or sufficient conversational response. Tool success alone
  does not prove semantic completion, and prose length is not progress.
- Make expensive continuations state the unmet requirement and intended evidence
  change. Repeated output/failed admission does not refresh progress accounting.
  Runtime evidence validates the proposal; the model cannot grant itself budget.
- Reserve an adaptive allocation for required synthesis, verification and delivery
  within the root budget before allocating more exploration. Small turns need no
  mandatory planner or verifier model call.
- Use measured estimates to allocate additional work within admitted policy.
  Begin with observable evidence transitions and conservative uncertainty; evaluate
  more speculative predictions of value before they can control termination.
- Preserve quality requirements across routes and recovery. On exhausted resources,
  deliver supported partial findings if useful, mark unmet requirements explicitly,
  preserve the obligation and progress, and reserve completion for satisfied work.
- Resume only through existing user/commitment/task authority, rechecking policy,
  permissions, freshness and uncertain effects. Reuse completed work rather than
  repeat it. Introduce no automatic new recurring job.

**Exit gate**: stalled work changes strategy or holds without spending indefinitely;
valid creative/conversational answers need no tool proof; complete verdicts retain
all must-requirements; partial delivery is useful and truthful; resumption preserves
evidence and does not duplicate effects or create unauthorized background work.

## E5 — shared surfaces, evaluation and rollout

Depends on E4.

**Work**

- Project one consistent outcome/wait state to CLI, terminal, gateway, desktop and
  browser. Plain language explains a rate-limit wait or held work and retained
  progress. Technical disclosure shows recorded tokens, estimates, missing usage,
  root/child attribution, attempt counts and reasons without exposing secrets.
- Add root usage to existing snapshots and projections. Live reconnect/resync is
  observational and never redispatches work.
- Run deterministic fault and concurrency coverage plus blinded quality evaluation
  across the E0 workload set. Include unrelated-history growth, cache misses,
  small/large contexts, missing pricing, provider outages, TPM throttling,
  malformed results, parallel children, revocation, cancellation and restart.
- Set release gates against the measured baseline before rollout: retained quality
  and recall, lower avoidable attempts/context, correct completion semantics and
  accountable latency. Do not accept a token reduction alone as improvement.
- Keep production observations passive during evaluation. Live paid runs require
  an explicit spend allowance. Deploy only through the normal release and AWS
  managed-install process after the implementation is reviewed.

**Exit gate**: general workloads meet the joint gate and all execution/surface
consumers are accounted for. Update affected design docs, AGENTS.md invariants,
release notes and plan status; remove superseded code in the same changes.

## Change boundaries and verification

Each stage should be a reviewable change with its named acceptance tests and doc
updates. Keep existing policy authority, sandbox/broker boundaries, route contracts
and append-only ledgers. New runtime durable files, if unavoidable after E0,
must use canonical paths and be declared in `vak_core::state::REGISTRY`.

Before implementation, refresh file locations and the dispatch consumer inventory.
Before any commit, run the repository-required formatting, lint, workspace tests,
version and documentation-path checks. Frontend changes also require matching
bundles and the applicable visual acceptance process. This planning change makes
no AWS configuration, scheduling, version or deployment changes.

## Progress log

- 2026-09-30: Added a process-shared, cancelable cooldown keyed by opaque
  provider-capacity identity; OpenAI Chat Completions and Responses adapters
  share the key when configured with the same endpoint and credential. The
  Responses adapter now carries structured retry timing from streamed failures
  and the response header. The agent waits before spend admission and dispatch,
  shares typed rate-limit outcomes across aliased routes, and clears learned
  cooldown only after a successful probe. This is a bounded E2 foundation, not
  the E2 exit gate: it does not yet coordinate independent processes, model the
  provider's full refill capacity, or cover every non-agent provider consumer.
- Existing code inspection confirmed that long tool results already retain full
  evidence and are windowed in the open turn; closed turns project schema-aware
  digests with recall handles. No context truncation change is justified by the
  incident evidence so far. E0 still needs deployed-source matching, the full
  provider-consumer inventory, the content-free fixture and diverse baseline.
