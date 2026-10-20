# Decisions

- 2026-10-20: Library entries should show readable content rather than raw
  markup, account for their different content types, and retain their actions,
  flows and interactive controls. See [Library](design/82-library.md#5-the-artifact-page).

- 2026-10-19: Keep conversation previews compact: a single file card where
  possible, otherwise a collapsed preview, with short primary labels and
  icons for secondary actions. See [composed answers](design/87-composed-answers.md).

- 2026-10-19: For the requested next release, fix the sync takeover/catalog
  encryption blocker and require the full gate to pass; do not reuse the
  previous release's waiver. The shared import boundary resets encrypted
  projections declared by the state registry; see
  [data architecture](design/73-data-architecture-and-lifecycle.md).

## 2026-10-18 — uncertain charges and shared call limits

The maintainer chose provider-verified actual cost for crash reservations;
unresolved liabilities stay reserved, with no dismissal action. The owner
records the correction as append-only evidence through FinOps. The maintainer
also approved optional request-wide model-call limits, disabled by default,
shared by the request and all child workers. See the hardening plan and
`docs/design/88-control-plane.md` for scope and retained limitations.


Status: the log of the maintainer's product and architecture decisions,
newest first. Each line says what was decided and where it is recorded in
full; the recording document is the authority. Add a line whenever the
maintainer decides something a later session might otherwise ask again or
undo, in the same commit that records it. Do not ask again about anything
listed here unless the maintainer reopens it.

## 2026-10-18

- The maintainer chose encryption for the derived search catalog too,
  accepting the implementation/performance work. Searchable projections,
  journal/WAL and temporary-file behavior are included in the at-rest
  confidentiality boundary; a plaintext SQLite catalog is not the accepted
  final policy. Implementation and acceptance are recorded in the
  [hardening plan](plans/architecture-and-presentation-hardening-plan.md).

- During the Canvas continuation the maintainer requested no full gate
  ("we dont need gate right now"). This continuation uses focused tests,
  browser checks where available, bundle builds and focused Clippy; no
  complete workspace test result is claimed. Recorded in
  [the handover](plans/handover-2026-10-18.md#current-encrypted-reload-recovery-batch).

- The maintainer waived the full repository gate for the responsive
  chart/map and browser-fixture batch ("we dont need gate right now").
  Stop the active run; retain the passing browser, bundle, typecheck and
  formatting/Clippy evidence without claiming a completed workspace test
  run. This exception applies to this batch, not future work. Recorded in
  [the handover](plans/handover-2026-10-18.md#current-responsive-presentation-batch).

- The workers work (doc 100 and its plan) is deferred as a whole: the
  design and the decided choices stay recorded, and no stage starts until
  the maintainer reopens it. Recorded in
  [docs/design/100-workers.md](design/100-workers.md).

- Workers, the four open choices (asked to choose by scenario and blast
  radius): unasked, a worker only runs its approved pack, makes its own
  output's new version current when its checks pass, delivers candidates
  and pauses itself; every pack change, fix, update, unnamed effect and
  mail or calendar change asks. A worker's conversation belongs to the
  owner Agent. Code steps run in the existing broker worker and sandbox,
  no container per pack. Budgets come from a measured trial run; the owner
  sets the monthly cap; fan-out items run one at a time by default.
  Recorded in [docs/design/100-workers.md](design/100-workers.md) §12.

- A worker is a run of a well-defined pack that carries the engine,
  handed over from something Vakyartha made and kept running; one owner may
  have one or a thousand. Build only the narrow path: the pack, give it to a
  worker, wake and work, keep it current, work through items, keep
  building it and the caretaker, with workers running inside their home
  and the home run wherever the owner chooses. Deferred, with reasons:
  workers placed away from their home (AWS, GCP, Azure, Docker, VPS),
  Vakyartha's own hosting and inbound gateway, requests from outside,
  serving copies and autoscaling, enterprise identity. Each stage passes
  its gate on the installed route before the next. Doc 81 is superseded.
  Recorded in [docs/design/100-workers.md](design/100-workers.md) and
  [the workers plan](plans/workers-plan.md).

- Commit and push directly to `main`; do not create pull requests. This
  applies to future work unless the maintainer changes it. Recorded in
  [the current handover](plans/handover-2026-10-18.md#current-canvas-document-batch).

- Every document carries a status word, and `scripts/check_records.py`
  fails on drift the tree can contradict: a stale version, baseline or
  crate count, a missing layout row, a `vak` command the CLI lacks, or a
  roadmap that lists built work. The blueprint lists only findings with a
  cost today; fixed or unsupported findings leave it. Recorded in
  [docs/README.md](README.md) and [the blueprint's rules](../blueprint/AGENTS.md).

- Known vulnerabilities fail the gate (`scripts/check-advisories.sh`). An
  advisory is ignored only when Vak cannot reach the vulnerable code, with
  the reason in `.cargo/audit.toml` and an open item to remove it.

- Vak owns its storage layer: no third-party database beyond the embedded
  SQLite it already uses for refs and the catalog. Findings about the
  storage substrate describe its cost and the testing it needs, never its
  replacement ([the blueprint's rules](../blueprint/AGENTS.md)).

- Keep the system blueprint in `blueprint/`: one place to see what the
  system is, what is happening and what needs attention, generated as
  README.md, HTML and JSON from one model by `blueprint/harness/run.py` on
  every build and checked by `scripts/release.sh`, so git holds its
  versions and diffs. It reads the record (decisions, open work, specs)
  where it is written and never copies it. Its rules are
  [blueprint/AGENTS.md](../blueprint/AGENTS.md); its methods are
  `blueprint/methods/`, in tool-neutral form.

- Begin implementing the hardening plan. Do not use Astra; use Luna for
  well-defined delegated work and keep token use lean. The main agent owns
  integration and review; the first slice addresses strict authoritative
  reads and writer fencing (the plan's implementation log).

- Consolidate every review pass from this session into a new handover and
  an actionable plan, including system architecture, answer/projection,
  UI/canvas and dynamic/interlinked cards with abilities. Replace and
  delete the old handover; retain other historical records. No further
  Astra consultation. The evidence is in
  [the consolidated audit](audits/architecture-and-presentation-review-2026-10-18.md)
  and the proposed work in
  [the hardening plan](plans/architecture-and-presentation-hardening-plan.md).
  Recording the plan does not approve its unresolved architecture choices
  or claim the repairs are implemented.

## 2026-10-17

- Where an automation's result goes is one argument of the automation
  tool, `result_to`: the conversation (default), a new page, or a page's
  id; it replaces `as_page` and `page`
  (`docs/design/98-answers-beyond-one-card.md` §12).
- A card naming a `deliverable` with no plan declared is refused, as a
  name the plan lacks is (`docs/design/87-composed-answers.md`).

- Steps cards stay flat: a step's alternatives are the card's own
  top-level `choices`, each naming its step, never records nested in a
  step (doc 30 §30.1, `docs/plans/open-items.md` "Card forms").
- Markup previews get libraries by the client inlining a pinned library
  into the opaque frame's srcdoc, not from a preview origin; for now only
  the diagram library the client already pins, and only when the card's
  typed `libraries` field, required, names it; charts stay the chart card's
  (`docs/design/96-answer-projection-gaps.md` P3).
- An MCP tool's UI resource is shown as text only; Vakyartha never mounts
  a server's UI (`docs/design/96-answer-projection-gaps.md` P4).
- `mail_calendar` loads only when an account is connected, decided from a
  connected-accounts rollup Document, never by replaying the connection
  ledger on the turn path (`docs/design/97-cards-end-the-answer.md` §7).

- Living pages (`docs/design/98-answers-beyond-one-card.md` §6 S5, §12):
  a saved answer is a page of its own items, never Markdown; an automation
  keeps a page current through one field, `Trigger::page`, set by the
  owner's Keep current or the Agent's automation tool; a page's runs go to
  the page alone, not a conversation; a viewer's control values are their
  own and never saved.

## 2026-10-16

- The map form (`docs/design/96-answer-projection-gaps.md` P5): places
  are drawn with no network by default; map tiles reach the screen only
  through vak-server's tile route, only while the owner's `[maps] tiles`
  switch (Shared layer, privileged, off by default) is on; the tile source
  is an editable template, OpenStreetMap by default; coordinates are the
  model's own, with no geocoding step.
- Card actions (doc 96 Q3): the marks a turn's tail carries are read from
  the conversation's own `CardAction` entries since the previous
  directive, like the other tail sections, rather than a rollup Document
  (a session ledger is not a record chain); a choice starts its turn with
  a user message whose metadata names the `CardAction` entry.
- Cards end the answer by declaration
  (`docs/design/97-cards-end-the-answer.md`): every card takes a required
  `then` (`more` or `done`), and a step of accepted cards whose last card
  said `done` ends the turn when the plan has nothing left. Measured: the
  open step after a card restated it on the dev route whatever the prompt,
  result, thinking or sampling; with `then`, one-card answers said `done`
  and the step after `more` added a card 5 of 5. Native tool search stays
  unused after a survey of every supported provider: three routes have it
  and a discovery step hurt the smallest route; deferral remains the "More
  tools" catalogue, called by name.
- One history tool and no discovery tool
  (`docs/audits/history-lookup-2026-10-16.md`,
  `docs/design/68-context-engine.md` §5): `recall` searches this
  conversation, other conversations, notes, the profile, sources and
  deliverables in one call (`session_search` removed); a "More tools"
  entry is called by its name (`find_tools` removed); no wire is sent a
  deferred tool, so Anthropic's `defer_loading` and tool search are not
  used. Designs are rewritten, not patched, with no compatibility kept.
- Cards that show only what a call produced load once the conversation
  holds such a call (`docs/audits/grounded-cards-2026-10-15.md`).

## 2026-10-15

- Scheduled work you can see (`docs/design/94-scheduled-work-you-can-see.md`
  §2): a scheduled result appears in the conversation it belongs to, as
  the Agent's turn marked scheduled, with a notification (the Inbox lists
  it too); people steer automations three ways over one set of
  operations: a card in the chat, an Automations page, and asking the
  Agent; wrong screens and contracts are rewritten, not patched.

## 2026-10-14

- Self-evolving capabilities (`docs/design/93-self-evolving-capabilities.md`
  §2, D1 to D8): a skill may carry a sandboxed script with tests; tiered
  admission (live only when sandboxed, no network, workspace reads and
  draft writes, else Review); evolution stays above the Rust runtime; a
  near match is extended as a new version and the answer says so;
  Agent-owned first, Shared on evidence from tests and real use; an
  opt-in review of failures proposes fixes; a compact index in the prompt
  plus `find_tools`. Doc 43 is superseded. Screens that get in the way are
  rewritten, not patched: there are no users yet.
- On the maintainer's delegation ("make the best decisions"), doc 93 §2
  R1 to R7: a capability is a skill with a program (one kind); promotion
  and withdrawal by observed states, no counts; dependencies pinned with
  hashes and installed once by their own run (Review the first time);
  a capability shows only through cards, draft previews and declarative
  packs, and a card may take its data from the call; Office files and
  PDFs only through `office_apply`; a proposed automation is always
  Review.
- Keep one handover and one tracker: superseded handovers are deleted (git
  keeps them); open work lives in `docs/plans/open-items.md`. Every design
  doc's Status starts with one word from a closed set, and
  `docs/design/INDEX.md` is generated from them (`docs/README.md`).
- Docs are kept, never deleted, so people and agents read the same record;
  what is past is marked "Historical record", not rewritten.
- `recall` opens another conversation only through an artifact attached in
  this one (`docs/design/90-file-state.md`, Status).
- Card results come from the call that ran: the test-report card only, not
  every card kind (AGENTS.md invariant 36, `Tool::output_of_command`).
- Concurrent artifact writes across processes are a designed fix, not an
  accepted gap: one `Artifacts::transaction` per decision
  (`docs/design/90-file-state.md` §3.5).
- Next release is 7.2.0 once `docs/plans/open-items.md` "Before the next
  release" is done; tagging and publishing wait for the maintainer.

## 2026-10-13

- Stopping rule: no new measured patterns in this round; they are the next
  round's backlog (`docs/plans/open-items.md`).
- An `/until-done` hold that adds nothing new ends the turn and leaves the
  commitment open (`docs/design/88-control-plane.md`).

## 2026-10-12

- Each card is its own call of the card tool that owns its kind, never one
  nested `answer` call (`docs/design/87-composed-answers.md` §2).
- File state decisions D1 to D4 (`docs/design/90-file-state.md` §4).

## 2026-10-11

- The control plane's decisions at C0 (`docs/design/88-control-plane.md`
  §12); the model plans and the runtime checks
  (`docs/design/86-model-plans-runtime-checks.md`); commitments open only on
  an explicit ask (`/until-done`, `/goal replace`).
- Nothing is ended because a number of seconds in code went by
  (`docs/design/88-control-plane.md` §5a).

## Earlier, recorded in their documents

- Data architecture: decisions L1 to L12, revision 4, one owner
  (`docs/plans/data-architecture-plan.md`); not built by decision: an S3 or
  Postgres remote, key escrow, data roles, single-item retention labels, an
  erasure-request queue, audit export, merging two machines that both wrote.
- Visual refresh: decisions D1 to D5 (`docs/plans/visual-refresh-plan.md`).
- The product name is Vakyartha (2026-09-25, AGENTS.md "Identity").
- Single-owner app (2026-10-08): one owner per install, no other accounts,
  data roles, admins or second-person approvals of the owner's actions
  (`docs/design/74-lifecycle-and-data-administration.md`, Status). Guests
  exist only through share links and coworking invitations to one
  conversation (`docs/design/69-shared-conversation-coworking.md`); the
  owner may hand a guest one pending approval there.
- Mail and calendar against the storage of the time, on a feature branch
  since merged (`docs/design/80-mail-and-calendar.md`).
- Main only: no pull requests, side branches or leftover worktrees.
