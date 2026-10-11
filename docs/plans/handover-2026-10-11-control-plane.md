# Handover — 2026-10-11: build the control plane

## Prompt for a fresh session

"Build the control plane in /Users/nisheethranjan/Projects/Vak/code on
main. Read AGENTS.md (especially 'No word lists, no tuned limits' and
'Investigating a failure'), then docs/design/88-control-plane.md, then
docs/design/86-model-plans-runtime-checks.md and
docs/design/87-composed-answers.md, then this file. First run the full gate on
a clean tree (see 'Where main stands') and fix anything it shows. Then start
at stage C0: scan the blast radius, and ask me the four open questions in
doc 88 §12 before writing code. Work only on main: no branches, PRs or worktrees; nobody else
is working in this repository. Everything is typed, in its own crate, with
no free text carrying a decision. Verify every stage with real turns on the
configured local route through the web app in the built-in browser, read
back from the ledger."

## Rules the maintainer set (all still in force)

- The model never owns a decision. It proposes a plan, a call and an
  answer; the runtime decides, from recorded facts, and records the
  decision.
- No word lists, no tuned limits, no per-tool or per-case fixes. When a
  second fix of the same kind appears, stop and design the one general
  layer.
- Tools are added, removed and switched at runtime and may be generated:
  nothing may depend on a tool, model or provider being named.
- Never blame the model: trace every failure to what Vak sent and answered.
- Zero users: no compatibility code, no stale code or documents. What a
  change replaces goes in the same change.
- Decision, judgement, evaluation and correction each live in their own
  crate and are called by the loop; the design must be solid, fast,
  efficient, self-healing and able to evolve under review.
- Keep AGENTS.md and every design doc true as the code changes.
- Disk: `cargo clean` after every full test run and after each commit, then
  build only `cargo build -p vak -p vak-server --bins`. Run tests with
  `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0`.
- Tokens: Claude Pro plan. Keep reads narrow, filter command output, commit
  each verified step, and refresh this file before the session limit.
- Live tests use the real installed home (`~/vak-home`) and local Ollama.
  Testing dev binaries against it is fine.

## Where main stands

See `git log`. The last commits before this work: plans and receipt checks
with block tools; the one argument reader; the plan check that reports what
it saw; the answer outline, its layout and the whole-answer Library save.
The full gate (fmt, clippy `-D warnings`, `cargo test --workspace`,
`scripts/check-version.sh`, `python3 scripts/check_doc_paths.py`) was green
at each.

The last commit of that session (the answer outline and layout, the
whole-answer Library save, the harness removal, doc 88 and this file) was
made at the maintainer's request before its full test run ended: fmt and
clippy were clean, the doc and version checks passed, and 205 of about 286
test suites had passed with none failing when the run was stopped.
`vak-server`'s unit tests (409) and its Library tests had passed on their
own. **First step of the next session: run the full gate once on a clean
tree and fix anything it shows before starting C0.** Two things in that
commit have not been seen in a live turn: the outline layout with a planned
answer, and the Save to Library button on an answer.

docs/plans/handover-2026-10-11.md has the detail of what was fixed, the
seven live run ids, and what is still open.

## Progress

- C0 done 2026-10-11: blast radius re-scanned, §12 decided (doc 88 §12).
- C1 done 2026-10-11: `crates/vak-control`, `EntryPayload::Decision`,
  `GET /sessions/{id}/decisions`; seen live on the local route (admission,
  frames, call verdict, accepted answer, ending read back from the ledger).

- C2 done 2026-10-11: `ChatRequest.settings`, adapter reach plus
  discovered and learned capabilities (`models::DispatchSettings`),
  applied settings and `reasoned` per attempt, `WorkReceipt.asked`; admin
  Run detail "How it went" (`/runs/{id}/decisions`); doctor fact. Seen live.
- Found live (to fix, all wires): the first step of every new turn
  re-prefills about 6k message tokens (17–20 s on the dev route) while later
  steps of the turn reuse the cache (0.4 s): the message part of the request
  changes shape at each turn boundary.

- C3a done 2026-10-11: step contract per purpose from `[control]`
  (shipped, steps, routes), every side dispatch framed by it,
  `direct_reply` removed, `/config/control`, Settings and admin editors;
  seen live (asked low, wire sent default, receipt says it reasoned).
- Next: C3b, tool loading by tool declarations only (no reading), the
  misread ledger removed; then C4.

## Order of work

1. **Doc 88, stages C0 to C7**, in order. Each stage: code, tests, docs,
   AGENTS.md, removal of what it replaces, live turns, commit, push, clean.
2. **Then the projection work that was paused**, on top of C6:
   - doc 87 stage 2: the answer form as the final step's contract, the
     server outline (`OutputTimeline.answers`, already built) and the
     client layout (`composeAnswer`, already built) seen live at
     1440 × 900 and 390 × 844, light and dark;
   - doc 87 stage 3: packs by block type, a stats pack and its test;
   - channels: a channel's text form of an answer follows the outline too.
3. **Smaller items left open** (handover-2026-10-11.md): `label` and
   `title` are two headings a caller cannot tell apart; the admin
   transcript does not show a call's arguments, which made refused calls
   hard to read back; `ArtifactKind::Card` has no creator left;
   `crates/vak-client-ui/tests/plain-words.mjs` fails on two "receipt"
   strings in `YourData.tsx`; `~/vak-home/pantry_weekly_routine.md` is test
   debris.

## How to run live

`cargo build -p vak -p vak-server --bins`. The `vak-home-live` entry in
`.claude/launch.json` (`vak-home-live`) serves `~/vak-home` on 8902 with a throwaway token
read from a scratch file (write a new token file first and point the entry
at it). Get the sign-in link with `vak open app --print --port 8902`. The
original Agent conversation is
`ses_01a12500-dbf0-7ec3-a5d8-078bd439258a`. Read a turn back with
`GET /admin/api/sessions/<id>/transcript`, `GET /runs` and
`GET /sessions/<id>/presentation`. After a client change run
`npm run build` in `crates/vak-client-ui` before building the server, or
the build refuses a stale bundle.

## Traps found this session

- Editing sources while a background `cargo test` is still compiling gives
  results for a mixed tree. Wait for it, or stop it first.
- Two `vak-server` `http_e2e` tests time out under the full parallel run
  and pass alone.
- A repeated request makes the model read the file the previous turn
  wrote, because the previous turn's card is always in context. Use a new
  request for each live case.
- Each time a reader guessed (cut a text in two, joined a list, read lines
  out of text) a live turn showed the damage. If a conversion needs a
  guess, it is not the reader's.
