# 101 — Topics: Agent-owned organisation for people and the Agent

Status: implemented as an initial conversation-scoped view. Sealed Topic records,
main-model tools, human controls, Agent navigation, CLI, Admin and catalog
integration and shared Doctor diagnostics are built. Focused checks, responsive component acceptance and a
local OpenAI Luna run are recorded in
`docs/audits/topics-acceptance-2026-10-21.md`. Background semantic enrichment,
cross-conversation Topic merging, busy focused queuing and managed-run focus
remain outside this cut.

## Product contract

A person writes what they need. The main model organises ongoing work into
**Topics**, using its understanding of the request and addressed existing work.
There is no fixed subject taxonomy, keyword classifier or classification model
dispatch. Topic titles and outcome labels come from the person or model. Simple
independent answers need no organisation call. Automatic organisation starts on;
a person can disable it for a conversation.

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
