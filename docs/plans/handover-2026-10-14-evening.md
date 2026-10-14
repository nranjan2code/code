# Handover — 2026-10-14 evening: card forms done, 8.0.0 released

The one handover. Earlier handovers were removed on 2026-10-14 (git
history keeps them); every note of theirs that is still true is under
"Working notes" below.

## Prompt for a fresh session

"Continue in /Users/nisheethranjan/Projects/Vak/code on main. Read
AGENTS.md first (especially 'No word lists, no tuned limits',
'Investigating a failure', 'Verification before every commit' and the
card-forms paragraph under Output presentation), then
docs/plans/handover-2026-10-14-evening.md whole and
docs/plans/open-items.md ('Before the next release').

Run `git status` and `git log --oneline -8`. Stage only your own paths;
keep only `main`. 8.0.0 is released and installed fresh on this Mac;
the maintainer runs `vak setup`. Next: bump the data baseline to 8.0
(open items), then ask the maintainer which item to take. Fix causes as
designs, never spot fixes, keyword rules or tuned numbers. Live runs use
launch entry `vak-forms-live` (port 8904, /tmp/vak-live/forms, token in
/tmp/vak-live/test-gateway-token; route gpt-6-luna, check GET /config);
after the purge the dev data home may need its route set again. Short
progress lines, lean token use, update this handover at the end."

## Where things stand

**8.0.0 released 2026-10-14** (`db1dc61db`, tag `v8.0.0`), installed
fresh on this Mac after a purge; services running, `self verify` clean.
The maintainer still runs `vak setup` with the provider key. The data
baseline is still 7.0: bumping it to 8.0 is the first open item.

**AWS on 8.0.0, fresh** (`fe131988f`: 8.0.0 plus the web sign-in fix):
the 7.1.0 install was purged, setup run (openai-responses, gpt-6-luna,
workspace-write), `[server]` trusted host and public URL added again to
the host's Shared config, owner passkey enrolled; the temporary SSH rule is
revoked (80 and 443 only). The fix: a web client that was not signed in
counted the 401 from `GET /host` as no answer and never showed sign-in
(`host/web.ts`); it ships in the next release.

**Card forms (doc 92) are done, F1 to F6.** This session, each step with
the full gate green and checked live:

- `dcd281816` figures kept as written; first summary leads; comparison
  points hang-indent; chart series 2 green; Latest pill bottom-right.
- `b8ad78e20` a plan sent in the same step as cards is recorded before
  they are checked (they went unchecked); a `[card-plan]` refusal lists
  the cards still to show apart from those shown (a model had re-sent
  shown ones); the plan's result asks for ready cards in one step. New
  agent test: several cards in one step are recorded in call order.
- `19f68d3fe` card prose renders as the conversation's markdown and long
  words wrap (a raw `[title](url)` pushed a callout off a phone); Today
  moved from a floating pill into the top bar.
- `04b0c8a52` the model places and revises its own cards (maintainer's
  decision): optional typed `replaces` (a presentation id from the ack;
  unknown ids refused with the answer's ids) and `beside_previous`
  (`AnswerOutline.beside`, paired two to a row where wide). These replace
  the client's `SMALL_BLOCKS` list and the projection's supersede-by-type
  after a repair nudge (doc 87, "Placement and revision").

Live findings (dev route): one card per step came from the model's own
choice; nothing in the request disabled parallel calls. With the wording
in the prompt seed and the plan result, cards batched in four of six
card-bearing turns (wording only, recorded in open items). Format-free
requests (a fact, hidden numbers, two small asks, a bill split) stayed
prose; a hidden plan drew summary, steps, callout and sources; the model
set `beside_previous` itself on a summary and its callout.

Earlier the same day:

Pushed earlier, each with the full gate green and checked live:

- `b5c0ab862` Compare with your folder lists what an Office or PDF version
  changes (the night list's item 3).
- `4bc9ebd13` every artifact write is one `Artifacts::transaction`: the
  rows are appended only if the `artifacts/` chain did not move since the
  decision read it (`RecordChain::append_all_after`, checked under the
  chain's writer lock, which every process takes), otherwise it decides
  again. `recording()` is gone. Eight handles seeing one hand edit record
  one version (eight without the check). Maintainer chose to build it.
- `d3f3eb1b0` a fetched page is lineage: catalog node
  `page:<space>:<url>` with a `fetched_by` edge to its call (the lineage
  walk follows it), turn link `FetchedPage`. A session's Lineage tab now
  lists the files and pages its calls touched (files were missing too).
  Catalog stamp 4, so an existing catalog rebuilds once.
- `9c177d9a0` a terminal card shows what its command really printed: the
  card takes only `command`; `Tool::output_of_command` declares which
  argument the loop fills, and `show_card` and the fence path take it from
  the latest call that ran that exact command (`SessionLog::command_result`,
  matched by the tool's `Subject::Command` declaration). A card naming a
  command no call ran is refused. `[limit]` is now a typed inline hint, so
  no surface shows it to the person.
- `64f610675` handover ticks; doc 88's status line no longer claims three
  open waits (they were closed on 2026-10-12; an audit found none left).
- `9004b269f` the tool-name alias word list (`canonical_tool_name`) is
  gone: a call reaches a tool only by its exact name, and a retired or
  unknown name gets `unknown_capability` with the available tools. The
  projection's file artifacts come from the call's recorded
  `CallEffect::FileWrite`, not a table of tool names and path keys.

`recall` with a query beside a `turn_id` was already fixed (`2e8c916d8`).

After the maintainer's critical review:

- `e43c46753` a terminal card fills only from a call that ran a process:
  a tool says so (`ToolOutput::ran`, carried by the worker protocol), the
  loop records `CallEffect::Ran` whether the call failed or not, and the
  card takes output, exit code and duration from it ("Stopped · 5.0 s",
  "Exit 0 · 120 ms"). A call refused before running ("working directory
  does not exist") is never shown as output. A name in `RETIRED_TOOLS`
  gets `unknown_capability` with `use_instead` from that registry.
- `ed4c1d6b6` a command stopped at its limit is stopped there under
  Seatbelt: the sandboxed broker worker's SIGKILL to the command's process
  group was refused by `(deny default)`, so `sleep 5; echo ready` with a
  1000 ms limit printed "ready" after 5 s. The restricted profiles now
  `(allow signal (target same-sandbox))`; a process outside the sandbox
  still cannot be signalled (tested). Landlock does not restrict signals;
  not tested on Linux.
- `54dc3e261` the test-report card names the `command` that ran the
  tests; its exit code and duration come from that call, a card naming a
  command no call ran is refused with the commands that did run this turn,
  and a run that exited non-zero or was stopped cannot report every test
  passed. The per-test rows are still the model's words (binding them
  would mean reading the runner's output). Maintainer chose "test report
  only"; other cards stay model-written.
- Decided (maintainer): `recall` opens another conversation only through
  an attached artifact, as now; recorded in doc 90's status line.

Later the same day (record and audit round), each with the gate green:

- `6157e60dd` the record system (`docs/README.md`, status words, the
  generated `docs/design/INDEX.md`, `docs/decisions.md`, the one tracker
  and one handover, enforced by `scripts/check_records.py`); `8069f09ef`
  plans carry a status word too.
- The design-doc audit is done: every `implemented` doc was read against
  the code and corrected, past sections marked `> **Historical record**`
  (`ed998818e`, `1c70d6ae9`, `e45f4e047`, `debb3efb6`).
- Bugs it found, fixed: `da4285997` `vak term`'s approval box offered to
  edit the command and discarded the edit (removed); `c7006bc34` a
  promoted skill was installed where discovery never looked (now the
  Shared layer's skills, `Core::shared_skills_dir`); `35baa512e` the
  unreachable `vak-tray` binary removed.
- Contract gaps it found, not fixed (each needs the maintainer's call):
  listed in `docs/plans/open-items.md` under "Not audited".

## The record

Start at `docs/README.md`: one place for each kind of fact, the six status
words, and what to update for each kind of change. `docs/design/INDEX.md`
is generated from every design doc's Status (`python3
scripts/check_records.py --write`), `docs/decisions.md` logs the
maintainer's decisions, and `scripts/check_records.py` (in `gate.sh` and
`release.sh`) enforces it. Built 2026-10-14.

## Next

Everything open is in `docs/plans/open-items.md`, the one tracker: the
release checklist, what was seen only in tests, what has not been run,
what needs the maintainer, known limits and decisions not to build. Ask
the maintainer which item to take first after the 8.0 data baseline: an
open card-forms item (a tuned length in `isFleetingNarration`, the unread
`tool_domains` field), or one of the contract gaps the audit listed.

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
