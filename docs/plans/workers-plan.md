# Plan — Workers (doc 100)

Status: proposal (2026-10-18). No stage has started. The design is
`docs/design/100-workers.md`; the maintainer chose the narrow path on
2026-10-18 (`docs/decisions.md`). Stages run in order, and each passes its
gate on the installed route before the next starts. A failed gate stops
the path there and is recorded here and in `docs/plans/open-items.md`.

## Rules for every stage

- Use the existing primitives: triggers and `triggers::claim_due`, run
  records (`Core::runs()`), effects, cursors, Documents, the Library and
  Review, task environments. Add no queue, schedule, lock or result list.
- Each stage replaces what it supersedes in the same change (invariant 30),
  with the tests and doc paragraphs that pinned the old way.
- A stage is done when its "done when" holds in the running app at
  1440 × 900 and 390 × 844, light and dark, and on the installed route,
  never on a build or a fixture alone.
- Keep the record current in the same commit: doc 100's Status line, the
  CHANGELOG, the tracker, the handover.

## W0 — the contract

Scope: the pack type (doc 100 §4) in `vak-session` beside the Document
types, reusing `Deliverable` and `WorkCriterion`; the worker record and its
catalog source; the `TriggerKind` additions a worker needs (item ready,
backlog entry), each declared in `vak_core::state::REGISTRY`.

Done when: a pack round-trips as a Document with versions and a parent;
a worker and its runs are found through the catalog by id; the records
reference regenerates; no user-facing change.

## W1 — give it to a worker

Scope: the approval sheet in the client; the loop's `worker` tool (create
from an artifact version or the current deliverable, show, change, run
now, pause, stop, hand back), the HTTP routes behind the same operations,
the worker's conversation and card, the Workers page; a run on the home in
a task environment that survives a restart and delivers candidates.

Done when: from a dashboard made in a live conversation, the owner gives
it to a worker, approves the sheet, presses Run now, and gets a candidate
in the worker's conversation; a restart mid-run continues or records the
run as interrupted; a run that needs a host outside its scope is refused
and says so; Hand back leaves the artifact as it was.

Gate: five different real artifacts handed over on the installed route,
each Run now completing or failing honestly.

## W2 — wake and work, keep it current

Scope: schedule and stale-on-open duties on the existing triggers;
delivery as doc 94 does it; checks and the dated last good result; the
freshness promise as a status. Replace `task { background }` and the
in-memory `WorkerRegistry` as a record with runs of a pack; retire doc 84
§5's lifetime rule, depth and `MAX_WORKERS` in favour of
`[control.limits]`.

Done when: a nightly worker runs at its time in the owner's zone, delivers
into its conversation and is asleep between runs; a broken source leaves
the last good result dated and the card stale with a reason; the parent
model's background work is a worker run, with doc 84 and AGENTS.md updated.

Gate: two weeks of at least three scheduled workers on the installed route
with every run either delivered or failed with a reason the owner can act
on, and the measured cost per run recorded here.

## W3 — work through items

Scope: a fan-out item source (files in a folder, rows, Library items);
items claimed once, re-claimed after an interrupted run; per-item run
records; concurrency from observed state within `[control.limits]`; the
grouped result for Review.

Done when: a pack over 200 real files completes with one grouped result;
stopping and restarting the home mid-way finishes every item once; an
effect in a step is sent at most once across the restart.

Gate: one real batch the owner would otherwise have done by hand, with its
cost and review time recorded.

## W4 — keep building it, and the caretaker

Scope: the backlog from the worker's conversation; one candidate per
entry; the caretaker: the reconciler for workers and an Agent that turns a
grouped failure into a proposed fix, applied only within `unasked`.

Done when: three change requests become three reviewed versions; a
source that changes format produces one incident and one proposed fix.

Gate: a month of workers whose failures were all either repaired as
reviewed fixes or left honestly stale.

## W5 — keep analysing (only after W2 to W4 pass)

Scope: watchers over a stream through cursors and intake connectors,
alerting only on change.

## Deferred

Recorded in doc 100 §10 with reasons: workers on machines other than
their home, our own hosting and inbound gateway, requests from outside,
serving copies and autoscaling, moving workers, enterprise identity, doc
81's tiles and micro apps, and the bus as a work queue. None starts
unasked.

## Progress log

- 2026-10-18: design and plan written; no stage started.
