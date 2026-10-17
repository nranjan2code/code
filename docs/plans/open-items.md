- [x] Live, dev route (2026-10-15): "Pause my morning line." was
  answered "The morning line is paused" with no call. Cause: the set of
  tools a session has loaded ("stays loaded for the session", doc 88
  §12.1) is `Core::inner.loaded_tools`, in process memory; the dev server
  had restarted since the turn that found `automations`, so it was
  deferred again and the turn did not call `find_tools`. Fixed
  2026-10-15: the carried set is `SessionLog::called_tools`, read from
  the ledger (test `vak-core/tests/it/loaded_tools.rs`, a restart
  between the turns).
# Open items

Status: the one tracker of what is not done, kept current. Started
2026-10-14 from the handovers it replaced (git history keeps them). Each
item says what is missing and what closes it. A design doc's own Status
line says what of that doc is built; this file lists only what is open
across them. Nothing here is started unless it says so.

## Installed route failures (qwen3.5:0.8b-mlx, 2026-10-15)

Read from the installed home's ledger through its API, then replayed on a
dev server behind the Ollama capture proxy. Each cause was in Vak's
contract:

- [x] "remember that" never called `remember`: the tool was deferred
  (declared no `always_loaded`), so the request sent 34 tools, 20 of them
  cards, and `remember` only as a "More tools" line "about this workspace
  for FUTURE sessions", one `find_tools` call away. It is loaded now and
  says it keeps what the person asks it to remember.
- [x] "whats my name?" ran 23 steps until stopped: a card refused for a
  `Target` fault (a file preview of a file that does not exist) never moved
  along the repair ladder (`vak_judge::call::repairable` took shape faults
  only), so it was told the same thing at every step. A card tool's target
  faults now move it on (`keeps_failing`); other tools' do not, so a few
  dead pages never withdraw `webfetch` (docs/design/15-reliability.md).
- [x] The same turn's `recall` missed the turn holding the name: a stop-word
  list in `history_query_terms` and a "half the query's terms" rule in both
  `TurnIndex::search` and the catalog's `covers_recall_query` hid it (the
  query had a misspelling and an id the record lacked). Both removed;
  ranking decides order.
- [x] One-cell table rows refused three times: the ladder ran as built
  (told, schema, withdrawn), but the refusal quoted the model's own first
  row as "the header" and the schema's "separate strings, the header first"
  read as one column name per string; the next calls copied both. Both
  rewritten; the refusal names the list and details cards by shape.
- [x] The step decision recorded deferred tools as offered, so the audit
  showed `remember` as callable; `StepContract::deferred` now lists them.
- [ ] Still in the recall path: the catalog's 64-candidate cap, its 250 ms
  wait in code and `limit.clamp(1, 20)` are numbers picked in code
  (AGENTS.md "No word lists, no tuned limits"; doc 88 §5a).
- [ ] `mail_calendar` is loaded, and its long guardrail paragraph is in the
  seed, on every turn with no account connected. Deferring it was tried
  and reverted (2026-10-16): "what's on my calendar today?" reached it
  16/16 loaded, 3/16 deferred, and drew a calendar card with no data
  6/16 (docs/audits/tool-catalogue-2026-10-16.md §6). Its 7,486 chars
  (18 % of tool bytes) need a design that keeps it findable.
- [ ] `remember` writes a workspace note; a fact about the person (a name)
  is a profile fact, and the tool cannot write the profile. Found from
  another workspace only through `session_search` of that space.
- [ ] Inside one conversation, `session_search` leaves the conversation
  out because "its content is already in context" (doc 23), but the
  planner carries only the previous turn's card; older turns are reached
  only through `recall`. The doc's reason is no longer true.
- [ ] Not checked live after the fix on the installed route: needs a
  release, or a dev build against the installed home.
- [x] Grounded cards (2026-10-15): the terminal, test report, diff, file
  preview, sources and the two ability cards load once the conversation
  holds the call they show (`Tool::grounded_in`, `vak_tools::grounds_held`),
  promoted in place the step one appears; AGENTS.md and doc 68 §5 say so.
  Measured before and after on the dev route
  (docs/audits/grounded-cards-2026-10-15.md).
- [ ] The repeated card-level argument descriptions (`deliverable`,
  `replaces`, `beside_previous`, `label`, `title`, about 600 characters on
  each card tool) could be shorter, with their rule stated once in the
  presentation contract; measure separately.
- [ ] OpenAI Responses tool search (gpt-5.4 and later, `defer_loading`,
  hosted or client-executed, namespaces of under ten) is not used: declare
  it in the adapter's reach and learn it from refusal, as `strict_tools`.
- [x] The lookup reflex (docs/audits/history-lookup-2026-10-16.md): not
  the prompt's wording but two look-alike history tools and a discovery
  hop; `session_search` folded into `recall`, `find_tools` removed (tools
  are called by name), Anthropic deferral dropped. Lookups halved; turn-1
  recall 3 to 9 of 16.
- [x] `entity_query` folded into `recall` (2026-10-16): an entity hit
  carries its whole record; turn-1 recall in replays 9 to 13 of 16.
- [x] `open` beside `query` now answers both (live, a search sent with the
  visible turn number never ran).
- [ ] Fresh requests still reach for `recall` first often (`name` 13 of 16
  with the name in the carried card).
- [ ] Measure whether the loop's regeneration repairs the invalid card
  arguments the replay counts (29 of 155 first steps).
- [ ] "Total sales per region in sales.csv": after `read`, the next step
  never computes (0/16 `bash`, with or without `data_query` listed;
  8/16 answer with no call, live it called the data "truncated").
- [x] Tool catalogue audit built (2026-10-16): consolidation's word lists
  removed (O1), the entity store retired into `remember` (O2),
  `data_query` removed (O3); `mail_calendar` deferral reverted (O5).
  Then O4, O6, O7, O9 built, O10 by removing reflection, O8 written as
  a proposal (docs/design/95-one-capability-index.md, maintainer's call).
- [x] `propose_skill`'s Jaccard duplicate screen removed (2026-10-16).
- [x] The `skill` tool defers when every admitted skill is an ability
  (2026-10-16): the date question's `skill` calls 3/16 → 0/16.

## Before the next release

8.3.0 (2026-10-15), additive within 8.x, no purge (the shipped skills):

- [x] CHANGELOG: the `## 8.3.0` section.
- [x] Version: 8.3.0 (`scripts/bump-version.sh`).
- [x] `scripts/release.sh`: every gate passed (`e9bcf11fc`), tagged `v8.3.0`.
- [x] This Mac: `dist/8.3.0/vak self install`, `self services-sync`,
  `self verify` clean; everyday-starter and developer-starter 1.2.0 and
  answer-layouts 1.1.0 seeded; the six 8.2.0 seed copies in
  `~/vak-home/.vak/skills` (unedited) moved out by hand, since nothing
  seeds or removes them now; the server lists 13 skills, one per name.
  The route stays ollama `qwen3.5:0.8b-mlx` (the maintainer's choice).

8.2.0 (2026-10-15), additive within 8.x, no purge:

- [x] CHANGELOG: the `## 8.2.0` section.
- [x] Version: 8.2.0 (`scripts/bump-version.sh`).
- [x] `scripts/release.sh`: every gate passed (`6957c0dd5`), tagged `v8.2.0`;
  artifacts in `dist/8.2.0`.
- [x] This Mac (2026-10-15, maintainer's choice: purge, no upgrade):
  `vak self uninstall --purge --yes` (private deployments copied first to
  `~/Documents/Vakyartha-private-deployments-2026-10-15-8.2`),
  `dist/8.2.0/vak self install`, the CLI linked again at
  `~/.local/bin/vak`, `vak self services-sync`; `self verify` clean, both
  services running, the home names `8.2.0`, the plugins seeded.
- [ ] The maintainer runs `vak setup` with the provider key; then `doctor`
  in `~/vak-home`.
- [ ] AWS: still 8.0.0; purge and install 8.2.0 fresh when the maintainer
  asks.
- [x] Seen before the purge: the 8.1.0 gateway could not read automations
  written by dev builds of the doc 94 work. Expected (maintainer): a home
  a dev build has written is purged, never kept readable by an older
  install.

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

## Capability coverage (docs/audits/capability-coverage-2026-10-16.md)

- [ ] Finish the Ollama run (`python3 scripts/capability_coverage.py run
  --label base --singles 1`, then `replay` and `report`); stopped by the
  maintainer after 8 of 135 cases on 2026-10-16.
- [ ] Run a hosted route once the maintainer sets a key; until then no
  finding can be read as contract (both routes) versus wire capability.
- [ ] Review `scripts/capability_coverage/cases.json` (written by the
  authoring session from the emitted prompts, unreviewed) and regenerate
  it through a hosted route.
- [ ] Investigate from the first cases: file calls naming invented
  absolute paths (`read`'s `path` says "relative to cwd or absolute"), and
  no instruction skill loaded in 6 of 6 skill turns.

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
- Answer projection review (2026-10-16), staged in
  docs/design/96-answer-projection-gaps.md (Q1 to Q3, P1 to P6), each a
  Q1 to Q3 built 2026-10-16:
  - [ ] No live controls: a `controls` form of typed inputs with outputs
    written in a closed set of pure expressions evaluated in the client,
    and a worked example the builder checks. No code runs.
  - [x] A map form (P5) drew in a real turn (2026-10-17): one
    `emit_map_card` call with the model's own coordinates, at 1440×900
    and 390×844, light and dark, tiles off and on; the tile route refuses
    while the switch is off. No change was needed; the earlier turn that
    looked coordinates up on the web did not recur.
  - [ ] webfetch gets 403 from Google where curl with the same User-Agent
    gets 200 (also over HTTP/1.1): reqwest with rustls is refused, so the
    cause is likely the TLS fingerprint, not a header. Not investigated
    further.
  - [x] "Use this" put a fixed sentence in the composer and a list's ticks
    were lost on reload (Q3: `CardAction` entries; a choice starts a turn,
    ticks reach the next turn's tail).
  - [ ] A choice ("Use this") is verified through the HTTP test only: no
    live turn on the dev route drew a steps card with choices. Found
    2026-10-17: the one call that wrote choices put them inside a step
    (`steps[n].choices`, with `fact` and no `label`); the builder dropped
    them and the model was told the card was shown. Fixed: a card tool's
    `call_schema` is closed at every record, so such a field is refused and
    the call written again; the advertised schema stays open
    (`a_field_the_card_does_not_name_is_refused_not_dropped`). The steps
    card's description did not say a step can offer choices; it does now,
    and three fresh runs of "let me choose between French press, pour-over
    and moka pot" each drew a steps card (two before drew details cards),
    but none wrote `choices`: each put the alternatives in a step's text.
    Open for the maintainer: choices nested in their step is the shape
    the model reached for, and the flat-schema rule (no list of records
    inside a list of records, doc 30 §30.1) forbids it.
  - [x] A tick while a turn runs says why (checked 2026-10-17): the tick
    goes back and the notice reads "Agent is working; try again after this
    turn settles". The open line was stale.
  - [x] A card shows nothing until its call is complete and valid (Q2:
    `CardDrafting` and the empty card shell). On the dev route a tool call
    arrives whole, so the shell stands only while the call is checked.
  - [x] The shell is superseded: recorded cards are drawn live in the
    running turn (`CardRecorded`, `LiveAnswer`) and the plan's progress is
    the presence wheel (2026-10-16).
  - [x] A page reloaded mid-turn shows that turn's earlier cards: did not
    reproduce (2026-10-17). The running snapshot is the handle's live
    timeline and carries every recorded card; a reload drew them at once.
    Pinned by `a_running_turns_timeline_carries_its_recorded_cards`.
  - [x] A page reloaded while a turn waits on an approval or a worker's
    question shows the card at once (2026-10-17; 10.2 s before, 0.4 s
    after, live on the dev route). Cause: on a fresh page the conversation
    has no item list yet, so restoring the gates threw and was logged as a
    warning; the card came back only on the next heartbeat. The restores
    now start from an empty list, a transcript keeps unanswered gates
    (`keepLiveGates`), and opening a running conversation asks for them.
    The question card shares the path and was not seen live.
  - [x] Markup previews get none of the theme's tokens (Q1).
  - [ ] Serve pinned vendored libraries from the preview origin, keeping
    `connect-src` closed (P3).
- Found live 2026-10-16 while checking Q1 and Q2:
  - [x] After a card call returns "Shown to the user", the next step wrote
    the card's content again as text (or sent the card again). Cause: the
    step itself, not the prompt, result, thinking or sampling; every card
    now says `then: more | done` and the turn ends on `done`
    (docs/design/97-cards-end-the-answer.md).
  - [ ] `mail_calendar` (18% of tool bytes) loaded only when an account is
    connected needs a connected-accounts rollup (doc 97 §7).
  - [ ] Composition skills (`answer-layouts`) reach only a model that calls
    the skill loader; the small route never does (doc 97 §7).
  - [ ] Whether the plan should be the answer's outline (doc 97 §7).
  - [x] Details → Activity lists only sandbox executions (fixed: it shows
    each turn's steps; plan, card and recall calls report start and end).
  - [ ] A card refused in an earlier turn reads "Used emit chart card" in
    Activity: no card was recorded for it and the client has only the
    tool's name.
  - [ ] A page bound to its data: a Library artifact of composed cards
    whose new version an automation's run produces, shared through grants.
  - [ ] Decide whether `ui://` HTML resources from MCP servers are drawn
    by Vakyartha's own renderer in the opaque sandbox, with calls from the
    UI going through the permission engine, or stay text only. Record the
    choice in docs/decisions.md.
  - [ ] Measure format choice: a live eval of whether a card helped the
    request, reported beside cost.
- Media card checked live 2026-10-16 (image and video by URL, an image in
  the workspace):
  - [x] A workspace path drew an empty card: the renderer drew only web
    addresses. A path with no scheme is now read through the authenticated
    file route (`mediaSource`, `tests/media-source.mjs`).
  - [x] The desktop shell's policy refused web images and every video
    (`img-src` without web schemes, no `media-src`); fixed and pinned
    (`the_shell_policy_admits_what_a_media_card_draws`).
  - [ ] The desktop shell drawing a media card is not seen in the running
    app (checked in a browser only).
  - [ ] A media card naming a file that is not in the workspace is
    recorded and then says the file could not be read; the builder does
    not check the path as a file preview's grounding does.

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
- [x] The approval a first install asks reads "'<name>' needs approval:
  <name>": it should say it installs packages from the declared hosts.
  Closed by E3b (the approval carries `Tool::reaches`, "Installs its
  packages from <hosts>, once"); pinned by a test 2026-10-15.
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
- [x] E3b (2026-10-15): `[owner] abilities_go_live` (on by default; a
  narrower layer can only turn it off; `PUT /config/owner`, read live),
  the switch in Settings → Capabilities → Manage → Skills;
  `Program::reaches` as the one wording, declared as `Tool::reaches` and
  carried by an approval's reason; a proposal waiting for review records
  `ProgramReview` (why, reaches, Library version) and writes a
  `ProposalOpened` inbox entry, cleared when the owner approves or
  declines (`Core::proposal_settled`); packs in a skill's
  `presentation/` checked at proposal and active from the Agent's own
  skills. Live with gpt-6-luna (switch off in the folder's layer): a
  kilometres-to-miles ability passed its test and waited "because the
  owner reviews every new ability", with its two reach lines in the
  proposal and the inbox; declining cleared the entry. The switch toggled
  in the running app at 1440 × 900 dark and persisted to the Shared
  layer alone.
- [ ] Found live (E3b): Settings lists the proposals of the Agent's own
  folder, but a proposal is filed under the folder the turn ran in, so
  one made in another folder never shows there (the API for that folder
  lists it). Review belongs to E6, drawn from the Library's waiting
  versions rather than a list per folder.
- [x] E4 (2026-10-15): `Tool::serves` returns `Vec<&str>` borrowed from
  the tool, so a program's tool carries its skill's `serves:`
  (`propose_skill` takes `serves`) and defers; the prompt's catalogue
  lists the turn's program tools; loading the skill shows its program's
  files and how to improve it; `Origin::Agent` names an Agent's own
  skill. Live with gpt-6-luna on the installed home: a bill splitter
  built with `serves: [code-exec, planning-and-organizing]`; a fresh
  conversation asked about a dinner bill, loaded the skill, found the
  tool with `find_tools` and answered to the cent from it; "improve your
  ability with weights" wrote and tested a copy and proposed it again,
  recorded as versions 2 to 4, each naming its parent. Found and fixed on
  the way: the skill block called the Agent's own ability `workspace`
  (the model prepared a copy for the owner instead of proposing); a
  `recall` search of the same conversation was labelled someone else's
  words, sending the improvement to Review (it now holds what the
  conversation's turns held); verifying the same program twice failed on
  its kept record (each run is now a new version of it).
- [ ] Seen live (E4): before proposing a new version of a live ability the
  model asked for confirmation once ("Confirmation before activating
  version" in its plan). Nothing in Vak asks it to; left as observed.
- [x] Seen live (E1a, dev route): a card the model drew after the
  program answered showed one per-person figure and dropped the extra
  cent the program gave one person. Closed by E9: a table or chart of an
  ability's answer is filled from that answer.
- [x] E5 (2026-10-15): `AbilityRecord` per skill
  (`agents/{agent}/ability-standing`), settled at each turn's end from
  that turn's program calls; `vak_judge::ability::verdict` offers a
  version that passed, worked in another turn and never failed
  (`AbilityOffered`, `POST /skills/abilities/{name}/promote`) and
  withdraws a fix that fails after its parent failed
  (`AbilityWithdrawn`); a program refuses arguments by exit status 2
  (`Fault::Target`), which never counts against it. Live with
  gpt-6-luna: `weighted-bill-splitter` made in one conversation, called
  in another ($90 weighted 1:2:2 gave 18, 36, 36 dollars), and the inbox
  offered it to every Agent. The withdrawal ladder is covered by the
  integration test, not seen live; the offer was left for the owner.
- [x] E6a (2026-10-15): the Library's Learned abilities view
  (`GET /skills/abilities`, `Core::abilities`), proposals listed and acted
  on across folders (fixes the E3b finding), and an ability's versions
  kept like a starred artifact's. Checked in the running app on the
  installed home at 1440 × 900 light and 390 × 844 dark: three abilities,
  Working and Ready for every Agent, with what they reach.
- [ ] Found (E6a): every ability version was a draft (nobody "saved" it),
  so the lifecycle pass trashed an ability's Library history after the
  drafts' window; fixed by `Artifact::keeps_every_version`. On the
  installed home the history of the abilities made on 2026-10-15 was
  already trashed (their cards show no versions link); the installed
  copies themselves are untouched.
- [x] E6b (2026-10-15): Approve for this Agent (`Core::approve_for_agent`,
  `POST /skills/proposals/{id}/approve`; approving a program proposal
  now replaces the version it improves); the inbox's proposal and
  ability entries ("See learned abilities") and Settings → Capabilities
  ("Open in Library") open the Learned abilities view. Checked in the
  running app at 1440 × 900 dark.
- [x] E6c (2026-10-15): the ability line under an answer: an `Activity`
  of kind `Ability` (event, skill, version) written at turn end from the
  turn's own program calls, records and proposals, projected as
  `OutputKind::Ability` and drawn as one quiet line that opens Learned
  abilities. Live with gpt-6-luna: "Five of us split a $123.45 dinner
  with a 15% tip" called restaurant-bill-splitter, answered $28.40 × 2
  and $28.39 × 3, and showed "Used restaurant-bill-splitter" (1440 × 900
  light; 390 × 844 dark checked through the DOM). Learned and improved
  covered by the integration test.
- [x] E6 rest (E6d, 2026-10-15): the capability sheet the line, the
  Library, Settings and the inbox open (versions, tests, uses, Use this
  version, Turn off, Review); waiting abilities in the Library's Needs
  you; filters by name, Agent and state; Settings → Capabilities as
  Learned, Installed, Connections and Discover. Checked in the running
  app on the installed home.
- [x] A conversation in the catalog has no title, so the sheet names a
  use by the sidebar's title or its Agent; give session nodes their
  title. Done 2026-10-15: `MessageRecord::conversation_title` is the one
  wording (the sidebar's list uses it too); the catalog sets a session
  node's title from its first such message (schema 6, rebuilt once).
- [x] E7 (2026-10-15): admin Learned abilities (`#/capabilities`) with
  revoke, the per-version table in How the models did, `vak skills`
  verbs replacing `skills-review`. Checked against the installed home.
- [x] E8 (2026-10-15): the opt-in nightly review of failures, one
  `ability_review` automation per Agent; checked live (failure, offer,
  review run, fix waiting, approved).
- [x] E9 (2026-10-15): `emit_ability_table_card` and
  `emit_ability_chart_card` fill their rows or points from the named
  ability's latest answer (`Tool::data_of_call`); checked live: a
  300-row table from `powers-table`, every value equal to its answer,
  chosen by the model unasked.

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
- [x] A run in a copy of a folder read its configuration from the copy,
  which has no `.vak` (2026-10-15): `spawn_isolated_run` gives the copy
  the folder's configuration layer (`WorkspaceScope::give_layer_to`,
  never control state), which replaces the route pin added earlier.
- [x] A delivered run's cards are drawn as cards in the home
  conversation (2026-10-15): `PresentationSource::Scheduled`,
  `ScheduledMeta::narration`; the model still reads their text.
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
- [x] Live, dev route (2026-10-15): "Pause my morning line." was
  answered with no call while `automations` was a deferred tool; the
  conversation now keeps the tools it called loaded from its ledger
  (`SessionLog::called_tools`). Checked again after a restart
  (2026-10-15): the same request listed and paused it. "Run my morning
  line now" on the paused automation called `resume`: the `action`
  field said only "What to do", so for a paused one run-now and resume
  read alike; it now says what each does (`run_now` runs once, paused or
  not, and leaves the schedule as it is), and "pause it again, then run
  my morning line now" called pause then `run_now`. The run's answer (a
  summary card) came back as a card, but its prompt was drawn as the
  owner's bubble: the projection put the narration in place of the
  prompt's text too (an all-card answer has none, so the head item was
  dropped), and the chat draws people's turns from `/transcript`, whose
  entries did not say an automation wrote one. The narration now
  replaces only the answer; transcript entries carry `scheduled` and
  `at`, and the chat draws the shared `ScheduledHead`. Seen in the
  running app: "Scheduled · morning line · Thu 11:55" over the card.
- [x] S5 (2026-10-15): the Automations page (`AutomationsPage.tsx`,
  from the sidebar and the Inbox) replaces `AutomationsModal`: each
  automation as its card (the same buttons as in chat), its history, and
  "Open its conversation"; this Agent's or all Agents'; New automation
  with name, what it does (a shell command behind Show technical
  details) and the schedule picker, delivered to the conversation open
  now. The client's own schedule wording (`cadenceWords`,
  `intervalWords`, `cadenceBadge`, `scheduleZone`) is removed for the
  server's `when_words`, and the modal's styles with it. Delete confirms
  in place; a refresh keeps each card (reconciled by id). Checked in the
  running app at 1440 × 900 dark and 390 × 844 dark: create (every 2
  hours), history, delete.

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
  (Pip's and Tavi's tuned sounds first). Batch 5 (2026-10-15) added each
  companion's own spoken lines and completed every sound family, two takes
  each, to choose from too; nothing in it is wired into the app yet.
  Batch 6 added the Open Graph card and three feature illustrations (four
  takes each); the spot has a v2 cut with the card landing
  (`docs/video/spot-2026/`), and the "Heard" film's two generated shots
  are made (`docs/video/heard-2026/`), its screen recordings and edit
  not yet. The
  generation subscription ends on 2026-10-16, so nothing can be generated
  again in a companion's voice after that.
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
