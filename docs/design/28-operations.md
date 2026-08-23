# 28 — Operations: service control plane

Design notes behind `vak-ops`, `vakcoder-tray` and the `/ops` HTTP routes
(operational how-to lives in `docs/hosting.md`).

## Principle

One source of truth about background services, consumed identically by
every surface:

```
launchd (macOS) / systemd --user (Linux)
        ▲
     vak-ops   status · start · stop · restart · install/uninstall · logs
        ▲
 ┌─────┼──────────────┬──────────────┐
 tray  TUI /services  desktop panel  GET|POST /ops/*
```

Rules:

1. **vak-ops owns no daemon.** It shells out to the platform manager —
   launchd/systemd are already doing KeepAlive, restart-on-crash and boot
   start. Duplicating that would create two opinions about reality.
2. **HTTP health beats manager opinion for liveness** (`GET /health`,
   2s timeout); the manager decides installed/stopped/not-installed.
3. **Every command is idempotent**: starting a running service and
   stopping a stopped one are no-ops from the user's point of view.
4. **Uninstall never touches data.** Only plists/units are removed;
   sessions, memory and tasks under `<home>` are untouched.

## Tray (`vakcoder-tray`)

- Colour-coded dot: green both-up, amber degraded, red down, grey not
  installed. Polls every 3 s on a worker thread; UI updates via winit user
  events; menu rebuilt per refresh.
- Watchdog (persisted in `~/.vakcoder/tray.json`): when a previously
  running service disappears, restarts it at most once per minute and
  posts a system notification.
- Menu actions map to encoded ids (`(slot << 8) | action`) forwarded as
  user events so all mutation runs on the UI thread.

## Token pinning

Bridges must survive gateway restarts without human help:
`VAKCODER_GATEWAY_TOKEN` (from `~/.vakcoder/.env`) overrides the random
per-process bearer token. The value is honoured verbatim and never logged;
when absent, behaviour is unchanged (fresh token printed once).

## Surfaces

| Surface | Access |
|---|---|
| Tray | native menu |
| TUI | `/services [start\|stop\|restart] [gateway\|telegram]` |
| Desktop | Settings ▸ Services (5 s poll) and Learning pages |
| HTTP | `GET /ops/status`, `POST /ops/{gateway\|telegram}/{action}` |

## Test isolation convention

Fixtures that drive gateway turns must be hermetic against the developer's
global config (which may legitimately enable `reflection = true`): write a
project `.vakcoder/config.toml` disabling learning flags and construct
`Core::new_with_trust(.., true)`. See any `tests/*.rs` spawn helper using
the HERMETIC pattern.
