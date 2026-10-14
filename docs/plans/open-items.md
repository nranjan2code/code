# Open items

Status: the one tracker of what is not done, kept current. Started
2026-10-14 from the handovers it replaced (git history keeps them). Each
item says what is missing and what closes it. A design doc's own Status
line says what of that doc is built; this file lists only what is open
across them. Nothing here is started unless it says so.

## Before the next release

- [x] CHANGELOG: entries under `## Unreleased` for the work since `v7.1.0`
  (2026-10-14).
- [ ] Version: 7.2.0 (additive within 7.x); `scripts/bump-version.sh`.
- [ ] Upgrade from a real 7.1.0 data home: the catalog rebuilds once
  (stamp 4) and new artifact writes go through `Artifacts::transaction`
  on an existing chain. Install over 7.1.0 and open the app, Library and
  Lineage.
- [ ] One hosted route (a wire that enforces tool schemas): one turn each
  for the terminal card and the test-report card, whose schemas changed.
- [ ] Linux: a command stopped at its limit, on the AWS box or the Docker
  lab (the Seatbelt fix is macOS-only; Landlock does not restrict signals).
- [ ] `scripts/release.sh` (isolated target, clean tree, 15 GB free).

## Seen only in tests

- The unknown-name answer (`unknown_capability`, and `use_instead` for a
  retired name): the local model never calls an unknown name live.
- Settings › Delete saved copies: needs a real provider account,
  connected and then disconnected.
- A real network share or cloud-synced folder as the sync remote (the
  Docker lab and a Mac with a Linux machine passed, 2026-10-09).
- One-person erasure through a real channel (a gateway test covers it).
- The desktop shell for the brand media (seen in a browser only).

## Not yet run

- The 24-hour mail and calendar soak (`scripts/mail-soak`, README there).
  A full run was stopped by the maintainer after 15 minutes on
  2026-10-10. Run: `sh scripts/mail-soak/build.sh`, `sh up.sh`,
  `caffeinate -i python3 soak.py --hours 24`, record the report in the
  mail and calendar plan and AGENTS.md's mail paragraph, `sh down.sh`.
- The rest of the design-doc audit. Done 2026-10-14: every Status line
  starts with a status word (`docs/design/INDEX.md`), and 73, 74, 76, 80,
  82, 86, 88 and 54 were checked against the code and corrected. Not done:
  the 53 docs marked `implemented` were not each checked against the code
  (only their status word was set), and the in-progress docs not listed
  above keep the open parts their own Status lines name.
  A sweep on 2026-10-14 for code names these docs cite that no longer
  exist found and corrected stale claims in 03, 13, 22, 23, 28, 30, 33,
  34, 41, 42, 47, 64, 65 and 66 and in AGENTS.md invariant 32; past
  sections of 23, 29, 34, 47 and 48 are marked historical. Still to do: a
  reading of each implemented doc for behaviour that changed under names
  that still exist, which a name sweep cannot find.

## Needs the maintainer

- V4.3: drag, double-click and full screen in the running desktop app
  (built; no mouse input to native windows here).
- V4.6: who draws the character glyphs. V4.5 (close out the visual
  refresh: doc 75 to shipped with evidence) follows V4.3 and V4.6.
- Brand media: listen and choose a sound family, hero image and theme
  take, and fill the "Chosen" table in `docs/brand/media/README.md`
  (Pip's and Tavi's tuned sounds first). The generation subscription
  ends around 2026-10-16.
- Sync: moving one conversation alone, and whether provider keys may
  travel (today no secret is ever written to the remote).
- Mail and calendar: live provider accounts for conformance and for
  Delete saved copies.
- Measured patterns waiting in `/eval/proposals`: the next round's
  backlog, by the maintainer's stopping rule of 2026-10-13.

## Not audited

- Other tables of names or words that decide behaviour (AGENTS.md "No
  word lists"): two were removed on 2026-10-14 (`canonical_tool_name`,
  the projection's `artifact_from_tool`); nobody has swept the tree.
- `vak_delivery::skills::signals_from_text` reads words and is now called
  only by the delivery audit tests; decide whether it and those audits
  should go.

## Known limits in the product

- Hand-over moves all the work; one conversation cannot move alone.
- On a cloud-synced folder the lease is re-checked just before a push
  counts, which narrows a simultaneous-push race and does not remove it.
- Provider keys and bot tokens do not travel between machines.
- The desktop shell asks to be started again after a pull or restore
  (`vak serve` starts itself again).
- A test-report card's per-test rows are the model's words; only how the
  run ended is taken from the call.

## Not built, by the maintainer's decisions

An S3 or Postgres remote, hosted key escrow, retention labels on single
items, data roles, an erasure-request queue, audit export, merging two
machines that both wrote; `recall` opening another conversation except
through an attached artifact (2026-10-14).

## Recorded model choices (no work unless asked)

A model sometimes invents a folder or writes after the work is done; a
table's header and row joined by a newline in one string, or a one-row
table with no header, are told and fixed on the next call; prose can claim
what no call did (the "not verified" banner warns).
