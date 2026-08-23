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
| TUI/desktop/server | `GET /search?q=…&limit=…` over the same function |
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

## Phases

| Phase | Delivers | Exit criterion |
|---|---|---|
| **M0 ✅** | scan+score search in vak-session, `session_search` tool wired into every run, `/search` endpoint, `[memory]` config | relevance unit tests + agent-loop e2e proving the result lands on the ledger; fmt/clippy/tests green |
| M1 | mtime-cached index for large stores; `/search` in TUI + desktop UI | 10k-entry store searched < 50ms warm |
| **M2 ✅** | Write path via model-invoked `remember` tool → `<home>/memory/<hash>/MEMORY.md` (plain markdown, provenance blocks, hand-edit-tolerant parser); recalled by search with outranking bonus; `[memory] write_enabled` flag; `GET /memory` + `vakcoder memory` | curated notes rank above equal transcript hits (`memory_extras_outrank_equal_transcript_hits`); agent remembers mid-run and a later run recalls it citing memory (learning_loop e2e) |
