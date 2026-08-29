# 22 — Gateway: always-on surfaces + cron delivery

Closes the platform gap versus OpenClaw / Hermes-agent (see research notes in
`06-research-notes.md`): both are *always-on personal agent platforms* whose
core differentiator is a gateway that routes many chat surfaces to persistent
agent sessions, plus unattended scheduled work with delivery back out.

vak's structural advantage: **the core was always headless.** TUI and
desktop were built as consumers of the server contract from day one. The
gateway is therefore not a new product bolted on the side (OpenClaw's mistake
— its gateway owns everything) but one more consumer of the exact same
contract the desktop already speaks. Core stays hostable anywhere — a $5 VPS,
a homelab box, a cloud worker — and every surface keeps working unchanged.

## Telegram bot ownership (single-consumer model)

Telegram permits exactly ONE getUpdates poller per bot token; a second
consumer gets `409 Conflict` for as long as both run. The bridge treats
ownership as first-class state:

- **Local mutual exclusion** — an O_EXCL marker at
  `$VAK_HOME/locks/telegram-<fnv(token)>.lock` records holder
  host+pid. A second bridge on the same machine fails fast naming the
  holder; a stale marker (crashed process) is detected via pid liveness
  and taken over.
- **Hot-standby takeover** — if a rival on ANOTHER machine owns the
  long-poll, the local bridge does not hammer 409s: it probes quietly
  (1s→30s capped backoff, one log line per ~10 probes) and takes over
  automatically the moment the rival disappears.
- **Send never conflicts** — sendMessage works regardless of who polls,
  so outbound delivery is unaffected during contention.

Operational rule: run at most one bridge per bot token across your fleet;
the lock + standby make violations safe instead of silent.

## Thesis

```
Telegram bot ┐                          ┌ TUI (local)
Discord bot  ├─ POST /gateway/inbound ──┤ Desktop (embedded secured_router)
Slack app    │   (bearer token)         │ exec / flows / evals
cron tick    ┘                          └ any future client
                    │
             vak-server gateway layer
             bindings · routing · steering · approvals · delivery
                    │
                 vak-core  (unchanged, surface-agnostic)
```

One `Core` per workspace; N concurrent routed sessions above it. The Core owns
one atomic provider/model default, while each binding may own an atomic
provider/model override. Sessions remain append-only JSONL trees keyed per-cwd.

## Non-goals for this phase

- No in-tree Telegram/Discord/Slack clients (they are thin adapters over
  `/gateway/inbound`; shipping them is config-level work, G1 below).
- No media I/O (images/voice/TTS) yet.
- No multi-workspace routing: one gateway process serves the cwd it was
  started in (per-binding `workspace` override is the extension point).
- No daemon self-daemonization: run under systemd/launchd/tmux like every
  serious long-lived service. `serve --gateway` is foreground by design.

## Architecture

### Surfaces and bindings

A **surface** is anything that can deliver an inbound message and receive an
outbound reply. The universal surface adapter is plain HTTP:

- `POST /gateway/inbound` `{surface, chat, sender?, text, wait?, capabilities?}` →
  - `202 {state:"started"|"steering_queued", session_id}` or, with
    `"wait": true`, `200 {state:"completed", text, session_id, delivery}` after the
    turn finishes (240s cap). A curl one-liner is a complete adapter:
    message in, final assistant text out. Streaming consumers use the
    existing per-session SSE instead.
- `GET /gateway/status` → enabled flag + binding table.
- `DELETE /gateway/bindings/{key}` → unbind (URL-encoded `surface:chat`).

A **binding** maps a routing key to a session:

```
routing key = "{surface}:{chat}"        e.g. "telegram:48211", "webhook:ci-alerts"
binding     = key -> { session_id?, provider?, model?, workspace?, route_revision? }
                                      persisted at <home>/gateway/bindings.json
```

The bindings document is schema-versioned. Legacy `key -> session_id` maps are
read without data loss and become versioned records on the next binding write.
Writes use temp-file + rename atomicity.

Route precedence is `channel override > workspace default`. Provider and model
are accepted, persisted, and applied only as a pair. A channel with no override
inherits the workspace route revision. First message from an unknown key
creates a fresh session; later messages resume it across gateway restarts.

Session contracts never mutate. Before every inbound dispatch, the gateway
compares the bound session's workspace/provider/model contract with the
binding's effective route. Missing ledgers and mismatches are stale bindings:
the old JSONL remains intact, the binding rotates, and the inbound message
starts a fresh session under the new contract. This is how an administrative
default propagates without corrupting history.

Authenticated administration exposes register, pairwise route update,
inherit-default, rotate-now, and remove operations. Status includes effective
route and provenance, frozen session contract, workspace, route revision, and
machine-readable stale reasons.

### Busy handling = steering, never 409

The single-writer discipline (`Option<SessionLog>` handoff) is kept. When an
inbound arrives mid-run:

1. The text goes onto the session's `SteeringQueues` (logged user entry —
   invariant 1 holds: model-visible ⇒ reconstructable from the JSONL).
2. The running loop drains steering between turns, so the agent sees the new
   instruction inside the same run.
3. If the run finishes before draining, the gateway runner drains what is
   left and starts a continuation turn automatically. Chat semantics:
   nothing typed while busy is ever dropped, and nothing is lost between
   runs.

### Approvals: fail-closed for unattended turns

Unattended surfaces cannot click "approve". Gateway-driven turns run with
`AutoDeny`: Ask-classified tool calls are denied with the reason fed back to
the model as a tool error, which it can route around. Interactive surfaces
(TUI/desktop) keep their existing approval queues untouched — they share the
session, not the policy.

Since G2, `approvals = "forward"` routes each gate to the configured
`approver` target through the normal delivery transports. The request is
announced (and published as an `ApprovalRequested` event for SSE consumers),
then the turn blocks until one of:

- a reply arrives **from the approver chat only** matching the strict verdict
  vocabulary (`y/yes/approve/approved/ok/allow` → allow;
  `n/no/deny/denied/block` → deny) — any other text from that chat falls
  through to ordinary conversation routing and resolves nothing. A reply may
  address one gate explicitly (`yes ab12cd34`, the short id from the
  announcement); an addressed verdict resolves **only** that gate — a yes
  meant for one session can never approve another session's tool run. A bare
  verdict resolves the oldest outstanding gate, and the resolution response
  reports which gate and session it decided;
- the `approval_timeout_secs` window lapses (default 300s, minimum 5s) —
  deny, and the gate is retired so a late reply resolves nothing;
- the run is cancelled — deny.

Verdict-shaped chatter from any non-approver chat never touches the gate
queue; in `deny` mode nothing is announced at all.

Webhook deliveries retry transient failures (network errors, 429, 5xx) up to
three attempts with bounded exponential backoff; permanent 4xx answers return
immediately. Inbound messages carry an optional typed `sender`; when a busy
session queues the message as steering, the sender is attributed on the
queued turn and image attachments ride along undegraded (steering entries are
full user messages, not flattened text).

### Cron → delivery targets

The desktop's routines daemon graduates into the platform scheduler:

- `TaskDef` gains `deliver_to: Option<String>` (a routing key). Serde default
  keeps existing `tasks.json` files valid.
- On `RunFinished`, the watcher records `last_summary` as before and, when
  `deliver_to` is set, pushes `"routine '<name>' finished:\n<text>"` to that
  surface through the same outbound path used for chat replies.
- Since G1, `last_summary` holds the run's **real final assistant text**
  (extracted from the restored ledger), not a status word — desktop task
  rows and deliveries both show the actual answer.
- Transports:
  - `log:<chat>` — append-only journal at `<home>/gateway/deliveries.jsonl`.
    No network, works headless, doubles as the test double.
  - `webhook:<name>` — POST `{target, text, ts, job_id, delivery}` JSON to a URL configured
    under `[gateway.outbound.webhooks.<name>]`; optional `token_env` names
    an env var whose value is attached as a bearer token at delivery time.
    A configured-but-missing credential fails the delivery **closed** (the
    run's answer is still recorded on the task) rather than posting
    unauthenticated.

### Channel formatting and adapters

The agent emits GitHub-flavored markdown; `vak-delivery` projects it through a
typed, loss-accounted packet. See `30-output-engineering.md` for the isolated
renderer, templates, durable outbox, multi-message contract, and adapter recipe.

| Surface | Flavor |
|---|---|
| Telegram | HTML (`parse_mode=HTML`) — headings→bold, links, inline code; fences/tables→monospace `<pre>`; bullets→•; model-emitted HTML escaped first |
| webhook | semantic JSON plus exact `text` compatibility fallback |
| log | raw markdown plus the semantic packet |

Rules: conversion is injection-safe (escape before translate); long messages
chunk at paragraph boundaries without cutting tags; a chunk rejected by the
channel (400) is resent stripped to plain text — degradation is ugly, never
lost. Sidecar channels declare capabilities on `/gateway/inbound`; native push
channels implement the registry adapter trait. Agent code remains unchanged.

## Configuration

```toml
[gateway]
enabled   = true      # default false — remote execution must be opt-in
approvals = "deny"    # "deny" | "forward"
approver  = "telegram:48211"   # required for "forward"; <surface>:<chat>
approval_timeout_secs = 300    # min 5

[gateway.outbound.webhooks.ci]
url       = "https://ci.example.com/vak"
token_env = "CI_HOOK_TOKEN"   # name only; value resolved per delivery

chat_allowlist = ["telegram:48211", "log:ops"]  # empty fails closed (0c-02)
# chat_allowlist_open = true   # explicit opt-out: accept any chat instead
```

- Unknown keys warn-not-fail per house convention (`KNOWN_GATEWAY_KEYS`,
  `KNOWN_OUTBOUND_KEYS`, `KNOWN_WEBHOOK_KEYS`).
- `[gateway]` is a **privileged section**: stripped entirely from untrusted
  project config (enabled gates remote execution; outbound URLs are exfil
  targets).
- `serve --gateway` forces `enabled` regardless of config (CLI override).
- `chat_allowlist` fails closed: an empty list rejects every inbound chat
  with `403` until the operator either lists chats or sets
  `chat_allowlist_open = true` to explicitly accept all of them (0c-02).
  Earlier revisions treated empty as "allow all" by default; that default
  meant any user who found a deployed bot got a full agent session, so it
  was inverted rather than kept for compatibility.

## Channel bridges

Bridges are thin clients of the HTTP contract and can run anywhere the bot
API and the gateway are both reachable:

```bash
vak serve --gateway --trust          # process 1: hosted core
TELEGRAM_BOT_TOKEN=123:abc \
vak telegram --server http://10.0.0.5:8901 --token vk_...   # process 2
```

The Telegram bridge long-polls `getUpdates`, routes each text/photo/document
message through `/gateway/inbound` with `wait`, and answers via `sendMessage`
(chunked at Telegram's 4096-char cap on newline-safe boundaries). Edits and
other unrouted update kinds advance the offset without routing so they are
never replayed. Transient failures back off 3s; ten consecutive failures give
up with a clear error. `TELEGRAM_API_BASE` overrides the API host for
self-hosted relays and tests. Tokens live in `.env` / the user `.env`, never
in config or flags.

### Document attachments

A Telegram `document` (code, logs, CSVs, ...) up to 256 KiB downloads and
rides alongside the text as an `InboundAttachment` with `kind = "document"`.
The gateway has no generic-file content block, so a document is either text
the model can read directly or it isn't included: `compose_prompt` decodes it
and inlines it as a fenced text block (capped at a further 64 KiB — the
inline-limit is deliberately smaller than the download cap, since a document
that decodes to more than that is better excerpted by the sender than dumped
whole into every turn's context) or, past either cap, appends a note telling
the sender it wasn't attached rather than truncating it silently. Photos keep
using the existing `kind = "image"` vision-content path.

### Inline-keyboard approvals

`[gateway] approver = "telegram:<chat>"` previously had no way to actually
reach Telegram: `deliver()` only had `log`/`webhook` adapters registered, so
a forwarded gate to a Telegram approver failed the async push and denied
closed. A `TelegramAdapter` (`crates/vak-server/src/delivery.rs`) now
registers under the `telegram` scheme whenever `TELEGRAM_BOT_TOKEN` is
configured — or, since docs/design/34 Phase 5 (multi-bot-per-channel), when
no legacy token is set but at least one `Bot` row for the surface has one —
rendering `DeliveryPacket.actions` (the existing
approve/deny `ApprovalPayload` actions) as a Telegram
`reply_markup.inline_keyboard` instead of requiring a typed `yes`/`no`.
Button taps arrive as `callback_query` updates; the bridge's
`callback_data_to_verdict_text` maps `"approve:<id>"` / `"deny:<id>"`
straight onto the same `"yes <id>"` / `"no <id>"` verdict text a typed
reply produces, so gate resolution runs through the one existing
`parse_verdict` path rather than a second one. `answerCallbackQuery` is
called either way so the button never shows a stuck loading spinner.

### Inbound channel identity (0c-03)

`chat_allowlist` and the per-conversation session binding are only as
strong as `chat`/`sender` being the bridge's real remote identity — a
bridge that reuses one fixed value for every user would silently merge
every stranger into one session and defeat the allowlist outright. Bridges
build their `/gateway/inbound` payload through `vak_server::gateway::
InboundChannel` + `InboundRequest::new` rather than hand-rolling the JSON:
the constructor rejects an empty `chat`/`sender` and rejects either being
left as a copy-pasted placeholder equal to the surface name, so a new
bridge (Slack, Discord, ...) that hasn't wired up real per-user identity
fails loudly at send time instead of shipping a silent allowlist bypass.
The Telegram bridge implements `InboundChannel` and maps `chat` to the
Telegram chat id and `sender` to Telegram's own numeric `from.id`.

## Security posture

Lessons from OpenClaw's incident history, inverted:

1. Bearer-token auth on every route (existing middleware), `/health` open.
2. Gateway **off by default**; enabling requires trusted config or CLI flag.
3. Untrusted projects cannot enable the gateway via project config.
4. Unattended turns auto-deny escalations; no silent yes.
5. Bindings file lives under `<sessions_home>` next to tasks.json — same
   trust domain as session ledgers, no secrets inside (channel tokens belong
   in `.env` at the adapter layer, never here).
6. Binding route overrides contain identifiers only, never credentials; model
   catalogues still come from live provider discovery.

## Invariants mapping

| Invariant | Status |
|---|---|
| Model-visible ⇒ logged | inbound texts enter as prompt or steering entries |
| Append-only sessions | untouched — bindings reference, never rewrite |
| Configuration authority | provider/model is one atomic pair; stale bindings rotate to a new frozen session |
| Errors are values | inbound validation + delivery return typed errors; turn failures deliver `is_error` summaries |
| Abort preserves output | `/cancel` still works per session; partial text delivered on abort |
| No unsafe | none added |

## Phases

| Phase | Delivers | Exit criterion |
|---|---|---|
| **G0 ✅** | HTTP inbound + wait mode, bindings persistence, steering-busy path + continuation, AutoDeny unattended turns, log surface, `TaskDef.deliver_to`, `[gateway]` config, `serve --gateway` | e2e tests: roundtrip, reuse, busy-steering, disabled-gateway 409, cron delivery; fmt/clippy/tests green |
| **G1 ✅** | Outbound webhook transport (`webhook:<name>`) with fail-closed bearer auth; Telegram bridge client + `vak telegram` subcommand; real-text routine summaries; restart-resilience proof; scheduler-free `gateway_router()` for embedders | offline e2e vs mock Bot API and webhook receiver; missing-credential fail-closed test; bindings survive simulated process restart |
| **G2 ✅** | Approval forwarding: `[gateway] approvals = "forward"` + `approver` target + `approval_timeout_secs`; gates announced via any delivery transport, resolved by strict yes/no replies from the approver chat only; timeout/silence fails closed, late replies resolve nothing. **Media passthrough**: inbound attachments ride the ledger as native image blocks — Anthropic/OpenAI/Google wire formats all supported, Telegram photos auto-downloaded (largest variant) | gateway_approvals.rs e2e: forward→yes→tool really runs; unanswered gate times out and a late "yes" resolves nothing; deny mode never announces; media_passthrough.rs proves image reaches request AND ledger; bridge photo flow vs mock Bot API (getFile→download→base64) |
| **G3 ✅** | Remote exec backends behind the Sandbox seam — Docker implemented in vak-core, see docs/design/25-docker-sandbox.md (same-path bind mount, no-network container, read-only root, caps, fail-closed daemon probe); `[sandbox] backend/image` privileged config | live e2e vs real daemon: exec, mount visibility, network deny, RO enforcement, config-flow naming |
| G4 | Cross-session memory & recall: FTS index + logged `session_search` tool | recall across restarts auditable in transcript |
| **G5 ✅** | Learning loop via `propose_skill` → review queue → human promotion to user-level discovery (never automatic); see docs/design/26-learning.md | promote→discovery loop closes in learning_loop e2e; overwrite refusal; HTTP + CLI review paths |

## Implementation notes (as built)

- `sender` is accepted on the wire but deliberately untyped until group-chat
  routing lands (G2); serde ignores it today.
- `secured_router`'s routines scheduler pins every session handle until
  process exit. Embedders needing clean teardown use
  `vak_server::gateway_router(core)` — gateway enabled, no background
  tasks, locks released on drop.
- Restart resilience is structural: bindings live in
  `<home>/gateway/bindings.json`; on a fresh process the first inbound
  reopens the bound ledger through the normal attach path. Proven by
  `bindings_survive_process_restart`.
- Workspace defaults are refreshed from layered config at session admission.
  Explicit CLI/task/heartbeat/subagent pins remain scoped and are never
  overwritten by an admin default refresh.
- The wait long-poll returns the turn chain's final text; if the session was
  busy the reply is `202 steering_queued` and callers follow the SSE stream
  or poll the transcript.

## Test matrix

Covered in `crates/vak-server/tests/`:

1. Roundtrip + reuse + persisted binding (`gateway.rs`,
   `inbound_wait_roundtrip_reuses_binding`).
2. Authz: no bearer → 401 (same test).
3. Unbind semantics + status projection (`unbind_removes_route_and_404s_after`).
4. Busy → logged steering, run completes after consuming it
   (`busy_message_is_steered_not_dropped`).
5. Disabled by default → 409 (`gateway_disabled_by_default_returns_conflict`).
6. Cron → log journal (`cron_task_delivers_summary_to_log_surface`).
7. Cron → webhook receiver, no token configured
   (`webhook_delivery.rs::webhook_delivery_posts_run_output`).
8. Cron → webhook, credential missing → nothing posted, answer still
   recorded (`webhook_missing_token_fails_closed`).
9. Bindings survive a simulated process restart
   (`bindings_survive_process_restart`).
10. Telegram bridge end-to-end against a mock Bot API: update routed,
    final text delivered back, offset advances, quiet polls stay quiet
    (`telegram_bridge.rs`).
11. Forwarded gate: announcement lands on the approver transport, chat
    `yes` resolves it, the tool really executes, pending count returns to
    zero (`gateway_approvals.rs::forwarded_gate_resolves_from_approver_chat`).
12. Unanswered gate times out (5s window); a late `yes` resolves nothing;
    the run still completes without executing the tool
    (`unanswered_gate_times_out_and_fails_closed`).
13. Deny default: no forwarding announcements ever; verdict text from any
    chat is inert; unattended turn completes via auto-deny
    (`default_policy_denies_without_forwarding`).
