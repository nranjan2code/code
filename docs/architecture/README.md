# Architecture diagrams

Self-contained, interactive HTML documents describing how vak is put together.
Open any file in a browser — no build step, no external assets.

| File | What it covers |
|---|---|
| [`layered-architecture.html`](layered-architecture.html) | The **static** crate map: all 18 workspace crates + 2 SolidJS frontends, arranged as a five-layer dependency stack (foundations → surfaces). |
| [`runtime-topology.html`](runtime-topology.html) | The **runtime** view: which processes exist, who hosts a `Core`, the gateway/channel request path, and what is shared-central vs. private-and-local. |
| [`concurrency-model.html`](concurrency-model.html) | An **animated** walkthrough of what happens when many calls arrive at once — same session (ledger lock → 409/steering), many sessions (one `Core`, rate limit), and many workspaces (`CorePool` eviction). |

All three are derived from the workspace manifests, `README.md`, `PRODUCT.md`,
`DESIGN.md`, and the source under `crates/vak-server`, and use vak's own design
tokens so they read as part of the product.
