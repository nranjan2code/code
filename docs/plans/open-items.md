# Open items

Status: the one tracker of what is not done, kept current. Started
2026-10-14 from the handovers it replaced (git history keeps them). Each
item says what is missing and what closes it. A design doc's own Status
line says what of that doc is built; this file lists only what is open
across them. Nothing here is started unless it says so.

## Before the next release

8.1.0 (2026-10-15), additive within 8.x, with a purge on this Mac
(maintainer's choice: the 8.0.0 home predates the home-version stamp):

- [x] CHANGELOG: the `## 8.1.0` section.
- [x] Version: 8.1.0 (`scripts/bump-version.sh` set Cargo.toml, then
  stopped at its `cargo clean`, which refused a target with no
  `CACHEDIR.TAG`; the badge and lockfile steps were finished by hand).
- [x] `scripts/release.sh`: every gate passed, tagged `v8.1.0`.
- [x] This Mac: a copy of the private deployments kept in
  `~/Documents/Vakyartha-private-deployments-2026-10-15` (this purge did
  not touch `Application Support/Vakyartha`), `vak self uninstall --purge
  --yes`, `dist/8.1.0/vak self install`, the CLI linked again at
  `~/.local/bin/vak`, `vak self services-sync`; `self verify` clean, both
  services running, the home names `8.1.0`, the plugins seeded.
- [ ] The maintainer runs `vak setup` with the provider key; then `doctor`
  in `~/vak-home`. AWS stays on 8.0.0 until the maintainer asks.
- [ ] Seen on 2026-10-15: `vak setup` run from the source checkout set the
  route in that folder's project layer (`.vak/config.toml`, tracked in
  git) and left the Shared layer with no provider, so `doctor` in
  `~/vak-home` still reports none. Setup's route belongs in the Shared
  layer whatever folder it was started from (invariant 27); to find
  where the setup screen chooses the layer. Its terminal also showed
  `WARN`/`INFO` log lines (the agent network broker stopping, a
  lifecycle pass) among the words meant for the person.

8.0.0 (2026-10-14), a major version with a purge (maintainer's choice):

- [x] CHANGELOG: the `## 8.0.0` section.
- [x] Version: 8.0.0; `scripts/bump-version.sh`.
- [x] `scripts/release.sh`: every gate passed, tagged `v8.0.0`.
- [x] This Mac: `vak self uninstall --purge --yes`, then `dist/8.0.0/vak
  self install`, the CLI linked again at `~/.local/bin/vak` (the purge
  removes the link), `vak self services-sync`; `self verify` clean, both
  services running.
- [ ] The maintainer runs `vak setup` with the provider key; then `doctor`
  in `~/vak-home`. AWS is still on 7.1.0.
- [x] The data baseline is 8.0 (`vak_core::baseline`, invariant 29): a
  data home names the version that created it (`home-version`), and sync
  indexes and backup manifests name theirs; anything older, or unnamed, is
  refused with the one message. `TurnCapabilitiesBound.tool_domains` is
  retired. The 8.0.0 homes on the Mac and AWS predate the record: purge
  and set them up again with the next release.

7.3.0 (2026-10-14):

- [x] CHANGELOG: the `## 7.3.0` section.
- [x] Version: 7.3.0 (additive within 7.x); `scripts/bump-version.sh`.
- [x] `scripts/release.sh`: every gate passed, tagged `v7.3.0`.
- [x] Installed over 7.2.0 on the Mac: `self update` refuses an app
  bundle (the feed carries executables only), so `dist/7.3.0/vak self
  install` placed the bundle with the release's frontend, then `self
  services-sync` restarted both services and seeded. The starters moved to
  1.1.0 and `answer-layouts` is recorded as shipped. The social add-ons,
  which the earlier seeding never recorded, were updated once with `vak
  plugins update packages/social-*` (still disabled); every installed
  package and the six standard skills now match the seed manifest. `self
  verify` clean, `doctor` all checks passed in `~/vak-home`.

Carried over from 7.2.0:

- [ ] One hosted route (a wire that enforces tool schemas): one turn each
  for the terminal card and the test-report card, whose schemas changed.
- [ ] Linux: a command stopped at its limit, on the AWS box or the Docker
  lab (the Seatbelt fix is macOS-only; Landlock does not restrict signals).

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

## Card forms (docs/design/92)

- A map, a board, a graph and a form to fill in are not built; places go in
  a list or details with a link. Build one only when real turns show a
  shape the forms cannot carry.
- `isFleetingNarration` (vak-client-ui `structured.ts`) folds a short
  message by a length of 180 characters, a tuned number; replace it with a
  typed marker from the host.
- Cards arriving in one step rests on wording only (the prompt seed and
  the plan's result); measured live 2026-10-14 it batched in four of six
  card-bearing turns. Not a mechanism; revisit if it regresses.
- The next release reads no presentation library saved before this change
  (old primitive names): it is a major version under invariant 29, and the
  dev and installed data homes are set up again.

## Presentations as capabilities (docs/design/91)

- Built (stages 1 to 5). Live, 2026-10-14, the Pune Saturday plan:
  the Agent's route (`gpt-6-luna`) planned, loaded `plan-in-time`, and
  showed a lead, the pack's `at_a_glance` row, an itinerary and a callout
  in the plan's one section; `qwen3.5:4b-mlx` (98 s) showed an itinerary
  and a stats card, with no failed call. In the browser at 1440 × 900 and
  390 × 844 (dark): no sideways scroll, the figure row three columns wide
  and one on a phone, nothing under 12 px. Not verified: light, because
  the app's theme setting overrides the emulated scheme; and the first
  run's route was not read from its receipts. Figures read "3000 ₹", the
  unit after the value.

## Self-evolving capabilities (docs/design/93)

Agreed 2026-10-14, nothing built. Each stage's "done when" is in doc 93
§11; tick here only when it holds with the full gate green and checked
live.

- [x] E0: design agreed, decisions logged, doc 43 superseded.
- [x] E1a: a skill's program (`capability.json`, `vak_tools::capability`)
  runs as a tool named after the skill through `WorkerTask::Capability`,
  under a sandbox built from its declaration (no network, reads the
  workspace, writes only its drafts when declared), JSON in and out,
  refused when it changed since the turn froze it.
- [x] E1b: declared `hosts` through the program's own egress proxy (moved
  from vak-mcp to `vak_tools::egress`, one proxy for both), and pinned
  Python `requirements` installed by their own worker run into a
  content-addressed environment (checked live: `six` from PyPI, first call
  asked, the next ran offline).
- [ ] The install is a worker dispatch inside the call, not a run record
  of its own (`Core::runs()`): it gets one when verification does (E2).
- [ ] Dependencies for Node programs (a lockfile and `npm ci`): an ES
  module ignores `NODE_PATH`, so the environment has to sit where the
  module resolves from; designed when a Node capability needs it.
- [ ] The approval a first install asks reads "'<name>' needs approval:
  <name>": it should say it installs packages from the declared hosts.
  Part of the Review screen (E6).
- [ ] E2: verification (`tests/`, the `VerifyCapability` worker task as a
  run, per-case results on the version).
- [ ] E3: proposal and tiers (`propose_skill` with program, tests,
  presentation packs and `parent`; Library versions; the tier gate; registry epoch; the AGENTS.md
  outcome-runtime paragraph on review of tools).
- [ ] E4: catalog source, prompt index, `find_tools` ranking; the tool
  carries the skill's `serves:` so it defers (E1a loads it every turn).
- [ ] Seen live (E1a, dev route): a card the model drew after the
  program answered showed one per-person figure and dropped the extra
  cent the program gave one person. E9 (card data from the call) is the
  designed fix; until then the card is the model's words.
- [ ] E5: per-version measurement; promotion and withdrawal by observed
  states (R2).
- [ ] E6: client experience (projection line, capability sheet, Review,
  Library Abilities, Settings → Capabilities rewritten, inbox).
- [ ] E7: admin `#/capabilities` and `#/measurements`; `vak skills` verbs.
- [ ] E8: the opt-in review of failures as a trigger per Agent.
- [ ] E9: card data from a capability call (R5), generalising
  `Tool::output_of_command`.

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
