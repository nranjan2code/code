# 34 — Channel onboarding: lifecycle, governance, admin/desktop UX

Status: **Phases 1, 2, 3, and 4 implemented, plus multi-bot-per-channel
(Phase 5, 0.11.19)** — allowlist store, gateway pending lifecycle, admin API
routes, Admin UI panels, the multi-tenant `CorePool`, the Discord and
Slack bridges, `PATCH .../allowlist/{key}` (unified with the existing
binding route-override path), the `vak doctor` "gateway channels" check
with `--repair` wiring for expired pending entries, the
known-workspaces picker, channel capability overlays, and now a first-class
`Bot` identity independent of surface. See each phase section below for
what landed.

Two sub-pieces are deliberately **deferred**, not silently dropped:

1. **Real-time transports for the new bridges.** Discord's gateway
   websocket and Slack's Socket Mode would each add a websocket
   dependency the workspace does not have today (no `tokio-tungstenite`
   anywhere in `Cargo.lock`). Both bridges instead poll an explicitly
   configured set of channel ids (`DISCORD_CHANNEL_IDS` /
   `SLACK_CHANNEL_IDS`) — correct, dependency-free, and the same "watch
   a few channels" shape the Telegram long poll has. The cost is that
   channels must be named up front rather than DMs being auto-discovered.
2. **Interactive approval components** on Discord/Slack. Forwarded
   approval gates render as a typed yes/no prompt (`Reply \`yes <id>\``),
   the same fallback Telegram used before its inline keyboard, resolving
   through the one existing `parse_verdict` path. Discord buttons and
   Slack Block Kit are the follow-up; the adapters declare
   `supports_actions: false` so nothing renders buttons that do not exist.

Written after a live incident
(2026-08-28): the Telegram bridge returned `403` for a chat that used to
work, because `gateway.chat_allowlist` is a config-file-only setting with
no UI, no runtime API, and no visible pending-request state — the operator
had to grep `security-events.jsonl` to find out why, then hand-edit a
`.vak/config.toml` and manually bounce the gateway process to apply it.
Separately, the gateway's default workspace once silently became whatever
directory `vak self services-sync` happened to be run from (the tool's own
source checkout, in the incident). Durable services now always use the
canonical `~/vak-home`; this doc also answers "which workspace should this
channel talk to?" These are the same underlying
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
- The gateway's workspace is the canonical default workspace
  (`vak_config::paths::default_workspace()`), independent of the directory
  where service management is invoked. `GatewayBinding` *does* carry a per-binding
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

**Known gap: no pause.** The status enum is exactly `pending | allowed |
denied`; there is no fourth "temporarily off, config kept" state. `deny`
is permanent and sticky ("never re-prompts" above); `revoke` (the DELETE)
does not flip a status at all — it removes the entry outright
(`GatewayState::allowlist_revoke`), so the workspace/route/permission-pin/
capability-policy an operator configured is gone with it, and the next
message from that chat starts a brand-new `pending` review from zero. An
operator who wants to quiet one channel for a while without losing its
setup, or without taking down the whole bridge (removing the bot token
stops every channel on that surface, not just one), has no lever for it
today. A `paused` status — dispatch rejected the same way `denied` is, but
carrying the same `workspace`/`route`/`permission_mode`/`policy` fields
`allowed` does, flip-backable to `allowed` without re-approval — is the
natural shape for it; not yet built.

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

## Editing an already-allowed entry, and what changing `workspace` means

Phase 1 as first written only set `workspace`/`route` at approve time —
an allowed entry was otherwise immutable except for revoke. That's not
enough: an operator needs to *re-point* a channel later (move it to a
different project, change its pinned model) without revoking and
re-approving from scratch, which would lose `added_at`/`added_by`
provenance and momentarily 403 the channel.

**Fix**: add
```
PATCH /admin/api/gateway/allowlist/{key}   body: {workspace?, route?}
```
for any `allowed` entry (pending/denied entries aren't edited, they're
approved/denied/re-triggered). This does not create a new entry or touch
`added_at`/`added_by` — it mutates `workspace`/`route` in place.

**This must reuse the existing session-rotation contract, not bypass
it.** `docs/design/22-gateway.md` already establishes: "Session contracts
never mutate... a changed effective route rotates to a new frozen session
while preserving the old append-only ledger." Editing an allowlist
entry's `workspace` or `route` is exactly the kind of change that
invalidates a bound session's contract, so `PATCH .../allowlist/{key}`
must go through the *same* stale-detection path `PATCH
.../bindings/{key}` already uses — not a second, divergent way to change
a channel's effective route. Concretely: the allowlist entry's
`workspace`/`route` and the binding's `workspace`/`route` override are
the same conceptual field observed from two admin surfaces (Phase 1's new
one and the pre-existing one); implementation should have exactly one
source of truth for "what does this channel route to" that both surfaces
read/write, not two configs that can drift from each other. (This is a
correction to Phase 1/2's implementation, not just future work — worth
checking before Phase 2 lands whether `AllowlistEntry.workspace` and
`GatewayBinding.workspace` already risk this exact drift.)

**Implemented.** The drift the parenthetical above warned about was real:
`allowlist_approve` wrote `route` onto the entry, but `binding_route` at
dispatch only ever read the *binding's* provider/model override, so the
entry's pinned route was dead config. There is now exactly one resolver,
`GatewayState::effective_route_override` — the allowed entry's route when
it has one, the binding override otherwise — and
`PATCH .../bindings/{key}` writes through to the entry when one exists, so
neither surface can hold a stale answer. `PATCH .../allowlist/{key}`
mutates the entry and then calls `invalidate_binding_revision`, the same
"drop the cached revision, never the ledger" move `set_route_override`
makes, so an edit takes effect through the existing rotation path on the
next inbound message.

**Workspace no longer resolvable** (directory moved, deleted, or a typo'd
path was approved): the pool (Phase 2) fails to start a Core for it. This
must not silently 500 on the next inbound message — it should reject with
a clear "workspace unreachable, ask an operator to repair" error *and*
surface as a `vak doctor` failure (see below), so it's caught before a
user ever hits it, not just when they do.

## Lifecycle completeness: doctor and repair

Channel state is now part of the system's health surface, not a config
detail. `vak doctor` (`crates/vak-core/src/health.rs` /
`crates/vak/src/doctor.rs`) gains a new check:

- **`gateway channels`**: fails if any `allowed` entry's `workspace` no
  longer exists / isn't readable, or if any `pending` entry has sat
  longer than the expiry window (see below) without being acted on. Pass
  detail lists counts; fail detail names the offending keys so `doctor`'s
  output is actionable without a separate admin-console trip.

`vak doctor --repair` (already built for self-install drift) gains a
matching mechanical fix for the one case that has one:
- an expired `pending` entry auto-denies (see expiry policy below) —
  mechanical, no judgment call, matches the existing "act only on checks
  with a known fix" rule from `AGENTS.md` invariant 19.
- an `allowed` entry with an unreachable `workspace` is **not**
  auto-repaired (repointing it to a different workspace is a judgment
  call, not mechanical) — `doctor --repair` reports it and points at
  `PATCH .../allowlist/{key}` or the Admin UI, same as it already defers
  provider-auth and config-warning failures to the operator today.

**Implemented.** `vak_core::health::gateway_channels_check` reads
`<sessions_home>/gateway/allowlist.json` directly (no `Core` started per
workspace just to test one) and is collected alongside the existing
checks; `crates/vak/src/doctor.rs`'s `repair_known_failures` gained the
matching case. `[gateway] pending_expiry_days` (default 7) is the window,
following the `KNOWN_GATEWAY_KEYS` pattern.

One implementation note worth stating plainly: `doctor --repair` runs in
a *different process* from a live gateway, which holds the allowlist in
memory. The repair therefore edits the store on disk (preserving unknown
fields, temp-file+rename), and `GatewayState::load` applies the same
expiry on startup — so a running gateway converges on the same state at
its next restart rather than the two paths fighting. Expiring from inside
a live gateway on a timer was not added; the startup pass plus doctor
covers the case without a new background task.

## Open questions (resolved / for review before implementation)

1. **Expiry for pending entries** — **resolved**: 7 days, configurable
   via a `[gateway]` key following the existing `KNOWN_GATEWAY_KEYS`
   pattern. Auto-deny (not silent deletion — a denied-by-expiry entry
   stays visible with `added_by: "expiry"` so "why did this stop
   working" has an answer) is now wired into `doctor --repair` above
   rather than left as a standalone cron-only cleanup.
2. **Multi-operator approval** — single-user desktop doesn't need it, but
   a hosted gateway with an admin team might want "who approved this" to
   matter beyond the audit log. Deferred; `added_by` is captured from day
   one so it's available if needed later.
3. **Per-chat rate/cost caps** — out of scope here; `finops` already caps
   globally per `docs/design/42-managed-work-contracts.md`. Whether a per-channel cap belongs on the
   allowlist entry is a separate decision.
4. **Should `workspace` on an entry be restricted to a known/registered
   set of workspaces**, or free-text any path the gateway process can
   read? **Resolved toward a picker, not free-text**: the Admin UI's
   workspace field (approve, and the new PATCH-based edit above) should
   offer a dropdown of "workspaces vak has actually been run in" (a list
   already derivable from existing session ledgers grouped by cwd —
   check `vak-store`'s indexed sessions for a ready source) with a
   fallback "custom path" option that free-types but visibly warns it's
   unverified until the pool successfully starts a Core there. This
   directly serves the doctor check above: a typo'd path is caught at
   entry time by offering known-good options first, not just diagnosed
   after the fact.

## Phase 1 scope vs. later phases

Phase 1 (this doc's original scope: allowlist store, admin API, Admin UI
pending/approve/deny/revoke) does not include real multi-tenant Core
isolation or Discord/Slack bridges — those were flagged as non-goals, then
promoted to Phase 2/3 below once it was clear "control surface" means all
three need to exist for the feature to be complete, not just the
allowlist mechanics.

**Implemented.** The allowlist store, gateway pending-lifecycle change,
admin API routes, and Admin UI panels described above are built. Phases 2
and 3 below are built too — see each section's own status note.

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

**Implemented.** `crates/vak-server/src/core_pool.rs`'s `CorePool` holds
the map; `GatewayState::core_for_entry` (`gateway.rs`) resolves it on
dispatch after allowlist resolution, ahead of session creation. Idle
eviction and the pool cap are both config keys under `[gateway]`
(`core_pool_idle_secs`, default 1800; `core_pool_max`, default 8) — see
`crates/vak-config/src/lib.rs`'s `KNOWN_GATEWAY_KEYS`. `GET
/admin/api/gateway/status` reports pool state under `core_pool`, rendered
by the Admin UI's new "Core pool" panel and a warm/cold chip in the
pending-channel approve flow.

## Per-channel permission mode

Phase 2 made a channel run its target workspace's own `Core` — including
that workspace's permission mode. That closed one gap and exposed the
next one, straight from the field: permission mode is now *workspace*-
scoped and nothing else. Every channel routed to a workspace inherits
that workspace's `.vak/config.toml` mode, so an operator who wants a
personal Telegram chat to be read-only while a team channel keeps
workspace-write has exactly one lever — stand up a second, otherwise
identical workspace purely to vary trust level. That is the same problem
the route override already solved for provider/model, and it takes the
same shape.

**Design**: `AllowlistEntry` grows `permission_mode:
Option<PermissionMode>`, a sibling of the existing `route:
Option<AllowlistRoute>` and read the same inherit-or-override way.
`None` — the default, and every pre-existing entry, since the field is
`#[serde(default)]` — inherits the workspace's own configured mode, which
is exactly today's behavior. `Some(m)` pins this one channel to `m`.

**Cap, never escalate.** An override is *not* applied blindly. The target
workspace's own resolved mode is a ceiling, and the pin is clamped to it:

```
effective = min(channel_pin, workspace_configured_mode)
```

So a channel may match or reduce what the workspace already grants, and
can never exceed it. Pinning `full-access` on a channel routed to a
workspace whose config says `read-only` yields `read-only`, not
`full-access`. This preserves Phase 2's invariant verbatim — a pooled
Core still never gives a channel more than a local `vak` run in that
workspace has — and it makes the failure mode of an operator mistake a
*denial*, not an exposure. The clamp is `PermissionMode::capped_by`
(`crates/vak-config/src/lib.rs`), a `min` over an explicit permissiveness
`rank()`; both the enforcement path and the admin display path go through
that one function, so the number the console shows and the number the
dispatch path pins cannot drift.

**CorePool keying**. This is the part that must be right. A `Core`
carries exactly one permission mode. If two channels sharing a workspace
but holding different pins were handed the same pooled `Core`, whichever
resolved first would silently dictate the other's permissions — the
lower-trust channel would exercise the higher-trust channel's mode. So
the pool key widens from a canonical workspace path to the pair
`(workspace, permission_override)`. Un-overridden channels keep key
`(workspace, None)` and therefore keep sharing the exact instance they
share today, including the gateway's own permanent default entry.
Override entries are ordinary evictable entries under the same
idle-eviction and cap semantics as any other — an override on the
gateway's *own* workspace is evictable, because it is not the gateway's
Core.

Capping happens once, in `CorePool::resolve_at`, on the freshly
constructed `Core` before it is published to the map — so no request can
ever observe the instance at its un-capped mode. The ceiling read there
is `Core::effective_permission_mode()` on a Core that has no override
yet, which is precisely the workspace's own configured value.

**Admin API**: `POST .../allowlist/{key}/approve` and `PATCH
.../allowlist/{key}` both accept an optional `permission_mode` field
alongside the existing `workspace`/`route` — same requests, no new
endpoint, exactly the way `route` already rides along. Absent, `null`, or
`""` means inherit (and on PATCH, clears an existing pin, mirroring how
an empty `route` object clears a pinned route). An unparseable value is a
400, never a silent inherit: a typo must not quietly change a channel's
access. `GET .../allowlist` entries now carry `permission_mode` (what was
pinned), `workspace_permission_mode` (the ceiling),
`effective_permission_mode` (what the channel actually gets), and
`permission_capped`. The `PATCH .../bindings/{key}` write-through
preserves an entry's pin — that surface only speaks about routes and must
not clear a permission pin as a side effect.

**Admin UI**: the pending-channel approve form and the "Edit access"
editor each gain a "Pin a specific permission mode (otherwise inherits
the workspace default)" checkbox, parallel to the existing provider/model
pin, revealing the same three-button `mode-btn` control the Settings
page's "Permission Mode" panel uses. The channel row shows "Effective
permission" beside "Effective route", labelled with its provenance
(inherited / channel override / capped by workspace), and approving with
an over-broad pin raises an alert toast naming what it was reduced to.

**Audit**: a capped grant is recorded as a `permission_capped` security
event — a new `EventKind::PermissionCapped` rather than a reused
`config_change`, so a silently-reduced grant is greppable in the audit
log. It means an operator believes a channel has access it does not have,
which is worth its own kind. It fires in two places: at approve/PATCH
time (the moment the operator sets it) and in `CorePool::resolve_at` (the
moment enforcement actually applies), so the log shows both intent and
effect.

**Known limitation**: a pooled Core's ceiling is read when that instance
is constructed. If a workspace's config later *tightens*, an already-warm
instance — overridden or plain inherit alike — keeps its earlier mode
until idle eviction drops it, up to `core_pool_idle_secs` (1800s default)
later, or indefinitely on a channel active enough to never idle out. Locked
down by `core_pool::tests::
warm_pool_entry_does_not_see_a_permission_mode_change_written_after_it_started`.
This is narrower than it first looks: the gateway's *own default*
workspace is exempt, because `CorePool::new` seeds that one entry with the
exact `Core` object `AppState.core` already is (an `Arc`-backed clone, not
a copy), so `apply_permission_mode`'s direct mutation of `state.core` (the
one thing `POST /config/mode` from this process can ever change) is
visible there immediately, same object identity. The gap is real only for
a *different* workspace this same gateway also serves (multi-tenant
CorePool) whose own `.vak/config.toml` changes independently — by a
separate `vak` process, or a hand edit — which is exactly the scenario
`apply_permission_mode`'s revoke-in-flight sweep over `state.sessions`
(docs/design/33, AGENTS.md rule 17) does reach for the *current* turn
(gateway-registered handles share that same map) but the *next* message on
that channel still resolves the stale, unrefreshed pool entry.

A fix was attempted and reverted: re-deriving the mode from disk on every
`CorePool` cache hit (respecting each entry's own permission-mode pin, so
an explicit channel override still gets re-capped against a moved ceiling
rather than clobbered outright) reproducibly hung
`busy_message_is_steered_not_dropped` (`vak-server/tests/gateway.rs`) — a
steering message resolving mid-turn on the same warm Core a live tool call
was still running on. Wrapping the refresh in
`tokio::task::block_in_place` did not fix it, so the cause is not simple
executor-thread starvation from synchronous config-file IO; it never
surfaced in `core_pool`'s own unit tests, only against a real in-flight
dispatch, and was not root-caused before the attempt was reverted. Any
future fix must pass both that gateway test and the `core_pool` test named
above — the second currently asserts the *unfixed* behavior and must flip
to asserting the mode changed, on purpose, as part of the same change.

## Phase 3: Discord/Slack bridges + per-surface admin UI

The routing model is already surface-agnostic by construction
(`key = "{surface}:{chat}"`, `InboundChannel` trait per docs/design/22's
"Inbound channel identity" section) — Phase 3 is adding the actual bridge
clients and their onboarding UI, following the Telegram bridge
(`crates/vak-server/src/surfaces/telegram.rs` server-side adapter,
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

**Implemented.** `crates/vak-server/src/surfaces/discord.rs` and
`crates/vak-server/src/surfaces/slack.rs` implement `InboundChannel`, driven by
`vak discord` / `vak slack` (same flags and env-first credential handling
as `vak telegram`); `DiscordAdapter` and `SlackAdapter` are registered in
`delivery.rs` when their bot tokens are set. Phase 1's UI was confirmed
surface-agnostic — nothing keyed on "telegram" — so the panels needed only
a `SurfaceBadge`, not generalizing. Desktop Settings renders one
Telegram-shaped token field per surface from `chat_surfaces` in
`GET /config`, via `PUT/DELETE /config/bot-token/{surface}`; only Telegram
has a managed service unit today, so the others say plainly that the
bridge is started by hand rather than promising a restart that will not
happen. See the two deferrals at the top of this doc for the polling
transport and typed-verdict approvals.

## Phase 4: channel capability overlays (0.11.3)

The channel editor now persists a restrictive `ChannelPolicy` alongside each
allowlist entry. It supports inheritance or explicit allow/deny patterns for
built-in tools, MCP servers/tools, discovered skills, and lifecycle hooks. An
explicit empty allow list blocks the category; deny patterns always win.

The policy is enforced at every relevant boundary: the Core filters built-in
tools, skills, and hooks; the MCP meta-tool filters its advertised tools and
rejects disallowed calls; and the permission engine receives the corresponding
channel-scoped rules before dispatch. `CorePool` includes a serialized policy
fingerprint in its key so channels sharing a workspace cannot share a Core
with different capabilities. Existing entries default to inheritance, and
route-only binding edits preserve their policy.

The Admin UI exposes these controls in both pending approval and registered
channel editing. It makes clear that skills control visibility while
permission rules remain the security boundary, and that secrets stay
workspace-owned. The policy does not copy API keys into channel state or grant
an overlay access to a workspace it was not already approved to use.

## Phase 5: multi-bot-per-channel (0.11.19)

Before this phase a "channel" (`telegram`/`discord`/`slack`) *was* the bot:
`Core::bot_token_env` mapped each surface to exactly one env var
(`TELEGRAM_BOT_TOKEN`, etc.), and setting a second token for the same
surface silently overwrote the first — there was no way to run two Telegram
bots side by side, and no policy tier scoped to a bot identity rather than a
chat or a workspace.

**Data model.** `Bot { id, surface, label, token_env, policy, permission_mode,
route, workspace }` (`vak-server/src/gateway.rs`) is now a first-class
entity, independent of the legacy single-slot env vars, stored in
`<sessions_home>/gateway/bots.json` next to `allowlist.json`/`bindings.json`.
`AllowlistEntry` gained `bot_id: Option<String>` (which bot this chat is
bound to) and `inherit_bot_policy: bool` (default `true` — the explicit
"break inheritance" switch: when `false`, the chat resolves purely against
the workspace and the bot tier is skipped entirely).

**Resolution chain.** Dispatch now composes three tiers — **bot → chat →
workspace** — reusing the pre-existing `Option<T>` = inherit / `Some(T)` =
override convention rather than inventing a new one:

- *Capability policy*: `ChannelPolicy::merge(bot_policy, chat_policy)`
  (`vak-config/src/lib.rs`) folds the two tiers before `Core::apply_channel_policy`
  runs — an `_allow` list from the chat wins outright when set, otherwise
  falls back to the bot's; `_deny` lists concatenate across tiers since
  denies only ever remove access, never grant it.
- *Permission mode*: chat pin capped by bot mode capped by workspace mode
  (`GatewayState::core_for_entry`), via the same `PermissionMode::capped_by`
  the chat→workspace cap already used — a bot can narrow but never widen
  what the workspace allows, and a chat can narrow but never widen what its
  bot allows.
- *Model/route*: `effective_route_override` now falls through chat route →
  bot route → legacy per-binding override → workspace default.

**Storage and credentials.** A bot's token is stored under its own env var
(`BOT_TOKEN__<ID>` by convention, though the migration path below reuses the
existing legacy var name), set/cleared via `PUT/DELETE
/gateway/bots/:id/token` — never returned by the admin API once set, same
property the legacy per-surface token slot already had. `GET/POST
/gateway/bots` and `PATCH/DELETE /gateway/bots/:id` manage the row itself
(label, policy, permission_mode, route, workspace).

**Migration.** The first time `bots.json` doesn't exist, `GatewayState::load`
synthesizes one `Bot` row per surface that already has a legacy token
configured, pointing at the *same* env var the legacy slot uses — so an
existing single-bot deployment shows up as an editable bot identity
immediately, without any operator action and without changing which env var
the running bridge reads.

**Bridges run per bot, not per surface.** `vak telegram/discord/slack
--bot-id <id>` (`vak/src/cli.rs`, `vak/src/main.rs`) resolves its token from
that specific bot's `token_env` instead of the fixed legacy slot. Because
`TelegramBridge`'s `InstanceLock` was already keyed by token (not global), a
second `vak telegram --bot-id telegram-sales --server ...` process runs
concurrently with the first and receives that bot's messages independently.
Outbound delivery (`AdapterRegistry::built_in`, `vak-server/src/delivery.rs`)
falls back to any configured multi-bot token when the legacy env var for a
surface is unset, so a bot created purely through the new flow can send
replies too. Phase 5 shipped this arbitrarily (with two or more bots on one
surface, outbound reply routing picked one token regardless of which bot a
chat was actually bound to) — Phase 6 below closes that gap.

**Admin UI.** Connect and Credentials both grow an "+ Add another bot" list
below the existing single-slot rows (`ExtraBotsList`/`BotRow`,
`vak-admin-ui/src/App.tsx`) — this is the concrete answer to "why can't I
add a second bot". The live per-chat editor (`ChannelAccessEditor`) gained a
bot picker scoped to the chat's surface and an "Inherit this bot's policy..."
checkbox bound to `inherit_bot_policy`. `BotAccessEditor` gives a bot its own
settings panel under Connect, built from the exact same components
(`WorkspacePicker`, `ChannelPermissionPicker`, `ChannelCapabilityPolicy`) the
per-chat editor uses, rather than a second, different-looking form.

## Phase 6: bot-scoped channel identity (0.11.20)

Phase 5 gave a bot its own token, policy, and permission tier, but a
*channel*'s identity was still `surface:chat` — one allowlist entry, one
session, one policy, no matter how many bots were actually members of that
physical chat. Two bots added to the same Telegram group collided onto one
shared conversation and whichever bot's row happened to hold the policy;
outbound replies picked one bot's token for the whole surface arbitrarily
(Phase 5's documented limitation, quoted above). This phase makes a channel
genuinely support multiple independent bots, not just multiple bots that
happen to share a surface.

**Key scheme.** An allowlist/session/binding key stays `surface:chat` for a
legacy or single-bot chat; once an inbound bridge tells the gateway its own
bot id (`InboundRequest.bot_id`/`InboundBody.bot_id`, threaded from
`--bot-id`), the key becomes `surface:chat:bot_id`
(`gateway_inbound`, `vak-server/src/gateway.rs`). `legacy_key_for` strips the
third segment back off. Because every multi-bot bridge already sends its own
bot id (the fix that preceded this phase), every chat routed through one
becomes bot-scoped automatically — no configuration or opt-in needed.

**Non-breaking migration.** A bot-scoped key seen by
`allowlist_resolve_inbound` for the first time checks for an already-
`Allowed` legacy `surface:chat` row and, if found, clones its
workspace/route/policy/permission forward under the new key instead of
starting a fresh pending review. This covers both a chat approved through
the admin console before bots were scoped into the key, and a
`chat_allowlist` row seeded from `config.toml` — either would otherwise stop
matching the instant a bridge started sending a bot id, forcing a needless
re-approval. A second bot joining the same physical chat inherits
independently too, producing its own entry rather than sharing the first
bot's.

**Bot-scoped outbound delivery.** `AdapterRegistry` (`vak-server/src/
delivery.rs`) now holds one adapter per `(surface, bot_id)`, built from
every bot in `bots.json` with a token set, not just one arbitrary pick per
surface. `resolve` reads the bot id off a three-part target and returns that
bot's own adapter; a target naming a bot with no token configured fails with
a specific error rather than silently answering under a different bot's
identity. `deliver_record` normalizes the target back to `surface:address`
before calling an adapter's `send`, so none of the four existing
per-adapter `target.split_once(':')` call sites needed to learn about the
bot id segment.

**What did not change.** The synchronous per-turn reply path (a bridge
process posts to `/gateway/inbound` with `wait: true` and replies with its
own `bot_token` directly) was already correctly bot-scoped before this phase
— it never went through `AdapterRegistry` at all, since each bridge process
*is* one bot. Only proactive/asynchronous delivery (approval forwarding,
scheduled pushes) needed the fix above.

## Non-goals (still, even after phases 2-6)

- No cross-machine/cross-account process isolation (see Phase 2's "what
  stays single-process").
- No email, SMS, or other non-chat surfaces — deferred until a concrete
  need names one; the `InboundChannel` trait doesn't preclude it, but
  nothing here designs for it yet.
- The admin UI does not yet visually group a physical chat's several
  bot-scoped rows together as "one chat, N bots" — Chats lists them as
  separate rows (each independently editable, which is correct), but
  nothing yet says "these three rows are the same Telegram group."
