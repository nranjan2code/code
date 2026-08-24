# 22 — Gateway: always-on surfaces + cron delivery

Closes the platform gap versus OpenClaw / Hermes-agent (see research notes in
`06-research-notes.md`): both are *always-on personal agent platforms* whose
core differentiator is a gateway that routes many chat surfaces to persistent
agent sessions, plus unattended scheduled work with delivery back out.

vakcoder's structural advantage: **the core was always headless.** TUI and
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
  `$VAKCODER_HOME/locks/telegram-<fnv(token)>.lock` records holder
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

One `Core` per workspace; N concurrent routed sessions above it; sessions
remain append-only JSONL trees keyed per-cwd exactly as before.

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

- `POST /gateway/inbound` `{surface, chat, sender?, text, wait?}` →
  - `202 {state:"started"|"steering_queued", session_id}` or, with
    `"wait": true`, `200 {state:"completed", text, session_id}` after the
    turn finishes (240s cap). A curl one-liner is a complete adapter:
    message in, final assistant text out. Streaming consumers use the
    existing per-session SSE instead.
- `GET /gateway/status` → enabled flag + binding table.
- `DELETE /gateway/bindings/{key}` → unbind (URL-encoded `surface:chat`).

A **binding** maps a routing key to a session:

```
routing key = "{surface}:{chat}"        e.g. "telegram:48211", "webhook:ci-alerts"
binding     = key -> session_id          persisted at <home>/gateway/bindings.json
```

First message from an unknown key creates a fresh session (normal frozen-
contract header, normal JSONL); later messages resume that session across
gateway restarts. Deleting a session through the normal endpoint leaves the
binding stale; the next inbound detects the missing ledger and rebinds fresh.

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
  through to ordinary conversation routing and resolves nothing;
- the `approval_timeout_secs` window lapses (default 300s, minimum 5s) —
  deny, and the gate is retired so a late reply resolves nothing;
- the run is cancelled — deny.

Verdict-shaped chatter from any non-approver chat never touches the gate
queue; in `deny` mode nothing is announced at all.

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
  - `webhook:<name>` — POST `{target, text, ts}` JSON to a URL configured
    under `[gateway.outbound.webhooks.<name>]`; optional `token_env` names
    an env var whose value is attached as a bearer token at delivery time.
    A configured-but-missing credential fails the delivery **closed** (the
    run's answer is still recorded on the task) rather than posting
    unauthenticated.

### Channel formatting

The agent emits GitHub-flavored markdown; delivery converts per surface
(`crates/vak-server/src/channels.rs`):

| Surface | Flavor |
|---|---|
| Telegram | HTML (`parse_mode=HTML`) — headings→bold, links, inline code; fences/tables→monospace `<pre>`; bullets→•; model-emitted HTML escaped first |
| webhook / log | raw markdown (machines) |

Rules: conversion is injection-safe (escape before translate); long messages
chunk at paragraph boundaries without cutting tags; a chunk rejected by the
channel (400) is resent stripped to plain text — degradation is ugly, never
lost. New channels implement a flavor + adapter; agent code unchanged.

## Configuration

```toml
[gateway]
enabled   = true      # default false — remote execution must be opt-in
approvals = "deny"    # "deny" | "forward"
approver  = "telegram:48211"   # required for "forward"; <surface>:<chat>
approval_timeout_secs = 300    # min 5

[gateway.outbound.webhooks.ci]
url       = "https://ci.example.com/vakcoder"
token_env = "CI_HOOK_TOKEN"   # name only; value resolved per delivery
```

- Unknown keys warn-not-fail per house convention (`KNOWN_GATEWAY_KEYS`,
  `KNOWN_OUTBOUND_KEYS`, `KNOWN_WEBHOOK_KEYS`).
- `[gateway]` is a **privileged section**: stripped entirely from untrusted
  project config (enabled gates remote execution; outbound URLs are exfil
  targets).
- `serve --gateway` forces `enabled` regardless of config (CLI override).

## Channel bridges

Bridges are thin clients of the HTTP contract and can run anywhere the bot
API and the gateway are both reachable:

```bash
vakcoder serve --gateway --trust          # process 1: hosted core
TELEGRAM_BOT_TOKEN=123:abc \
vakcoder telegram --server http://10.0.0.5:8901 --token vk_...   # process 2
```

The Telegram bridge long-polls `getUpdates`, routes each text through
`/gateway/inbound` with `wait`, and answers via `sendMessage` (chunked at
Telegram's 4096-char cap on newline-safe boundaries). Non-text updates
advance the offset without routing so they are never replayed. Transient
failures back off 3s; ten consecutive failures give up with a clear error.
`TELEGRAM_API_BASE` overrides the API host for self-hosted relays and tests.
Tokens live in `.env` / `~/.vakcoder/.env`, never in config or flags.

## Security posture

Lessons from OpenClaw's incident history, inverted:

1. Bearer-token auth on every route (existing middleware), `/health` open.
2. Gateway **off by default**; enabling requires trusted config or CLI flag.
3. Untrusted projects cannot enable the gateway via project config.
4. Unattended turns auto-deny escalations; no silent yes.
5. Bindings file lives under `<sessions_home>` next to tasks.json — same
   trust domain as session ledgers, no secrets inside (channel tokens belong
   in `.env` at the adapter layer, never here).

## Invariants mapping

| Invariant | Status |
|---|---|
| Model-visible ⇒ logged | inbound texts enter as prompt or steering entries |
| Append-only sessions | untouched — bindings reference, never rewrite |
| Errors are values | inbound validation + delivery return typed errors; turn failures deliver `is_error` summaries |
| Abort preserves output | `/cancel` still works per session; partial text delivered on abort |
| No unsafe | none added |

## Phases

| Phase | Delivers | Exit criterion |
|---|---|---|
| **G0 ✅** | HTTP inbound + wait mode, bindings persistence, steering-busy path + continuation, AutoDeny unattended turns, log surface, `TaskDef.deliver_to`, `[gateway]` config, `serve --gateway` | e2e tests: roundtrip, reuse, busy-steering, disabled-gateway 409, cron delivery; fmt/clippy/tests green |
| **G1 ✅** | Outbound webhook transport (`webhook:<name>`) with fail-closed bearer auth; Telegram bridge client + `vakcoder telegram` subcommand; real-text routine summaries; restart-resilience proof; scheduler-free `gateway_router()` for embedders | offline e2e vs mock Bot API and webhook receiver; missing-credential fail-closed test; bindings survive simulated process restart |
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
