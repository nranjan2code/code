# Conversation history growth review

Status: historical — source review and synthetic client measurements on
2026-10-21. Findings remain open; no runtime behavior changed.

## Verdict

Fix this before launch. The main chat already windows mounted turns, but
its data path still loads and repeatedly processes whole conversations.
Keeping recent turns and fetching older pages on scroll is the right
direction. It needs a bounded server read, bounded client state, and a live
stream that does not send all settled history at each boundary.

This is not a claim that every browser paint rebuilds the entire DOM.
Solid's reactive updates and the existing turn window avoid that. The
confirmed problem is whole-history work around fetching, projection,
hydration, stream updates and discovery.

## Findings

### H1 — Whole conversation refreshes even when nothing changed

`crates/vak-client-ui/src/App.tsx`, `refreshSessions`, refreshes every visible
settled conversation through `hydrate` on the ten-second heartbeat. `hydrate`
fetches `/sessions/{id}/snapshot` plus sandbox executions, then stringifies
the entire response to detect duplicates. Its fingerprint avoids applying an
unchanged view, but only after the server projection, transfer, JSON parsing
and client serialization have happened.

`crates/vak-server/src/lib.rs`, `conversation_snapshot`, returns complete
`transcript_json` and `live_presentation_snapshot` outputs together. There
is no page parameter or conditional revision check. The atomic pairing is
valuable and must survive pagination. Idle cost currently grows with the
conversation even when the reader does nothing.

### H2 — Stream boundaries resend history and live updates scan it

`crates/vak-server/src/stream.rs`, `presentation_frames`, initializes a
private complete timeline per subscriber and sends a full initial snapshot.
At `RunFinished` it clones the durable presentation and sends it through
`projection::settled_frame`; lag recovery also sends a complete snapshot.
`http_settle` rebuilds the durable projection over the session before
publishing completion. Opening a chat hydrates its HTTP snapshot before
opening the stream, which also supplies its presentation baseline.

Ordinary text events already send deltas, not complete snapshots. However,
`presentationHydration.ts::applyPresentationDelta` maps the whole items array
on every text update, followed by Solid reconciliation in `store.ts`.
Server `projection::apply_stream_event` similarly finds the target by a
linear search of the subscriber's timeline. Across equal-sized turns,
repeated full settlement snapshots have quadratic cumulative transfer as
the conversation grows. A delta wire alone does not make processing bounded.

### H3 — Render window is useful, but does not bound loaded data

`ChatPane.tsx` opens roughly the last 40 turns and shifts the mounted range
in batches while scrolling, normally keeping at most 120 turns mounted.
Its `turns` memo still groups every loaded item. `projectedTurn` finds a
user's server turn and filters the full presentation array for each rendered
turn. The window is a DOM bound, not a network or heap bound; an individual
turn can itself contain substantial content.

`store.ts` retains transcript and presentation arrays by visited session.
`pruneStreams` unsubscribes invisible sessions but does not evict their view
data. Server handles have an existing count-based idle eviction policy
(`MAX_LIVE_SESSIONS`); that does not bound bytes in an individual session or
the client's caches. Do not describe either process as having no eviction.

### H4 — Taking a slice after projection will leave cold reads unbounded

`projection.rs::snapshot_inner` walks `session.chain_to_root()` and builds
whole-session maps for presentations, tool results and other facts.
`SessionLog::open_read_only` parses all segments into `entries` and `by_id`.
Returning only the last page after those operations improves the response
size, but still reopens and projects all history on a cold read.

Reuse the existing catalog entry locations and branch ancestry in
`crates/vak-catalog/src/history.rs`, and the supported session readers.
`TurnIndex::from_log` is itself a full traversal, not a persistent pagination
index. Catalog support is a starting primitive, not an already finished UI
history reader. Segment materialization also needs measurement: a located
record is not proof of bounded physical I/O or decode work.

### H5 — Growing chat count has its own discovery cost

`lib.rs::list_sessions` accepts only a trash flag, walks workspace and Agent
session directories, deduplicates paths with repeated linear searches,
summarizes every candidate and sorts the complete result. It is called by
the same heartbeat. `summarize_jsonl` stops its callback after finding a title
and passing 400 entries, but `SessionLog::scan` materializes the current
segment's located records before callbacks. It is not necessarily a complete
history replay, nor a constant-cost summary lookup.

Move discovery to catalog-backed, audience-scoped summary pages and targeted
status updates. This also resolves the mismatch with the repository's
catalog-first discovery contract. Paging one open chat alone will not fix
growth in the number of chats.

### H6 — Secondary readers can bypass the main window

`SharedConversation.tsx` fetches full transcript and presentation responses
on a ten-second refresh. Its message slice grows on “Show earlier messages”;
shared results are rendered separately without that slice.
`TranscriptModal.tsx` fetches the full transcript and mounts every converted
item. The main chat's window therefore does not protect all human readers.
Explicit full exports should remain complete; everyday reading should share
the bounded history contract.

## Recommended contract

1. Return a recent page containing matching transcript identities and
   presentation items from one committed revision, plus an opaque older-page
   cursor and a live-stream watermark. Page by real turn identity, not array
   offset, timestamp alone or guessed message counts. Bind cursors to session,
   branch/leaf and projection revision; recheck current audience and erasure
   on every request. A cursor never grants access.
2. Read older pages near the top of the viewport, with one request per
   boundary, cancellation on navigation, stable-id deduplication and explicit
   retry. Preserve an anchor turn and pixel offset while prepending. Load
   around an addressed turn for search hits and navigation; do not download
   all intervening turns. A compact navigation projection supplies the rail
   instead of requiring every prompt body in memory.
3. Separate settled page storage from the active turn. Subscribe/replay from
   the snapshot watermark with overlap deduplication or an explicit resync;
   never leave a snapshot-to-stream gap. Stream updates address a small live
   item map. Settlement replaces the affected turn, not the entire history.
   A gap invalidates/refetches affected loaded ranges, not all prior turns.
4. Keep a measured byte budget for page caches and a bounded mounted range,
   protecting visible, live, focused and unsaved interaction state. Evict
   reloadable old pages and inactive chat views. Preserve approvals/questions
   as current conversation state even when their originating turn is off-page.
   The SharedWorker baseline cache must participate in the revised contract.
5. Detect unchanged conversations by a cheap durable revision before
   projecting or downloading content. Preserve missed-finish recovery and
   participant-message discovery; simply deleting polling would regress
   both. Revisions must cover card actions, set-aside/restore, replacements,
   contributor/account erasure and other projection changes, not only text
   appends. Prefer the existing shared stream for change notification with
   conditional reconciliation as recovery.
6. Build a rebuildable, scoped history projection/index over canonical ledger
   facts. Fetch full original turn contents when requested, not summaries
   substituted for them. Follow existing fenced/index and encrypted-content
   rules; add no plaintext transcript cache, alternate ledger, retention
   policy or raw filesystem discovery path.

A page should normally keep a complete turn together, but a single huge turn
needs bounded detail/body reads as well. Page-size defaults must follow
measured byte and latency budgets; a fixed number of turns is insufficient.
Full transcript exports can be separate explicit operations. UI paging does
not change context selection for the model or delete append-only history.

The current merger treats durable snapshots as whole replacements.
`transcriptToItems` also generates assistant keys from a local sequence, and
`ChatPane` saves window positions by numeric index. Reusing these unchanged
for independently fetched pages would discard older pages, collide keys or
move the reader. Use canonical entry/turn/output ids throughout. Apply
settled transcript and presentation pages atomically as today.

## Order and acceptance

Start with the versioned page/revision contract and genuinely bounded
server reads; then page-aware client state and live settlement; then all
secondary readers and catalog-backed chat listing. Avoid a temporary second
history protocol or UI-only slice presented as completion.

Acceptance must compare equal recent content over 100, 1,000 and 10,000
historical turns, independently of 100, 1,000 and 10,000 chat summaries.
Measure cold and warm server latency, records/segments decoded, response
bytes, browser heap and mounted nodes, delta-update work, and unchanged idle
traffic. Opening a recent page and applying a live delta should depend on
that page/live turn, not total retained content. Index catch-up/rebuild may
scale with history, but cannot silently recur on each normal read.

Exercise rapid scroll, duplicate/out-of-order pages, navigation cancellation,
reconnect gaps, multiple tabs, split panes, live appends while reading older
turns, huge individual turns, cards/actions, scheduled and participant turns,
branches, set-aside/restore, erasure and revoked sharing. Preserve scroll
anchors after async image/code/card layout, keyboard focus and accessibility;
do not force a reader following older history to the live tail. Verify at
1440 × 900 and 390 × 844, light/dark, in browser and shared desktop client.

## Evidence and limits

The existing `tests/presentation-hydration.mjs` passed with Node's type
stripping. A synthetic process invoked the actual
`applyPresentationDelta` function 100 times against arrays of 1,000, 10,000
and 100,000 items, targeting the final item. Each item had a 1,000-character
fallback and source document. One local run measured:

| Items | Serialized timeline MiB | Mean stringify ms (10 reads) | 100 deltas ms |
|---|---:|---:|---:|
| 1,000 | 2.03 | 1.49 | 4.80 |
| 10,000 | 20.37 | 12.97 | 17.81 |
| 100,000 | 203.87 | 134.22 | 383.61 |

These are synthetic item counts, not user-turn counts or production payload
estimates. Serialization is uncompressed. The timings exclude server work,
network, parsing, Solid reconciliation and DOM rendering; they demonstrate
the existing growth mechanism, not an end-to-end capacity limit. No private
data home was read, no provider called, and no live performance or visual
acceptance is claimed. Runtime implementation and scale/browser acceptance
remain open in the tracker.
