# Composer command fixes — 2026-10-23

Status: historical. The three defects from composer-commands-2026-10-23.md are fixed and verified with rebuilt debug binaries, component acceptance and authenticated live-provider checks. No release or installation was performed.

## What changed

- `SessionLog::restore_branch` appends and syncs a `BranchRestored` entry.
  Its parent selects the main head; `from` preserves the side head for audit.
  This makes a return survive read-only opens and writer reacquisition without
  rewriting or deleting entries. The server removes its duplicate question
  append, restores after successful and failed side turns, renews the side
  cancellation token per question, and reports restoration failure instead of
  putting a contaminated log back in the main handle. The spawned task carries
  its tracing span; new diagnostics contain an error kind, not error text.
- The chat pane renders local system feedback alongside a settled server
  answer, instead of discarding it when the server projection replaces the
  item fallback. The answer is still drawn only once.
- The composer quotes selected paths with spaces, punctuation, escapes or
  non-ASCII characters using JSON string syntax: `@"space name.txt"`.
  Completion supports quoted partial names and Unicode combining marks.
  The resolver checks the decoded full filename against the canonical
  workspace before stripping `@`, and retains the quotes for the model.
  Unknown, malformed, directory and escaping references remain untouched;
  no filename-prefix fallback is used for a quoted reference.

## Verification

- Clippy passed for `vak-session`, `vak-core` and `vak-server`, all targets,
  with warnings denied. Formatting, frontend TypeScript checks, record checks,
  doc paths and version stamps passed.
- All 216 session tests passed (two other tests remain ignored), including
  a durable branch-return regression across read-only opening, releasing and
  reacquiring the writer, another main turn and reopening again. Side records
  remain available through the sealed ledger.
- All 27 server-extension tests passed. The expanded side-question test
  checks the secured HTTP transcript and browser snapshot, a subsequent main
  turn, the exact messages sent to the provider (no side question or answer),
  and exactly one stored side directive. The existing cancellation/reuse test
  also passed.
- Six core file-mention tests passed. They include spaces with an existing
  shorter filename, Unicode, punctuation, JSON escapes, unknown references,
  malformed quoted text, directories, traversal, absolute paths and symlink
  escapes. The pure client mention test passed.
- Both embedded frontend bundles and the debug `vak`/`vak-tool-worker`
  binaries were rebuilt. The linker emitted its existing large-unwind-table
  warning; compilation succeeded.
- `/app/tests/composer-commands.html?run` passed all 18 component checks at
  1440 × 900 and 390 × 844, both light and dark. It exercises the real
  composer and chat pane with a settled projection: once-only visible command
  feedback, quoted picker choices and two skills followed by a custom command
  and a quoted file reference. No horizontal overflow was observed.

## Live acceptance

The rebuilt server used loopback port 8937, the normal data home and disposable
workspace `/tmp/vak-command-fix-evidence/workspace`, without `--gateway`.
The installed service on 8901 and installed configuration were left unchanged.
Only synthetic test files/skills/commands were created. Credentials and real
ledgers were not opened; inspection used authenticated app/HTTP routes.

Conversation `01a16672-59a0-7062-a17c-f847f8af3e88` verified:

1. Picker insertion produced
   `/skill:check-alpha /skill:check-beta /check-echo @"space name.txt"`.
   The live command returned its expected `CHAIN_FIXED_OK` marker.
2. `/status` displayed "Nothing is currently tracked for this conversation."
   after that completed answer.
3. `/btw` returned `SIDE_ISOLATED_FIXED_7135` in the side panel. Closing it and
   reloading left no side question or answer in the main conversation or Agent
   subtitle.
4. A later main turn read `space name.txt` and `notes.txt` successfully and
   returned their exact markers. A separate file named `space` held a conflicting
   marker and was not read. The transcript and browser snapshot still excluded
   the side question and answer after this turn. The generic outcome grader
   labelled the read answer Partial result, as it had in the initial audit;
   successful tool results, rather than that label or model prose alone,
   establish the file reads here.
5. An idle `/pause` produced its visible "Nothing is running to pause."
   acknowledgement after the later answer.

Evidence is under `/tmp/vak-command-fix-evidence`: four exact-size theme/viewport
screenshots, `after-main-transcript.json`, and `after-main-snapshot.json`.
The temporary server was stopped and its throwaway token removed after testing.

This does not qualify process termination during a side turn, every installed
skill/plugin, native WebKit, or running-task pause/resume/stop transitions.
Earlier test conversations were not rewritten or repaired. Live acceptance
did not release or install the changes. Subsequent commit verification is
recorded below.


## Commit verification

The maintainer subsequently requested commit and push. The full
`scripts/gate.sh` passed on 2026-10-23: formatting, workspace Clippy with warnings
denied, workspace tests (764 seconds), version stamps, doc paths, records and
advisories. Its first Clippy attempt found an exhaustive entry-kind test helper
in `crates/vak-agent/tests/it/work_receipts.rs` missing `BranchRestored`; that
helper was updated before the successful gate. All client Node tests passed
as well. The implementation/record and generated blueprint use separate commits.
No release or installation was performed.

Full gate log: `/tmp/vak-command-fix-full-gate.log`.
Client tests: `/tmp/vak-command-fix-client-tests.log`.
