# 61 — Adaptive assistant experience

Status: Phase 1 implemented in the desktop and web client; capability-specific journeys remain phased below.

## Product decision

Vak has one assistant experience. The user does not choose between Everyday and
Advanced modes. The conversation stays calm by default and reveals controls,
evidence, files, previews, and execution details only when the work creates a
reason to use them.

The safety and execution system remains unchanged: every effect is authorized,
every model-visible input is logged, sessions remain append-only, and receipts
remain inspectable. Those guarantees support the experience instead of defining
its initial appearance.

## Job and audience

Vak serves one person who may move from a quick answer to substantial delegated
work without changing products. They may ask, research, compare, remember,
schedule, monitor, browse, book, write, analyze, code, debug, fix, or operate a
system. Success means they can state an outcome in ordinary language, understand
what Vak is doing, make the decisions only they can make, and receive a useful
result without managing the harness.

## Interaction thesis

The conversation is the stable place. A turn may create one or more work objects:
an answer, plan, reminder, comparison, reservation option, research brief,
dataset, document, code change, running process, or approval. Each object belongs
to the turn that created it and keeps the same identity while streaming, settling,
reconnecting, or being reopened later.

Work objects use a shared lifecycle rather than capability-specific screen logic:

1. **Understanding** — Vak briefly reflects the requested outcome only when the
   request is ambiguous or consequential.
2. **Working** — one compact live row names the current action. Completed internal
   steps collapse automatically.
3. **Decision** — a focused approval or choice appears inline with the facts needed
   to decide.
4. **Result** — the outcome leads, with actions attached to the result itself.
5. **Evidence** — sources, receipts, logs, diffs, paths, and diagnostics remain
   available through Details.
6. **Continuation** — the composer naturally accepts refinement, correction, or
   the next action without resetting the context.

## Information architecture

The default shell contains only:

- **Home** — a blank conversation with useful starting points and, later, a quiet
  briefing of due commitments or items needing attention.
- **Conversations** — recent and searchable history across kinds of work.
- **Scheduled** — reminders, recurring work, monitors, and their next run.
- **Inbox** — decisions, failures, completed background work, and meaningful
  changes that require attention.
- **Saved** — conversations intentionally removed from recency without deleting
  their ledger.

Projects, models, permissions, skills, terminals, editors, and providers are
context or configuration. They do not compete with these destinations in primary
navigation.

## Adaptive depth

There is no global expertise mode. Depth follows the object:

- A weather answer shows the answer and sources. It does not expose request JSON,
  tool invocations, or unrelated files.
- A research result shows synthesis, source quality, and citations; query traces
  remain in Details.
- A scheduled action shows timing, recurrence, notification behavior, and a clear
  edit/pause action.
- A booking flow shows comparable options, constraints, price freshness, and the
  exact commitment before purchase.
- A coding result shows the outcome, changed files, and verification. Diff,
  terminal, editor, preview, and receipts open from that result or from Details.
- A failed step shows a short explanation and recovery action. Raw output is a
  disclosure inside evidence, never the main answer.

Details is task-scoped and user-opened. Opening it may reveal files, changes,
terminal, preview, subagents, sources, commitments, and receipts relevant to the
current conversation. It never opens because an event merely exists.

## Capability presentation contract

Presentation is driven by typed semantic output and lifecycle state. It must not
recognize provider wording, package names, command fragments, file names, or
regular-expression guesses to choose a renderer.

Every presentable object supplies:

- stable object and turn identifiers;
- semantic kind and schema version;
- lifecycle state and updated snapshot;
- primary content and compact fallback;
- available actions with authorization requirements;
- optional evidence and artifacts;
- freshness, partial-result, and error state.

Unknown kinds render as clean prose or a compact generic object. They never dump
transport JSON. Sandbox artifacts stay in the task evidence collection unless a
turn explicitly presents one or the user opens it.

## Assistant behavior by use case

### Answering and finding

Lead with the answer. Show citations adjacent to supported claims. If freshness
matters, show when the information was checked. Offer a next action only when it
is plausible from the result.

### Remembering

Confirm what will be remembered, its scope, and how to amend or forget it. Memory
appears in conversation when used materially; the full memory store lives in
Settings and search.

### Scheduling and monitoring

Translate natural language into a readable schedule before it becomes durable.
After creation, show the next run, notification rule, scope, and controls to edit,
pause, run now, or remove it. Background outcomes go to Inbox and the originating
conversation.

### Research and analysis

Separate findings, evidence, uncertainty, and recommendations. Tables, charts,
and source collections appear only when they help the question. The research
object remains refinable in place across follow-ups.

### Transactions and bookings

Gather constraints conversationally, compare real options, preserve price and
availability timestamps, and stop at the irreversible commitment for explicit
human approval. The receipt joins the same object after completion.

### Creating documents and media

Show a readable result first. Attach the generated artifact with Preview, Edit,
Download, and Replace actions as supported. A blank iframe is never a success
state; loading, unsupported, and failed previews are explicit.

### Coding and systems work

Keep narration outcome-oriented. Live commands use one compact activity row.
After completion, show changed files, test status, and material risks. The task
drawer provides diff, terminal, files, preview, pull request, subagents, and full
receipts without turning every conversation into an IDE.

## States

- **Empty:** teaches breadth through natural examples, without category cards
  that constrain what users think Vak can do.
- **Working:** preserves the current assistant message and scroll position while
  its contents update.
- **Waiting:** names the decision and why work paused.
- **Partial:** preserves completed work and names what remains.
- **Failed:** gives the shortest useful cause and recovery; evidence stays folded.
- **Offline/resyncing:** shows one connection banner and preserves unsent input.
- **Complete:** keeps the same objects that streamed, with final actions and
  evidence attached.

## Layout and motion

Desktop uses a conversation column with an optional task drawer. The drawer does
not reserve space while closed. Mobile uses the conversation as the only base
surface; navigation and task details become overlays, while approvals remain
fully actionable.

Scrolling follows reading intent. New output stays pinned only when the reader is
already at the end. Content growth, preview loading, and completion hydration do
not jump the transcript. Motion is limited to state changes: the live assistant
mark, expanding details, opening task context, and inserting a newly completed
object. Reduced-motion preferences remove nonessential transitions.

## Removal and implementation phases

### Phase 1 — one assistant shell (implemented)

- remove Everyday/Advanced state, controls, onboarding choice, and CSS branches;
- remove the companion context rail;
- replace separate navigation trees with Home, Conversations, Scheduled, Inbox,
  and Saved;
- move workspace, model, permission, file mention, and skill controls into a
  deliberate composer disclosure;
- provide one task-level Details action and keep the drawer closed by default;
- keep infrastructure status invisible while healthy.

### Phase 2 — universal work-object runtime

- replace renderer alias tables with schema-registered object presenters;
- associate artifacts and sandbox receipts with their originating turn/object;
- preserve object identity across streaming and durable hydration;
- standardize loading, partial, stale, failed, unsupported, and complete states;
- attach actions and evidence to the object they affect.

### Phase 3 — assistant journeys

- ship first-class memory, schedule, monitor, research, comparison, transaction,
  document, and coding object families on the shared contract;
- build a real Home briefing from commitments and Inbox evidence;
- unify background completion delivery with originating conversations;
- add transaction freshness and irreversible-action review.

### Phase 4 — finish and accessibility

- verify desktop, narrow desktop, and mobile compositions;
- test long conversations, mixed capability turns, reconnects, concurrent work,
  large artifacts, keyboard-only use, screen readers, and reduced motion;
- remove superseded components, selectors, settings, docs, and regression fixtures
  in the same change that replaces each path.

## Non-goals

- a dashboard of every capability;
- a permanent IDE layout;
- a chat transcript that doubles as a raw event log;
- automatic panels caused by tool or sandbox events;
- renderer selection from textual heuristics;
- separate novice and expert products.
