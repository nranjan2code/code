# 8.29.0 release and local installation — 2026-10-23

Status: historical. Native macOS aarch64 release and installed-component verification.

The maintainer requested release and local installation after the progress and
navigator acceptance. Source/release record `125f2aa63`, separate deep blueprint
`0f3a25b13`, and tag `v8.29.0` are pushed on main. No PR was opened.

- Full `scripts/gate.sh` passed: formatting, Clippy, workspace tests (801 seconds),
  versions, doc paths, records and advisories. Log: `/tmp/vak-8.29-gate.log`.
- `python3 blueprint/harness/run.py deep` passed with no needs-review lines.
- `scripts/release.sh` passed without skipped checks: clean-room frontend
  rebuilds matched committed bundles, blueprint matched, formatting/Clippy and
  all workspace tests passed, then the optimized native binaries were built.
  Log: `/tmp/vak-8.29-release.log`.
- Artifact provenance reports `vak 8.29.0 (0f3a25b13)`. All three SHA256SUMS
  entries verify: vak, vak-desktop and vak-delivery-worker.
- `dist/8.29.0/vak self install` installed `/Applications/Vakyartha.app`.
  `vak self services-sync` bounced gateway and restarted desktop.
- `vak self verify` verified all three binaries and desktop frontend;
  `vak self status` reports matching build/manifest and both services running.
- Final record full gate also passed: formatting, Clippy, workspace tests
  (807 seconds), versions, doc paths, records and advisories.
  Log: `/tmp/vak-8.29-record-gate.log`.
- AGENTS.md, prompt/control/card design records, superseded-plan historical
  boundaries, CHANGELOG, decisions, tracker and handover were reviewed/updated.

Artifacts are retained locally in `dist/8.29.0/`; the update feed uses localhost.
No public binary publication or other platform build is claimed. Earlier local
8.27.0 and 8.28.0 artifacts were preserved and restored after clean-room assembly.
The installed route and channel credentials were not changed. Status includes
model-discovery retry (Network) and the earlier removed Telegram-token warning;
installation verification itself is clean. Native WebKit visual qualification
remains open; browser/live acceptance is in the linked progress and navigator audits.

## Existing dev conversation after installation

The maintainer reported repeated “Conversation history is unavailable” messages
in the installed app and explicitly authorized erasing/resetting dev data.
Read-only diagnosis through Core/catalog and SessionLog found two unreadable
Decision records in conversation `01a1631a-abbc-7ae3-aa53-a07cde89b3eb`:
`Cause::TopicsPlacement` serialized as `topics_placement`, deliberately removed
with the old plan compatibility code. The strict indexed snapshot fails rather
than silently skipping those records. Binary integrity checks did not establish
readability of existing conversations; the earlier acceptance used fresh ones.

Used the installed app's supported More → Move to trash action for only that
conversation. The app opened a fresh conversation without the history errors.
No ledger, key or catalog file was manually deleted or rewritten. Retired-type
compatibility was not restored. Repeated identical errors from background
hydration are a separate UI issue, recorded in open-items.md.

The fresh installed conversation `01a16652-e896-7663-b8ce-afdbb93acd13`
completed the direct smoke question with “2 + 2 = 4.” Native accessibility
inspection and the settled screenshot show the answer and no history-load
errors. Screenshot: `/tmp/vak-8.29-native-reset.png`. This verifies the reset
and a native direct turn, not the outstanding full native progress/navigator
visual matrix. AGENTS.md and the release runbook now require existing-history
acceptance alongside a fresh turn.
