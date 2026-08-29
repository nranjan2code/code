# 23 — Memory & cross-session recall

Pillar 2 of the platform plan (see `22-gateway.md` intro): Hermes' crown
differentiator is not the loop — it is that **nothing learned is ever
stranded**. Past sessions become retrievable context via a model-visible
search tool plus FTS-style indexing. We get the same capability with zero
new dependencies because our session store is already structured JSONL.

## Model

```
model calls session_search {query, limit?}
        │
vak_session::search::search(home, cwd, query, limit, exclude)
        │  scans <home>/sessions/<hash_cwd(cwd)>/*.jsonl
        │  scores user+assistant texts (term frequency w/ saturation,
        │  whole-phrase bonus), newest-first tiebreak
        ▼
top-N hits: {session, ts, role, score, snippet}
        │
returned as an ordinary tool result → appended to the ledger
```

Invariant 1 holds by construction: the answer reaches the model only as a
logged `tool_result`; nothing is injected outside the chain.

### Scoring

Deterministic, dependency-free:

- Query tokens lowercased, alphanumeric, length ≥ 2.
- Per-hit score: `Σ over unique terms: 1 + ln(count)` (saturating), plus
  `+3` when the full phrase occurs verbatim (case-insensitive), plus a
  small recency nudge (`ts` descending rank × 0.01).
- Bounded work: at most the trailing `MAX_SCAN_LINES = 4000` message lines
  per ledger, `DEFAULT_LIMIT = 8` hits, snippets ≤ 240 chars centered on
  the first matched term.

An incremental index (per-ledger mtime-keyed cache) can slot in behind the
same function signature later; scan-first keeps v1 honest about relevance
and simple.

## Surfaces

| Surface | Access |
|---|---|
| Agent loop | `session_search` tool, injected in `Core::run_turn_with` next to task/MCP |
| TUI/desktop/server/admin | `GET /search?q=…&limit=…` over the same function; Admin exposes scoped CRUD and cleanup controls |
| Future | compaction integration: auto-cite prior sessions in summaries |

## Configuration

```toml
[memory]
search_enabled = true   # default; false removes the tool from the loop
```

Privileged rules do not apply (read-only, workspace-scoped), but unknown
keys still warn per convention.

## Exclusions & safety

- The **current** session id is excluded — its content is already in
  context; recalling it would double-count.
- Subagent ledgers (`child-*`) are included; they carry `parent_session_id`
  and are legitimate knowledge.
- Search reads only; it never mutates ledgers, and respects the frozen
  contract (no rewriting, branching untouched).

## Operational invariants (2026-08)

All durable-memory writes use a per-store lock and atomic temp-file replacement;
append, amend, and forget are bounded and reject control characters in header
metadata. Server and desktop search use the same curated-document ranking as
the agent tool, including stored provenance timestamps. Channel overlays are
applied after the complete tool set is assembled, and background reflection is
skipped in read-only or channel-denied contexts.
Retention is conservative: age alone never deletes a durable note. `vak memory
clean` removes only abandoned lock/temp artifacts and empty workspace
directories; stale knowledge is removed explicitly with `forget` or amended in
place. There is no automatic “dream”/overnight consolidation pass; reflection
is bounded to at most two deduplicated proposals after a clean completion.

## Phases

| Phase | Delivers | Exit criterion |
|---|---|---|
| **M0 ✅** | scan+score search in vak-session, `session_search` tool wired into every run, `/search` endpoint, `[memory]` config | relevance unit tests + agent-loop e2e proving the result lands on the ledger; fmt/clippy/tests green |
| **M1 ✅** | mtime-keyed per-ledger index cache + `search_all` cross-project scan; `/search` in TUI (`--all` flag), desktop SearchModal global toggle, `GET /search?all=true` | warm 10k-message store answered from cache < 500ms CI-safe bound (typ. ≪50ms); appends visible next query without restart; cross-dir exclusion tested |
| **M2 ✅** | Write path via model-invoked `remember` tool → `<home>/memory/<hash>/MEMORY.md` (plain markdown, provenance blocks, hand-edit-tolerant parser); recalled by search with outranking bonus; `[memory] write_enabled` flag; `GET /memory` + `vak memory` | curated notes rank above equal transcript hits (`memory_extras_outrank_equal_transcript_hits`); agent remembers mid-run and a later run recalls it citing memory (learning_loop e2e) |
| **M3 ✅** | Tiered + editable memory (docs/design/29 P1): global `<home>/memory/user/USER.md` profile tier recalled as `profile` role extras; explicit `forget_note`/`amend_note` (byte-safe rewrites, FNV id per provenance header); full HTTP CRUD (`POST/PATCH/DELETE /memory`) shared by desktop/gateway/CLI | forget removes exactly one block preserving the rest byte-for-byte; profile notes outrank equal transcript hits (`profile_tier_recalled_as_profile_role_alongside_memory`); amend/forget round-trip over HTTP e2e |
| **M4 ✅ (0.11.20)** | `[memory]` toggles (`search_enabled`, `write_enabled`, `reflection`, `skill_proposals`) are live-editable, not just config-file settings read once at startup: `Core::effective_memory_*` read every call site instead of `config().memory.*` directly, backed by the same override+`refresh_persisted_preferences` mechanism route/theme/max_turns already use; `PATCH /config` accepts `memory_*` fields (persisted via `persist_project_memory_prefs`/`persist_global_memory_prefs`, same atomic-write shape as the route/theme preferences writer); `GET /config` reports the effective values in a `memory` object it did not carry before. Admin console's Memory page replaced its three read-only status strings with real checkboxes wired to this PATCH. | a value persisted to disk by one process reaches an already-running `Core` in another without a restart (`cross_process_memory_refresh_takes_effect_without_restart`); a PATCH naming one flag leaves the other three at their current value, not whatever was on disk before the call (`patch_config_memory_flags_apply_live_and_persist`) |
