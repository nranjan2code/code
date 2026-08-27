# 28 — Operations

Operations control the one installed Runtime service. The CLI and optional
tray surface are control clients; they do not own daemon state or agent state.

## Control graph

```text
launchd / systemd --user
          ▲
   vakcoder service commands
          ▲
 CLI · tray · admin · TUI
          │ authenticated Runtime API
          ▼
     vak-server → vak-runtime
```

The service manager is authoritative for process liveness. Runtime health is
authoritative for readiness and state. These are intentionally separate
signals: a process can be alive while the API is unavailable, and an API can be
ready only after the gateway lock and data home are valid.

## Commands

```text
vakcoder self install
vakcoder self services-sync
vakcoder self status
vakcoder self uninstall
vakcoder doctor
```

Install and sync are idempotent. They operate only on the managed program and
service files; Runtime data remains in the canonical data home. Disposable
tests always pass `--no-service` and never load host service-manager units.

## Runtime singleton

`serve --gateway` acquires `<data_home>/locks/runtime.lock` and writes
`runtime/gateway.json`. A non-gateway server does not create a second Runtime.
Shutdown removes the runtime receipt and releases the lock even when the
process receives SIGTERM.

## Tray and diagnostics

The tray displays service health and opens the admin console. It delegates
start/stop/restart/install/uninstall to `vak-ops` and never creates a Runtime.
`doctor` reports service status, API health, data paths, configuration, and
capability/credential problems without mutating state.
