# 34 — Channel onboarding: lifecycle, governance, admin/desktop UX

Status: **proposed**, not yet implemented. Written after a live incident
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

## Non-goals for this phase

- No multi-tenant workspace isolation (one gateway process still serves
  one Core; per-channel `workspace` picks which existing workspace's
  config/route to use, it doesn't spin up isolated Cores per channel).
- No Discord/Slack-specific UI; the lifecycle is surface-agnostic by
  construction (`key = "{surface}:{chat}"`), same as bindings today.
