# 100 — Workers: work handed over, kept running

Status: proposal, deferred by the maintainer on 2026-10-18: no stage
starts until the maintainer reopens it. Nothing in this document is built. It
records the maintainer's direction for workers and the narrow scope chosen
to build first (§9); everything else is recorded as deferred with its
reason (§10). The four choices left open were decided on 2026-10-18 (§12). The stages and their gates are
`docs/plans/workers-plan.md`. It supersedes `81-pieces-and-home.md` (its
pieces become kept workers, §3.4) and, once stage W2 is built, the
lifetime, depth and cap rules of `84-worker-questions-and-control.md` §5.
It builds on the trigger, run, effect, cursor and Library primitives of
`73-data-architecture-and-lifecycle.md`, scheduled work
(`94-scheduled-work-you-can-see.md`), task environments and Review
(`54-task-environments-and-promotion.md`), managed work items
(`42-managed-work-contracts.md`) and the control plane
(`88-control-plane.md`). Where this document and AGENTS.md disagree,
AGENTS.md wins until the change is made to both.

## 0. Why

A worker was first a sub-call: the parent model hands a child one prompt,
the child runs inside the parent's turn and returns text
(`crates/vak-agent/src/task.rs`). Doc 84 gave it questions, a background
mode and a `workers` tool, but it is still that: one prompt string, an
in-memory registry, a cap of eight (`MAX_WORKERS`), and cancellation when
the parent turn ends.

Vakyartha has since grown everything a worker needs to stand on its own:
triggers with one claim per slot, run records, effects that are sent at
most once, cursors, the Library with versions and Review, task
environments, the egress proxy, delivery into the conversation, the
Operations Center, and an always-on install. So the idea can be what it
was meant to be:

> A worker carries the engine and a well-defined pack, and runs. A person
> can have one or a thousand.

The moment that matters is after Vakyartha has made something: a
dashboard, an analysis, a small system. The person says "give this to a
worker", and from then on the worker looks after it: keeps it current,
wakes at two in the morning to rebuild it, works through change requests,
repairs it when a source changes, and says honestly how it stands.

## 1. Principles

1. **A worker is a run of a pack, not a call inside a turn.** It has its
   own run record, ledger, environment and conversation, and it does not
   die with the turn that started it.
2. **The model builds; code runs.** Where a step can be ordinary code, the
   worker runs code. The model returns to change, repair, judge (where the
   pack asks for judgement) or explain.
3. **One mechanism per need.** Starting work is a trigger, claimed through
   `triggers::claim_due`. Each run is a run record. Each action outside
   Vakyartha is an effect. Each result is a Library version or a
   candidate. A worker adds no queue, schedule, lock or result list of its
   own (AGENTS.md, "The data rules").
4. **Authority is set by the owner, before the run.** Nobody is watching
   an unattended run, so a pack is approved up front. What it may do
   unasked is fixed then; anything else fails closed (invariants 15, 32).
5. **Results come back for Review.** A worker never changes the owner's
   files or a kept version in place; it delivers candidates and versions.
6. **Honest status.** Every result says when it was true; a missed promise
   is a status, never silence.

## 2. Vocabulary

- **Pack** — the typed, versioned description of a piece of work: what to
  do, its inputs, its scope, its deliverables and checks, its triggers,
  its limits and its owner Agent (§4). A Document owned by that Agent.
- **Worker** — a pack the owner has handed over, with its conversation,
  its card and its history. "Give it to a worker" creates one.
- **Run** — one execution of a worker's pack: a run record
  (`Core::runs()`), its own ledger, its environment.
- **Duty** — what the owner asked the worker to do with the pack (§3).
- **Item** — one input of a fan-out run (a file, a row, a document).
- **Caretaker** — the reconciler and the Agent that look after workers
  (§7).

## 3. Duties

A worker holds one or more duties. Each duty is a trigger kind plus what
the run delivers.

| Duty | The owner says | The worker does | In scope (§9) |
|---|---|---|---|
| Wake and work | "Every night at 2, rebuild the report" | a scheduled run that starts, works, delivers into the worker's conversation and the place it was asked, and ends | yes |
| Keep it current | "Keep these numbers live" | refreshes a dashboard or living document on a schedule or when opened stale; every value carries its as-of time | yes |
| Work through items | "Review these 1000 contracts" | one pack fanned out over items, each item claimed once, results grouped for Review | yes |
| Keep building it | "Add last year for comparison" | a backlog of change requests, each delivered as a candidate | yes |
| Look after it | (none) | repairs a broken source or check, as a candidate | yes, through the caretaker (§7) |
| Keep analysing | "Tell me when competitor prices move" | a watcher over a stream (cursor), alerting only on change | later (W5) |
| Run it | "Put this dashboard online" | hosts a site or API at an address | deferred (§10) |
| Answer for it | "People can fill this form" | handles requests from outside | deferred (§10) |

### 3.1 The moment of hand-over

"Give it to a worker" is a button on an artifact or card, and an
utterance the model turns into a call of the loop's `worker` tool
(stage W1). Both open one **approval sheet** in plain words:

- what it looks after (the artifact version, the folder);
- its duties and when it wakes;
- what it may reach: folders, hosts, connected accounts;
- what it may do unasked (§12.1; the sheet lists it, it is not a choice
  of the sheet);
- what one run cost in its trial run, what a month would cost on its
  schedule, and the monthly cap the owner sets (§12.4);
- where it runs (this home; §8).

The owner approves once. The sheet is the pack's authority; the worker
cannot widen it, and a change to it is a new pack version the owner
approves again.

### 3.2 Fan-out

A fan-out run claims its items one at a time through the trigger claim
model, so each item is done once even when a run is stopped or the home
restarts; an item whose run ended without a result is claimed again. Each
item has its own run record under the worker's run (`TraceKey::delegate`).
An action outside Vakyartha stays at most once across retries because it
is an effect. Results return as one grouped result: a table of what
passed, what failed and what needs a look, with each item's candidate
behind it, so Review sees exceptions rather than a thousand rows.

How many items run at once is decided by observed state (items waiting,
the provider's answers, the circuit breaker, the spend gate) within the
owner's limits in `[control.limits]` (`vak_control::ControlLimits`), shown
in Settings. No number is picked in code. When the work outgrows the
limits the worker says so with an estimate rather than exceeding them.

### 3.3 Keep building it

A message in the worker's conversation that asks for a change becomes a
backlog entry. The worker works through entries one at a time, each in its
task environment, each delivered as a candidate through the one Review
path. Messages are information, never authority: a request that needs a
new host, account or folder waits for the owner to change the pack.

### 3.4 Kept workers replace pieces

Doc 81 proposed pieces: kept work with a description, versions, jobs and
views. A piece is a worker whose pack holds code steps and a view, with
the "keep it current" duty. There is one description, the pack, not a
`piece.toml` beside it (invariant 30). Doc 81's ladder (document, live
document, tile, micro app, system), Home and tiles, and its gateways are
carried as ideas into §10.

## 4. The pack

The pack is a Document owned by the worker's Agent
(`vak_session::documents`), versioned, with the current version a ref. A
new version names its parent; nothing overwrites one. Fields, additive
within a major version:

| Field | Holds |
|---|---|
| `objective` | what the work is for, in the owner's words |
| `steps` | model steps (instructions) and code steps (a script in the pack's folder, run in its environment) |
| `inputs` | artifact versions by id, the worker's drop box, a fan-out item source |
| `scope` | folders (as paths in a task environment), tools, connected accounts, hosts the egress proxy admits |
| `deliverables` | the `plan` forms (card, text, file, change, check), each named |
| `checks` | deterministic checks a run must pass for its result to become current (the page renders, totals are not empty) |
| `duties` | the triggers and what each delivers (§3) |
| `limits` | spend per run and per month, how many runs or items at once, a freshness promise |
| `unasked` | fixed by §12.1, not a field a pack can widen; the pack only lists which outward effect kinds the owner approved on the sheet |
| `owner` | the Agent; the worker's conversation lives in that Agent's home |

The pack reuses existing types rather than defining new ones: deliverables
are the `plan` tool's `Deliverable`, criteria are doc 42's `WorkCriterion`,
and the run's outcome is checked by `vak_judge::plan` against receipts, so
a worker's result is judged the same way a turn's is.

## 5. The run

1. A trigger fires (schedule, item ready, backlog entry, Run now). The run
   is claimed through `triggers::claim_due` and opened with
   `Core::runs().open` before any work; its trace key names the worker's
   Agent and the cause.
2. The run starts in a task environment built from the pack
   (`54-task-environments-and-promotion.md`): a copy or worktree of the
   folders it may touch, its execution root, its egress proxy.
3. Code steps run in the broker worker under the environment's sandbox;
   model steps run the agent loop with the pack's tools and scope. The
   loop is the same loop a turn uses, so every correction, plan check,
   provenance label and permission decision applies unchanged.
4. Checks run. A result whose checks fail is not current; the last good
   result stays, dated.
5. The run settles: deliverables become Library versions or candidates,
   a card in the worker's conversation says what happened, and the run
   record says how it ended. A failure says why in the conversation and
   the inbox (`RoutineFailed`), never silently.
6. A run survives a restart of the home: it continues from its ledger and
   claim, not from memory. A run whose process died is recorded as
   interrupted and its items are claimed again.

## 6. What the owner sees

- **The Workers page**, and a line per worker on Home: what it looks
  after, its status in plain words ("Updated 10 minutes ago", "Asleep
  until 02:00", "Waiting for you", "Stale since Tuesday: the source
  changed"), next wake, spend this month. Show technical details opens
  runs, logs, receipts and the pack.
- **The worker's conversation**, an ordinary conversation of the owner
  Agent: results arrive there, and "why did last night's run fail?" or
  "make it weekly" go there.
- **Controls**, one set: Pause, Resume, Run now, Stop, Change the pack,
  Hand back (the worker ends and what it looked after stays an ordinary
  artifact).
- **Quiet by default**: nothing is said when runs are fine; the inbox and
  the morning digest name what needs the owner.

## 7. Who watches, controls and fixes

| Layer | Its job | Never |
|---|---|---|
| The run | checks its own results, reports status, stops cleanly | widen its scope or spend |
| The service manager | restarts the home process (invariant 25) | decide anything else |
| The reconciler | level-triggered (invariant 31): what should be scheduled, claimed and current versus what is; closes the gap from records | act on a guess |
| The caretaker Agent | reads failures and incidents, finds the cause, proposes a fix as a candidate; applies it only when the pack's `unasked` allows | approve its own proposal |
| The owner | anything that widens authority, spends more or cannot be undone | — |

Failures are grouped by cause ("40 items failed: the source changed its
format" is one incident in the `operations/incidents` chain). Updates of
Vakyartha itself are the home's own update; updates of what a worker looks
after are candidates; dependency updates are candidates with their test
results.

## 8. Where workers run

**In this scope, a worker runs inside a Vakyartha home, and a place is
where that home runs.** The home already installs on a Mac, a Linux box, a
VPS, a cloud VM or a Docker host (`docs/hosting.md`); the always-on AWS
install is one. A person who wants workers to run while the laptop is
closed runs their home on an always-on machine. This gives "anywhere" with
no new provisioning code, keeps the home the one writer of its data home,
and keeps every key at the home.

Running a worker on a machine other than its home's is deferred (§10).

## 9. Build scope

The narrow path, in stages with gates (`docs/plans/workers-plan.md`):

- **W0** — the pack contract and the decision record; nothing user-facing.
- **W1** — give it to a worker: the approval sheet, the pack Document, the
  worker's conversation and card, Run now, Pause, Stop, Hand back. Runs
  on the home, survive restarts, deliver for Review.
- **W2** — wake and work, and keep it current, on the existing triggers and
  doc 94's delivery. `task { background }` and the in-memory registry
  become runs of a pack (invariant 30); doc 84 §5's lifetime rule, depth
  and cap are replaced.
- **W3** — fan-out over items, grouped Review.
- **W4** — keep building it and the caretaker.
- **W5** — keep analysing (watchers over cursors), only if W2 to W4 pass
  their gates.

Each stage passes its gate on the installed route before the next starts.
A gate that fails stops the path there.

## 10. Deferred, with reasons

The maintainer chose the narrow path on 2026-10-18 after a critical
review. These are recorded so a later session does not start them unasked.

| Deferred | Why |
|---|---|
| Workers placed on other machines (AWS, GCP, Azure, any Docker host or VPS, separately from their home) | It needs sealed result bundles imported into the home, because only one process writes a data home; per-worker credentials outside the home; and provisioning adapters per provider. Running the home itself anywhere gives most of the value without it (§8). |
| Our own hosting: "run it" at an address, an inbound gateway, public sites | It makes Vakyartha an internet-facing server for code a model wrote, against the loopback-first rule (invariant 34), and duplicates platforms that do hosting well. Private previews already exist (invariant 35). A later alternative is deploying to the owner's own host as a reviewed effect. |
| "Answer for it": requests from outside (forms, webhooks, guests) | Depends on hosting; and someone else's words beside the owner's data put every exit before the owner (invariant 43), which an unattended worker cannot satisfy. |
| Copies of one worker serving traffic, autoscaling | Depends on hosting. Fan-out over items (W3) covers batch scale. |
| Moving workers between places | Depends on remote placement. |
| Enterprise use: many users, roles, teams, single sign-on, compliance | Vakyartha is a single-owner app by decision (doc 29); workers do not change that. Governed batch and scheduled work (W2, W3) is what an enterprise would value, and it stays compatible with the tenant and principal primitives already in the data model. |
| Doc 81's Home tiles, micro apps and ingress/egress gateways as a platform | Home shows workers (§6); tiles and micro apps wait on W2 to W4 proving that kept work is used. |
| The bus (`vak-bus`) as the work queue | Triggers and claims already give one start per slot or item; a second queue is forbidden by the data rules. The bus is reconsidered only with remote placement. |

## 11. Rules this keeps or changes

| Rule | How |
|---|---|
| Invariant 1, 2 | a run's model-visible input is in its own ledger; the worker's conversation is append-only |
| Invariant 13, 15 | a worker never gets FullAccess automatically; an unattended run fails closed outside its approval |
| Invariant 30 | the pack replaces `task { background }`, the in-memory registry as a record, and doc 81's `piece.toml` |
| Invariant 32 | messages and backlog entries are information; only the owner changes a pack's scope; an Agent controls only its own workers |
| Invariant 33 | a worker's result passes only on runtime-observed checks |
| Invariant 37 | every worker, pack and run has one owner Agent |
| Invariant 41 | every action outside Vakyartha is an effect |
| Invariant 43 | a run that holds someone else's words and the owner's data asks before any exit; unattended, it is refused |
| Doc 84 §5.2 | replaced at W2: a worker outlives the turn that created it, because its results go to its own conversation |

## 12. Decisions (2026-10-18)

The maintainer asked for the best choice for each open question, weighing
every scenario in §3 and the blast radius at runtime. Each choice is the
one that changes the fewest existing contracts and fails smallest.

### 12.1 What a worker may do without asking

Unasked, a worker may:

- run its approved pack on its triggers, retry its own step under the
  ladder, and claim again an item whose run was interrupted;
- make a new version of **its own output** current when every check
  passes: the refreshed dashboard or report its approved pack produces
  (keep it current needs no Review per refresh, because the code and steps
  that made it were approved and only the data moved);
- deliver candidates, cards and inbox notes;
- pause itself, with a reason, when it reaches its cap or its checks keep
  failing.

Always asks, whatever the pack says:

- any change to the pack itself (steps, code, scope, hosts, accounts,
  duties), including the caretaker's fixes and dependency or security
  updates: each is a candidate for Review;
- any change to the owner's files or to anything the worker does not own;
- every outward effect the sheet did not name, and every mail or calendar
  change, which stays one reviewed candidate per effect (invariant 41);
- every exit once a run holds someone else's words beside the owner's
  data (invariant 43); unattended, that is a refusal.

Why: a refresh is the duty itself, so asking for it would make "keep it
current" useless, and its blast radius is one dated version with the last
good one kept. Applying fixes or updates unattended could break a working
thing with nobody watching; whether any fix may apply itself is decided
only after the W4 gate measures how often proposed fixes were right.

### 12.2 Whose conversation a worker has

A worker's conversation is a conversation of **the Agent that owns it**,
shown on the worker's card, and its runs carry that Agent's identity with
the worker named in the trace key. A result is also noted in the
conversation where the worker was handed over, as doc 94 delivers
scheduled results. A pack may name a saved Agent's identity and working
style for its model steps, as `task` already allows, without changing
permissions.

Why: an Agent per worker would mean a thousand Agent homes, memories,
settings and lifecycles for a thousand workers, and a second answer to
who owns the work (invariant 37). Under the owner Agent, pausing or
archiving the Agent stops its workers at the next boundary with the rule
that already exists, audiences and search are unchanged, and nothing new
is admitted.

### 12.3 Where code steps run

Code steps run through the **existing broker worker and sandbox**
(invariant 14): Seatbelt or Landlock, or the Docker backend where the
owner already chose it for the environment. Dependencies are pinned in the
pack and installed by their own worker run into the execution cache, as a
skill's program does (`93-self-evolving-capabilities.md`). There is no
container per pack.

Why: one execution boundary is already reviewed, tested and fail-closed;
a container per pack needs Docker on every home (a Mac often has none),
image building and pinning, and a second boundary to keep equal to the
first. Containers are reconsidered only with remote placement (§10).

### 12.4 Budgets on the approval sheet

Nothing is guessed. Before a worker's first scheduled run, it does one
**trial run** while the owner watches; the sheet then shows that run's
measured cost (tokens, and time on this home) and what its schedule would
cost in a month. The owner sets the worker's monthly cap; the sheet fills
it with that estimate, and the owner may change it. Every run reserves
through the shared spend gate, so the install's existing day and month
caps still bound all workers together. Items at once in a fan-out run
default to one, an operator limit in `[control.limits]` shown in Settings
beside that shipped default; one is not a tuned number but no
concurrency, which fails smallest. There is no default freshness promise:
the owner sets one or there is none.

Why: a picked default budget would be a number tuned to nobody, and a
worker that has never run has no cost to show. A measured trial gives the
owner a real figure, and serial items mean a fault in a new pack affects
one item at a time.
