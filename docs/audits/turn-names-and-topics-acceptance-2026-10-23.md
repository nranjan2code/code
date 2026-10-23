# Turn names and Topics: development acceptance, 2026-10-23

Status: historical. Record of the approved post-answer implementation and its
qualification on this date; the installed 8.30.0 investigation remains a
separate earlier record. Full-gate results are recorded at completion below.

## Contract and implementation

The maintainer approved useful names and automatic Topics within the current
answer design, without restoring plans or forced answer-tool calls. Names and
Topic placement run independently after the answer on the configured main
model, using existing Trigger, claim, Run, admission and spend services. Existing
Topic Documents hold the metadata; source turn addresses remain canonical.

The maintainer's question about turn links exposed an incomplete first input:
it contained only the current request and final answer. The implementation now
uses the answer's recorded ContextSelection, its admitted exchanges and exact
stored summary. It supplies recorded provenance links and recall call addresses
without treating them as semantic dependencies or admitting Topic history.
Older explicit backfill resolves visible closing records and excludes later
dialogue. No raw provider tool bodies are copied into the auxiliary input.

The complete auxiliary request is stored before dispatch and reconstructed from
its new ledger entry. Each contributing person's scope seals a layer of the
request; the outer conversation object holds no copied contributor text.
Human naming and placement win against late results, automatic-off disables
grouping, archived Topic ids are rejected, and superseded pending triggers
cannot apply their results. Failures are visible and explicitly retryable.

## Checks

- The Core integration fixture blocks metadata dispatch, proves the main answer
  has returned, then accepts names and Topics for a reply with no heading. Its
  follow-up is only “Improve that”: the metadata input contains the original
  admitted turn address and subject. Earlier backfill excludes that later ask.
- Session tests cover atomic outcomes, human corrections/automatic-off, archived
  and unavailable targets, stale retries and sealed request roundtrip.
- Server tests use actual bearer/participant middleware, including refusal of
  guest organisation and rename writes. Catalog tests check name audience,
  trash and restore alongside Topics.
- The full gate exposed a source-run assertion that incorrectly included
  independent pending metadata runs. The assertion now checks the admitted
  answer runs; the metadata integration test separately checks its live status
  and source-session binding. Manual backfill uses the existing attached writer.
- The telemetry gate exposed an independent metadata run inheriting the source
  answer's tracing span. Metadata now starts outside that span and enters its
  own ordinary run span. The trace test checks distinct roots and trace ids.
- The complete durability measurement waits for metadata settlement: about
  9 KB of ledger growth and 13 record syncs per fixture turn, within the 20 KB
  byte allowance and the original eight-sync answer allowance plus eight
  explicit metadata boundaries (input, pending Document, Trigger, claim, run
  open, source binding, accepted Document and settlement).
- Browser component checks used the actual shared Sidebar, ChatPane and
  TopicsPanel at 1440 × 900 and 390 × 844 in light and dark, including pending
  metadata, completed plain-answer names, sidebar Topics, and inline human
  rename. Phone horizontal scroll width was 390. Screenshots remain in
  `/tmp/vak-org-visual/`. A held old-workspace fetch followed by a workspace
  switch also recovered the Topic list. This fixes the stale-request path that
  invalidated an old request without clearing its loading state.

## Real-model check

The disposable workspace is outside the checkout, served by the debug binary
on loopback port 8943 with a throwaway process token. Credentials and real-home
ledgers were not opened. Supplied synthetic park notes avoid external research.

The first live attempt named the plain answer, but the grouping wording allowed
its short substantive proposal to remain explicitly ungrouped. The general
contract was clarified: substantive work gets a Topic from its first request
regardless of answer length; simple standalone answers may stay ungrouped.
This is a model semantic decision, not a keyword rule or a domain classifier.

On the revised development build, session
`01a167f4-f0b8-7ca0-bb39-9f6cfe243464` answered the two-sentence proposal request
without headings or tool calls. The independent step produced “Draft a park
restoration proposal” and created “Park restoration proposal”. Both were
complete, with no pending or failed metadata. A second plain request, “Improve
that by adding a clear first step and what we should review next”, was named
“Refine the park restoration proposal” and reused the exact same Topic id.
Neither final-build answer called a tool. The earlier live follow-up also
proved the actual browser shows names in the turn slider and Topics in the
Agent sidebar. After rebuilding and restarting the development service, the
sidebar loaded correctly and the navigator retained its recorded name. A human
rename to “Neighbourhood park proposal”, followed by the real manual retry
endpoint, preserved the human title, completed successfully, reused the single
Topic and left exactly two placements, one for each source turn. The installed
app was not replaced. The two synthetic acceptance conversations and the empty
conversation opened by the browser were archived and moved to recoverable Trash
through authenticated app APIs; an Agent Topic query then returned no synthetic
Topics. The temporary services were stopped.

## Completion checks

The full `scripts/gate.sh` passed: formatting, workspace Clippy with warnings
as errors, all workspace tests, versions, documentation paths, records and
advisories. The final log is `/tmp/vak-org-gate-push.log`. All 46 client Node
test files passed, both client bundles were rebuilt, and the live follow-up,
human retry and browser checks above passed. Generated views are regenerated
in their required separate commit after the implementation. The maintainer
requested committing and pushing all work; no new release or installation was
requested.
