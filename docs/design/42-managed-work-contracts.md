# 42 — Managed work contracts

Status: implementation complete for shipped managed-work paths. Managed agent turns dispatch contract-owned static flows through the production flow executor; standalone CLI flows remain deliberately unlinked unless a future explicit linkage option is added. Outcome revisions and collaborative goal updates are recorded by the shared outcome-directed runtime (`docs/design/52-outcome-directed-runtime.md`), while this document remains the durable work-item contract.

## Mission

Make complex Vak work explicit, durable, inspectable, resumable, and
evidence-backed without slowing down ordinary conversation.

Given a complex request, Vak must be able to answer, from durable state:

1. What did the user ask for?
2. Which constraints and assumptions were understood?
3. What work items were created?
4. Which item is ready, running, blocked, delegated, or complete?
5. Which parent agent, worker, flow, tool, or human owns each item?
6. What evidence supports each result?
7. Why is the overall work complete, incomplete, failed, or awaiting input?

This document is an implementation contract for long-running general-purpose
agent work, including coding, research, writing, analysis, and operational
tasks.
An implementing agent must continue through every phase, preserve the
invariants below, run the required verification after each phase, and leave a
clear handoff if a phase cannot be completed. Do not mark the mission complete
because the types or UI exist: the end-to-end behavior, recovery behavior, and
tests must pass.

## Non-goals

This work must not:

- replace the existing direct agent loop;
- route every message through a planner;
- create a project-management record for every simple answer;
- give workers broader permissions than their parent;
- make model-authored text the source of progress truth;
- allow a model to mark work complete without verification;
- create a second durable store that competes with the session ledger;
- weaken gateway allowlists, permission checks, capability overlays, or
  brokered tool execution;
- add a dependency without an exact workspace version and justification.

Existing direct chat remains the default compatibility path. Managed work is
selected explicitly first and becomes automatic only after the evaluation
criteria in this document pass.

## Existing foundation to preserve

The implementation builds on:

- append-only session JSONL in `crates/vak-session`;
- `SessionLog::derive_messages()` as the model-context projection;
- the `vak-agent` model/tool loop and stop gate;
- `PermissionEngine` before every effectful tool execution;
- brokered and sandboxed tools in `vak-tools`;
- frozen session contracts and capability descriptors;
- child sessions and `WorkerRegistry` in `vak-agent/src/task.rs`;
- static flows and dynamic planner in `vak-flow`;
- goal mode and audited completion in `vak-agent/src/goal.rs`;
- checkpoints, receipts, activities, SSE, and presentation projections;
- server endpoints and desktop/admin surfaces already used for sessions.

Read the relevant existing design documents and source before changing code:

- `docs/design/03-agent-loop.md`
- `docs/design/08-permissions.md`
- `docs/design/10-flows.md`
- `docs/design/11-planner.md`
- `docs/design/22-gateway.md`
- `docs/design/30-output-engineering.md`
- `docs/design/33-admin-console.md`
- `docs/design/41-capability-registry.md`
- `crates/vak-session/src/types.rs`
- `crates/vak-session/src/log.rs`
- `crates/vak-agent/src/lib.rs`
- `crates/vak-agent/src/task.rs`
- `crates/vak-core/src/lib.rs`
- `crates/vak-server/src/lib.rs`

## Product behavior

```text
simple request
  → direct agent loop

complex request
  → contract draft
  → deterministic validation
  → resolve material ambiguity
  → activate work items
  → parent/tool/worker/flow execution
  → evidence collection
  → criterion verification
  → complete, continue, block, or ask
```

The parent session owns the user-facing conversation and final answer. A
worker owns only its assigned item and returns evidence to the parent. A
flow owns deterministic nodes. A scheduled task is a recurring automation.
These are different concepts and must remain different in names and APIs.

## Terminology

### Work contract

The durable, versioned interpretation of one user objective. It contains the
objective, constraints, assumptions, acceptance criteria, work-item
definition, and lifecycle status.

### Work item

One independently trackable unit of work. It has dependencies, an owner,
execution scope, status, and evidence. It is not necessarily a scheduled task
and it is not necessarily a worker.

### Scheduled task

The existing recurring `TaskDef` managed by the `tasks` tool and task store.
It fires later on a schedule. It is not a work item created for every turn.

### Worker task

The existing one-shot `task` tool call. It creates a child session and blocks
the parent tool call until the child finishes. The implementation must attach
the child to a work item when managed mode is active.

## Execution profiles

Add a run-scoped profile:

```rust
pub enum WorkMode {
    Direct,
    Managed,
    Auto,
}
```

`Direct` is the current behavior. `Managed` always drafts a contract before
effectful execution. `Auto` evaluates whether a contract is warranted.

Initial rollout requirements:

- default remains `Direct`;
- CLI supports `vak exec --managed`;
- server accepts `work_mode: "managed"`;
- TUI and desktop expose an explicit “track this work” action;
- `Auto` is initially disabled or equivalent to `Direct`;
- automatic activation is enabled only after the evaluation gate passes.

**Superseded for `Auto` (docs/design/47-commitment-kernel.md).** `Auto` no
longer evaluates a keyword heuristic. Managed admission is now a consequence
of the intent kernel's `horizon` axis: `session` and above run managed. The
`is_managed_work_request` scan this document's rollout depended on — thirteen
English verbs plus two conjunctions — is deleted; it read "explain what this
and that mean" as durable multi-step work. `Direct` and an explicit
`work_mode: "managed"` are unchanged, and an explicit run-scoped mode still
overrides the reading.

## Durable model

Add work types to `crates/vak-session/src/types.rs` or a focused
`crates/vak-session/src/work.rs` module.

```rust
pub struct WorkContract {
    pub contract_id: String,
    pub revision: u32,
    pub source_entry_id: String,
    pub objective: String,
    pub constraints: Vec<Constraint>,
    pub assumptions: Vec<Assumption>,
    pub criteria: Vec<WorkCriterion>,
    pub items: Vec<WorkItemDefinition>,
}

pub struct WorkItemDefinition {
    pub item_id: String,
    pub title: String,
    pub instructions: String,
    pub dependencies: Vec<String>,
    pub owner: WorkOwner,
    pub required: bool,
    pub readonly: bool,
    pub path_claims: Vec<String>,
    pub criterion_ids: Vec<String>,
}

pub enum WorkOwner {
    ParentAgent,
    Worker,
    Flow { name: String },
    Tool { name: String },
    Human,
}

pub struct Constraint {
    pub constraint_id: String,
    pub text: String,
}

pub struct Assumption {
    pub assumption_id: String,
    pub text: String,
    pub requires_confirmation: bool,
    pub resolution: Option<String>,
}

pub struct WorkCriterion {
    pub criterion_id: String,
    pub statement: String,
    pub kind: CriterionKind,
    pub required: bool,
}
```

Runtime state is separate from the definition:

```rust
pub struct WorkItemState {
    pub item_id: String,
    pub status: WorkItemStatus,
    pub attempt: u32,
    pub child_session_id: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub blocker: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}
```

Contract status must include `Draft`, `AwaitingInput`, `Active`, `Blocked`,
`Verifying`, `Completed`, `Failed`, `Cancelled`, and `Unverified`.

Item status must include `Proposed`, `Ready`, `Running`, `WaitingApproval`,
`Blocked`, `ReadyForVerification`, `Succeeded`, `Failed`, `Skipped`,
`Cancelled`, and `Interrupted`.

Use serde defaults for newly added optional fields so old ledgers remain
readable. Do not use `deny_unknown_fields` on session ledger entries where
forward compatibility is required.

## Ledger events

Add `EntryPayload::Work(WorkEvent)` to the append-only session ledger.

```rust
pub struct WorkEvent {
    pub contract_id: String,
    pub revision: u32,
    pub kind: WorkEventKind,
}

pub enum WorkEventKind {
    ContractCreated { contract: WorkContract },
    ContractRevised {
        previous_revision: u32,
        contract: WorkContract,
        reason: String,
    },
    ContractStatusChanged {
        from: ContractStatus,
        to: ContractStatus,
        reason: String,
    },
    ItemStatusChanged {
        item_id: String,
        from: WorkItemStatus,
        to: WorkItemStatus,
        attempt: u32,
        reason: String,
    },
    ItemAssigned {
        item_id: String,
        owner: WorkOwner,
        child_session_id: Option<String>,
    },
    EvidenceAttached {
        item_id: String,
        evidence: EvidenceRef,
    },
    AssumptionResolved {
        assumption_id: String,
        resolution: String,
    },
    VerificationRecorded {
        criterion_id: String,
        result: CriterionResult,
    },
}
```

Implement a pure projector:

```rust
pub fn project_work(entries: &[Entry]) -> Result<Option<WorkProjection>, WorkError>;
```

The projector must enforce:

- one active contract per parent session unless explicitly branched;
- strictly increasing revisions;
- valid item and criterion references;
- valid state transitions;
- dependency existence and acyclicity;
- no completion before required items and criteria pass;
- no model-authored transition directly to `Succeeded` or `Completed`;
- deterministic output after repeated projection;
- explicit warnings for interrupted or unreconciled runtime state.

The ledger is the sole durable authority. Runtime registries and UI caches are
indexes or control channels only.

## Model projection and compaction

The active work state must be visible to the model after every turn and after
compaction. Extend the session projection used by
`SessionLog::derive_messages()` rather than creating a parallel prompt path.

*(This section describes the phase as originally shipped. Since 3.5.0 the
`<work_contract>` block below renders in the per-turn tail rather than being
spliced into history, and compaction operates on whole turns rather than
individual messages — docs/design/68-context-engine.md §6, §10. The
projection contract — one bounded control message, survives compaction,
partition accounting doesn't double-count it — is unchanged; only where and
how it is assembled moved.)*

Project only one bounded control message for the latest active state:

```text
<work_contract revision="3">
Objective: migrate the configuration loader
Required items:
- inspect: succeeded
- implement: running
- verify: ready
Rules:
- do not claim completion while required items remain unfinished
- use the work tool for state changes
</work_contract>
```

The control projection must:

- be reconstructable from JSONL;
- be bounded in size;
- survive compaction while active;
- not be counted as dropped transcript history;
- disappear from model context after terminal completion;
- never contain secrets or untrusted raw tool output.

Update compaction partition accounting so control state is neither falsely
reported as dropped nor duplicated across repeated compactions.

## Contract authoring

For managed mode, author a contract through a dedicated structured provider
dispatch. **As built**, that dispatch lives in `crates/vak-agent/src/lib.rs`
rather than the separate `work_contract.rs` this doc originally proposed —
it is recorded with `WorkPurpose::Plan` through the normal work-receipt
machinery, which is the part that mattered.

The authoring prompt must request strict JSON containing:

- objective;
- constraints;
- assumptions;
- acceptance criteria;
- work items;
- dependencies;
- owner recommendation;
- readonly/path scope.

The runtime must validate the response independently. The model is not
trusted to validate its own contract.

Reject contracts with:

- empty objective;
- duplicate IDs;
- unknown dependencies;
- cycles;
- missing required criteria;
- unsupported owners;
- paths outside the workspace;
- invalid or excessive item counts;
- permission or capability escalation;
- unsupported tools or flow names.

Explicit managed mode fails closed on invalid authoring. Auto mode may fall
back to direct mode only after recording the reason as an audit event.

## Ambiguity and user confirmation

Introduce a typed `NeedsInput` outcome or equivalent durable status. Ask the
user only when an unresolved assumption materially changes:

- destructive scope;
- workspace, account, or recipient;
- security or permission boundary;
- external side effect;
- cost;
- acceptance criteria;
- output format that cannot be inferred safely.

Low-risk assumptions may proceed but must remain visible in the contract.

The answer to a clarification must append `AssumptionResolved` before the
next effectful action. A stale answer must not silently modify a newer
contract revision.

## Work-management tool

Add a model-visible `work` tool with operations such as:

```text
get
transition
revise
attach_evidence
request_input
```

The tool proposes a typed event. It must not write the session ledger itself.
The agent validates the proposal, appends the event, and only then returns a
successful tool result.

Allowed model transitions include:

```text
ready → running
running → blocked
running → ready_for_verification
blocked → ready
```

Only deterministic runtime code or the verifier may transition an item to
`Succeeded` or a contract to `Completed`.

The `work` tool itself must pass through the same permission/broker registry
path. Internal state mutation is still an effect and must not bypass the
authorization boundary.

## Agent-loop integration

In `vak-agent/src/lib.rs`:

1. Append the user message as today.
2. Load the active work projection.
3. Drain steering input as today.
4. Build the request from the normal session projection plus active work
   control state.
5. Dispatch the provider with normal retry, route, receipt, spend, and timeout
   handling.
6. Authorize every tool call before execution.
7. Commit work events produced by the work tool.
8. Attach tool, child, flow, and verification evidence.
9. Run stop hooks and the work completion gate.
10. Run goal verification when goal mode is active.

The work gate must block a final completion when a required item is pending,
running, blocked, or awaiting verification; when a child is still live; when a
required criterion lacks evidence; or when a newer mutation invalidated prior
evidence.

Continuation is bounded by the existing max-turn and stop-guard policies.
Exhaustion produces `Unverified` or `Failed` with a clear reason.

## Worker integration

Extend the existing `task` tool arguments with optional:

```json
{
  "contract_id": "work-...",
  "work_item_id": "inspect_gateway"
}
```

Add optional lineage fields to `SessionHeader`:

```rust
pub contract_id: Option<String>,
pub work_item_id: Option<String>,
```

When a child starts:

```text
ItemAssigned
ItemStatusChanged: ready → running
```

When it finishes, attach a child ledger reference as evidence and transition
the item to `ReadyForVerification`, `Failed`, or `Cancelled` as appropriate.

Preserve all existing narrowing rules:

- child tool set is inherited and narrowed;
- readonly children get read-only tools and permission mode;
- child cannot recursively spawn children;
- child path claims are enforced by the normal resource-claim scheduler;
- child tools independently pass permission and sandbox checks.

Keep `WorkerRegistry` for live list, steer, follow-up, and stop controls.
The registry must never be the only record of assignment or completion.

## Evidence

Use typed references rather than copying large output into work events:

```rust
pub enum EvidenceRef {
    LedgerEntry { session_id: String, entry_id: String },
    ToolResult { session_id: String, tool_use_id: String },
    Receipt { session_id: String, entry_id: String },
    CheckpointDiff { session_id: String, from_seq: u32, to_seq: u32 },
    FlowNode { flow: String, run_id: String, node_id: String },
    ChildSession { session_id: String },
    ExternalOperation { integration: String, operation_id: String },
}
```

Plain assistant prose is not sufficient evidence for deterministic criteria.
Evidence must be resolvable after restart or be marked unavailable.

## Verification

Support typed criteria:

```rust
pub enum CriterionKind {
    Shell { command: String },
    FileExists { path: PathBuf },
    FileContains { path: PathBuf, pattern: String },
    ToolSucceeded { tool: String },
    FlowCompleted { flow: String },
    ExternalReceipt { integration: String },
    Semantic { statement: String },
}
```

Verification order:

1. Run deterministic checks through brokered tools.
2. Re-run regression obligations.
3. Resolve child, tool, flow, receipt, and checkpoint evidence.
4. Send only remaining semantic criteria to the read-only judge.
5. Append criterion verdicts.
6. Mark items succeeded only after their required criteria pass.
7. Complete the contract only after all required items and criteria pass.

Goal mode composes with this flow:

```text
work contract complete AND goal audit passes = audited completion
```

The verifier must not mutate the workspace it evaluates. Semantic managed
criteria use the independent read-only judge; deterministic criteria are
checked before that dispatch. The current goal-mode limitation—judging largely
from transcript evidence—must be reduced by adding checkpoint/workspace-diff
evidence before claiming production completeness.
Shell criteria in managed verification therefore require a configured
read-only sandbox and fail closed when one is unavailable. Flow execution can
carry a contract/item context, records flow-node evidence, and maps its
running item through interruption or verification readiness without allowing
the flow to mark the item succeeded directly.
The CLI flow entry points currently run without that context; managed contracts
must not treat those standalone runs as linked work execution.

## Recovery and idempotency

On session reopen, reconcile every `Running` item:

```text
live child exists                         → remain running
finished child ledger exists              → attach result and verify
no child + readonly/retry-safe operation  → interrupted, retry eligible
possible write/external side effect       → interrupted, require review
```

Child sessions persist an append-only terminal marker (`completed`, `failed`,
`aborted`, or `max_turns`) before their parent observes the result. Recovery
must use that marker; the existence of a child ledger alone is not evidence
that the child finished.

Never automatically replay a possible external mutation. Add an operation
identity for retryable effects:

```rust
pub struct OperationIdentity {
    pub contract_id: String,
    pub item_id: String,
    pub attempt: u32,
    pub operation_id: String,
}
```

External integrations should use `operation_id` as an idempotency key where
supported. If an integration cannot prove idempotency, recovery must stop for
human review.

## HTTP, TUI, desktop, and admin surfaces

Add authenticated endpoints:

```text
GET  /sessions/{id}/work
POST /sessions/{id}/work/confirm
POST /sessions/{id}/work/revise
POST /sessions/{id}/work/items/{item}/retry
POST /sessions/{id}/work/items/{item}/cancel
POST /sessions/{id}/work/items/{item}/reassign
```

`GET /work` returns the projected snapshot. It must not expose a merged view
that could be written back as a full replacement and must preserve workspace
boundary checks for every session.

Add typed SSE events:

```text
WorkContractCreated
WorkContractRevised
WorkItemChanged
WorkEvidenceAttached
WorkVerificationFinished
WorkNeedsInput
```

SSE is a live projection only. Reconnect must fetch the snapshot and rebuild
the view from the ledger.

The UI should show objective, revision, item status, owner, child session,
blocker, criteria, and evidence links. Progress must never be derived from
assistant prose.

## Configuration and compatibility

Add a scoped `[work]` configuration section with defaults:

```toml
[work]
enabled = true
default_mode = "direct"
max_items = 20
max_revisions = 8
max_parallel = 4
confirmation = "risk-based"
```

Unknown keys warn and do not fail. Provider and model remain one atomic route.
Work mode is scoped to the run; it must not silently change workspace defaults
or an existing frozen session contract.

Do not touch the documented CorePool warm-entry permission limitation while
implementing this feature unless a separate, fully tested change is requested.

## File-level plan

Expected implementation locations:

| Area | Work |
| --- | --- |
| `crates/vak-session/src/types.rs` | durable work types and ledger payload |
| `crates/vak-session/src/work.rs` | pure projector and transition rules |
| `crates/vak-session/src/log.rs` | append/query/project control state |
| `crates/vak-agent/src/lib.rs` | contract authoring/validation, work projection, event commit, completion gate |
| `crates/vak-agent/src/task.rs` | contract/item lineage and child evidence |
| `crates/vak-core/src/lib.rs` | work mode and contract admission |
| `crates/vak-server/src/lib.rs` | work APIs, auth, SSE, reconciliation |
| `crates/vak-admin-ui` | admin work inspection and operations |
| `crates/vak-desktop` | tracked-work panel and controls |
| `crates/vak-config` | scoped `[work]` configuration |
| `docs/design/00-roadmap.md` | implementation status and exit criteria |

Do not create a separate work database. If a query needs indexing, treat the
index as rebuildable cache derived from session JSONL, like `vak-store`.

## Phased implementation

### Phase 0 — contract and test harness — complete

- Add this design to the documentation index/roadmap.
- Add fixture builders for contracts, events, evidence, and projections.
- Define compatibility behavior for old ledgers.
- No runtime behavior change.

Exit: fixtures compile, old session tests pass, and the contract invariants
are represented by tests.

### Phase 1 — durable ledger and projection — complete

- Add work types and `EntryPayload::Work`.
- Implement append helpers and pure projector.
- Implement transition, revision, dependency, and cycle validation.
- Integrate active control state with `derive_messages()`.
- Make compaction preserve active control state.

Exit: restart and compaction tests pass; direct mode behavior is unchanged.

### Phase 2 — explicit managed mode — complete

- Add `WorkMode` and explicit CLI/API/UI controls.
- Implement structured contract authoring with `WorkPurpose::Plan` receipts.
- Implement deterministic validation.
- Implement `NeedsInput` and assumption resolution.
- Add `GET /sessions/{id}/work`.

Exit: a managed request produces a durable validated contract or a clear
blocked/input outcome before effectful work begins.

### Phase 3 — tracked execution — complete

- Add `work` tool.
- Commit proposed work events in the agent loop.
- Add work completion gate.
- Attach tool and receipt evidence.
- Combine work and goal verification.

Exit: required unfinished work cannot be reported as complete; verified work
can complete; failed verification re-enters execution with findings.

### Phase 4 — worker and flow ownership — complete

- Add contract/item fields to child session headers.
- Link task tool calls to work items.
- Attach child ledger evidence.
- Link flow nodes to work items.
- Expose a managed-only `flow` tool through the core dispatcher. It validates
  the contract/item/flow owner tuple before execution, then the executor owns
  running, node evidence, completion-marker evidence, and final work state.
- Add work-aware worker events and UI.

Exit: every delegated item can be traced parent → child/flow → evidence →
verdict, including parallel execution.

### Phase 5 — recovery and external safety — complete

- Reconcile interrupted items after restart.
- Classify retry-safe operations.
- Add operation identities and idempotency checks.
- Add durable approval/input resume behavior; chat clarification requires the
  explicit answer: prefix and the API cannot resume unresolved assumptions.
- Never replay uncertain external mutations automatically.

Exit: crash/restart tests demonstrate safe recovery without duplicate
side-effects.

### Phase 6 — automatic selection — complete

- Add deterministic complexity/risk signals.
- Add `Auto` mode telemetry and fallback reasons.
- Run direct-vs-managed evaluation comparison.
- Enable auto only when it improves completion and does not regress simple
  latency, cost, or safety.

Exit: auto mode is evidence-backed, reversible, and still fails closed when
contract authoring is required but invalid.

## Required tests

### Session and projection

- old ledgers open without migration;
- work events round-trip;
- revisions are monotonic;
- invalid transitions fail;
- dependency cycles fail;
- projection is deterministic;
- active work survives compaction;
- terminal work leaves model context;
- torn/unknown entries remain recoverable with warnings.

### Agent loop

- managed authoring occurs before effectful tools;
- invalid managed contract fails closed;
- model cannot mark completion directly;
- unfinished required item blocks completion;
- work tool events are authorized and committed;
- tool failure becomes evidence and model-visible feedback;
- goal and work gates compose;
- max-turn exhaustion is typed and durable.

### Workers and flows

- child inherits narrowed capabilities;
- readonly child cannot write;
- child cannot spawn another child;
- child lineage contains parent, contract, and item IDs;
- parallel disjoint items run concurrently;
- overlapping claims serialize;
- child failure maps to item failure;
- flow node status maps to work state without duplicate ownership.

### Recovery

- finished child is reconciled after restart;
- readonly interrupted work may retry;
- uncertain write is not replayed;
- duplicate operation ID is rejected or treated idempotently;
- approval/input resume uses the correct contract revision;
- SSE reconnect returns the same projected state.

### Surfaces and security

- workspace/session boundary checks apply to single and bulk endpoints;
- gateway chat/bot/workspace policy remains unchanged;
- unattended gateway approval remains fail-closed;
- capability overlays remain restrictive;
- all work mutations appear in audit history;
- no secret appears in contract projection, evidence, or UI responses.

## Verification protocol

After every implementation phase, run:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Before declaring the mission complete, also run the relevant deterministic
scenario suite and verify:

```text
target/debug/vak eval
scripts/scenarios/run_all.sh
```

If API keys are available, run `target/debug/vak eval --live` separately and
label it as live-provider evidence. Do not make live-provider success a
requirement for hermetic CI.

Inspect at least one real ledger for each path:

```text
direct session
managed session
managed session with clarification
managed session with child
managed session with failed verification
managed session interrupted and resumed
```

Confirm that the raw JSONL is append-only, the projected work state is
deterministic, and the UI/API agrees with the ledger.

## Definition of done

The implementation is complete only when all statements are true:

- simple chat still follows the existing direct loop;
- complex managed work has a durable, versioned contract;
- material ambiguity pauses for input before effectful execution;
- every required work item has an owner and dependency state;
- parent, worker, flow, tool, and human ownership are distinguishable;
- every effectful action passes the existing authorization boundary;
- progress is projected from durable state, not generated prose;
- child sessions and flow nodes are linked to work items;
- criteria and evidence determine completion;
- goal mode and work mode cannot falsely claim success;
- restart does not duplicate uncertain side effects;
- API, SSE, desktop, and admin surfaces expose the same projection;
- old ledgers remain readable;
- all required format, lint, unit, integration, and scenario checks pass;
- documentation and roadmap status describe the shipped behavior accurately.

The final implementation report must name the completed phases, tests run,
known limitations, and any deferred work. A partially implemented contract
layer must remain marked incomplete rather than being described as production
ready.
