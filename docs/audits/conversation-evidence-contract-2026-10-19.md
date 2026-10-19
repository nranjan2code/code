# Conversation and evidence contract repair — 2026-10-19

Status: historical: shared state and evidence contracts implemented locally;
focused tests and real research inspected; final browser matrix and installed
acceptance remain open. This is not a release or completion of semantic review.

## Scope

The maintainer asked for general design repairs after
[the ChatJimmy trace](chatjimmy-response-trace-2026-10-19.md), explicitly rejecting
hardcoded topics and spot fixes. The changes use existing conversation,
capability, plan and outcome structures; no provider exception, keyword list,
extra retry budget or model loop was introduced.

## Implemented

- One `/sessions/{id}/snapshot` projects transcript identities and output from
  the same immutable ledger view. The active-run path uses one committed prefix
  for both and marks it running. It uses historical-session access and the
  existing owner Core resolution. The client no longer hydrates from separate
  transcript and presentation requests.
- `ConversationHydration` tracks pending read generations separately from
  applied fingerprints. Deferred reads do not become applied state. New
  requests, stream boundaries and retries invalidate old reads. Slower reads
  cannot replace newer reads or publish stale errors. Identity and presentation
  apply in one Solid batch. Existing durable-id turn pairing is preserved.
- Card tools acknowledge acceptance for presentation, not client visibility.
- `source-discovery` is a declared work domain. Frozen descriptors retain
  declarations; the prompt names admitted discovery capabilities, distinguishes
  an undeclared external server from absence, and distinguishes discovery from
  fetching a URL. Native/ability/MCP declarations use the same vocabulary.
- A plan entry can declare evidence `questions`. Only accepted plan receipts
  change the evaluated plan. Its questions become separate evidence
  requirements; retrieval without questions still carries a claim-support
  requirement. Neither cards nor recent tool success certify semantic support.
- `Tool::retrieves_evidence`, forwarded by wrappers, distinguishes source
  retrieval from planning/display/commands. Integration catalogue listing is
  not retrieval. Retrieval age does not establish source freshness or relevance.
- Outcome projection uses separate fields rather than joining evaluation text,
  receipt ids and completion with a delimiter. Authored punctuation cannot
  change status. Card-only answers receive the same evaluated requirement ids,
  evidence verdict and receipt links as prose; user prompts are not outcomes.

## Verification

The original failure schedule was reproduced against the actual hydration
function before changes. Node tests now cover deferred-then-identical reads,
out-of-order reads, admission invalidation, duplicate snapshots and failed
application. The real ChatPane harness passed 22 assertions before the atomic
endpoint change, including recovery of an optimistic message's durable id,
visible saved card, one copy of closing prose, duplicate reconciliation and
viewport fit. Browser access was then disabled in Work settings. Final browser
acceptance after the endpoint change, and phone/light/dark matrix, are unverified.

Server tests drive the snapshot route through real `require_bearer` and
`enforce_participant_audience` layers: admitted read, wrong conversation, wrong
audience and revoked bearer. The active-ledger test checks running state,
matching cursor and transcript ids represented in the presentation prefix.
Projection tests cover delimiters, corrupt evaluation data and card-only
outcomes. Core integration drives the real agent and worker: an unanswered
plan question and an irrelevant successful source read both finish execution
without semantic completion. The worker path must point to the built binary;
the initial test failure used the test executable as its worker, then passed
with the real worker configured.

A live research turn used the configured OpenAI Responses route in disposable
workspace `/tmp/vak-reconcile-live/workspace`, never the implementation checkout.
Session `01a152cc-8ccd-7c71-8111-f45d28510485` declared three evidence questions,
read sources, distinguished company claims from independent measurements, and
returned summary/sources cards with the new acceptance wording. The new server
snapshot retained both cards and evaluated requirements including
`plan-evidence-1-1` through `plan-evidence-1-3`; completion remained unknown,
with review recommended. The claims themselves are not certified by this audit.
A second live topic, sodium-ion battery progress without a supplied URL,
session `01a152d2-d6a3-7322-a1e7-1121a3d94cee`, completed in 19 model steps
with three cards. It declared four evidence questions, navigated primary
sites and made zero Google/Bing/DuckDuckGo search-page fetches. The answer
separated company specifications, certification statements and independently
established field outcomes. This is one sample per topic, not a measured rate
of research quality or proof of exhaustive coverage.

Private API captures and check logs are in `/tmp/vak-chatjimmy-trace/` and
`/tmp/vak-reconcile-live/`, outside git. No credentials are printed or committed.

## Remaining boundaries

This fixes state reconciliation and false certification, not semantic truth.
A general semantic reviewer with linked source/claim assessments and feedback
through the existing correction ladder is still unbuilt. Ordinary structural
outcome checks conservatively leave support unknown; they do not run a model
review secretly or promote a model's self-assessment to runtime proof.
Setup/Inbox connection guidance using the discovery declaration remains open.
The client retains its legacy stream view during a live run; all view storage
has not been replaced. No installed app was changed or original turn rerun.

## Check results

- Core unit tests: 556 passed, one ignored.
- Agent integration tests: 219 passed.
- Tools unit tests: 165 passed, two ignored; judge unit tests: 35 passed.
- MCP retrieval declaration: one passed.
- Projection suite: 50 passed, one ignored, including card-only status and
  delimiter/corruption regressions.
- Actual-source App hydration tests also cover the new snapshot validation:
  mismatched revision, missing message ids and unsupported schemas are refused
  before committing view state. This supplements, not replaces, pending
  final browser verification.
- Client test suite has one pre-existing failure in `preview-isolation.mjs`:
  its old `body{font-family:…}` assertion contradicts the existing
  `:where(html)` base stylesheet. It reproduces from unchanged HEAD in an
  isolated temporary copy. This task does not edit it.

Both client bundles were rebuilt. Focused all-target Clippy passed with
warnings denied; normal binary linking reports the existing macOS compact
unwind-size warning. Record/path/version checks and diff whitespace checks
pass. No full workspace gate, commit, release or install was performed.

Final HTTP readback from the rebuilt disposable server confirmed coherent
cursor/identity snapshots for both live sessions. The hardware answer retains
two cards, each with unknown completion and all five evaluated requirement ids;
the battery answer retains three cards, each with unknown completion and all
five evaluated requirement ids. Both user prompts have no result outcome.
This verifies the server/model/data boundary; final browser rendering of the
new endpoint remains unverified because browser access is disabled.

Final client checks: 34 of 35 test files pass; the remaining file is the
unchanged-HEAD preview-isolation failure described above. Both frontend bundles
and the CLI/worker/server binaries build. Blueprint generation reports no new
`needs review` entries. Generated views include the pre-existing uncommitted
Storage simulator work and must follow the separate-view commit rule if later
committed. Nothing has been committed in this task.

The disposable server and Vite process were stopped after readback; the
throwaway server token was removed. The installed service was left running.


## Release preparation continuation

The maintainer subsequently requested commit, push, a version bump, release
and local installation, targeting 8.13.0. The first full gate passed fmt and
workspace Clippy but stopped in the CLI integration suite: 14 passed, two
sync lease tests failed at takeover with catalog encryption unavailable.
The existing 8.12.0 release binary reproduces the same failure in isolated
homes: a standby-created catalog scope key is removed/replaced by the next
pull while the old encrypted database survives. The maintainer explicitly
chose to fix the blocker and require a passing gate rather than waive it.
The import lifecycle now marks encrypted projections in the state registry,
retires cached catalog handles, resets the declared projection trees and
rebuilds from the imported records/keys. Ordinary read corruption still fails
closed. Release and installation results will be recorded when completed.


The sync lease cases and sync-erasure case pass after explicit import
invalidation. The round-trip fixture raced its own asynchronous catalog
initialization; it now explicitly catches the catalog up before asserting a
second push is unchanged. Its no-key case now requires import completion to
fail closed, consistent with encrypted catalog rebuilding. The complete
round-trip then passes. The obsolete client-vocabulary equality tests required
client-side text stripping that the shipped typed server projection retired.
Their entry point now drives the secured snapshot HTTP route for every
ControlKind, authored marker lookalikes and matching transcript/presentation
identities, replacing list equality without restoring duplicate marker lists.
Client `prose-fidelity.mjs` checks authored markers survive.


The Telegram gateway packet regression differed only by one terminal newline
between the generated fixture and the normalized settled reply (9,815 versus
9,814 characters). Its fixture is now canonical before scripting the provider;
full-content equality, multiple chunks, size limits and balanced HTML checks
remain. The client preview theme regression now expects the shipped zero-
specificity inherited default rather than an obsolete body rule. All 35 client
test files pass. No product text-stripping lists or whitespace exception were
added to satisfy these tests.


A complete workspace test pass recorded 3,850 successes and no failures.
The gate then caught stale citations after removing the old list-equality
fixture. Its documented entry point is now retained as a secured HTTP
regression for the current structural contract, and the living contract/docs
no longer claim that clients maintain duplicate marker lists.


The final full `scripts/gate.sh` passed without a waiver: 3,851 tests passed,
zero failed, 23 ignored; fmt, workspace Clippy with warnings denied, version,
document paths, records and advisories all passed. Log:
`/tmp/vak-chatjimmy-trace/full-gate-green.log`. The secured HTTP vocabulary
regression passes alongside the other integration cases. Source and test
files in the isolated release checkout were compared byte-for-byte with the
tested tree. Unrelated Storage simulator source and records are excluded.
