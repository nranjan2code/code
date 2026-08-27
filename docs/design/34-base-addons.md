# 34 — Runtime and client surfaces

The Runtime is the product boundary. The executable base starts one
`vak-runtime` behind `vak-server`; every other surface is a client addon. This
split is about packaging and process lifetime, not separate application
architectures.

## Ownership

| Component | Owns | Connects through |
|---|---|---|
| `vakcoder` | Runtime process, CLI, gateway, admin assets | `vak-runtime` directly |
| `vak-tui` | terminal input and rendering | `vak-client` HTTP/SSE |
| `vak-desktop` | native shell and rendering | secured Runtime router |
| admin SPA | browser presentation | `/admin/api` on `vak-server` |
| external channel adapters | transport ingress/egress | Runtime commands and delivery |

No client opens SQLite, session JSONL, blobs, or secrets. No client constructs
an agent or invokes a tool. The Runtime admits runs, freezes their contract,
authorizes effects, owns cancellation, and emits ordered events.

## Runtime services

- `vak-domain` provides IDs, requests, responses, run context, status, and
  events.
- `vak-config` owns global/project TOML and secret loading with revision-checked
  atomic writes.
- `vak-storage` owns SQLite transactions, append-only audit, session files, and
  content-addressed blobs.
- `vak-services` provides transactional CRUD for tasks, approvals, memory,
  skills, bindings, inbox, deliveries, checkpoints, and flows.
- `vak-agent` executes an admitted turn over provider/tool interfaces.
- `vak-runtime` composes those services and enforces leases, cancellation,
  capability epochs, and broker dispatch.
- `vak-server` exposes the authenticated HTTP/SSE contract; `vak-client`
  consumes it.

## Process and data rules

There is one gateway lock per data home and one runtime receipt at
`<data_home>/runtime/gateway.json`. A plain server/client process does not
create a second Runtime or write control state.

The data home layout is defined once in
[36-greenfield-runtime.md](36-greenfield-runtime.md). SQLite is the mutable
control-plane authority, session JSONL is the model-visible history, blobs are
content addressed, audit is append only, and indexes/caches are rebuildable.

## Install and verification

```bash
scripts/build-install.sh --gates
scripts/build-install.sh --with-tui --with-desktop
```

`--no-service` keeps artifact tests isolated. All clients must complete the
authenticated `/version` handshake and use the same project/session/run IDs.
The release gate exercises CRUD, streaming, cancellation, approval revocation,
backup/restore, and clean shutdown across CLI, TUI, desktop, admin, and
external delivery adapters.
