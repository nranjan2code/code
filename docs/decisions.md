# Decisions

Status: the log of the maintainer's product and architecture decisions,
newest first. Each line says what was decided and where it is recorded in
full; the recording document is the authority. Add a line whenever the
maintainer decides something a later session might otherwise ask again or
undo, in the same commit that records it. Do not ask again about anything
listed here unless the maintainer reopens it.

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
