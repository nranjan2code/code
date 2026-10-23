# 101 — Topics: Agent-owned organisation for people and the Agent

Status: implemented. The conversation-scoped Topic view, main-model capability,
human controls, CLI, Admin and catalog integration are built. Post-answer turn
naming and Topic placement are qualified in the development build; see
`docs/audits/turn-names-and-topics-acceptance-2026-10-23.md`. The removed
answer-completion placement gate stays removed. Cross-conversation Topic merging, busy focused queuing and
managed-run focus remain outside this cut.

## Current runtime boundary, 2026-10-23

An answered top-level turn queues an independent names-and-Topics request to
the same configured main model. It uses the existing Trigger/claim/run services,
request admission, capacity estimate and spend gate. It does not delay the
answer, force an answer tool call, change progress or revive retired plans.
The main-model `topics` capability remains available for explicit organisation.

The input names its canonical turn and the answer's recorded ContextSelection.
Only that selection's historical exchanges and exact stored summary accompany
the current request, answer and presentation records. Existing file/turn links
and recall call addresses describe provenance; Topics and names never choose
historical admission, certify support or grant authority. A follow-up such as
“improve that” therefore retains the already-admitted subject without a second
history-routing algorithm. Addressed backfill uses that earlier selection and
never includes later dialogue. The normal branch, reset and set-aside visibility
rule applies. Tool bodies are not copied into the metadata request.

The exact auxiliary ChatRequest is a conversation object named by a new
OrganisationRequested ledger entry before dispatch, and is reconstructable
through SessionLog::organisation_requests. Contributor scopes seal the complete
input in layers so erasing any contributor makes the input unreadable. It never
enters a later answer's derive_messages projection. The same sealed Topic
Document holds authored names, pending Trigger/request references and raw
response/receipt references. Status is queried from existing Runs, not written
onto Triggers. Catalog names inherit the source conversation's audience and
trash boundary.

A structured result atomically names the turn and places its outcomes into
active addressed Topics, new model-named Topics or explicitly no Topic.
Substantive work belongs in a Topic from its first request regardless of answer
length; simple standalone answers may remain explicitly ungrouped. No
keyword/domain classifier or fixed taxonomy is involved. Human names and
placements win over late results; automatic-off disables model grouping while
names can still arrive. Archived Topics cannot receive placements. A retry
replaces the pending Trigger identity, fencing older results. Failed input,
route changes and invalid model output leave the answer intact and expose an
explicit retry; there is no bespoke queue or retry loop.

The client polls metadata while pending and refreshes independently of the
unchanged answer snapshot. A workspace change cancels the old fetch identity,
clears its loading state and reloads an expanded Topic list. Recorded names take precedence over answer-heading
fallbacks. Topics offers inline name corrections and a request to name/group
currently shown turns, rather than silently charging to backfill all history.
No release or local installation is implied by development acceptance.

> **Historical record** — the placement-enforcement description and its
> acceptance claims below describe the earlier implementation, superseded by
> the 2026-10-22 card redesign. Storage, tools and human controls remain built.

## Product contract

A person writes what they need. The main model organises ongoing work into
**Topics**, using its understanding of the request and addressed existing work.
There is no fixed subject taxonomy, keyword classifier or classification model
dispatch. Topic titles and outcome labels come from the person or model. Simple
independent answers may remain ungrouped through an explicit model placement
with an empty Topic list. Before accepting an ordinary or card-only answer while
automatic organisation is on, the runtime checks the current sealed Topic
projection for a placement decision for this turn. Listing Topics, creating an
empty Topic or placing an older turn does not satisfy the check. Missing
placement follows the shared correction ladder: tell once, require a Topics
call on the next failure, then let the model's answer stand with the gap recorded.
A failed or refused Topics call, unavailable capability, automatic-off, or human
placement never holds the answer open. The operator's last step is preserved.
The main model still chooses whether work needs grouping and authors every
Topic title and outcome label; there is no request-word classifier or separate
model dispatch. This checks a recorded decision, not its semantic correctness,
and does not backfill earlier answers. Automatic organisation starts on;
a person can disable it for a conversation. The system prompt asks the main
model to list and reuse or create a fitting active Topic before finishing
ongoing, follow-up or multi-outcome work, using plan deliverable names where
appropriate; a refused organisation call never blocks the requested answer.
The sidebar refreshes on actual Topic changes instead of retriggering on its
own loading state. Turn navigation uses addressed plan names and marks a paged
count as loaded, preserving the leading turn slot in focused Topic views.

An Agent owns its Topics, which are views over existing conversation turns.
The conversation remains the canonical ledger and security/lifecycle boundary
([Agent model](64-agent-owned-platform.md)). The initial implementation gives
each Topic one source conversation. Agent navigation collects these views
without merging audiences or transport histories. A compound request may have
several outcomes, and each outcome may belong to several Topics. No messages
are copied, split or rewritten to organise them.

A Topic is not a worker, a goal, an execution thread or an approval. Grouping
does not admit historical context: the main model still queries and opens
evidence under the [context contract](68-context-engine.md). Consulting a source
does not certify support. Recorded provenance and semantic organisation are
distinct; neither is a universal semantic-resolution guarantee.

## First-class access

The broker-owned `topics` tool lets the model page/search names, open membership
addresses, create a named Topic and associate the current turn's outcomes.
An `assign` call with an outcome label and an empty `topic_ids` list records
an explicit ungrouped decision without creating an empty Topic.
Creation can include the requested outcomes in one atomic operation, avoiding
a dependent create/assign model step. List and open return compact metadata and
paged addresses, with recent membership first; no age cutoff hides older work.
Exact turn addresses can then be opened with `recall`. Model arguments/results
use ordinary ledgered tool exchanges, preserving `derive_messages()` replay.
No worker receives a session handle or Topic store, and delegated work does not
inherit the parent's organisation capability.

People browse Topics under an Agent, open the source conversation at a stable
Topic address, inspect membership and return to the complete conversation.
The composer names a selected destination; a selected Topic is explicit human
placement, even when automatic placement is off. While a run is active, the
focused composer retains the draft and waits;
the existing steering queue cannot preserve independent destination metadata.
A focused request never steers an unrelated active run. Titles and ordinary
labels are shown in everyday screens; ids and trace details belong in
administrative detail.

People can create, rename, pin, correct membership, archive and reopen. Settings
for automatic organisation live alongside those controls. Admin inspects the
same records and mutation provenance rather than keeping a second index of truth.

The CLI uses the same Topic records and grants-aware catalog discovery:

```sh
vak topics --agent vak list --limit 20
vak topics open <topic-id> --limit 20
vak topics --session <conversation-id> create "A name chosen for this work"
vak topics --session <conversation-id> archive <topic-id>
vak topics --session <conversation-id> reopen <topic-id>
vak topics --session <conversation-id> automatic false
vak topics --session <conversation-id> assign --turn <turn-id> --outcome "Requested result" --topic <topic-id>
```

Assignment accepts repeated `--topic` arguments; omitting them leaves the
outcome unassigned. `--whole-turn` applies an explicit human destination to the
whole request. `--revision` refuses a change based on a stale observed version.
These administrative commands need no model invocation. Agent organisation uses
the shared Core loop in CLI, terminal, server and channel turns; channel-specific
human menus and native terminal Topic navigation have not been live-verified.

## State and concurrency

`vak_session::work_threads::TopicStore` keeps one sealed Document per source
conversation through `AgentScope::work_threads`. Immutable Document versions
retain previous states; each mutation names its actor, trace and time. Contents
are additionally sealed under the source conversation key so conversation
erasure makes old organisation snapshots unreadable. Storage follows existing
Document CAS and writer fencing, with no new lock or queue.

Topic ids are full UUIDv7 values; turn addresses retain their canonical ledger
ids. Human writes name the revision they observed. The CAS closure refuses
stale writes, including races discovered on retry. A human correction controls
that named outcome; an explicit whole-turn destination controls the turn. A model
cannot bypass a whole-turn choice by changing an outcome label, or enable an
owner-disabled automatic-placement preference. Outcome labels are authored
names, not a proof that two differently named outcomes mean the same thing;
whole-request controls provide the stronger turn-wide correction boundary.

Readers fail on undecodable records. Reads and discovery honour the source
conversation's Agent, audience, trash and erasure state. Topic catalog entries
inherit those boundaries. Sharing an Agent or a Topic name grants no source
conversation access. Invited readers use the authenticated conversation routes;
owner-only organisation controls remain outside the participant mutation allowlist.

## Archive and deletion

Archive removes a Topic from the active list and prevents new placement into it.
Only an explicit human control reopens it. Archived content remains inspectable
through authorised history. Archive affects organisation only: it neither stops
running work nor disables schedules, and the interface must say so.

There is no Topic-specific deletion of ledger turns, key destruction or retention
clock. Conversation trash, retention, holds and erasure remain governed by the
[existing lifecycle](74-lifecycle-and-data-administration.md). Removing a
membership is an organisation correction, not deletion of its source.

## Doctor

The shared Doctor report includes a **Topics data** check for CLI, terminal,
web and Admin. It inspects catalog-known source conversations using canonical
headers and sealed Topic metadata, rather than materialising their history.
An owner-wide check covers known Agents; a scoped check keeps its Agent and
audience boundary. Trashed and erased sources remain excluded.

The check reports unreadable records, missing addressed turns, inconsistent
catalog rows and pending indexing separately. Automatic organisation being off
and archived Topics with existing memberships are valid states. New or
unindexed conversations are explicitly unobserved, so a clean indexed sample
does not establish whole-install coverage.

Inspection opens the catalog read-only and does not advance source cursors,
repair metadata, change preferences or delete anything. Findings name existing
verification or catalog-rebuild commands where appropriate; repairs stay
explicit operator actions. Ordinary output contains counts and status rather
than Topic titles, outcomes or conversation text.

## Acceptance and limits

`crates/vak-agent/tests/it/topics_completion.rs` covers missed placement,
list-only and older-turn receipts, card-only answers without duplicate cards,
explicit ungrouped decisions, human precedence, automatic-off, failed calls,
bounded noncompliance and preservation at step/outcome/model-call limits.
The default-on Core integration is exercised by
`automatic_organisation_is_checked_before_core_accepts_an_answer`; unrelated
finite provider fixtures explicitly disable automatic Topics through its real
preference control. These deterministic checks establish the runtime boundary; they do not certify
main-model semantic Topic choices or installed native-host behaviour.

Verify multi-outcome/multi-topic placement, human precedence, automatic-off,
archive/reopen, stale writes, corrupt-state refusal, replay, admitted-run identity,
and source-session isolation. HTTP acceptance uses bearer and live audience
middleware, including wrong-audience and revoked-participant cases. Visual
acceptance uses the actual shared components at 1440 × 900 and 390 × 844, light
and dark. A typecheck or mockup does not establish visual acceptance.

The initial sealed snapshot grows with its source conversation; it is not a
lifetime-scale incremental graph store. Names and addressed memberships can be
indexed using the existing catalog upkeep, but no independent semantic background
worker is introduced. The kept-workers proposal remains deferred. Cross-session
Topic identity, suggested regrouping, model-generated Topic summaries, related-work
edge enrichment, and natural-language lifecycle approvals require further design.
Explicit Topic focus for managed/automatic/goal runs is initially refused rather
than silently dropping their execution contract.
