# 33 — Admin console

One binary, one URL, full control. The web admin console (`/admin` on the
secured server) is the canonical management surface for vak:
observation, operation, and interaction against the same audited core the
TUI and desktop use.

Status: **shipped**. All endpoints live behind the standard
auth stack; nothing here bypasses permission checks or brokered tools.

## Goals

- **Single deployment**: the SPA is embedded in `vak-server`
  (`include_dir`) — no separate web tier, no node at runtime.
- **Canonical client**: run prompts, steer active runs, fan out best-of-N,
  answer approvals, switch modes — from any browser on the LAN.
- **Real-time by default**: a global event hub feeds SSE for event-driven
  views; control-plane snapshots use a bounded refresh interval because
  service-manager probes and durable ledgers are read as a coherent sample.
- **Operations first**: a single evidence-backed control plane makes server,
  CorePool, channels, runs, approvals, services, delivery and doctor state
  navigable from posture → subsystem → resource → incident.
- **Rebuildable intelligence**: SQLite FTS5 index over JSONL ledgers.
  JSONL remains the source of truth; dropping `store.db` is always safe.

The Operations Center is part of this same embedded shell. It is a
bookmarkable, evidence-first control plane over the server, CorePool, gateway,
channels, work, delivery, automations, providers, incidents, and Doctor
checks; it is not a second source of operational truth.

## Architecture

```
crates/vak-admin-ui     SolidJS + Vite + TS SPA (built dist committed)
crates/vak-client-ui    the WORKSPACE client, served at /app by the same
                        process (docs/design/48-web-client.md); a separate
                        surface with a different job, sharing this auth
crates/vak-catalog      the data catalog: search, lineage, "where is it" (plan M6)
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

### Operations URL contract

The Operations Center uses hash routes so a browser refresh never loses the
operator's place. Its context bar writes the selected workspace and time
window into the query string and every Operations link carries those values:

| Route | Purpose |
|---|---|
| `#/operations` | Posture and topology overview |
| `#/operations/work` | Sessions, active runs, approvals, scheduled work |
| `#/operations/runtime` | Service manager, CorePool, action receipts |
| `#/operations/channels` | Gateway bindings and sent messages (effects) |
| `#/operations/automations` | Durable scheduled task definitions and state |
| `#/operations/providers` | Effective provider/model route and provenance |
| `#/operations/incidents` | Durable correlated incident queue and history |
| `#/operations/work/runs/:id` | Run ledger and provider receipt trail |
| `#/operations/channels/:target` | Binding identity, route, and dependencies |
| `#/operations/incidents/:id` | Incident timeline, correlation, and Doctor |

The browser can therefore move from posture → subsystem → resource → incident
→ raw evidence without losing scope. Detail screens use actual server
responses; historical or unavailable records are labelled rather than filled
with placeholders.

### Agent scope and large rosters

The shell distinguishes three scopes: **All agents** is an aggregate view of
agent-owned records, **Global platform defaults** is the shared configuration
layer, and a named agent resolves its own workspace and private ledger. The
scope picker is searchable by agent name or id; Operations offers the same
search next to its workspace filter. Lists remain native keyboard-operable
controls rather than growing into a long row of agent tabs.

Aggregate ledgers fan out to each agent in batches of eight requests. Returned
memory and commitment rows retain their owning agent id, so a combined view
never sends an edit to whichever agent happened to be selected previously.
Memory writes and cleanup require one named agent; combined views are
read-only for those actions. Home, Operations, sessions, commitments, memory,
proposals, spending, and inbox signals apply their available agent/session
filters to their values and rows, not only to the scope label. Settings show
the shared default layer in the combined scope; choose a named agent to
inspect or change that agent's effective configuration.

### The data catalog (plan M6)

- One SQLite file per tenant at `tenants/<tenant>/catalog/catalog.db`
  (`crates/vak-catalog`, docs/design/73 §9), WAL mode, schema-versioned and
  rebuilt from the records alone.
- Nodes for sessions, turns, tool calls, files a call wrote, runs, effects,
  triggers, memory notes, entities and commitments; lineage edges from each
  up to its run and cause; FTS5 over doc 73's text projections (message
  text and each tool result's digest, never thinking or raw output).
- Lifecycle: a tailer with one cursor per source catches up at server
  start, after each turn and on the scheduler tick. Admin reads (the
  sessions list, search) catch up before answering; the forensics
  transcript reads the ledger itself, without its writer's lock.
- Screens (plan M6.4): Search (`#/search`) reads `/search` with kind
  filters; conversation detail has a **Lineage** tab listing what the
  catalog holds for the conversation (`/catalog/sessions/{id}/nodes`) and,
  for the chosen item, its path up to its run with the cause, Agent and
  actor (`/lineage/{id}`), linking to Run detail; Configuration's
  Housekeeping panel says whether the catalog has taken every record and
  how much of each kind it holds (`/catalog`), beside **Rebuild search**
  (`POST /catalog/rebuild`).

### Auth (cookie + bearer)

Same token, two channels:

1. `Authorization: Bearer <token>` — CLI/desktop/service clients.
2. Cookie `vak_session` — browser flows. `POST /auth/login` validates the
   token with `subtle::ConstantTimeEq` and sets an HttpOnly SameSite=Strict
   cookie (lifetime from `[server] session_ttl_hours`, default 168h).
   `Secure` is set only behind real TLS (`[server] public_url` is https, or
   a proxy sends `X-Forwarded-Proto: https`) — a `Secure` cookie delivered
   over plain http is silently discarded by the browser, so setting it
   unconditionally would make every loopback login appear to succeed and
   then never persist. `POST /auth/logout` clears it; `GET /auth/session`
   reports whether one exists.

**This login is shared with the workspace client at `/app`**
(docs/design/48-web-client.md). It used to live at `/admin/login`, which was
*removed* when `/auth/*` replaced it rather than kept alongside: two
endpoints against one cookie is two contracts that must agree forever
(AGENTS.md invariant 30).

Auth-exempt paths: `/health`, the static SPA shell + assets (no data),
`POST /auth/login`, `GET /auth/session`, and the `/app` shell. Every data route requires the token. Failures append
to the security-events log AND emit to the hub (live alerting).

Rate limiting applies to all POSTs including `/auth/login` — brute force
is bounded by `[gateway.rate_limit]`.

Beyond the token, two checks gate every request (invariant 34): the `Host`
header must be loopback or listed in `[server] trusted_hosts`, and a
state-changing request carrying a cookie must also carry an `Origin` we
recognise. A request with no `Origin` at all is not a browser mutation
(curl, the CLI, a bridge) and must satisfy the bearer check instead.

## API surface (`/admin/api/*`)

### Provider model discovery

`GET /providers/:name/models` returns the model IDs and optional capabilities
reported by that provider's published API. Settings uses the same discovery
and selection behavior for every provider. Account- or model-specific
invocation failures are surfaced through the normal provider error contract;
the server does not make vendor-only control-plane calls or require vendor
SDKs to preflight a model.

| Endpoint | Verb | Purpose |
|---|---|---|
| `/sessions` | GET | Every workspace's sessions, from the data catalog |
| `/sessions/:id/transcript` | GET | Paginated ledger entries, read from the ledger |
| `/approvals` | GET | Pending gates across all live sessions (oldest first) |
| `/bestofn` | GET | Active candidate runs |
| `/events` | GET | SSE stream of SystemEvents |
| `/security` | GET | Security-events audit log, kind-filterable |
| `/ops/center` | GET | Unified operational posture and evidence snapshot |
| `/ops/incidents` | GET | Folded durable incident history |
| `/ops/actions` | GET | Operation receipts and post-action verification |
| `/effects` | GET | Effect records (`?run=&state=`), plan M4.5 |
| `/effects/:id/resend` | POST | Send again: same effect where the provider drops repeats, else a superseding one |
| `/effects/:id/reconcile` | POST | The owner says whether an unknown effect was sent |
| `/runs/:id/spans` | GET | One run's span closes in start order, each with `started_at`, for its waterfall (plan M5b) |
| `/telemetry/services` | GET | The services that write a structured log |
| `/telemetry/logs` | GET | Content-free log lines, newest first (`?service=&level=&trace=&spans=&limit=`) |
| `/search` | GET | One search over the data catalog (`?q=&all=&kind=&agent=&limit=`; `kind=item` for intake items), audience and trash filtered before ranking (plan M6; outside `/admin/api`); a person's search includes held items with their status |
| `/nodes/:id` | GET | One catalog node |
| `/lineage/:id` | GET | A node's path up to its run, session, turn, Agent, space, actor and cause |
| `/catalog` | GET | Whether the catalog has taken every record, and its counts |
| `/catalog/rebuild` | POST | Drop and rebuild the catalog from the records |
| `/catalog/sessions/:id/nodes` | GET | What a session holds (turns, calls, files, effects, notes), oldest first, for the Lineage tab; 404 for a trashed session |
| `/intake/sources` | GET, POST | Intake sources (plan M6.5): each with its connector, tags, trust and its poll's interval and state; POST makes the source and its `source_poll` trigger |
| `/intake/sources/:id` | GET, PATCH, DELETE | One source; PATCH changes it and its poll's interval or whether it runs; DELETE removes both, and what it took stays |
| `/intake/sources/:id/poll` | POST | Polls now through the trigger's claim |
| `/intake/items` | GET | What the polls took, newest first (`?source=&status=&limit=`) |
| `/intake/items/:id` | GET | One item with its body, labels and evidence |
| `/intake/items/:id/release`, `/intake/items/:id/quarantine` | POST | A person lets a held item reach the Agent, or holds one back; a release evaluates the Agent's alerts |
| `/intake/alerts` | GET, POST | Intake alerts: keywords, tags or sources to match, a cooldown and an optional channel; a match is an `intake_match` inbox entry, each item once |
| `/intake/alerts/:id` | PATCH, DELETE | Change or remove an alert |
| `/config` | GET | Effective config snapshot, including provider/model provenance |
| `/config/global` | PATCH | User-level defaults inherited by project workspaces |
| `/config/mcp/global` | GET, PUT | Shared user MCP registry; values never expose secrets |
| `/gateway/status` | GET | Default route + provenance, binding contracts, stale reasons, allowlist |
| `/gateway/bindings/:key` | PATCH | Set provider/model pair, or `{}` to inherit workspace default |
| `/gateway/bindings/:key/rotate` | POST | Detach session; preserve ledger; create fresh on next inbound |
| `/gateway/bindings/:key` | DELETE | Remove binding and override; preserve session ledger |
| `/memory` (secured shared route) | GET, POST | List or append workspace/profile memory notes |
| `/memory/:note_id` (secured shared route) | PATCH, DELETE | Amend or explicitly forget one scoped note |
| `/memory/cleanup` (secured shared route) | POST | Remove abandoned memory artifacts and empty workspace directories; never notes |
| `/sessions/:id/workers` (secured shared route) | GET | List live child agents for a session |
| `/sessions/:id/workers/:child/steer` (secured shared route) | POST | Queue steering text for a live child, parent-scoped |
| `/sessions/:id/workers/:child/stop` (secured shared route) | POST | Cancel a live child, parent-scoped |

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

Memory and worker lifecycle routes are also secured shared routes rather
than duplicated under `/admin/api`; the Admin SPA uses the same authorization
and workspace-scope enforcement as desktop and gateway clients.

## Console views

- **Traces & logs** (System, `#/diagnostics`, plan M5b) — every service's
  content-free log lines, newest first, filtered by service, level and run
  (`?trace=<run>` opens it filtered). Lines carry ids, counts, durations
  and outcomes only; a line's run opens its Run detail, whose **timeline**
  draws the run's spans as a waterfall (run › turn › step › model call or
  tool use › the tool worker › sending), each with how long it took.

- **Home** — the operator's first screen, answering four questions in order
  and nothing else: is the system healthy, what is waiting on me, what is
  running, what is it costing.
  - **Readiness ring** — eight arcs, one per subsystem (doctor, model route,
    permissions, chat gateway, channels, delivery, approval gates, budget),
    each drawn from that subsystem's own probe. A probe that did not answer
    is `unknown` and is excluded from the healthy count rather than counted
    as healthy; a subsystem the operator switched off (`off`) is excluded
    too, and the count says how many. State is carried by colour, stroke
    pattern, and a printed word, never by colour alone.
  - **Attention queue** — one severity-ranked list merging every signal in
    the product that blocks, asks, or drifts: held approval gates, failed
    doctor checks, open incidents, undecided and still-sending messages,
    budget position, chats knocking at the allowlist, a stopped gateway
    unit, drifted bindings, configuration warnings, provider errors and
    rate limits seen on the hub, overdue scheduled jobs, best-of-N drafts,
    unread inbox, skill proposals, security events, and unfinished setup
    steps (whose own `repair` string is the row's detail). Incidents whose
    fingerprint Home already states in its own words are dropped so one
    stuck queue does not read as three problems. Empty is a measured claim:
    it names how many probes produced it and when they were sampled.
  - **Approval gates** — the one thing Home answers rather than links to,
    since a gate is a run that has already stopped.
  - **Right now** — live runs and their state, CorePool occupancy, service
    state, the next scheduled job, server version and uptime.
  - **Pulse** — event rate and family mix off a 30-minute ring buffer of hub
    arrivals kept separately from the 60-item display feed, because a rate
    computed from the display buffer pins itself exactly when the system
    gets busy. The rate names the window it is over and never divides by
    less than a minute.
  - **Spend today** — settled cost against the admission cap with an 80%
    mark, burn rate, when the cap lands at that rate, top model and
    provider, a 14-day trend, budget alerts, and the count of dispatches
    carrying no price — which makes the headline figure a floor, stated as
    such.
  - **Recent sessions** and the **live event stream**.

  Home samples `/ops/center` on the same bounded interval the Operations
  Center uses and reads everything else off the event hub. Sampling it also
  reconciles the durable incident ledger, so incidents open and resolve
  while an operator sits on Home rather than only while Operations is open.
  Home is a *projection*, not a second source of truth: every figure resolves
  to a probe, a ledger, or a manager state, and every row links to the screen
  that owns it.
- **Operations** — first-class control plane with Posture, Live work,
  Runtime & pools, Channels & delivery, Automations, Providers, and Incidents
  routes. The
  Posture view renders a selectable topology (server → gateway/CorePool →
  work/delivery), live run and service cards, and an inspector. Deeper routes
  expose approval gates, manager-backed service actions, pool idle state,
  route provenance, sent messages with Send again and reconcile, security evidence, and doctor
  checks. Empty or unavailable data is labelled explicitly; the page does not
  infer health from a missing record.
  The context bar keeps workspace and time-window scope in the URL, alongside
  connection, approval, incident, search, and Doctor affordances. Internal
  drill-down links carry that context into run, binding, delivery, and
  incident detail pages. Each detail page links the next evidence layer and
  stops at the raw transcript, receipt, manager probe, or effect record rather
  than inventing a summary. Resolved incidents remain visible as history;
  operation mutations show their durable receipt and verification state.
- **Commitments** — the portfolio (`docs/design/47-commitment-kernel.md`).
  Four questions in the order an operator asks them: what does this agent owe,
  what will it work next and why, what is stuck and on what, and can I believe
  the ones it says are finished. Rows rather than cards, because this is a
  ledger: the operator scans a column and compares across rows, and cards
  would let four commitments fill a screen that should hold thirty.
  - **Evidence meter** — the signature element, and the one fact no other
    agent surface shows. The satisfaction lattice is drawn: four segments
    filled to the level actually achieved, a rule beneath the segment for the
    level this work is *held* to, and the shortfall in the accent, because a
    gap blocking a closure is exactly what the single accent colour is
    reserved for. Work closed on the model's own say-so and work closed on a
    check the runtime ran look identical in an ordinary status column and are
    not the same claim. Colour is never the only channel: fill differs too,
    and the component carries an `aria-label` naming both levels in words.
  - **Priority** — the scheduler's arithmetic, decomposed into its named
    components on click, and its withholding reason in words when a
    commitment cannot be worked. In a product whose thesis is auditability,
    "why did it pick that one" must not be the single unanswerable question.
  - **Close** — a refused closure answers 409 and the message names the
    evidence that was missing, surfaced verbatim. The operator learns the
    closure invariant by hitting it rather than by reading about it.
  - Comparison columns drop progressively as width runs out; evidence goes
    last, because it is the column that carries the idea.
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
  selector fans out best-of-N candidates. A live-workers panel lists child
  session id, elapsed time, and parent-scoped Steer/Stop actions; child ledgers
  remain available after completion or cancellation.
- **Search** — debounced global FTS5 with `<mark>` highlighted snippets;
  click-through to transcripts.
- **Inbox** — attention entries with unread badge (30 s poll) and acks.
- **Security** — color-coded audit trail with per-kind filters.
- **Gateway** — four sub-routes, one per operator task; see below.
- **Settings** — provider/model editor (dirty-tracked), Backup models (the
  same model at other services, proposed from live catalogues and confirmed
  once for every agent, plus other models allowed per scope;
  docs/design/15-reliability.md), permission-mode cards (ReadOnly /
  WorkspaceWrite / FullAccess with consequences stated), gateway status,
  index rebuild, sign out.
- **Memory** — workspace and global profile notes with provenance, inline
  amend/explicit forget, effective search/write/reflection status, and a
  confirmed cleanup action for abandoned lock/temp artifacts.

The Operations Center's mutation contract is deliberately auditable. Service
actions and delivery replays return a receipt id plus before/after verification
and append the receipt to the `operations/actions` record chain. Current probe
candidates are folded into the `operations/incidents` chain, where repeated
observations are
grouped by fingerprint, disappearance records resolution, and reappearance
reopens the same incident. The Admin UI displays both ledgers and links them
back to the affected run, binding, or effect record.

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
| `#/gateway` (Chats) | "What channels do I have, and what do they do?" | Scan-first table; a row expands into the full route/access editor |
| `#/gateway/connect` | "I need to add a new bot" | Three-step guided sequence with live state per step |
| `#/gateway/credentials` | "Set or clear a bot token" | Credentials only — no routing field on the screen |
| `#/gateway/routing` (Defaults & status) | "Is this healthy?" | Routing defaults, provenance, Core pool |

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
   status (labelled *chat / app / project / model / can do / status*, with
   *reduced* and *new session next* for the two edge states). Editing is a row
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
`#/integrations` (Connected apps), `/skills` (Skills), `/hooks`
(Automations), `/tasks` (Scheduled tasks) — presented in
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
  reference resolved from the credential store at process launch rather
  than a literal secret, since that is how config.toml already stores them,
  and the value
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
in the Channels workspace column split `/Users/example/Projects/
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
skill sources, Home's workspace facts and run list, the routing summary,
Best-of-N repos.
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

## Plain language, and the controls that carry it

The console's audience is not only the person who wrote the config. Every
screen was reviewed against one question: does this ask the reader to know a
syntax, an internal name, or a Rust identifier in order to act? Where the
answer was yes, the wire form stayed exactly as it was and the presentation
changed.

Two rules govern the rewrite:

1. **The wire shape is never bent to fit the wording.** No endpoint, config
   key, glob grammar, cron dialect, or serde tag changed. `read-only` is
   still `read-only`; the console prints "Look, don't touch" and keeps the
   raw value in a `title` so anyone matching the UI against a log or a config
   file still can.
2. **A picker offers what exists, and never eats what doesn't.** The
   capability pickers are built from live data — `GET /config/mcp`'s servers,
   `GET /config/skills`, `GET /config/hooks` — and anything already in a
   stored list that no longer matches an offered option is rendered as a
   removable chip rather than dropped on save. Editing a channel cannot
   silently discard a pattern the picker does not understand.

`src/controls.tsx` holds the shared pieces:

- **`AccessPicker`** replaced eight free-text glob fields on the channel
  access editor. `allow` is tri-state on the wire (`null` inherits, `[]`
  blocks everything, a list limits), which is read back as three words:
  *Everything the workspace allows / Only what I pick / None*. Because
  "limit to nothing yet" and "block all" are the same `[]`, the control
  remembers the operator's choice locally — deriving the mode from the wire
  alone would snap the picker back to "block all" the instant they switched
  to "limit". Deny stays available, behind a disclosure, over the same
  options.
- **`ScheduleBuilder`** emits the cron string the tasks API already accepts,
  from a preset list and a time field. `parseSchedule` recognises the shapes
  it emits, so an existing task opens on the preset that produced it rather
  than on "custom"; anything else round-trips through the raw field
  untouched. `describeSchedule` gives the tasks table its English.
- **`MatcherBuilder`** covers the hook matcher grammar's common cases —
  every call, one tool, one tool with an argument pattern — and hands
  anything else back to a raw field. A matcher that does not parse is still
  flagged, because a hook that will not load is exactly what an operator
  needs to see.
- **`BUILTIN_TOOLS`** names the eight built-ins by what they do. The
  permission engine matches tool names case-insensitively, so the lowercase
  wire name is what gets stored either way.

Vocabulary is shared across screens on purpose: what the channel editor calls
"connected apps" and "automations" is what the sidebar calls them, and what a
permission rule is said to grant. `modeLabel` prints the same three phrases
wherever a permission mode appears, in either casing the server may send
(`read-only` from the gateway, `ReadOnly` from `GET /config`).

Chips carrying a phrase rather than a label use `.chip-phrase`, which drops
the global `text-transform: lowercase` and `white-space: nowrap` — a sentence
is not a status label, and truncating one mid-word helps nobody.

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
