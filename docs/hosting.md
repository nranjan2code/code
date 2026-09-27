# Hosting Vakyartha

The architectural bet of this project: **the core is headless and runs
anywhere; the terminal, desktop and chat bridges are clients.** This guide
covers local services and a headless host.

## Topology

```
┌────────────── any machine ──────────────┐     ┌── your phone ──┐
│ vak serve --gateway --port 8901   │◀────│ Telegram bot    │
│ ▲ bearer token (pinned)                 │     │ (bridge process │
│ └── sessions/, memory/, tasks/ live here │     │  can run anywhere)│
└──────────────────────────────────────────┘     └────────────────┘
```

- Durable gateway services use the canonical default workspace (`~/vak-home`);
  an admitted chat can have an explicit workspace binding.
- Bridges need network access to their channel API
  and to your gateway — they do NOT need to be on the same machine.
- TUI/desktop connect locally as always; remotely via SSH tunnel or
  tailnet pointing at the same port.
- The **web client** is served by the same process at `/app` — the same
  workspace client the desktop app ships, in a browser
  (docs/design/48-web-client.md). See "Reaching the web client" below.

## Managed local install

```bash
scripts/build.sh
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
- `VAK_GATEWAY_TOKEN` is pinned automatically on first activation
  (`ensure_gateway_token`) — nothing to set by hand. Set channel
  credentials (bot tokens, provider keys) through the admin/Settings API
  (`PUT /config/key`, the gateway bot endpoints) before starting services
  so bridges keep working across restarts; there is no file to hand-edit —
  secrets are never stored as plaintext (docs/design/44-shared-config.md,
  "Secrets Chain").
- Re-run `self install` after upgrading binaries and `self services-sync` after
  changing the served workspace or generated-unit contract.
- `self services-sync` uses the canonical default workspace for headless
  services, regardless of the directory where it is run. Use an explicit
  gateway chat binding for another workspace. Units preserve non-secret
  `HOME`; GUI launches can resolve the OS account home when it is absent.

## Secrets

All user-level secrets — provider keys, bot tokens, and
`VAK_GATEWAY_TOKEN` — live in the canonical Shared secret scope, resolved
through `vak_config::credentials` to an OS-native secret service (macOS
Keychain / Linux Secret Service) or, when
none is reachable (the common case for a headless server with no D-Bus
session), an AES-256-GCM encrypted-file fallback under the shared data
home — never a plaintext file (docs/design/44-shared-config.md, "Secrets
Chain"). This sits beside the Shared config layer at
`~/vak-home/.vak/config.toml`. Sessions, ledgers, and gateway state live
separately under the platform data home (`~/Library/Application Support/vak`
on macOS, `~/.local/share/vak` on Linux).

Nothing secret is written to config.toml, generated units, the repo, or logs. Generated bearer tokens are
printed only to an interactive terminal; Telegram HTTP errors omit Bot API URLs.

## Reaching the web client

`vak serve` serves the full workspace client at `/app`. Three ways in,
in increasing order of how much you are taking on:

**1. Loopback (default).** Nothing to configure.

```bash
vak serve --port 8901
open http://127.0.0.1:8901/app
```

**2. SSH tunnel — the recommended way to use a headless box.**

```bash
ssh -N -L 8901:127.0.0.1:8901 you@box     # then open http://127.0.0.1:8901/app
```

Zero new attack surface: the server stays on loopback, `Host` is
`localhost`, SSH does the crypto, and the terminal keeps working. Prefer
this unless you specifically need a browser that cannot tunnel.

**3. A real hostname.** Requires opting in, because a non-loopback bind
exposes the agent — including its tools — to whoever can reach the port.

```toml
[server]
bind = "127.0.0.1"                        # behind a TLS proxy; or a tailnet IP
trusted_hosts = ["vak.example.com"]       # exact names; wildcards are refused
public_url = "https://vak.example.com"    # enables Secure cookies
session_ttl_hours = 24                    # shorter than the 168h default

[server.web]
terminal = false                          # a shell over HTTP is remote code execution
```

`vak serve` **refuses to start** on a non-loopback bind with no
`trusted_hosts`, rather than starting and then rejecting every request
with 421. Put a TLS-terminating proxy in front (Caddy, nginx) and make
sure it passes `Host` and sets `X-Forwarded-Proto`.

Sign in with the server's token — the one it prints on startup, or your
`VAK_GATEWAY_TOKEN`. It is exchanged once for an HttpOnly cookie and is
never stored in the page.

## Docker (the headless Linux image)

```bash
export VAK_GATEWAY_TOKEN="$(openssl rand -hex 32)"
export VAK_WORKSPACE=/path/to/your/project
docker compose -f docker/compose.yaml up --build
# then open http://127.0.0.1:8901/app
```

The image is `vak serve` and nothing else: the web client at `/app`, the
operations console at `/admin`, one port. It runs as a non-root user, the
binaries stay root-owned (a compromised agent must not be able to rewrite
its own executable), and it contains **no Node.js** — the admin and client
bundles are committed precisely so a headless host builds the server
without a JavaScript toolchain.

Two volumes matter:

- `/workspace` — the project the agent acts on. Bind-mount real code here.
- `/home/vak/.local/share` — sessions, memory, tasks, commitments, audit
  logs. A **named volume**, because this is append-only history rather than
  a cache: losing it loses the receipts.

The compose file publishes to `127.0.0.1` deliberately. `"8901:8901"` would
bind every interface and hand an agent that can run commands to anyone who
can reach the host. Serving a real hostname is the separate, deliberate step
below.

Inside a container `vak serve` binds `0.0.0.0`, because that is the only
address the published port can reach — and unlike on a host, it does not
demand `trusted_hosts` first, since `docker run -p` is already the explicit
act that decides reachability. The `Host` check itself does not relax: DNS
rebinding is defended identically inside a container.

## Security posture

1. Bearer token on every route except `/health` and the client shell.
2. Bind to loopback by default. For remote access use a tunnel
   (`ssh -L`, Tailscale/WireGuard) or, deliberately, `--host` /
   `[server] bind` with `trusted_hosts` set and a firewall in front.
3. `Host` headers are pinned: loopback names, plus exactly what
   `trusted_hosts` lists. This is the DNS-rebinding defence — a page you
   visit can resolve its own domain to 127.0.0.1 and `SameSite` will not
   save you, but a pinned `Host` will.
4. Cross-origin mutations are refused even when they carry a valid
   session cookie.
5. The web terminal is off by default, and loopback-only even when on.
   Everything else the client can reach is permission-gated; a shell is
   not.
6. `[gateway]`, `[sandbox]` and `[server]` are privileged config sections
   — untrusted repositories cannot enable remote execution, pick sandbox
   backends or images, or widen network exposure.
7. Unattended turns auto-deny approval gates unless you configure
   `approvals = "forward"` with an approver surface (docs/design/
   22-gateway.md G2).
8. Use `vak backup --help` for a supported backup. The data home contains
   durable sessions and bindings; copying only a project directory is not
   a complete backup.

## Updating

```bash
git pull
scripts/build.sh
/Applications/Vak.app/Contents/MacOS/vak self services-sync
```

Before updating an existing data home, read the release notes for its
supported baseline and run `vak self verify` after installation.

## Troubleshooting

| Symptom | Check |
|---|---|
| bridge replies "(gateway unreachable)" | gateway down or token mismatch — `vak open admin --print` prints the pinned `VAK_GATEWAY_TOKEN` as a loopback-only link; compare it against what the bridge is configured with |
| replies "(aborted)" | Inspect the current run and gateway logs at `~/Library/Logs/vak/gateway.log` (macOS) or `~/.local/state/vak/logs/gateway.log` (Linux) |
| tool calls denied on phone | expected in default deny mode; configure an approver surface or use TUI/desktop for escalations |
| model errors | `/health` shows effective provider/model plus provenance/revision; keys live in the Shared secret scope (docs/design/44-shared-config.md, "Secrets Chain"), not a file — use the Settings UI or `PUT /config/key` to check/change them |
| MCP server "spawn failed" / dies at handshake | under service managers PATH is minimal: use the absolute interpreter path (`which npx`) in `[mcp.servers.*].command`; network-client tools also need `network = true` |
| Tavily/web search denied on phone | add `allow = ["+mcp(tavily/*)"]` to trusted config — scoped to that server |

Telegram bridges are single-consumer per bot token: local duplicates fail fast via `$VAK_HOME/locks`, cross-machine rivals put the local bridge into hot-standby with automatic takeover (`docs/design/22-gateway.md` § Telegram bot ownership).
