# Hosting your vak core

The architectural bet of this project: **the core is headless and runs
anywhere; TUI, desktop and chat bridges are just clients.** This guide makes
that concrete — from laptop LaunchAgent to a $5 VPS.

## Topology

```
┌────────────── any machine ──────────────┐     ┌── your phone ──┐
│ vak serve --gateway --trust :8901  │◀────│ Telegram bot    │
│ ▲ bearer token (pinned)                 │     │ (bridge process │
│ └── sessions/, memory/, tasks/ live here │     │  can run anywhere)│
└──────────────────────────────────────────┘     └────────────────┘
```

- One gateway per workspace (`--trust` the workspace once).
- Bridges (Telegram today; more later) need network access to the Bot API
  and to your gateway — they do NOT need to be on the same machine.
- TUI/desktop connect locally as always; remotely via SSH tunnel or
  tailnet pointing at the same port.

## Managed local install

```bash
scripts/release.sh
target/release/vak self install --force
cd /path/to/the/workspace-the-gateway-should-serve
/Applications/Vak.app/Contents/MacOS/vak self services-sync  # macOS
# ~/.local/share/vak/bin/vak self services-sync              # Linux
```

- `self install` places one verified manifest of the CLI, desktop, and workers
  in the managed platform prefix; services never run from `target/`.
- macOS → LaunchAgents `com.vak.gateway` / `com.vak.telegram`
  (KeepAlive + RunAtLoad; survives reboot & crashes).
- Linux → systemd user units `vak-gateway.service` /
  `vak-telegram.service` (`systemctl --user ...`, enable lingering for
  boot start: `sudo loginctl enable-linger $USER`).
- The desktop app gets a unit too — `com.vak.desktop` /
  `vak-desktop.service` — so the menu-bar icon comes back at login and
  survives a reboot. It starts as `vak-desktop --tray`: menu-bar icon
  only, no window until you open one (Dock, Finder, or the tray's
  `Open Vak`). Unlike the headless pair it is **not** kept alive, so the
  tray's `Quit Vak` actually quits; the next login or `self services-sync`
  brings it back.
- On a headless server there is no `vak-desktop` binary in the release,
  and the unit is skipped. If you installed a desktop build on a machine
  with no display, drop just that one:
  `launchctl bootout gui/$UID/com.vak.desktop` /
  `systemctl --user disable --now vak-desktop.service` — re-running
  `self services-sync` will recreate it.
- Put `VAK_GATEWAY_TOKEN` and channel credentials in `data_home()/.env`
  before starting services so bridges keep working across restarts.
- Re-run `self install` after upgrading binaries and `self services-sync` after
  changing the served workspace or generated-unit contract.
- `self services-sync` records the workspace directory in each generated unit;
  run it from the workspace that the gateway should serve. This keeps the
  gateway's provider/model config and project `.env` aligned with that
  workspace instead of inheriting launchd/systemd's default directory. Units
  also preserve non-secret `HOME`, while the shared resolver falls back to the
  OS account home for GUI launches that omit it.

## Secrets

All user-level secrets live in `data_home()/.env` (0600)—
`~/Library/Application Support/vak/.env` on macOS or
`~/.local/share/vak/.env` on Linux—including provider keys,
`TELEGRAM_BOT_TOKEN`, and `VAK_GATEWAY_TOKEN`. Nothing secret is written
to config.toml, generated units, the repo, or logs. Generated bearer tokens are
printed only to an interactive terminal; Telegram HTTP errors omit Bot API URLs.

## Security posture

1. Bearer token on every route except `/health`.
2. Bind to loopback by default. For remote bridges use a tunnel:
   `ssh -R` reverse tunnel, Tailscale/WireGuard, or bind
   `--host` behind a firewall that allows only your bridge's IP.
3. `[gateway]` and `[sandbox]` are privileged config sections — untrusted
   repositories cannot enable remote execution, pick sandbox backends or
   images.
4. Unattended turns auto-deny approval gates unless you configure
   `approvals = "forward"` with an approver surface (docs/design/
   22-gateway.md G2).
5. Backups = copy `<home>` (the data home: `~/Library/Application Support/vak` on macOS, `~/.local/share/vak` on Linux): sessions, memory,
   tasks, bindings are all plain files.

## Updating

```bash
git pull && cargo build --release -p vak
target/release/vak self install --force
cd /path/to/served/workspace
/Applications/Vak.app/Contents/MacOS/vak self services-sync
```

Sessions and memory are append-only JSONL/markdown — upgrades require no
migration.

## Troubleshooting

| Symptom | Check |
|---|---|
| bridge replies "(gateway unreachable)" | gateway down or token mismatch — compare `VAK_GATEWAY_TOKEN` in `.env` vs the gateway's launchd environment |
| replies "(aborted)" | pre-0.3.0 bug; upgrade. Also check `~/Library/Logs/vak/gateway.log` (macOS) or `~/.local/state/vak/logs/gateway.log` (Linux) |
| tool calls denied on phone | expected in default deny mode; configure an approver surface or use TUI/desktop for escalations |
| model errors | `/health` shows effective provider/model plus provenance/revision; keys live in `data_home()/.env` |
| MCP server "spawn failed" / dies at handshake | under service managers PATH is minimal: use the absolute interpreter path (`which npx`) in `[mcp.servers.*].command`; network-client tools also need `network = true` |
| Tavily/web search denied on phone | add `allow = ["+mcp(tavily/*)"]` to trusted config — scoped to that server |

Telegram bridges are single-consumer per bot token: local duplicates fail fast via `$VAK_HOME/locks`, cross-machine rivals put the local bridge into hot-standby with automatic takeover (`docs/design/22-gateway.md` § Telegram bot ownership).
