# Composer commands and chaining — 2026-10-23

Status: historical. Initial local acceptance found three defects; the follow-up fixes and verification are recorded in composer-command-fixes-2026-10-23.md.

The maintainer asked whether all `/`, `@` and chaining work, authorizing local
testing on the dev machine. No runtime code or installed configuration changed.
The server ran on loopback port 8937 against the normal data home, without
`--gateway`, in disposable workspace `/tmp/vak-command-check.BnuY9F/workspace`.
The installed service on 8901 was left running. Only synthetic test files,
skills and a custom command were created. Credentials and real ledgers were
not opened; transcript evidence came through authenticated HTTP APIs.

## Verified behavior

- `/` lists the eleven built-in actions, available skills and the synthetic
  project command. Selecting a skill inserts `/skill:<name>`.
- After one skill, `/` offers other skills and custom commands, excludes the
  selected skill and excludes built-in actions. The second skill is likewise
  excluded after selection.
- `/skill:check-alpha /skill:check-beta /check-echo @notes.txt @second.txt`
  was assembled through the actual pickers and returned `CHECK_ECHO_OK` from
  the live provider. The command template explicitly requested that marker.
- Multiple file mentions complete in the same prompt. A separate live turn
  requested reading both files. The transcript API records successful `read`
  calls with bare paths `notes.txt` and `second.txt`, followed by the correct
  markers `THE_FILE_MARKER_8261` and `SECOND_9382`. The result UI nevertheless
  labelled this answer Partial result; no claim of full runtime acceptance is
  made from the markers alone.
- `/clear` clears the draft; `/files` opens the file picker; `/help` opens
  keyboard shortcuts; `/diff` opens Changes (correctly reporting that this
  disposable folder is not a Git repository).
- `/terminal` opens its pane, which reports that terminal access is disabled
  by this server. Terminal execution was not enabled or qualified.
- `/compact` dispatched and displayed its progress notification. Actual
  context reduction was not qualified on this short conversation.
- `/btw` produced the requested `SIDE_CHECK_OK` in the side panel, but its
  isolation failed as described below.

## Failures and limits

1. **Side-question isolation fails.** After `/btw Reply with exactly
   SIDE_CHECK_OK. Do not use tools.`, the main conversation displayed the
   question twice and its answer once, including after a reload. Immediately
   after the side turn, `GET /sessions/<id>/transcript` still returned only
   the original main prompt and answer. After the next main turn, that same
   endpoint included both copies of the side question and its answer ahead of
   the new main prompt. This is more than a transient rendering problem:
   the main transcript reacquires the side branch on subsequent work. Root
   cause and model-context impact need a focused regression test. The side
   handler is `side_chat` in `crates/vak-server/src/lib.rs`.
2. **Local command feedback is hidden after a settled presentation.**
   `/status` cleared the composer without showing its status. Idle `/pause`,
   `/resume` and `/stop` likewise produced no visible acknowledgement.
   `sendPrompt` appends system items for these outcomes, but `ChatPane` renders
   the projected answer instead of its item fallback. Running-task pause,
   resume and cancellation transitions were not qualified by these idle tests.
3. **Spaced file names have no consistent mention representation.** The file
   picker offers `space name.txt` and inserts `@space name.txt `.
   `file_mentions::resolve_file_mentions` scans only alphanumerics and
   `._/-`, so it cannot resolve that full selected name. If a file named
   `space` exists, it can instead resolve the wrong prefix. This audit
   observed insertion and inspected the resolver; it did not rely on a
   model guessing the intended path to establish correct normalization.

Chaining is specifically leading `/skill:<name>` tokens, optionally followed
by one admitted custom command and its arguments. It is not an arbitrary
built-in-command pipeline. A second custom command is argument text for the
first. `@` is a workspace file reference, not a general agent/account command.
Unknown names, directories and workspace escapes are not resolved as files.

## Checks and evidence

All 24 focused core tests passed:

```sh
cargo test -p vak-core --lib custom_commands::tests -- --nocapture
cargo test -p vak-core --lib file_mentions::tests -- --nocapture
cargo test -p vak-core --lib skills::tests -- --nocapture
```

These cover six custom-command cases, four file-mention cases and fourteen
skill cases, including chain order, duplicate skills, unavailable skills and
traversal/symlink refusal. They do not cover the three failures above.

Local evidence is in `/tmp/vak-command-check.BnuY9F`: `browser-result.png`,
`transcript-after-side.json` and `transcript-files.json`. The test conversation
is `01a16660-35b4-7ad3-a03c-c15a1cf680b3`. The temporary server was stopped
after testing. No exhaustive acceptance claim is made for installed skills,
plugins, other clients, viewport/theme combinations or running-task controls.
