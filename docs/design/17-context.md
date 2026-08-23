# 17 — Long-horizon context

How multi-turn sessions survive context-window limits. Inspired by
vakyartha's prompt-budget protocol, reduced to a coding-harness kernel.

## Policy

`ContextPolicy` (vak-agent/context.rs):

| field | default | meaning |
|---|---|---|
| `context_window` | 128_000 | model window (config: `context_window`) |
| `max_output` | 8_192 | completion reserve |
| `compact_threshold` | 0.8 | trigger at 80% of input budget |
| `keep_recent` | 6 | recent messages always verbatim |

Input budget = window − output. Trigger = threshold × budget.

## Loop integration

Before every model step:

1. **Estimate** the projection (chars/4 heuristic — planning estimate only,
   never treated as provider usage).
2. Over trigger ⇒ **compact**: render the older segment (all but the last
   `keep_recent` messages) to a transcript, ask the same provider for a
   dense structured summary (task/state/files/decisions/errors/open items,
   ≤400 words), and write it as a **compaction entry** via
   `SessionLog::compact_tail`.
3. Re-estimate. Still over the *full* input budget ⇒ **reset-with-handoff
   rescue** (doc 27 Phase H, once per run, `[goal] handoff_reset`): the
   model writes a structured shift-change summary and the projection
   becomes ONLY that summary (`reset_all` compaction entry). If the rescue
   is disabled or its write fails ⇒ **fail closed** with a
   typed error (no silent truncation, no provider switch).

The compaction entry is ledger data like any other: append-only, audited,
and `derive_messages()` renders it as a `<context_summary>` user message.
Recent turns stay verbatim so stateless follow-ups keep working.

## Events

`AgentEvent::ContextCompacted { before_tokens, after_tokens,
summarized_messages, selected_messages, dropped_messages }` — surfaced in
the TUI as a 📦 line.

## Packet accounting + deterministic gate (doc 27 Phase C)

Every compaction plan and entry now carries a **partition**: the message
entries visible in the projection split into `selected` (verbatim tail)
and `dropped` (summary material) — disjoint, jointly exhaustive over
visible entries; prior compaction pseudo-entries belong to neither.
`plan_compaction` computes it; `apply_compaction` persists it on the
entry (`Option`, serde-defaulted — old ledgers parse unchanged).

`vak-eval::run_context_scorecard()` gates the machinery deterministically
(no model calls): partition integrity across a keep_recent sweep,
recent-turn recall floor, verbatim exclusion of dropped turns,
evidence-loss visibility, tool-pair boundary safety, repeated-compaction
behavior. Printed by every offline `eval` run; a failing metric fails
the eval. Semantic selection quality (what a summarizer chooses to keep)
stays with `eval --live` — absent evidence is UNKNOWN, never assumed.

## Still not built

- Relevance-scored retrieval of dropped turns (vakyartha's
  context-assembly goes further with `<session_state>` packets and typed
  reference frames; our ledger keeps everything on disk so recall can be
  added later without changing the format).
- Provider-cache-aware prefix shaping beyond our stable system prompt.

## Tests

- estimation + budget math + request shape (unit)
- overflow triggers compaction → summary entry in projection + on-disk
  ledger → recent turns verbatim → run completes
- still-over-after-compaction fails closed with typed error
