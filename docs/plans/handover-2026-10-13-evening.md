# Handover — 2026-10-13 evening: after doc 90 stage 5

Supersedes docs/plans/handover-2026-10-13-release.md for "Prompt",
"Where things stand" and "Next, in order". That file's "Live runs" and
"Traps", and "How we work" and "Traps" in
docs/plans/handover-2026-10-12.md, still apply.

## Prompt for a fresh session

"Continue in /Users/nisheethranjan/Projects/Vak/code on main. Read
AGENTS.md first (especially 'Tool arguments are written, then read', 'No
word lists, no tuned limits', 'Investigating a failure', and 'Verification
before every commit'), then docs/plans/handover-2026-10-13-evening.md
whole, then 'Live runs' and 'Traps' in
docs/plans/handover-2026-10-13-release.md, then 'How we work' and 'Traps'
in docs/plans/handover-2026-10-12.md.

First run `git status` and `git log --oneline -6`; main should be clean at
or after the evening handover commit. Then take the first item under
'Next, in order', finish it whole (code, tests, docs, AGENTS.md where a
rule changes, screens), verify it with real turns or in the browser, run
the full gate, commit and push, then the next.

Live runs: never on ~/vak-home. Use /tmp/vak-live/ws13 in workspace-write
mode via the `vak-tmp-live` launch entry (port 8902, token in
/tmp/vak-live/test-gateway-token; sign in at
/app/?token=<token>). The route is set (ollama, qwen3.5:4b-mlx); check
`GET /config` before a turn. Work autonomously with short progress lines;
ask only on a genuine product decision. Main only. No provider, model or
tool is first-class. Never blame the model: find what in Vak's contract
made a reasonable attempt fail; when the contract states the facts and the
failure remains, record what the model did and stop tuning. Do not edit
sources while a `cargo test` is running. Keep target/ under 45 GB. UI work
is to a world-class standard by hand (no outside design skill), checked at
1440 × 900 and 390 × 844, light and dark. Token budget is tight (Claude
Pro): lean reads, Haiku agents with exact briefs for mechanical slices,
commit each verified step."

## Where things stand

Pushed today after 7.1.0, each verified live or in the browser, full gate
green:

- `3f9b581c1` a card call is recorded as the block it was made to; a card
  the host cannot record is a tool fault, never "shown".
- `a1224b3f0` doc 90 §5.1 Agent half: the note names a version's
  conversation, turn (the id `recall` takes) and parent; one path resolver
  (`vak_tools::context::resolved`); `office_apply` says outside the
  workspace first.
- `a84225b47` doc 90 stage 5a: `file_state::version_states` and `needs`,
  served by `/library`, `/library/{id}` and `/library/made/{session}/{call}`;
  a copy the workspace held is never a draft (it would have been trashed).
- `9e38f5b86` doc 90 stage 5b–e: the artifact page's version graph and
  origin, live states on file cards, the "Needs you" strip, Library state
  filters, and a reworked Library look.

Maintainer decisions today: stage 5 built to the mockup as it stands;
item "card claims a draft no call made" is left alone (no typed fact).

## Next, in order

1. Done 2026-10-13: both flaky tests had causes. The pause tests treated
   "session busy" as "turn admitted"; a pause in that gap refuses the turn
   at admission, so they now wait for the model to be called. A gate opened
   after a mode change or stop had already swept the pending list waited
   its whole approval window: the loop now races every gate against the
   run's cancel token (`ask_unless_cancelled`), and both server approvers
   settle a dropped gate (`OpenGate`, `ForwardedGate`).
2. Control plane leftovers: done 2026-10-13 — the commitment posture
   follows the commitment, not the horizon reading (`apply_commitments`);
   a wire refusing a named tool choice is learned (`shape_choice`);
   `scripts/measure_tool_form.py` replaces the toy-schema
   `measure_answer_form.py` (verified live: 16 requests captured from one
   `/until-done` turn, one replayed 3 times, 2 of 3 valid as written).
   C7 done 2026-10-13 (maintainer: findings for review, CLI + admin):
   `vak eval --ledgers`, `/eval/ledgers`, `/eval/proposals/review`, admin
   System › How the models did (`#/measurements`), checked at 1440 × 900
   and 390 × 844, light and dark. Doc 88's stages are done.
   Seen live while verifying, not yet looked into: a `/until-done` turn
   on the dev route ended with the model apologising for "declaring
   completion prematurely" after a stop-gate correction, and wrote its
   table as a list although the first replayed step called the table
   card.
3. Check whether a card-only answer (no response text) is evaluated
   `Unknown` by `evaluate_response_with_failures` and so marked unverified
   (the chat showed "The requested outcome is not verified" on such turns).
4. `office_apply` refuses a new file whose folder is missing while `write`
   creates it (a model ran `mkdir` to get past it): decide one contract.
5. Doc 90 leftovers that need record fields first: a version's own title,
   the request that asked for it and what its turn read
   (`derived_from`), shown as "You asked" / "It read first" on the page and
   in the note; Compare with your folder. Also: the Library list page is
   820px wide while the artifact page is 1180px.
6. Model choice, recorded (do not tune without a new contract cause): a
   model sometimes invents a folder (`private/tmp/vak-work/…`, `ws13/…`)
   or writes after the work is done; a card can claim a change no call
   made.

## Traps learned today

- The vak binary embeds the server: after a client change run
  `npm run build` in crates/vak-client-ui, then
  `cargo build -p vak -p vak-server --bins`, then restart `vak-tmp-live`.
- The bundle check fails the server build on any stale `src/` change,
  CSS included: rebuild the bundle before `cargo check -p vak-server`.
- In a flex-column page, a child with `overflow-x: auto` collapses to zero
  height unless it has `flex: none`.
- The browser pane's clicks by coordinate miss when the viewport is
  scaled; use `find` refs or a `javascript_tool` click.
- `file_state::observe` records an outside edit as a version: read the
  artifact again after observing.
- The person's session in the client works in ~/vak-home even on the dev
  server (the built-in Agent's workspace); API sessions use the served
  folder.
