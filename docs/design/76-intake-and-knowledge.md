# 76 — Intake and knowledge: one path for data in, one place to use it

Status: **proposal, 2026-09-27; §9 shipped 2026-09-27 (`5426018f`).
Nothing else here is shipped.** It is milestone M6.5 of the data
architecture plan (revision 3), and its Sources poll through that plan's
Trigger model (`source_poll`) with cursors (doc 73 §8). It supersedes the
feed system's target model in `docs/design/51-feed-system.md` (which stays the
accurate description of the shipped 5.x Python pipeline until this lands) and
is the intake-and-retrieval facet of the data architecture in
`docs/design/73-data-architecture-and-lifecycle.md` and
`docs/design/74-lifecycle-and-data-administration.md`. It depends on that
work: M1 (trace key), M2 (object store), M6 (catalog). §8 places it in the
plan. §9 is a security fix that lands independently and now.

## 0. The thesis, and why today misses it

The intended model: **one place any data comes in, and once in, it is usable
for everything** — the agent answering a question, a person searching, an
alert firing, a digest, a report. What ships instead is a narrow poller with a
human-only reader:

- **The agent cannot use feeds.** `feed_mcp.py` is invoked only by the Rust
  HTTP handlers (`crates/vak-server/src/feeds.rs`); it is never registered as
  an MCP server the model can reach, and there is no `feed_*` tool in the
  agent's toolset. Ingested content is a dead end for the assistant, which
  answers from `webfetch`/`browse` instead.
- **"Any data" is one shape.** Only RSS/Atom, YouTube, HN/Reddit/Lobsters and
  custom HTTP. Channel attachments and dropped files take a *separate* path
  (`save_to_inbox` → `inbox/`, `crates/vak-server/src/inbox.rs`). Nothing else
  enters.
- **Four disconnected stores.** Feeds (DuckDB), memory notes (`MEMORY.md`),
  entities (`ENTITIES.jsonl`) and transcripts (SQLite FTS `store.db`) are four
  schemas and three search implementations. `session_search` merges memory +
  entities + transcripts but not feeds; the feed's own `search_index` table is
  written on every item and read by nothing.
- **No lifecycle.** Items, the search index, the alert log and the run log are
  never pruned; `max_items_per_feed` is loaded and never enforced; a matched
  alert with no channel target is a silent run error rather than a durable
  signal.

This document makes the thesis real: intake is one contract feeding the
catalog, retrieval is one query the agent and every surface share, and
lifecycle is the shared reconciler — not a subsystem beside the platform.

## 1. Vocabulary

- **Source** — a durable declaration that produces items over time by
  *pulling* (an RSS feed, a channel, a mailbox, a drive folder). Desired-class
  state owned by an Agent (`docs/design/73` §5), with a `src_` id.
- **Intake** — the one pipeline every item travels, whatever its origin.
- **Item** — one observation from a source or a push. It is an **Object**
  (content-addressed body) plus a **catalog node** with a `TraceKey`,
  `derived_from` (its source and the run that fetched it), and a security
  disposition. It is never a bespoke row in a private DuckDB file.
- **Connector** — host code (Rust, or a pinned worker-shipped script) that
  knows how to fetch and normalise one source kind. Connectors are a closed,
  host-owned set, never resolved from a workspace (§4).
- **Push intake** — an item that arrives without a source: a channel
  attachment, a file dropped in a client, a URL or note a person hands in.
  `save_to_inbox` becomes one push connector, not a parallel path.

## 2. The one pipeline

```
Source (pull)  ┐
Push (attach,  ├─▶ Run (cause: Schedule|Channel|User)  docs/design/73 §4,§8
 drop, "save") ┘        │
                        ▼  in the broker worker, sandboxed, SSRF-checked (§4)
                 Fetch ─▶ Normalise ─▶ Detect (label, never drop; §5)
                        │
                        ▼
     Object store (content-addressed, deduped)   +   Catalog rows
        body, dedup by object id                     nodes: item, TraceKey,
                                                      derived_from, disposition
                                                      text: FTS projection
                                                      edges: produced_by,
                                                             references
                        │
        ┌───────────────┼───────────────────────────┐
        ▼               ▼                             ▼
  Agent tool      UI reader / search            Alert evaluation
  (§6, ACL)       (§6, ACL)                     → Inbox + optional channel (§7)
                        │
                        ▼
              Lifecycle reconciler (§8): retention, dedup window,
              max-items, seal, erase — the shared one, not a feed cron
```

Every stage is a Run record (`docs/design/73` §8): a poll that fetched
nothing, a fetch that failed, a source not due, a blocked SSRF target — each is
a typed decision, never `eprintln!` + `return`. This fixes today's "a failed
fetch is recorded as empty" and "a source that never succeeds has no row"
directly, because the record exists whatever the outcome.

## 3. Identity and classes

An item maps onto the existing data classes (`docs/design/73` §5); intake
introduces no new storage model.

| Part | Class | Notes |
|---|---|---|
| item body (title, content, raw) | **Object** | content-addressed, deduped within the tenant; the same article from two sources is stored once |
| item node (ids, source, times, tags, disposition) | **Record** | carries the full `TraceKey`; `derived_from` names the source and the fetching run |
| source declaration | **Desired** | Agent-owned; privileged against untrusted projects (§4, §9) |
| FTS/vector projection | **Derived** | rebuilt from records + objects; erased with its `derived_from` |
| alert rule | **Desired** | Agent-owned |
| alert match / delivery | **Record** | keyed to the conversation it is delivered into |

Consequences that fix today's leaks by construction:

- **Scope is the space/audience, not a cwd hash.** An item is visible through
  the catalog's ACL filter (`docs/design/73` §9, invariant 37), so the
  cross-workspace leak in the current reader cannot recur — there is one
  filtered query, not a scoped path and an unscoped one.
- **Quarantine and removal are node state**, applied before ranking in the one
  query, so no surface can forget them (today `get_latest` does).
- **Every item has a stable catalog id** the UI can open — the current reader
  returns rows with no id.

## 4. Security — the boundary is the worker, not the server

Today the feed scripts run *in the server process*: `feeds.rs` spawns
`python3` with `.envs(...)` **added to** the inherited environment (no
`env_clear`), and `feeds_dir()` prefers the **workspace's own**
`scripts/feeds/feed_ingest.py`. That is arbitrary code execution with the
server's secrets — see §9, reproduced live.

The redesign closes it structurally:

- **Connectors are host-owned and closed.** Fetch and parse run in the broker
  worker (invariants 14, 39), under a network-scoped, `env_clear`ed sandbox
  with only the operational allowlist — never the server's `VAK_GATEWAY_TOKEN`
  or provider keys (invariant 12). A workspace can never contribute a
  connector, and no path is ever resolved from `cwd`.
- **Untrusted parsing runs where hostile bytes belong** — the same worker,
  network-denied for the parse step, bounded in size and time — because feeds
  are external content exactly like an Office file (invariant 39).
- **Enabling intake is privileged.** `feeds.enabled` / intake config is
  stripped from untrusted project layers, like `hooks`/`mcp`/`gateway`
  (`docs/design/73` and the demotion block in `crates/vak-config/src/lib.rs`).
- **SSRF and redirect policy are the host's**, applied to every source URL and
  every redirect target, once, in the worker — not re-implemented per driver.

## 5. Detection labels, never drops

Today the injection detector runs inside every driver and *discards* any item
that matches, and the false-positive rate makes it unusable: ordinary tech
headlines ("a tiny eval() sandbox", "system: notes", "pretend you are a
compiler", "Assistant: we shipped it", "`<s>`old`</s>`") are dropped, and the
recorded evidence is an empty string.

The contract (already stated in `docs/design/51` and 73/74, now enforced):

- Detection **labels**, it does not drop. An item gets a disposition
  (`accepted`, `quarantined`, `blocked`) and retained evidence; quarantined
  content is excluded from agent retrieval and alerts until an operator
  releases it, but it is never silently lost and its evidence is never empty.
- Instruction-like content from a source is **data** and cannot change the
  agent's authority, tools, prompt layers or permissions — enforced by the
  boundary (§4), not by pattern-matching prose.
- The detector is tuned against a real headline corpus and its false-positive
  rate is a tracked metric, because a filter that quarantines a third of a
  normal feed trains people to bulk-release.

## 6. Retrieval — one query, and the agent can finally use it

- **One catalog search** (`docs/design/73` §9: FTS5 + optional vector) replaces
  the feed's unread `search_index`, `store.db`, and the ad-hoc scans. It ranks
  by relevance first (freshness/trust only re-rank a match, never create one),
  filters by the caller's audience and grants before ranking, and returns only
  what the caller could open directly.
- **The agent gets a tool.** Intake content becomes reachable in a turn through
  one retrieval tool over the catalog. The cleanest shape is to **widen the
  existing recall** rather than add a fourth search: `session_search`
  (`crates/vak-core/src/session_search.rs`) already merges transcripts, memory
  and entities — it gains catalog items as another source, or is renamed to a
  single `recall`/`knowledge_search` that the model reaches for by default
  (open decision D1, §10). Either way, "usable for various things" means the
  assistant, not only a person in a tab.
- **The UI reader and alerts call the same query**, so there is one relevance
  model and one ACL, not three.

## 7. Alerts reach the durable attention layer

An alert match becomes a durable **Inbox** entry (a new
`vak_core::inbox::Kind`, e.g. `FeedMatch`), so nothing is silently dropped when
no channel is configured — every other signal already lands there
(`docs/design/29`). Channel delivery stays optional and goes through the
one host delivery runtime (`vak-delivery`); intake code never writes an outbox
record itself. Evaluation is idempotent per `(alert, item)` with a real
cooldown, fixing today's re-fire-every-tick and unbounded `alert_log`.

## 8. Lifecycle and sequencing

- **Retention is the shared reconciler** (`docs/design/74` §5), not a feed
  cron. Intake objects and records carry retention labels; the reconciler
  seals, tiers, prunes and erases them with everything else. `max_items` per
  source and the dedup window are enforced there, and a run row is written only
  when work happened (not once per idle 20-second tick, as today).
- **Erasure reaches intake** because items are objects with `derived_from`;
  erasing a conversation or a source erases its derived catalog rows and
  embeddings and GCs orphaned objects (`docs/design/74` §4).
- **Sequencing.** Intake is a *consumer* of the catalog, so it builds on:
  - **M1** TraceKey — every item carries one;
  - **M2** object store — item bodies are objects;
  - **M4** triggers and cursors — a Source polls through a `source_poll`
    trigger, and its position is a cursor ref;
  - **M6** catalog — nodes/edges/text are where items live and are searched.

  It is therefore an **M6-or-later** deliverable (proposed **M6.5**, before or
  with M8 artifacts). It does not start before M6, and it ships whole
  (invariant 30): the Python pipeline, the DuckDB store, `feed_mcp.py`,
  `feed_search.py`'s BM25, the `search_index` table and the four dead helpers
  are **deleted in the same change** that lands catalog intake. There is no
  compatibility path and no migration (0 users; the data baseline refuses pre-baseline
  state, invariant 29).

## 9. The one fix that lands now, independently (shipped 2026-09-27)

The code-execution hole (§4) is live in 5.x and must not wait for M6. Verified
on a running server against an **untrusted** `/tmp` workspace
(`trusted: false`): a `.vak/config.toml` with `[feeds] enabled = true` plus a
workspace `scripts/feeds/feed_ingest.py` caused the scheduler to run that
script **every 20 seconds unattended**, and the script read
`VAK_GATEWAY_TOKEN` from the inherited environment. A feed script has network
access by design.

A small change on the current pipeline, independent of the redesign:

1. **Strip `feeds.enabled` (and any intake config) from untrusted project
   layers** in `crates/vak-config/src/lib.rs`, beside `hooks`/`mcp`/`gateway`.
   An untrusted repo cannot enable an unattended surface (invariant 15).
2. **`env_clear` the feed subprocess** in `crates/vak-server/src/feeds.rs` and
   pass only the operational allowlist plus the `VAK_FEEDS_*` paths — the
   parent environment, gateway token and provider keys never reach it
   (invariant 12).
3. **Resolve connectors from the install, never `cwd`** — drop the
   `cwd/scripts/feeds` preference in `feeds_dir()` so a workspace cannot
   supply the script the server runs.

These three are the interim boundary until §4 makes it structural. They are a
separate, small change from the redesign and carry their own regression tests
(an untrusted workspace's `feeds.enabled` is ignored; the subprocess sees no
parent secret; a workspace `scripts/feeds` is never executed).

## 10. Open decisions (need the maintainer)

- **D1 — one recall tool or a named intake tool?** Widen `session_search` into
  a single `recall` over transcripts + memory + entities + intake, or add a
  distinct `knowledge_search`. Recommendation: **one `recall`** — fewer tools
  measure better on small models, and "everything I know" is one question.
- **D2 — connectors in Rust or worker-Python?** Rewrite the drivers in Rust
  (one language, no Python runtime dependency, easiest to sandbox), or keep the
  parsers as pinned worker-shipped Python behind the broker. Recommendation:
  **Rust for RSS/HTTP/YouTube-XML** (small), reassess for anything needing a
  large parser.
- **D3 — is intake M6.5 (before M8) or folded into M8 artifacts?** Settled
  by plan revision 3: M6.5, in parallel with M8.
  Recommendation: **M6.5**, so retrieval is unified as soon as the catalog
  exists.
- **D4 — do we ship §9 now** as a standalone 5.x change ahead of the redesign?
  Settled: shipped in `5426018f`.
  Recommendation: **yes** — it is a live token-exfil hole.
