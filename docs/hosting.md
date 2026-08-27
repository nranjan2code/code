# Hosting vakcoder

`vakcoder serve --gateway` runs the single Runtime authority. The CLI, TUI,
desktop, and admin console are clients of that process; external channel
adapters consume its authenticated delivery contract; none
of them owns sessions, configuration, credentials, schedules, or tool
execution.

## Topology

```text
                    ┌──────────────────────────────┐
 CLI / TUI / desktop ─▶ authenticated vak-server  │
 admin / adapters ────▶        │                  │
                              ▼                   │
                         vak-runtime              │
                    state.db + sessions + blobs    │
                    broker + leases + audit       │
                    └──────────────────────────────┘
```

Run one gateway for a data home. A second gateway cannot acquire the runtime
lock. Clients connect over loopback by default; remote access requires an SSH
tunnel or an explicitly protected network endpoint.

## Start

```bash
vakcoder self install          # mints VAKCODER_GATEWAY_TOKEN into <data_home>/.env
vakcoder serve --gateway --port 8901
```

`self install` provisions the bearer token and the service unit; run `serve`
by hand only when managing the process yourself. To supply the token out of
band instead, export `VAKCODER_GATEWAY_TOKEN` before starting. Without
`--gateway` the process serves the API but claims no lock and publishes no
receipt, so it cannot redirect installed surfaces away from the managed
Runtime.

The service creates the canonical data home and writes its runtime status to
`<data_home>/runtime/gateway.json`. Stop it with SIGTERM or Ctrl-C; both run
graceful shutdown, cancelling in-flight runs and releasing the runtime receipt
and lock. A lock left by a killed process is reclaimed on the next start. Use the managed installer for launchd or
systemd integration; service units must execute the installed `vakcoder`
launcher, never a Cargo build directory.

## Canonical state

The Runtime data home contains:

```text
<data_home>/
├── config.toml                    global Runtime configuration
├── .env                           credentials (0600; never logged)
├── state.db                       transactional control-plane state
├── sessions/<project>/<id>.jsonl  append-only model-visible history
├── blobs/<sha256>                 content-addressed file/checkpoint data
├── audit/operations.jsonl         append-only operation receipts
├── runtime/gateway.json           live gateway endpoint metadata
├── locks/runtime.lock             singleton Runtime lock
└── logs/                          service logs
```

The project layer is `<project>/.vakcoder/project.toml`. It is loaded only for
the registered canonical project root. Cache/search indexes are rebuildable and
never the source of truth.

## Credentials and security

Provider and channel credentials belong in `<data_home>/.env` or the process
environment. They are injected only into the intended provider/channel worker;
they are never ambient Bash subprocess state, configuration output, logs, or session
content.

Every API request is authenticated except `/health` and the public admin
assets/login route. Every effectful command is
authorized by the Runtime before broker dispatch. Restricted filesystem access
is rooted at the registered project; cancellation and capability-epoch changes
revoke in-flight work and stale approvals.

## External delivery adapters

`vak-delivery` emits ordered packets for external adapters. The Runtime stores
delivery jobs and their status; an adapter acknowledges delivery and owns any
destination-specific retry policy through its transport boundary. An outage
cannot create a second session or state owner.
Transport credentials remain outside Runtime state and are supplied only to the
adapter that owns the destination.

## Backups and upgrades

Use the Runtime backup command/API to export `state.db`, session ledgers, blobs,
audit records, and configuration metadata. Restore is explicit and conflict
checked. Program upgrades replace the installed Runtime binary only; they do
not rewrite application state. This is a greenfield contract: installation
uses only the canonical data home and does not discover alternate layouts.

## Diagnostics

```bash
vakcoder doctor
vakcoder config dump
vakcoder sessions
vakcoder admin --print
```

Check the gateway health endpoint and logs before changing configuration. A
client that cannot authenticate or complete the `/version` handshake must stop
with an actionable error rather than silently creating local state.
