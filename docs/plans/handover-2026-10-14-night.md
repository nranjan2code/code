# Handover — 2026-10-14 night: self-evolving capabilities designed

The one handover. Earlier handovers were removed on 2026-10-14 (git
history keeps them); every note of theirs that is still true is under
"Working notes" below.

## Prompt for a fresh session

"Continue in /Users/nisheethranjan/Projects/Vak/code on main. Read
AGENTS.md first (especially 'The data rules', 'No word lists, no tuned
limits', 'A tool's permission is what it declares', 'Investigating a
failure' and 'Verification before every commit'), then
docs/plans/handover-2026-10-14-night.md whole.

Run `git status` and `git log --oneline -8`. If the newest commit is not
'Handover: 8.3.0 released and installed', read 'Where things stand' and finish what
it says is unfinished. Stage only your own paths; keep only `main`; check `git show
--stat` before every push. We are on a Claude Code Pro plan, so keep
token use to what the task needs: read with grep and line ranges, never
whole large files; batch independent commands; filter command output;
run the full gate once per finished step, in the background, using
`scripts/gate.sh --changed` while working; commit each verified step;
tick docs/plans/open-items.md; and rewrite this handover's prompt and
'Where things stand' before the session's limit. The maintainer is the
only user: rewrite a wrong design or screen rather than patch it.

Self-evolving capabilities (doc 93) are implemented, E1 to E9. What is
left is in docs/plans/open-items.md 'Self-evolving capabilities' (the
open lines: an Agent asking before proposing a new version (observed,
nothing in Vak asks it), ability history lost before E6a on the
installed home (data only; fixed for new versions), Node dependencies
(designed when needed)) and the release questions under 'Next', which are
the maintainer's to decide; ask before cutting a release. 8.2.0 is
released and tagged; installed fresh on this Mac (purged); `vak setup` is
the maintainer's next step; AWS waits for the maintainer.

Live runs: the installed home (8.2.0, fresh; once the maintainer has
run setup with the OpenAI key) may be used sparingly: launch entry `vak-e3-live` (a dev server on port 8907 in
/tmp/vak-live/e3 against the real home; token in
/tmp/vak-live/test-gateway-token; /tmp/vak-live/turn.sh with PORT=8907
for a new conversation, /tmp/vak-live/turn-same.sh <session> for a
follow-up). For the local model, launch entry `vak-e1-live` (port 8905,
throwaway VAK_HOME, PATCH /config to ollama qwen3.5:4b-mlx). Short
progress lines."

## Where things stand

**8.3.0 released and installed on this Mac** (2026-10-15, `e9bcf11fc`,
tag `v8.3.0`): the shipped skills are real procedures in the packages
(the `seed.rs` skill table is gone), with two tested programs,
`date-calculator` and `table-profile`. The installed route is ollama
`qwen3.5:0.8b-mlx` with no OpenAI key, by the maintainer's choice:
Vakyartha must work with the smallest model, so failures seen there are
Vak's contract to investigate, never a reason to change the route.
Seen on that route before 8.3.0 and investigated on 2026-10-15 (open-items
"Installed route failures"): `remember` was deferred behind `find_tools`;
target faults never moved the repair ladder, so a refused file preview
repeated until the turn was stopped; recall's word list and "half the
terms" rule hid the turn holding the name; the table refusal's wording led
to the same mistake. Each is fixed in Vak's contract (unreleased); the
open lines there are what the investigation found and did not fix.

**8.2.0 released 2026-10-15** (`6957c0dd5`, tag `v8.2.0`, artifacts in
`dist/8.2.0`, feed at the local 127.0.0.1:8899 URL like 8.1.0): doc 93
E2 to E9, scheduled work (doc 94) and catalog titles. Additive within
8.x, no purge required. This Mac was purged and installed fresh at 8.2.0 the same day
(maintainer's choice; `self verify` clean, services running, home names
8.2.0). The maintainer runs `vak setup` with the provider key next; until
then the installed home has no key and no route. AWS still runs 8.0.0. AWS's 8.0.0 home predates the home-version stamp, so it
needs a purge whenever it is next installed.

**Catalog session titles** (2026-10-15, after E9):
`MessageRecord::conversation_title` is the one wording for a
conversation's name (first line of the first user words, as the
sidebar showed it); the server's session list and the catalog both use
it, and a session node takes its title from its first such message
(catalog schema 6, rebuilt once on open). The ability sheet already
read the node's title, so its uses are now named. The first-install
approval wording item was already closed by E3b; a test pins it.
Checked live on the installed home (port 8907): the sheets of
`restaurant-bill-splitter` and `powers-table` list their uses by the
conversations' first words.

**Scheduled work (doc 94) is built, S0 to S5** (2026-10-15, each step
gated and pushed; checked live on the dev route, port 8905, throwaway
home /tmp/vak-live/e1-home). The owner's time zone is one setting
(Settings > General > Your time zone, `PUT /config/owner`, read live).
A trigger names its `home` conversation and `made_by` (the Agent's turn
that made or last changed it; the owner's screen edits leave it alone).
A prompt run's answer, a refusal's reason or a script's output is
appended to the home conversation as a turn of its own
(`vak_core::scheduled::deliver`, `MessageMeta::scheduled`, turn card,
the run's provenance), drawn with a "Scheduled" head; replies follow it.
`triggers::describe` (`when_words`) is the one wording; `triggers::Repeat`
is the picker's vocabulary (`repeat` on the server's create and change).
The automations tool is list, add, change, pause, resume, run_now and
remove by name or id; `run_now` goes through `triggers::Runner`, which
`start_scheduler` installs. `Reach::Schedule`: managing automations no
longer asks; a `script` is judged as that command. The card in chat
(`AutomationCard.tsx`) and the Automations page (`AutomationsPage.tsx`,
replacing the modal) share one set of buttons and the `SchedulePicker`.
Found live and fixed: a copy run had no route; a failed dispatch was
reported "completed" (`begin_turn` now reports the outcome); the claims
restart test was timing-dependent. Its three follow-ups were built on
2026-10-15 (`5754c3c0c`, full gate green; not yet checked live): the
tools a conversation keeps loaded are `SessionLog::called_tools`, read
from the ledger (they lived in process memory and a restart lost them);
a run's working copy gets the folder's configuration layer
(`WorkspaceScope::give_layer_to`, never control state; replaces the
route pin); a scheduled run's cards are drawn as cards in the home turn
(`PresentationSource::Scheduled`, `ScheduledMeta::narration`, the
model still reads their text). Left on the dev home: the "morning line"
automation and its test conversation.

**Self-evolving capabilities (doc 93) are implemented** (E1 to E9,
2026-10-15), each pushed with the full gate green; E9 is the newest
commit. One server test now skips a card tool that declares
`data_of_call` (it required rendering from a call's arguments alone). (tracker lines in
open-items "Self-evolving capabilities").

**E6d is built** (checked in the running app on the installed home,
1440 × 900 and 390 × 844, light and dark): `Core::ability_sheet`
(`vak-core/src/ability_sheet.rs`, `GET /skills/abilities/{name}`),
Use this version and Turn off routes, `Catalog::calls_of` with call
status (catalog schema 5, rebuilt once on first open), the sheet
(`AbilitySheet.tsx`, mounted in App, `openAbilitySheet`), the Library's
filters and Needs you, the Settings rewrite (`LearnedByAgents`),
`inbox::Entry::ability`, `.modal-back` z-index above Settings. E7
design noted: CLI verbs list/show/use/approve/decline/promote/turn-off
on the same Core functions, removing `skills-review` (AGENTS.md layout
and docs 26, 29 name it); admin inventory over
`agent_definitions::effective` with each Agent's scoped Core; revoke
only Agent-made abilities; per-version calls by time window from
`calls_of`. Left on the installed home: temperature-converter versions
1 (tests failed) and 2 (in use), its test conversation; its record has
three entries for version 2 from before re-use stopped adding one.

**E9 is built** (checked live on the installed home with gpt-6-luna):
`emit_ability_table_card` and `emit_ability_chart_card`
(`vak-core/src/presentation_tools.rs`), `Tool::data_of_call`,
`fill_from_data`, `answers_data`, `with_call_data` in `vak-agent`,
`SessionLog::answer_of` and `tools_answered`. A fresh conversation
asking for powers 1 to 300 as a table called `powers-table` and chose
the ability table card unasked; all 300 stored rows equal the answer.
Left on the installed home: `powers-table` (offered to every Agent) and
its conversations.

**E8 is built** (checked live on the installed home with gpt-6-luna):
`vak-core/src/ability_review.rs`, `TriggerAction::AbilityReview`, the
review branch in `automations::fire`, `/skills/review`, the inbox
offer, and the Settings switch. Found and fixed: an ability's Library
artifact was keyed by the proposing folder's space, so a fix made in a
copy became a second artifact (now `Core::ability_space`, bound first:
an unbound folder's key changes once something binds it). Left on the
installed home: `list-average` (its fix approved; its older versions
split across two artifacts from before the fix), the review automation
(turned off), and the test conversations.

**E7 is built** (checked on the installed home): `/admin/api/capabilities`
and its revoke route (`vak-server/src/admin.rs`), `Core::ability_measure`
and `Core::revoke_shared_ability`, the admin `Capabilities.tsx` page and
`AbilityVersionsPanel` in Measurements, and the `vak skills` verbs
(`run_abilities` in `crates/vak/src/main.rs`).

**E6a is built** (checked in the running app on the installed home):
the Library's Learned abilities chip over `GET /skills/abilities`
(`Core::abilities`), proposals across folders, and
`Artifact::keeps_every_version` (ability versions were drafts and the
lifecycle trashed their history; the installed home lost the 2026-10-15
abilities' history that way).

**E5 is built** (checked live with gpt-6-luna): one `AbilityRecord`
Document per learned skill (`agents/{agent}/ability-standing`), settled
at each turn's end from that turn's program calls;
`vak_judge::ability::verdict` offers a version that passed, worked in
another turn and never failed (`AbilityOffered` inbox entry, `POST
/skills/abilities/{name}/promote`) and withdraws a fix that fails after
its parent failed (`AbilityWithdrawn`). A program refuses arguments by
exit status 2 (`Fault::Target`), which never counts against it; a
`"fails": true` test now expects that refusal. Left on the installed
home: `weighted-bill-splitter` and its unanswered offer.

**E4 is built** (checked live with gpt-6-luna on the installed home):
`Tool::serves` returns `Vec<&str>` borrowed from the tool, so a program's
tool carries its skill's `serves:` (`propose_skill` takes `serves`,
described from `Domain::known`) and defers; the prompt's "More tools"
catalogue lists the turn's program tools (`tool_catalogue_for`), the one
capability index; loading a skill with a program shows its files and how
to improve it (a changed copy proposed again under its name, a new
Library version naming its parent); `Origin::Agent` labels an Agent's
own skill `agent`. Fixed on the way: a `recall` search or card of the
same conversation now holds what its turns held (was always "someone
else's words"); verifying the same program twice saves a new version of
its record (was a create conflict). Left on the installed home: the
`restaurant-bill-splitter` skill (versions 1 to 4, v4 live), declined
proposals, and the folders in /tmp/vak-live/e3.

**E3b is built** (2026-10-15): `[owner] abilities_go_live` (on by
default, a narrower layer only turns it off, `PUT /config/owner` now
takes each field optionally) in Settings > Capabilities > Manage >
Skills; `Program::reaches` is the one wording of what a program reaches,
declared as `Tool::reaches` and carried by an approval's reason; a
proposal waiting for review records `ProgramReview` and writes a
`ProposalOpened` inbox entry, cleared on approve or decline
(`Core::proposal_settled`); packs in a skill's `presentation/` are
checked at proposal and active from the Agent's own skills.

**Doc 94's follow-ups were checked live** (2026-10-15): tools stay loaded
after a restart; "run it now" on a paused automation called `resume`
until the `action` field said what each action does; a scheduled answer
of cards only lost its "Scheduled" head (the narration replaced the
prompt's text too, and the chat drew the prompt from `/transcript`,
which now carries `scheduled` and `at`).

**E3a is built** (2026-10-15, gate green, checked live with gpt-6-luna on
the installed home): `propose_skill` takes `source`, a folder the Agent
wrote; `Core::propose_skill_program` (`vak-core/src/skill_programs.rs`)
bundles it (`capability::bundle`), writes its SKILL.md from the call's
words, verifies it (E2), records a Library version (`ArtifactKind::Skill`,
parent the latest, typed `ArtifactStep::Verified`), and admits it: Live in
`agents/<id>/skills` (discovery reads it right after the project's,
`skills::roots`), Review as a proposal naming the version (promotion
installs the bundle, `learning::promote(.., library)`), or Failed with
each case. Live: the first try failed three times on capability.json
(nothing stated its shape; each refusal named one field); with
`capability::FORMAT` in the description and refusals it built a skill
first try (4 of 4 tests, live) and a fresh conversation called it. Left
on the installed home: the Agent's live `temperature-converter` skill,
its Library versions, and a memory note from the failed first attempt.

**E2 is built** (2026-10-15, gate green, checked live): a program's
tests are `tests/<name>.json` (`arguments` plus `expect` or `"fails":
true`; `vak_tools::capability::TestCase::judge` names where an answer
differs); `Core::verify_skill_program` (`vak-core/src/skill_programs.rs`,
CLI `vak skills verify <name>`) runs each case through `CapabilityTool`
in a run of its own under `runs::span`, settles it completed or failed,
and keeps the report as a Document per digest
(`AgentScope::capability_checks`, registry `agents/{agent}/capability-
checks`). A program with no tests is refused at load. Live on the
installed home: one case passed, one failed with "$.each is 33.33,
expected 33.34", exit 1, run id printed. The dev build can now open the
installed home (8.1.0, stamped).

**E1b is built** (2026-10-15, gate green, checked live): a program's
declared `hosts` are reached through an egress proxy of its own (the MCP
proxy, moved to `vak_tools::egress` so there is one), and its sandbox may
connect only to that port; pinned Python `requirements` (file + index
hosts, both declared) install by their own worker run into
`cache_home()/capability-envs/<digest>` from a staging folder the host
moves into place; until installed the tool declares Network, so the first
call asks. Tests in `capability_worker.rs` (declared host tunnelled,
another host 403, direct connect blocked; install once, no staging left).
Live: `six` 1.17.0 installed from PyPI after the approval, next call ran
offline with no ask. Open: the install has no run record yet (E2), Node
dependencies, and the approval wording (E6).

**E1a is built** (2026-10-14, gate green, checked live): a skill may
carry `capability.json` + a script; `vak_tools::capability` loads and
checks it (entry inside the skill, every argument typed and described,
`timeout_ms` reserved, digest over every file), `CapabilityTool` offers it
under the skill's name (added after the turn's other tools, only when the
name is free; `skills::programs`), and `broker::run_capability` runs it as
`WorkerTask::Capability` in a worker wrapped by a sandbox built from the
declaration (reads system, workspace, skill dir, interpreter install;
writes its tmp, cache and declared drafts; no network, whatever the
session's mode). `FrozenSkill::load` tells the model the skill has a
program, or why it cannot run. Tests: `vak-tools` unit tests and
`vak-server/tests/it/capability_worker.rs` (JSON answer, no JSON, no
network, drafts only, changed program refused). Live (dev route): a
request that never named the skill called it, the loop filled
`timeout_ms`, the result was exact; the card the model drew dropped a
cent (recorded; E9). Also fixed: the baseline refusal for an unstamped
home said "7.x or earlier"; it now says the home was written by "a
version of vak that did not record itself". And the full gate found the
baseline check counting a top-level `agents/` folder as old records:
current paths make `agents/<id>` before any Core stamps a home, so a new
home could be refused for good (seen as an order-dependent failure in
`vak-core`'s `agent_identity_per_turn`). Every data home is 8.x (maintainer, 2026-10-14), so
only a tenant store with no version stamp marks an older home now.

**Self-evolving capabilities are designed**
(`docs/design/93-self-evolving-capabilities.md`, status in progress). The
maintainer chose D1 to D8 on 2026-10-14 (`docs/decisions.md`): a skill
may carry a sandboxed script with tests; live at once only when it is
sandboxed, reaches no network, only reads the workspace and writes its
drafts, and the turn that wrote it held no untrusted content, otherwise
Review; evolution stays above the Rust runtime; near matches are
extended as new versions and the answer says so; Agent-owned first,
Shared on evidence; an opt-in review of failures; a compact prompt
index plus `find_tools`. On the maintainer's delegation the doc
also settles R1 to R7: one kind (a skill with a program); promotion by
observed states; dependencies pinned with hashes, installed once by
their own run; a capability shows only through card forms, draft
previews (Canvas) and declarative presentation packs (§3.1), and a card
may take its data from the call (E9); Office files and PDFs only through
`office_apply`; a proposed automation is always Review. Doc 43 is superseded. Stages E1 to E8 with "done when" are in
§11 (E1 to E9) and tracked in `docs/plans/open-items.md`. The experience is §9:
a typed activity line in the conversation, a capability sheet, Review,
an Abilities view in the Library, Settings → Capabilities rewritten,
inbox kinds, admin `#/capabilities`, CLI verbs.

What the study found in the code (the starting points for E1 to E3):
- `vak-core/src/learning.rs`: `ProposeSkillTool` writes a Document to
  `AgentScope::skill_proposals`; `promote` writes `SKILL.md` into
  `Core::shared_skills_dir` with `std::fs::write` and forgets the
  proposal; `duplicate_of` screens near duplicates. No versions, no
  parent, no program.
- `vak-core/src/skills.rs`: `Skill { name, description, path,
  provenance, shadowed, serves }`; `SkillTool` loads instructions only.
- `vak-core/src/capability/`: kinds are `vak_session::CapabilityKind`
  (Tool, Skill, McpServer, Hook, Command, Presentation); `provider.rs`
  turns discovery into declarations; `domain.rs` is the `serves`
  vocabulary.
- `vak-tools/src/broker.rs`: `WorkerTask` (Tool, VerifyTargets,
  OfficeReview, OfficeProject, …) is where `Capability` and
  `VerifyCapability` join.
- `vak-permission/src/reach.rs`: `Reach` (Display, Coordination,
  ReadPath, ReadStore, WritePath, Learning, Command, Network,
  Integration, Other) is what a capability's `reach` names.
- `vak-judge/src/measure.rs`: `TurnFacts` per turn by tool name; the
  per-version measure extends it.
- UI today: client `Settings.tsx` "Capabilities" section lists skills,
  plugins and connections; `LibraryPage.tsx`, `InboxPage.tsx`,
  `NeedsYou.tsx`; admin `Inbox.tsx` already shows `proposal_opened`,
  admin routes include `#/measurements` and `#/integrations`.

**8.1.0 released 2026-10-15** (`61d65be2c`, tag `v8.1.0`): E1 and the
8.0 data baseline. This Mac was purged and 8.1.0 installed; its home
names 8.1.0, services run, `self verify` clean. The maintainer runs `vak
setup` (OpenAI key, gpt-6-luna) next; after that a dev build can use the
installed home and its key for bigger skill tests, sparingly. AWS is
still on 8.0.0 (its home predates the stamp too: purge with its next
install). `scripts/bump-version.sh` stopped at its `cargo clean` (no
`CACHEDIR.TAG` in target/); the badge and lockfile were finished by hand.

**After 8.1.0** (2026-10-15, unreleased, gate green): setup saves the
route and access level to the Shared layer from any folder (`f438f0f36`,
checked in the running admin); a terminal shows warnings and errors only
unless `VAK_LOG` is set; a second server leaves the gateway's agent
network broker alone (`Served::Elsewhere`); and a home names its version
before its first tenant store exists (`2e3790db2`, the one writer is
`vak_session::objects::stamp_home_version`). The maintainer ran setup:
the installed Shared route is ollama `qwen3.5:0.8b-mlx`, full access.

Earlier: **8.0.0 released** (`db1dc61db`, tag
`v8.0.0`) and installed fresh on this Mac; AWS on `fe131988f` (8.0.0
plus the web sign-in fix), fresh and set up. The data baseline is 8.0
(`a1a29c8f6`, unreleased): the 8.0.0 homes on the Mac and AWS predate
the `home-version` stamp, so the next release needs both purged and set
up again. The maintainer still runs `vak setup` on this Mac.

## The record

Start at `docs/README.md`: one place for each kind of fact, the six status
words, and what to update for each kind of change. `docs/design/INDEX.md`
is generated from every design doc's Status (`python3
scripts/check_records.py --write`), `docs/decisions.md` logs the
maintainer's decisions, and `scripts/check_records.py` (in `gate.sh` and
`release.sh`) enforces it. Built 2026-10-14.

## Next

1. Doc 93 E6 rest, then E7 to E9 (doc 93 §11), each with the full gate and a
   live turn; tick `docs/plans/open-items.md` and the doc's Status.
2. Release questions (the 8.0 data baseline, purge on Mac and AWS) are
   the maintainer's; ask before cutting.

## Working notes

Live runs:
- Build `cargo build -p vak -p vak-server --bins`; the server embeds the
  client bundle it was built with, so after a client change run
  `npm run build` in crates/vak-client-ui (both bundles), rebuild, then
  restart the server.
- Launch entries in `.claude/launch.json`: `vak-tmp-live` (port 8902) and
  `vak-tmp-live-8903` serve the dev build from /tmp/vak-live/ws13 with
  `serve --trust` (trust for that process only, which is what lets the
  folder's `permission_mode = "workspace-write"` apply). Port 8902 can be
  held by another chat's server; `preview_stop` cannot stop it, so use
  8903 (copy /tmp/vak-live/*.sh with `sed 's/8902/8903/g'`).
- /tmp/vak-live/test-gateway-token holds the dev token (recreate a random
  one after a reboot); sign in with `/app/?token=<it>`. The link
  `vak open app --print` gives carries the installed home's token, which
  the dev server refuses. Ollama must be running with the dev model.
- A fresh dev data home has no route: `PATCH /config` with the provider
  and model once, then check `GET /config` (route and `WorkspaceWrite`)
  before a turn.
- /tmp/vak-live/turn.sh runs one turn in a new session and prints its
  calls; sum.sh summarises the last; wait.sh prints the newest run's
  decisions. `/runs/{id}/decisions` is empty while the run is open: poll
  its status first. Sessions made over the API do not open in the client
  list; open `/app/#/s/<id>` directly. The person's own session in the
  client works in ~/vak-home even on the dev server.
- A `/until-done` turn is released by a steering message that is exactly
  `done` (`POST /sessions/{id}/steering {"text":"done"}`).
- `scripts/measure_tool_form.py replay` records outcomes, not responses:
  to see how a no-call response ended, post the captured body to
  `/api/chat` with `stream: false` and read `done_reason`, `eval_count`,
  `tool_calls` and the thinking's tail.
- `/library/{id}` lists versions under `history`; `/library/{id}/versions/{v}`
  returns a version's bytes (how to put an earlier version in the folder).
- An untrusted project's `permission_mode` is dropped (privileged key).
- The ledger keeps a call as the model first wrote it; a regenerated call
  is not in the transcript.
- A turn that fetches a page after reading private data asks first
  (invariant 43); a tool the person declined leaves the turn.

Browser pane:
- The screenshot does not paint a sandboxed preview frame: check it
  through the DOM (`srcdoc`, `sandbox`, CSP). The static mockup preview
  always renders light; use its own theme button.
- Clicks by coordinate miss when the viewport is scaled: use `find` refs.
  The Results panel covers the chat: close it before clicking behind it.
- At 390 × 844 the client's sidebar opens over the conversation: press
  Escape (after a second) before a screenshot.
- Publishing a hosted artifact page was refused by the permission
  classifier; mockups live in `docs/assets/`.

Tests and the gate:
- `scripts/gate.sh` (full, about 13 minutes warm) before every commit;
  `--changed` while working. `cargo test` prints nothing for a binary
  until it finishes and stops at the first failing binary, so a failure
  means the rest did not run. Never edit sources while it runs.
- A test that fails only beside others reads more than it made, or acts on
  the whole home: fix it or give it its own binary for that reason, never
  to hide a failure. A test in a shared-home binary filters by something
  it made; `/eval/ledgers` and `/library` read every session in the home.
- `#[path]` inside a nested module resolves through directories that do
  not exist; declare a shared fixture at the crate root. `cargo fmt`
  rewraps lines: re-read before a scripted edit.
- A new field on `ToolContext` needs every literal updated
  (`git grep -l 'ToolContext {'`). Clippy's argument limit is 7: fold an
  argument into its struct rather than allowing the lint.
- A `CallEffect` is a commit point unless it touched nothing outside
  (`commits()` in vak-session/src/log.rs); a new variant decides which.
- A WebSocket route needs no `Origin` check of its own (the router does
  it); test it through `secured_router_with_port`, never
  `vak_server::router`, which has no auth layer.
- A sandbox test that redirects to `/dev/null` fails in a read-only grant
  and proves nothing about the network.
- Linux checks run in Docker as `scripts/sync-lab/build.sh` does
  (`rust:1-bookworm`, the `vk-target` and `vk-cargo` volumes). As root,
  `server_owned_cleanup_removes_a_protected_candidate_tree` and
  `revision_copy_uses_only_verified_saved_version` fail because root
  ignores permission bits.
- `tempfile` is a dev dependency of vak-server only; a route's temporary
  files go under `vak_config::paths::runtime_dir()`.
- In a flex-column page, a child with `overflow-x: auto` collapses to zero
  height unless it has `flex: none`.

Git and release:
- Another session may share this checkout: stage only your own paths,
  never `git add -A`; if its half-finished edits break the build, wait for
  its commit rather than gating over them. `git stash` and `pkill` are
  refused by the permission classifier: commit your own steps, then
  `git pull --rebase`; a conflict in crates/vak-client-ui/dist-web/ is
  resolved by rebuilding the bundle and adding the whole directory.
- `release.sh` rebuilds the committed bundles in place, so the tree looks
  dirty while it runs. The AWS inventory uses the `vak-hosting` SSO
  profile; running remote `setup` over SSH is the owner's to do.

## Traps learned this session

- Two dev servers on `localhost` share cookies (cookies ignore the port):
  signing in on one signs the other's tab out (401s in Settings). Keep
  one tab and sign in again with `?token=` when switching ports.
- `scripts/gate.sh --changed` fails clippy on "items after a test
  module" when a function is appended to the end of a file that ends in
  `mod tests`.

- `git mv` stages the rename at once; naming the old path in a later
  `git add` fails the whole add, and a `&&`-chained commit then commits
  only the rename (`afe6fc5d4` does not build alone; `eb750e972` follows
  it). Stage the new path only, and check `git show --stat` before
  pushing.
- A purge on the Mac deletes `~/Library/Application Support/Vakyartha`,
  which held the private AWS inventory; keep a copy elsewhere. A purge on
  the host drops the Shared `[server]` section (421 behind the proxy until
  it is added again), and the gateway unit exits 209 if
  `~/.local/state/vak/logs` is missing. Run setup in a real terminal: a
  command with no stdin skips every question.

- A test that fails only in the full gate with `NotFound` on a path from
  `env!("CARGO_MANIFEST_DIR")` is a stale binary: another session built in
  a worktree that shares this `target/` and was deleted, and cargo reused
  its binaries. Check with `strings <binary> | grep scratchpad/wt`. Clean
  only the workspace packages:
  `cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys;print('\n'.join(a for p in json.load(sys.stdin)['packages'] for a in ('-p',p['name'])))" | xargs cargo clean`
  (zsh does not split a `$pkgs` string). The first build after it takes
  about ten minutes.
- `git stash` and `pkill` on a test run are refused by the permission
  classifier. To take in another session's push, commit your own steps
  locally first, then `git pull --rebase`; a conflict in
  `crates/vak-client-ui/dist-web/` is resolved by rebuilding the bundle
  (`npm run build`) and adding the whole directory.
- A card tool's own field check refuses an argument its schema does not
  list, and `show_card` passes the filled arguments through it: a field
  the loop fills must be known to the shape (`output_of_command`).
- In the browser pane, resizing to 390 × 844 opens the client's sidebar
  over the conversation: press Escape (after a second) before a
  screenshot.
- `scripts/gate.sh` takes about 13 minutes from warm; the first build
  after cleaning the workspace packages adds about ten.
- The session nodes route lists nodes whose `session` column is the
  conversation, plus what its calls wrote, read or fetched; a node shared
  by several conversations (a page, a file) traces through every call that
  touched it.
