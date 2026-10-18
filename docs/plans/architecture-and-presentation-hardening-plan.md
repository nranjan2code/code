# Architecture and presentation hardening

Status: in progress — requested on 2026-10-18 to address all observations from
the session's architecture, answer/projection, surface and dynamic-card
reviews. The first S1 slice implements strict grant/effect/connection reads
and atomic ref fencing; its full repository gate passed. Catalog grant
authorization (S1) and owned circuit probes (S2/A05) are now implemented,
with the full repository gate passing. Verified-cache provenance and strict
artifact transaction reads also pass the full gate. Other work remains open.
Unresolved product choices are not maintainer decisions.

## Outcome and scope

A world-class result here means that the system preserves what was said
and shown, executes only recorded and authorized work, survives corruption
and cancellation safely, and presents grounded, coherent answers across
views. Adding more card forms is not the first priority.

The [dated consolidated audit](../audits/architecture-and-presentation-review-2026-10-18.md)
retains all 30 distinct observations, their source locations, evidence
strength, successful checks and limits. This plan addresses every ID;
[open-items.md](open-items.md) owns stage status across the product. The
[handover](handover-2026-10-18.md) supplies the continuation prompt.

This is a code-led repair plan. Old design status is context, not evidence
that a path works. The initial review spanned 8.8.0 and 8.9.0; establish a
fresh source baseline and reproduce before each repair. Work in small,
reviewable slices; do not bundle unrelated storage, UI and core refactors.

## Constraints and decisions

Preserve the modular monolith, broker boundaries, revocation semantics,
append-only causal records, full evidence-backed ability data, and existing
run/trigger/effect/cursor/artifact/grant primitives. Extend existing encrypted
fenced Documents and rollups. Build no parallel answer ledger, outbound
queue, scheduler, authorization list or interaction persistence store.

Read the repository contract and relevant design/security/failure-handling
records before changing their areas. Update their Status only when the
implementation changes what is actually built. Keep viewer controls local
and unsaved as already decided for living pages. Do not introduce a
model-name exception, lexical success heuristic, static model catalog,
magic precision limit or automatic escalation to FullAccess.

Resolve these choices with concrete alternatives and evidence before their
dependent implementation; they need not block independent correctness fixes:

| Choice | Proposed starting point | Required decision/evidence |
|---|---|---|
| Catalog confidentiality (A10) | Treat derived searchable content as part of the data threat model | State the product promise and choose protection/key lifecycle or an explicit narrower boundary |
| Interaction ownership (P04) | Explicit viewer/answer interaction scope with deliberate multi-view synchronization | Specify whether chat and canvas are two views of one interaction; pages remain viewer-local and unsaved |
| Replacement semantics (D01) | Resolve a live graph and preserve logical identity through replacement | Define whether existing consumers follow a replacement or stay pinned; no silent orphaning |
| Graph scope (D02) | Harden the implemented controls-to-consumers model first | Generic derived-card DAG only if requested and justified; define typing, ordering and cycles before expanding |
| Ability freshness (D03) | Pin a recorded invocation/version and expose its state | Decide when a stable snapshot is intended and how refresh is authorized; no silent latest-success substitution |

No further Astra consultation is part of this work. Maintainer decisions
belong in the decision log; recommendations above must not be recorded there
as approved architecture.

## S0 — establish a reproducible baseline

Scope: all IDs, especially qualification of A12, P02, P04 and D01.

- Record HEAD, version, dirty files and exact test commands. Preserve other
  sessions' work. Check every observation against current code; mark an
  already-fixed observation with the fixing commit and counterexample test.
- Convert primitive recipes into focused repository regressions in the
  appropriate stage. Use isolated homes and deterministic interleavings;
  do not corrupt a live home or depend on temporary review harnesses.
- Build a surface matrix covering each supported form and the structured,
  adaptive, chat, right panel, canvas, saved-page and constrained paths.
  Include identity, source fidelity, order, action authority and fallback.
- Determine whether the blob API in A12 is reachable in shipped code. Keep
  a primitive defect explicit even if product reachability is absent.

Exit: every ID is classified reproduced/source-confirmed/resolved/decision
or unreachable in the product, with no evidence inflation. Test limitations
and pending product choices remain visible. This stage does not require a
complete browser audit before the first storage fix.

## S1 — fail closed and fence authoritative state

Scope: A01, A02, A10 and A12; P07 storage repair may proceed after the fence
contract is established. Highest-risk correctness work starts here.

1. Separate strict authoritative chain/tail/rollup reads from explicitly
   best-effort browsing. Propagate segment, key, frame and decode failures;
   a cached positive grant cannot survive an unreadable revocation suffix
   as an authorization success. Audit all decision consumers, including
   effects, accounting, grants and connection/capability state. Keep the
   existing rollup model for the turn path.
2. Check the global writer epoch and mutate/remove a ref atomically in the
   underlying cross-process transaction. Audit adjacent bookkeeping and
   lock order. Do not give an old process the store's newest epoch.
3. Resolve A10's confidentiality boundary and implement the chosen derived
   catalog protection as its own slice, including WAL, temporary files,
   backups, erasure and rebuild behavior. Do not call secure deletion
   encryption or silently weaken the product promise.
4. If A12 is reachable, repair shared chunk ownership before release of its
   callers; otherwise quarantine/repair the primitive before introducing
   a caller. Integrate with existing root/lifecycle rules.

Exit tests: corrupt and truncate each authoritative source boundary after
a cached grant, revoke across segments, inject malformed rows, lose a key,
and simulate stale rollup catch-up. Decisions fail closed with typed errors.
Deterministic stale-writer races for CAS and remove fail across independent
handles/processes, including restore/takeover. Shared-chunk release preserves
all remaining live roots and does not retain unreachable data forever.
Catalog acceptance matches the explicit decision. Record any blocked
catalog decision separately rather than claiming S1 fully complete.

## S2 — make run execution causal and cancellation-safe

Scope: A03, A04, A05, A06 and A07. Can proceed alongside independent S1
slices without changing storage contracts under a running test.

1. Require a successfully recorded causal assistant/call entry before
   tool execution. Model-visible inputs remain reconstructable from the
   ledger. Separate required journal writes from optional diagnostics and
   preserve partial output when an append fails or a turn aborts.
2. Introduce one production run-construction contract: immutable admitted
   policy plus required services, with explicit child narrowing. Enumerate
   main, blocking child, background child, scheduled and flow entry points.
   Preserve artifacts, protected paths, file provenance and step/history
   behavior; fixtures remain explicit exceptions, not production fallbacks.
3. Make circuit probe admission an owned permit with neutral release on
   cancellation, budgets, early returns and unwinding. Preserve the existing
   informed-transience versus breaker-failure distinction.
4. Remove the compaction/error lock cycle. Audit helpers that acquire the
   same session lock while a guard is live; make ownership visible in APIs.
5. Give unknown-effect tools conservative scheduling. Consolidate execution
   effect/claim facts enough that workspace mutation and claims cannot
   disagree silently; retain optional presentation facets.

Exit tests: inject assistant append failure and prove no dependent dispatch;
fail a compaction append and prove bounded completion; cancel after probe
admission and prove future recovery. Compare parent/child/service contracts
using real production constructors. Mixed MCP/built-in waves serialize
unknown or exclusive work and permit concurrency only with justified claims.
Revocation during each run kind stops stale capability. No broader Core
rewrite is necessary to close these failures.

## S3 — preserve answer identity, content and data fidelity

Scope: P01, P02, P03, P13, P14; A11 protocol work can be introduced here
and completed in S6. Depends on S2 for delegated run/journal guarantees.

1. Preserve source scalar types and lexical strings through card building,
   sorting, display and export. Numeric interpretation is explicit, with
   locale and precision behavior; ability results use the same contract.
2. Make answer block roles, source order, supersession and actual
   deliverable attribution explicit in existing presentation records and
   projections. Remove independent text-hiding guesses in server/client.
   Preserve useful narration and correction text before, within and after
   card emission, including multipart answers and cards fulfilling text.
3. Transfer the resolved delegated answer with provenance and a complete
   binding/replacement ID map in both blocking and background paths.
   Required append failures must be visible and cannot claim delivery.
4. Replace word/marker stripping with typed control/authorship boundaries.
   Literal examples, code fences, quoted markers and user content survive.
5. Use typed terminal outcomes and mechanically checked Rust/TypeScript
   event contracts. Plan a deliberate protocol transition if needed; avoid
   maintaining two indefinitely divergent answer representations.

Exit: golden/replay cases assert semantic content and attribution across
source ledger, wire, hydrated answer and rendered view. Cases include
multiple meaningful assistant blocks, an error then recovery, before/same/
after-card prose, replaced cards, two text deliverables fulfilled by cards,
child linked cards and background completion. Scalar fixtures include
leading-zero identifiers, locale commas, large integers, decimals, missing
values and heterogeneous records. No prose is dropped solely because a
card exists or a phrase resembles runtime chatter.

## S4 — make dynamic and interlinked cards coherent over their lifetime

Scope: P04, D01, D02, D03 and D04. Depends on S3's identity/transfer contract;
independent invalidation fixes may start earlier.

1. Resolve active presentations after replacement before checking published
   names. Apply the chosen identity policy to all dependent bindings; refuse
   genuinely invalid references with an actionable typed reason.
2. Implement the chosen interaction scope consistently across controls,
   consumers and views. Invalid compute results invalidate publications;
   reset, replace, remount, unmount and reconnect have explicit behavior.
   List/form state follows authoritative actions; stale rollback cannot
   overwrite a newer success. Page values remain viewer-local and unsaved.
3. Specify and validate the current controls-to-consumers topology, including
   multiple sources, types, missing names and ordering. A generic DAG is
   optional decision work, not a prerequisite for fixing existing links.
4. Bind ability cards to explicit recorded call/version provenance and
   freshness. Distinguish absent, failed, stale and successful results.
   Keep full-result grounding and cover later-only table fields. Refresh
   goes through existing admitted work and effect/trigger primitives.

Exit: retain passing price/quantity, multi-source, table and chart examples;
add same-name controls replacement, dependent identity migration/pinning,
invalid expressions, duplicate names, source failure after earlier success,
switching branches, multiple instances, child transfer, reload and cleanup.
Verify the chosen chat/canvas ownership behavior and separate page viewers
in the browser. Unsupported topology is explicit; no stale successful value
masquerades as a fresh result.

## S5 — make canvas and saved pages durable and authorized

Scope: P05, P06, P07, P08 and D04 surface coverage. P07 uses the S1 fence
contract; P08 uses S3/S4 identity and provenance.

1. Derive or exhaustively check persistence validators from the subject
   union. Every supported kind round-trips, including answer and page;
   invalid records fail explicitly rather than silently erasing drafts.
2. Store canvas content through existing scoped encrypted fenced Documents.
   Preserve revision conflicts, retention/erasure and unknown-field policy
   consistent with the supported data baseline. Do not carry raw plaintext
   content forward as an undocumented compatibility store.
3. Protect unsent drafts independently of the tab cap. Distinguish conflict,
   expired auth, throttling and oversized writes; show recoverable unsaved
   state and never silently acknowledge or clear work that did not persist.
4. Resolve saved-page resources through stable artifact/version and audience
   references, without an ambient current session/workspace. Cover files,
   media and preview origins as well as ordinary card values.

Exit: all-kind state round-trip, tab eviction with drafts, refresh/restart,
simultaneous writes and 401/409/413/429/network failure tests preserve work
or show an explicit recovery state. Fence and erase while handles are open;
read back through real server layers. Saved/shared pages work after their
source conversation closes and deny an unauthorized guest. No note/HTML
plaintext bypass remains. Run browser acceptance at 1440×900 and 390×844,
light and dark, including right-panel/canvas navigation and focus.

## S6 — render truthfully, stream efficiently and measure answer quality

Scope: P09, P10, P11, P12, A11 and remaining D04 conformance. Correctness
acceptance comes before performance tuning.

1. Validate required primitive payloads after adaptive binding. Report
   meaningful rendered/omitted source coverage. Preserve card ID context,
   isolate render errors and display the original fallback for compilation
   and runtime failure. Exercise every advertised primitive/form.
2. Make chart axes, missing-data gaps, precision and units explicit. Test
   finite nonzero displayed ranges, tiny/large/negative/constant values,
   empty/single points, irregular times and absent values. Use numerical
   properties rather than one example-specific rounding patch.
3. Measure long streaming answers through the real projection path. Retain
   in-process delta plus snapshot; send client deltas and boundary/resync
   snapshots. Reduce repeated Markdown work only with measured improvement.
   Reconnect, slow consumers, abort and coalescing retain complete partial
   output and converge to the authoritative snapshot.
4. Build a task-level quality evaluation set: direct text, structured data,
   multipart explanation, comparisons, tools/abilities, dynamic controls,
   partial failure and recoveries. Score completeness, grounding, readable
   form, action usability and honest partial outcomes. Structural receipts
   remain evidence of delivery, not a claim of semantic success.

Exit: structured/adaptive and all advertised surfaces agree on content,
identity, attribution and action authority; fallbacks retain information.
Wire contract checks cover every event/outcome and replay transition.
Publish latency, payload and compilation measurements with workload, route,
hardware and baseline, not an unmeasured speed claim. Quality evals inspect
actual answers rather than card/tool counts or keyword matches. Browser
acceptance includes themes, narrow layout, keyboard use and accessibility.

## S7 — simplify ownership and bound growth

Scope: A08 and A09, with A04/A07 interfaces retained. Begins with measured
profiles and stable contracts from earlier stages, not a wholesale rewrite.

- Separate process services, Agent/workspace services, admitted run context
  and turn execution incrementally within the modular monolith. Move domain
  features behind narrow extension interfaces where appropriate. Every move
  must remove a responsibility from the old owner, not add a forwarding layer
  that leaves two competing constructions.
- Replace hot full-chain decisions with existing rollup/ref projections;
  make endpoint pagination genuinely bounded and move unavoidable blocking
  work off async executors. Include effects, pending work and accounting.
- Measure small and growing histories, cold start, rebuild, concurrent
  writes and recovery. Verify projection catch-up errors retain S1's strict
  semantics and catalog/audience rules.

Exit: one production construction path per run kind; explicit ownership and
lifetimes; no new parallel scheduling/data system; bounded decision work
with reported measurements and recovery tests. Retain fault, security and
presentation conformance checks through each refactor.

## Completion and reporting

### Implementation log — 2026-10-18, first S1 slice

> **Historical record** — committed as `e469dc976`; the continuation below
> updates coverage beyond this first slice.

- Baseline `5c218fc9b` (8.9.0), with only this session's consolidation
  documents already dirty. Implementation authorized by the maintainer;
  narrow storage work delegated to Luna, with integration here. No Astra.
- A01 partial: strict tail/head/chain and rollup APIs verify frame/seal
  integrity, surface read/decode failures and reject a chain behind its
  rollup. Grants, effects and mail/calendar account state use them;
  conditional chain appends use a verified suffix check. Regressions cover
  a cached grant followed by an unreadable/truncated revocation, missing
  cached history, malformed rows, erased keys and partial fold failure.
- A02: global epoch validation, user ref CAS/removal and commit generation
  move in one underlying transaction; epoch advance is atomic as well.
  Pre-ack still runs after the durable local commit. Independent-handle
  stale/racing writers and read-only torn-manifest behavior have regressions.
- Integrated review added rejection of malformed/ambiguous segment names
  and duplicate/out-of-order seal entries. Seal verification reads one
  byte snapshot and never repairs the manifest. Integration testing also
  exposed contention between the new guarded commits and the old ordinary
  commit counter's read/CAS sequence; all commit-counter increments now use
  an atomic RefStore operation. A mixed object/ref concurrency regression
  checks shared and independent handles and exact commit accounting.
- A new regression reproduced a missing unread revocation segment followed
  by a later valid segment: the first strict-reader implementation still
  allowed the grant. The reader now requires every unread segment named
  by the seal manifest to be present. Strict empty/head reads also verify
  manifest-only history and never create a directory. Acknowledged prefix
  data need not be replayed; missing unread history cannot be skipped.
- A01 is not closed: catalog/search grant projections, artifact, commitment,
  routing, accounting and remaining authoritative chain consumers still
  need migration/audit. Strict grant-service reads alone do not establish
  catalog search authorization safety under corruption. The catalog
  confidentiality choice and blob ownership work have not started. S0's
  all-finding/surface matrix is also incomplete. No UI acceptance is claimed.
- Verification: `bash scripts/gate.sh` passed in full on the final source
  tree: formatting, workspace Clippy with warnings denied, workspace tests
  (749 seconds), version stamps, document paths and records. The cached-
  grant, missing revocation segment, connection/vault, strict-frame/fold and
  mixed object/ref concurrency regressions all passed. Two earlier gate
  attempts were deliberately interrupted to finish the missing-segment
  coverage; they are not counted as completed gates. No browser acceptance,
  release, install or deployment is claimed. The maintainer requested committing this verified slice before continuation.

### Continuation — catalog grant authorization and owned probes

- Baseline `e469dc976`, after committing the verified first slice at the
  maintainer's request. No Astra; Luna handled A05 and a read-only catalog
  review, with integration and catalog implementation by the main agent.
- A01 catalog scope: two new tests first reproduced stale search access
  without caller catch-up and silent acceptance of unreadable grant
  history. Catalog search/list/tool-history queries now catch up grants in
  the query transaction, using shared canonical grant types from
  `vak-session::grants` (re-exported by `vak-core::grants`). Strict ingestion
  rejects malformed rows/frames and missing indexed history, rolling back
  the projection and cursor together. Schema 7 rebuilds the older derived
  grant cursors; the primary-data baseline is unchanged.
- Focused catalog tests: all 9 grant integration tests and all 12 catalog
  unit tests pass, including the existing million-node query check. Added
  rollback/recovery and version-six tolerant-cursor rebuild regressions.
- A05: a provider leg holds an owned admission permit. Neutral drop
  releases an abandoned probe; unique reservation tokens prevent stale
  drops or late outcomes from altering a newer probe. Added concurrent,
  retry, stale ownership and real Agent abort regressions. The diagnostic
  health caller was adapted to retain/drop the new permit correctly.
- A01 remains open for artifact, commitment, routing, accounting and other
  authoritative decision readers. Other S2 findings remain open; A05 alone
  does not complete S2. Catalog confidentiality (A10) is a separate choice.
- Verification: the full `bash scripts/gate.sh` passed (formatting, workspace
  Clippy, workspace tests, version, paths and records). The first run exposed
  a stale traced-record reference after moving GrantEvent; its source path
  and generated dictionary are corrected and the completed gate includes
  that check. Circuit tests: 8 unit and 7 integration cases. No UI acceptance
  or deployment is claimed.

### Continuation — verified rollup provenance and artifact transactions

- Catalog authorization and owned probes passed the full gate (622-second
  workspace test phase) and are committed as `dd0cc34bb`. The first slice
  remains `e469dc976`.
- A01: strict rollup reads now reject unverified caches and rebuild from
  the chain. Only a strict fold saves a verified cache; an equal-position
  tolerant cache cannot block the verified replacement. This closes the
  migration gap where a previously skipped malformed row lay behind the
  cache cursor. A focused test checks rebuild and subsequent cached reads.
- Artifact transactions now use strict snapshots and fallible content
  restoration before invoking a decision closure. Explicit erasure remains
  an intentional omission; missing/unreadable content is an error. Tests
  prove malformed rows hidden by a tolerant view cannot authorize a
  transaction and corrupt/truncated suffixes cannot invoke its closure.
  All 14 artifact integration tests pass. Read-only artifact get/list and
  their decision-making callers still need audit; A01 stays open.
- Luna investigated accounting but its partial patch was withdrawn. The
  same-day cached baseline bypasses a first-use-only strict read. The next
  accounting slice must refresh verified cost state on every admission,
  avoid double-counting settlement rows and retain conservative reservations
  on append/read failure. Design it on the existing rollup/ref primitives
  with a registered scoped Document if a new projection is necessary;
  replaying the entire chain per dispatch is not an acceptable final fix.
- Verification: the full `bash scripts/gate.sh` passed: formatting, workspace
  Clippy, workspace tests (737 seconds), version stamps, document paths and
  records. The artifact transaction, strict-cache rebuild and erasure suites
  passed. No other architectural or presentation stage is closed by this work.

### Continuation — required journals, compaction locks, claims and Canvas restore

- A03: assistant appends return a required Result. A failed message write
  settles before dependent tools dispatch; system-authored completion
  appends also propagate failure. A real Agent regression swaps its session handle to read-only
  during provider dispatch and asserts no tool ran or second request was
  sent. Other mandatory causal records still need audit.
- A06: the compaction writer releases its session mutex before recording an
  optional failure diagnostic. A bounded test forces an invalid packet and
  verifies error reporting completes and the diagnostic is recorded.
- A07: unknown footprints conflict with other unknown, path-writing and
  exclusive claims. Every call uses conflict-based wave placement; the
  previous unknown-to-wave-zero bypass is removed. Explicit read-only and
  disjoint declared scopes retain their existing concurrency. MCP call
  metadata has no trusted effect declarations and stays unknown.
- P05: Canvas subject validation now includes answer session/turn identity
  and page artifact identity. Tests cover all nine subject kinds through
  openEntry/toStored/fromStored and refuse missing or mismatched identities.
  Both shared-client bundles build; source subject/stack tests pass.
- Verification: the full `bash scripts/gate.sh` passed (formatting,
  workspace Clippy, workspace tests in 715 seconds, versions, document
  paths and records). Four focused Agent regressions and seven resource
  claim tests pass. The first gate attempt caught a collapsible-if Clippy
  warning; it was fixed before the completed gate. Both client bundles and
  subject/stack source tests pass. No source changed during the completed
  gate. Running-app reload acceptance remains open. This slice changes no
  layout and does not close S2 or S5; other mandatory writes, draft
  preservation, encrypted Canvas storage and saved-page resource context
  remain open.

### Continuation — commitment decisions and numerical chart ranges

- The required-journal, compaction, unknown-claim and Canvas restoration
  slice is committed as `c6b65b579`, after the full gate passed.
- A01: commitment projections use strict fallible folds and verified caches.
  get/all/open expose errors, and new commitment creation verifies history
  under the existing writer lock. Episode planning and model-visible prompt
  projection propagate failures; maintenance/tool/server/CLI callers no
  longer turn unreadable history into empty successful state. Erasure
  already invalidates the derived commitment rollups before rebuilding.
- P10 numerical slice: tiny ticks no longer collapse to zero; clean decimal
  grids avoid repeated-addition drift. Grid generation is bounded and falls
  back to exact endpoints when a grid cannot represent the domain safely.
  Labels retain nearby values while keeping extremes compact. Chart y
  coordinates avoid opposite-sign span overflow. Existing percentage scales
  remain unchanged. Source tests and 10,000 deterministic finite-domain
  probes pass; both client bundles build.
- P01 chart slice: chart scalars retain category strings instead of parsing
  them as numbers, and y values require a number after argument reading.
  Tests cover lexical categories and an ability-backed category, and reject
  comma-formatted numeric strings instead of guessing a locale. Tables,
  large numeric values across JSON and explicit numeric/locale contracts
  remain open; this does not close P01.
- The completed full gate passes for this continuation. P10 is not closed: x-axis
  semantics, missing-data gaps, units/empty states and running-app acceptance
  remain. A01 still includes routing/accounting and other decision readers.
- A bounded read-only P01 consultation confirmed that a coherent fix must
  preserve lexical strings, give numeric columns explicit semantics, avoid
  stringify/reparse of ability records, keep large numbers exact across the
  JSON/client boundary and retain old recorded payloads. No scalar payload
  or schema change for tables is included in this continuation.

### Verification-discovered credential presence race

- The commitment/chart gate passed formatting and Clippy, then the Core
  suite failed `trust::tests::a_project_config_or_env_makes_the_workspace_privileged`
  (498 passed, one failed). It passed alone. The test also lacked home
  isolation; trust fixtures now isolate before touching credential state.
- Code inspection confirmed an independent concurrency defect: presence
  writers read/overwrite the shared index without coordination and readers
  can observe an in-progress write. A present secret can lose its marker.
  The fix serializes index reads and secret/presence updates using the
  existing index inode, with presence published before setting a secret.
  Metadata I/O/format errors no longer become an absent scope, and failed
  backend removals retain presence. No secret value enters the index.
- Seven focused credential tests pass, including concurrent readers/writers,
  malformed metadata, failed-removal retention and invalid variable names.
  The focused trust regression passes. This fixes concurrency and detected
  read errors; in-place publication can still leave an empty/corrupt index
  after a crash. Durable publication and erasure receipt propagation on
  credential cleanup errors remain A01 follow-ups; no crash guarantee is
  claimed. The completed full gate passes after integration.
- Parallel dependency/advisory/record-drift work is committed through
  `1ac94fe55` (including `825b18564`). The final gate ran with that baseline:
  formatting, workspace Clippy, workspace tests (788 seconds), versions,
  paths, records/drift and advisories all pass. Earlier attempts caught a
  missed fallible-API test caller, a CLI lint and the trust fixture/index
  issue; each was repaired before this completed run. No source changed
  during the completed gate. Both client bundles also pass; no live UI
  acceptance is claimed.

### Prepared next implementation scopes

These are implementation proposals from bounded code inspection, not
maintainer approval of a new product model:

- Accounting (A01): add a registered scoped cost rollup with per-UTC-day
  totals, validate finite/nonnegative USD, and read it strictly on every
  admission. Keep unresolved reservations across day rollover. Hold the
  existing shared day mutex across settlement append and reservation
  disposition; release only after successful append, otherwise retain at
  least the estimate or larger known actual in day/run state. Do not count
  a settled row both in a cached baseline and the rollup. Preserve the
  reserving `authorize` contract or replace all callers explicitly; no
  whole-chain replay per dispatch. Trace belongs to the reservation, not a
  mutable later gate trace. Recovery of in-memory reservations after process
  restart remains separate durable-accounting work.
- Child construction (A04): share a production run-service contract through
  TaskDeps, with child context and factories for session/run-bound services.
  Inherit protected paths and narrowed policy/capacity. Rebind artifacts,
  file notes, history and checkpoint services to the correct child context
  where their current closures capture the parent. Reuse the common day
  budget; do not mint an independent budget when making a child gate.
  Preserve child role, admitted tools, trace/run, provenance and read-only
  narrowing. The parent's aggregate run budget must also remain shared:
  giving each child a fresh gate with the same ceiling would widen it.
  Pass dispatch attribution separately from the shared budget state (the
  current SpendCheck has a session id but no trace). Verify each captured
  service against code before copying it.

### Prepared presentation scopes

These bounded inspections do not close P01/P02:

- P01 table loss is in `presentation_tools::build_table` and ability-table
  filling: spelling-based inference strips commas and parses through f64.
  Keep text as text, add an explicit numeric-column tool argument, and
  preserve exact decimal strings in the existing rows plus numeric-column
  metadata. Ability numeric status comes from source JSON types. Existing
  table validators already accept scalar strings, numbers and booleans;
  keep legacy payloads valid and existing expression/binding paths intact.
  The client needs decimal comparison without Number conversion or exponent
  expansion, while display/export preserve strings. Parsed floating-point
  source tokens cannot be recovered by a downstream builder.
- P02 eligible leading prose survives server projection but grouped client
  rendering picks the last assistant item and can omit earlier text.
  Server `card_followup` currently includes only text after the first card.
  Fix the composition boundary using existing source order and typed draft/
  runtime exclusions. Preserve prose before, between and after cards; merely
  concatenating all prose after cards is not full ordering acceptance.
  Typed PlanCheck and rejected-draft filtering must stay covered. Residual
  lexical envelope/scaffolding cleanup remains a separate typed-boundary
  concern, not permission to suppress more prose.

### Continuation — routing evidence, accounting and live revisions

- Route evidence folds reject undecodable records and read verified rollups.
  Planning errors propagate through session/turn admission and discovery;
  discovery and ranking semantics otherwise remain unchanged.
- Day-cap checks read a strict daily cost rollup on each capped admission,
  including same-day external appends and unknown-cost rows. Its derived
  path is declared in the registry and resolved through scope accessors.
- Reservations track active liabilities even while uncapped, preserve their
  admission trace/actor, and append settlement before releasing liability.
  Failed settlement appends retain conservative liabilities across rollover.
  Successfully recorded unknown costs block today's day cap through the
  rollup and keep an aggregate unknown run liability without leaking a
  per-call daily marker forever. Identity-free legacy settlement releases
  the smallest known estimate conservatively.
- Live outcome revisions publish only after recording succeeds; failed
  writes preserve the prior runtime contract. Steering and managed
  assumption completions require recording. A failed post-approval budget
  recheck stops before provider dispatch.
- Focused checks pass: 29 FinOps tests, 9 spend-gate persistence tests,
  14 routing tests, 2 route-backup tests, flat-path and discovery tests,
  outcome/steering regressions and all 5 agent spend-gate tests.
- Full repository gate passes: formatting, workspace Clippy, all workspace
  tests (725 seconds), version, doc paths, records and advisories. The first
  attempts caught a helper argument-count lint and an adoption-matrix fixture
  that relied on approval bypassing a second denial. The helper now uses a
  settlement context; the fixture explicitly changes its gate on approval
  and asserts re-admission. Persistent denial still has a zero-dispatch test.
- A01/A03 and stages S1/S2 remain partial:
  other readers/causal writes and durable/cross-process reservations are
  not closed. No UI source changed and no new live UI acceptance is claimed.

### Continuation — table scalar fidelity

- Authored tables preserve string cells; `numeric_columns` explicitly names
  decimal columns and refuses missing/ambiguous/repeated labels, malformed
  declarations and ambiguous numeric spelling. Ability tables derive default
  numeric semantics from source JSON types and preserve integer text.
- Existing stored table payloads remain valid. Validation and text delivery
  keep exact decimal strings, IDs and commas; CSV keeps its existing formula
  escaping. Dynamic bindings keep their existing path and numeric evaluator.
- Client numeric sorting compares decimal order/significands exactly, with
  stable invalid/missing ordering; text columns sort lexically. A review
  caught and fixed a zero-versus-subunit ordering error. Six helper tests and
  10,000 deterministic comparisons against integer-scaled reference values
  pass; the delivery compatibility/text fallback regression passes.
- `tests/table-fidelity.html` uses production FormView, controls publication,
  binding and CSV. Seven checks pass at 1440×900 and 390×844, light and dark:
  display, numeric sort in both directions, text sort, CSV, binding updates
  and page overflow. Screenshots were inspected in all four configurations.
- The general presentation gallery fails its 260px chart/map fit assertion;
  this batch changes no chart/map renderer. Its remaining checks did not run
  after that assertion. This is separate unresolved S6 responsive acceptance,
  not a passing whole-gallery claim.
- Both client bundles and the full repository gate pass: formatting,
  workspace Clippy, workspace tests (722 seconds), version, doc paths,
  records and advisories. Source float token recovery,
  arbitrary-precision dynamic expression arithmetic and remaining S3–S6
  findings are not closed by this slice.

### Continuation — ordered composed-answer prose

- Removed the client's final-assistant-only grouping and independent
  card-based prose suppression. Adjacent projected parts share a result
  header/actions, while each prose document remains in source order.
  Card layout runs only within adjacent card segments, never across prose.
  Known distinct result ids and non-result entries split groups; files keep
  their result file area. Technical mode draws all projected entries.
- Removed the turn's phrase-based fleeting-narration classifier. Typed
  runtime/rejected-draft exclusions remain on the server. Other lexical
  scaffolding/envelope filters and actual outline attribution remain open.
  Existing `card_followup` metadata stays compatible for older consumers;
  the shared client no longer draws it as duplicate prose.
- Added executable production-helper tests for leading/interleaved/closing
  prose, text-only multipart answers, lexical prefixes, distinct result ids,
  non-result boundaries, adaptive cards and files. Added a server projection
  source-order regression and a production timeline browser fixture.
- Both bundles and all 48 server projection tests pass. The full gate passes:
  formatting, workspace Clippy, workspace tests (662 seconds), versions,
  document paths, records and advisories. Review retained each group's known
  result id for actions/files even when its last prose has no result id;
  absent result ids are not invented. The gate was restarted after this
  correction, with source frozen through its successful run. Browser acceptance
  is pending because local browser use is disabled in Work settings; no
  visual result is claimed for this slice. P02/S3 remain partial.

### Continuation — child policy, scoped spending, delegated bindings and notes

- A04 immutable inheritance: TaskDeps carries configured per-purpose steps,
  protected paths and stop policy from the admitted parent; child config
  preserves them. Existing admitted-tool/read-only narrowing remains.
- Budget attribution: SpendCheck supplies a dispatch trace, snapshotted by
  the existing shared gate into each reservation. Child actors/traces stay
  distinct while aggregate run/day counters remain shared. Legacy callers
  without a dispatch trace retain the existing turn-trace fallback.
- P03 transfer carries child presentation IDs in process. Background worker
  records preserve cards and provenance; result/wait return separate groups
  per source worker. Foreground and background use the same importer. A
  per-turn digest index and child-to-parent map resolve newly recorded and
  deduplicated cards; only bound source IDs change. Unknown/malformed/forward
  references and append failures are reported in the tool result. The
  original identity digest predates binding metadata, so stays unchanged.
  Evidence references remain parent-scoped. Only successfully appended or
  reused parent IDs contribute card-emission receipts. Ordinary tool batches
  with no delegated cards skip the index entirely. Durable child presentation
  provenance is still open.
- P06 automatic eviction skips tabs with any unsent note (including typed
  whitespace). When every tab has one, opening a new subject leaves state
  unchanged, queues no rejected open, and gives a visible notice; an existing
  tab can still reopen. Clearing a note makes that tab evictable. The server's
  eight-tab limit stays unchanged. Pure-state round-trip regressions pass;
  a scoped Canvas browser fixture is ready but not run while access is denied.
- Both client bundles pass. Focused validation passes: child-step/path/stop
  policy and readonly regressions; 30 FinOps cases with separate actors/traces,
  shared-cap denial and out-of-order settlement; foreground binding/dedupe and
  rejected-source tests; background result/repeated-wait imports and multiworker
  grouping, parent isolation and conservative provenance. Agent/core Clippy
  passes. The full repository gate passes: formatting, workspace Clippy,
  all workspace tests (740 seconds), version, doc paths, records and
  advisories. Source was frozen throughout the completed gate.
  No new live UI acceptance is claimed. No stage or whole A04/P03/P06 finding
  is closed: child-scoped services, durable child-card provenance, replacement state,
  durable reservations and safe sync/conflict/reload handling remain open.

### Interaction ownership implementation

Code-led follow-up for P04/D01 found that `answerValues.ts` has one module-global
publication map keyed only by presentation ID, and each Controls mount owns
settings and republishes them independently. PageView's empty session disables
durable interactions but still reuses source presentation IDs. This can mix
page-local inputs with conversation values or reset values on another mount.

The implementation now uses an in-memory answer-values scope/context holding
both publications and current control settings. Conversation views share the
same session/turn scope; page views share an immutable page-version scope
within this viewer, separate from source conversations and other versions.
Scopes count their mounted owners, reconcile their union of source IDs, and
release temporary settings/publications after the last owner leaves. Library,
SharedArtifact and Canvas page loaders carry the immutable version together
with its decoded bytes, so retained resource data cannot acquire a newer
version's scope while another request loads. Conversation actions and unsaved
page-local edits keep their existing behavior. Invalid control inputs clear
stale publications and dependent fields use their recorded fallback. Source
snapshots replace removed names. Pure state tests pass; mounted UI fixtures
are prepared and typechecked but not browser-run. Both client bundles and
the full repository gate pass: formatting, workspace Clippy, all workspace
tests (708 seconds), version, doc paths, records and advisories. Source was
frozen throughout the completed gate. An initial attempt stopped at the
desktop bundle freshness check because it started before the final rebuild
finished; both bundles were rebuilt before the successful full run.
This does not settle replacement identity or the wider lifecycle matrix.

### Continuation — active replacement scope

- SessionLog derives active presentations from recorded accepted card calls
  on the active branch. Only an earlier active card of the same turn can be
  retired; raw presentation history stays append-only. An unrecorded tool
  attempt cannot retire a card. A recorded replacement takes effect before
  the tool-result batch is written, so later calls in that batch see it.
- Runtime binding names, replacement targets, duplicate checks, completion
  evidence and delegated transfer use the active view. An unused controls
  replacement may keep its names. A source with active bound consumers is
  refused with `card_source_in_use` and dependent IDs. This is an explicit
  unsupported operation, not automatic binding migration.
- Snapshot projection shares this resolver and drops retired cards before
  content deduplication, allowing an accepted replacement to restore an
  earlier payload. No ledger record is rewritten or deleted.
- Validation passes: ten session presentation tests, two actual Agent
  replacement regressions, all 48 server projection tests and the full
  repository gate (749-second workspace tests, formatting, Clippy, version,
  doc paths, records and advisories). Source was frozen during the completed
  gate. The Agent fixture initially rejected controls under its chart-only
  test schema; it was corrected before the successful runs. Live-event
  supersession, dependent identity migration, interaction-action cleanup
  and the wider lifecycle matrix remain open. No browser acceptance claimed.

### Continuation — preserve failed Canvas saves

- Extracted the existing queue into an injected coordinator so failure/race
  tests execute its production policy. No additional durable store or queue
  model was introduced. All pending operations survive failed writes until
  their sent prefix receives an acknowledgement.
- Authentication, missing/invalid/newer/oversized responses pause automatic
  retries; network, server and throttling failures retry after five seconds.
  The frame shows saving/error status and Retry saving; focus/visibility and
  local edits can retry paused work. Stream hints cannot force retries of
  paused queues or overwrite pending notes. A standard before-unload warning
  is requested while any conversation has unsaved changes.
- Cancel a timer scheduled by newer edits when an in-flight request fails
  permanently. Prevent a delayed read from lowering an observed revision.
  Track only active conversation changes in the setup effect, preserving
  the intended typing debounce.
- Focused Node queue/Canvas tests and fixture typechecking pass; both client
  bundles pass. The first full gate stopped in `server_ext` with SIGBUS:
  the macOS report showed a SQLite WAL page-in failure during a fence check.
  Two fixtures dropped directories while process-global stores retained
  their mapped files. They now retain those directories for the process's
  lifetime, matching the other fixtures. A standalone rerun passed all 27
  tests before the fix; all 27 also pass after the correction. The fresh full
  gate passes: formatting, workspace Clippy, all workspace tests (661 seconds),
  version, doc paths, records and advisories. Source was frozen throughout
  the completed gate. The UI fixture is not browser-run because
  local browser access remains denied. This queue remains in memory; forced
  reload/process loss, conflict rebase losses and encrypted Document
  persistence are still open. Do not close P06/P07 or S5 from this slice.

### Continuation — guard Canvas stale replay

- Track the last accepted server Canvas beside its revision in the existing
  in-memory coordinator. Rebase compares pending local draft changes with
  that baseline and the stale-write response; ordinary reads and view-only
  changes accept remote note updates normally.
- Restore a remotely closed tab containing a pending local note using the
  existing open/eviction rules. Preserve final local input across queued
  keystroke snapshots, device mode, and unrelated remote notes. A pending
  explicit close supersedes its earlier typing; a final cleared draft is
  not revived by intermediate edits.
- Competing changes (including remote clears) or an unavailable safe slot
  pause saving without publishing a partial merge, clearing pending work or
  advancing the CAS revision. Retry therefore cannot overwrite the remote
  head after a rejected merge. The frame explains the conflict and offers
  Copy unsent notes; a clipboard failure has a manual-copy explanation.
- Pure rebase, coordinator and integrated tests pass, including conflict
  retry from the original base and successful saving after manual remote
  reconciliation. Both client bundles and the full gate pass: formatting,
  workspace Clippy, all workspace tests (658 seconds), version, doc paths,
  records and advisories. Canvas source was frozen during the completed
  gate. Concurrent blueprint subsystem-map changes were preserved and the
  record checks rerun after they landed. Browser UI acceptance remains blocked. This is conservative conflict protection,
  not an in-app note-version chooser. That recovery UI, forced-reload and
  process-loss durability, and encrypted Document persistence remain open.

### Continuation — encrypt and fence saved Canvas state

- Replace ordinary Canvas file reads/writes and custom locks with the
  existing Agent-owned Document update/CAS path. Each version contains the
  complete Canvas ciphertext, including unknown fields, encrypted under the
  conversation scope key. Existing conversation holds and crypto-erasure
  therefore cover notes and markup without adding an identity or scope.
  Document collection owns the ciphertext alongside its version in the
  Agent scope, so pruning does not strand separately granted snapshots;
  conversation ledger grants stay separate.
- Preserve Canvas revisions and stale-response shapes while retries happen
  through Document CAS. Failed validation and oversized updates write no
  Canvas head. Existing plaintext files convert once under the historical
  file lock; delete the source only after reading back the identical sealed
  Document and checking the writer fence. Divergent or failed conversions
  refuse and leave the source available; no permanent plaintext fallback.
- Focused validation passes: five Canvas storage tests, three Document
  tests, restore/fencing with scoped content, and real-router erasure.
  The first full gate passed formatting and Clippy, then failed an unrelated
  smoke test whose fixed port was occupied. That test now requests an
  available port and reads readiness before connecting. The full gate rerun
  passes: formatting, workspace Clippy, workspace tests (935 seconds), version,
  document-path, record and advisory checks. Canvas browser recovery checks now pass (retention, visible
  recovery, retry, acknowledged cleanup). The general gallery still fails
  chart/map fitting at 260px. Real-router erasure coverage
  saves a note, retains its encrypted Document, erases through preview/hold/
  confirmation, and reads both the content and route back afterward. This
  does not persist unacknowledged client operations or settle per-contributor
  drafts, saved-page resource authorization, or live browser acceptance.

### Reporting contract

For each slice record finding IDs, exact baseline, reproduction, chosen
change, focused tests, broader required checks and remaining limitations.
Run the repository gate before a code commit as required by its contract;
client changes require both bundles and running-app acceptance. A build or
typecheck does not close a visual item. Environment-blocked live acceptance
stays open with the reason; do not substitute a pure-state probe for it.

Update the tracker, relevant design Status/records and user-visible
CHANGELOG in the same implementation commit. Keep exactly one current
handover. Close a stage only when every listed exit holds or an explicit
maintainer decision removes a scoped item. The original planning-only
commit changed no runtime behavior. Implementation slices do not change
the data baseline, release, install or deploy unless separately recorded.
