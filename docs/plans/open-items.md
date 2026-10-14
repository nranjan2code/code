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
- The design-doc audit is done (2026-10-14): every doc whose status is
  `implemented` was read against the code and corrected, past sections
  are marked historical, and the bugs and contract gaps it found are fixed
  or listed below. Docs that are `in progress` or `proposal` keep the open
  parts their own Status lines name. A later change keeps them true by the
  rule in `docs/README.md` ("Keeping it current").

## Presentations as capabilities (docs/design/91)

- Built (stages 1 to 5). First live turn, 2026-10-14, home route
  `qwen3.5:0.8b-mlx`, a Saturday plan in Pune: the model loaded
  `how-to-guide` rather than `plan-in-time`, looked for transport it was
  not asked about, sent `emit_steps_card` with no `steps` (the regeneration
  did not recover it), and sent `chart_type: "bar_chart"` beside a chart
  card whose kinds include `bar_chart`. Fixed: the `bar_chart` kind is
  removed, and a call its schema refuses is regenerated at once (doc 89
  §4.4). Open: the same turn again on both local models; the answer seen
  in the browser at 1440 × 900 and 390 × 844, light and dark.

## Installed 7.2.0 run (docs/audits/installed-7.2.0-run-2026-10-14.md)

- The details card offers itself for "a set of readings" and accepts
  `readings`, competing with the stats card: figures were written as
  sentences in a details box.
- `office_apply` `add_image` takes only inline base64 and Word has no
  chart op, so a chart passes through the model's output: one copy broke,
  then the chart was shrunk to fit. Take an image from a workspace or
  draft file, read in the worker.
- Thirteen `recall` calls in one turn, eight repeats: not yet explained.
- The prompt does not name the host platform; the model tried Linux-only
  commands on macOS.
- An accepted document is shown twice (draft card and file card).
- The first request appears twice in the export.

## Approvals

- The approval card offers "Create rule…" on every ask, including a
  provenance ask (`AskSource::Provenance`), which no rule can answer, so a
  rule made there does not stop the next one. The card needs the ask's
  source as a typed field: `Approver::approve` carries only the reason's
  text, across 16 implementations (`ChatPane.tsx` "Create rule…").

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
- `[intent] accept_confidence` is read and shown (`vak intent show`, the
  admin intent policy) but decides nothing since 2026-10-11; within 7.x it
  can only be documented as such (invariant 29). Decide whether to stop
  showing it.
- `vak term`'s approval box takes clicks at fixed screen coordinates; its
  second region closes the box without answering, and which pill is drawn
  there was not checked.
- Two limits for a person's approval: `[gateway] approval_timeout_secs`
  (a forwarded gate, default 300 s) and `[control.limits] approval_secs`
  (every other approval, default 900 s). Decide whether the forwarded
  window is its own limit or the same one (invariant 30).
- The dynamic planner (`vak plan`, `crates/vak-flow/src/planner.rs`) reads
  a TOML flow out of the model's prose and patches raw newlines and nested
  quotes by guessing, with `output_schema: None`. AGENTS.md asks a step
  whose output is data to set `ChatRequest::output_schema` and never to
  guess at broken syntax; doc 11 describes the repair as designed.
- A request carries a tool result verbatim only up to
  `RESULT_WINDOW_CHARS` (30,000) and lines up to 2,000 characters
  (`crates/vak-tools/src/window.rs`), with the rest one `recall` away.
  Nothing is lost, but the numbers are picked in code, against invariant
  36's "no character-count truncation" and "No tuned limits"; the measured
  budget (`CapacityProfile`) is the candidate to decide it.
- `[ui] composer`, `keymap`, `osc52` and `accessibility` parse and are read
  by nothing since the inline TUI was withdrawn; like `accept_confidence`,
  decide whether anything should show or use them.
- The agent loop recognises its own no-op acks by the start of their text
  (`is_no_op_ack`, `DRAFT_REPEAT_ACK`, `DRAFT_COPY_REFUSED` in
  `crates/vak-agent/src/lib.rs`); AGENTS.md asks runtime-authored traffic
  to be typed, never sniffed, so a no-op should be a typed outcome of the
  call.
- An `/until-done` hold is released by a message that is exactly one of
  "done", "stop", "you can stop" or "that's enough"
  (`StopPolicy::is_done_message`): a list of English phrases deciding
  control, where invariant 32 asks for explicit commands
  (`vak_intent::parse_command`).
- `OutcomeSpec.stop` (the reading's stop profile) is computed and narrowed
  for workers, and no gate reads it; decide whether it goes.
- Presentation recipes are planned from words: the server's projection
  calls `signals_from_context` (`crates/vak-server/src/projection.rs`),
  which reads the answer's text for keyword signals, and
  `PresentationPlanner::plan` picks a composition from them
  (docs/design/30-render-architecture.md §30.4). It decides how a result is
  laid out, not what runs, but it is a word list deciding behaviour; and
  `signals_from_text` is otherwise called only by the delivery audit tests.
  Decide whether layout should come from the cards and their types alone.

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
