# Handover — 2026-10-13 night: evening list done, doc 88 done

Supersedes docs/plans/handover-2026-10-13-evening.md for "Prompt",
"Where things stand" and "Next, in order". Still apply: "Live runs" and
"Traps" in docs/plans/handover-2026-10-13-release.md, "How we work" and
"Traps" in docs/plans/handover-2026-10-12.md, and "Traps learned today"
in the evening handover.

## Prompt for a fresh session

"Continue in /Users/nisheethranjan/Projects/Vak/code on main. Read
AGENTS.md first (especially 'Tool arguments are written, then read', 'No
word lists, no tuned limits', 'Investigating a failure', and 'Verification
before every commit'), then docs/plans/handover-2026-10-13-night.md whole,
then 'Live runs' and 'Traps' in docs/plans/handover-2026-10-13-release.md,
'How we work' and 'Traps' in docs/plans/handover-2026-10-12.md, and
'Traps learned today' in docs/plans/handover-2026-10-13-evening.md.

First run `git status` and `git log --oneline -6`; main should be clean at
or after the night handover commit. Then take the first item under 'Next,
in order', finish it whole (code, tests, docs, AGENTS.md where a rule
changes, screens), verify it with real turns or in the browser, run the
full gate, commit and push, then the next.

Live runs: never on ~/vak-home. Use /tmp/vak-live/ws13 in workspace-write
mode. Port 8902 may be held by another session's server: if
`vak-tmp-live` will not start, use the `vak-tmp-live-8903` launch entry
(port 8903, same token in /tmp/vak-live/test-gateway-token; sign in at
/app/?token=<token>) and run the turn helpers with the port changed (see
'Traps'). The route is set (ollama, qwen3.5:4b-mlx); check `GET /config`
before a turn. Work autonomously with short progress lines; ask only on a
genuine product decision. Main only. No provider, model or tool is
first-class. Never blame the model: find what in Vak's contract made a
reasonable attempt fail; when the contract states the facts and the
failure remains, record what the model did and stop tuning. Do not edit
sources while a `cargo test` is running. Keep target/ under 45 GB. UI work
is to a world-class standard by hand (no outside design skill), checked at
1440 × 900 and 390 × 844, light and dark. Token budget is tight (Claude
Pro): lean reads, Haiku agents with exact briefs for mechanical slices,
commit each verified step."

## Where things stand

Pushed this session, each with the full gate green and verified live or in
the browser:

- `bc7b5bda7` the two flaky tests had causes: the pause tests paused
  before admission; a gate opened after a mode change or stop waited its
  whole window. Every gate now gives way to the run's cancel token
  (`ask_unless_cancelled`); `OpenGate` and `ForwardedGate` settle a
  dropped gate.
- `f8058ebac` the commitment posture (`Full` context, `Defer`) follows the
  fact that a strand works on an open commitment (`apply_commitments`),
  never the horizon reading.
- `941934588` a wire that refuses a named tool choice is learned
  (`DispatchSettings::shape_choice`, offering only that tool).
- `0e44eda25` `scripts/measure_tool_form.py`: a capture proxy and a replay
  of Vak's real requests (doc 89 §4.6); the toy-schema script is gone.
- `ba3cfea5e`, `ff19e1200` C7, which finishes doc 88: `vak eval
  --ledgers`, `/eval/ledgers`, `/eval/proposals/review`, the
  `proposal-reviews/` chain, admin System › How the models did
  (`#/measurements`). Maintainer's choice: findings for review, never
  drafted changes; no threshold.
- `8c309d6db` a card-only answer is `Produced`, not "not verified".
- `1bed4f8e2` a new Office file may name a folder that does not exist yet
  (one contract with `write`; Review's promotion makes it).
- `591d11e82` doc 90 §5 finished: `Version::title` (the call's `change`),
  `Version::derived_from` (with `CallEffect::Fetched` and
  `Tool::fetched`), "You asked" (`…/origin`), Compare with your folder
  (`…/compare`, marked word diff), the Library list as wide as the
  artifact page; and a race fixed: version recording and folder
  observation share `artifacts::recording()`.

## Next, in order

1. Investigate the `/until-done` turn seen live: after a stop-gate
   correction the model's answer apologised for "declaring completion
   prematurely" and continued; it also wrote the table as a list while a
   replay of its first step called the table card. Read its decisions
   (`/runs/{id}/decisions`) and the correction it was told, and find the
   contract cause (a correction's wording, `vak_control::render`, or what
   the managed loop asks) before anything else. Use
   `scripts/measure_tool_form.py` to replay the captured steps.
2. Work the measured patterns: open admin › How the models did on the dev
   data (32 waiting). Take the biggest that are Vak's own contract
   (`office_apply` refused by its schema 4 of 13, `emit_table_card`
   shape faults, `corrected:empty_step` 6 of 19, `doc_read` target
   failures 7 of 10), investigate each per 'Investigating a failure', fix
   the cause, and answer the proposal (accept with the fix, or dismiss
   with the reason in the commit). Never tune a word list or threshold.
3. Doc 90 leftovers: Compare with your folder for Office files and PDFs
   shows only which version the folder holds; give it the semantic diff
   the Review already computes in the worker (two versions, not a draft
   and the workspace). Also: whether `recall` can open another
   conversation, which its refusal states (doc 90 status line).
4. `artifacts::recording()` is one process's lock: the CLI and the server
   writing one data home can still race. Decide whether a ref/CAS under
   the writer epoch is warranted (doc 73's fenced paths) or record it as
   accepted.
5. A fetched page (`CallEffect::Fetched`) is not a catalog node or a turn
   link yet (`turns.rs::from_effect` returns `None`): add it as lineage
   ("references") if search or Lineage should trace pages.
6. Older open items: `recall` refuses a query with a `turn_id` ("cannot
   be combined"); a card claimed a command printed "ready" when its
   receipt said it stopped at its limit (run_01a12edd); doc 88 §5a's
   three remaining waits.
7. Model choice, recorded (do not tune without a new contract cause): a
   model sometimes invents a folder or writes after the work is done; a
   card can claim a change no call made.

## Traps learned this session

- Port 8902 can be held by another chat's dev server; `preview_stop`
  cannot stop it. `vak-tmp-live-8903` exists in `.claude/launch.json`.
  `/tmp/vak-live/turn.sh` and `wait.sh` are hardcoded to 8902: copy them
  with `sed 's/8902/8903/g'` (and point turn.sh at the copied wait.sh).
- The loop maps some tool names to built-ins (`read_file` became `read`
  and was refused as unknown): name a test tool something no alias
  covers.
- `/library/{id}` lists versions under `history`, not `versions`.
- A test in a shared-home binary must filter by something it made (a
  unique file or tool name); `/eval/ledgers` and `/library` read every
  session in the home.
- A version-recording race shows as two versions with one digest, one
  `from: "workspace"`: suspect an observation without
  `artifacts::recording()`.
- Clippy's argument limit is 7: fold a duplicate argument into its struct
  (`NewVersion { parent, ..new }`) rather than allowing the lint.
- `cargo test` prints nothing for a binary until it finishes; a gate takes
  12–14 minutes. Wait on its output file, never edit sources meanwhile.
- `npm run build` in crates/vak-client-ui writes both bundles; a
  `cargo build -p vak` that finishes in seconds has still re-embedded
  them.
