# 34 — Base + Addons architecture

## Axioms

1. **One headless core is THE server.** The gateway process owns all
   time-based effects: cron, watchdog, outbox replay, index rebuilds,
   channel delivery, approval forwarding. No other surface may schedule,
   replay, rebuild, or bridge.

2. **Every surface is a thin client.** TUI, desktop, Telegram bridge,
   exec, plan, flow, eval — all connect to the base over HTTP+SSE. They
   read state, send commands, and render. They never own state.

3. **One process owns time.** Only the gateway fires cron jobs, rebuilds
   the FTS index, replays the outbox, delivers to channels, and forwards
   approval gates. A local-only `serve` process may do these if no
   gateway is running.

4. **State is server-side only.** Sessions, store.db, memory files, tasks,
   checkpoints, approvals, channels, provider keys, config, and
   permissions live exclusively under the data home. Clients keep only
   presentation preferences.

## Milestones delivered (M0–M3)

### M0 — Concurrency floor

| Item | File | Detail |
|---|---|---|
| Singleton duties gated | `vak-server` | Cron, replay, rebuild guarded behind `force_gateway` flag. Only the `serve --gateway` process runs time-based effects. |
| SQLite busy_timeout | `vak-store` | `busy_timeout=5000` set on connection; prevents SQLITE_BUSY under concurrent reads. |
| Transactional FTS import | `vak-store` | `rebuild_index` wrapped in `BEGIN IMMEDIATE` / `COMMIT`; no partial FTS state on crash. |
| Gateway flock | `vak-server` | `libc::flock` on `data_home/locks/gateway.lock`; prevents multiple gateway processes. |
| Unique `.env.tmp` | `vak-config` | `upsert_env_file` / `remove_env_file` use `.env.tmp.<pid>` to avoid races. |
| Dead code removed | `vak-config`, `vak-server`, `vak-tray`, `vak-desktop` | `migrate_legacy_home`, `LEGACY_HOME_SUFFIX` removed; `layout_check` simplified. |

### M1 — Identity & discovery

| Item | File | Detail |
|---|---|---|
| Token bootstrap | `vak-server` | On first gateway boot, generates `vk_*` token, persists to `data_home/.env`. Reused on subsequent boots. |
| Runtime file | `vak-server` | Writes `data_home/runtime/gateway.json` (pid, token, addr, started_at, gateway=true). Cleaned on SIGINT+SIGTERM. |
| Doctor topology | `vak-core` | Reads `runtime/gateway.json`, checks PID liveness via `kill -0`, reports "gateway running (pid, addr)" or "no gateway running locally". |

### M2 — Connect command

| Item | File | Detail |
|---|---|---|
| Config section | `vak-config` | `[connect]` in `FileConfig` / `Config`: `url`, `token`, `profile`, `profiles.*` (name → url+token). |
| Connect subcommand | `vakcoder` | 5-step resolution: explicit `--url`/`--token` → `--profile` → `[connect]` config → `runtime/gateway.json` → onboarding prompt (tty) or error (non-tty). |
| Onboarding | `vakcoder::connect` | TTY: interactive prompts for URL + token. Non-tty: typed error with guidance. |
| Save | `vakcoder::connect` | `--save` persists resolved connection back to `~/.config/vakcoder/config.toml`. |
| Token masking | `vakcoder::connect` | Token displayed as first 8 chars + `…` in output. |

### M3 — Multi-project base

| Item | File | Detail |
|---|---|---|
| Per-session cwd | `vak-core`, `vak-server` | `create_session` endpoint accepts optional `{"cwd": "path"}` in JSON body. `Core::start_session_in(cwd)` stamps the session header with the workspace. Without body, falls back to `Core.cwd`. |
| Cross-project listing | `vak-server` | `list_sessions` scans all hash directories under `sessions/`, not just `Core.cwd`'s hash. Sessions from any project are visible. |

## Connection model

```
┌────────────── any machine ──────────────┐
│ vakcoder serve --gateway --trust :8901  │
│   owns: sessions/, memory/, tasks/      │
│   writes: runtime/gateway.json          │
│   enforces: flock, busy_timeout,        │
│             singleton duties            │
└─────────────────────────────────────────┘
         ▲            ▲            ▲
         │            │            │
   vakcoder tui  vakcoder exec  Desktop
    (thin client) (thin client) (thin client)
```

Client resolution order:
1. Explicit `--url` / `--token` flags
2. `--profile` from `[connect]` config
3. `[connect]` defaults from user config
4. `data_home/runtime/gateway.json` (local, same machine)
5. Interactive prompt (tty) / typed error (non-tty)

## Ownership contract

| What | Where it lives | Who writes |
|---|---|---|
| Sessions (JSONL trees) | `data_home/sessions/<hash>/` | Base only |
| FTS index | `data_home/store.db` | Base only (rebuilds) |
| Memory (MEMORY.md, USER.md) | Project workspace or data_home | Agent via base |
| Tasks / cron | `data_home/tasks/` | Base only |
| Checkpoints | `data_home/sessions/<hash>/checkpoints/` | Base only |
| Approvals | Ephemeral (in-flight) | Base mediates |
| Channels / outbox | `data_home/channels/` | Base only |
| Provider keys | `data_home/.env` | Base (via config endpoints) |
| Config | `~/.config/vakcoder/config.toml` | Client or base |
| Presentation prefs | Client-local only | Client only |

## Concurrency rules

- **busy_timeout=5000**: SQLite connection sets `PRAGMA busy_timeout=5000`
  on open. Prevents `SQLITE_BUSY` under concurrent reader contention.
- **Transactional FTS rebuild**: `rebuild_index` uses
  `BEGIN IMMEDIATE` / `COMMIT` so a crash mid-import leaves the old
  index intact (WAL mode ensures readers are never blocked).
- **Gateway flock**: `libc::flock(fd, LOCK_EX|LOCK_NB)` on
  `data_home/locks/gateway.lock`. If the lock is held, the second
  process exits with an error. Lock is released on process exit (kernel
  guarantee).
- **Unique env tmp**: `upsert_env_file` / `remove_env_file` write to
  `.env.tmp.<pid>` then rename, avoiding races between concurrent
  config mutations.
- **Force gateway gating**: cron, replay, rebuild, outbox, and channel
  delivery are skipped unless `force_gateway` is true (set by
  `serve --gateway`). A plain `serve` process is safe for local
  development without affecting scheduled work.

## Token lifecycle

1. **First boot**: gateway generates `vk_<uuid>`, writes to
   `data_home/.env` as `VAKCODER_GATEWAY_TOKEN`.
2. **Subsequent boots**: reads existing token from env.
3. **Runtime file**: gateway writes `data_home/runtime/gateway.json`
   with `{pid, token, addr, started_at, gateway: true}`.
4. **Client discovery**: `connect` reads `runtime/gateway.json` for
   local connections; PID liveness checked via `kill -0`.
5. **Shutdown**: SIGINT / SIGTERM handler removes `runtime/gateway.json`
   before exit.

## M4.2 — TUI thin-client rewrite (delivered)

The 5.8k-line `app.rs` god-file is gone. The TUI now holds zero agent
state: it renders what the base reports over HTTP+SSE and persists only
presentation prefs locally (`[ui]` tables via `vak-config`).

| Module | Role |
|---|---|
| `app.rs` (~1.7k) | `App` struct + `tokio::select!` loop: SSE events, keyboard, turn completion, idle tick |
| `data.rs` | The only place that talks HTTP: typed wrappers over every `vak-client` call, poison-tolerant caches, custom-command expansion |
| `events.rs` | SSE → transcript rendering (delta+snapshot consumers), diff/args helpers |
| `commands/{session,config,memory,system}.rs` | All ~36 slash commands against `ClientData`, no filesystem scanning |
| `state.rs` / `modals.rs` / `pickers.rs` / `transcript.rs` | Shared UI types + rendering |
| `prefs.rs` | Local-only theme/keymap/composer/bell/a11y persistence with full TOML upsert |
| `inbox.rs` / `tasks.rs` | Inbox attention layer; cron/watchdog/proposal management |

Server additions for parity: `GET /config/commands`, `GET /tools`,
`GET /breaker`, `POST /sessions/{id}/compact`,
`POST|GET /config/sandbox`. `vakcoder tui` connects through
`connect::discover` like every other surface.

## Status

M0–M3, M4.2, and M4.3 delivered. Remaining milestones:

| Milestone | Scope |
|---|---|
| M4 remainder | Desktop SPA audit against `vak-client` types |
| M5 | Hardening: version handshake ✅, TLS guard ✅, fan-out tests ✅ — remaining: docs sweep |
| M5 | Hardening: TLS guard (http:// non-loopback), version handshake, fan-out tests, docs |
