# Architecture and presentation hardening

Status: in progress — requested on 2026-10-18 to address all observations from
the session's architecture, answer/projection, surface and dynamic-card
reviews. The first S1 slice implements strict grant/effect/connection reads
and atomic ref fencing; the full repository gate passes. Other stages
remain open. Unresolved product choices are not maintainer decisions.

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
maintainer decision removes a scoped item. This planning change itself
does not ship fixes, change a data baseline, release, install or deploy.
