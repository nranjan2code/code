# Decisions

- 2026-10-23: After the installed dev conversation failed on removed
  `topics_placement` records, the maintainer explicitly authorized erasure or
  reset on this dev machine. Reset only the affected conversation through
  the supported Trash action; do not restore retired plan compatibility.

- 2026-10-23: The maintainer requested committing, pushing, releasing and
  installing the completed progress and navigator changes locally. Cut 8.29.0;
  this supersedes the earlier instruction to leave these changes unreleased.

- 2026-10-22: Progress belongs in the existing top semicircle and right
  Details panel, not in the chat answer. Both surfaces read the same latest
  accepted list, without inferring delivery from cards.

- 2026-10-22: Restore only optional visible progress for multi-step work and
  one reminder about explicitly open items. `progress` is loop-owned, uses
  whole-list updates, and never gates cards, layout or the answer. Completeness
  reads only its declared states; no intent reader or strands. The old plan
  remains removed. No release is requested. See doc 88’s optional-progress
  contract and docs/plans/open-items.md.

- 2026-10-22: The maintainer explicitly rejected backward compatibility for
  the removed plan tool. Delete its parser, outline projection and wire fields,
  plan-only correction tags and reply filters; retain no old-plan label fallback.
  Stored ledger bytes are not rewritten. This supersedes the earlier decision
  to retain `parse_plan_args`, `Cause::Plan`, `Cause::TopicsPlacement` and
  `ControlKind::PlanCheck` solely for recorded history.

- 2026-10-22: The maintainer reported that turn navigator hover exposed the
  raw request after removal of the plan tool, and asked about the design
  dependency. Navigation now reads recorded answer headings or authored card
  titles, removes the old plan reader, and shows a turn number when
  untitled. No model call, planning tool or subject-based rules are added. See
  [the regression check](audits/turn-navigator-labels-2026-10-22.md).

- 2026-10-22: The maintainer asked for the card-side redesign: the runtime
  builds cards from tool results and markdown, strict tools stay only for
  chart, stats, map, calendar and controls, and the model authors no
  layout. Step 1 is done: the `plan` tool and plan check, the forced card
  and Topics calls, and the `deliverable`, `replaces`, `beside_previous`
  and `then` fields are removed (docs 86 and 97 superseded). The remaining
  work is tracked in docs/plans/open-items.md. Step 2 offers only the five
  interactive tools; Markdown replaces the other model-authored card calls,
  while every form remains drawable. Step 3 records runtime sources and actual
  file diffs at turn close as Derived presentations after the answer, with no
  model call. Live acceptance and release remain in the tracker.

- 2026-10-22: The maintainer asked for one design run on conversation context
  instead of more one-slot fixes, and to fold the paused parallel continuity work
  into one change. Decided: a request carries the newest exchanges whole while
  they fit the measured budget, with one stored summary of everything older,
  written once per window fill and reused (invariant 36 rewritten). This
  replaces one-turn admission and the unshipped chronological `recall`
  browsing. Kept from that work: the surface fix, the equal-answer drift
  removal, historical dialogue rendering, the visibility iterator, and the
  scenario corpus, runner and audit. See
  [doc 68](design/68-context-engine.md) and
  [the audit](audits/conversation-continuity-2026-10-22.md).

- 2026-10-22: The maintainer approved implementing the shared chat/projection
  UX review and selected Continuous answer. Fragmentation in code work is one
  example across arbitrary subjects: use open composed sections, compact exact
  artifact collections and shared actions without domain routing, role guessing
  or hiding forms. Deliberately opened objects retain the existing focused
  workspace. See [the review](audits/chat-projection-ux-2026-10-22.md) and
  [implementation acceptance](audits/chat-projection-ux-implementation-2026-10-22.md).

- 2026-10-22: The maintainer asked whether script restrictions were covered
  and should be evaluated rather than applied automatically. The design now
  specifies capability-based script admission: existing authority plus
  qualified enforcement, not a blanket ban or automatic trust of generated
  code. Model/static judgement cannot grant access or prove isolation. Current
  protection stays until its replacement passes the explicit script matrix;
  the local checks observed pausing but did not enable script execution. See
  [script admission](design/103-kept-work-lifecycle.md#61-script-admission-is-a-capability-decision).

- 2026-10-22: The maintainer requested installed local `vak-home` try-outs
  with no expectation of technical users, and clarified that free-form edits
  to the same work create versions on the same artifact/Library card, not
  duplicate entries. Existing history/actual parent and human edits must be
  preserved; an independent copy is a separate semantic request. The normal
  Open/Use path must expose usable controls and understandable necessary
  consent, rather than source filenames, setup commands or inert pages marked
  ready. See [the contract](design/103-kept-work-lifecycle.md#52-everyday-use-and-free-form-changes)
  and [installed observations](audits/kept-work-everyday-use-2026-10-22.md).

- 2026-10-22: The maintainer requested fixing the general design for creating,
  editing, running and using kept work, without domain routing or a dashboard
  spot fix. Keep the existing artifact identity; compose optional execution,
  views, capability bindings and duties through one Agent-owned pack and
  existing runtime services. Workers operate the artifact rather than define
  it. Separate view, process, turn, duty and data lifetimes; operational claims
  require linked runtime receipts. This reopens design only, not Worker or
  kept-work implementation, public hosting or remote placement. See
  [Kept work](design/103-kept-work-lifecycle.md) and
  [the stage gates](plans/kept-work-plan.md).

- 2026-10-22: The maintainer requested a version bump, release and local
  install of the native channel-card fix. Prepare 8.23.0 through the normal
  release gate and managed installer, then reconcile and verify the installed
  services. Keep real provider/client visual qualification explicit.

- 2026-10-22: The maintainer requested a proper channel-card design and fix,
  with no domain rules, hardcoded subjects or backward-compatibility paths.
  Draw the existing typed forms through shared semantic projection and native
  provider compilers, used for direct replies and proactive pushes. Protocol
  limits cause continuation, never selective truncation. See
  [Native channel cards](design/102-channel-cards.md).

- 2026-10-22: The maintainer requested fixing automatic Topics after an enabled
  conversation remained ungrouped. Add a runtime check for a recorded placement
  decision through the existing correction ladder; keep semantic Topic choices
  with the main model and preserve human controls and the requested answer.
  This does not introduce a background organizer or historical backfill. See
  [Topics](design/101-topics.md).

- 2026-10-22: The maintainer explicitly requested completing installable Canvas
  plugins, isolation, domain commands, undo and Agent context. Implement v1
  through existing package/capability and sealed state/admission services. Use
  owned declarative UI and disposable no-import WebAssembly calculations; keep
  specialized full domain surfaces and physical-device qualification explicit.
  See [contract](design/66-immersive-artifact-canvas.md#installed-workspace-contract-v1-2026-10-22).

- 2026-10-22: The Canvas is a shared workspace host, with domain engines in
  extensions. Research other systems without adopting another agent UI runtime.
  Compiled registration and editing checkpoints are the first foundation;
  executable contributions require an isolated capability boundary. Security
  should preserve useful rendering, interaction and computation through scoped
  authority, not impose domain-specific feature bans. See
  [research](research/canvas-workspaces-2026-10-22.md) and
  [acceptance](audits/canvas-foundation-2026-10-22.md).

- 2026-10-22: For the Canvas workspace redesign, the maintainer requires one
  common web/desktop core and a clean end-state implementation: no stale
  parallel paths, migrations or backward-compatibility code. Remove what a
  change supersedes; plugin unavailability remains an explicit current state.
  See the [review constraints](audits/canvas-workspace-architecture-2026-10-22.md#maintainer-constraint-clean-replacement-no-compatibility-layer).

- 2026-10-22: The maintainer requested dial-style turn navigation with mouse
  and keyboard, retained delayed loading and no added border decoration; use
  existing plan names for previews. See [the contract](design/75-visual-refresh.md#turn-navigator-2026-10-22).

- 2026-10-22: The maintainer approved implementing request-focused Details:
  follow live work, pin inspected history, scope Activity/Files/Evidence to
  the same recorded request, preserve full command and file-review controls,
  and clarify the scope of the other right-panel views. See
  [the Details contract](design/75-visual-refresh.md#request-focused-details--2026-10-22)
  and [browser acceptance](audits/turn-details-2026-10-22.md).

- 2026-10-21: The maintainer broadened Review fixes to all file types and
  requested expert UX focus plus modern, smaller controls throughout the app.
  Review now prioritises document, scope and decision; shared client and Admin
  use compact mouse controls with larger touch targets and readable typography.
  See [the control acceptance](audits/control-density-review-2026-10-21.md).

- 2026-10-21: The maintainer reported generated HTML losing its charts because
  all preview scripts were blocked, requested distinguishing internal/local
  code from external sources, and requested fixing the clipped Review panel.
  The implementation separates script locations without inferring trust from
  authorship, and provides explicit per-document execution with possible
  network access disclosed. Local `vak-home` was authorised for dev checks.
  See [the preview contract](design/66-immersive-artifact-canvas.md#explicit-page-code-permission--2026-10-21).

- 2026-10-21: The maintainer requested implementing automatic work organisation
  under Agents, with people and Agents both first-class, human corrections,
  navigation, settings, administration and archive/reopen. All subjects and
  labels remain dynamic: no domain taxonomy or keyword routing. The everyday
  name is Topics; placement is organisation, never context or permission
  authority. The maintainer also required the shared Doctor to report Topic
  health. See [Topics](design/101-topics.md).

- 2026-10-21: The maintainer approved the clean turn-relations/context/answer-sizing
  redesign, with no compatibility or migration requirement, arbitrary subjects
  and references thousands of turns back. Existing query/open tools resolve
  uncertainty; do not add domain branches or sample-specific fixes. See
  [context](design/68-context-engine.md#current-request-contract-2026-10-21) and
  [the review](audits/turn-relations-and-sizing-2026-10-21.md). The maintainer
  requested documentation updates, then commit and push after verification.

- 2026-10-20: The maintainer requested fixes for the integrity and hostile-prompt
  audit, safe external media, and functioning external hyperlinks on desktop
  and web. Restricted mutation/egress approval and mandatory backend control
  protection take priority over automatic rendering or unqualified execution.
  Full Access remains explicitly trusted. See the
  [follow-through](audits/security-hardening-2026-10-20.md).

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

- 2026-10-20: Prioritize fast everyday voice conversation and study current,
  affordable models across reachable providers, with multilingual coverage
  including Indian languages, using account discovery rather
  than a static model list. Evidence and limits: [voice experience review](audits/voice-experience-2026-10-20.md).

- 2026-10-22 — Investigate installed plain-looking answers and make one proper
  general design correction, using the configured OpenAI Luna route for
  judgment where useful. No domain-hardcoded fix; preserve the recent display
  optimizations. [Evidence](audits/answer-design-contract-2026-10-22.md) and
  [implementation plan](plans/answer-design-contract-plan.md).

- 2026-10-21 — Before launch, replace complete conversation hydration with
  recent history pages and scroll-driven loading; keep model context and
  retention separate. Maintainer approval: “lets do it”. Contract and current
  limits: [web client](design/48-web-client.md#conversation-history-pages);
  original [review](audits/conversation-history-growth-2026-10-21.md).
