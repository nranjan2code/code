# Architecture, answers, surfaces and dynamic cards — consolidated review

Status: historical — code review and isolated probes from the 2026-10-18
session, consolidated across all review passes. This is evidence, not a
claim that the proposed repairs have shipped.

> **Historical record** — initial architecture and presentation checks ran
> against 8.8.0; the dynamic-card follow-up ran against 8.9.0. At consolidation
> HEAD was `5c218fc9b` (Release 8.9.0). Selected source paths were checked
> again; the entire fault suite was not rerun on that release.

## Scope and how to read this record

The session covered four overlapping scopes: the system architecture;
answer planning, presentation and projection; chat, right panel, canvas and
saved pages; and dynamic/interlinked cards with ability-backed data. This
record retains distinct observations from all of them and deduplicates
shared causes. It is not limited to the last dynamic-card response.

Source code and executed checks supplied the findings. Design documents
were not treated as proof of implementation. Earlier consultation supplied
some candidate findings; included findings were independently examined.
The maintainer subsequently requested no further Astra consultation, and
none is part of the continuation plan.

Evidence labels: **probe** means an isolated executable demonstrated the
specified primitive behavior; **source** means a traced code path, without
a completed end-to-end failure reproduction; **decision** means a boundary
or product choice needing an explicit resolution. P1 is a correctness,
security or durability priority; P2 is a significant consistency,
maintainability or performance priority. These are review priorities, not
incident severity ratings. A passing primitive test does not establish
browser or deployed-system correctness.

The [implementation plan](../plans/architecture-and-presentation-hardening-plan.md)
owns stages and exit criteria. IDs below remain stable when work closes.

## Strengths to preserve

- The encrypted content-addressed substrate, scoped grants, append-only
  ledgers, Documents, versioned artifacts and catalog provide reusable
  primitives. Repairs should strengthen these, not create parallel stores.
- Tools and untrusted document parsers cross worker boundaries. Permission
  changes revoke active capability before reporting the new mode.
- Runs, triggers, cursors and effects distinguish work, admission, dispatch
  and uncertain outcomes. Durable outbound work should retain that model.
- Model plans and runtime receipts are separate; the correction ladder
  can inspect actual calls and deliverables instead of trusting prose.
- Shared client projection, adaptive fallback and declarative card forms
  are a useful foundation across browser and desktop.
- Dynamic controls, expressions, linked consumers and ability-backed
  tables/charts actually exist and have passing checks. They are not merely
  aspirational features. The problems below concern fidelity and lifecycle.
- Content-free telemetry and narrow capability admission are valuable
  constraints on any decomposition of the core.

## System architecture findings

### A01 — authoritative reads can silently retain stale decisions (P1; probe/source)

`crates/vak-session/src/tail.rs` returns the previous position on segment,
key or frame-read failure. `rollup.rs::read_at` returns state without a
fallible catch-up result. `crates/vak-core/src/grants.rs` can therefore keep
a cached grant when the segment containing its revocation is unreadable;
the fold's unreadable flag never sees that segment. An isolated probe
cached a grant, sealed its segment, appended a revocation to the next
segment, corrupted that segment and observed authorization still succeed.

`chain.rs` also has readers that skip unreadable segments/malformed JSON.
Effects and budget/accounting consumers must be audited for the same
distinction: an authoritative decision cannot interpret an unknown suffix
as no new records. Best-effort browsing and strict decision reads need
explicitly different contracts. Saving a rollup and reading its source
have different failure consequences; do not collapse them.

### A02 — global writer fencing and ref mutation are separate operations (P1; probe)

`crates/vak-storage/src/store.rs::Meta` checks the global epoch before
calling the ref store. The SQLite ref transaction in `refs.rs` checks the
ref's epoch, without atomically checking the global epoch in that same
transaction. A deterministic wrapper paused after epoch validation, allowed
restore to advance the epoch, then resumed the old writer: the write
succeeded at epoch zero after the global epoch became one. This exercised
the copied production Meta algorithm, not a live backup restore.

The guarantee must hold across processes and cover CAS, remove and their
commit bookkeeping. A process-local mutex does not repair this boundary.

### A03 — causal assistant journaling is optional before execution (P1; source)

`crates/vak-agent/src/lib.rs::append_assistant` converts an append failure
to an optional entry ID. The turn can continue toward tool execution after
that failure. Decision recording also has best-effort paths, whereas tool
result recording is stricter. Mandatory model-visible input and causal
tool-call records must succeed before work depends on them. Optional
diagnostic telemetry may remain best effort; it is a different contract.

### A04 — child-run construction drops parent services and policy (P1; source)

`crates/vak-core/src/lib.rs` supplies artifacts, protected paths, file-note,
step and history services to main runs. `crates/vak-agent/src/task.rs`
builds child `AgentConfig` from defaults and selectively copies dependencies;
`TaskDeps` does not represent all those services. Delegation can therefore
have different artifact accounting and protection behavior. Establish a
shared production run-construction contract with explicit narrowing for a
child, rather than duplicating a growing field list. Audit scheduled and
flow entry points as well; their equivalence was not established here.

### A05 — an abandoned half-open circuit probe blocks future probes (P1; probe/source)

`crates/vak-agent/src/circuit.rs` reserves a probe with mutable flags.
Dropping the caller without recording success/failure leaves it occupied.
The primitive probe demonstrated permanent refusal after abandonment;
source tracing found cancellation/budget exits after admission. Give
admission an owned permit whose neutral drop releases the reservation,
without counting a user abort as a provider failure.

### A06 — compaction error reporting can reacquire the session mutex (P1; source)

In `crates/vak-agent/src/lib.rs`, a compaction append-error path holds the
session lock and awaits `record_activity`, which takes that lock again.
This is a reachable lock-order risk, not a completed injected deadlock
test. Make journal ownership explicit and release the guard before any
helper that can acquire it.

### A07 — unknown tool effects are scheduled as nonconflicting (P2; source)

`crates/vak-tools/src/lib.rs::ResourceClaims::conflicts` treats unclaimed
claims permissively. MCP tools can report workspace-changing behavior
without overriding those claims, and the agent scheduler puts unclaimed
work into an early wave. Distinguish an unknown effect set from a proven
read-only/pure tool. A conservative default must respect exclusive work.
The broad Tool trait also spreads related execution facts across many
facets; a coherent execution descriptor can reduce drift while keeping
optional presentation capabilities separate.

### A08 — Core owns too many lifetimes and composes turns centrally (P2; source/design)

`crates/vak-core/src/lib.rs::CoreInner` combines process-wide clients and
discovery, storage, Agent/workspace state, worker orchestration, accounting,
mail, voice and UI configuration. `run_turn_recorded` centrally assembles
many of these. Preserve the modular monolith, but distinguish process
services, Agent/workspace services, admitted run policy and turn execution.
File splitting alone would leave ownership and construction problems intact.
This is an architectural recommendation, not a demonstrated runtime failure.

### A09 — hot decisions and endpoints replay growing chains (P2; source)

`crates/vak-session/src/effects.rs::get/list` reconstruct and sort history;
`crates/vak-server/src/effects.rs` applies limits after reconstruction.
Some synchronous work runs on async request paths. Budget/accounting
readers also warrant strictness and growth review. Use existing rollup/ref
primitives for decisions, bounded blocking work where needed, indexed
pending work and real pagination. No latency benchmark was collected, so
this is a scaling concern, not a measured performance regression.

### A10 — catalog confidentiality differs from encrypted primary data (P1 decision)

`crates/vak-catalog/src/lib.rs` stores searchable content in an ordinary
SQLite catalog. Its erasure path already uses secure deletion, vacuuming
and WAL truncation; those mechanisms do not encrypt a live catalog.
Define the promised at-rest confidentiality boundary explicitly and test
derived data against it. Encryption/keying or a narrower documented promise
is a design decision; the review did not authorize a new storage system.

### A11 — wire contracts and terminal outcomes drift across languages (P2; source)

Rust client events and TypeScript types are maintained separately. Some
terminal classification relies on summary strings such as `aborted` and
`max_turns`. Generate or mechanically check the wire contract and send
typed terminal outcomes from their source. Reconnect, replay, partial abort
and protocol-version behavior belong to that contract.

### A12 — releasing a blob can revoke another blob's shared chunk (P2; probe)

Production `crates/vak-storage/src/blobs.rs` with MemoryStore: store
`AAAABBBB` and `AAAACCCC` in four-byte chunks under one scope; release the
first; reading the second fails with `NoGrant` for the shared `AAAA` chunk.
Active use of this blob API by the shipped product was not established.
First establish reachability; repair shared-root/grant ownership before
using it on a product path. A standalone grant counter is not automatically
equivalent to the existing lifecycle/root model.

## Answer pipeline and surface findings

### P01 — card scalar conversion changes source values (P1; probe)

`crates/vak-core/src/presentation_tools.rs::number_or_text` strips commas
and parses strings through floating point. The exact function produced
`00123 → 123`, `1,5 → 15`, and `9007199254740993 → 9007199254740992.0`.
Tables infer numeric columns using this conversion, including ability data.
Preserve lexical values and require explicit numeric/locale semantics;
formatting cannot silently rewrite identifiers or precision.

### P02 — several layers suppress answer prose independently (P1; source)

`crates/vak-server/src/projection.rs` can label an assistant response after
an earlier answered plan check as a runtime reply and remove noncard
content. `crates/vak-client-ui/src/components/PresentationRenderer.tsx`
also drops earlier assistant documents under final-answer grouping.
Card-followup metadata is populated only after the first card emission,
which matters for prose before or in the same message as a card.

Correction to avoid carrying an overstatement forward:
`vak-delivery::visible_card_followup` retains the trimmed prose; it is not
a semantic content filter. This review does not establish that every final
paragraph after a card is lost. The concern is independently applied
visibility rules. Use explicit block roles and supersession in one answer
composition model, preserving meaningful source order and correction text.

### P03 — delegated cards lose lifecycle and binding identity (P1; source)

`crates/vak-agent/src/task.rs` transfers presentations for blocking work,
but exports raw presentations including superseded ones and omits original
IDs/replacement/placement relationships. Parent re-recording assigns new
IDs, which can break `bound.from`. Some append failures are ignored.
Background completion retains text/error/time without the same card
transfer. Transfer a resolved answer graph, with provenance and a deliberate
ID mapping, for both modes. Do not implement a second result ledger.

### P04 — interactive state has inconsistent ownership and invalidation (P1; probe/source)

`crates/vak-client-ui/src/answerValues.ts` publishes globally by presentation
ID; independently mounted controls maintain their own local settings.
A production-store probe showed a second publication under the same ID
changing all consumers. This establishes shared-store behavior, not a
browser reproduction of conflicting controls. Source tracing also found
failed computation can leave the previous successful publication usable,
and local list/form state does not consistently resync with authoritative
actions. Optimistic rollback must not overwrite a newer action.

Choose intentional interaction ownership across chat, canvas and page
views; represent invalid/unavailable values explicitly and clean up on
unmount/replacement. The existing maintainer decision remains binding:
living-page control values belong to the viewer and are never saved.

### P05 — answer and page canvas subjects fail persistence validation (P2; probe)

`crates/vak-client-ui/src/canvasSubject.ts` declares `answer` and `page`,
but `isCanvasSubject` does not accept them. Production
`openEntry → toStored → fromStored` returned null for each. Derive/check
runtime validators against the complete union and test every subject kind.

### P06 — canvas tab eviction and failed sync can discard unsent work (P1; probe/source)

`canvasStack.ts` evicts the oldest tab at its cap without protecting a
draft: opening eight more tabs removed an unsent note in a pure-state
probe. `canvasSync.ts` clears pending writes on any 4xx response without
distinguishing authentication, conflict, payload size or throttling.
Draft durability must be independent of tab visibility, with an explicit
recoverable unsaved state and monotonic conflict/retry behavior.

### P07 — canvas persists content outside encrypted fenced Documents (P1; source)

`crates/vak-server/src/canvas.rs::commit` writes raw JSON through its own
lock/rename path. That state can include unsent notes and inline HTML.
The state registry describes a Document, but the writer bypasses the
Document substrate and writer fence. Move it onto existing scoped encrypted
Documents while preserving revision conflict semantics, erasure and
lifecycle behavior. Do not add another encrypted file store.

### P08 — saved pages lack a durable resource-reading context (P2; source)

`crates/vak-client-ui/src/components/PageView.tsx` supplies an empty session
ID, while media preview context uses nullish fallback; empty is not absent.
Copied page items also need stable authorized resource references instead
of relying on a live conversation/workspace context. Test file, media and
preview cards in saved/shared pages, after source closure/restart, through
the actual audience/authentication layers. Do not grant ambient access to
the current session as a repair.

### P09 — adaptive compilation can succeed without meaningful content (P2; probe/source)

`vak-presentation::compile` accepted an empty-props chart as Rich despite
real input data and a usable fallback; coverage reported no rendered or
omitted paths. Structural tree validity did not establish payload validity
or data coverage. The adaptive client path also lacks equivalent fallback
use, local error isolation and card identity context to the structured path.
Validate resolved form requirements and meaningful coverage, preserve
binding identity, and prove fallback handles compilation and render failure.

### P10 — charts infer misleading geometry and can collapse tiny ranges (P2; probe/source)

`crates/vak-client-ui/src/components/cards/Chart.tsx` uses index spacing for
x and filters missing y values, potentially joining gaps. Those choices
need explicit categorical, numeric and temporal semantics. Production
`niceTicks(0, 2e-12)` returned five zeroes, creating a zero displayed range.
Add numerical invariants, missing-data semantics, units and empty states;
do not repair precision with another fixed decimal threshold.

### P11 — streaming still transmits and recompiles growing prefixes (P2; source)

The live text path carries both delta and accumulated text and repeatedly
compiles the accumulated Markdown in `projection.rs`. Work can grow with
the sum of prefixes. This is not a measured latency result. Trace the full
path before optimizing: in-process delta plus snapshot is an invariant;
client wire uses deltas and deliberate snapshot boundaries. Benchmark
incremental/scheduled rendering while retaining replay, resync and partial
output on abort.

### P12 — a receipt for text is not evidence of semantic completeness (P2; probe/design)

Production `vak_judge::plan::gap` accepted `Done.` for two separate text
deliverables, Explain risks and Give alternatives. The receipt is
structural. Presentation prompting also strongly encourages cards. Evaluate
whether the answer addresses the task, uses an appropriate readable form
and preserves grounding; card/tool counts alone cannot establish quality.
Do not blame a smaller model or add lexical success gates. A second model
on every step is not an assumed solution.

### P13 — outline attribution assumes planned card form (P2; source)

`projection.rs::answer_outlines` associates card sections with deliverables
planned as cards, while the runtime permits a card to fulfill a text
deliverable. Carry actual delivery attribution into the composed outline,
including multiple text deliverables and explicit beside placement.

### P14 — text cleanup removes content based on words and marker patterns (P2; source)

`crates/vak-client-ui/src/structured.ts` strips control scaffolding and
assistant phrases through text patterns. Literal code, quoted markers and
ordinary user content need preservation tests. Replace presentation-time
guessing with typed authorship/control metadata; this finding is a source
risk, not a demonstrated loss of every matching string in every view.

## Dynamic, interlinked and ability-backed cards

### D01 — replacing controls collides with its own published names (P1; source)

`crates/vak-agent/src/lib.rs::show_card` accepts a replacement target, then
builds its published-name scope from all current-turn presentations,
including the controls being replaced. Reusing its variable names can be
refused as a duplicate. `SessionLog::presentations` does not itself resolve
supersession. Bindings to the old card ID also need a defined replacement
policy. This is source-confirmed; a full live-agent replacement was not run.

### D02 — the supported topology is controls to consumers (decision)

Controls publish; bound stats, tables and charts consume expressions.
Expressions can combine multiple controls sources. Consumers do not
automatically republish derived output as arbitrary graph nodes; controls
are not a generic bound node. This is a capability boundary, not proof the
existing implementation is defective. State it clearly. Decide whether a
general DAG is actually required before building one; validate references,
types and cycle/ordering rules for whatever topology is supported.

### D03 — ability-backed cards select a latest successful answer by name (P1; source)

`with_call_data` and `SessionLog::answer_of` select a successful ability
answer from the active branch by ability/tool name. A later failed call
does not clear that older successful answer. This is a snapshot lookup,
not an invocation-pinned subscription. Give a card explicit call/version
provenance and a freshness/unavailable state, or explicitly identify the
intended stable snapshot. Do not silently fetch again outside the existing
authorized trigger/effect model. Test heterogeneous records: automatic
table columns taken from the first record can omit later-only fields.

Ability table/chart filling does read recorded result data, including the
full evidence body; it is not simply trusting rows copied by the model.
Preserve that grounding in the repair.

### D04 — lifecycle conformance is thinner than expression coverage (P2; source/test gap)

Expression evaluation has passing tests. Replacement, multi-view state,
invalid values, transfer, reconnect, saved-page resources, audience checks
and cleanup were not proven together. Add a lifecycle matrix across the
existing render paths: ordinary/structured, adaptive, chat, right panel,
canvas, saved page and any constrained surface that claims support.
Check identity, order, provenance, action authority and fallback as well as
appearance. Unsupported forms must degrade explicitly and retain content.

## Executed evidence and limits

At the initial review baseline:

- `cargo test -p vak-storage --lib --offline`: 49 passed.
- `cargo test -p vak-delivery -p vak-presentation -p vak-judge --lib --offline`:
  delivery 81, presentation 21, judge 34; all 136 passed.
- Existing client tests `canvas-subject.mjs`, `canvas-stack.mjs`,
  `presentation-hydration.mjs`, `controls-expression.mjs`, and
  `chart-series.mjs` passed under the client tests directory.
- Four isolated desired-guarantee assertions failed as expected: A01
  cached revoked grant; A02 epoch interleaving; A05 abandoned circuit probe;
  A12 shared blob grant. Four original storage tests included with the
  copied Meta module passed. These were not added as repository tests.
- Scalar conversion used the exact extracted function. Adaptive compilation
  and judge probes linked production crates. Canvas restore, eviction,
  tick generation and value publication exercised production client modules.

At the 8.9.0 dynamic follow-up:

- `cargo test -p vak-agent --test it card_gates::an_ability_card_shows_every_record_its_call_answered --offline`:
  passed (one selected test).
- Delivery controls tests and binding tests each passed two selected tests;
  the client controls-expression test passed.
- A production host/binding probe started with price 100 and quantity 2:
  stats 200, table 205, chart 100/200/300. Changing to price 150 and quantity
  3 produced stats 450, table 455, chart 150/300/450. Combining two controls
  sources with values 2 and 3 produced 5. This confirms implemented reactive
  expressions and consumers, not their entire UI lifecycle.

The isolated harnesses and logs were temporary, outside the repository.
The observations and recipes above are the durable record; future work
must add focused repository regressions rather than depend on those paths.
No source changes, live corruption of product data, completed browser
responsive/theme audit, installed desktop acceptance or real-provider
end-to-end run is claimed. Browser use was unavailable before assertions
completed; the two review dev servers were stopped. Existing green suites
do not refute the missing guarantees, and the probes do not prove every
surface fails. Reproduce each finding at the implementation baseline before
changing it, record counterevidence and close it explicitly if already fixed.
