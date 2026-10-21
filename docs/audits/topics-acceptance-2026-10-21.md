# Topics acceptance — 2026-10-21

Status: historical. Implementation-session evidence for the initial
conversation-scoped [Topic contract](../design/101-topics.md). Further scope and
open verification belong in the tracker, not inferred from this dated record.

## Implemented boundary

Topics are shared runtime organisation, not a browser-only feature. The main
model has a broker-owned tool; people have shared web/desktop controls, Admin
inspection and `vak topics` commands. Names and outcome labels are dynamic.
Membership remains separate from context admission, source support and grants.

The sealed per-conversation Document records actor/trace/time, preserves earlier
versions, and uses existing fencing/CAS. A human whole-request correction
replaces prior outcome placements in the current view, including explicit
unassignment. Human corrections and automatic-off are checked inside CAS.
Archived Topics reject new membership and require explicit reopening. The
catalog reconciles removed membership edges and inherits source audience,
trash and erasure boundaries.

## Deterministic checks

- Storage covers many outcomes/Topics, human correction, whole-request move and
  unassignment, automatic-off races, archive/reopen, stale revisions, corrupt
  state, sealed conversation isolation and mutation provenance.
- Atomic create-with-outcomes writes one Document version, attributes model
  contributions to the Agent acting for the requester, and refuses corrected
  outcomes without leaving an orphan Topic.
- Catalog coverage checks inherited audience, trash/restore and removal of
  stale `assigned_to` edges after correction.
- HTTP tests run real bearer authentication and live participant-audience
  middleware: owner mutations, stale write refusal, cross-conversation turn
  refusal, guest read, wrong-audience refusal, guest mutation/Agent-list refusal,
  revoked-reader refusal and trash hiding.
- Broker tests cover model organisation, exact turn addresses and human-only
  lifecycle controls. Tool parsing tests cover bounded paged discovery and
  dynamic outcome arguments. Children do not inherit the parent's Topic store.
- Client tests include a Topic link across a mocked 1,000-turn gap; selection
  opens a snapshot around the exact turn rather than loading intervening pages.
  Both client target bundles and the Admin bundle were rebuilt.

## Doctor checks

The shared health suite passes 20 tests. Four Topic-specific checks cover corrupt
sealed metadata without cursor advancement, valid multi-membership with archive
and automatic-off, missing turn references, and custom Agent ownership. Doctor
reads indexed sources through canonical headers and a read-only catalog; it does
not materialise conversation history, repair records or change preferences.
Unindexed conversations remain explicitly unobserved.

The rebuilt debug CLI was run with `vak doctor` from the authorised local
`vak-home` workspace. Its Topics check passed over 108 indexed conversations
and one Topic Document, while reporting the unobserved boundary. The report
is captured at `/tmp/vak-topics-acceptance/doctor.txt`. CLI help also confirms
that automatic and pin controls accept explicit `true` and `false`.
The installed service was not replaced.

The full repository gate passed before the implementation commit: workspace
formatting, clippy with warnings denied, all workspace tests, versions,
documentation paths, records and dependency advisories. The HTTP Doctor
contract assertion includes the new Topics check.

## Browser observations

The actual Sidebar, WorkspaceHeader, TopicsPanel and per-request correction
components were exercised against the editable API fixture at
`crates/vak-client-ui/tests/topics.html`, served on a temporary loopback dev
port. This verifies component behaviour with synthetic records, not an installed
desktop-shell run or all real channel combinations.

At 1440 × 900 and 390 × 844, light and dark, the Topic sheet was readable and
scrollable, with no horizontal document overflow. Controls wrapped on the
phone. The temporary browser viewport override was reset. Dynamic creation,
automatic-off, archive visibility, reopen and multi-Topic correction were
observed. A simulated concurrent edit produced a conflict, refreshed the
checkboxes to the other person's actual membership and kept the correction
visible for review. Explicit unassignment persisted after reopening the control.
No browser warning/error was reported in these fixture checks.

Screenshots and the synthetic provider-test evidence are local under
`/tmp/vak-topics-acceptance/`; they are not a shipped data source.

## Live OpenAI run

The maintainer authorised the local `vak-home` workspace and stored OpenAI key
for a lean Luna test. The provider's live catalogue returned `gpt-6-luna`; no
hardcoded fallback or more expensive substitute was used. A rebuilt debug
binary ran a temporary loopback server on port 8931, with gateway disabled.
The test bearer stayed in process memory. The provider credential remained in
the normal credential store; neither key was printed or copied into a test
file. The temporary server was stopped.

Two runs completed. The model created a synthetic Topic, placed two outcomes,
and a follow-up used `recall` and Topic lookup; API inspection returned the exact
assigned turn. The first call sequence was `plan`, `topics`, `topics`, `topics`,
`emit_callout_card`; the second was `recall`, `topics`, `topics`. The first
exceeded the requested target of three calls. Combined recorded usage was
29,897 input and 2,023 output tokens. This was a functional organisation test,
not a comparison of alternative model costs or universal semantic resolution.

Topic-open API responses measured 2,366 bytes after the first run and 3,146
after the follow-up; the full transcript API response after the follow-up was
13,379 bytes. These are response-size measurements, **not measured token
savings**. The sequence also does not establish that Topic lookup caused the
follow-up's earlier `recall` call. The live script did not send explicit human
`topic_id` focus; deterministic admission tests cover that separate path.

Following this run, create-with-outcomes was combined into one atomic tool
operation to avoid a dependent creation/assignment step. That improvement has
deterministic coverage; no further paid run was used to claim a new call count.

## Limits

Topic identity is currently scoped to one source conversation, aggregated under
its Agent. This is not cross-conversation merging, a semantic summary service,
or a lifetime-scale incremental graph store. A conversation's sealed snapshot
still grows with its memberships. Topic title search is discovery rather than
a semantic classifier. Remote channel delivery, native terminal interaction and
installed desktop-shell behaviour were not all live-tested by this fixture.

Focused sends wait while a run is busy because the existing steering path does
not retain independent destination metadata. Goal/managed/automatic focus
combinations fail explicitly. Natural-language lifecycle approval remains open;
people can archive/reopen through UI or CLI controls. No release, installed
service replacement, data reset or migration is part of this work.
