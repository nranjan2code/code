# 33 — Admin console

One binary, one URL, full control. The web admin console (`/admin` on the
secured server) is the canonical management surface for vakcoder:
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
| `/gateway/status` | GET | Gateway enablement, bindings, chat allowlist |

Answering approvals, running prompts, steering, cancelling, mode changes,
config patches, inbox acks reuse the EXISTING secured routes. Config patches
are durable workspace mutations: the server writes the selected fields
atomically before applying the live Core override, and returns source metadata
so clients can distinguish project config, global config, environment, and
runtime/scoped overrides.
(`/sessions/:id/approvals/:req`, `/sessions/:id/run`, `/sessions/:id/
steering`, `/sessions/:id/cancel`, `/config/mode`, `PATCH /config`,
`/inbox/:id/ack`). The console is just another client of the same contract.

## Console views

- **Overview** — stat cards, system health, pending-approval card with
  Approve/Deny (live via SSE), activity feed.
- **Sessions** — filterable catalog; "+ New session".
- **Transcript** — role-railed entries, tool badges, error highlighting,
  kind/role filters, pagination, **Live tail** toggle (session-scoped SSE →
  debounced refetch), Cancel button while a run is active, and the
  **composer**: Enter-to-send prompts, mid-run sends become steering, ×1–×4
  selector fans out best-of-N candidates.
- **Search** — debounced global FTS5 with `<mark>` highlighted snippets;
  click-through to transcripts.
- **Inbox** — attention entries with unread badge (30 s poll) and acks.
- **Security** — color-coded audit trail with per-kind filters.
- **Settings** — provider/model editor (dirty-tracked), permission-mode
  cards (ReadOnly / WorkspaceWrite / FullAccess with consequences stated),
  gateway status, index rebuild, sign out.

Toasts surface high-signal events everywhere (runs finished, gates
waiting, security alerts); the sidebar dot shows hub connection state
with exponential-backoff reconnect.

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

## Testing

- Rust unit tests cover store schema/import/search/idempotency, hub
  delivery, login→cookie→authenticated-call flow, unauthenticated 401s,
  POST-only mutations, approvals aggregation.
- End-to-end smoke: boot `vakcoder serve --port P`, verify health open,
  shell serves unauthenticated, API 401 without auth, login sets working
  cookie, SSE delivers a triggered security event in real time, refresh=
  true surfaces just-appended entries.
