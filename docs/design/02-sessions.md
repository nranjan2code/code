# 02 — Sessions (vak-session)
Status: implemented in 2.0.0

## Format

One JSONL file per session at
`$VAK_HOME/agents/<agent-id>/sessions/<cwd-hash>/<session-id>.jsonl` (with the
default agent using `agents/vak/sessions/`). Every line:

```json
{"id":"…","parent_id":"…|null","ts":"…","kind":"header|message|compaction|receipt|goal", …}
```

- `header` — the **frozen execution contract**: app version, provider, model,
  full system prompt, tool list, permission mode, cwd, parent session. Audits
  and replays explain every decision from this snapshot, never current state.
- `message` — neutral `Message` + optional meta (model, stop_reason, usage).
- `compaction` — `{summary, first_kept_entry_id, tokens_before}`.
- `receipt` — one work unit's provider dispatches (`WorkReceipt`:
  purpose, winning attempt, per-attempt reason/domain/settlement/usage).
  Audit only — `derive_messages()` skips it (`docs/design/42-managed-work-contracts.md`).
- `goal` — objective/criteria lifecycle statuses (Active/Done{audited}/
  Unverified). Audit only — skipped by projection (`docs/design/42-managed-work-contracts.md`).
- `intent` — this turn's resolved reading, engagement, and provenance
  (`docs/design/47-commitment-kernel.md`). Unlike `receipt` and `goal` this
  one **is** model-visible: it carries `model_visible`, the exact text the
  engagement contributed, and the projection emits those recorded bytes
  rather than re-deriving them — so a replay reproduces the prompt even if
  the derivation rules have since changed. Only the newest intent entry is
  projected, immediately before the turn it governs, and it is tagged as
  control so it stays out of compaction packets.

## Invariants

1. **Append-only.** No rewrite, no delete. Branching = append an entry whose
   `parent_id` points anywhere in the tree (`branch_at`). Compaction = an
   entry; full history stays greppable forever.
2. **Model-visible means logged.** Model context is *only ever* produced by
   `derive_messages()` projecting the log. The agent loop builds requests from
   this projection; the projection-invariant test asserts every request
   message appears in the log.
3. **Projection semantics for compaction:** everything strictly before
   `first_kept_entry_id` is replaced by a `<context_summary>` user message;
   entries from the marker onward remain in order.

## Why trees, not lists

Branching, fork, time-travel, and compaction all fall out of parent pointers +
one walk (`chain_to_root`). No special-case machinery per feature.

## Later

- subagent sessions linked via `parent_session_id` + spawning tool-call id

## Diff note — ledger robustness (this change)

Session directories hash the cwd with FNV-1a (fixed), not `DefaultHasher`
whose output changes across Rust releases — old sessions stay reachable
after toolchain upgrades. `open`/`create` take an exclusive `try_lock` for
the handle's lifetime (second process gets `SessionError::Locked`);
`create` on a non-empty file is refused (no double headers). A torn or
damaged line no longer makes a session unresumable: it is skipped and
surfaced via `warnings()`. `total_usage` sums the active chain only, so
abandoned branches stop inflating counts.
