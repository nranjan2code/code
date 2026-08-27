# 02 — Sessions

`vak-session` provides the append-only ledger used by `vak-runtime`. A session
belongs to one registered project and contains the model-visible record of every
prompt, response, tool call, tool result, compaction, receipt, and terminal
outcome.

## Storage

```text
<data_home>/sessions/<project-id>/<session-id>.jsonl
```

Each line is an immutable entry with an ID and timestamp. Branching records a
parent ID; compaction records what was retained and omitted. Runtime also
projects control metadata into `state.db`, but the ledger remains authoritative
for reconstructing model messages.

## Invariants

- Anything sent to a provider has a corresponding ledger entry.
- Entries are appended, never rewritten or deleted by normal operations.
- A run has one admitted immutable contract and one terminal outcome.
- Cancellation preserves committed and partial output.
- Receipts record provider/model, route leg, usage, failure domain, and outcome.

Clients read transcripts through Runtime queries. They never scan session files
directly and never maintain a competing history.
