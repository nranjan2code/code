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
2. **The service manager owns process liveness.** A registered unit with a
   live PID is running; HTTP health describes application readiness but may
   not authorize a restart. A busy event loop or transient health timeout
   must never make a tray watchdog send `SIGTERM` to a healthy process.
   HTTP health is only the fallback for an unmanaged gateway.
3. **Every command is idempotent**: starting a running service and
   stopping a stopped one are no-ops from the user's point of view.
4. **Uninstall never touches data.** Only plists/units are removed;
   sessions, memory and tasks under `<home>` are untouched.

## Tray (owned by `vak-desktop`)

The desktop process owns the menu-bar icon and the visible window. Close
hides the main window; Dock/Finder re-open, tray Open, and repeated
launches all reveal and focus that same window.

- A left click and the `Open Vak` menu item reveal and focus the main
  window; `Quit Vak` ends the desktop process explicitly.
- `Open Admin Console` opens the distributed admin shell at its overview;
  `Open Operations Center` opens the same authenticated shell directly at
  `#/operations`. Both resolve the configured gateway port and reuse the
  pinned token when one exists.
- Closing the main window hides it, preserving the local embedded backend and
  the tray until the user explicitly quits. Durable gateway and Telegram work
  remains independently supervised by the platform service manager.
- The tray watchdog reports state transitions. It never restarts managed
  services; launchd/systemd `KeepAlive` is the sole automatic recovery owner.

### `com.vak.desktop`: the tray is a durable service too

The desktop app is the surface that starts, stops and watches everything
else, so it is the one thing that must not disappear at a logout. It has
its own unit (`com.vak.desktop` / `vak-desktop.service`) alongside the
headless pair, generated the same way. It differs from them in four ways,
each deliberate:

| | gateway/telegram | desktop |
|---|---|---|
| `RunAtLoad` / `WantedBy` | yes | yes — the whole point |
| `KeepAlive` / `Restart` | `true` / `always` | `false` / `no` |
| `WorkingDirectory` | the canonical default workspace (`~/vak-home`) | the account home |
| `LimitLoadToSessionType` | absent | `Aqua` |
| shipped always | yes | only when the build produced `vak-desktop` |

**`LimitLoadToSessionType = Aqua` is what makes the icon appear at all.**
Without it launchd loads the job into the plain background `gui/<uid>`
domain: the process starts and stays up, but it never checks in with
LaunchServices and is given no WindowServer (CGS) connection, so it can
place no status item — a unit that "works" and shows nothing.
`lsappinfo` names the difference exactly: `bundle path=[NULL]`,
`Arch=!!none`, `!cgsConnection` without the key, versus
`type="Foreground"` with a real session token once it is set. The
headless units must *not* have it — they have no UI to place, and
pinning them to Aqua would stop them loading in a session with no
logged-in GUI user.

**KeepAlive is off** because the tray's `Quit Vak` calls `app.exit(0)`:
under KeepAlive launchd would relaunch a second later and Quit would
visibly not quit. Rule 1 above still holds — the manager is still the only
supervisor — it is just told, correctly, that this process exiting is
usually a decision rather than a fault. A crashed GUI app respawning in a
loop the user cannot see is worse than one that stays down until the next
`services-sync` or launch.

**The working directory is the account home**, not a workspace: the
desktop app picks its project in its own UI (`ProjectGate` /
`recent_projects` in `desktop.json`), so recording a workspace would
record one it never honours.

**It is skipped when `vak-desktop` was not built.** A unit exec'ing a
missing path is worse than no unit; on a headless server it could only
ever fail.

### Why this is not the `com.vak.tray` bug again

The retired `com.vak.tray` broke Finder double-click: the bundle's
`CFBundleExecutable` and a launchd service were the same binary, so macOS
answered a double-click by activating the already-running process instead
of launching one — and that process was window-less and had no reopen
handling, so nothing visible happened at all. `com.vak.desktop`
reproduces the structure (same binary, both identities) but not the
failure, because both halves of the cause are gone:

1. `RunEvent::Reopen` reveals and focuses the main window, so an
   activation of the running instance does what the double-click asked.
2. If macOS launches a second process instead,
   `tauri_plugin_single_instance` hands its argv to the live one, which
   reveals the window — and no second menu-bar icon appears.

Exactly one of those two paths runs for any given launch, and both end
with a window on screen. The one exception is a hand-off carrying
`--tray`: that is the LaunchAgent firing at a login where the app is
already up, nobody asked for a window, and the callback stays silent.

The hand-off also costs a process. The plugin calls
`std::process::exit(0)` from inside its own setup, before this app
writes anything, so a login hand-off used to leave `desktop.log` empty
and the job reporting `last exit code = 0` — indistinguishable from a
unit that never ran. `main` therefore writes one line naming its pid and
argv before the builder runs; an empty `desktop.log` now genuinely means
the binary never started. The launchd instance is a normal (not
`Accessory`, not `LSUIElement`) app so it keeps a Dock icon and its
ordinary LaunchServices registration; the bundle-wide `LSUIElement` that
made the old tray invisible to all of this is still forbidden
(`install::bundle`).

### `--tray`

`vak-desktop --tray` starts with the menu-bar icon and no window. The
main window is created hidden (`visible: false` in `tauri.conf.json`) and
revealed in `setup` on an ordinary launch, so nothing flashes on screen
in either mode. Only the unit passes `--tray`; every other entry point —
double-click, Dock, `Open Vak`, a handed-over second launch that did not
carry the flag — reveals the window. Without it, login-launching the app would throw a 1440×900 window
at the user on every boot, a worse regression than the missing
persistence it fixes.

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
| Tray | native menu — Open Admin Console or Open Operations Center |
| TUI | `/services [start\|stop\|restart] [gateway\|telegram]` |
| Desktop | Settings ▸ Services (5 s poll) and Learning pages |
| HTTP | `GET /ops/status`, `GET /ops/diagnostics`, `GET /ops/center`, `GET /ops/incidents`, `GET /ops/actions`, `GET /ops/outbox`, `POST /ops/{gateway\|telegram}/{action}`, `POST /ops/outbox/{job_id}/replay` |

### Operations Center projection

`GET /ops/center` is the read-only control-plane snapshot used by the admin
console. It combines the doctor report (including a separate `posture` signal
so the legacy `/health` `status = ok` contract remains intact), platform
service state, gateway bindings and approval count, warm CorePool entries,
live session handles, scheduled tasks and next-fire markers, security events,
and the durable delivery outbox. Incidents are derived only from those
sources: failed checks, blocked approvals, service drift, or pending/dead
delivery jobs. An unreadable outbox is itself a critical incident rather than
an empty queue. The incident projection is reconciled into the append-only
`<sessions_home>/operations/incidents.jsonl` ledger: repeated observations
are grouped by fingerprint, disappearance records a resolution, and a later
reappearance reopens the same causal record. `GET /ops/incidents` exposes the
folded history for audit consumers.

`GET /ops/outbox` exposes metadata for all persisted jobs. The replay endpoint
re-enters the same serialized renderer and adapter path as the background
replay worker; delivered jobs are rejected and a failed replay remains
durable with its incremented attempt/error state. Service actions return a
conflict on a manager failure and append an audit event; the HTTP probe never
starts a competing supervisor.

Every mutating operations endpoint returns an operation receipt with a
stable id, before/after manager or outbox state, and an explicit
`verified`/`pending`/`failed` verification result. Receipts are also appended
to `<sessions_home>/operations/actions.jsonl` and surfaced in the center;
they remain useful when a manager accepts a request but takes time to reach
the target state.

### Operations Center navigation contract

The embedded admin shell exposes one stable control-plane route:
`/admin#/operations`. It is the Operations area within the six-area admin
shell, with the following operator views rather than a separate monitoring
product:

| Area | Route | Evidence it owns |
|---|---|---|
| Posture | `#/operations` | server health, topology, uptime, and first-hop posture |
| Work | `#/operations/work` | live runs, approval gates, and scheduled work |
| Runtime | `#/operations/runtime` | service-manager state, CorePool occupancy, action receipts |
| Channels | `#/operations/channels` | bindings, route provenance, durable delivery and replay |
| Automations | `#/operations/automations` | task cadence, workspace, model pin, and last run |
| Providers / spend | `#/operations/providers` | effective route, provenance, sandbox, and security evidence |
| Incidents | `#/operations/incidents` | correlated open/resolved history and Doctor checks |

The workspace and time-window selectors are URL parameters (`workspace` and
`time=live|1h|24h|7d|custom`). Internal links preserve both parameters, so a
bookmark always reopens the same operational scope. Detail routes descend
from the same namespace — for example
`#/operations/work/runs/<session>`,
`#/operations/channels/<target>`,
`#/operations/channels/delivery/<job>`, and
`#/operations/incidents/<incident>` — and each ends at raw ledger, receipt,
outbox, incident, or manager evidence. A missing record is rendered as an
explicit historical/unavailable state; the UI never invents a green value.

Service-manager probes that use a blocking client execute on a blocking worker
before the response is assembled. This keeps `/ops/status`, `/ops/diagnostics`,
and `/ops/center` safe under Tokio and prevents shutdown-time runtime panics.

The listener's actual port is carried into the server state and all service
health probes. Generated gateway and Telegram units resolve their port at sync
time, so a non-default `VAK_PORT`/listener cannot silently leave the console or
bridge pointed at `8901`.

The desktop and menu-bar tray use that same recorded listener port and pinned
token when opening `/admin` or `/admin#/operations`; the native launch path is
therefore bound to the exact server instance the operator is already watching.

## Test isolation convention

Fixtures that drive gateway turns must be hermetic against the developer's
global config (which may legitimately enable `reflection = true`): write a
project `.vak/config.toml` disabling learning flags and construct
`Core::new_with_trust(.., true)`. See any `tests/*.rs` spawn helper using
the HERMETIC pattern.
