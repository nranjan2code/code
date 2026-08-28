# 34 — Channel onboarding: lifecycle, governance, admin/desktop UX

Status: **proposed**, not yet implemented (Phase 1: allowlist lifecycle +
Admin UI; Phase 2: multi-tenant Core pool; Phase 3: Discord/Slack
bridges — see phase sections below). Written after a live incident
(2026-08-28): the Telegram bridge returned `403` for a chat that used to
work, because `gateway.chat_allowlist` is a config-file-only setting with
no UI, no runtime API, and no visible pending-request state — the operator
had to grep `security-events.jsonl` to find out why, then hand-edit a
`.vak/config.toml` and manually bounce the gateway process to apply it.
Separately, the gateway's default workspace silently became whatever
directory `vak self services-sync` happened to be run from (the tool's own
source checkout, in the incident), because nothing in the onboarding path
ever asks "which workspace should this channel talk to?" This doc is the
design for closing both gaps together, since they're the same underlying
problem: **adding a channel/chat has no first-class lifecycle** — today
it's an implicit side effect of config files and shell commands, not a
flow with visible state, approval, and governance.

## Current state (as built, `docs/design/22-gateway.md`)

- `gateway.chat_allowlist` / `chat_allowlist_open` live in `.vak/config.toml`
  (privileged, project-config only), loaded once at process start.
  Changing it requires a hand-edit + process restart — `self
  services-sync` regenerating an *unchanged* unit does not restart the
  process, so even an operator who knows to edit the file can end up with
  a service that silently keeps running on stale config (today's
  incident, twice).
- A rejected chat is recorded as a `chat_allowlist` / `chat_rejected`
  security event (`vak_core::security_events`) — discoverable, but only
  by an operator who thinks to look, after the fact, with no forward
  path from "I see it was rejected" to "now let it through" except
  re-editing the same file.
- The gateway's workspace is the process's `cwd` (`docs/design/32`
  invariant 3: services execute from "the workspace captured by `self
  services-sync`"). `GatewayBinding` *does* carry a per-binding
  `workspace` override in the data model (`admin.rs`'s
  `configured_workspace` field), but nothing in any UI or bridge ever
  sets it — so every channel silently inherits the process workspace,
  and nothing tells you what that workspace even is until you go look.
- Admin UI's `GatewayStatus` type already fetches `chat_allowlist` /
  `chat_allowlist_open` from `/admin/api/gateway/status`, but
  `GatewayView()` never renders them — the only place the concept
  appears in the UI is as a filter chip on the security-event log.
  Desktop Settings has a Telegram **bot token** field (the credential)
  and nothing for chat scope.
- There is no admin API route to mutate the allowlist at all — only
  `PATCH /admin/api/gateway/bindings/{key}` (route override) and the
  read-only `GET /admin/api/gateway/status`.

Net effect: onboarding a channel today means (1) a stranger messages the
bot, (2) they get silently rejected, (3) the operator has no notification
this happened, (4) fixing it means finding the right file, hand-editing
TOML, and knowing to restart a background service — four steps with no
UI at any of them, and step 4 has already burned real time twice.

## Proposed lifecycle

```
message arrives, key unknown ──▶ PENDING (recorded, not rejected silently)
        │
        ▼
operator reviews in Admin UI / Desktop Settings:
  chat id, surface, sender, first message text, arrival time
        │
   ┌────┴────┐
   ▼         ▼
APPROVE    DENY (or ignore — expires after N days, configurable)
   │
   ▼
operator picks, at approval time (not silently inherited):
  - workspace this channel talks to (explicit path, defaults to the
    gateway's own cwd but must be a visible, overridable choice)
  - route: inherit workspace default, or pin provider+model
   │
   ▼
ALLOWED — written to a live-reloadable store, applied without restart
   │
   ▼
ongoing: visible in a channel list (workspace, route, added_at, added_by,
last_active) with a REVOKE action; revoke takes effect immediately
```

Every transition is a `security_events` entry (`chat_pending`,
`chat_approved`, `chat_denied`, `chat_revoked`) — governance is an
audit-log read, not a TOML diff.

## Storage: move off config.toml, onto a live store

`chat_allowlist` stays a *valid* config.toml key (declarative, GitOps-able
for scripted deploys — someone bootstrapping a fleet still wants to
commit a starting allowlist). But the source of truth for a *running*
gateway becomes a schema-versioned file next to `bindings.json`:

```
<sessions_home>/gateway/allowlist.json
{
  "schema": 1,
  "entries": [
    {
      "key": "telegram:8846301562",
      "status": "allowed",           // pending | allowed | denied
      "workspace": "/Users/x/Projects/assistant",
      "route": null,                  // null = inherit workspace default
      "added_at": "...",
      "added_by": "admin",
      "first_seen_text": "..."        // only kept while pending, for review
    }
  ]
}
```

At startup, config.toml's `chat_allowlist` seeds this file if it doesn't
exist yet (one-time import, same pattern as other migrations); after
that, the file is authoritative and config.toml is not consulted again
for a live process — same relationship `bindings.json` already has to
route overrides. `GatewayState` holds this in memory and every mutation
(approve/deny/revoke) updates it in place — no restart, matching how
`PATCH /admin/api/gateway/bindings/{key}` already works.

## Admin API additions

```
GET    /admin/api/gateway/allowlist            list all entries (any status)
POST   /admin/api/gateway/allowlist/{key}/approve   {workspace?, route?}
POST   /admin/api/gateway/allowlist/{key}/deny
DELETE /admin/api/gateway/allowlist/{key}          revoke an allowed entry
```

`workspace` on approve is the fix for today's incident: it's an explicit
field the operator fills in (defaulting to the gateway's own cwd, shown
plainly, not assumed silently), not a value nothing ever surfaces.

## UI additions

**Admin UI** (`crates/vak-admin-ui`, extends the existing `GatewayView`):
a "Pending channels" panel above the binding list when any entry is
`status: pending` (badge count in the nav, matching the existing pattern
for approvals/inbox), each with Approve (workspace + route pickers) /
Deny. The existing binding list gains a chip showing allowlist status per
target instead of pretending the concept doesn't exist there.

**Desktop Settings** (`crates/vak-desktop/ui`, extends the Telegram
section): once a bot token is configured, a "Chat access" subsection
lists pending/allowed chats the same way, so a single-user desktop setup
doesn't require opening the web admin console just to approve their own
phone's chat id.

## Open questions (for review before implementation)

1. **Expiry for pending entries** — auto-deny after N days so the list
   doesn't accumulate abandoned probe attempts indefinitely? Proposed
   default 7 days, configurable.
2. **Multi-operator approval** — single-user desktop doesn't need it, but
   a hosted gateway with an admin team might want "who approved this" to
   matter beyond the audit log. Deferred; `added_by` is captured from day
   one so it's available if needed later.
3. **Per-chat rate/cost caps** — out of scope here; `finops` already caps
   globally per docs/design/27. Whether a per-channel cap belongs on the
   allowlist entry is a separate decision.
4. **Should `workspace` on an entry be restricted to a known/registered
   set of workspaces**, or free-text any path the gateway process can
   read? Free-text today (matches how `self services-sync` already picks
   an arbitrary cwd), but a picker constrained to "workspaces vak has
   actually been run in" would prevent typo'd paths creating a binding
   that silently 500s.

## Phase 1 scope vs. later phases

Phase 1 (this doc's original scope: allowlist store, admin API, Admin UI
pending/approve/deny/revoke) does not include real multi-tenant Core
isolation or Discord/Slack bridges — those were flagged as non-goals, then
promoted to Phase 2/3 below once it was clear "control surface" means all
three need to exist for the feature to be complete, not just the
allowlist mechanics.

## Phase 2: multi-tenant Core isolation

Today: one gateway process = one `Core` = one `cwd`, fixed at process
start. A channel's `workspace` field (Phase 1) only *selects* which
already-loaded workspace's config/route to route to — it doesn't run that
workspace's own Core. That's a real gap: a channel approved for
`/Users/x/Projects/other-repo` while the gateway process's own Core is
rooted at a different cwd has no way to actually get that other
workspace's tools, sandbox, permission mode, or session ledger — only its
provider/model pair, via the existing route-override mechanism. It cannot
actually work as designed without this.

**Design**: `GatewayState` holds a `CorePool` — a map from canonical
workspace path to a lazily-started `Core` instance, instead of the single
`state.core` it holds today (which becomes the pool's entry for the
gateway's own default workspace). On inbound dispatch, after allowlist
resolution, look up the entry's `workspace`; if it's not the default,
resolve (or start) that workspace's `Core` from the pool instead of using
`state.core` directly. Each pooled `Core` gets its own:

- session ledger home (already implied — sessions are keyed by cwd today,
  per docs/design/22's "Sessions remain append-only JSONL trees keyed
  per-cwd" — this phase makes that actually reachable from a remote
  channel instead of only from a local CLI/desktop client already sitting
  in that cwd).
- permission mode / sandbox / trust resolution, loaded the normal way
  `Core::new_with_trust` already does for any workspace.
- idle eviction: a pooled Core with no active binding and no recent
  activity (default: 30 min, configurable) is dropped, so a gateway
  fielding channels for a dozen workspaces doesn't hold a dozen live
  Cores (and their sandboxes/file watchers) forever. Re-approving or the
  next inbound message on that channel simply re-starts it.
- resource bounds: a process-wide cap on concurrently pooled Cores
  (default TBD, propose 8) with the oldest-idle evicted first when the
  cap is hit — prevents an approval mistake (or a compromised approver)
  from turning the gateway into an unbounded fork bomb of Core instances.

**What stays single-process**: this is still one `vak serve --gateway`
process, one bearer token, one admin surface — it pools *Core* instances
(the per-workspace session/tool/permission engine), not processes. A
workspace that genuinely needs process-level isolation (a different user
account, a different machine) is out of scope here — that's still "run
another gateway process," unchanged from today.

**Admin UI**: the Gateway page's workspace picker (Phase 1's approve
flow) needs a live indicator of which workspaces currently have a pooled
Core running (vs. cold, will start on next message) — reuses the existing
`ops` status pattern already shown elsewhere in the admin console
(`GATEWAY routing control`'s summary strip).

**Security**: a pooled Core for workspace W still goes through the exact
same trust/permission resolution as running `vak` locally in W — pooling
does not grant a channel more access than a local session in that
workspace would already have; the *approval* step (Phase 1) is what gates
a channel getting there at all.

## Phase 3: Discord/Slack bridges + per-surface admin UI

The routing model is already surface-agnostic by construction
(`key = "{surface}:{chat}"`, `InboundChannel` trait per docs/design/22's
"Inbound channel identity" section) — Phase 3 is adding the actual bridge
clients and their onboarding UI, following the Telegram bridge
(`crates/vak-server/src/telegram.rs` server-side adapter,
`vak telegram` CLI bridge process) as the template for both.

**Discord bridge**: a `vak discord --server <url> --token <bot-token>`
bridge process (new binary or `vak` subcommand, matching `vak telegram`'s
shape), long-polling or gateway-websocket per Discord's bot API,
implementing `InboundChannel` with `chat` = Discord channel id, `sender`
= Discord user id. Delivery adapter registered in
`crates/vak-server/src/delivery.rs` alongside the existing
`TelegramAdapter`, for forwarded-approval inline components (Discord
buttons are the equivalent of Telegram's inline keyboard).

**Slack bridge**: a `vak slack` bridge using Slack's Events API +
`chat.postMessage`, same `InboundChannel` shape, `chat` = Slack
channel/DM id, `sender` = Slack user id. Slack's interactive Block Kit
buttons play the same role as Telegram's inline keyboard for approvals.

**Per-surface admin/desktop UI**: extends the Phase 1 Pending
channels/Registered channels panels (which are already surface-agnostic
in rendering, since they key everything off `surface:chat`) with:
- a bot-token field per surface in Desktop Settings, mirroring the
  existing Telegram token field exactly (same storage convention:
  `.env` at the user data home, owner-only permissions, save restarts the
  bridge).
- a surface icon/label in the Admin UI channel list so a mixed
  Telegram+Discord+Slack deployment reads clearly at a glance (the
  `gateway` icon path already in App.tsx's icon map is generic; add
  per-surface icons or at minimum a text badge).
- no new *allowlist* concept — Phase 1's pending/approve/deny/revoke
  lifecycle already applies uniformly to any surface's `key`, exactly as
  designed; Phase 3 only adds the bridges that can actually deliver a
  `surface:chat` key from Discord/Slack in the first place.

## Non-goals (still, even after phases 2-3)

- No cross-machine/cross-account process isolation (see Phase 2's "what
  stays single-process").
- No email, SMS, or other non-chat surfaces — deferred until a concrete
  need names one; the `InboundChannel` trait doesn't preclude it, but
  nothing here designs for it yet.
