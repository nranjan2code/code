# Handover — 2026-10-11: build the control plane

## Prompt for a fresh session

"Continue the control plane in /Users/nisheethranjan/Projects/Vak/code on
main, inside stage C6. Everything is committed and pushed; the full gate
was green on the code at `5ba4eaad3` (289 suites) and only this file
changed after it.

Read AGENTS.md (especially 'No word lists, no tuned limits' and
'Investigating a failure'), docs/design/88-control-plane.md (Status, §5a
including 'Still open here', §12, 'Open in C6', 'Carried into later
stages'), then this file: 'Where C6 stands', 'Done since', 'Next, in
order', Rules and Traps.

Done in the last session: the §5a time constants are `[control.limits]`
(`vak_control::TimeLimits`, `Core::time_limits`,
`Tool::time_limit_argument`), seen live in
`run_01a12b7f-2142-7062-85f7-b5c2879cf4f2`; the loop's receipts read tool
declarations (`Tool::command`, `Tool::looks_up`, `Tool::planned`) and
`is_substantive_command` is removed.

Take the next item under 'Next, in order', finish it whole (code, tests,
docs, screens), run the gate, commit and push, then the next.

Not yet checked from the last session: the Time limits screens at
390 × 844 and in light theme, the hook editor now that an unset hook
timeout stays unset, and the receipts change beyond one live turn.

Verify behaviour changes with real turns through the web app on local
Ollama, read back from `/runs/{id}/decisions` or `vak runs show <id>`. Do
not edit sources while a full `cargo test` run is going. Put back any
shared setting a live check changes. Another session is adding a
frosted-glass theme to `crates/vak-client-ui` on main: pull before work
and before each commit, and rebuild `dist-web` rather than merging it.

Work autonomously, report short progress lines, ask only on a genuine
product decision. Main only: no branches, PRs or worktrees. No provider,
model or tool is first-class. Never blame the model. Ollama is the dev
route; a hosted route (OpenAI Responses, `gpt-6-luna`, key in the
installed home) only for a few one-off checks."

## Where C6 stands (end of the 2026-10-11 sessions)

Pushed, gate green, seen live: the answer document (doc 87 §2). The model
answers with one loop-owned `answer` tool of blocks built at run time from
the admitted card tools; card tools are not callable by name; a later
`answer` in a turn replaces the earlier one; the outline places each
block; cards fit a phone (the harness checks all 113 types at 260/293/358
px). Also done and pushed: `checkpoint_before_effect` removed; the
repeated-answer check rests on the repeat alone; inspection receipts from
declarations; the unknown-tool and uncited-source texts in `vak_control`,
and the citation check applied per `answer` block; `vak runs show` prints
decisions; the outcome requirements guessed from words removed (verb,
extension and Office-term lists, the unrequested-file gate); durable work
only when asked explicitly (`/until-done`, `/goal replace`; maintainer's
choice), `Posture::managed`/`open_commitment` removed. Run ids are in
doc 88's Status and 'Open in C6'.

Done since, in the next session (2026-10-11):
- §5a time constants are `[control.limits]` (`vak_control::TimeLimits`,
  `Core::time_limits`), edited through `/config/control`, the client's
  Settings › Time limits and the admin console's Time limits panel. A tool
  names its time-limit argument (`Tool::time_limit_argument`) and the loop
  fills it (`3bf6d93af`). Seen live:
  `run_01a12b7f-2142-7062-85f7-b5c2879cf4f2`, a command call recorded with
  `timeout_ms` 45000 after setting 45 s in the client's Settings; the
  admin panel showed 45 s in force. In the client the group is on an
  Agent's Overview after "Change the shared defaults for every agent
  instead".
- The loop's receipt name tables are declarations (`Tool::command`,
  `Tool::looks_up`, `Tool::planned`); `is_substantive_command`, a word
  list, is removed. Doc 88 'Carried' lists the tool names the loop still
  reads. The same live run ended answered with no correction. The full
  gate was green on this tree (289 suites).

Next, in order:
1. Seen in the live check and not looked into: the admin transcript route
   (`/admin/api/sessions/{id}/transcript`) did not contain the newest
   turn of a long conversation, while `/sessions/{id}/transcript` did.
2. §5a's three open points: bash's 600 s `MAX_TIMEOUT_MS` (also in its
   schema), the fixed 600 s `request_timeout` in the `vak flow` and
   `vak plan` executors, and an operator-limit ending recorded in the
   turn's `Ending`. The CLI question prompt uses the shipped question
   limit, not the configured one.
3. The tool names the loop still reads (doc 88 'Carried'): `auto_approve`,
   the argument aliases in `normalize_tool_call`,
   `batch_has_file_dependency`, the audit command's and scheduled
   script's lookup of `bash`.
4. The horizon reading still sets the context profile (`derive_context`)
   and an unattended gate's fallback (`Authority::gate_fallback`).
5. A wire that refuses a named tool choice is not learned; the hosted
   named tool choice has not been sent live (needs a hosted check).
6. `scripts/measure_answer_form.py` with the real block schema on each
   configured route.
7. Open with no contract fault found: one "cannot create Word files"
   answer with `office_apply` offered but deferred (doc 88).
8. Then C7 (doc 88 §11).

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
- Disk: build lean (`CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0`) and
  keep `target/` warm (about 32 GB after full gates). `cargo clean` only when
  `target/` passes 45 GB or free space runs low, and at the end of a
  session, then build `cargo build -p vak -p vak-server --bins`.
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
- C3b done 2026-10-11 (three commits): tools load by declaration
  (`Tool::always_loaded`; bash, edit, write, webfetch and doc_read declare
  it), `DomainSet` and the act-to-domain table removed, the misread ledger
  removed, the tier-2/3 model reading and its `[intent]` keys removed. Seen
  live: a bash turn answered, decisions read back.
- The full gate was run on the C3b tree at the start of the next session:
  green (283 suites).
- C4 done 2026-10-11: `crates/vak-judge` (call verdicts from gathered
  facts, unrecovered failures, the plan check, the outcome evaluators moved
  from `vak-intent`); `ToolOutput::error(Fault, …)` at all 261 sites,
  carried over the worker protocol (version 5) and recorded as
  `CallEffect::Failed`; `ToolErrorKind::classify`, `never_ran` and
  `Reading::may_slice_capabilities` removed; `Refusal::Unknown`; the
  checkpoint taken at the first accepted call that declares
  `Tool::changes_workspace`. A ledger written before C4 has no fault
  records; purge the dev data home's conversations if an old one misreads.
- C5 done 2026-10-11: the correction ladder, typed findings and one
  renderer (`vak_control::correct`); freshness, grounding and topic checks,
  the stop gate's block budget and wording check, the worker gate's count
  and the repeated-call thresholds removed; a model step's timeout bounds
  silence, not duration. The time-limit audit is doc 88 §5a.
- Live runs for C4 and C5 (local route, read back from
  `/runs/{id}/decisions`): `run_01a12a3e-7526-7e41-9b49-c97b3e043f74` (a
  missing file: the failed read recorded as a `target` fault, no
  checkpoint), `run_01a12a3f-a503-7822-ae43-9156df57c40d` (a write: one
  checkpoint at the accepted call), `run_01a12a5e-0f19-7d80-ab77-61db1972930e`
  ("right now … current count": two cards and an answer in two steps, no
  freshness gate), `run_01a12a6d-1eb6-7cf3-bef0-7d596de66363` (a command
  that fails: the failure left standing is recorded as
  `correct {unresolved_failure, end}`, the honest answer stands and the
  client marks it a partial result).
- Seen in that last run, for C6: after a card's result ("Shown to the
  user") the model called the same card tool again instead of ending, was
  told the card was already shown, then a malformed call drew the
  tool-call-repair text ("continue the same request … regenerate"), and it
  ran the command twice more. Nothing tells the model that a shown card
  ends the work: that is the answer form's job. A bash call between two
  identical calls also clears the repeat memory, because bash declares it
  may change the workspace.
- C6 first slice done 2026-10-11: a required tool reaches every wire
  (`ChatRequest::must_call`, `Applied::forced`), and the plan check's
  reframe rung requires the tool a plan named for a missing card (`with`),
  once for each. Unit-tested; the hosted wires' request shape has not been
  sent to a hosted provider.
- Live for the C6 slice: `run_01a12a7f-b4c1-7e11-a137-d1eb6f4f7102` (a
  plan of two cards, both shown, answered in seven steps; the reframe rung
  was not needed, so a required step has only been seen in tests). In it
  `emit_comparison_card` and `emit_recipe_card` were each refused once
  ("options must be array", "steps must be array"): the model sent the
  list as text that was not valid JSON, and its next call was right. The
  admin transcript does not show a call's arguments, so the exact text was
  not read back. This is the nested-list weakness doc 88's "Open in C6"
  weighs against one nested answer document.
- Also done in C6 so far: the fixed 120 s reflection limit (three copies)
  and the 120 s heartbeat limit are removed (doc 88 §5a); the admin
  transcript shows each call's arguments; `scripts/measure_answer_form.py`
  measures a nested answer document against flat block tools on a local
  Ollama model (first numbers in doc 88, "Open in C6").
- Next: the rest of C6, starting from doc 88's "Open in C6": decide by
  measurement whether one nested answer document is written as reliably as
  flat per-block tools before retiring the block tools; then the items
  under "Carried into later stages".
- Slowness investigated 2026-10-11: in steady state the provider's prompt
  cache is reused across turn boundaries (first-step prefill 0.6-0.7 s);
  the 17-49 s first steps seen earlier came after a new build changed the
  system prompt (cold cache) and one miss that did not reproduce. The
  request bodies were captured with a throwaway logging proxy in front of
  Ollama (`VAK_OLLAMA_BASE_URL` pointed at it), not kept.

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

Write a throwaway token to a scratch file and point the `vak-home-live`
entry of `.claude/launch.json` at it (the file is gitignored). The admin
console's Settings › Models has "How the model thinks"; the client's
Settings (Shared) has the same group. `GET /config/control?scope=user`
shows the layer and the effective step settings.

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

- A full `cargo test --workspace` compiles its doctests last: sources
  edited during the run failed that step for a tree that was otherwise
  green. Do not edit until the run has ended.
- When the web app says "Your session expired", Settings silently drops
  back to General on a scope change. Open the sign-in link again.
- The client's Time limits and "How the model thinks" groups are on an
  Agent's Overview after "Change the shared defaults for every agent
  instead"; they are on no page of their own.
- A live check that changes a shared setting changes the maintainer's real
  home: put it back (`PUT /config/control?scope=user`) before stopping.
- A separate session is adding a frosted-glass theme to
  `crates/vak-client-ui` on main (started 2026-10-11): pull before work and
  before each commit, and rebuild `dist-web` rather than merging it.

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
- `python3 scripts/check_doc_paths.py | tail -1` hides its exit code;
  run it without a pipe before committing.
- The admin and client bundles must be rebuilt (`npm run build`) after any
  change under their `src/`, or `cargo check -p vak-server` fails on a stale
  `dist`.
- A select's accessible name in the built-in browser is its selected text,
  not its `aria-label`; find it by role (`combobox`).
- `answer` repair (2026-10-11, after C6 (9)): eight identical `answer`
  refusals in one turn came from four faults in Vak, all fixed and
  recorded in docs/design/15-reliability.md "Staged repair": withdrawing
  the loop-owned `answer` removed nothing; a step calling another tool
  restarted the stages; a withdrawn tool stayed in the frozen tool
  definitions; and the refusal said "sent as text" with a column number.
  `vak_tools::json_fault` now locates broken JSON for every tool. Open:
  the malformed-`blocks` rate on the dev route (doc 88 "Open in C6").
