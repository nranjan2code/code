# 14 — Checkpoints

Checkpoints are Runtime-owned records attached to a session. A checkpoint
stores a caller-supplied manifest in the content-addressed blob store and
records its digest, label, and creation time in SQLite. The session ledger is
never rewritten.

## API

```text
vakcoder checkpoints list
vakcoder checkpoints restore <session> <index>
```

The HTTP client exposes list, create, manifest, and restore operations. Restore
validates that the checkpoint belongs to the requested session and returns the
stored manifest; any filesystem effect must still pass the Runtime broker and
workspace authorization boundary.

## Invariants

- checkpoint IDs and manifest digests are stable;
- blobs are content-addressed and never overwritten;
- checkpoint rows are transactional and auditable;
- restoring a checkpoint cannot address another project or session;
- session JSONL remains append-only and model-visible history is unchanged.
