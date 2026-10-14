# Handover — 2026-10-14 evening: the night list is done

Supersedes docs/plans/handover-2026-10-14.md for "Prompt", "Where things
stand" and "Next". Still apply: "Traps learned this session" in that file,
"Live runs" and "Traps" in docs/plans/handover-2026-10-13-release.md, "How
we work" and "Traps" in docs/plans/handover-2026-10-12.md.

## Prompt for a fresh session

"Continue in /Users/nisheethranjan/Projects/Vak/code on main. Read
AGENTS.md first (especially 'Tool arguments are written, then read', 'No
word lists, no tuned limits', 'Investigating a failure', 'Verification
before every commit', 'Parallel agents' and invariant 43), then
docs/plans/handover-2026-10-14-evening.md whole, then the sections it names
in the older handovers.

First run `git status` and `git log --oneline -8`. Another session may work
in this same checkout: stage only your own paths, never `git add -A`. Keep
only `main`: no branches, no worktrees, commit and push each verified step.

The night list is finished. Start the next round from 'Next' below: ask the
maintainer which backlog item to take first if it is not clear, then finish
each whole (code, tests, docs, AGENTS.md where a rule changes, screens),
verify it live or in the browser, run the full gate, commit only your paths,
push.

Live runs: never on ~/vak-home. Use /tmp/vak-live/ws13 in workspace-write
mode, launch entry `vak-tmp-live` (port 8902) or `vak-tmp-live-8903`; token
in /tmp/vak-live/test-gateway-token; sign in at /app/?token=<token>. The
route is ollama, qwen3.5:4b-mlx; check `GET /config` before a turn. Work
autonomously with short progress lines; ask only on a genuine product
decision. No provider, model or tool is first-class. Never blame the model.
Do not edit sources while a `cargo test` is running. Keep target/ under
45 GB. UI work by hand, checked at 1440 × 900 and 390 × 844, light and
dark. Token budget is tight: lean reads, commit each verified step."

## Where things stand

Pushed this session, each with the full gate green and checked live:

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

## Next (the next round's backlog; nothing here is started)

1. Watch for fallout from `9004b269f`: a model that calls `read_file`,
   `shell` or a retired name now gets `unknown_capability` and must call a
   listed tool. If a live run shows a model stuck there, investigate the
   unknown-capability answer's contract (what it lists, how it reads),
   never restore an alias.
2. A card or prose can claim what no call did (seen again live: a
   test-report card saying "1 passed" and prose saying a command finished
   in time, beside a terminal card showing it stopped at its limit; session
   `01a13867-c776-7381-ada4-b80021969a28` in the /tmp live home). The
   terminal card's fix (fill from the receipt) is the pattern; whether the
   test-report card should take its results from a call too is a design
   question for the maintainer.
3. Still open from doc 90: whether `recall` can open another conversation,
   which its refusal states.
4. Recorded model choices, no work unless the maintainer asks: a model
   sometimes invents a folder or writes after the work is done; a table's
   header and row joined by a newline in one string, or a one-row table
   with no header, are told and fixed on the next call.
5. New measured patterns from `/eval/proposals` are this round's backlog.

## Traps learned this session

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
- The session nodes route lists nodes whose `session` column is the
  conversation, plus what its calls wrote, read or fetched; a node shared
  by several conversations (a page, a file) traces through every call that
  touched it.
