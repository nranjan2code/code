# Plan — Kept work lifecycle

Status: proposal — design requested 2026-10-22; no implementation stage
started or authorized by the design-only request. The contract is
[Kept work](../design/103-kept-work-lifecycle.md). Workers remain deferred
under [their plan](workers-plan.md); the operation foundation below can serve
ordinary artifacts without first implementing durable model workers.

## Rules for every stage

- Preserve the architecture table in
  [task environments](../design/54-task-environments-and-promotion.md).
  Use existing artifacts, Documents, triggers/claims, runs, effects, grants,
  capability registry, broker, environment, catalog and lifecycle services.
- No domain dispatch, model/provider names, framework success shortcuts,
  folder-name inference, app taxonomy or second schedule/store/authority.
  Adapter capabilities are discovered data. No mandatory model work for a
  deterministic run and no mandatory service for a passive deliverable.
- Replace superseded runtime paths and their tests/docs in the same slice.
  No migration/compatibility implementation for an unshipped design.
- Human UI, Agent operations, CLI and automation runner share core mutations
  and projections. Scope every mutation to the current subject/revision and
  actor, and verify current authority at dispatch and publication.
- Test in disposable workspaces. Do not adopt, start, change or erase the
  owner's generated apps/data as an acceptance shortcut.
- Each stage records what is observed versus asserted, its failure cases,
  and unqualified platforms. Shared UI changes need 1440 × 900 and 390 × 844,
  light/dark, plus actual installed-shell qualification. A fixture/build alone
  does not close the installed-route gate.

## K0 — Types, ownership and read projections

Scope: the one pack Document and its artifact-version references, independent
desired/observed status, resource declarations, typed operation receipts and
optional operational plan requirements. Reuse doc 100's pack contract; do
not build Worker conversations, fan-out or caretakers to make K0 work.

Done when: the schema and authority table round-trip through existing sealed
Documents/refs; invalid ownership/revisions/unsupported fields fail without
mutation; catalog search/audience and trash/erasure lineage are verified;
delivery receipts cannot satisfy an operational requirement. Ordinary file,
text/card, code-check and installed-workspace requests remain on their existing
paths. No source filename or topic activates a pack.
Replanning cannot silently drop an accepted operational requirement after a
failed probe; an explicit recorded scope change is required. Do not claim
structural receipt checks establish complete understanding of the request.

## K1 — One execution-lifetime and operation service

Scope: inspect, prepare, run once, start/stop service and read receipts across
the Agent, UI and CLI. Replace the current preview process map as lifecycle
truth with the shared broker-owned operation service. View leases and explicit
kept services are admitted owners of the same mechanism. Preparation goes
through existing scoped setup, dependency and environment paths.

Done when: one-shot code completes normally; a kept service survives the
initiating tool/turn and Canvas closure; a view-only preview releases normally;
two surfaces join one instance; stop records desired state before termination;
restart settles interrupted runs and reconciles orphan instances without
duplicate writers. Startup races, occupied ports, wrong revisions, missing
runtimes, dependency failure, process crash, cancellation, spend failure and
writer fencing produce typed status and preserve partial output.

Gate: actual broker and real child processes, including the existing Bash
descendant-cleanup guarantee. No `nohup`, detached-shell escape or FullAccess
escalation is an implementation of service ownership.

## K2 — Bound capabilities and isolated running views

Scope: discovered operation bindings, narrow broker handles, account
provenance and real view-to-resource/action routing; one typed running-view
action. Correct file/markup preview wording and execution instructions.
Keep static preview, explicit page code and installed workspaces distinct.

Script admission follows doc 103 §6.1: no blanket script ban in the target
runtime and no automatic trust of generated code. Qualify the actual backend
before running within existing authority; static/model judgement alone cannot
grant capabilities or establish offline isolation.

The script-specific gate must cover inert JSON, embedded scripts, event
handlers, local modules/assets, external references and isolated calculations;
allowed local interaction and denied undeclared effects; fetch/WebSocket and
WebRTC/private-destination attempts; cookie/credential/native isolation;
clipboard/download/popout boundaries; runaway compute and cancellation;
reload/close/reopen, edits and authority revocation; and Canvas/Library/window
parity. Record each supported backend/engine's actual outcomes. Missing engine
coverage remains unqualified, not inferred from another engine's success.

Done when: generated work calls an admitted capability through its actual
schema and shows the returned data on the intended Canvas subject. Missing,
revoked, schema-changed, denied and transiently unavailable bindings are
distinguishable. Arbitrary handles/URLs/commands, cross-artifact access,
secret/environment inheritance, CSRF, rebinding, WebSocket and origin escapes
are refused through the real authenticated routes. Permission/plugin/account
revocation closes current access before success is reported. The existing
private/untrusted provenance ask still applies unattended.
Where it requires a person, the unattended pipeline is explicitly unavailable;
a positive case must establish actual admission of every step, never assume
that the pack is an exemption or silently select FullAccess.

Gate: one connected read-only view, one offline interactive program and one
mutation that reaches existing effects/Review, on the installed route. No real
outward send without its separately authorized review. A remote browser gets
either a qualified isolated transport or an explicit unsupported reason;
loopback on the home's machine is never advertised as remotely reachable.

## K3 — Durable duties, results and honest freshness

Scope: pack references to existing Triggers and permitted entry points, saved
results and last-good CAS, extension-supplied eligibility, status and delivery.
Existing automations remain the scheduler; existing time/catch-up semantics
remain authoritative. No worker-only duplicate is introduced later at W2.

Done when: closing the view and ending the original turn leave an enabled
duty intact; a real later claimed slot updates the saved result without a
model for deterministic code. Sleep/restart, DST, overlapping attempts, a
slow source, unknown eligibility, missed slots, offline source and stale
publication races have recorded outcomes. Fetch time and source time are
distinct; a failed check preserves dated last-good data. Pause, resume,
cancel, stop service and stop all operation have separate verified effects.
Limits on execution do not become content deletion rules.
Routine result updates and notification policy are exercised separately, so
refreshing data does not force an unsolicited chat message on every slot.
Duty admission must verify that a live runner serves the bound Space. Test
the home's workspace and another bound folder: creating an enabled Trigger
with no responsible runner must not be presented as successful operation.

Gate: same mechanism exercised by a scheduled document, a connected view and
a headless batch with no view. Fixed-clock tests establish slot boundaries;
installed real-slot runs establish runner/publication behaviour. Report
armed-but-not-yet-exercised duties separately from observed cadence.

## K4 — Revision activation, state recovery and lifecycle

Scope: immutable executable/result versions, isolated revision preparation,
reviewed activation and rollback, supported state adapters and existing
backup/restore/sync/retention/erasure integration.

Done when: a failed revision does not overwrite a valid active result; stale
edits cannot activate; concurrent revisions cannot share an unfenced writer.
Rolling back code does not pretend to revert state or effects. Unsupported
state compatibility blocks activation with its recovery options. Restore
reapplies erasures, fences the old process and reconciles only approved
operation. Holds, trash/archive, Agent removal and account erasure reach the
right resources. Mutable adapters are not advertised until their consistent
encrypted snapshots, writer fencing and lineage erasure pass.

Gate: isolated process binaries for whole-home restore/sync/erasure tests,
and actual installed revision/recovery flows. No direct data-home copy or
ad-hoc deletion; no app-specific backup path.

## K5 — Complete-flow and preservation gate

Run these as independent user requests, in mixed order and across follow-ups:

| Work | Must prove |
| --- | --- |
| Static document or report | Saved/viewable without a service, schedule or Worker |
| Offline interactive calculation | Local interaction without connector/network authority |
| Existing repository app | Uses its real environment and declared setup; no root/subfolder guessing |
| Connected periodically refreshed view | Actual binding, backend, later slot, last-good data and source freshness |
| Batch transformation without UI | Runs and produces versioned outputs without Canvas or a permanent process |
| Long-lived local service | Explicit lifetime, bounded resources, stop/restart and no unsolicited exposure |
| Installed specialized workspace | Existing commands/checkpoints/undo/Ask Agent still work independently |
| Free-form edit of existing work | Same artifact id, new version with the real parent, one Library card and preserved history |
| Explicit independent copy | New related artifact only when requested, without replacing the original |
| External mutation or deployment | Existing Review, effects and unknown-outcome reconciliation |
| Read-only request or channel-only conversation | Does not acquire execution authority or require the desktop |
| Unrelated ordinary conversation | No forced pack, app tool, recurring commitment or classification step |

For each supported capability, exercise missing preparation, revocation,
failure after an earlier success, conflicting revisions and restart. Include
non-Git folders, multiple artifacts under one workspace, multiple Agents,
multiple surfaces and an absent extension. These cases probe the contract;
they are not a built-in domain list or a claim to exhaust every scenario.
Include multi-process dependency failures, human edits concurrent with Agent
changes, and natural-language paraphrases/follow-ups that preserve the requested
lifetime instead of reducing it to file creation. Inspect both accepted plans
and linked runtime receipts; success in only one is not complete acceptance.
Exercise the actual default Open/Use flow with an everyday-language prompt:
usable controls/data, understandable consent, no blank interactive result
advertised as ready, and no commands/env/schema setup handed to the person.
Keep the Library open during revisions: reconcile latest metadata and content
on the same card without a manual reload; retain an explicitly selected old
version and any unsaved edits. Test real behaviour, not HTML marker assertions.

Done when: runtime requirements and the final delivered claims agree on the
installed route; all superseded runtime paths are removed; existing broad
checks pass; measured evidence and remaining platform limits are recorded.
Public hosting, remote worker placement, inbound guests/webhooks, arbitrary
mutable stores and full specialized Canvas layouts are separate gates, never
silently included by calling this lifecycle complete.
