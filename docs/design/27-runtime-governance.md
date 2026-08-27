# 27 — Runtime governance

This document records the governance contracts implemented by the greenfield
Runtime. Runtime owns admission, the frozen provider/model contract, budget
checks, capability epochs, cancellation, terminal status, and append-only
operational receipts.

Every run is admitted once with a complete immutable session contract. The
provider/model selection supplied by the caller is frozen into that contract;
provider model discovery is a separate explicit Runtime query and is never a
fallback catalogue. Permission changes revoke the old capability epoch before
new work is admitted. Cancellation preserves partial output and persists
exactly one terminal result.

The CLI, TUI, desktop, admin console, HTTP gateway, and delivery adapters
submit the same typed commands and consume the same event stream. No surface
creates a private planner, state store, or protocol.

Receipts and audit events are operational records; they are not model-visible
context unless explicitly appended to the session ledger. Session JSONL is the
source of truth for reconstructing every provider request.
