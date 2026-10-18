---
name: record-drift
description: Keep Vak's documents true to the code. Use when the blueprint's health shows a failing record check or a stale-name count, when a change renames or removes something documents cite, or when asked to bring the docs up to date. Covers what the gate checks, the stale-name triage, and how to fix each kind of drift.
---

# Keeping the record true to the code

Status: reference

The record is every document under `docs/`, the root entry points and
this folder's methods. `docs/README.md` ("The record") says which file
holds which fact. Drift is a document saying something the code
contradicts. This method finds it and fixes it at its source.

## What the gate already checks

`scripts/check_records.py` (in the gate, and shown on the blueprint's
health) fails when:

- a document under `docs/` (or `DESIGN.md`, `PRODUCT.md`, a method here)
  has no status word, or a dated record is not `historical`;
- a living document names a current version, baseline or crate count the
  workspace contradicts;
- a living document shows a `vak` command the CLI does not have;
- a roadmap section lists a document whose status says it is built;
- `AGENTS.md`'s layout map misses a crate, or Markdown sits at the root
  that belongs in `docs/`;
- the design index is stale, there is not exactly one handover, or code
  changed since the last release with no CHANGELOG entry.

`scripts/check_doc_paths.py` fails on a cited repository path that does
not exist. `scripts/check-version.sh` checks version stamps. Each failure
names the file and what to do; do that.

## The stale-name report

`python3 blueprint/harness/run.py names` lists code names (snake_case
identifiers, `Type::name` paths, `name()` calls) that living documents cite
and that occur nowhere in the code. It is a report, not a check, because
three kinds of hit are not drift:

- external API names (a provider's request parameter, a model id);
- names a sentence marks as removed or retired;
- test or function names a plan proposed and the code named differently.

Triage one document at a time:

1. For each name, find what replaced it: `git log -S'name' -- crates`
   shows the commit that removed it, and its message usually says what
   took its place.
2. If the document describes current behaviour, correct the name and the
   sentence. If what it describes is gone, rewrite or remove the passage.
3. If the whole document no longer holds, change its status word
   (`historical`, or `superseded` naming what replaced it) rather than
   patching it. A dated note on its Status line says what was found.
4. If the name is external or historical, leave it, worded so the reason
   is plain (name the provider whose parameter it is, or say when it was
   retired).

Record a document that needs more than a fix (a rewrite) in
`docs/plans/open-items.md`, under "The record and the blueprint".

## Rules

- Fix the source, never the summary: the blueprint reads the record, so a
  correction belongs in the design doc, the tracker or the decision log.
- Never delete a document. Change its status word; git keeps the text.
- A guide's `reference` status says what was checked and when, not that
  everything in it is true.
- When a check misses a kind of drift that keeps recurring, add a check
  ([adding-a-check.md](adding-a-check.md)) rather than another manual pass.
