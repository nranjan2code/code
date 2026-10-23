# Installed turn names and missing Topics — 2026-10-23

Status: historical. Investigation of the installed 8.30.0 default-workspace
conversation reported by the maintainer; no runtime changes made.

## Findings

1. `crates/vak-client-ui/src/turnPairing.ts::turnDisplayLabel` reads recorded
   assistant document headings, then authored structured card titles. It excludes
   Derived runtime cards and does not use artifact titles or request text.
   `ChatPane.tsx::turnPreview` falls back to Turn N. The exported dashboard
   answer has bold prose and a named file link, but no Markdown heading; the
   artifact's authored title does not reach this resolver. This explains the
   screenshot's turn-number label without assuming a projection join failure.
2. The installed CLI's session-scoped Topics list reports automatic=true,
   revision=0 and total=0. Its supported transcript export contains zero
   `topics` calls. The Agent-wide list also returns zero Topics. Organisation
   was never recorded; these observations do not indicate a sidebar fetch fault.
3. Commit `2fdf4a0b7` added placement enforcement; `9d09f9c24` removed it during
   the card redesign. The current test explicitly accepts an answer without any
   placement or correction. The main-model prompt still asks for organisation
   and the tool remains available, but neither ensures a recorded decision.
4. Commit `bc8d1f92b` deliberately changed navigation from plan/request labels
   to answer titles with a numeric fallback. The 2026-10-22 decision log records
   both this choice and removal of forced Topics calls. Doc 101 and the older
   tracker still described the removed placement check; its Status/current
   boundary now identifies that history.

## Implication and follow-up

The earlier design's reliable authored metadata depended on mechanisms removed
by the redesign. Existing acceptance tested authored headings and permissive
Topics completion, so it did not establish useful names or automatic grouping
for ordinary untitled live answers.

Design turn naming independently of visible answer formatting and restore a
clear automatic-organisation contract while preserving main-model semantics,
human controls, ordinary-answer delivery and the removal of plan/layout tools.
Do not infer names from domain word lists, reintroduce raw-request hover text,
or silently restore forced Topics calls. No backfill or human-state mutation
was performed. Open work is tracked in `docs/plans/open-items.md`.

## Evidence

Read through the installed CLI: sessions in the default workspace, Agent-wide
and session-scoped `topics list`, and `export` for the reported conversation.
Private exported content remains outside the repository. Source/history
inspection identified the two intentional removals and the current tests;
this investigation did not run a new model request or change installed state.
