# Handover — 2026-10-14 night: self-evolving capabilities designed

The one handover. Earlier handovers were removed on 2026-10-14 (git
history keeps them); every note of theirs that is still true is under
"Working notes" below.

## Prompt for a fresh session

"Continue in /Users/nisheethranjan/Projects/Vak/code on main. Read
AGENTS.md first (especially 'The data rules', 'No word lists, no tuned
limits', 'A tool's permission is what it declares' and 'Verification
before every commit'), then docs/plans/handover-2026-10-14-night.md
whole, docs/design/93-self-evolving-capabilities.md whole, and
docs/plans/open-items.md ('Self-evolving capabilities' and 'Before the
next release').

Run `git status` and `git log --oneline -8`. Stage only your own paths;
keep only `main`. We are on a Claude Code Pro plan: read lean (grep and
line ranges, not whole files), commit each verified step, keep the
tracker ticked, and rewrite this handover's prompt and 'Where things
stand' before the session's limit. Next: build stage E2 (doc 93 §11:
tests/ run by the runtime as a VerifyCapability worker task with a run
record, per-case results on the version; E1 is built; decisions D1 to D8
and R1 to R7 are settled) as a design, never spot fixes or tuned numbers;
UI that gets in the way is rewritten, not patched (no users yet). Live
runs use launch entry `vak-e1-live` (port 8905, a throwaway
VAK_HOME=/tmp/vak-live/e1-home, workspace /tmp/vak-live/forms; set the
route once with PATCH /config to ollama qwen3.5:4b-mlx; the Shared layer
is then /tmp/vak-live/e1-home/vak-home/.vak). The installed data home
cannot be used by a dev build until it is purged (it predates the
home-version stamp). Short progress lines."

## Where things stand

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

Unchanged from earlier today: **8.0.0 released** (`db1dc61db`, tag
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

1. Build E2, then E3, each step with the full gate and a live
   turn; tick `docs/plans/open-items.md` and doc 93's Status as stages
   land.
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
