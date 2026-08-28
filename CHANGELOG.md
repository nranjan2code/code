# Changelog

## Unreleased

### Channel onboarding: live allowlist store + Admin UI approvals

`gateway.chat_allowlist` is no longer a config-file-only, restart-required
setting. Unknown inbound chats now land as a reviewable **pending** entry
in a new schema-versioned, live-reloadable store
(`<sessions_home>/gateway/allowlist.json`, sibling to `bindings.json`)
instead of a flat rejection — the operator has a forward path from "I see
it was rejected" to "let it through" that doesn't require hand-editing
`.vak/config.toml` and bouncing the gateway process. A repeat message from
an already-pending chat is logged lightly instead of spamming a fresh
security event each time. `chat_allowlist_open = true` still bypasses the
store entirely, as before. Existing `gateway.chat_allowlist` entries are
imported into the new store once, on first load, as `allowed`; after that
the store is authoritative, matching `bindings.json`'s relationship to
route overrides.

New admin API routes: `GET /admin/api/gateway/allowlist`, `POST
/admin/api/gateway/allowlist/{key}/approve` (body: `{workspace?, route?}`
— approving always makes the effective workspace explicit in the
response, even when the caller didn't supply one, so it's never a silent
inherited default), `POST /admin/api/gateway/allowlist/{key}/deny`, and
`DELETE /admin/api/gateway/allowlist/{key}` (revoke an allowed entry).
Every transition is recorded via `vak_core::security_events` under new
`chat_pending` / `chat_approved` / `chat_denied` / `chat_revoked` kinds.

The Admin UI's Gateway page gains a "Pending channels" panel (chat key,
first-seen text, arrival time, Approve/Deny — approving lets the operator
confirm or edit the workspace and optionally pin a provider/model) above
"Registered channels," and registered channels now show an allowlist
status chip (allowed/pending/denied) with a Revoke action for allowed
entries.

**Phase 2: multi-tenant Core pool.** An allowlist entry's `workspace`
field used to only pick provider/model — the gateway ran every channel
through its own single `Core`, fixed at process start, regardless of what
workspace an entry named. `GatewayState` now holds a `CorePool`
(`crates/vak-server/src/core_pool.rs`): a canonical-workspace-path →
lazily-started `Core` map. The gateway's own default workspace is the
pool's permanent, never-evicted entry; on inbound dispatch, once the
allowlist resolves an entry as `allowed`, the gateway looks up (or lazily
starts) that entry's own workspace `Core` via `Core::new_with_trust` —
the exact same trust/permission/sandbox resolution a local `vak` run in
that workspace gets — and routes the turn's session, ledger, and turn
execution through it instead of the gateway's own `Core`. Pooling never
grants a channel more access than a local session in that workspace
already has; the allowlist approval step is still what gates a channel
reaching a workspace at all.

Pooled Cores for non-default workspaces are idle-evicted after
`[gateway] core_pool_idle_secs` (default 1800 = 30 minutes) of no inbound
activity, and capped at `[gateway] core_pool_max` (default 8) concurrently
pooled Cores — over the cap, the oldest-idle non-default entry is evicted
to make room; the default workspace is never evicted. Both are new
recognized `[gateway]` config keys (unknown-keys-warn-not-fail, like every
other gateway key). `GET /admin/api/gateway/status` gains a `core_pool`
field (max, idle_secs, and the list of currently warm workspaces with
idle time) and the Admin UI's Gateway page shows a "Core pool" panel plus
a warm/cold indicator next to the workspace field in the pending-channel
approve flow, so the operator can see whether approving a workspace will
reuse a live Core or start a fresh one.

### `vak doctor --repair` and `scripts/vak.sh`

`vak doctor` gained `--repair`: it acts on the checks that have a known
mechanical fix (today: self version parity, via `self install --force`)
and re-collects the report, leaving checks with no mechanical fix
(provider auth, config warnings) for the operator — it never guesses at
those. `scripts/vak.sh <verb>` is a new thin dispatcher over
`scripts/build.sh` / `scripts/release.sh` / `vak self <verb>` / `vak
doctor`, for anyone who'd rather remember one entry point than which
script or subcommand owns a given lifecycle step; it only routes, it
never reimplements a verb.

## 0.10.0 — 2026-08-28

### Rebrand: VakCoder → Vak

The project and CLI are renamed from "VakCoder"/`vakcoder` to "Vak"/`vak`
across source, docs, config, and CI. The umbrella CLI crate moves from
`crates/vakcoder` to `crates/vak` (package and binary both `vak`); the
tray helper binary is now `vak-tray` (was `vakcoder-tray`). The Tauri
desktop app identifier moves to `dev.vak.desktop`. The on-disk config/data
layout already used `~/Library/Application Support/vak` (etc.) as of the
0.8 release with its own `~/.vak` legacy-dotdir migration
(`crates/vak-config/src/paths.rs`), so no further path migration was
needed here. Older `vakcoder`-named artifacts (release archives, prior
changelog entries) are left as historical record.

## 0.9.0 — 2026-08-28

### Telegram document attachments and inline-keyboard approvals

Telegram messages can now include a `document` (code, logs, CSVs, ...) up
to 256 KiB; it's inlined into the prompt as a fenced text block (capped
at 64 KiB decoded) rather than silently dropped or truncated. Forwarded
approval gates (`[gateway] approver = "telegram:<chat>"`) now render as
tappable inline-keyboard buttons instead of requiring a typed `yes`/`no`
— a new `TelegramAdapter` gives the gateway's async delivery path an
actual route to Telegram, which it never had before (a forwarded gate to
a Telegram approver previously failed the push and denied closed with no
adapter registered for that scheme). Button taps resolve through the
same verdict-parsing path a typed reply already used.

### Gateway chat allowlist fails closed

`gateway.chat_allowlist` now fails closed: an empty list rejects every
inbound chat with `403` instead of allowing all of them, unless the
operator explicitly sets `chat_allowlist_open = true`. Inbound channel
bridges (Telegram today; Slack/Discord later) now build their
`/gateway/inbound` payload through a new `InboundChannel` trait and
`InboundRequest::new` constructor, which rejects an empty or
placeholder-equal `chat`/`sender` so a careless new bridge cannot
silently collapse every remote user into one session. The Telegram
bridge now sends the message sender's real Telegram user id instead of
a fixed `"telegram"` placeholder.

## 0.8.12 — 2026-08-28

### Canonical service home

Generated launchd/systemd units now preserve the invoking user's non-secret
`HOME` alongside the workspace. This prevents a sanitized service-manager
environment from treating the workspace as the user home and relocating
project `.vak/config.toml` into a nested platform-data directory. The
canonical path resolver also falls back to the operating-system account home
when GUI launch environments omit `HOME`; it never falls back to the current
workspace.

## 0.8.11 — 2026-08-28

### System-wide route control plane

Provider and model are now one atomic route across Core, configuration writes,
session admission, task and heartbeat pins, desktop, gateway, and admin
surfaces. Authenticated workspace changes persist the complete pair before
hot-apply; independent long-running local processes refresh persisted defaults
when admitting a new session. Explicit scoped pins remain isolated.

The same refresh path now covers max turns, theme, MCP servers, hooks, and
permission mode. A permission change is applied only after active main/side
runs are cancelled and pending approvals denied, preserving capability
revocation across process boundaries.

Gateway bindings now use a backward-compatible, versioned record containing
the bound session, optional channel route, workspace, and route revision.
Admin-default precedence is deterministic (`channel override > workspace
default`). A frozen session that no longer matches its effective route is
reported stale and rotates to a new session on the next inbound message; the
old append-only ledger is preserved.

The admin console adds a dedicated Gateway control view with workspace default
and provenance, registered-channel creation/edit/removal, live-discovered model
selection, inherited versus overridden routes, frozen-contract comparison,
stale reasons, and explicit conversation rotation.

Non-interactive server logs no longer print generated bearer tokens, and
Telegram transport/decode failures are rendered without request URLs so bot
tokens embedded in Bot API paths cannot enter service logs.

## 0.8.10 — 2026-08-27

### Workspace-aware durable services

Generated launchd/systemd units now retain the workspace directory captured by
`self services-sync`. Gateway and Telegram deployments therefore load the
selected workspace's provider/model config and project `.env` instead of
starting from the service manager's root directory and incorrectly returning
503 for a missing provider credential.

## 0.8.9 — 2026-08-27

### Configuration audit follow-up

Profile-derived provider/model settings now report accurate provenance, and
the admin transcript projection exposes the same frozen session contract and
configuration-mismatch status as the normal session endpoint.

## 0.8.8 — 2026-08-27

### System-wide configuration authority

Provider/model settings saved through the secured configuration API now
persist atomically in the workspace configuration, report their effective
source across health, provider, and admin endpoints, and survive process
restart. Existing sessions continue to use their immutable frozen provider
and model contract, with transcript metadata exposing mismatches against
current workspace defaults. Desktop settings now describe the saved-workspace
behavior instead of implying that changes are runtime-only.

The follow-up audit also aligned the admin transcript projection with the
normal transcript contract and added profile-aware provenance reporting.

## 0.8.7 — 2026-08-27

### Desktop tray lifecycle

The macOS desktop app now owns the menu-bar tray and its full service
controls in one process. Finder/Dock activation, tray Open, and repeated
launches reveal the same window; Gateway and Telegram controls, logs, Admin
Console, watchdog, and Quit remain available from the tray.

## Unreleased

### The desktop app could never receive a single agent event

Every desktop SSE connection had always been rejected with 401, so the
chat window never received one event: runs completed and were durably
logged while the UI showed no reply, "Working" that never cleared, and
`0 in / 0 out`. Nothing appeared in the console either, because a 401 on
an `EventSource` surfaces only as a bare `onerror`.

`require_bearer` accepted an `Authorization` header or the `vak_session`
cookie. `EventSource` cannot set headers, and the desktop never performs
the `/admin/login` cookie exchange -- that is the browser console's flow.
`?token=` was the only channel it had, `openEventStream` and
`openSideStream` have always used it, and the startup banner has always
advertised it -- but the middleware never accepted it. The middleware now
matches the contract it advertises.

Found by opening the real SSE endpoint with curl and getting zero bytes
back. Three earlier releases shipped fixes for this symptom -- a
swallowed exception in the message handler, a dead stream with no
reconnect, and missing server-state reconciliation. All three were real
defects and are worth keeping, but none of them was the cause, because
none of them was ever tested against the actual event stream.

### Two menu-bar icons, and an app that opened nothing

The bundle's `CFBundleExecutable` is `vak-tray`, and
`com.vak.tray` also runs it as a launchd service with `RunAtLoad`.
Nothing guarded against both. The ordinary path -- install,
`services-sync`, then open Vak from Finder or Spotlight -- produced
two identical menu-bar icons; and once macOS began merely re-activating
the already-running app rather than spawning a new process, launching it
did nothing visible at all, because the tray only opens the chat window
once at startup.

The tray now takes a single-instance lock. A launch that finds a live
holder opens the chat window -- what launching the app actually asks for
-- and exits. Same mechanism `vak_server::telegram::InstanceLock`
already used: an O_EXCL marker plus a liveness probe on the recorded pid,
so a crashed holder leaves a marker the next launch reclaims rather than
one that wedges the menu bar until reboot. Not flock, which would need
`unsafe`; the workspace denies it. Verified all three behaviours against
real processes: a second launch adds no icon and opens the window, and a
`kill -9`'d holder's lock is reclaimed.

### Install bloat

`self install`'s bundle-asset copy (`copy_dir`) only ever adds files; it
never removes ones absent from the source. The desktop frontend's
filenames are content-hashed (Vite) and change on every rebuild, so every
reinstall left the *previous* build's JS and CSS sitting in
`Contents/Resources/assets/` alongside the new one — harmless to which
file actually gets served (`index.html` always names the current hash),
but unbounded bloat, and confusing to anyone inspecting the bundle with
no way to tell which files are actually live. `write_metadata` now clears
the hashed `assets/` subtree before copying — only that subtree, not all
of Resources, which also holds `install.json` and `Info.plist`.

### Admin console serving a blank shell

The admin console loaded to a blank dark screen with nothing in the
console — no JavaScript ever ran, so nothing had a chance to error.
Found by opening the one-click link this release added and seeing
exactly that; confirmed fixed by loading the real page and reading its
network requests and rendered content, not by inspecting code.

Three layered defects, found one at a time as each fix exposed the next:

- **The actual bug**: `crates/vak-server/src/admin_ui.rs` registered a
  route for every embedded file via `Dir::files()` — which is not
  recursive. Everything under `dist/assets/` (the JS bundle, the CSS)
  therefore had no route at all; a request for either 404'd from axum's
  router itself, before ever reaching the file lookup (which — via
  `get_entry` — was recursive and would have found them fine). Only
  `index.html`, at the top level, ever actually loaded, which is exactly
  why the failure read as "the page loads, but nothing on it does."
  Fixed by walking the embedded tree recursively when registering
  routes. A `#[tokio::test]` now drives the real `Router` `routes()`
  builds through every asset URL `index.html` references, exactly as a
  browser would — confirmed it fails with the exact defect's diagnostic
  when the non-recursive version is restored, and passes clean
  otherwise. An earlier, narrower test that only checked the assets were
  *embedded* passed throughout; embedding was never the problem.
- `vak-server` embedded a stale, mismatched build of the SPA on top of
  that: nothing told Cargo that `crates/vak-admin-ui/dist` was a build
  input, so a `cargo build` after `npm run build` regenerated it could
  reuse an incremental build of `vak-server` from before the
  regeneration, embedding an `index.html` that referenced filenames the
  embedded directory no longer had at all. Added `crates/vak-server/build.rs`
  declaring the directory a build input.
- The `vak-admin-ui` source fix for one-click login shipped without ever
  being compiled: `dist/` is committed (`vak-server` embeds it, not
  reads it live) and nothing forced a rebuild before release. `scripts/release.sh`
  now rebuilds both `vak-admin-ui` and `vak-desktop/ui` from source as
  part of the gate, before the working-tree-clean check, and fails the
  release outright — naming the exact fix — if the rebuilt
  `vak-admin-ui/dist` differs from what is committed.

Rebuilt the install, update, and release lifecycle around one owner, one
version, and verifiable artifacts. Found by exercising every command
against a real prefix rather than reading the code.

### Desktop chat replies that never appear

A desktop task could complete a full turn — correct provider, correct
model, correct reply, durably logged to the session's JSONL — and the
window would still show "Working" forever, with no reply, no error, and
nothing in the console. Confirmed the backend was never at fault by
reading the session log directly: the assistant's message was there,
timestamped, `settlement: "ok"`.

- Fixed `openEventStream`'s message handler swallowing exceptions raised
  by its own caller. `es.onmessage` wrapped both `JSON.parse(m.data)`
  *and* the call to the event handler in one `try/catch` commented as
  "ignore keep-alive/comment frames" — but that catch also silently
  discarded any exception thrown while handling a successfully parsed
  event, including `RunFinished`, the one that clears "Working" and
  reveals the reply. The connection stayed healthy throughout, so
  nothing ever looked wrong from the outside. The parse and the handler
  now have separate try/catches; a handler exception is reported via
  `console.error` with the event attached, not discarded.
- Fixed a dead event stream having no way back. `openStream`'s error
  callback was a no-op, and its own guard (`if (streams.has(id)) return`)
  meant a session whose `EventSource` had genuinely closed could never
  be reopened — every future run on that session would complete on the
  backend and never reach the UI, permanently, until the app was
  relaunched. It now clears the stale entry and retries after a short
  delay, but only once the browser's own reconnect has actually given up
  (`readyState === CLOSED`), so a transient error the browser is already
  retrying isn't torn down and duplicated.
- Neither of the above was actually the fault: verified live, in a build
  containing both fixes, against a brand-new task — same symptom, same
  correct reply already sitting in the session log. The real cause is a
  race the two fixes above don't touch: `openStream` constructs the
  `EventSource` but never awaits its connection actually opening before
  the prompt is sent, and the server's broadcast channel does not
  replay history to a subscriber that attaches after an event has
  already fired. A run that finishes before that connection is fully
  live loses `RunFinished` permanently — nothing dropped, nothing
  errored, the event simply never had a listener at the moment it was
  sent. `refreshSessions()` already polls the session list every 10s
  but only refreshed the sidebar; it never reconciled the "running" flag
  gating the header pill and `hydrate()`'s own guard (`if
  (!isRunning(id))`), so even once the server knew the run was done, the
  client had no path back to that fact without a relaunch. It now
  cross-checks every session's server-reported `running` state on each
  poll and, on a mismatch, corrects it and loads the transcript the push
  path missed — self-healing within 10 seconds instead of requiring a
  relaunch.

### Desktop credential visibility and silent run failures

Found by installing fresh, launching the desktop app, sending a message,
and getting nothing back — then reproducing the exact request against the
live gateway instead of guessing.

- Fixed the desktop app being unable to see a provider credential saved
  anywhere else. `vak-desktop` hardcoded its own data home as
  `~/.vak`; the wizard and the TUI's `/key` command save through
  `Core::set_provider_key`, which writes to the canonical home
  (`vak_config::paths::data_home()`, doc 32). Those are different
  directories, so a key saved through either path was invisible to a
  desktop launch — every run failed `Core::provider()` regardless of
  whether the user had ever configured a key. This is also why "home
  migration skipped: both ... exist" kept appearing: the desktop was
  actively writing into the legacy dir, so migration could never
  complete. `vak_home()` now resolves the same canonical home as every
  other surface.
- Fixed `/sessions/{id}/run`, `/side`, and `/bestofn` returning a bare
  503 with an empty body and nothing logged when no provider credential
  is configured. A client saw an empty response and an operator reading
  gateway.log saw nothing at all — "the agent never replied" was a
  symptom with no server-side trail. All three now return
  `{"error": "provider auth missing: set ANTHROPIC_API_KEY for provider
  'anthropic'"}` and log `[run] refused: ...` server-side. The message
  reuses `CoreError::MissingAuth`'s existing text and is deliberately a
  single `error` field, matching every other handler in this file — a
  `{"error": <code>, "detail": <message>}` shape was tried first and
  reverted because the desktop frontend's existing error handling reads
  `.error` as the human-readable string, and would have shown the user a
  machine code instead of the fix.

### Concurrency

- Fixed a self-deadlock in `Core::cache_home` that hung `cargo test
  --workspace` indefinitely. It locked `sessions_home_override` and then,
  still holding the guard, called `sessions_home()` — which locks the same
  mutex. `std::sync::Mutex` is not reentrant, so the thread wedged. The
  branch is only reached when the override is set, which production never
  does and every test fixture does; all seven tests in
  `crates/vak-server/tests/gateway.rs` blocked on it and now run in 0.35s.
- Removed the idiom that made this possible. `if let Ok(g) =
  slot.lock() && …` keeps the guard alive for the whole body, so the
  hazard is invisible at the call site. All fifteen override accessors now
  go through `Core::read_override` / `write_override`, which clone out
  under a minimal scope, so no guard is ever held across another call.
- Added regression coverage that runs each accessor on a worker thread
  with a deadline. A reintroduced deadlock fails the suite in 10s with a
  message naming the cause, rather than hanging it — a test that hangs
  reports nothing and blocks every gate behind it.
- Fixed `doctor_reports_checks_facts_and_optional_ladder`, which had
  expected four health checks since before the "install layout" check
  joined the ladder in f6131a5. Cargo runs test binaries sequentially, so
  the hang in `gateway.rs` meant this binary never ran and the stale
  assertion stayed invisible for the whole 0.8.0 cycle.

### Install and uninstall

- Fixed `self status` telling a freshly installed machine its service
  units "exec outside the managed prefix" when no unit files existed at
  all. `ServiceRow::unit_points_at_installed` is false both for a missing
  unit and for one pointing at the wrong binary, and status collapsed the
  two into the misconfiguration message — which every first install hits,
  and which sends the operator after a problem they do not have. A
  `unit_present` flag now separates them, so an unsynced install reads
  "not registered — run `self services-sync`".

- Removed the second installer. `build-install.sh` copied a Tauri bundle
  to `~/Applications/Vak.app` while `self install` managed
  `vak.app` — the same directory on a case-insensitive volume, which
  every macOS default is. The script's `rm -rf` destroyed the manifest of
  a managed install, after which `status` reported nothing installed and
  `uninstall` could not clean up. Placement is now solely `self install`;
  `scripts/build.sh` builds and hands off.
- `--prefix` is accepted by every `self` subcommand, not just `install`.
  Installing to a custom prefix previously left an install that could not
  be inspected, updated, or removed.
- Install, reinstall, and update are transactions. Every file is staged
  and verified before any is placed, and a failure restores the prior
  state — no half-installed prefix, and no window where the CLI is new
  while the tray is still old.
- Added `self verify` and `self reinstall`. The manifest (schema 2) now
  records a SHA-256 per component, so tampering and truncation are
  detected instead of assumed absent. Schema 1 manifests migrate on read.
- Install verifies its own result before reporting success, and reports
  how to put the CLI on PATH when it is not.
- Uninstall names components living outside the prefix that removing the
  prefix will not reach, and no longer treats an absent install as an
  error.

### Update

- Fixed `self update` panicking before it did anything. It builds a
  `reqwest::blocking` client inside the CLI's tokio runtime, which aborts
  with "Cannot drop a runtime in a context where blocking is not
  allowed". The transfer now runs on its own thread, the confinement the
  passive update check already used.
- Fixed version comparison being lexical. `manifest.version <= CARGO_PKG_VERSION`
  compares strings, and `"0.10.0" <= "0.8.0"` is true — the first release
  past `0.9` would have reported "up to date" permanently. Ordering now
  goes through `semver::Version`, with build metadata stripped, because
  the crate's own `Ord` ranks `0.8.0+build.7` above `0.8.0` while semver
  §10 requires build metadata be ignored for precedence.
- Downloaded artifacts are verified against a SHA-256 from the feed
  before anything is written. A feed entry without a digest is refused
  rather than trusted.
- Update replaces every component in the release, not only `vak`.
  Previously the manifest version was rewritten while the tray, desktop,
  and worker stayed on the old build, so `status` reported a clean
  install that was actually mixed-version.
- Update compares against the installed version rather than the running
  build, and gained `--dry-run`.

### Versioning, build, and release

- Restored version singularity per docs/design/32. `tauri.conf.json` and
  both frontend `package.json` files carried their own `0.7.0` stamp
  while the workspace was at `0.8.0`, so the shipped app reported the
  wrong version. Tauri now derives the version from its crate, the
  private frontends are pinned to `0.0.0`, and `scripts/check-version.sh`
  fails if a second stamp reappears.
- Added `scripts/release.sh`, which produces the `release.json` feed that
  `self update` consumes. That feed had a consumer and no producer, so
  the update path could never work end to end. It gates on version
  singularity, fmt, clippy, tests, a clean tree, and an unused tag before
  building, then emits binaries, `SHA256SUMS`, and the feed.
- Added `scripts/bump-version.sh` (one edit plus a lockfile refresh) and
  `scripts/build.sh` (build, install, verify), replacing the root
  `build-install.sh`.

## 0.8.0 — canonical layout release

Platform-standard filesystem locations. One-time automatic migration from
`~/.vak` to `~/Library/Application Support/vak` (macOS) or
`~/.local/share/vak` (Linux). Logs to `~/Library/Logs/vak`,
cache (store.db) to `~/Library/Caches/vak`. Desktop app now ships
in the install bundle with frontend assets.

- **`vak-config::paths` module**: single source of truth for data home,
  cache home, and log directory. `VAK_HOME` override nests everything
  under one directory for tests and portable installs.
- **`user_env_path()` follows data home**: secrets live in
  `data_home()/.env`, not a hardcoded `~/.vak/.env`.
- **Desktop in bundle**: `self install` copies `vak-desktop` binary and
  frontend assets into the app bundle. `Resources/` now contains the
  SolidJS SPA.
- **Stale reference purge**: 36 files changed — all user-facing
  `~/.vak` strings replaced in source code, 19 doc references fixed
  across 16 design docs, AGENTS.md, SECURITY.md, README.md, hosting.md.
- **Legacy migration**: one-time rename of `~/.vak` → canonical data
  home. store.db* → cache. logs → Library/Logs. Skipped when
  `VAK_HOME` is set. Called at all binary entry points.

## 0.7.0 — admin console release

The web admin console: one binary, one URL, full control of a running
vak from any browser (docs/design/33-admin-console.md).

- **`vak-store` crate**: SQLite FTS5 rebuildable index over session JSONL.
  BM25 full-text search with snippets across ALL content blocks (tool
  commands, results, thinking — not just prose), structured metadata
  queries (role/kind/provider/model/date/project), idempotent import,
  WAL mode. JSONL stays the source of truth; the index is disposable and
  rebuilt automatically (startup, post-run, on-demand `refresh=true`).
- **Global event hub + SSE**: typed SystemEvent broadcast (agent runs,
  session lifecycle, config changes, gateway inbound,
  approval requested/granted/denied, security alerts) streamed at
  `/admin/api/events`; browsers reconnect with backoff and lag explicitly.
- **Cookie auth alongside bearer**: `POST /admin/login` validates with
  constant-time comparison and sets an HttpOnly SameSite=Strict cookie —
  EventSource cannot send headers, so this unlocks browser SSE. Login is
  rate-limited; failures land in the security-events log and alert live.
- **Embedded console SPA** (`crates/vak-admin-ui`, SolidJS+Vite, dist
  committed so cargo needs no node): overview with live activity feed and
  stat cards, session catalog → transcripts with role rails/tool badges/
  error highlighting and pagination, global FTS5 search with highlighted
  snippets, color-coded security audit log, inbox with unread badge.
- **Operational console**: answer approval gates from anywhere (args
  preview, live refetch via SSE), edit provider/model, switch permission
  mode via consequence-labeled cards, rebuild the index, cancel runs,
  live-tail transcripts during active runs (session-scoped SSE → debounced
  refresh), sign out.
- **Canonical-client interaction**: composer sends prompts (Enter),
  mid-run sends become steering automatically, ×1–×4 selector fans prompts
  out as best-of-N candidates in isolated worktrees; "+ New session"
  creates sessions from the browser.
- **Security floor** (Phase 0 hardening): per-IP sliding-window rate
  limiting (`[gateway.rate_limit]`), path confinement for tool resolution,
  hook/PTY env allowlisting, SHA-256 trust markers, temp-file atomicity,
  Telegram chat allowlist + sender attribution, append-only security-
  events JSONL surfaced live, config audit trail across all mutation
  endpoints, SECURITY.md disclosure policy.

## Unreleased

- **Channel-aware message formatting**: the agent writes GFM markdown once;
  delivery converts per surface. Telegram replies now render as native HTML
  (bold headings/links/inline-code, fences and tables as monospace,
  blockquotes, • bullets) with tag-safe chunking at 4096 chars and an
  automatic plain-text fallback. Future channels add flavors (Slack mrkdwn,
  Discord cards) without touching agent code.
- **Tavily web search via MCP**: stdio servers get `${VAR}` interpolation in
  env values (resolved through the standard secret path — keys stay out of
  config files), a per-server egress flag (`[mcp.servers.X] network =
  true`, privileged) for remote-API tools, and operational env (PATH with
  the server's own dir, HOME/TMPDIR) so npx-based servers run correctly
  under launchd/systemd.

- **TUI `/services`**: status lines plus start/stop/restart for the gateway
  and Telegram bridge via vak-ops.
- **Desktop ▸ Settings ▸ Services**: live status dots with
  Stop/Start/Restart per service and Install/Uninstall for the pair; polls
  every 5 s while open.
- **Desktop ▸ Settings ▸ Learning** (L2): skill-proposal queue with
  Promote/Reject and a recent-notes viewer with provenance.
- **L1 reflection loop**: opt-in `[memory] reflection = true` runs an
  auxiliary call after clean completions proposing ≤2 notes (+ optional
  skill draft), deduped by Jaccard against existing notes; drafts always
  land in the human review queue.
- Gateway-path test fixtures are hermetic against the developer's global
  config so personal defaults (reflection etc.) never leak into CI.

## 0.3.0 — 2026-08-23 · platform release

The always-on platform phase (docs/design/22-gateway.md through 27):
one headless core, many surfaces, durable services.

- **Gateway**: `POST /gateway/inbound` (+wait long-poll) routes chat
  surfaces to persistent sessions; bindings survive restarts; busy turns
  queue as logged steering; unattended approvals fail closed by default or
  forward to an approver surface (`approvals = "forward"`, timeout-deny).
- **Transports**: Telegram bridge (`vak telegram`) and outbound
  webhook targets with fail-closed bearer auth; cron routines push real
  final answers to any surface (`TaskDef.deliver_to`).
- **Media passthrough**: images from chat reach vision models as native
  content blocks (Anthropic/OpenAI/Google/Responses wire shapes); Telegram
  photos auto-download.
- **Memory & recall**: `session_search` tool + `/search` endpoint rank
  curated MEMORY.md notes above transcripts; `remember` persists decisions
  with provenance.
- **Learning loop**: `propose_skill` queues drafts for human promotion via
  HTTP/CLI into user-level skill discovery — never automatic.
- **Docker exec backend**: `[sandbox] backend = "docker"` runs bash in a
  no-network container with the workspace bind-mounted at its real path.
- **Security pass**: broker-worker tool execution, bounded subprocess
  environments, workspace-rooted restricted reads, permission-change
  revocation, threat model in docs/design/24-agent-security.md.
- **Operations**: `scripts/install_gateway_service.sh` installs
  launchd/systemd services; `vak-tray` menu-bar controller with live
  indicators and a watchdog that auto-restarts crashed services;
  `VAK_GATEWAY_TOKEN` pins auth across restarts; hosting guide in
  docs/hosting.md.
- **Reliability fix found in production**: OpenAI-compatible endpoints that
  close tool-call turns without canonical finish reasons no longer strand
  dangling tool calls, and subscriber-less runs are never self-cancelled.

## 0.4.0–0.6.0 (released)

- **Model catalogues are discovered, not hardcoded.** `Core::models_for` — a
  static per-provider table — is gone. `vak_llm::models::list_models` asks the
  provider what the user's key actually reaches (`GET /models`), following
  pagination for Anthropic (`has_more`/`last_id`, 20/page default) and Google
  (`nextPageToken`, 50/page default) so long catalogues are not silently
  truncated; `Core::discover_models` memoises for 5 minutes. Exposed as
  `GET /providers/:name/models`, consumed by the desktop gate, desktop
  Settings and the TUI `/model` picker. Discovery failure reports the reason
  (invalid key, provider down) instead of substituting a stale list — the old
  table advertised 2 opencode-zen models where the key reaches 64, and offered
  4 Anthropic models for a key that no longer authenticates.
- **Provider keys are revocable.** `DELETE /config/key`, a Remove key control
  in desktop Settings, and `/key <provider> --remove` in the TUI. Revoking
  strips the entry from the user `.env`, clears the runtime override and
  the loaded-dotenv copy, drops the cached provider client and discovered
  models, and reports `shadowed_by_env` when the variable is also exported in
  the real environment (which the app cannot unset).
- **Desktop onboarding fixes.** The project gate could strand on the folder
  picker: a `Composer` ref was dereferenced before assignment
  (`ta.selectionStart`), and the throw propagated out of `setBackend` inside
  `refreshBackend`, leaving providers unset. Also: `backend-ready` was emitted
  before the webview subscribed, the gate's recovery poll tore down its own
  timer, `showConnect` captured a non-reactive boolean, and CORS omitted
  PATCH/DELETE while the router and client both used them.
- **Desktop UI**: project switcher and run controls (permission mode,
  transcript density, tokens, context ring) moved from the sidebar footer and
  the status strip into the composer toolbar; sidebar gutters normalised.
- **Single instance**: a second launch refocuses the live window instead of
  starting a rival shell with its own backend.

- **Doc 21 closed out — world-class TUI pass complete.** Interactive
  keymap rebind UI (`/keymap`: ↑↓ select, `r` capture, conflicts surfaced);
  subagent picker + attach/steer (`Alt-S` / `/subagents`): every live child
  registers its steering queues and cancel token in a shared
  `SubagentRegistry`, so Enter steers the child, Tab queues its follow-up,
  Ctrl-C stops only it, Esc detaches.
- **Personalization**: three truecolor theme packs (midnight, synthwave,
  forest) plus custom themes via `[ui.themes.<name>]` (hex or named colors
  over the dark base), all previewed live in the theme picker; Emacs/Vim
  composer modes (`[ui] composer`, `/composer [emacs|vim]`, `/vim`,
  `/emacs`) with normal-mode motions, operator+motion edits, yank/paste
  register, and undo/redo.
- **Custom slash commands**: markdown prompt templates from
  `.vak/commands/*.md` (project), `.vak/plugins/*/commands/*.md`
  (plugin-contributed palette actions), and user `commands/*.md`;
  `$ARGUMENTS` substitution; project > plugin > user precedence; wired into
  completion, the Ctrl-P palette, and `/help`.
- **Opt-in OSC52 clipboard copy** (`[ui] osc52 = true`, then `Alt-Y` or
  `/copy` copies the last response) — never automatic.
- **Accessibility modes** (`[ui.accessibility]`, runtime `/a11y
  plain|motion|reader on|off`): plain/screen-reader strip imposed colors and
  fold box-drawing/decorative glyphs to ASCII in committed history and
  transient panels; reduced motion renders a static spinner glyph.

- **Stop gate** (built-in premature-completion policy, on by default):
  blocks completions that look truncated (trailing plan marker, non-heading
  colon line, unclosed code fence) or that skip verification the prompt
  explicitly demanded with zero commands run. Reuses stop-hook
  continuation (`[stop-guard]` prefix), capped at `max_blocks` per run so
  it can nudge but never trap. `[stop_policy]` config section to tune or
  disable. Born from the dogfood campaign's 5/7 premature-stop rate.
- Workspace-write sandboxes now allow OS temp areas (/tmp plus macOS's
  /var/folders TMPDIR and $TMPDIR): test suites using tempfile/std::env::temp_dir
  no longer die under `cargo test` driven through the agent. Found by
  dogfooding — the sandbox was doing its job a little too well.
- **Learned allow rules**: `[p]` on an approval persists a scoped rule
  (`bash(cargo *)`, `edit(src/x.rs)`, `mcp(server/*)`, …) to
  `.vak/permissions.local.toml`; loaded into every future run in that
  workspace (trusted only), round-trip validated, and unable to shadow
  explicit denies. `[a]` stays session-only for unscopeable calls.
- Sandbox network parity: all sandboxed modes now deny TCP bind/connect —
  Landlock (Linux) handles it explicitly via ABI v4 with fail-closed
  enforcement checks in both read-only and workspace-write; Seatbelt
  (macOS) already denied network implicitly via deny-default profiles.
  FullAccess remains unsandboxed. Smoke scenarios extended to 7.

## 0.2.0 (2026-08-22)

Five post-0.1.0 phases. Headline changes:

### Safety

- **Linux Landlock sandbox** behind the existing `Sandbox` trait: reads and
  execute everywhere, writes only under the canonicalized cwd in
  workspace-write mode, nothing writable in read-only mode. Commands run
  through a hidden `__sandbox` self-exec runner that applies the ruleset
  before spawning the command (children inherit containment). Fail-closed:
  unsupported kernels report sandbox `off` instead of pretending.
  Enforcement smoke (`scripts/landlock_smoke.sh`, 5 scenarios) gates CI's
  new ubuntu job; macOS keeps Seatbelt.
- New dependency: `landlock = "0.4"` — safe wrapper over the Landlock LSM
  syscalls, keeping raw `unsafe` out of the workspace.

### TUI/UX (details in docs/design/18-tui.md)

- Markdown rendering of assistant output with syntax-highlighted fenced
  code; unified diffs for `edit` calls and approval previews; error tails
  on tool failures; thinking indicator.
- Live status row: spinner · elapsed · ↑/↓ tokens · context-window %.
- Approval queue with structured cards (pretty-printed args, rule reason),
  `[y]` allow-once / `[a]` always-this-tool-this-session / `[n·Esc]` deny;
  stray keystrokes can no longer silently answer or deny a pending approval.
- Input: multiline (Alt/Ctrl-J), bracketed paste, wrap- and CJK-aware caret,
  readline keys (Ctrl-U, Ctrl-W, Alt-b/f), Tab completion for slash commands
  and `@file` paths, Ctrl-R reverse history search, persisted history.
- Sessions: `/resume [n|id-prefix]`, `/rewind [seq]` checkpoint browser,
  `/transcript [n]` dump, session browser with first-prompt snippets.
- `/doctor` health panel; `/theme dark|light|plain` runtime switch; bell on
  turn finish; window title.
- Subagents: lifecycle and per-tool-call streams visible live in the parent
  UI; child token usage rolls up into `/cost` and the completed-turn footer.

### Core / reliability

- Steering queues wired through `Core::run_turn_with` — mid-run user input
  is now actually consumed by the loop (previously a UI-side dead end).
- `AgentEvent` carries tool args/result previews; `Approver::approve` sees
  full tool input, enabling informed approval cards.
- `[ui]` config section (`theme`, `bell`); cost estimates from a per-family
  pricing table (unknown models omit dollar figures rather than guessing).

## 0.1.0

Initial release — the full original roadmap:

- Kernel: agent loop + six core tools + append-only JSONL sessions with a
  frozen execution contract (`model-visible means logged`)
- Multi-provider over raw APIs: Anthropic, OpenAI responses + completions,
  OpenRouter, OpenCode Zen (incl. free models), Gemini, Ollama
- Permission engine (rules × modes) + macOS Seatbelt sandbox
- Extensibility: skills, blocking subagents with lineage-linked child
  sessions, lifecycle hooks, MCP client via a lazy meta-tool
- Agentic depth: parallel fan-out gated by resource-claim waves, static flow
  DAGs, dynamic planner with bounded replan (fail-closed)
- Eval harness (deterministic + live-model), HTTP+SSE server, checkpoints/
  rewind, git-worktree isolation
- Reliability pass: retry/backoff honoring Retry-After, per-step watchdog,
  cross-run circuit breaker with run-level endurance, session resume, server
  cancel, graceful shutdown; auto-compaction for long-horizon sessions
