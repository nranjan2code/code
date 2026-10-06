# 85 — The turn graph: linked context across turns, sessions and years

Status: in progress. G0 is built, and G1 is built (2026-10-03): turn links, recall links, merged
threads and the planner's two-step `link` signal (§10). G2 and G3 are
proposed. §2.4 is the audit, taken before G0 started. Phase G1 changes the
in-session planner and adds no store. Phases G2 and G3 ride
data-architecture M6 (the data catalog, `docs/design/73-data-architecture-and-lifecycle.md`
§9) and start only when the maintainer says so. This document extends
`docs/design/68-context-engine.md` (the planner), `docs/design/47-commitment-kernel.md`
(strands and threads) and doc 73 §9 (catalog edges). Where it and AGENTS.md
disagree, AGENTS.md wins until the change is made to both.

## 1. The problem

Every turn's data is on the ledger: the directive, every tool call and its
full result, the cards, the intent reading with its strands and threads, the
commitment it served, and the exact prompt and tools it ran with. What is
missing is the connections. When turn 10 continues work from turn 3, or a
request two years from now touches a file an old conversation wrote, the
runtime has the facts to know it, and the part that decides what the model
sees does not use them.

The question this document answers: how does every turn get the context
that is *linked* to it, not just the context that shares its words, across
one session, across sessions, across years, and across every change to
code, prompt, model, tools, MCP servers, commands and plugins?

## 2. What exists today

Facts, read from the tree at `0ab1e4ffb`.

### 2.1 Four separate link mechanisms

1. **Intent lineage.** A request is split into strands
   (`crates/vak-intent/src/strand.rs`). Each strand is compared with the
   session's last 12 open threads (`open_threads`,
   `crates/vak-core/src/intent.rs`) by shared content words, or by a deictic
   word ("it", "that") with the same act, and gets
   `Lineage::Continues { thread_id }` or `New` (`lineage_for`,
   `crates/vak-intent/src/resolve.rs`). `Corrects` and `Replaces` come only
   from explicit commands. The `IntentRecord` stores each strand's
   `thread_id`.
2. **The context planner.** `plan()` (`crates/vak-context/src/planner.rs`)
   scores every closed turn of the current session as
   `max(recency, relevance, anaphora)`. Relevance is BM25 over the turn
   card's `index_text()`, which is the asked text, card titles and digests,
   narration, tool names and argument digests, the act and the domains.
   Anaphora gives 1.0 to the immediately preceding turn only. The winners
   ride `Full`, the rest `Card`, and the overflow collapses into a range-keyed
   `Packet`. The only thing it takes from intent is the `minimal` flag.
3. **Card provenance.** `PresentationRecord::derived_from` lists the evidence
   ids a card was built from, within its own turn
   (`crates/vak-session/src/types.rs`).
4. **Explicit retrieval.** `recall` reopens a turn, card or piece of evidence
   by id (doc 68 §3). `session_search` and the data catalog search
   other sessions by keyword. Both run only when the model calls them.

### 2.2 Who reads what

| Link | Recorded in | Read by |
|---|---|---|
| Strand `thread_id` | `IntentRecord.strands` | commitments, the steering-drift check (`intent_threads_by_turn`, `crates/vak-agent/src/lib.rs`) |
| Card text, argument digests | `TurnCard` | planner relevance (as words) |
| `derived_from` | `PresentationRecord` | `recall`, the card's own provenance |
| Files a tool touched | inside `TurnCard.did[].args_digest` | planner relevance, only if the directive repeats the path's words |
| Commitment per strand | `IntentRecord.strand_commitments` | commitments |
| Other sessions | session ledgers, `vak-store` | `session_search`, when the model calls it |

### 2.3 The gaps

- **G-a. The planner ignores lineage.** The resolver records that turn 10
  continues turn 3's thread. The planner rediscovers the link from shared
  words or misses it, so turn 3 stays a one-line card and the model must
  think to call `recall`.
- **G-b. Anaphora reaches one turn back.** "Do that again for the other
  file" after an unrelated interjection points at nothing.
- **G-c. Observed links aren't used.** A turn that reads a file an earlier
  turn wrote is linked by a fact the runtime saw happen. Nothing scores it.
- **G-d. Only one link per strand.** `Lineage::Continues` holds one
  `thread_id`, and `lineage_for` keeps the best match (`max_by_key`).
  `strand_commitments` maps a strand to one commitment. "Combine the parser
  work with last month's test cleanup" continues two threads and records
  one.
- **G-e. No merge or split.** Two threads that become one, or one that
  forks, have no record.
- **G-f. Threads end at the session.** `open_threads` reads one session's
  chain and keeps 12. Nothing links work across sessions, and nothing
  survives as a thread past a few dozen turns.
- **G-g. Long-horizon recall is keyword search on request.** A related
  conversation from six months ago is found only if the model calls
  `session_search` and the words happen to match.

### 2.4 Identity audit

An edge needs an id at both ends. Every turn, call, result, invocation and
record must have one, and every record must name the turn and call it
belongs to by id, never by position in the file. Audited at `0ab1e4ffb`:

| Thing | Own id | Names its turn | Names its call | Notes |
|---|---|---|---|---|
| Ledger entry | yes, UUIDv7 (`Entry::new`) | — | — | `parent_id` and `prev_hash` chain them |
| Session | yes, UUIDv7 string | — | — | typed `SessionId` exists, unused |
| Turn (context side) | the directive's entry id | — | — | used by `TurnIndex`, cards, presentations, `recall` |
| Turn (intent side) | a second UUIDv7, `turn_id` in `Core::run` | — | — | used by strand and thread ids, commitments, the checkpoint label; **never recorded beside the directive's entry id** |
| Tool call / evidence | `tool_use_id` | by position | — | unique within a session chain: `assign_unique_tool_use_ids` replaces a provider's reused id (`call_0`, `gemini-call-0`) with a fresh UUIDv7. **Not unique across sessions** |
| Evidence body | — | by position | yes | |
| Presentation | entry id | yes (`turn_id`) | yes (`ToolCall`/`Delegated` source, `derived_from`) | |
| Turn card | — | yes | yes (trace lines) | |
| Intent record | entry id | **no**, by position (written before the directive) | — | strand ids embed the intent-side turn id only |
| Work receipt | — | **no**, by position | — | records model and prefix digest |
| Capabilities bound / ref | entry id | **no**, by position | — | records prompt, schemas, epoch |
| Context selection | — | via `leaf_id` | — | |
| Activity | `activity_id` | **ordinal** (`turn: Option<usize>`), not id | **no** | ordinals shift with branches and resets |
| Approval | `approval-{request id}` | **no** (`turn: None`) | **no**: tool name and arguments only | |
| Hook run | span id (`ActivityRow` in the FinOps ledger) | **no** | **no** for pre/post-tool hooks | outside the session ledger |
| Plugin invocation | an Activity row's `plugin` (no separate log since 2026-10-04) | yes | yes | `activity-log` |
| Child run | child session `child-{uuid}-{seq}` | — | yes, `Cause::Delegation { tool_use_id }` | **empty** when the call has no sandbox sink |
| Commitment, episode | yes | **no**: an episode names only its session | no | |
| File effect | none; path only | no | yes, `execution_id` = call id | `ArtifactGenerated` in the `sandbox/executions/{session}` record chain, in the session's Agent home (D25 fixed in M3b slice 3); no content digest; reads are not recorded; typed `ArtifactId` unused |
| MCP call | the `mcp` call's `tool_use_id` | by position | — | which server, server version and tool schema answered is **not recorded** |
| Per-turn run (`RunId`, `TraceKey`) | yes, minted each run | **`TraceKey.turn` is never set** | spans per call | carried to side ledgers (commitments, misread, FinOps, hooks, outbox) but **not written to the session ledger**; the only join back is `request_id`, when the surface supplied one |
| Delivery / outbox | `job_id` | via trace, when present | no | typed `DeliveryId` unused |

What this means:

- **I-1. A turn has two ids that are never joined.** The intent side and
  the context side each mint one. Threads and commitments name one; cards,
  presentations and `recall` name the other. Today they line up only
  because the intent entry is written just before the directive.
- **I-2. The run id doesn't reach the session ledger.** Side ledgers carry
  the turn's `TraceKey`; the session ledger carries the session's first
  run in its header and nothing per turn. `TraceKey.turn` is never filled.
  A cost row, a misread row or a commitment event cannot be joined to its
  turn except through `request_id` or a timestamp.
- **I-3. Several records link by position.** Intent, receipts and
  capability bindings belong to a turn because of where they sit in the
  file. A graph extractor can recover that, but every reader re-derives it,
  and a branch or an insertion changes the answer.
- **I-4. Approvals, hooks and activities don't name their call.** An
  approval links to its tool call by tool name and arguments, which two
  identical calls share.
- **I-5. Episodes don't name their turn or strand.**
- **I-6. Files have no identity.** A file effect is a path in a side file,
  with no content digest and no read events, so "the file turn 3 wrote" and
  "the file turn 10 read" can't be shown to be the same version.
- **I-7. Evidence ids are unique only per session.** Fine for `recall`
  today; a cross-session edge must key evidence as `(session, tool_use_id)`
  or mint its own id.
- **I-8. MCP answers are unattributed.** Which server and tool schema
  served a call isn't on the ledger.
- **I-9. The typed ids are mostly unused.** `crates/vak-session/src/ids.rs`
  defines `TurnId`, `SessionId`, `ArtifactId`, `ConversationId` and more;
  most records still carry plain strings. Moving to them is data-architecture
  M3a/M3b work.

## 3. Principles

1. **Edges are facts or labelled inferences, never guesses dressed as
   facts.** Every edge carries its provenance: `observed` (the runtime saw
   it: a tool wrote this file, this card came from this evidence) or
   `inferred` (the resolver judged it: this strand continues that thread),
   with a confidence for inferred edges.
2. **Edges point at the facts layer, never at live configuration.** An edge
   names a ledger entry, an evidence id, an artifact id, a commitment id.
   It never names a registry entry, a loaded plugin or a connected server.
   §8 depends on this.
3. **The graph is derived.** The ledger is the record. The graph is rebuilt
   from it, never migrated, and a lost or wrong graph is rebuilt, not
   repaired (doc 73 §5, the Derived class).
4. **Deterministic first.** Building, walking and scoring the graph is plain
   code with no model call. A model is used only where §9 says, and never on
   the turn path for linking.
5. **Linking chooses context; it never grants anything.** The graph decides
   which recorded context is loaded. It does not grant a tool, widen a
   permission or skip a gate (invariants 16 and 32).
6. **Linked context is still logged context.** Anything the graph puts in
   front of a model is reconstructable from the session ledger (invariant
   1), and each request stays a function of the ledger and the bound
   model's profile (invariant 36). §6.4 says how this holds for
   cross-session context.

## 4. The model

### 4.1 Nodes

| Node | Identity | Source |
|---|---|---|
| Turn | turn id (the directive's entry id) | `TurnIndex` |
| Strand | `{turn_id}.{index}` | `IntentRecord.strands` |
| Thread | `thread_id` (the strand id that opened it) | strands |
| Evidence | `tool_use_id` | tool results |
| Card | presentation id | `PresentationRecord` |
| Artifact | artifact id, stable across renames and versions | G1: workspace-relative path; G2: the catalog's artifact and version nodes (doc 73 §10) |
| Commitment | commitment id | `vak-commit` ledger |
| Thread capsule | `(thread_id, last_turn_id, model)` | §6.2 |
| Session, conversation, Agent | existing ids | session header, `TraceKey` |

### 4.2 Edges

One table, one row per link. A row is:

```
from, kind, to, provenance (observed | inferred), confidence, derived_by
(extractor@version), trace key, recorded_at
```

| Kind | From → to | Provenance | Extracted from |
|---|---|---|---|
| `has_strand` | turn → strand | observed | `IntentRecord` |
| `continues` | strand → thread | inferred (or observed when from a command) | `Lineage` |
| `corrects`, `replaces` | strand → thread | observed (commands only) | `Lineage` |
| `merged_into`, `split_from` | thread → thread | observed (commands) or inferred | new, §5.3 |
| `produced` | turn → evidence | observed | tool results |
| `wrote`, `read` | evidence → artifact | observed | `write`/`edit`/`office_apply` calls; `read`/`doc_read`; bash only where the execution reports changed files (invariant 35's Workbench report) |
| `derived_into` | evidence → card | observed | `derived_from` |
| `shows` | turn → card | observed | presentation records |
| `recalled` | turn → turn, card or evidence | observed | `recall` calls |
| `serves` | strand → commitment | observed | `strand_commitments` |
| `summarises` | capsule → thread | observed | §6.2 |
| `renamed_to`, `retired_at` | tool, server, plugin or artifact → its successor or epoch | observed | the capability registry and catalog, when the change happens (§8) |
| `retracts` | edge → edge | observed | a later correction of an inferred edge |

`recalled` matters: when a model reopens an old turn on its own, that is the
strongest evidence that the two turns are linked, and it is already on the
ledger.

## 5. Relation shapes

### 5.1 Cardinality is rows, never fields

A turn with three strands is three `has_strand` rows. A thread fed by 400
turns over a year is 400 `continues` rows. Evidence feeding two cards, and a
card built from two pieces of evidence, is three `derived_into` rows; the
edge row is the join, so many-to-many needs nothing else. No node holds a
list of its neighbours.

### 5.2 Cross relations are found, not stored

Thread A and thread B both wrote `src/parser.rs`. No A↔B row is written: a
walk from A reaches B in two hops through the artifact. A stored A↔B row
would be a second copy of a fact the graph already holds, and it would go
stale the moment either side changed.

### 5.3 Many threads per strand, merges and splits

G-d and G-e need the intent record to hold more than one link:

- `lineage_for` returns every thread above the match threshold, not the
  single best; the tie rule (`New` beats a weak `Continues`) holds per
  thread.
- The record gains a list of continued threads beside the existing single
  `Lineage` (§11, D1 decides how).
- A strand continuing two threads emits `merged_into` for the older one,
  inferred. `/goal` commands may make it observed.
- A thread whose later strands keep splitting into disjoint artifacts and
  keywords emits `split_from`, inferred, by the extractor only (never the
  resolver on the turn path).
- `strand_commitments` becomes strand → set of commitments by the same rule.

### 5.4 Corrections

The ledger is append-only (invariant 2), and so is the edge history. A wrong
inferred edge is corrected by a `retracts` row from a later turn, an explicit
command, or a newer extractor version. A walk skips retracted edges. The
history of what the system believed, and when, stays readable.

## 6. Using the graph

### 6.1 In-session: a fourth planner signal (G1)

`plan()` gains `link`, computed from the current turn's anchors (§6.3) over
the session's edges, so a turn's value becomes
`max(recency, relevance, anaphora, link)`. Like relevance, `link` admits a
turn: a turn with no lexical hit and no link still gets only a card.

- **Path weight.** The product of edge weights along the path.
  `observed` edges weigh more than `inferred`, and an inferred edge weighs
  its confidence. Kinds are weighted in one table in `vak-context`, pinned
  by a test, like the tier-1 lexicon.
- **Converging paths add up.** A turn reached through the thread, the file
  and a card scores the capped sum of its paths, so cross relations
  strengthen a match rather than add noise.
- **Bounded walk.** At most 3 hops, a fan-out cap per node, a visited set
  (which also handles cycles), and every cost check still turn-whole.
- **Hubs are discounted.** An artifact or thread that most turns touch
  (`Cargo.toml`, a README) has its weight divided by its degree, the graph
  form of inverse document frequency.
- **Age never cuts a link.** Recency only orders turns that are already
  linked or relevant.
- **Anaphora reaches the last turn on the referenced thread,** not only the
  last turn (G-b): when the open strand continues a thread, "that" points at
  that thread's most recent turn.

The plan already records which turns it promoted beyond recency
(`WorkingSetPlan::retrieved`); it gains which paths promoted them, so a
transcript can show why a turn was in context.

G1 needs no new storage. Edges are extracted from the `TurnIndex`, the
intent records and the presentation records when the index is built, as the
cards are now.

### 6.2 Thread capsules

A thread with 200 turns cannot come back as 200 cards. When a thread goes
quiet (no strand continues it for a set number of turns or a set time), the
existing summariser writes one capsule for it, keyed by
`(thread_id, last_turn_id, model)` like a packet is keyed by its range. A
capsule is a cache, never a boundary: the walk returns the capsule as one
card, and `recall` reaches any turn inside it by id. Retrieval has three
levels: capsule → turn card → full turn.

### 6.3 Anchors

The walk starts from anchors, all taken from the current turn without a
model:

- the threads its strands continue;
- the artifacts its directive names, and the artifacts touched so far in
  the open turn;
- the commitment it serves;
- the turns BM25 already finds (keyword search becomes one anchor, not the
  only way in);
- the turns it has already `recall`ed.

### 6.4 Across sessions and years (G2, at M6)

The edge kinds of §4.2 become rows in the catalog's `edges` table (doc 73
§9), next to `produced_by`, `derived_from`, `version_of` and the rest.
Capsules become catalog nodes, and their text joins the catalog's FTS. A new
turn's anchors seed the same bounded walk over the catalog, across every
session the caller may read.

Cross-session context must still be reconstructable from this session's
ledger (principle 6). So what the walk selects is written into the session
first, as a new audit entry naming the selected capsules, turns and cards by
id with their content digests and the paths that selected them. The plan
then renders from that entry. Replaying the ledger reproduces the request
without the catalog. This is invariant 1's rule: new model-visible input
means a new entry type.

### 6.5 Feeding intent

Lineage can use the graph too. Touching the same artifact or evidence as an
open thread is a far stronger `continues` signal than one shared word. The
tier-1 resolver stays pure (no I/O, `crates/vak-intent`), so `Core` passes
the anchors' linked threads in as `ThreadFact`s, the way it passes
`open_threads` today, and `open_threads` widens from "last 12 in this
session" to "threads linked to this turn's anchors, plus the last 12".

## 7. Safety

- **Agent isolation (invariant 37).** The walk runs only over nodes the
  calling Agent and audience may read. In the catalog, the ACL filter runs
  before ranking (doc 73 §9). The graph never links across Agents.
- **Trash and erasure.** A trashed session's nodes and edges drop out of
  every walk (`vak_core::trash`, M0). Erasure (M7a) removes their derived
  rows and tombstones the edges that pointed in (doc 73 §7.3).
- **Untrusted content.** Capsules and recalled content are data from
  earlier turns, never instructions. They enter the prompt as the cards and
  turns already do.
- **No authority.** The graph changes context only. Permission, the
  broker, the sandbox and approvals are evaluated exactly as before.
- **No content in logs.** Edge rows carry ids, kinds and digests, never
  conversation text.

## 8. Surviving change

Three layers:

| Layer | What it holds | How it changes |
|---|---|---|
| Live | code version, prompt layers, model, tools, MCP servers, commands, plugins, renderers | at any time |
| Facts | the ledger | never rewritten; each turn records what it ran with |
| Derived | edges, capsules, FTS, embeddings | rebuilt by versioned extractors |

No edge points at the live layer (principle 2), so a change there can add
edges and never breaks one. Most of the facts layer already exists:
`TurnCapabilitiesBound` records each turn's exact system prompt, tool
schemas and capability epoch; `FrozenContract.prompt_layers` records layer
digests; `WorkReceipt` records the model used; every tool result is on the
ledger (invariant 1); a card records its skill version and schema version.

| Change | What happens to existing links | Needed |
|---|---|---|
| Tool renamed or removed | The edge keeps the name the tool had when it ran, with its canonical name (`canonical_tool_name`). The result is on the ledger, so `recall` works without the tool. | `renamed_to` and `retired_at` written by the capability registry when the change happens. |
| MCP server changed or removed | The edge names server, tool and a digest of the schema recorded that turn. A past call is evidence, never re-run. | The schema digest on `produced`. |
| Plugin or command removed | A card whose renderer is gone falls back to its exact Markdown; its edges point at the presentation record, not the renderer. | Nothing new. |
| Prompt changed | Past turns are not re-run or relabelled. The graph can say which prompt produced which turn. | Nothing new. |
| Model changed | Packets are range-keyed and hide nothing from a larger model (invariant 36). Capsules are keyed by model and rewritten lazily when the bound model would gain from it. | Capsule keying. |
| Resolver or extractor changed | Old inferred edges stay, labelled with the version that made them. A new version adds its own edges; a disagreement is a `retracts`, never an overwrite. | `derived_by` on every edge. |
| Code version within a major | Readers ignore unknown fields, and the format only grows (invariant 29). The graph is rebuilt, never migrated. | The catalog's staleness checksum (doc 73 §9). |
| A data baseline | Not survived today: the baseline refuses and purges every earlier data home (invariant 29, plan L3). | Decision D2. |

## 9. Where models are used

| Part | How |
|---|---|
| Edge extraction, anchors, the walk, scoring | Deterministic code. No model. |
| Thread capsules | The existing summariser, once per quiet thread, cached, off the turn path |
| Linking with no shared artifact, evidence or word | Optional: an embedding index behind the catalog's search interface (doc 73 §9 allows one). A small local embedding model, not an LLM. G3. |
| Edge weights | Optional and offline: learned from what the model actually used. A `recalled` edge to a turn the plan left as a card is a measured miss, the same epistemics as `crates/vak-core/src/misread.rs`. G3. |
| Ambiguous lineage | The existing intent tiers 2 and 3, unchanged. |

## 10. Phases

### G0 — Identity first

Built (2026-10-03):

- One turn id: `Core::run` mints it and calls `SessionLog::begin_turn`,
  and the next directive is written with that id as its entry id. A
  reserved id that already names an entry is refused.
- `Entry::at_turn`, stamped by `SessionLog::append` on every entry of a
  turn, including the intent written before its directive. It replaces two
  joins by position: `TurnIndex` attaching a reading to its turn, and the
  drift check's thread lookup.
- `TraceKey::in_turn`: the run's key names its session and turn, so every
  side-ledger row written under it does.
- Tests: `one_turn_one_id`, `every_turn_record_names_its_turn`,
  `a_reserved_turn_id_already_used_is_refused`,
  `a_reopened_ledger_keeps_its_current_turn` (vak-session) and
  `one_turn_one_id_from_intent_to_side_ledgers` (vak-core).

Also built (2026-10-03):

- Approvals name their call: `Approver::approve` takes the call id, and the
  pending and resolved approval activities record it as `tool_use_id`
  (`resolved_approval_activity_names_the_verified_decision_maker`).
- Pre/post-tool hook runs and tool activity rows record their call's
  `tool_use_id` (`finops::ActivityRow`). Session-start and stop hooks have
  no call.
- Episodes record their strand (`EpisodeStarted.strand_id`,
  `Episode::strand_id`); their turn comes from the event's trace key
  (`an_episode_names_its_strand`).
- File effects: a tool declares `Tool::file_access` (read, write, edit and
  `doc_read` do), and after a successful call the agent loop writes a
  `CallEffect` entry with the path, a SHA-256 of the content and its size.
  A path that resolves outside the workspace is recorded without a digest
  (`file_effect_has_digest`, `a_path_outside_the_workspace_is_not_measured`).
- MCP attribution: the client keeps the server's `serverInfo`, and every
  answered `mcp` call writes a `CallEffect::Mcp` with server, reported name
  and version, tool and schema digest (`mcp_result_names_its_server`).

Finished (2026-10-03):

- Gateway-forwarded approvals name their call in the inbox
  (`inbox::Entry::tool_use_id`, `record_for_call`,
  `a_forwarded_approval_names_its_call`).
- `ActivityRecord.turn` is gone. The timeline projection keys outcome
  evaluations, reviews, run results and admission outcomes by
  `Entry::at_turn`, which also replaced its "the next turn is `n + 1`"
  guess for admission outcomes. An outcome review takes the reviewed turn's
  id (the client sends the id it was given) and is written into that turn
  (`SessionLog::append_in_turn`).
- More file effects: a file bash's before/after scan reports is recorded as
  an observed write by that call (D4, decided); `office_apply` declares the
  document it edits from (`source`, or `path` with a `base_digest`); a file
  a Review promotes is recorded as a write by the call that drafted it, in
  that call's turn, while the session's ledger is idle in this process.
- End to end: `a_turns_file_calls_record_their_effects` runs a scripted
  turn through the real write, read and bash tools and checks the three
  effects, their digests and their turn.

G0 is complete.

Live check of the finished G0, 2026-10-03, same setup as the G1 checks:
turn 1 created `logs/status.txt` with bash (three commands, each writing
it), turn 2 asked "What is 17 times 23?", and turn 3 asked what
`status.txt` contains. Read through the app:

- each of turn 1's bash commands recorded a `file_write` of
  `logs/status.txt` under its own call, with the same digest;
- turn 3's plan put turn 1 at `Full` through `file:logs/status.txt`, a
  node only bash's scan had recorded, and the model answered "ready".
  Turn 3 read the file with `cat`, which records nothing: bash reads are
  not observable by a before/after scan;
- the timeline still groups turns, and `POST …/outcome-review` with turn
  3's id was accepted, a made-up id answered 404, and one with no id took
  the latest evaluated turn; the verdict showed on turn 3's items only.

Not exercised live: gateway-forwarded approvals (a dev run has no
gateway) and Review promotion (needs an Office draft); unit tests cover
both.

Finding: turn 3's plan also put turn 2 (the arithmetic question) at `Full`,
through a thread link: the resolver read turn 3 as continuing turn 2's
thread. Lineage that only cost a duplicate thread before G1 now costs
context. Fixed the same day: the link came from the one keyword the two
requests shared, "what". Question words, auxiliaries, pronouns and
quantifiers are no longer keywords (`strand::keywords`), so they cannot
continue a thread (`a_shared_question_word_does_not_continue_a_thread`,
`question_words_are_not_keywords`); `RESOLVER_VERSION` is 10. Live
re-check with the same three turns: turn 3 opened a thread of its own, and
its plan put only turn 1 at `Full`, through `file:logs/status.txt`
(288 tokens of history); the model answered "ready".

Nothing in G1 to G3 is sound until §2.4 holds. Each item is a new field or
entry, added under invariant 29's additive rule, or folded into M3a/M3b
where the typed ids land anyway (§11, D5).

- **One turn id.** The intent side stops minting its own: `Core::run` takes
  the directive's entry id, or the directive records the intent-side id.
  One of the two goes (invariant 30).
- **The turn's trace on the ledger.** The run's `RunId` and `TraceKey`,
  with `turn` filled, are written to the session ledger at admission, and
  every side-ledger row under it then names its turn.
- **Turn ids on turn records.** Intent records, work receipts and
  capability bindings name their turn by id. Activities name a turn id in
  place of an ordinal.
- **Call ids on call records.** Approvals, pre/post-tool hook runs and
  activities about a call name its `tool_use_id`. A delegated child always
  carries its parent's call id, with or without a sandbox sink.
- **Episodes name their turn and strand.**
- **File effects as records.** A write, edit, `office_apply` or reported
  bash change records the call id, the path and the content digest; a read
  records the same. Until the catalog's `ArtifactId` (M6), path plus digest
  is the identity.
- **MCP attribution.** An `mcp` call's result records server, the server's
  reported name and version, the tool and a digest of the tool schema used.

Exit tests:

- `one_turn_one_id`: every strand id, thread id, commitment episode, card
  and presentation of a turn names the same turn id.
- `every_turn_record_names_its_turn`: walking a ledger, every intent,
  receipt, capability, activity and presentation entry resolves to a turn
  by id, without using position.
- `side_ledger_rows_join_to_their_turn`: a cost row, a misread row and a
  commitment event written during a turn join to it through `TraceKey.turn`.
- `approval_names_its_call`: two identical calls in one turn each get an
  approval naming its own `tool_use_id`.
- `file_effect_has_digest`: a write then a read of one file record the same
  digest; a later edit records a new one.
- `mcp_result_names_its_server`.

### G1 — In-session links (no storage change)

Built (2026-10-03):

- `Turn::links` (`TurnLink`, `LinkKind`): each turn's threads (own or
  continued), files read or written (from `CallEffect` entries) and
  commitments, collected by `Entry::at_turn`.
- The planner's fourth signal, `link` (`link_values` in
  `crates/vak-context/src/planner.rs`): for every node the open turn shares
  with a closed turn, both ends' link weights times an inverse-frequency
  discount, summed and capped at 1.0. At most `MAX_TURNS_PER_NODE` (8)
  turns, the most recent, are reached through one node. The walk is one
  shared node deep in this first cut.
- Anaphora points at the last turn of the thread the open turn continues.
- `WorkingSetPlan::links` records the nodes behind each linked `Full` turn,
  and the audit activity carries them; the plan policy version is 3.
- Tests: `linked_turn_promoted_without_shared_words`,
  `hub_artifact_does_not_link_everything`, `anaphora_follows_the_thread`,
  `link_weights_are_pinned`.

Also built (2026-10-03), finishing G1:

- Recall and evidence links: a `recall` call that reopens a turn by number,
  turn id, presentation id or evidence id links the two turns through the
  reopened turn's `turn:` node (`LinkKind::RecalledTurn`/`ThisTurn`,
  `a_recall_links_the_two_turns`).
- A two-step walk: a turn that shares a node with a turn that shares one
  with the open turn is reached at the second step, worth at most
  `STEP_DECAY` (0.5) of the first; only the best `MAX_TURNS_PER_NODE` turns
  of a step go on (`a_second_step_reaches_through_a_shared_turn`).
- Many threads per strand (D1, decided: no compatibility is needed):
  `Lineage::Continues` carries `merges`, the other open threads a clause
  shares at least two content words with, and the turn links to each
  (`strand_continues_two_threads`). `RESOLVER_VERSION` is 9.

G1 is complete. Splits (`split_from`) and `retracts` stay with G2, where
edges are stored rather than derived per request.

`GET /sessions/{id}/context` shows each turn's links, the effects its
calls recorded and every recorded context plan with its link paths, so why
a turn was in context can be checked through the app.

Live check, 2026-10-03, against the configured hosted model on a dev
server (port 8931, a disposable `/tmp` workspace): turn 1 wrote
`notes/plan.md`, turns 2 to 4 were unrelated one-liners, and turn 5 said
"Do that again, but make it two lines." Read through the endpoint:

- turn 5's strand continued turn 1's thread, and its plan put turn 1 at
  `Full` through that thread link (`retrieved` and `links` both name it),
  with turns 2 to 4 as cards. Before G1, anaphora would have pointed at
  turn 4. The model edited `notes/plan.md`, as asked;
- turn 1's write and turn 5's read recorded the same SHA-256, and turn 5's
  write a new one;
- every record carried its turn. Plan cost: 255 of a 104,735-token budget.

What it showed is missing: a turn's plan is frozen before its first tool
call (invariant 36), so the files the open turn reads cannot link it to
anything for that plan; only threads and commitments are anchors at plan
time. The anchor §6.3 lists for this, files the directive names, was built
next (below).

Directive-named files (built 2026-10-03): the planner reads the file names
a directive mentions (`named_files`: tokens shaped like a path or a name
with an extension; not URLs, e-mail addresses or numbers) and anchors the
walk at every recorded `file:` node whose path equals the name or ends with
it as its last segment, as `LinkKind::NamedFile` (weight 0.8). This works
at plan time, before the open turn has run a call
(`a_named_file_links_at_plan_time`,
`named_files_are_paths_not_urls_or_numbers`).

Live check, 2026-10-03, same setup as above: turn 1 wrote
`recipes/tomato-bisque.md` (the model chose the name), turns 2 and 3 were
unrelated, and turn 4 asked to "append exactly one short garnish line near
the end of tomato-bisque.md". The resolver gave turn 4 a thread of its own,
so no thread linked it; its plan put turn 1 at `Full` with
`links: file:recipes/tomato-bisque.md`, turns 2 and 3 as cards, at 716 of
a 104,531-token budget. Turn 4 read the file with the digest turn 1 wrote
and wrote a new one. The plan records the link, not whether word search
would also have found turn 1; the path is in turn 1's card too.

- Edge extraction over `TurnIndex`, intent records and presentation records,
  in `vak-session` beside the turn index.
- The `link` signal, path weights, hub discount and bounded walk in
  `vak-context::planner`; `retrieved` records its paths.
- Anaphora over the referenced thread.
- Multiple continued threads per strand, if D1 allows it within the major.

Exit tests (the context-engine gate in `vak-eval` and planner unit tests):

- `linked_turn_promoted_without_shared_words`: turn 3 edits `parser.rs`,
  six unrelated turns follow, turn 10 asks "fix the tests" and reads
  `parser.rs`. Turn 3 rides `Full`.
- `unrelated_keyword_turn_stays_card`: a turn sharing only a common word
  with the directive and no edge stays `Card`.
- `hub_artifact_does_not_link_everything`: twenty turns touching
  `Cargo.toml` do not all rise.
- `anaphora_follows_the_thread`: "do that again" after an unrelated
  interjection promotes the thread's last turn.
- `strand_continues_two_threads`: a merge request records both threads.
- `link_walk_is_bounded`: a dense synthetic graph never exceeds the hop and
  fan-out caps or the planner's budget.
- `plan_records_link_paths`: the recorded plan names the path for every
  turn promoted by `link`.

### G2 — Across sessions (with M6)

- The edge kinds join the catalog's `edges`; capsules join `nodes` and FTS.
- The cross-session walk and the new audit entry of §6.4.
- `open_threads` widened by linked threads (§6.5).
- `renamed_to` and `retired_at` from the capability registry.

Exit tests:

- `old_thread_found_by_artifact`: a session 18 months back (by fixture
  timestamps) that wrote `src/parser.rs` surfaces as a capsule when a new
  session's turn reads it.
- `cross_session_context_replays_from_ledger`: with the catalog deleted,
  replaying the session reproduces the request byte for byte.
- `walk_never_crosses_agents`: another Agent's linked thread is never
  returned.
- `trashed_session_absent_from_walk`.
- `removed_tool_edges_still_resolve`: after a tool and an MCP server are
  removed, every edge to their past results still resolves through
  `recall`.
- `extractor_change_adds_never_overwrites`: a new extractor version leaves
  the old version's edges readable.

### G3 — Learning and embeddings (optional)

Offline edge-weight learning from measured misses, and an embedding index
behind the catalog search. Each needs its own eval showing it beats G2 on
pass rate without raising token use.

## 11. Open decisions

- **D1. Many threads per strand within the 6.x line.** Decided
  2026-10-03: there are no users, so `Lineage::Continues` gained `merges`
  directly (G1). Invariant 29 allows
  adding fields, not changing them. Either add a `continues: Vec<String>`
  field beside `Lineage` now, or wait for the data baseline and change the
  type there. Recommendation: wait for the baseline, and let G1 ship
  everything except G-d and G-e first; a parallel field is two ways of
  saying the same thing (invariant 30).
- **D2. History across future baselines.** Plan L3 cuts the data baseline
  at 7.0.0 with no migration, which is right with no users. Long-horizon
  linking only means something if the facts layer then crosses every later
  cut. Recommendation: lock, at M3b, that 7.0.0 is the last baseline cut
  without a path, and that any later cut ships an export-and-reingest of
  the facts layer (`vak data export`); the derived layer rebuilds itself.
- **D3. When a thread is quiet.** By turn count, by time, or both, and
  whether a capsule is rewritten as a thread resumes or appended to.
  Recommendation: both, and append (a resumed thread gets a newer capsule
  keyed by its new last turn; the old one stays a valid cache).
- **D4. Bash file effects.** Decided 2026-10-03: observed; they come from
  the runtime's own before/after scan. `wrote`/`read` from bash needs the changed-file
  report invariant 35 already requires of Workbench. Confirm it is
  complete enough to be an `observed` edge, or mark bash edges `inferred`.

- **D5. Where G0 lands.** The data architecture already plans typed ids
  (M3a/M3b) and a trace key with its actor on every record (M1, done for
  side ledgers). Recommendation: the single turn id, the turn's trace on the
  session ledger and turn ids on turn records ship now as additive entries
  (they are M1's rule applied to the session ledger, which M1 left out);
  the switch to typed ids and the file identity move with M3b and M6.

## 12. Not in scope

- A graph database. The edges table is SQLite locally and Postgres in the
  cloud, as doc 73 §9 says.
- Linking across Agents, tenants or audiences.
- A model on the turn path for linking.
- Re-running, relabelling or rewriting past turns after any change.
