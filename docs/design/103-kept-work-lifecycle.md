# 103 — Kept work: create, change, run and use

Status: proposal — design revision requested by the maintainer on 2026-10-22.
No implementation stage has started. This is the composition contract for
kept artifacts, execution, connected capabilities, recurring work and Canvas.
It replaces the equation of a kept artifact with a worker in
[Workers](100-workers.md) §3.4. Worker implementation remains deferred;
public hosting, remote placement and enterprise identity remain deferred.
The implementation sequence and exit evidence are in
[the kept-work plan](../plans/kept-work-plan.md).

## 1. Product decision

A person may ask Vakyartha to make something, change it, use it, run it once,
or keep it operating. These are independent capabilities of the same work.
The thing remains an artifact; the Agent owns its operation. A worker is an
optional way to undertake work on it, not the identity of the thing itself.

Use the smallest composition that fulfils the request. A file needs no server;
an interactive local calculation needs no schedule; a refresh job needs no
permanent process; a running service needs no model call on each request.
Several capabilities may coexist, and none implies the others. There is no
document-to-app ladder that changes identity, and no domain classifier that
chooses an architecture. The main model resolves the request using available
capabilities; the runtime validates the proposed operations and their receipts.

The design must serve arbitrary work: passive deliverables, local tools,
connected views, batch transformations, scheduled publications, interactive
editors and services. Domain semantics, layouts, source interpretation and
special calendars belong in extension definitions or versioned programs.
The host owns identity, admission, execution lifetime, authority, storage,
receipts and presentation access. It does not recognise stock markets,
weather, projects or any other subject to decide how work runs.

This extends the [Agent-owned product model](64-agent-owned-platform.md),
[Library](82-library.md), [task environments](54-task-environments-and-promotion.md),
[Canvas](66-immersive-artifact-canvas.md) and [scheduled work](94-scheduled-work-you-can-see.md).
It introduces no second artifact store, scheduler, permission system,
execution broker, conversation hierarchy or UI runtime.

## 2. Why the current contracts lose the request

> **Historical record** — local investigation on 2026-10-22.

The owner asked for code, a connected dashboard and recurring updates. The
accepted plan listed a code file and an integration check. The agent created
a standalone client with missing connection settings, an in-process timer
and memory-only results. It later started it through Bash with `nohup`, then
emitted a file-preview card for the HTML. The immediate HTTP check succeeded;
the next turn found no listener. It queried installed Canvas workspaces and
treated an empty result as evidence that no live app could be served.

The following are distinct contract defects, not a verdict about a model:

- Bash deliberately kills remaining process-group children after command
  completion (`crates/vak-tools/src/bash.rs`). It cannot own a kept service.
- Managed previews already have start/stop/logs, view leases and a pinned
  lifetime (`crates/vak-server/src/lib.rs`); that is not a durable operation
  contract exposed consistently to the Agent, CLI and UI.
- `emit_file_preview_card` calls itself a live preview, but a file preview
  does not select the generated application's backend. Automatic document
  previews block page scripts; explicit page-code permission is separate.
- `workspace_read` lists declarative installed workspaces and saved resources.
  It is not a discovery API for executable artifacts or managed processes.
- A preview's listening allowance does not grant outbound network access.
  Starting it cannot manufacture a connected MCP capability.
- Source files and a successful command can satisfy delivery receipts while
  leaving operation, connectivity and recurrence unproved.

The rest of this document specifies the replacement contract. It does not
claim those replacements are shipped.

## 3. One identity, several independent descriptions

| Concern | Canonical owner and reference |
| --- | --- |
| What the person keeps | Existing Library artifact id; immutable versions and their real parents |
| Who acts and who can see it | Existing Space, Agent, conversation audience and grants; keeping does not transfer ownership |
| How it is operated | One versioned pack Document owned by its Agent, referring to exact artifact versions |
| Why something runs | Existing Trigger and claim, or an admitted human/Agent operation; every attempt opens an existing Run |
| Where code runs | Existing environment and broker, keyed by RunId, with an admitted generation and policy |
| What may be reached | Existing capability registry, connected accounts, grants, credential store and permission engine |
| What it shows | A typed Canvas subject/view referring to the artifact, resource or admitted running instance |
| What happened | Existing run, execution, effect, review and delivery records; projections over these records |

There is no new app id beside the artifact id, no `piece.toml` beside the
pack, and no per-app Agent or mandatory per-artifact conversation. Continue
working follows the Library's Agent conversation. A worker, when built,
references the same pack and artifacts; its own conversation follows doc 100.
It does not copy their schedules, credentials or desired state.

An artifact's kind describes its content, not how long it runs. Saving,
sharing, viewing, editing, running and scheduling remain separate operations.
Sharing a view grants neither execution nor the connected account behind it.
An installed workspace contribution remains a plugin capability, not a
required representation for every generated app.

### 3.1 The pack is the sole operation description

Extend the proposed pack in doc 100 §4 rather than introduce another recipe
store. Logical fields below are design targets, not existing Rust types.
No generated folder or filename constitutes registration or authority.

| Part | Required meaning when present |
| --- | --- |
| Subject | Artifact id, exact code/input versions, owning Agent/Space and originating conversation |
| Inputs and parameters | Typed values, resource references and lineage; defaults do not grant access |
| Execution | Declared entry points, arguments, required capabilities, preparation recipe and pinned dependencies; one-shot or service lifetime |
| Bindings | Named capability operations/resources, input/output schemas, account provenance and permitted scope; secret references only |
| Views | Optional registered renderer or isolated executable view, its subject and the resource/instance it actually reads |
| Duties | References to existing Triggers, the entry point each invokes, overlap/catch-up policy and optional freshness requirement |
| State | Declared durable resources, owner, lineage, schema, retention and snapshot/restore requirements |
| Checks | Readiness, connectivity, result and view checks with evidence strength; each names what it proves |
| Limits and authority | Existing admission/grant references, budgets, resource limits, approved effects and policy generation |

Code, a view and a model step are optional. A pack can operate an existing
capability without inventing a script. Deterministic code runs without a
model; model steps use the normal logged agent loop when judgement is needed.
External deployment is a reviewed effect through an available adapter, not
an implied built-in cloud or public-hosting service.

A pack may have several entry points, views and resources, with explicit
dependency references. A multi-process app is not forced into one process;
each admitted instance has its own Run, readiness and cancellation receipt.
Preparation validates dependencies and cycles before starting work. Failure
propagates only to declared dependants; unrelated outputs remain usable.
Shared resources are referenced by identity and granted scope, never copied
into each pack or discovered by walking a workspace. Merely previewing an
existing repository still needs no kept pack unless persistent operation
was requested.

## 4. Lifetimes, desired state and observed state

These lifetimes are deliberately independent:

| Lifetime | Ends when | Must not imply |
| --- | --- | --- |
| Turn/tool invocation | Its admitted work returns, cancels or fails | Its child process is a kept service |
| View | The person closes it, navigates away or loses the view lease | The artifact, duty or explicitly kept service is deleted |
| Run | This attempt settles, including interruption | A recurring duty is removed |
| Service instance | Its process exits, is stopped, revoked or fenced | The saved artifact is gone or its data is current |
| Duty | It is paused, removed or its owner loses authority | Saved results are erased |
| Artifact | Existing archive/retention/erasure operations apply | Stopping execution erases content |

Desired state records the approved pack version, enabled duties and requested
service lifetime. Observed state records actual admitted generations,
instances, readiness, last check and result receipts. A level-triggered
reconciler compares them; events only accelerate it. Desired `running` is
never displayed as observed `running` before readiness is established.

A view-only preview retains today's lease-and-release semantics. An explicitly
kept service is owned by the operation record, not a Canvas tab or Bash call.
Both use one brokered service-lifetime mechanism, with different admitted
owners; implementation replaces the current process map as lifecycle truth
rather than adding a second manager beside it. Operational lease expiry,
deadlines and resource limits are authority bounds, not a content retention
rule or a timer that declares the user's goal complete.

Each service incarnation opens a Run before starting and records its exit.
Process handles/ports are ephemeral. After a server crash, the old run is
interrupted or reconciled; a PID is never sufficient proof of ownership.
An orphan must be identified and terminated or safely reattached through
the broker's verified instance identity before a replacement may start.
Only an enabled, approved operation may restart, under the current policy,
spend gate and fencing epoch. Do not promise in-memory continuation.
Restart/backoff uses shared reliability controls, never an app timer loop.

Two surfaces join the same admitted instance for the same subject and
execution version. Different artifacts, revisions or authority scopes cannot
alias because their labels or port numbers match. Instance claims use fenced
CAS under the same operation service; they are not a second trigger claim.
Start and restart causes still pass through the existing trigger/run model.

## 5. One operation service, several callers

The Agent, shared client, CLI and automation runner use the same typed core
operations. A bounded model-facing interface exposes discovery/inspection,
prepare, run once, start/stop service, manage duties, propose a revision and
read receipts. It does not grow a tool per domain, framework or provider.
HTTP and UI adapters do not implement a parallel lifecycle.

Every mutation binds the subject, expected pack revision, exact executable
version, current capability/policy generation and actor. Concurrent writes
use Document/ref CAS. Conflicts return the current revision and require a
fresh decision; starting an old command after disable is refused. Inspect
returns separate inventories for artifacts, installed workspace types,
bindings, duties and instances so an empty inventory cannot answer a different
question. Discovery reports available, unavailable, denied and unknown with
the reason; a failed probe never invents a callable capability.

Inspection and turn-path decisions read current refs or verified rollups,
not a replay of historical chains. Explicit reads and selected state sent
to the model are recorded through the existing tool/context ledger contracts;
opening or mounting a view does not silently admit its content into a prompt.
Direct human edits use the same revisioned command/state service, retaining
Canvas checkpoints, conflicts and undo where that resource supports them.
Visual selection and discussion do not widen execution or account authority.

The model's runtime contract explains what can outlive a tool call, how to
register/inspect operation, and which presentation opens the actual running
subject. File/markup preview descriptions say that they render documents;
they do not advertise a connection to an app backend. A typed running-view
action binds an admitted subject/instance, never an arbitrary model URL.
Opening remains a person's action; a completed tool does not hijack Canvas.

Authority is the intersection of existing owner intent, grants, environment
policy, channel ceiling and current capability state. A clear bounded request
can authorize the operation within those rules; do not require a ritual
"keep" command or ask again for authority already granted. Required Review,
scope expansion, irreversible effects and mandatory provenance asks remain.
An ambiguous duration or destination is resolved before granting it, while
independent creation/checking can proceed. Unattended work cannot wait for a
person who is absent: record the blocked/denied attempt and surface it.

### 5.1 Stop, pause, retire and revoke

- Cancel ends the selected Run and preserves partial output.
- Stop service records stopped desired state before terminating its instance,
  so reconciliation does not restart it. It does not silently disable a duty.
- Pause duty prevents new claims; the operation explicitly names whether it
  also cancels an in-flight run. Show any still-running attempt.
- Stop all operation disables duties and services and cancels active runs;
  the UI says what it will stop. Retire uses this barrier and keeps saved work.
- Permission/account/plugin revocation cancels affected work, invalidates
  handles and pending approvals, and closes bound connections before reporting
  success. No stale receipt or view lease restores the revoked authority.
- Archive, trash, erasure, sync takeover and restore respect their existing
  owner lifecycle and writer fences. No app implements its own deletion rule.

### 5.2 Everyday use and free-form changes

The person describes what they want to do. Vakyartha prepares it, offers one
clear Open/Use action and explains only the decisions the person must make.
Source filenames, launch commands, ports, environment variables, tool schemas,
package installation and raw test output are not the everyday hand-off.
Technical details remain available for inspection. Necessary authority choices
use plain language about what will run and what it can reach; authorship does
not establish trust, and simplifying the wording never silently enables code.

An interactive result is not ready because its HTML saved or syntax passed.
The intended controls and data must work in the actual delivered view. When
execution needs consent, the ordinary Open/Use flow explains and requests it
at that boundary; an inert page must not be advertised as working. Do not
leave the person to diagnose blank controls or discover a hidden technical
permission panel. State and recovery must use admitted durable resources,
not depend on a temporary preview origin's browser storage.

A normal follow-up such as a layout change or an added field resolves to the
existing artifact from conversation context, selected work and supported
recall. It creates a version of that artifact, based on the exact version
read. It does not create a new artifact because a new turn began, a title
changed or the model chose a new output filename. The Library shows one card
per artifact with its current state and version history; a revision updates
that card. Its latest-version metadata and open view must reconcile when
the version changes, while an explicitly selected older version stays selected.

Creating a separate copy or independent deliverable is a different semantic
choice: create a related artifact only when that is what the person requested.
No keyword list decides edit versus copy. Ambiguous targets need a plain
choice between the actual things; nobody must supply an id, path or version
number. Pending human edits and concurrent versions remain explicit conflicts,
and new work never silently rebases onto the newest head.

## 6. Connections, effects and exposed views

Discover the owner's existing capabilities and their actual schemas before
proposing bindings. An absent connection is a setup requirement; do not
invent an endpoint or substitute another provider silently. A pack binds a
capability identity and operation/schema, not whatever tool happens to have
a similar name after an update. Revalidate on capability generation changes.

Generated code receives narrowly scoped broker operations/resource handles,
not the parent's environment, raw provider secrets, gateway bearer or an
authenticated control-plane client. A handle names owner, subject, revision,
audience, operation, limits and expiry/policy generation; it cannot name an
arbitrary URL, command, account or another artifact. The trusted broker
resolves it through the same permission/MCP/intake paths as ordinary tools.
This is an execution adapter to existing capabilities, not a second MCP
manager, egress gateway or tool runtime. If the backend cannot enforce a
required boundary, the binding is unavailable with a reason.

Read results retain account provenance and source lineage in storage and
model-visible tool results (`ToolOutput::from_account`). Untrusted external
bytes enter through the existing intake/parser boundary. An outward mutation
is prepared/dispatched/settled through existing Effects. Unknown outcomes
are reconciled, never replayed by generated retry code. No request handler
gains authority merely because it runs inside an approved service.

Mandatory provenance policy applies to both code and model execution. A
private/untrusted combination that requires a human cannot be relabelled,
split into a fresh process or laundered through saved state to avoid the ask.
The existing explicit FullAccess rules still apply; no automatic escalation
or new unattended exemption is part of this proposal.
If that policy prevents a requested unattended pipeline, report it as
unsupported under the current authority. A positive unattended acceptance
must establish why every step is admitted; setup success is not permission
for future private/untrusted exits or mutations.

Static document viewing, explicitly permitted page code, installed declarative
workspaces and running app views retain their distinct isolation contracts.
An executable view runs on an isolated origin and receives only its own
scoped data/actions, never the app's cookies or native IPC. CORS, CSRF,
WebSocket, navigation, Host and DNS-rebinding checks cover the real transport.
UI code cannot broaden network access. Preview listening permission is not
egress permission, and a successful broker binding does not authorize arbitrary
browser fetches. Authenticated mutation actions re-enter normal admission.

The running instance is on the Vakyartha home's machine. A loopback URL is
usable only there; a remote browser must receive a truthful unavailable
reason until an authenticated isolated preview transport is implemented and
qualified. No wildcard bind, port forwarding or public route is inferred.
Public deployment, inbound webhooks and remote workers stay separate scope.

### 6.1 Script admission is a capability decision

Neither the presence of a script nor the fact that Vakyartha generated it
decides whether it runs. The main model proposes the computation and required
capabilities; trusted admission checks the exact version, existing authority
and what the selected execution backend can actually enforce. Script counts,
source location, static scans and a model's safety judgement are evidence,
never grants or proof that arbitrary code is safe.

| Requested behaviour | Admission requirement |
| --- | --- |
| Inert content, including non-executable JSON script data | Render without inventing execution permission |
| Local computation or interaction | Run within an admitted, enforceable confinement/resource profile; no implied network, host files, account or native access |
| Connected read | Bind the named capability and its permitted source through the existing broker; do not grant general browser network access |
| Stored edits or external mutations | Existing scoped state/command/effect operations, current policy and required Review/provenance gates |
| Unsupported or uncertain enforcement | Explain the missing boundary and use a supported execution adapter if available; never claim offline execution or silently widen access |

A request for an interactive result can already authorize the bounded local
operation. Do not require another technical consent ritual when existing
authority and qualified confinement suffice. A required new authority decision
is still presented to the person in plain words. A model judgement cannot
replace that decision, select FullAccess or dismiss a backend limitation.

In particular, the measured WebRTC escape in doc 66 means that CSP denying
fetch alone cannot qualify arbitrary browser scripts as offline. The new
runtime must establish an enforceable profile or use another isolated adapter;
until then, the current explicit page-code path retains its disclosure and
human-choice boundary. This proposal neither removes existing protection in
advance nor enshrines blanket script pausing as the desired end state.

Admission is bound to the executable/version and capabilities. A revision
revalidates the changed program and requested scope; an old permission cannot
authorize extra capabilities in new bytes. An unchanged existing grant need
not be requested again. Revocation, reload, popout and restoration must obey
the chosen lifetime without losing durable work or silently restoring trust.

## 7. Recurrence, events and freshness

All recurring starts use existing Triggers, `triggers::claim_due`, Runs and
doc 94's time rules. Duty references do not copy next-run/last-run fields
into a competing schedule. Interval anchors survive restarts; DST,
sleep/catch-up and overlap are explicit existing policies. Run now and
stale-on-open requests use the same claims/admission, not side locks.

Registration is not evidence that a runner serves the duty's Space. Before
advertising an armed duty, resolve its workspace binding and the responsible
live runner; show a missing/stale runner as unavailable with a recovery action.
The person must not know to start another server for a subfolder. Reconciliation
and catch-up remain in the existing scheduler contract, with explicit bounded
coverage rather than silent omission of duties outside one server's Space.

Domain-specific eligibility is an extension-produced bounded input/check
over a versioned source, recorded with its as-of time. The host interprets
typed eligible/ineligible/unknown results; it does not embed calendars,
weekday guesses or topic words. Unknown eligibility leaves a visible gap,
not an invented open/closed state. An extension can supply additional
bounded Trigger policy data through the shared scheduler contract; it cannot
start a competing timer or scheduler. Stream positions remain existing cursors.

An in-process timer is fine for animation, local UI refresh or a protocol
heartbeat under its admitted lifetime. It cannot implement a promise to
perform durable recurring work after the process/turn/view ends. No model
call is required to rerun approved deterministic code.

Status is a projection with independent facts: saved/reviewed version,
preparation/readiness, observed instance health, binding availability,
duty enabled/paused/next eligible slot, last attempt outcome, last good result,
source as-of time and freshness requirement. A listening process can have
unavailable data; a stopped process can have a valid saved result. Fetch time
does not prove source freshness. No default freshness promise is invented.
UI uses plain words and shows unknown where no receipt exists.

Result publication, conversation delivery and notifications are distinct.
The approved duty names its delivery target and notification policy through
existing delivery operations. Updating a saved value does not imply an
unsolicited message on every tick; routine success may stay in the result/status
view. Meaningful failure, missed freshness and required human action remain
visible through existing operations/inbox paths. This is a proposed duty
policy, not a claim that current automation notification defaults changed.

## 8. Durable state, results and revisions

Use existing Documents for bounded typed state and tenant objects/Library
versions for large payloads and deliverables. Declare every resource's real
kind in the state registry, resolve it through scope/path accessors, index
searchable records in the catalog and honour Audience, grants, holds and
erasure lineage. A program never receives direct write access to a data-home
ledger, credential store or Document object scope.

Arbitrary mutable databases are not implicitly supported by the Application
class mentioned in doc 73. An adapter needs fenced writes, consistent encrypted
snapshots, backup/restore and sync participation, source/contributor/account
lineage, retention and erasure before it can advertise durable support.
Otherwise use the supported state adapters or report the missing capability.
Temporary working files remain ephemeral; an unregistered file is not a backup.

Keep executable revision, input revision, mutable-state revision and result
version distinct. A successful refresh creates an immutable result with those
references and advances the last-good ref only after checks pass by CAS.
Concurrent/stale attempts cannot replace a newer accepted result. The pack
declares whether this output is its own approved refresh target; it cannot
overwrite an owner's unrelated file. A failed refresh retains dated last-good
data, partial output and its failed receipt.

A change produces a candidate based on the exact version inspected. Review
uses artifact id/version and existing `file_state`/Library operations. A
runtime revision does not silently activate because source files changed.
Prepare and check the new revision in isolation; switch the approved active
ref only when readiness and state compatibility are established. Old and new
instances must not write shared mutable state concurrently without an admitted
fenced adapter. Failed activation leaves the prior valid revision available
or reports an explicit stopped state if a safe switch is impossible.

Rolling code back does not roll back database state or undo external effects.
A state schema change needs an explicit adapter operation, snapshot and
rollback limits reviewed with the candidate. Unknown compatibility refuses
activation. There is no general-purpose migration runner or old-schema reader
added for compatibility; current-contract replacement names unsupported data
without rewriting or deleting it.

## 9. Completion is observed, not presented

Extend the existing plan/managed-criteria/receipt contract in doc 86. Delivery
form remains how an answer reaches the person. Add optional typed operational
requirements to a deliverable for its requested lifetime, bindings, subject,
readiness, recurrence and freshness; these are orthogonal to card/file/text.
The main model declares them from meaning. The runtime never detects an
"app request" from keywords or turns all coding work into managed operation.

The operational interface and plan instructions must make lifetime, binding
and recurrence obligations expressible on the initial request and on later
changes. An unavailable capability stays an unmet requirement; a replan cannot
quietly turn a requested operating result into files and call it complete.
Once accepted, dropping a requirement needs an explicit recorded scope change,
not a failure-recovery shortcut. As in doc 86, receipt checks prove the declared
requirements, not that the model captured every part of natural-language intent.
Undeclared intent remains semantically unverified. Acceptance must examine
whether the main model preserves the original request across paraphrases and
follow-ups, as well as whether the runtime enforces what it declared.

Runtime checks resolve requirements to the exact subject and revision:

| Claim | Minimum evidence |
| --- | --- |
| Saved | Artifact/version persisted and readable through the normal path |
| Prepared | Environment/dependency receipt for this executable revision |
| Running | Broker-owned instance still alive after the initiating tool returns, with successful readiness bound to its identity |
| Connected | Actual permitted capability call and schema-valid response for this binding, with source/provenance; a catalog listing is insufficient |
| Usable in Canvas | The intended subject loads on the intended surface and reaches its actual resources/actions; a file screenshot is not a backend check |
| Scheduled | Durable enabled Trigger/duty binding with zone/anchor, claim policy and next eligible evaluation; not a successful timer installation |
| Updating | Observed claimed run through the real runner publishes a checked result; live cadence needs later slot evidence, not just the initial run |
| Fresh | Source as-of evidence satisfies the declared requirement; otherwise stale/unknown |
| Recoverable | Crash/restart evidence for run settlement, instance reconciliation and durable state; a happy-path check is insufficient |

Bounded setup completion means the requested operation is admitted and its
checks have the stated evidence. It never certifies indefinite future success.
A scheduled future slot may be armed but not yet exercised; report those facts
separately rather than keeping the turn open until the future or saying it
has already updated. Semantic quality remains assessed at its actual evidence
strength; process health does not certify the result's meaning.

Receipts, status snapshots and logged observations are shared by cards, Canvas,
Library, Operations, CLI and channels. A card cannot make a process live or
make data current. Missing or failed requirements follow the existing
correction ladder and preserve useful partial output, never a new completion
loop or an automatic background commitment.

## 10. Replacement boundaries and implementation authority

This is a design change only. No Worker stage is reopened for implementation,
no existing unattended process is adopted, and no local generated app is
registered by this document. The maintainer's request authorizes this design
and plan; implementation is separate work.

When implementing, replace each superseded path in its slice: process-lifetime
truth, operation controls, misleading preview descriptions and receipt checks.
File previews, direct answers, existing automations and installed workspaces
remain useful independent capabilities, with their existing authority.
There are no dual app managers, compatibility readers, guessed folder scans
or topic-specific success exceptions. The stage plan requires varied workloads
and real failure paths before a lifecycle claim may be marked shipped.
