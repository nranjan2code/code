# Intent engine and commitment kernel: universal-platform audit

Status: audit complete; findings below are open, not implementation claims.

Date: 2026-10-02. Reviewed HEAD: `8cd42679dcb7e193f38af2c7b37f98efef96f1bf`
(workspace version 5.3.2), plus the working tree. Other work was changing the
shared checkout during this audit; no unrelated changes were reverted or edited.

## Assessment

The pure intent layer has a sound safety direction: fallible interpretation
cannot grant permission, uncertainty preserves discoverability, and authority
limits compose restrictively. The commitment ledger also refuses unsupported
fulfilled closures at its append boundary. Keep those properties.

The implementation does **not yet establish a complete universal commitment
runtime**. Interpretation, per-turn outcome evaluation, managed-work evidence,
and durable commitments are separate mechanisms whose connections are incomplete.
The result can be both too optimistic (an unrelated sentence counts as a complete
multi-result outcome) and too pessimistic (real evidence never advances the
commitment). Durable wakeups do not dispatch continuation work, and lifetime
economics do not constrain normal episode admission.

This is not solved by adding more coding keywords or more domain-specific core
branches. The central missing contract is a stable result/requirement identity
carried from interpretation through execution evidence, continuation and closure.

## Method and limits

Reviewed the authority and runtime contracts in `AGENTS.md`, the current status
and relevant sections of design documents 47-commitment-kernel,
52-outcome-directed-runtime, 64-agent-owned-platform,
65-universal-adaptive-platform and 24-agent-security. Traced the resolver,
strand lineage, engagement and envelopes, Core admission/settlement, commitment
ledger/projector, upkeep, CLI attestation and server portfolio endpoints. The
older intent scenario audit was treated as historical evidence.

Executed:

- `cargo test -p vak-intent -p vak-commit --quiet`: 331 tests passed.
- `cargo test -p vak-core --test intent_projection --test intent_wiring --quiet`:
  13 tests passed.
- `cargo test -p vak-session --test intent_entries --quiet`: 9 tests passed.
- Five focused audit probes: passed, meaning they reproduced the defects they
  assert. These are demonstrations of current behavior, not acceptance tests
  claiming the behavior is desirable.

One combined runner attempt passed the four commit/intent probes, including
the added `CompletionVerdict::Complete` assertion, but rebuilding the core probe
was blocked by a concurrent unrelated edit: `vak-agent/src/lib.rs:2106` referred
to a missing `vak_session::CONTEXT_PLAN_LABEL`. After that concurrent work added
the export, the final runner replay passed all five probes. `results.txt`
preserves the interrupted attempt; `results-replay.txt` records the successful
final replay. The unrelated code was not changed by this audit.

Reproduction sources, a collision-checked runner and captured output are in
[`docs/research/intent-commitment-audit-2026-10-02`](../research/intent-commitment-audit-2026-10-02/).
Run `bash docs/research/intent-commitment-audit-2026-10-02/run.sh` from the repository.
It temporarily installs two integration-test files and removes them on exit.

No production behavior changed. No model/provider calls, external effects,
browser acceptance run, long-duration service recovery run, full-workspace test
suite or full security audit was performed. Passing algebra/projection fixtures
does not measure real-world intent accuracy or prove end-to-end task completion.

## Findings

### F1 — P1: lifetime economics are not enforced at episode admission

Evidence: `crates/vak-core/src/commitments.rs:159` (`plan_episodes`),
`crates/vak-core/src/lib.rs:6765` (turn spend gate),
`crates/vak-intent/src/resolve.rs:886` (`apply_envelopes`),
`crates/vak-core/src/lib.rs:7844` (spend attribution).

`plan_episodes` selects any nonterminal commitment on the thread without
checking its remaining lifetime budget, expiry or stall limit. Portfolio ranking
withholds such commitments, but admission does not consume that decision.
The actual run gate narrows against `engagement.limits.spend_ceiling_usd`; the
commitment's `economics.lifetime_budget_usd - spend_usd` never feeds it.
An envelope contributes its full spend limit each turn, rather than its remaining
lifetime allowance. The per-call envelope check reads settled spend, so it cannot
stop an in-progress turn at the residual lifetime allowance either. General
FinOps caps remain separate and can still stop work.

Reproduced: open a commitment with a $1 lifetime cap, settle $2, then plan and
begin another episode on the same thread. A new episode starts successfully.
For compound turns, all spend goes to the first commitment; subsequent
commitments accumulate zero attributed spend.

Fix direction: one commitment admission decision must gate the turn and meet
each relevant remaining allowance into dispatch admission. Define conservative
shared-turn cost attribution. Cross-process reservations/fencing belong with the
approved data architecture, not a second accounting mechanism.

Acceptance: sequential episodes cannot exceed a lifetime cap silently; a $10
envelope with $9 spent admits at most the remaining $1; compound work cannot hide
spend by moving an obligation to a later strand. Test unknown pricing explicitly.

### F2 — P1: multiple requested results collapse into one easily satisfied outcome

Evidence: `crates/vak-intent/src/outcome.rs:1048` and `:1092` build the contract;
`:926` satisfies a deliverable from nonempty response text; `:575` computes
completion. Core consumes this evaluator at `crates/vak-core/src/lib.rs:7709`.

`OutcomeSpec::from_intent` preserves the set of acts and the composite stop
profile, but creates only the composite reading's single `deliverable-1` with
target `primary`. Strand identities, individual deliverables and dependencies
do not become separate requirements. A nonempty response that does not begin
with one of a few refusal prefixes marks that deliverable met.

Reproduced: “Draft a letter, then translate it into French, then summarize the
budget” produces three strands but one requirement. “The weather is pleasant.”
satisfies it; `evaluate_completion(Produced, ...)` returns `Complete`.
This is a deterministic evaluator reproduction, not a claim that a live model
was observed emitting that answer. Other stop checks do not supply the missing
per-result identities to this evaluator.

Fix direction: preserve a result ID and requirements per meaningful strand,
including dependency edges; evaluate the actual linked artifact or response
segment. Keep `Produced` separate from `Satisfied`; if semantic completion is
not established, report unknown rather than using text presence as proof.

Acceptance: a letter alone leaves translation and budget summary outstanding;
an unrelated answer completes none; individual partial results retain their own
status and evidence across a continuation and across presentation surfaces.

### F3 — P1: completed execution does not advance durable satisfaction

Evidence: `crates/vak-core/src/commitments.rs:56` (`seed_criteria`),
`crates/vak-core/src/lib.rs:7848` (episode settlement),
`crates/vak-commit/src/ledger.rs:29` (event vocabulary),
`crates/vak-core/src/commitments.rs:796` (`WorkspaceEvaluator`),
`crates/vak/src/intent.rs:577` (human attestation).

Automatically opened commitments have either no criteria or one required
`Semantic` placeholder. Normal turn settlement always calls `classify` with an
empty `criteria_moved`. It does not evaluate commitment criteria or attach the
turn's result/evidence to them. The event vocabulary has no criterion-add/revise
event through which a planner could replace the placeholder after admission.
The available workspace evaluator checks existence/content; shell, tool, flow
and external criteria return unknown there. Its maintenance caller only evaluates
a suspended predicate. Human CLI attestation is a real alternative path, but
does not establish automatic evidence integration.

Consequences: cited research can remain permanently outstanding even when the
turn produced checked sources; verified execution does not become observed
commitment progress. Outcome evidence requirements explicitly wait for linked
criterion evidence, but Core's outcome evaluator does not read that ledger.
There is no production `Readmitted` event writer either, despite the documented
re-admission/drift contract.

Fix direction: connect the existing work/evidence vocabulary to the same
requirement identities at settlement, with append-only requirement revisions.
Keep observed checks, semantic judgement and human attestation distinct. Do not
fix this by letting the model mark its own criteria passed.

Acceptance: runtime-observed work advances exactly its linked criterion; cited
research retains claim/source support; a changed requirement invalidates only
the evidence it supersedes; CLI attestation and runtime evidence project the
same authoritative standing.

### F4 — P1: a commitment waking up does not cause its continuation to run

Evidence: `crates/vak-core/src/commitments.rs:633` (`maintain`),
`crates/vak-server/src/lib.rs:20583` (maintenance consumer),
`crates/vak-commit/src/portfolio.rs:198` (`next`).

Upkeep appends `Resumed` for due timestamps, successful predicates or fulfilled
dependencies. The server only prints resumed/satisfied IDs. It neither admits a
continuation nor creates/dispatches a `TaskDef`. Portfolio `next` has no production
dispatch caller. `Schedule { cron, at: None }` and `External` have no handled wake
branch. `QuestionAnswered` has a projector and tests, but no production writer
in the inspected tree to connect the deferred question back to execution.

An existing independently configured `TaskDef` can run through its own scheduler;
this finding is specifically about the claimed commitment-to-continuation bridge.
The suspension table in shipped doc 47 overstates those connections.

Fix direction: explicitly bind a durable obligation to the existing scheduler,
Agent/conversation, audience, resumable request and current admission authority.
Use the planned shared trigger/effect/fencing work for reliable recovery. Until
then expose the limited state honestly rather than presenting a state transition
as resumed execution. No new scheduler is needed.

Acceptance: a due obligation dispatches once and records its run; a crash between
wake and dispatch recovers; an unanswerable or unsupported wake remains visibly
held; no model is dispatched while an unchanged predicate is false.

### F5 — P1: a shared action verb can attach unrelated work to an old commitment

Evidence: `crates/vak-intent/src/strand.rs:367` (`keywords`),
`crates/vak-intent/src/resolve.rs:474` (`lineage_for`),
`crates/vak-core/src/intent.rs:176` (`open_threads`),
`crates/vak-core/src/commitments.rs:159` (binding and envelope selection).

Lineage accepts any keyword overlap among same-act threads, and action verbs
remain keywords. `ThreadFact.domains` does not participate in this match.
Reproduced with distinct turn IDs: “Write a poem every morning” followed by
“Write a resignation letter” yields `Continues` on the poem's thread solely
through “write”. An existing commitment on that thread is therefore selected,
including its live envelope, economics and stale objective.

This can contaminate obligations and apply a commitment-specific preauthorization
to unrelated work within the envelope's tool/path scope. It is not a demonstrated
permission-engine or sandbox bypass; those checks remain independently active.
Conversely, another session's durable threads are not imported by `open_threads`,
so reading a portfolio item does not itself provide an explicit continuation
binding. Closed work is not filtered there either; only replacement history and
the most recent twelve thread IDs are considered.

Fix direction: treat lexical similarity as a suggestion, not sufficient identity
for attaching durable work or its authority. Bind continuation through explicit
conversation/artifact/commitment context; require stronger reference evidence
when inferring it and leave ambiguous attachment unresolved.

Acceptance: unrelated same-verb requests remain separate; explicit continuation
in another authorized conversation reaches the selected commitment; completed
threads do not reappear as active obligations.

### F6 — P2: starting an episode erases unresolved suspension state

Evidence: `crates/vak-commit/src/ledger.rs:383` (`EpisodeStarted`),
`crates/vak-core/src/commitments.rs:327` (orphan recovery),
`crates/vak-core/src/intent.rs:367` (deferred approval).

`EpisodeStarted` unconditionally sets active and clears the suspension and
blocker. Admission accepts suspended/blocked commitments on a matching thread.
Reproduced: suspend on an unanswered `Human` question, then begin a matching
episode; the question disappears from the current state without an answer event.
Orphan recovery records “review before retry” as blocked and immediately starts
another episode, clearing that blocker. Timeout `Reassign` also merely resumes;
it does not re-address the question to its configured recipient.

Starting work is not evidence that a required decision was answered. The tool
approval boundary can still deny a later effect, so this finding does not assert
that suspension loss alone authorizes an effect.

Fix direction: preserve blockers until a typed resolution authorizes their
transition. A new message can add useful information without silently resolving
the question. Recovery should retain uncertain effects and required review.

Acceptance: unrelated steering preserves the question; an authorized resolution
closes exactly that question; recovering an unfinished episode does not discard
its uncertainty or implicitly retry effects.

### F7 — P2: universal requests receive materially different contracts by language and vocabulary

Evidence: `crates/vak-intent/src/signals.rs:229` (verbs), `:849` (horizon phrases),
`:1114` (ASCII tokenization), `crates/vak-intent/src/resolve.rs:315` (weak fallback).

Measured with the default resolver:

| Request | Reading | Durable commitment posture |
|---|---|---|
| Write a poem every morning | author / durable, confidence .85 | Yes |
| हर सुबह एक कविता लिखो | answer / turn, confidence 0 | No |
| Escribe un poema cada mañana | answer / turn, confidence 0 | No |
| 每天早上写一首诗 | answer / turn, confidence 0 | No |
| Remind me to call Mum tomorrow | answer / turn, confidence .5 | No |
| Book a flight to Delhi | answer / turn, confidence 0 | No |
| Transfer 500 dollars to Alice | answer / turn, confidence 0 | No |
| Cancel my subscription | answer / turn, confidence 0 | No |

The fallback keeps tools recoverable and can be enriched by optional classifier
escalation; it is not a claim that the serving model cannot understand these
requests. The gap is that default runtime durability, completion and risk
posture differ. Extending a verb list alone cannot make this a universal semantic
contract. “Tomorrow” also needs a one-shot trigger interpretation, not just a
recurrence-word match.

Fix direction: retain cheap conservative inference, but represent unknown axes
as unknown and allow bounded structured interpretation against generic result,
effect and temporal schemas. Actual effect metadata must govern risk at dispatch.
Provide domain-specific evaluators as extensions. Do not use confidence in an
English lexical match as the only path to durable responsibility.

Acceptance: multilingual and paraphrase pairs preserve the same obligations;
one-shot and recurring requests both produce explicit schedulability decisions;
missing integrations produce an honest capability gap, not fictional completion.

### F8 — P2: episode accounting accepts duplicate, nonexistent and negative settlements

Evidence: `crates/vak-commit/src/ledger.rs:203` (`append`), `:401` (`EpisodeEnded`).

Append validates existence, terminal state and fulfilled closure, but not legal
episode transitions. Projection adds spend and changes the stall counter even
when the episode ID does not exist or was already ended. There is no event-ID
deduplication or nonnegative finite spend validation at this boundary.

Reproduced: append the exact same $2 `EpisodeEnded` twice; spend becomes $4.
Append a -$3 settlement for a nonexistent episode; spend becomes $1. This is an
internal ledger API reproduction, not a demonstrated model-accessible write API.
It matters for recovery, duplicate delivery and any future runtime integration.

Fix direction: enforce valid episode state and nonnegative finite accounting
under the existing lock, and define idempotency using stable settlement identity.
Do not allow a repeated callback to change lifetime economics or progress twice.

Acceptance: duplicate settlement is rejected or idempotent; unknown episode and
negative/nonfinite spend are refused; concurrency cannot create two terminal
settlements for one episode.

## Recommended order and platform boundary

1. Fix admission and identity first: F1, F5, F6 and F8. These determine what work
   runs under which obligation, limits and unresolved decisions.
2. Unify result/requirement identity and evidence settlement: F2 and F3. Preserve
   the evidence lattice and append-only records; remove competing completion
   interpretations as the canonical path replaces them.
3. Wire durable continuation through `TaskDef` and the already-approved shared
   trigger/run/effect architecture: F4. This audit does not authorize starting
   a later data-architecture milestone.
4. Validate universality with F7's language/domain matrix and real workloads.
   Measure omitted obligations, false completion, incorrect continuation,
   correction burden, latency and cost, not just classifier agreement.

The core should own generic obligations, results, dependency relationships,
authority, time/trigger references, evidence provenance and lifecycle. Tools and
extensions should supply domain-specific effects and evidence evaluators. A
document, a research claim, a calendar change and a repository patch should share
the lifecycle without being forced into file existence or shell-exit criteria.

Keep the explicit control-source matrix, narrowing lattice, logged model-visible
intent, audience-filtered portfolio tool and append-time closure refusal. The
documented FullAccess approval exception is an existing product decision, not a
newly discovered regression in this audit.

Before calling doc 47 fully shipped, reconcile its budget, wake, readmission and
satisfaction claims with these runtime connections and their exit tests. None
of the passing suites above substitutes for that acceptance work.
