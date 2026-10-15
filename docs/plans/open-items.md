- [ ] Live, dev route (2026-10-15): "Pause my morning line." was
  answered "The morning line is paused" with no call. Cause: the set of
  tools a session has loaded ("stays loaded for the session", doc 88
  §12.1) is `Core::inner.loaded_tools`, in process memory; the dev server
  had restarted since the turn that found `automations`, so it was
  deferred again and the turn did not call `find_tools`. Derive the
  carried set from the session's ledger (the tools its turns called or
  loaded) rather than memory, so a restart changes nothing.
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
- [x] Seen on 2026-10-15, fixed the same day: `vak setup` run from the
  source checkout set the route in that folder's project layer. The setup
  screen saved the route (and its access level step, through
  `/config/mode`) to `"project"`; both now save to the Shared layer
  (`PATCH /config/global`), checked in the running admin from a fresh
  project folder (the Shared file changed, the project got no `.vak`).
  Its terminal showed log lines: the CLI's terminal echo now shows
  warnings and errors only unless `VAK_LOG` is set, and a second server
  finding the agent network broker served by the gateway reports
  `Served::Elsewhere` at info instead of a warning that it stopped.
- [x] The maintainer ran `vak setup` (2026-10-15): the Shared route is
  ollama `qwen3.5:0.8b-mlx` with full access, by their choice.

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
- [x] The install's run: it happens inside the call that needs it, so it
  belongs to that call's run (a turn's, or a verification's since E2),
  under its span; it needs no run of its own.
- [ ] Dependencies for Node programs (a lockfile and `npm ci`): an ES
  module ignores `NODE_PATH`, so the environment has to sit where the
  module resolves from; designed when a Node capability needs it.
- [ ] The approval a first install asks reads "'<name>' needs approval:
  <name>": it should say it installs packages from the declared hosts.
  Part of the Review screen (E6).
- [x] E2: verification (2026-10-15): `tests/<name>.json` cases, run by
  `Core::verify_skill_program` (`vak skills verify <name>`) in a run of
  their own through the call path, per-case verdicts kept per program
  digest; a program with no tests is refused with what to add. Checked
  live on the installed home (one case passed, one reported where it
  differed, exit 1, run id printed).
- [x] E3a (2026-10-15): `propose_skill` with `source` (a folder the Agent
  wrote); the skill a Library version of kind `Skill` (a bundle), parent
  its latest version, with a typed `Verified` row; the tier gate (Live in
  `agents/<id>/skills`, which discovery now reads after the project's;
  Review naming the version, which promotion installs; Failed with each
  case); `capability::FORMAT` in the description and refusals; AGENTS.md
  paragraph. Checked live with gpt-6-luna: built, 4 of 4 tests, live, and
  a fresh conversation used it.
- [ ] E3b: the owner's switch "Let Agents switch on new abilities that only
  work with your files" (on by default; off sends every proposal to
  Review), an inbox entry for a program waiting for review, and the
  approval and Review wording saying what it reaches. Presentation packs
  in a skill (`presentation/`) are carried in the bundle but not yet
  activated from an Agent's own skills.
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

## Scheduled work (docs/design/94)

Agreed 2026-10-15. Each stage's "done when" is in doc 94 §7.

- [x] `vak inbox` read the Agent's own scope while the scheduler and the
  server use the shared one, so it showed nothing; it reads the shared
  inbox now (2026-10-15).
- [x] S0 rules (2026-10-15): `[owner] timezone` (`Core::owner_timezone`,
  default this machine's) named in the time line on every surface; every
  write path fills a cron's zone with it (`Trigger::in_zone`: the tool,
  the server's create and change, the CLI) and `validate` refuses a cron
  without one; the tool's `at` is the person's local time
  (`triggers::local_instant`, a past time refused with the time now),
  replacing `due_at`; a slot in a skipped hour runs at the first minute
  after it. Fixed-clock tests for the gap, the repeated hour, `at` and
  the zone.
- [x] S0 rest (2026-10-15): Settings → General → Your time zone
  (`PUT /config/owner`, Shared layer only; `GET /config` `owner`), read
  live across processes (`Core::owner_timezone_set`). The interval anchor,
  catch-up and one start per slot across two processes were already
  pinned (`triggers.rs` tests, `vak-core/tests/it/claims.rs`,
  `scheduler_personal_os.rs`); intake makes intervals and mail routines go
  through the automations route, so no maker leaves a cron without a zone.
- [x] S1 (2026-10-15): `Trigger::home` (set from the call's conversation,
  or `home` on the server's create and change) and `made_by` (the turn,
  or the owner); `vak_core::scheduled::deliver` appends a prompt run's
  answer, a refusal's reason or a script's output to the home
  conversation as a turn marked `MessageMeta::scheduled`, at a turn
  boundary, with the run's provenance; the client draws a "Scheduled"
  head. Tests in `vak-server/tests/it/scheduled_runs.rs`.
- [x] Live on the dev route (2026-10-15): "every day at 7am … morning
  line" made it with its home; "move my morning line to 9, then run it
  now" changed and ran it; the answer arrived as a Scheduled turn and
  "make it shorter" followed it. Found and fixed on the way: every
  automations call asked and could not be remembered (it declared no
  reach; now `Reach::Schedule`: a store write, or the command its
  `script` schedules); a run in a copy of a folder had no route (the
  copy holds no `.vak`; it takes the folder's live route); and a turn
  whose dispatch failed was reported "completed" (`begin_turn` now
  reports the outcome).
- [ ] A run in a copy of a folder still reads the rest of its
  configuration (permission mode, prompt layers) from the copy, which
  has no `.vak`: give it the folder's project layer.
- [ ] Draw a delivered run's cards as cards in the home conversation
  (they arrive in their text form today).
- [x] S2 (2026-10-15): `triggers::describe` (`Trigger::when_words`) is the
  one wording, used by the tool's results, `vak triggers list`/`add` and
  the server's `when_words`; `triggers::Repeat` is the picker's
  vocabulary (once, daily, weekdays, weekly, every N minutes or hours,
  custom cron), accepted as `repeat` by the server's create and change.
  The picker screen itself comes with S5.
- [x] S3 (2026-10-15): the automations tool is list, add, change,
  pause, resume, run_now and remove, naming one by name or id
  (`automation`; a name two share is refused with both ids) and
  answering in `describe`'s words; `change` takes any field and records
  `made_by`. `run_now` goes through `triggers::Runner`, which the server
  installs in `start_scheduler` (pooled and Agent Cores adopt it), so it
  takes the same claim as `POST /triggers/{id}/run`. `vak triggers`
  says pause and resume too.
- [x] S4 (2026-10-15): the automation card in chat
  (`AutomationCard.tsx`) under the turn its `made_by` names, drawn from
  the trigger record: plain words, next run, last run, Pause or Resume,
  Run now, Change time (`SchedulePicker.tsx`, posts `repeat`) and
  Delete. The owner's changes from a screen leave `made_by` alone, so a
  card stays under its turn. Checked live at 1440 × 900 light and
  390 × 844 dark (change time, pause and resume from the card).
- [ ] Live, dev route (2026-10-15): "Pause my morning line." was
  answered "The morning line is paused" with no call (a callout card
  only); `automations` is a deferred tool there (reached through
  `find_tools`, which the turn did not call). Find what in the tool
  surface left it out of reach (doc 68 §5: whether the More-tools
  catalogue names it, and whether a conversation that owns automations
  should load the tool) before calling it model behaviour.
- [ ] S5: the Automations page (replaces `AutomationsModal`).

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
