# Handover — 2026-10-13, after 7.1.0: where the work stands and what is next

Supersedes docs/plans/handover-2026-10-13.md for "Where things stand",
"Live runs" and "Next, in order". That file's history sections, and "How we
work" and "Traps" in docs/plans/handover-2026-10-12.md, still apply.

## Prompt for a fresh session

"Continue in /Users/nisheethranjan/Projects/Vak/code on main. Read
AGENTS.md first (especially 'Tool arguments are written, then read', 'No
word lists, no tuned limits', 'Investigating a failure', and 'Verification
before every commit'), then docs/plans/handover-2026-10-13-release.md
whole, then 'How we work' and 'Traps' in docs/plans/handover-2026-10-12.md,
then docs/design/90-file-state.md (status line, §3.4, §5, §6).

First run `git status` and `git log --oneline -6`; main should be clean at
or after the 7.1.0 handover commit. Before any live run, do the one-time
setup under 'Live runs' below (the dev data home was purged by the 7.1.0
fresh install). Then take the first item under 'Next, in order', finish it
whole (code, tests, docs, AGENTS.md where a rule changes, screens), verify
it with real turns, run the full gate, commit and push, then the next.

Live runs: never on ~/vak-home. Use /tmp/vak-live/ws13 in workspace-write
mode, as 'Live runs' says. Work autonomously with short progress lines; ask
only on a genuine product decision. Main only. No provider, model or tool
is first-class. Never blame the model: find what in Vak's contract made a
reasonable attempt fail; when the contract states the facts and the
failure remains, record what the model did and stop tuning. Do not edit
sources while a `cargo test` is running. Keep target/ under 45 GB. Token
budget is tight: lean reads, Haiku agents with exact briefs for mechanical
slices, commit each verified step."

## Where things stand

Released 7.1.0 (`1e377c7e7`, tag `v7.1.0`) after a green `release.sh`.
Installed fresh: the Mac at /Applications/Vakyartha.app (installed and
verified; first-run setup not done yet), and AWS (purged, built,
deployed; the owner ran setup: `openai-responses`, `gpt-6-luna`,
workspace-write). The SSH rule used for the deploy is revoked; the security
group allows 80 and 443 only.

What 7.1.0 carries from this stretch of work, each verified with live
turns on the local Ollama route:

- File state (doc 90): a call's versions are recorded before its file is
  described, so Vak's own `write`, `edit` and `bash` changes are never "an
  edit made outside Vak"; a direct workspace write is
  `VersionSource::Call { in_workspace }`, never a draft; a missing file is
  "was in the workspace as version X and is not there now".
- The workspace is named one way (resolved, as `pwd` shows it), in a
  code-owned `Workspace:` prompt line and in every outside-the-workspace
  error, with an accurate rule and no example file.
- `[empty-step]` quotes the step's thinking (the Ollama wire never replays
  it) and says the message is from Vak, not a new request.
- The preview card is two cards: `emit_file_preview_card` (`ui.preview`,
  `artifact_path` required) and `emit_markup_preview_card` (`ui.markup`,
  `html` required); a preview of a missing file or a folder is refused.
- The card argument naming a plan deliverable is `deliverable` (was
  `answers`, which models filled with content).
- The seed's file bullet keys on what the person keeps or runs; a snippet
  or a preview is not a file (3 of 3 runs then chose the markup preview
  first and wrote no file, against 1 of 3 before).

## Live runs

The 7.1.0 fresh install purged the default data home, so the dev server
has no route until one is set:

1. `cargo build -p vak -p vak-server --bins` (target/ was cleared by the
   version bump).
2. Make sure /tmp/vak-live/test-gateway-token exists (recreate a random
   token after a reboot) and that Ollama is running with the dev model.
3. Start `vak-tmp-live` from `.claude/launch.json` (it serves the dev build
   from /tmp/vak-live/ws13 with `serve --trust --port 8902`; the trust is
   for that process only and is what lets the folder's
   `permission_mode = "workspace-write"` apply).
4. Set the route once over the API (`PATCH /config` with the Ollama
   provider and a model the dev box has), then check `GET /config` says
   `WorkspaceWrite` and the route before sending a turn.
5. `/tmp/vak-live/turn.sh "<prompt>"` runs one turn in a new session and
   prints its calls; `/tmp/vak-live/sum.sh` summarises the last one;
   `/tmp/vak-live/wait.sh` prints the newest run's decisions. Sessions
   created over the API do not open in the client; send a turn from the
   client itself to check how something draws.
6. The browser pane's screenshot does not paint a sandboxed preview frame;
   check one through the DOM (its `srcdoc`, `sandbox`, CSP).

## Next, in order

1. Done 2026-10-13. The HTML-in-`text` spiral did not recur in 10 live
   runs; its refusal already names the missing `html`, the unexpected
   `text` and every field the card takes (the contract states the facts).
   The runs found a real fault instead: a markup call carrying a stray
   `"semantic_type": "media.image"` was accepted as `ui.markup` and acked
   "Shown to the user", but Core's rebuild re-routed it by that field to
   the media card, failed, and recorded nothing (2 of 3 runs). A call is
   now recorded as the block it was made to (`presentation_info`), and a
   card the host cannot record is a `Fault::Tool` error, never "shown".
   Live after: 4 of 4 markup cards recorded with an id.
2. Doc 90 stage 5: done 2026-10-13 (doc 90's status line), to the mockup
   the maintainer approved as it stands, with the Library's look reworked
   (one primary action, the rest under More; state chips; file-type icons).
   Open: version titles, "You asked" and "It read first" need record
   fields first; Compare with your folder.
3. Doc 90 §5.1, Agent half: done 2026-10-13 (see doc 90's status line).
   Live: a new conversation's `doc_read` of a draft was told "made in turn
   T of conversation C, made from version V" and read the draft. Found on
   the way and fixed: a new file named by an absolute path through `/tmp`
   was never recorded; `office_apply` said "the directory does not exist"
   for a path outside the workspace and did not name the workspace. Still
   open: `write` creates a missing folder but `office_apply` refuses a new
   file whose folder is missing (a model ran `mkdir` to get past it); the
   note does not list the inputs a turn read.
4. The evaluation item's second half: the maintainer decided (2026-10-13)
   to leave a card that claims a draft no call made: with no file
   deliverable in the plan no typed fact carries the claim, and reading the
   card's words is not allowed. Still to check: whether a card-only answer
   is evaluated `Unknown` by `evaluate_response_with_failures`.
5. Model choice, recorded with the contract now stating the facts (do not
   tune wording further without a new contract cause): after the work is
   done a turn sometimes makes a few more reads or writes first; a model
   sometimes writes outside the workspace (`/workspace/…`) despite the
   `Workspace:` line.
6. Flaky, cause not found: `agent_pause::pause_without_stop_running_…`
   (2 in 18 runs; prints the session list and transcript when it fails);
   `http_e2e::mode_change_revokes_run_waiting_for_approval` (one hang).
7. Control plane leftovers (docs/plans/handover-2026-10-11-control-plane.md
   'Next, in order' 6): the horizon reading in `derive_context` and
   `Authority::gate_fallback`; a wire refusing a named tool choice is not
   learned; `scripts/measure_answer_form.py`; then C7 (doc 88 §11).

## Traps learned in this stretch

- An untrusted project's `permission_mode` is dropped (privileged key);
  `serve --trust` trusts the folder for that process only.
- The ledger keeps a call as the model first wrote it; a regenerated call
  is not in the transcript.
- `release.sh` rebuilds the committed frontend bundles in place, so the
  tree looks dirty while it runs: start the AWS build after it ends.
- The AWS inventory uses the `vak-hosting` SSO profile
  (`aws sso login --profile vak-hosting`), not `default`.
- Running the remote `setup` over SSH was refused by the permission
  classifier; the owner runs it.
