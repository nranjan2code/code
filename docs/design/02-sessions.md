# 02 — Sessions (vak-session)
Status: implemented in 2.0.0; entry kinds and projection extended in 3.5.0
(docs/design/68-context-engine.md)

## Format

One JSONL file per session at
`$VAK_HOME/agents/<agent-id>/sessions/<cwd-hash>/<session-id>.jsonl` (with the
default agent using `agents/vak/sessions/`). Every line is hash-linked to its
predecessor (`prev_hash`) and carries a stable `id` + `parent_id`:

```json
{"id":"…","parent_id":"…|null","ts":"…","prev_hash":"…","kind":"header|message|compaction|receipt|goal|goal_update|activity|work|intent|child_run|turn_capabilities_bound|presentation|turn_card", …}
```

- `header` — the **frozen execution contract**: app version, provider, model,
  full system prompt, tool list, permission mode, cwd, parent session. Audits
  and replays explain every decision from this snapshot, never current state.
- `message` — neutral `Message` + optional meta (model, stop_reason, usage,
  a `control` tag for a runtime-authored nudge — see below).
- `compaction` — `{summary, first_kept_entry_id, tokens_before}`, covering a
  contiguous range of whole **turns**, not individual messages, since 3.5.0.
- `receipt` — one work unit's provider dispatches (`WorkReceipt`:
  purpose, winning attempt, per-attempt reason/domain/settlement/usage,
  `prefix_digest`/`prefix_tokens` since 3.5.0). Audit only — `derive_messages()`
  skips it (`docs/design/42-managed-work-contracts.md`).
- `goal` / `goal_update` — objective/criteria lifecycle and the turn's
  relationship to the active collaborative goal. Audit only — skipped by
  projection (`docs/design/42-managed-work-contracts.md`).
- `activity` — UI/audit lifecycle facts (approvals, retries, route fallback,
  prefix-cache breaks, capacity probes and feedback, drift and freshness
  outcomes — docs/design/68-context-engine.md). Never model-visible.
- `work` — durable managed-work lifecycle events; the projector, not the
  event stream, is authoritative for current state.
- `intent` — this turn's resolved reading, engagement, and provenance
  (`docs/design/47-commitment-kernel.md`). Unlike most audit kinds this one
  **is** model-visible: it carries `model_visible`, the exact text the
  engagement contributed, and the tail (below) renders those recorded bytes
  rather than re-deriving them, so a replay reproduces the prompt even if the
  derivation rules have since changed. Only the newest intent entry is
  projected.
- `child_run` — a terminal marker a child agent writes before its parent
  observes the result. Never model-visible.
- `turn_capabilities_bound` — the exact capability interface (core tools,
  deferred tools, the tool index text, declared domains per tool) admitted
  for one provider turn. Audit only.
- `presentation` — a validated `emit_*_card` payload or an equivalent `vak`
  fence, written once at the moment it validates (docs/design/68-context-engine.md
  §10): canonical payload, its digest, the evidence it was derived from, and
  a schema-driven identity digest. The display layer and the model-visible
  projection both read this entry; nothing is rebuilt from tool call
  arguments. Never model-visible raw — skipped by `derive_messages()` like
  `receipt`.
- `turn_card` — a turn's closing record (`TurnCard`: asked / did / answered
  as presentations + narration / outcome / reading / measured token cost),
  written once when the turn closes and never rewritten. Never model-visible
  raw: a later turn sees it through a one-line `<turns>` entry or, if
  promoted by relevance, through the turn's full record — never through this
  entry directly.

## Invariants

1. **Append-only.** No rewrite, no delete. Branching = append an entry whose
   `parent_id` points anywhere in the tree (`branch_at`). Compaction = an
   entry; full history stays greppable forever.
2. **Model-visible means logged.** Model context is *only ever* produced by
   `derive_messages()` / `derive_with_plan()` projecting the log. The agent
   loop builds requests from this projection; the projection-invariant test
   asserts every request message traces back to a logged entry.
3. **Projection is turn-based, not message-based (3.5.0).** `TurnIndex`
   walks the chain into turns — one user directive plus every assistant step
   and tool exchange until the final answer — and a turn is never split.
   `derive_messages()` (no plan) projects every closed turn at full fidelity
   plus the still-open turn verbatim; `derive_with_plan()` selects, per
   closed turn, `Full` (the turn's real `tool_use`/`tool_result` pairs, with
   results replaced by a schema-driven digest and cards by their short ack —
   never a character-count trim), a one-line `Card`, or omission behind an
   existing `Packet`/`Compaction` entry, decided against a measured
   `CapacityProfile` budget (docs/design/68-context-engine.md §2–§4). Control
   messages (nudges, intent notes) are never part of a turn's projected
   record; they render once, per turn, in the request's **tail** block
   (docs/design/07-prompt.md, docs/design/68-context-engine.md §6) instead of
   inline in history.
4. **Compaction summarizes turn cards, not raw exchanges.** `plan_compaction`
   operates on turn boundaries; a packet range is only created when the
   working-set plan needs it (incremental, not an overflow emergency), and
   the summariser sees the covered turns' `TurnCard`s, never their tool
   dumps.

## Why trees, not lists

Branching, fork, time-travel, and compaction all fall out of parent pointers +
one walk (`chain_to_root`). No special-case machinery per feature.

## Later

- worker sessions linked via `parent_session_id` + spawning tool-call id

## Diff note — ledger robustness (2.0.0)

Session directories hash the cwd with FNV-1a (fixed), not `DefaultHasher`
whose output changes across Rust releases — old sessions stay reachable
after toolchain upgrades. `open`/`create` take an exclusive `try_lock` for
the handle's lifetime (second process gets `SessionError::Locked`);
`create` on a non-empty file is refused (no double headers). A torn or
damaged line no longer makes a session unresumable: it is skipped and
surfaced via `warnings()`. `total_usage` sums the active chain only, so
abandoned branches stop inflating counts.

## Diff note — context engine (3.5.0)

Four entry kinds were added (`turn_capabilities_bound` predates this but
gained fields; `presentation` and `turn_card` are new; `activity` gained
capacity-probe/feedback and prefix-change kinds) and the projection was
rewritten from a flat message walk with a 300-character historical-result
trim to the turn-based, budget-planned projection described in invariant 3.
Full rationale, the measured findings that drove it, and the `CapacityProfile`
probe are in docs/design/68-context-engine.md, which supersedes
docs/design/17-context.md.
