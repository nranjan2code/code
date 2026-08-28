# 28 — Operations: service control plane

Design notes behind `vak-ops`, the desktop-owned tray, and the `/ops` HTTP routes
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

## Tray (owned by `vak-desktop`)

The desktop process owns the menu-bar icon and the visible window. It never
runs as a separate launchd service: a tray executable launched independently
from inside the same `.app` bundle can cause LaunchServices to activate that
background-only process when the user opens Vak. Close hides the main
window; Dock/Finder re-open, tray Open, and repeated launches all reveal and
focus that same window. Gateway and Telegram remain the only durable services.

- A left click and the `Open Vak` menu item reveal and focus the main
  window; `Quit Vak` ends the desktop process explicitly.
- Closing the main window hides it, preserving the local embedded backend and
  the tray until the user explicitly quits. Durable gateway and Telegram work
  remains independently supervised by the platform service manager.

## Token pinning

Bridges must survive gateway restarts without human help:
`VAK_GATEWAY_TOKEN` (from the user `.env`) overrides the random
per-process bearer token. The value is honoured verbatim and never logged;
when absent, a fresh token is printed only to an interactive terminal and is
suppressed in service logs. Generated service units carry only non-secret
operational environment such as `HOME`; credentials remain file-loaded by the
recipient process.

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
project `.vak/config.toml` disabling learning flags and construct
`Core::new_with_trust(.., true)`. See any `tests/*.rs` spawn helper using
the HERMETIC pattern.
