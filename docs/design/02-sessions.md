# 02 — Sessions (vak-session)

## Format

One JSONL file per session at
`$VAKCODER_HOME/sessions/<cwd-hash>/<session-id>.jsonl`. Every line:

```json
{"id":"…","parent_id":"…|null","ts":"…","kind":"header|message|compaction", …}
```

- `header` — the **frozen execution contract**: app version, provider, model,
  full system prompt, tool list, permission mode, cwd, parent session. Audits
  and replays explain every decision from this snapshot, never current state.
- `message` — neutral `Message` + optional meta (model, stop_reason, usage).
- `compaction` — `{summary, first_kept_entry_id, tokens_before}`.

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

- receipts as first-class entries (unified WorkReceipt stream)
- subagent sessions linked via `parent_session_id` + spawning tool-call id
