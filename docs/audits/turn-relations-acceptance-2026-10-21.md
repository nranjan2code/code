# Turn relation and sizing implementation acceptance — 2026-10-21

Status: historical. Implementation follow-through after the maintainer approved
[the review](turn-relations-and-sizing-2026-10-21.md), with no compatibility or
migration requirement. This record describes the implemented boundary and its
verification; it does not certify every language, subject or model.

## Built

- No lexical inference of natural-language thread continuation. Human goal
  commands retain explicit control lineage. Ordinary context admission uses
  only the actual recent visible settled card, labelled as a historical
  candidate. Older unfinished work does not become a current instruction.
- Ordinary planning reads the active turn and addressed closing card; the
  selected Card projection does not reconstruct its tool payloads. The
  diagnostic-only two-hop graph planner is removed. Existing provenance
  records remain available; the model determines dependencies through recall.
- Structured ContextSelection records are written before dispatch, with exact
  source addresses, policy version and pre-request leaf. Replay reads that
  leaf. The former duplicate context-plan activity is no longer written;
  the inspection API reads the structured record.
- `recall` supports `opens` for several exact sources and ranked candidate
  pagination with `next`. Pages advance past excluded candidates rather than
  applying a turn-age window. A batch preserves successful opens and misses
  separately. Production indexed search/open delegates membership, branch,
  access and turn marks to Core without replaying the full TurnIndex first;
  returned contributions retain conservative private/untrusted provenance.
- An unknown route has a configurable provisional request resource envelope,
  `[control] unmeasured_context_tokens` (65,536 shipped), recorded separately
  as `resource_ceiling`. It is never a discovered model window. The default
  lives in vak-control; measured/discovered bounds and output reserve still
  constrain requests. No provider/model/domain-specific table was introduced.
- Prompt, plan, recall and optional layout-skill guidance distinguish requested
  coverage from enrichment, make summaries optional, and discourage repetition
  across shapes. Query/open resolves missing historical facts and constraints,
  including implicit references, before clarification when necessary.
- Card acceptance resolves beside/replacement requests to exact Presentation
  entry ids in `PresentationRecord.placement`, before visibility changes.
  Same-part/prose boundaries apply; a removed anchor never rebinds. One row
  composer serves live, settled and Canvas views. Live/settled card item ids
  agree; live server/client stores remove the addressed replaced presentation.
  Form reading widths share
  space unequally when they fit and wrap when they do not; CSS column flow and
  summary/section reordering were removed.

## Verification

Deterministic checks include two addressed sources reopened across 1,000
intervening turns, pagination across 1,005 tied candidates without skipped or
repeated ids, candidate versus explicit-reference behavior, branch/aside
exclusion and restoration, request replay, output ordering and placement
replacement. These fixtures are subject-independent machinery checks, not
rules added for the installed weather or news samples.

The real answer-composition component fixture passes ten checks at 1440 × 900
and 390 × 844 in light and dark. It verifies source order across prose/card
boundaries, one copy of each prose fragment, one author/result, table survival,
viewport fit, exact pairing, form-width fit, stacking when minima do not fit,
Canvas sharing the row layout, and technical-view preservation. Node tests
check stale/missing anchors and distinct result boundaries. Both client bundles
were rebuilt. After these visual checks, Astra identified that visible-neighbour
pairing could rebind an anchor and live cards lacked matching placement metadata.
Those were fixed with accepted addressed placement, stable live/settled item ids,
and replacement handling in the real client store. The server regression feeds
actual CardRecorded events through the frame projector and compares their ids
and anchors to the settled snapshot, then removes an anchor with another card
preceding it. It passes, as do the Node anchor/hydration checks.

The added `runLivePlacementChecks` browser fixture covers pairing without an
outline and the actual client store's replacement path. It was **not executed**:
local browser use became disabled in Work settings before this final check.
Earlier responsive checks are evidence for the row geometry, not proof that this
new fixture passed. Final browser acceptance of that last refinement remains
open. Both bundles were rebuilt after the refinement.

The full workspace gate subsequently exposed a scheduled-page race: the detached
model-event forwarder could deliver a live card after the durable settlement
snapshot and overwrite its turn attribution. The run boundary now awaits that
forwarder before settlement, publishes a separate covered event sequence, and
rejects covered events in the projector and client. Same-id live updates also
cannot replace authoritative settled items. Tests cover a retired anchor's late
event, acceptance of the next turn's card, and forwarding drain order. The
scheduled-page integration test retains its card-content assertions and passes
in the focused rerun. Publication is shared by HTTP, gateway and scheduled
single-turn execution. Reconnect/gap baselines retain coverage, and lag replay
uses the same authoritative RunFinished rebase as ordinary streaming.

### Live model observations

A development CLI binary ran a fictional seven-turn exercise in a disposable
workspace with the existing configured OpenRouter route
`minimax/minimax-m3.1-flash:free`, read-only permissions, and ten model steps per
turn. No subject or model rule was added to the implementation for this test.
The first six completed turns:

| Request | Observed result |
|---|---|
| Store an exhibit budget and opening day | Acknowledged 420 credits and Thursday |
| Store a separate recital budget and seat count | Acknowledged 90 credits and 7 seats |
| Unrelated arithmetic, number only | Answered `414` without resuming earlier work |
| Revisit the earlier exhibit after the topic switch | Used recall and returned 420 credits and Thursday |
| Combine the two earlier projects | Used recall and returned 510 credits and 7 seats |
| Immediate implicit follow-up: split that total over three days | Answered 170 credits per day in one sentence |

The earlier-exhibit turn made one malformed open before recovering from the
argument error with valid references. The combined answer repeated its total
in prose and bullets despite the concise instruction. These are model-quality
limits observed in the sample, not reasons to add phrase-specific routing or
output rules. Each initial request had about 18,000 provider-reported prompt
tokens; repeated recall steps add cumulative usage. The change does not claim
the fixed prompt/tool inventory is small or that arbitrary semantic retrieval
is solved.

The seventh request, “Move it to Friday”, exposed an ambiguity failure: the
model made repeated searches, inferred an exhibit reschedule and a three-day
calendar without establishing the intended target, and reached the ten-step
limit after 183 seconds. A read-only `remember` attempt was denied. CLI exit
status zero did not mean this request completed: its settlement was max turns.
The revised general contract now separates record discovery from choosing user
intent, requires clarification when verified context leaves several plausible
targets or changes, and forbids inventing missing dates, values or scope. This
is a general revision rule, not a branch for that phrase or these subjects.
A fresh exercise used different fictional subjects: a workshop on Monday and
a gallery opening on Wednesday, followed by independent arithmetic and the
same ambiguous revision. The arithmetic returned 209. The revision still
searched excessively and emitted speculative alternatives and calendar dates
instead of clarifying (108 seconds, reported completed). This is a failed
semantic acceptance case even though the tools executed successfully.
Astra reviewed both failures and the multi-intent contract: no new classifier
or phrase rule can establish the user's missing choice. Its concrete prompt
finding was the conflicting “ask only when genuinely confused” operating rule;
that now says to ask when a missing choice materially changes an outcome and
continue independent parts. Ambiguity resolution remains open for cross-model
evaluation, not claimed fixed by this wording change.

Multi-intent coverage includes a resolver test retaining three distinct strands
for an earlier revision, earlier comparison and independent arithmetic, plus a
mixed-intent scripted agent test reopening two sources across 1,000 intervening
turns. Those prove machinery and isolation, not model semantic judgment.
The full workspace run also exposed three channel bridge failures. The reply
was sent before inbound delivery looked up outcome metadata from mutable live
presentation state, making its notice depend on scheduling. Replies now carry
metadata from their own settled turn. A separate delivery bug treated completed
answers with evidence state `none` as unverified, even when no evidence was
required. Status and text notices now share the same policy, with a five-channel
matrix retaining warnings for stale/unknown evidence and incomplete outcomes.
The bridge fixtures keep their original exact reply assertions. Astra reviewed
these corrections independently.

## Final verification

The complete workspace gate passed after the settlement and channel corrections:

- `cargo fmt --all --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace --no-fail-fast`: 3,892 passed, zero failed, 25 ignored
  across unit, integration and documentation test groups.
- Version consistency, documentation paths, record consistency and advisories.
- Both client builds, plus the Node composition and presentation-hydration checks.

The full run includes the original exact Discord, Slack and Telegram reply
assertions and the scheduled-page version regression. The five-channel delivery
matrix checks completed answers without retrieval and preserves stale, unknown,
partial and failed warnings. Browser and live-model limits above remain; these
workspace checks do not turn those semantic cases into successes.

## Limits retained explicitly

Session opening still materializes retained records; some other history
consumers still fold whole chains. The change removes whole-history projection
costing and provenance-card cloning from ordinary planning, and full TurnIndex
reconstruction before indexed recall, not every O(history) operation. The catalog remains lexical candidate
discovery; main-model reformulation/open/clarification resolves meaning. There
is no claim of perfect semantic retrieval or a new external embedding service.
Large individual turns still use the existing result windows and handoff
contract. Historical source authority, grants, branch, trash, erasure and
runtime permission gates are retained. No compatibility shim, migration, data
reset, release or installed-service replacement is part of this change.
