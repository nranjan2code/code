# 33 — Admin console

One binary, one URL, full control. The web admin console (`/admin` on the
secured server) is the canonical management surface for vak:
observation, operation, and interaction against the same audited core the
TUI and desktop use.

Status: **shipped** (Phases 1–3). All endpoints live behind the standard
auth stack; nothing here bypasses permission checks or brokered tools.

## Goals

- **Single deployment**: the SPA is embedded in `vak-server`
  (`include_dir`) — no separate web tier, no node at runtime.
- **Canonical client**: run prompts, steer active runs, fan out best-of-N,
  answer approvals, switch modes — from any browser on the LAN.
- **Real-time by default**: a global event hub feeds SSE; every view
  reflects live state without polling.
- **Rebuildable intelligence**: SQLite FTS5 index over JSONL ledgers.
  JSONL remains the source of truth; dropping `store.db` is always safe.

## Architecture

```
crates/vak-admin-ui     SolidJS + Vite + TS SPA (built dist committed)
crates/vak-store        rusqlite FTS5 rebuildable index (new crate)
crates/vak-server/src/events.rs    global hub (tokio::broadcast)
crates/vak-server/src/admin.rs     /admin/api/* data plane
crates/vak-server/src/admin_ui.rs  /admin static asset serving
```

### Event hub

`EventHub` wraps one `tokio::broadcast::Sender<SystemEvent>` (capacity
4096; slow consumers receive an explicit `Lagged` frame and reconnect).
Initialized once per process via `init_global()`; emitters are fire-and-
forget and never block producers.

Event families: agent lifecycle (run summaries), session lifecycle
(created / entry appended), config changes, gateway inbound, approval
requested/granted/denied, security (auth failures, rate limits), provider
errors, heartbeat.

The SSE endpoint `GET /admin/api/events` streams them all. Browsers
subscribe with `EventSource`; auth rides the session cookie because
`EventSource` cannot send headers.

### Store (FTS5 index)

- One SQLite DB at `cache_home()/store.db`, WAL mode, schema-versioned.
- `entries` table: entry_id, session_id, project_hash, parent_id, ts,
  kind (header/message/compaction/receipt/goal), role, provider, model,
  tool_name, content_text, is_error.
- `entries_fts`: FTS5 with `porter unicode61`; BM25 ranking + `snippet()`.
- Indexes ALL content blocks — tool commands, tool results, thinking — not
  just prose. This is the main recall win over the in-memory scanner.
- Lifecycle: background full rebuild at server start; per-session import
  after create and after each completed run; synchronous re-import when a
  transcript is read with `refresh=true` (safe: reads never conflict with
  the ledger writer's exclusive lock).

### Auth (cookie + bearer)

Same token, two channels:

1. `Authorization: Bearer <token>` — CLI/desktop/service clients.
2. Cookie `vak_session` — browser flows. `POST /admin/login` validates the
   token with `subtle::ConstantTimeEq` and sets an HttpOnly SameSite=Strict
   cookie (7-day Max-Age; no `Secure` flag by design — this server is
   loopback/LAN-first and plain http would silently drop Secure cookies).
   `POST /admin/logout` clears it.

Auth-exempt paths: `/health`, the static SPA shell + assets (no data),
`POST /admin/login`. Every data route requires the token. Failures append
to the security-events log AND emit to the hub (live alerting).

Rate limiting applies to all POSTs including `/admin/login` — brute force
is bounded by `[gateway.rate_limit]`.

## API surface (`/admin/api/*`)

| Endpoint | Verb | Purpose |
|---|---|---|
| `/sessions` | GET | Session catalog (GROUP BY over index) |
| `/sessions/:id/transcript` | GET | Paginated entries; `refresh=true` re-imports first |
| `/search?q=` | GET | FTS5 BM25 search w/ snippets, role/kind/project filters |
| `/approvals` | GET | Pending gates across all live sessions (oldest first) |
| `/bestofn` | GET | Active candidate runs |
| `/events` | GET | SSE stream of SystemEvents |
| `/security` | GET | Security-events audit log, kind-filterable |
| `/store/rebuild` | POST | Full index rebuild (mutation ⇒ POST) |
| `/store/import/:id` | POST | Import one session's JSONL |
| `/config` | GET | Effective config snapshot, including provider/model provenance |
| `/config/global` | PATCH | User-level defaults inherited by project workspaces |
| `/config/mcp/global` | GET, PUT | Shared user MCP registry; values never expose secrets |
| `/gateway/status` | GET | Default route + provenance, binding contracts, stale reasons, allowlist |
| `/gateway/bindings/:key` | PATCH | Set provider/model pair, or `{}` to inherit workspace default |
| `/gateway/bindings/:key/rotate` | POST | Detach session; preserve ledger; create fresh on next inbound |
| `/gateway/bindings/:key` | DELETE | Remove binding and override; preserve session ledger |

Answering approvals, running prompts, steering, cancelling, mode changes,
config patches, inbox acks reuse the EXISTING secured routes. Config patches
are durable workspace mutations: the server resolves provider/model into one
complete pair, writes the pair atomically, then hot-applies one atomic Core
route. New-session admission in other local Core processes refreshes that
persisted pair, so desktop, server, CLI, and inheriting channels converge
without a restart. Source metadata and a deterministic route revision let
clients distinguish project config, global config, environment, and
runtime/scoped overrides. Explicit runtime pins remain non-global.
The refresh also covers max turns, theme, MCP servers, and hooks. A persisted
permission-mode difference is routed through the revoke path (cancel main and
side runs, deny pending approvals) before the new mode is exposed.
(`/sessions/:id/approvals/:req`, `/sessions/:id/run`, `/sessions/:id/
steering`, `/sessions/:id/cancel`, `/config/mode`, `PATCH /config`,
`/inbox/:id/ack`). The console is just another client of the same contract.

## Console views

- **Overview** — stat cards, system health, pending-approval card with
  Approve/Deny (live via SSE), activity feed.
- **Sessions** — filterable catalog; "+ New session"; per-row Archive/
  Unarchive, Delete (archived only, soft — the ledger file is kept), and
  Export `.md`, plus a bulk "Delete all archived". A **workspace** column
  distinguishes rows this console instance can mutate (its own bound
  workspace) from rows it can only read (another project the store also
  indexes) — `attach`/`run`/`diff`/`receipts`/archive/delete/export all
  resolve a session's ledger file under this process's own
  `sessions_home/sessions/<hash(cwd)>/`, never another workspace's.
- **Transcript** — role-railed entries, tool badges, error highlighting,
  kind/role filters, pagination, **Live tail** toggle (session-scoped SSE →
  debounced refetch), Cancel button while a run is active, and the
  **composer**: Enter-to-send prompts, mid-run sends become steering, ×1–×4
  selector fans out best-of-N candidates.
- **Search** — debounced global FTS5 with `<mark>` highlighted snippets;
  click-through to transcripts.
- **Inbox** — attention entries with unread badge (30 s poll) and acks.
- **Security** — color-coded audit trail with per-kind filters.
- **Gateway** — four sub-routes, one per operator task; see below.
- **Settings** — provider/model editor (dirty-tracked), permission-mode
  cards (ReadOnly / WorkspaceWrite / FullAccess with consequences stated),
  gateway status, index rebuild, sign out.

### Gateway information architecture

The gateway area grew one panel at a time — routing summary, Core pool, bot
tokens, pending channels, registered channels — until it was one page an
operator had to scroll to form a mental model of. It did not survive contact
with a real operator: a bot token was pasted into the routing-key field,
because a credential input and a routing-key input sat two boxes apart on the
same scroll.

It is now split by task, each with its own hash route, listed in the sidebar
as sub-rows while the section is open and in a tab bar above the view:

| Route | Task | Shape |
|---|---|---|
| `#/gateway` | "What channels do I have, and what do they do?" | Scan-first table; a row expands into the full route/access editor |
| `#/gateway/connect` | "I need to add a new bot" | Three-step guided sequence with live state per step |
| `#/gateway/credentials` | "Set or clear a bot token" | Credentials only — no routing field on the screen |
| `#/gateway/routing` | "Is this healthy?" | Routing defaults, provenance, Core pool |

Rules the split enforces:

1. **A credential and a routing key never share a screen.** Bot tokens live
   under Credentials (and inside step 1 of Connect, which is the same
   component). The manual routing-key registration is demoted to a
   `<details>` on Channels and says in its own copy that a token does not go
   there. The surface prefix is still validated against the known bridges.
2. **Onboarding reads as a sequence, not a pile.** Connect's three steps —
   set the credential, message the bot, review and approve — report their own
   state from the backend (`GET /config`'s `chat_surfaces` for token
   presence, the allowlist for pending chats) rather than asking the operator
   to track where they are. The current step carries the accent border;
   finished steps dim.
3. **Ongoing management is a list, not a form.** Channels is a table of
   channel, surface, workspace, effective route, effective permission, and
   status (including *capped* and *rotates next*). Editing is a row
   expansion, so the settled state reads first and the form appears on
   demand. Every prior control — save route, rotate now, edit access,
   revoke access, remove binding, approve/deny — is preserved unchanged.
4. **Empty states teach.** A fresh install's Channels screen explains what a
   channel is and what the three steps are, and offers the Connect action; a
   cold Core pool says why cold is normal.

Loading is skeleton rows and blocks, not a spinner dropped into content. No
new tokens or colors: the split reuses `panel`, `table`, `chip`, `binding-*`,
and the DESIGN.md status vocabulary. The documented `:focus-visible` accent
ring, themed scrollbars, and tabular numerals were specified in DESIGN.md but
had never reached the console; they ship here.

Toasts surface high-signal events everywhere (runs finished, gates
waiting, security alerts); the sidebar dot shows hub connection state
with exponential-backoff reconnect.

## Extensions: visibility and scope

The Gateway split answered "who may talk to this agent, and under what
permission". Extensions answers the same question for everything else that
widens what the agent can do. `#/integrations` is four hash routes —
`#/integrations` (MCP servers), `/skills`, `/hooks`, `/tasks` — presented in
a tab bar and as sidebar sub-rows, structurally identical to the Gateway
section so the two governance surfaces read as one system.

Each screen is a scan-first table with its add-form demoted to a
`<details>`, matching the Channels pattern: what is configured reads first,
the form appears on demand.

### What backs the scope column

Nothing here is inferred where it could be read.

- **Permission rules** come from `GET /admin/api/config`'s `permissions`
  object (`allow`/`ask`/`deny`), added for this view. The console parses each
  string with the same grammar as `vak_permission::rules::Rule::parse` — a
  `+`/`?`/`-` prefix selects allow/ask/deny, a bare rule allows, `Tool(glob)`
  scopes to matching arguments and `Tool` covers every call.
- **Which rules reach an MCP server** follows the engine's own matching:
  `arg_candidates` renders an MCP call as the literal `server/tool`, so a
  rule reaches a server when it is blanket (`mcp`) or when the server half of
  its pattern globs to that name.
- **The fallthrough** is the mode arm of `PermissionEngine::evaluate`. `mcp`
  is neither a read nor a write tool, so an uncovered call is `allow` under
  full-access, `deny` under read-only, `ask` under workspace-write. The UI
  names that decision rather than leaving the cell blank.
- **MCP network and environment** are on `McpServerConfig` (`network`,
  `env`), both editable from the console (register/edit form). `env` values
  round-trip verbatim through `GET`/`PUT /config/mcp` — usually a `${VAR}`
  reference resolved from `.env` at process launch rather than a literal
  secret, since that is how config.toml already stores them, and the value
  is exactly what a filesystem-reading operator could already see.
- `GET /config/mcp` and `GET /config/hooks` report the *project's own*
  `.vak/config.toml` only, never the merged effective set (`state.core`'s
  global+project view). A channel/hook editor's `PUT` always resubmits
  whatever the matching `GET` reported; reporting the merged view would
  hand back an inherited global server or hook, which the very next save
  would write into the project file as if it were the project's own —
  silently forking an MCP server, and (worse, since `[[hooks]]` merges by
  `Vec::extend` rather than a name-keyed map) duplicating a hook on every
  edit cycle, compounding without bound and re-firing a side-effecting
  command once per copy. A user-scope entry is still available to the
  workspace at runtime; it is simply not listed or editable from this
  screen, and the panel copy says so.
- **A hook's `enabled` flag is real**, not a display convenience: disabling
  one keeps its definition in `.vak/config.toml` (`enabled = false`) rather
  than deleting it, and `vak_core::build_hooks_from` skips disabled entries
  when it builds the live `HookDef` list for a turn. Toggling it, and
  editing a hook in place (event/matcher/command/timeout), are both `PUT
  /config/hooks` — there is no per-hook endpoint, so every one of these
  writes the whole (project-scoped) list back.
- **Hook scope** is the hook's own matcher, which `vak-core` parses into a
  `Rule` before `vak-hooks` matches it. It prints verbatim, and a matcher
  that does not parse is flagged rather than hidden.
- **Skill provenance** comes from `GET /skills`, extended with `path` and a
  `scope` of `workspace` (`<cwd>/.vak/skills`) or `user`. **Known gap:**
  `vak_core::skills::discover` sorts by name and dedups same-named entries
  *before* scope is attached — a project skill silently shadows a
  user-level one of the same name (deliberate precedence, matching MCP
  servers and custom commands), but unlike custom commands (which document
  "names must be unique after precedence dedup," docs/design/09) nothing
  here discloses that the shadowed one exists. `GET /skills` cannot report
  it because `discover()` has already dropped it; a fix belongs there, not
  in the admin projection.
- **Channel capability overlays** are edited from the Gateway channel editor
  and pending-approval form. They support inherited or restrictive
  built-in-tool, MCP server/tool, skill-visibility, and hook patterns. The
  Admin UI sends the overlay with the allowlist mutation; the runtime stores
  it on the binding and applies the same policy before tool dispatch. Skills
  are visibility only, and secrets remain workspace-owned.

### What is deliberately not shown

- **Live MCP tool lists.** `McpManager` spawns a server lazily, per run;
  the server process holds no persistent client to enumerate. Listing a
  server's real tools means starting it, which is a side effect an admin
  read should not have. A route that does this deliberately is a separate
  decision, not something to fake with a config echo.
- **Rule editing.** The rule lists render read-only in Settings. A console
  session that can rewrite `allow` can widen its own reach; that belongs in
  the file and the trust prompt.
- **A per-tool effective decision.** A rule may cover only some of a
  server's tools (`mcp(github/read_*)`), so a single verdict per server would
  be a lie. The matching rules and the fallthrough are shown side by side
  instead, with the precedence stated: among rules matching one call, deny
  outranks ask outranks allow, and config order never decides it.

An older server omits `permissions` entirely. The console renders "scope not
reported" in that case, never an empty rule list — the difference between an
unknown scope and an unrestricted one is the whole point of the screen.

## Paths and tables at scale

A filesystem path in a table cell is not free text. `word-break: break-all`
in the Channels workspace column split `/Users/nisheethranjan/Projects/
vakcoder` across three lines mid-word; every row became a different height
and the column read as damage.

Paths render through one primitive (`truncatePath` / `PathCell`) with three
properties:

1. **Whole segments are dropped, never split.** The head is spent first
   because workspaces share their `/Users/<name>` prefix and differ in the
   tail.
2. **The budget is characters, not segments.** A segment-counted result can
   still overflow its column and be clipped from the right by CSS, throwing
   away the very tail that distinguishes two workspaces. Fitting a character
   budget means the CSS ellipsis is only a backstop for a single segment
   wider than the whole column. `PathCell` derives its `max-width` from the
   same budget, so the two can never disagree.
3. **Nothing is lost.** The full path is on the `title` and is selectable.

Applied wherever a path sits in a constrained cell: Channels, Core pool,
skill sources, the Overview workspace, the routing summary, Best-of-N repos.
`.wrap` remains for genuinely free text, but breaks at spaces before it
breaks a word.

Tables that scroll keep their headers: `.table` carried `overflow: hidden`
purely to round two corners, which made it a scroll container and silently
disabled `position: sticky` on `thead th`. Corners are rounded per-cell now.
Chips no longer wrap, so one two-word label cannot make its row taller than
its neighbours.

Measured at 100 rows against real payload shapes: uniform row height, sticky
header holding after a 1200px scroll, client-side filter at under 4ms, no
horizontal body overflow.

**Known limit.** `GET /admin/api/gateway/allowlist` and the bindings in
`GET /admin/api/gateway/status` return everything in one response with no
`limit`/`offset`/`q`. The table handles a hundred rows comfortably and the
filter is client-side over the full set, which is honest while the API is
unpaginated. Several thousand channels would want server-side paging and
search first; a paginated UI has not been built over an API that cannot
page, because it would have to invent the pages.

## Invariants

1. **JSONL is truth.** `store.db*` are derived artifacts: excluded from
   checkpoints, safe to delete, rebuilt automatically.
2. **Listing never resolves.** Cross-session approval listing displays;
   answering stays scoped to the owning session's endpoint.
3. **Mutations are POSTs.** Anything that changes state must never be
   triggerable by a prefetcher.
4. **No secrets in the shell.** The SPA carries no token; auth lives in
   the HttpOnly cookie; the login page stores nothing.
5. **Built assets are committed.** `crates/vak-admin-ui/dist` is embedded
   at compile time so `cargo build` needs no node; regenerate with
   `cd crates/vak-admin-ui && npm install && npm run build`.
6. **A route is a pair.** Admin APIs never persist or hot-apply a provider
   independently from its model. Binding changes mark old contracts stale;
   they never rewrite a session header.

## Testing

- Rust unit tests cover store schema/import/search/idempotency, hub
  delivery, login→cookie→authenticated-call flow, unauthenticated 401s,
  POST-only mutations, approvals aggregation.
- End-to-end smoke: boot `vak serve --port P`, verify health open,
  shell serves unauthenticated, API 401 without auth, login sets working
  cookie, SSE delivers a triggered security event in real time, refresh=
  true surfaces just-appended entries.
