# 48 — The web client: one workspace surface, three hosts

Goal: make the full vak workspace — chat, approvals, diffs, editor,
terminal, receipts, commitments — reachable from a browser, so a headless
Linux box (a VPS, a homelab node, a workstation you SSH into) is a
first-class place to *run* vak rather than only a place to host it. The
same bundle must keep serving the Tauri desktop shell, and must extend to a
single-tenant cloud deployment without a second codebase or a second
security model.

Status: **Phases 0–4 shipped.** What follows describes what exists, with
the places implementation corrected the plan called out in place. Phase E
(multi-user cloud) remains explicitly out of scope.

Related: 13-server (HTTP+SSE contract), 33-admin-console (the *operations* surface, which this does
not replace), 34-channel-onboarding (CorePool), 46-stabilization (trust and
onboarding), 47-commitment-kernel (intent/commitments).

---

## 1. Thesis

vak already claims "one core, many surfaces" (PRODUCT.md). The claim is
true of the *core* and false of the *client*: there were two hand-written
SolidJS applications against one HTTP contract — the workspace UI, then
living inside `vak-desktop` (14.4k lines), and `crates/vak-admin-ui`
(14.9k lines, operations) — and neither could be opened in a browser as
the workspace client. A headless install today
gets the operations console and a chat bridge; it does not get the
product.

The fix is not a third application. Invariant 30 ("one canonical way per
capability") says a second way to do a thing is two contracts that must
agree forever. So:

> **The web client *is* the desktop client.** One bundle, one design
> system, one set of components, behind a narrow host port with two
> implementations: a Tauri host and a browser host. The admin console
> stays what it is — the operations projection (invariant 26) — and is
> not merged in.

Everything below follows from that sentence.

### What this buys

| | today | after |
|---|---|---|
| Headless Linux box | ops console + chat bridge | full workspace in a browser |
| Phone / tablet | nothing | read, review, approve, steer |
| Cloud VPS | gateway only | full workspace over TLS |
| Client codebases | 2 | 2 (workspace + operations), not 3 |
| Surfaces sharing the workspace UI | 1 | 3 (desktop, browser, PWA) |

### Non-goals (this document)

- **Multi-user / multi-tenant cloud.** Phase D sketches the seams; it is
  not in the shipping scope. Until then a deployment is *one operator's*
  vak, and the auth model says so honestly rather than implying more.
- **Replacing the admin console.** Operations is a different job with a
  different information architecture (33 §Operations URL contract).
- **In-process TLS termination.** A reverse proxy does that better and
  every deployment already has one. We support being behind it correctly.
- **An offline-capable PWA with local persistence.** Service worker for
  shell caching only; the ledger is the server's.

---

## 2. Where we actually are

Honest inventory, because three of these are blockers people will hit in
the first five minutes and two of them are documented as working.

### 2.1 The server cannot answer a browser that is not on localhost

`require_bearer` calls `host_is_loopback` and returns `421 Misdirected
Request` for any `Host` that is not `localhost` / `127.0.0.1` / `::1`
(`crates/vak-server/src/lib.rs`). This is a correct and deliberate
DNS-rebinding defence for a loopback-only server. It also means:

- `http://192.168.1.20:8901/admin` from a laptop → **421**.
- `https://vak.example.com/` behind nginx → **421** (proxies pass the
  public `Host` through).
- Doc 33's "from any browser on the LAN" is **not achievable today**
  except through an SSH tunnel, where the `Host` is `localhost`.

### 2.2 There is no way to bind anything but loopback

`crates/vak/src/main.rs` hardcodes `SocketAddr::from(([127,0,0,1], port))`.
`docs/hosting.md` §Security posture tells operators to "bind `--host`
behind a firewall". **There is no `--host` flag.** Doc and code disagree;
the doc is aspirational.

### 2.3 SSE authenticates by query string, and cannot resume

`EventSource` cannot set headers, so `openEventStream` appends
`?token=<bearer>`. That token reaches access logs and `Referer`. It is
tolerable on loopback; it is not tolerable over a network. Separately,
the per-session channel is a bare `tokio::broadcast` with no
`Last-Event-ID` replay — a dropped connection loses every event in the
gap permanently. `App.tsx` already documents this and papers over it with
a 10-second reconciliation poll. On a laptop lid-close over Wi-Fi that
gap is the common case, not the rare one.

### 2.4 The desktop client's Tauri coupling is small

This is the good news, and it is what makes the whole plan cheap. The
entire native surface is:

| Tauri call | used by | web equivalent |
|---|---|---|
| `backend_info` | boot, `api.initBackend` | `GET /host` |
| `start_backend` | project gate, project switch | `POST /workspaces/open` |
| `review_workspace` | trust gate | `POST /onboarding/workspace-review` (exists) |
| `open_admin` | header, settings | same-origin link to `/admin` |
| `append_profile_note` | **nobody** — dead code (§11.7) | already `POST /memory` |
| `export_file` | transcript export, document download | `Blob` + `<a download>` |
| `open_workspace_file` | "Open with…" on an Office document (docs/design/72) | none: a browser cannot launch an application on the machine that holds the file |
| `spawn_pty` / `pty_write` / `pty_resize` | terminal | `WS /sessions/:id/pty` (new, gated) |
| `plugin-dialog` `open` | pick a project folder | `GET /fs/dirs` browser (new) |
| `plugin-dialog` `save` | transcript export | browser download |
| `plugin-notification` | run finished | Web Notifications API |
| `event.listen("backend-ready")` | boot race | SSE `host` event |

Eight commands and three plugins. Nothing else in 14k lines of client code
knows it is inside Tauri.

### 2.5 Defects the shared client would inherit

A browser build multiplies whatever the client already gets wrong, so
Phase 0 fixes these first. Full detail in §11; the headline is that the
**default transcript density renders a blank pane for the entire duration
of a run**, seven CSS custom properties are used but never defined, the
integrated terminal leaks a shell process per session switch, and there is
no light theme — which is survivable in a native window you chose to open
and is not survivable on a phone in daylight.

---

## 3. Architecture

```
                 crates/vak-client-ui/            ← ONE SolidJS app
                 ├── src/host/port.ts             ← the interface
                 ├── src/host/tauri.ts            ← desktop implementation
                 ├── src/host/web.ts              ← browser implementation
                 └── dist/                        ← one build output
                          │
        ┌─────────────────┴──────────────────┐
        │                                    │
  vak-desktop                            vak-server
  frontendDist = ../vak-client-ui/dist   include_dir!(".../dist") → /app
  Tauri commands (pty, dialogs, prefs)   /host, /workspaces, /auth, WS pty
        │                                    │
        └──────────── vak-core ──────────────┘
             one Core / CorePool, one ledger, one policy gate
```

Three hosts, one client:

1. **Desktop** — Tauri shell, embedded per-project server on an ephemeral
   loopback port. Unchanged behaviour.
2. **Browser, local** — `vak serve` on loopback, `http://127.0.0.1:8901/app`.
3. **Browser, remote** — `vak serve --host` behind a proxy, or through a
   tunnel/tailnet. Same bundle, same routes, stricter posture.

### 3.1 The host port

```ts
// crates/vak-client-ui/src/host/port.ts
export interface Host {
  readonly kind: "desktop" | "web";

  /** Capability probe. The UI asks, it never sniffs for `window.__TAURI__`. */
  can(feature: HostFeature): boolean;

  /** Backend identity: base URL, auth channel, cwd, recents, boot error. */
  info(): Promise<BackendInfo>;
  onInfoChanged(cb: (info: BackendInfo) => void): () => void;

  /** Workspace lifecycle. `trust` is the operator's answer when asked. */
  openWorkspace(cwd: string, trust?: boolean): Promise<BackendInfo>;
  reviewWorkspace(cwd: string): Promise<WorkspaceReview>;

  /** Choosing a folder. Native dialog on desktop; server-side browser on web. */
  pickWorkspace(): Promise<string | null>;

  /** Getting bytes to the operator. Native save dialog vs. download. */
  saveFile(suggestedName: string, bytes: Uint8Array, mime: string): Promise<SaveOutcome>;

  /** Only where can("open-with"): a workspace Office document, not
   *  macro-enabled, opened in its associated application. */
  openWith?(path: string): Promise<void>;

  notify(title: string, body: string): Promise<void>;
  openAdmin(route: string): void;

  /** null when the host has no terminal (web with terminal disabled). */
  terminal(sessionId: string): TerminalTransport | null;
}

export type HostFeature =
  | "native-dialogs"     // OS file pickers
  | "terminal"           // a real PTY
  | "multi-workspace"    // can open arbitrary folders
  | "system-notifications"
  | "tray"
  | "microphone"
  | "open-with";         // hand a workspace document to its application
```

`can()` is not decoration. The UI must render *differently*, not
*brokenly*, when a capability is absent: the dock's Terminal tab is not
disabled-and-confusing, it is absent, and Settings explains why in one
sentence. A capability the host cannot provide is a design decision, not
an error state.

### 3.2 Why not "just serve the desktop bundle"

Because five things genuinely differ and pretending they do not is how you
get a web app that feels like a port:

| | desktop | web |
|---|---|---|
| Auth | ephemeral token from the shell | cookie, login screen, logout, expiry |
| Backend | one embedded `Core` per launch | `CorePool`, several workspaces at once |
| Workspace picking | native folder dialog | server-side directory browser |
| Latency | in-process, sub-ms | WAN; needs resume, optimistic UI, backpressure |
| Viewport | ≥940×600, one operator | 375px phone → 4K; possibly two tabs |

The host port absorbs the first three. The last two are real design work
(§7, §8), not adapter work.

---

## 4. Server work

### 4.1 New and changed routes

| Route | Verb | Purpose |
|---|---|---|
| `/app`, `/app/*` | GET | The client bundle (auth-exempt shell + hashed assets, exactly like `/admin`) |
| `/host` | GET | What `backend_info` returns: ready, cwd, recents, boot error, host capabilities, surface |
| `/stream` | GET (SSE) | Every live subscription of a client on one connection, host-level changes included (§4.7) |
| `/auth/login` | POST | Token → `vak_session` cookie. **Replaces `/admin/login`** |
| `/auth/logout` | POST | Clears it. **Replaces `/admin/logout`** |
| `/auth/session` | GET | Who am I, when does this expire, what surface |
| `/workspaces` | GET | Pool status: open workspaces, warm/cold, trust state |
| `/workspaces/open` | POST | `{ path, trust? }` → resolve through `CorePool`, return `BackendInfo` |
| `/workspaces/close` | POST | Evict from the pool |
| `/fs/dirs?path=` | GET | Directory listing for the picker — dirs only, no file contents |
| `/sessions/:id/pty` | WS | Terminal transport (§6) |
| `/sessions/:id/events` | GET (SSE) | **+ `Last-Event-ID` replay** |

Per invariant 30, `/admin/login` and `/admin/logout` are **removed** in
the same change that adds `/auth/*`, along with the tests that pinned them
and the doc 33 paragraph that described them. Two login endpoints against
one cookie is exactly the "two contracts that must agree forever" the
invariant exists to prevent.

### 4.2 Binding and host policy

```toml
[server]
# Default is unchanged: loopback only, and every existing install keeps
# behaving exactly as it does today.
bind = "127.0.0.1"          # `vak serve --host <addr>` overrides
port = 8901

# Non-loopback Host headers are refused unless named here. This is the
# opt-in that makes 2.1 solvable without weakening the rebinding defence.
trusted_hosts = []          # e.g. ["vak.example.com", "100.71.4.9"]

# Set when behind a TLS-terminating proxy. Enables Secure cookies and
# makes generated links correct.
public_url = ""             # e.g. "https://vak.example.com"
```

`host_is_loopback` becomes `host_is_trusted(host, &config)`:
loopback names *or* an exact match in `trusted_hosts`. Wildcards are not
supported — a wildcard here is a rebinding hole with extra steps.

`bind` outside loopback with an empty `trusted_hosts` is a **startup
error**, not a warning. Binding a shell-capable agent to `0.0.0.0` and
finding out later is not a mistake we let someone make quietly.

### 4.3 Auth for browsers

Cookie only. No token in a query string, on any surface, after this lands.

- `POST /auth/login` takes the bearer token, compares with
  `subtle::ConstantTimeEq`, sets `vak_session`: `HttpOnly`, `SameSite=Strict`,
  `Path=/`, `Max-Age` from `[server].session_ttl_hours` (default 168),
  and `Secure` whenever `public_url` is https or `X-Forwarded-Proto: https`.
- **`?token=` is now loopback-only** (`require_bearer` gates it on the
  request's own `Host`).

  The plan said to delete it outright, on the theory that the desktop
  could do the same cookie exchange. It cannot: the Tauri webview's origin
  is the asset protocol and the embedded server's is
  `http://127.0.0.1:<ephemeral>`, so a cookie set by the latter is
  third-party to the former and modern webviews decline to send it.
  `EventSource` cannot set headers either, which leaves the query string as
  the desktop's only channel.

  So the rule became narrower and truer than "delete it": the exposure a
  token-in-a-URL carries — access logs, `Referer`, history — requires a
  proxy, a log, or a shared browser to exist. On an in-process loopback
  connection none do. On anything reachable by a hostname all three might,
  and the web client does not need the channel at all because it is
  same-origin. One channel per host, each the only one that works there.
- **On loopback the probe is the sign-in.** `GET /auth/session` answers
  `{authenticated}` so a client can tell "no session" from "server
  unreachable" — and when the request's own `Host` is loopback and
  `[server] loopback_auto_login` is on (the default), it *hands over* the
  cookie instead of reporting `false`. Asking someone to go and find a
  token to reach the machine they are sitting at is a prompt with no
  security value: anything that can reach loopback can already read the
  token off disk. A real hostname still gets `421` from the host check
  before this ever runs.
- **One login exchange, and only one.** `/admin/login` and `/admin/logout`
  were removed when this landed, because two endpoints setting one cookie
  is two contracts that must agree forever (AGENTS.md invariant 30). The
  admin console kept posting to the dead path for a release: the auth
  middleware 401s an unexempt path *before* routing, so the console showed
  a token form that could never succeed and read as a wrong token rather
  than a missing route. It now uses `/auth/login`, and probes with
  `/auth/session` so it gets the loopback grant like every other browser
  surface.
- **CSRF**: `SameSite=Strict` plus an `Origin` check on every
  state-changing method. A request with an `Origin` that is neither
  same-origin nor in `trusted_hosts` is rejected before routing. Requests
  with no `Origin` at all (curl, the CLI, bridges) must carry the bearer
  header — a cookie alone is never sufficient for a header-less request.
- **Rate limiting** already covers POSTs including login
  (`rate_limit.rs`); `/auth/login` inherits it unchanged.
- Failures append to the security-events log and emit to the hub, as
  today.

### 4.4 Event resume

`GET /sessions/:id/events` gains `Last-Event-ID` support:

- Every SSE frame carries `id: <monotonic seq>` per session.
- The session's live channel keeps a bounded ring (1024 frames, matching
  the broadcast capacity) of `(seq, frame)`.
- On reconnect with `Last-Event-ID: n`, replay `n+1..` from the ring, then
  attach to the live channel.
- If `n` is older than the ring, emit a single `event: resync` frame. The
  client responds by re-hydrating from the transcript — which it already
  knows how to do — instead of silently missing work.

This deletes the class of bug `App.tsx` currently mitigates with a
10-second poll, and it is a prerequisite for the client being usable on a
phone that sleeps.

### 4.5 Surface identity

`Surface::Web` joins the enum in `vak-core`. Every ledger entry, receipt,
approval, and commitment event originating from a browser says so.
"Which surface asked for this?" must have an answer for a remote surface
even more than for a local one.

---

### 4.5a Compression, and where it must not go

The three static bundles — `/admin`, `/app` and the site — are served
through a `tower_http` gzip layer. They are the big, highly compressible
responses (a site page is ~78 KB of inlined CSS and markup, the vendored
motion build is 141 KB, the SPA bundles larger still), and this surface
exists to be reached over a tunnel, where that is the entire first-visit
cost. Measured: 78 KB → 20 KB for a page, 141 KB → 47 KB for motion.

The layer is **scoped to those routers**, not applied at the root.
`/sessions/:id/events` is server-sent events, and a compressor between the
writer and the socket delivers a live transcript in buffer-sized batches
rather than per frame. A smaller JSON body nobody is waiting on is not
worth a laggy agent. `compression_covers_the_static_bundles_and_not_the_api`
keeps it scoped.

### 4.6 The front door at `/`

`/` used to answer a bare 401 with an empty body: someone opening
`http://box:8901/` learned nothing — not that the product had two
surfaces, not where they were, not even that anything was listening. It is
now an eight-page public site (`crates/vak-server/site/`), including the
outcome-directed runtime story, auth-exempt and
embedded with `include_dir!` like the two UI bundles, with each page's CSS
and script inline and only one shared, deferred asset: the front door must
render before, and independently of, anything else being up.

`site/build.py` renders `site/src` into the committed `site/dist`, and
`site/dist/.src-manifest` is the same format `scripts/ui_bundle_check.rs`
verifies — so a `src` edit that was never rebuilt fails `cargo build`
rather than shipping the previous pages. `crates/vak-server/site/README.md`
is the whole procedure.

`/wallpapers` offers the daylight and dusk scenes in desktop and separately
composed mobile editions. All 16 JPEG downloads and four lightweight previews
are static public assets under `/site/wallpapers/`, available without sign-in.
The shared footer links the gallery; the brand export script generates its
assets from the reviewed wallpaper masters.

What it may say is bounded by being auth-exempt. It shows the product, the
mechanism, and `/version` — version and commit, which that endpoint
already publishes. It must never show bind address, permission mode,
workspace names, or session counts; `/health` reports several of those and
is deliberately not what this page reads.

Its design is recorded in DESIGN.md ("the landing surface's ramp",
"Approval Gate") and `.impeccable/surfaces/`.

### 4.7 One connection per browser

A browser allows six HTTP/1.1 connections per host, across every tab, and
HTTP/2 is not available on plain-http loopback. The client once held an
EventSource per subscription — host changes, the session's agent events,
its presentation frames, plus side chat, coworking and config streams when
those views were open — so a tab cost three to seven connections. Measured
live: with three tabs of `/app` open, a `GET /sessions/:id/sandbox/records`
in the newest tab stayed pending indefinitely (the server answered it in
4 ms to curl), and completed the moment the older tabs closed.

So a client holds exactly one stream:

- **`GET /stream`** multiplexes any set of subscriptions
  (`crates/vak-server/src/stream.rs`). Each frame is a named SSE event whose
  JSON names its session. The per-session routes are built from the same
  per-subscription streams, so the two cannot drift.
- **Resume** is per session through one id: every `agent` frame's id is the
  cursor vector `<session>:<seq>,…`, so the browser's own reconnect sends it
  back as `Last-Event-ID`. A client that reopens the stream itself — because
  its set of sessions changed — passes the same string as `?cursor=`, and
  only the sessions it still follows are resumed. Presentation frames stay
  snapshot-based (§4.4).
- **Tabs share it.** In the web build the connection lives in a
  SharedWorker (`src/streamWorker.ts`): tabs post their interest, the worker
  opens one stream for the union and forwards each frame only to the tabs
  that asked for its session. A tab that joins a subscription already on the
  wire is handed the latest presentation snapshot and host frame from the
  worker's cache, since the server only snapshots when a stream opens. A tab
  cannot reliably announce its own death, so each holds a Web Lock for its
  lifetime and the worker waits on the same name: the grant is the death
  notice, and the union narrows.
- **Fallback.** The desktop shell, or a browser without SharedWorker or Web
  Locks (Web Locks needs a secure context, so plain-http non-loopback
  deployments fall here), holds one direct `/stream` per tab — still one
  connection, not seven.

Components never open an EventSource; they register a watcher with
`src/streamHub.ts`, and the hub derives the tab's interest from the
watchers it holds.

## 5. Workspaces in a browser

The desktop boots a *new embedded server per project*. A remote server
cannot: it is one process serving one operator who may want three
projects open in three tabs. `CorePool` (34 Phase 2) already solves this,
including the part that matters — `resolve_at` goes through
`Core::new_with_trust`, so a pooled workspace gets exactly the
trust/permission/sandbox resolution a local run there would get. Pooling
grants nothing.

- `POST /workspaces/open { path, trust? }` resolves a pooled `Core`,
  keyed as today by `(path, permission_override, ...)`.
- Session routes gain a workspace binding. A browser tab holds a
  `workspace` in its URL (`/app/#/w/<hash>/s/<session>`) so a refresh, a
  bookmark, and a second tab all land where they should. The hash is the
  same project hash the store already uses.
- The picker is `GET /fs/dirs?path=` — directory names only, never file
  contents, rooted at `[server].workspace_roots` (default: the operator's
  home). A remote directory browser is the *correct* interaction here;
  a native dialog on the client machine would be listing the wrong
  filesystem.

### 5.1 Trust in a browser

Trust (46 Step 2) is the decision that a folder's `.vak/config.toml` and
secret scope may be honoured. It is the highest-consequence click in the
product and it must not become a habit.

- The review is server-side and already exists
  (`POST /onboarding/workspace-review`): it reads section headers as text
  and never through the config loader, so describing a project's
  privileges never activates them. The browser reuses it verbatim.
- The gate names each requested privilege in words, defaults to **open
  safely** (no trust), and requires an explicit second action to trust.
- A trust decision made from a remote surface is recorded with
  `Surface::Web` and the request's origin, because "I trusted this from
  the coffee shop" is exactly the fact an audit needs.

---

## 6. The terminal (and why it is off by default)

A PTY over HTTP is remote shell access. Everything else in the web client
is mediated by the permission engine and the broker (invariants 14, 16);
a terminal is not — it is the operator's own hands. That is fine on
loopback and it is a different proposition on a public hostname.

```toml
[server.web]
terminal = false            # default; must be explicitly enabled
terminal_requires_loopback = true   # even when enabled, refuse remote hosts
```

- Transport: WebSocket at `/sessions/:id/pty` (needs the `ws` feature on
  axum, currently `["json"]` only). Binary frames both ways; the same
  `portable-pty` machinery `crates/vak-desktop/src/pty.rs` already uses,
  lifted into `vak-server` so both hosts share one implementation.
- Auth: the cookie, checked at upgrade. Origin checked at upgrade.
- **Lifecycle is part of the protocol, not an afterthought.** Socket
  close kills the PTY, reaps the child, and removes the map entry. The
  desktop's current leak (§11.4) is fixed by the shared implementation
  rather than reproduced in a second one.
- When `can("terminal")` is false the dock has no Terminal tab, and
  Settings says: *"The terminal is a real shell on the server. It is
  disabled for remote access; enable `[server.web] terminal` if that is
  what you want."*

---

## 7. Designing for the browser

The design system (DESIGN.md, "The Auditor's Desk") is not up for
renegotiation — the web client is the same product and reads as it. Four
things must be *added* to it, and one debt repaid.

### 7.1 A light theme is now mandatory

Three themes exist (`warm`, `dark`, `contrast`) and all three are dark.
`color-scheme: dark` is hardcoded on `:root` and nothing consults
`prefers-color-scheme`. A native window you deliberately opened can be
dark forever. A browser tab on a phone outdoors cannot.

- Add `light` as a fourth palette and `system` as the *default* setting,
  following `prefers-color-scheme`.
- The light palette is the same design: warm paper ground, one burnt
  terracotta accent, tonal depth, no shadow at rest. It is not an
  inversion — inverting a warm dark palette produces a cold light one.
- Every status colour needs a light-mode value that clears 4.5:1 on the
  light surfaces. The `--faint` token's history (DESIGN.md §Neutral) is
  the precedent: contrast is checked, not eyeballed.
- Fix the theming leaks found in the audit while doing this: the global
  focus ring (`rgba(238,146,120,.75)`), `::selection`, and the
  composer's context `Ring` all hardcode the *warm* accent, so they are
  wrong in `dark` and `contrast` today (§11.3).

### 7.2 Responsive down to 375px

The shell is `grid-template-columns: var(--sidebar-width) minmax(420px, 1fr) auto`
with a 940px minimum window. In a browser the minimum is whatever the
phone is.

| width | layout |
|---|---|
| ≥1200px | today's three regions: sidebar / main / dock |
| 900–1200px | dock overlays main rather than shrinking it |
| 600–900px | sidebar becomes a drawer; dock becomes a full-height sheet |
| <600px | single column; sidebar and dock are sheets; **no terminal, no editor**; composer sticks to the bottom above the keyboard inset |

The phone target is deliberate and narrow: **read the transcript, read a
diff, answer an approval, steer a run.** That is the whole job away from a
desk, and it is a job nothing else in the product does today. Do not port
the editor to a phone.

### 7.3 Approvals are the mobile feature

An approval gate is a run that has stopped and is waiting on a person.
Over a WAN that person is not looking at the screen.

- Web Notifications on `ApprovalRequested` (permission asked once, at the
  moment the first gate arrives — never on load).
- The notification deep-links to `/app/#/w/<hash>/s/<id>?approval=<req>`.
- The approval card is the one component that gets a mobile-specific
  layout: the target (`url` / `command` / `path`) at full width and
  legible, three full-width buttons, no truncation of the thing being
  decided about.

### 7.4 Latency honesty

In-process, "Working" is instantaneous truth. Over a WAN it is a claim
about a round trip that may not have happened.

- Every mutating action shows its own pending state and its own failure,
  locally, next to the control that caused it. No global spinner.
- The connection state is a first-class indicator in the status bar:
  `live` / `reconnecting` / `resyncing` / `offline`, with the same
  never-colour-alone rule as every other status signal.
- A queued prompt sent while offline is held and shown as held. It is not
  silently dropped and it is not silently retried into a duplicate run.

### 7.5 Repay the palette debt first

`styles.css` contains a second, cold palette — indigo `#818cf8`, emerald
`#34d399`, rose `#fb7185`, amber `#f59e0b`, sky `#38bdf8`, slate
`#94a3b8`, on near-black grounds `#090b10` / `#080a0f` — across the
presentation layer (the `table`, `diff`, `terminal`, `test_matrix` and
`chart` primitive renderers — at the time of this audit these were
separate `DataGrid`/`DiffInspector`/`TerminalConsole`/`TestMatrix`/
`UniversalChart` components; they are now `renderX()` functions inside
`presentation/GenericSpecRenderer.tsx`, which is where the palette debt
now lives — plus the badge set and the diff rows). The
integrated terminal ships a hardcoded Tokyo Night theme. DESIGN.md's
"Don't" list names both of these specifically.

These are the components a web client shows off. They are currently the
components that look like a different product. Retheming them onto tokens
is Phase 0 work, not polish — a second palette that has to be maintained
in two colour schemes across two hosts is four palettes.

---

## 8. Deployment topologies

### A — Loopback (default, unchanged)

```bash
vak serve --port 8901
open http://127.0.0.1:8901/app
```

Nothing to configure. `trusted_hosts` empty, bind loopback, terminal on.

### B — Headless Linux over a tunnel (recommended)

```bash
# on the server
vak serve --port 8901              # still loopback
# on the client
ssh -N -L 8901:127.0.0.1:8901 you@box
open http://127.0.0.1:8901/app
```

Zero new attack surface: `Host` is `localhost`, the existing defence
holds, the terminal stays usable, and SSH does the crypto. **This is the
topology the docs should lead with**, and it works the day Phase 1 ships.

### C — Headless Linux on a private network (tailnet / VPN / LAN)

```toml
[server]
bind = "100.71.4.9"                 # tailnet address, not 0.0.0.0
trusted_hosts = ["box.tail1234.ts.net"]
public_url = "https://box.tail1234.ts.net"
[server.web]
terminal = false
```

TLS from the tailnet's own certificates or a local proxy. Cookie is
`Secure`. Terminal off unless deliberately enabled.

### D — Cloud VPS, single operator

```
Internet → Caddy/nginx (TLS, HSTS) → 127.0.0.1:8901 vak serve
```

```toml
[server]
bind = "127.0.0.1"
trusted_hosts = ["vak.example.com"]
public_url = "https://vak.example.com"
session_ttl_hours = 24
[server.web]
terminal = false
```

Requirements the doc must state plainly, because this is where people get
hurt:

1. The proxy **must** set `X-Forwarded-Proto` and pass `Host`.
2. The bearer token is the only credential. It is a password. It lives in
   the credential store (OS keychain, or an encrypted-file fallback) and it
   is long.
3. `session_ttl_hours` should be short. A 7-day cookie on a public
   hostname is a 7-day shell.
4. Provider keys, the ledger, and the workspace are all on that box. A
   compromised box is a compromised everything — this is not a hosted
   service with blast-radius engineering, and saying otherwise would be a
   lie.

### E — Multi-user cloud (out of scope, seams only)

Not shipping, but the design must not foreclose it:

- `AppState` gains a `Principal` resolved by the auth layer (today:
  the single implicit operator).
- Data home becomes `home(principal)`; `CorePool` keys gain the principal.
- Every audit record already carries a surface; it would carry a
  principal.
- Approvals, commitments, and budgets become per-principal.

None of that is built. It is listed so that Phase 1–3 do not hardcode the
single-operator assumption into route shapes.

---

## 9. Build and packaging

- The workspace UI moved out of `vak-desktop` into
  `crates/vak-client-ui`. Git history is preserved with `git mv`; the
  desktop's `tauri.conf.json` points `frontendDist` at
  `../vak-client-ui/dist`.
- `vak-server` embeds the same `dist/` with `include_dir!`, served under
  `/app` by the same recursive-registration code `admin_ui.rs` already
  has — including its two regression tests, which exist because
  non-recursive registration shipped broken three times (v0.8.1–0.8.3).
  Do not write a second copy of that function; extract it.
- Vite gets `base: "/app/"` for the web build and `base: "./"` for the
  Tauri build, selected by an env var. One `npm run build` produces both
  or the build fails.
- `crates/vak-server/build.rs` already re-verifies the committed admin
  bundle against its sources by digest; the client bundle gets the same
  treatment. A stale embedded bundle is the failure mode that shipped
  three times.
- A service worker caches the shell only (hashed assets are immutable,
  `index.html` always revalidates). No API caching — a cached transcript
  is a lie about an append-only ledger.

---

## 10. Phases

Each phase is independently shippable and independently valuable. No
phase leaves the tree in a state where the desktop is worse than it is
today.

### Phase 0 — Repay what the shared client would multiply

Fix the defects in §11 *before* a second host inherits them. Nothing
user-visible about the web ships here; the desktop gets materially
better. **Gate: the audit list in §11 is closed and the theming tokens
are real.**

### Phase 1 — The client runs in a browser on loopback

- Extract `Host`; implement `host/tauri.ts` as an exact behavioural
  no-op relative to today.
- Implement `host/web.ts` minus terminal and minus multi-workspace.
- `/host`, `/auth/login`, `/auth/logout`, `/auth/session`; delete
  `/admin/login`, `/admin/logout`, and the `?token=` channel.
- `/app` embedded and served.
- `Last-Event-ID` resume.
- **Gate:** `vak serve` → `http://127.0.0.1:8901/app` gives a working
  workspace: create a session, run, stream, approve, diff, steer, export.
  The desktop is byte-for-byte unchanged in behaviour.

### Phase 2 — Headless Linux is a first-class deployment

- `[server] bind` / `--host` / `trusted_hosts` / `public_url`.
- `host_is_trusted`, Origin checks, `Secure` cookies behind a proxy.
- `/workspaces/*`, `/fs/dirs`, `CorePool`-backed workspace switching,
  URL-addressable workspace + session.
- Trust gate in the browser.
- `docs/hosting.md` rewritten to describe what exists (and the
  now-real `--host`).
- **Gate:** a fresh Debian box, `vak serve` behind Caddy, a laptop
  browser drives a full session over TLS. Loopback default is unchanged
  and a non-loopback bind without `trusted_hosts` refuses to start.

### Phase 3 — The surface earns the phone

- Responsive shell (§7.2), light/system theme (§7.1).
- Web Notifications + approval deep links (§7.3).
- Connection-state indicator, offline queueing, resync flow (§7.4).
- PWA manifest, installable, shell-only service worker.
- **Gate:** answer an approval and steer a run from a phone on cellular,
  with the screen having been asleep for ten minutes.

### Phase 4 — Terminal and parity tail

- `axum` `ws`; PTY served from `vak-server::web`; socket-scoped lifecycle
  (closing the tab kills the shell, by construction rather than by a
  cleanup call that can be forgotten — which is exactly how the desktop's
  own PTY leaked until `pty_close`).
- `[server.web] terminal`, off by default, loopback-pinned when on.
- Remaining desktop-only affordances audited: each one either gets a web
  equivalent or an explicit, written "this is desktop-only, because".
- **Gate:** the parity matrix in §12 has no unexplained gaps.

---

## 15. What shipped, and what it is measured by

Phases 0–4 are implemented. The security boundaries are covered by
`crates/vak-server/tests/web_client.rs`, which drives the **real secured
router over a real socket** rather than calling handlers directly — the
middleware is the security property here, so a handler test would prove
nothing about it:

| Property | Test |
|---|---|
| Shell loads unauthenticated, data does not | `the_client_shell_loads_without_a_session_but_data_does_not` |
| "No session" is distinguishable from "unreachable" | `session_status_answers_rather_than_rejecting` |
| Cookie authenticates; `Secure` is absent on plain http | `the_login_cookie_authenticates_subsequent_requests` |
| The superseded `/admin/login` is gone, not kept | `the_superseded_admin_login_is_gone` |
| Cross-origin mutation refused even holding a cookie | `a_cross_origin_mutation_is_refused` |
| Origin-less mutation still needs a real token | `an_originless_mutation_still_needs_a_token` |
| DNS rebinding refused (`Host` pinning) | `an_untrusted_host_header_is_refused` |
| `?token=` still works where it must (loopback) | `the_query_token_channel_works_on_loopback` |
| Picker cannot be walked out of its roots | `the_directory_browser_stays_inside_its_roots` |
| Terminal refused until enabled, and says which setting | `the_terminal_is_refused_until_it_is_enabled` |
| `/host` hands out no credentials | `the_host_descriptor_hands_out_no_credentials` |

Plus, in unit tests: the replay ring's three distinct answers
(`events::bus_tests` — gap, caught-up, and lost-events-must-resync are
three different things, and conflating the last two is how a client
silently loses a turn), and both embedded bundles' assets being embedded
*and* routable (`admin_ui`, `client_ui` — the failure that shipped three
times).

### Defects this work found in itself

Worth recording, because both were invisible until something exercised
them:

1. **`path_within` canonicalized only the root.** Every folder under a
   symlinked root — `/var` on macOS, `/home` on many Linuxes — was
   rejected as "outside `workspace_roots`", so the picker would have
   refused the operator's own projects. Found by a test written to check
   the opposite property.
2. **The terminal's config gate was unreachable by a plain GET**, because
   `WebSocketUpgrade` rejects a non-upgrade request before the handler
   body runs. Real clients always upgrade, so the behaviour is correct;
   the *test* had to be written the way a client actually asks, which is
   the more honest test anyway.

---

## 11. Phase 0 audit — the desktop defects to fix first

Found by review of `crates/vak-desktop` and its UI at 2.0.1. Ordered by
consequence.

### 11.1 The default transcript renders nothing while a run is working

`density` defaults to `"outcome"`. In that mode `visibleItems`
(`ChatPane.tsx`) keeps only user messages, *non-streaming* assistant text,
system notes, and unresolved approvals — tool cards, thinking, workers,
and streaming assistant text are all filtered out. `awaitingNextOutput`
returns `false` whenever the last item is a streaming assistant or an
in-flight tool, so the thinking indicator is *also* hidden. The settled
`PresentationTimelineView` is gated on `!isRunning`.

Net effect in the default mode: from the moment a tool call starts or the
final answer begins streaming until `RunFinished`, **nothing new appears
below the user's own message** — no text, no tool card, not even the
three-dot indicator. Prior settled turns stay on screen, which is what
makes it read as frozen rather than empty. This is precisely the "dead,
stuck UI" the `awaitingNextOutput` comment says it exists to prevent.

Fix: `outcome` density must still show *liveness* — either the streaming
answer (preferred; it is the outcome) or a progress row naming the current
tool. The presentation timeline should render live rather than only after
settle.

### 11.2 The header says "Retrying" forever

`retryOf()` is set by `RetryScheduled` and cleared only by
`RouteFallback` (`store.ts`). `WorkspaceHeader` renders it in preference
to both `Working` and `Ready`. One transient 429 therefore pins the header
to `Retrying · attempt 1` for the rest of the session — including when it
is idle. The store's own comment says the state should clear on "real
progress (a delta, a tool call, an end)". Nothing does that.

### 11.3 Seven CSS custom properties are used and never defined

`--dim`, `--line`, `--border-focus`, `--cyan`, `--text-dim`,
`--font-mono`, and (in the `chart` primitive renderer, formerly
`UniversalChart`) the pair above. Consequences are
not cosmetic:

- `.prompt-editor:focus { outline: none; border-color: var(--border-focus) }`
  → **the focus indicator is removed with nothing replacing it** (WCAG
  2.4.7).
- `.setup-banner { border-top: 1px solid var(--line) }` → the whole
  shorthand is invalid, so there is no border.
- `var(--dim)` on six muted labels → they inherit full-strength `--text`,
  which DESIGN.md explicitly forbids for micro-labels.
- `stroke="var(--cyan)"` → the second chart series has an invalid stroke
  and **does not render**.

Also in this class: the global focus ring, `::selection`, and the
composer `Ring`'s three thresholds hardcode the warm accent, so they are
wrong in the `dark` and `contrast` themes.

### 11.4 The integrated terminal leaks a shell per session switch

`pty.rs` exposes `spawn_pty`, `pty_write`, `pty_resize` — and no kill.
`PtyMap` entries are never removed. `TerminalPane`'s `dispose()` tears
down xterm, the resize observer and the event listener, then drops the
`ptyId` on the floor: the shell process, its reader thread, and the map
entry live until the app exits. Switching sessions with the terminal open
spawns a new shell each time.

Fix belongs in the shared implementation §6 asks for: a `pty_close`
command, `PtyMap` removal, child kill, and — since the web transport is a
socket — close-scoped lifecycle by construction.

### 11.5 Sidebar row actions are unreachable by keyboard

The per-session "view transcript" and "archive" controls are
`<span role="button" onClick>` **nested inside** the row's own `<button>`.
Interactive content inside a button is invalid HTML; with no `tabindex`
and no key handler these actions are mouse-only. Restructure the row: one
container, sibling buttons.

### 11.6 No focus management on modals, no error boundary anywhere

Ten modals carry `role="dialog"`/`aria-modal="true"`. None traps focus,
sets initial focus, or restores focus on close; tab leaves the dialog into
the page behind it. Separately there is no `ErrorBoundary` in the tree, so
one render exception in one pane blanks the entire application. Both are
table stakes before the same code runs in a tab the user can't restart by
relaunching an app.

### 11.7 Smaller, still worth doing

- `finalizeStream` is an empty function with unused parameters; `Stream.End`
  does nothing. Either close the streaming assistant there or delete it.
- Two parallel goal mechanisms: `armedGoal` (module-level, in `App.tsx`,
  driven by `/goal`) and `goalArmed` (a signal in `Composer`). Two ways
  to arm one thing (invariant 30).
- The `append_profile_note` Tauri command is dead: nothing invokes it,
  and its doc comment justifies its existence with "the embedded router
  exposes list/forget/amend but no append", which stopped being true
  when `POST /memory` landed. Settings already uses the HTTP route.
  Delete the command (invariant 30).
- `EmptyChat` returns `null`, so a new session is a blank void; DESIGN.md
  still specifies a headline style "for the chat empty state".
- `DiffPane`'s "review" sends to `activeId()`, not to the pane's bound
  session, so a best-of-N child's diff reviews the wrong session.
- `DiffPane`'s comment anchoring does a `findIndex` per rendered line —
  O(n²) on large diffs.
- Markdown is not rendered while streaming (raw `**bold**` and fence
  markers are visible), then reflows on settle — the opposite of the
  parity `md.ts` documents as its goal.
- Long transcripts and long session lists are not virtualized.
- The sidebar groups every session under the *current* `cwd`
  (`backend().cwd || session.cwd`) rather than the session's own.
  **Correction, found while implementing §5:** the visible symptom this
  described — other projects always reading "No tasks" — is not a bug.
  Sessions are stored per workspace (`<home>/sessions/<hash of cwd>/`) and
  `GET /sessions` lists exactly one workspace's, so a project you have not
  opened genuinely has no sessions *loaded*, whatever the grouping key
  says. The ordering fix is still right (a session's own cwd is what
  identifies it) and shipped; the consequence attributed to it was not.

---

## 12. Parity matrix (the Phase 4 gate)

| Capability | Desktop | Web (loopback) | Web (remote) |
|---|---|---|---|
| Chat, stream, steer, cancel | ✅ | ✅ | ✅ |
| Approvals (incl. "always allow") | ✅ | ✅ | ✅ |
| Diff review + line comments | ✅ | ✅ | ✅ |
| File editor | ✅ | ✅ | ✅ (≥600px) |
| Terminal | ✅ | ✅ | opt-in, off by default |
| Preview (dev server iframe) | ✅ | ✅ | ✅ if the port is reachable |
| Best-of-N | ✅ | ✅ | ✅ |
| Checkpoints / time travel | ✅ | ✅ | ✅ |
| Receipts, transcripts, export | ✅ save dialog | ✅ download | ✅ download |
| Search, inbox, feeds, automations | ✅ | ✅ | ✅ |
| Settings (all scopes) | ✅ | ✅ | ✅ |
| Workspace switch | native dialog | server browser | server browser |
| Trust gate | ✅ | ✅ | ✅ + origin recorded |
| Notifications | OS | Web Notifications | Web Notifications |
| Tray / background service | ✅ | — (by design) | — (by design) |
| Commitment portfolio | **missing today** | ✅ | ✅ |

The last row is a real gap and not a web-specific one: the commitment
kernel (47) shipped a portfolio in the *admin* console and only an intent
strip in the workspace client. A durable obligation the runtime will
verify is workspace information — the client should show open commitments
for the active workspace and let one be inspected and closed. Landing it
in the shared client fixes it on every host at once, which is the whole
argument of this document in one example.

---

## 13. Test plan

- **Contract**: the `Host` port gets a conformance suite run against both
  implementations. A capability either behaves identically or is absent
  via `can()`; there is no third answer.
- **Auth**: cookie set/expiry/logout; bearer-without-Origin accepted;
  cookie-without-Origin on a mutation rejected; cross-origin rejected;
  rebinding `Host` rejected; `trusted_hosts` exact-match only.
- **Startup refusal**: non-loopback `bind` with empty `trusted_hosts`
  exits non-zero with a message naming the setting.
- **Asset routing**: the two `admin_ui.rs` regression tests, generalized
  and run against `/app` — every asset `index.html` references is
  embedded *and* routable.
- **Resume**: kill an SSE connection mid-run, reconnect with
  `Last-Event-ID`, assert no gap; reconnect past the ring, assert a
  `resync` frame and a correct re-hydration.
- **PTY lifecycle**: socket close kills the child; no map growth across
  100 open/close cycles; the same assertion for the desktop command path.
- **Responsive**: the four breakpoints in §7.2 screenshot-tested at 375,
  768, 1024, 1440.
- **Contrast**: every token pair in all four themes checked against 4.5:1
  programmatically, in CI. `--faint`'s history is why this is automated
  and not reviewed by eye.

---

## 14. Open questions

1. **Does the admin console eventually fold into `/app` as a section?**
   Doc 33's operations IA is genuinely different and invariant 26 keeps
   it an evidence projection. The honest answer is probably "shared
   design tokens and shared components, separate routes" — but the two
   `styles.css` files (2.6k + 2.4k lines) should become one token file
   plus two sheets before that question can even be asked properly.
2. **Session-scoped vs. workspace-scoped tabs.** Two browser tabs on the
   same session both stream and both steer. Is that a feature (two
   screens) or a footgun (two composers)? Proposal: allowed, with a
   visible "also open elsewhere" marker, because the ledger makes it
   safe.
3. **Where does the token come from on a fresh cloud install?** Today it
   is printed to a TTY that a systemd unit does not have.
   `vak serve --print-token` writing once to a 0600 file is probably the
   answer; it needs deciding before Phase 2.
4. **Does `Surface::Web` change the prompt layer?** Doc 45 layers by
   surface. A phone-sized reader plausibly wants shorter outcomes. Do not
   guess — measure after Phase 3.
