# 31 — Network resilience

The always-on personal OS must survive DHCP renewals, network switching,
outages, hibernation/wake, and full restarts **without any user action**.
"Restart to fix" is a design failure. This doc is the contract; every
network-touching component maps to exactly one of four planes.

## Planes

### 1. Local plane — loopback-only, immune by construction

`serve` binds `127.0.0.1:<port>` (hardcoded); the desktop shell binds
`127.0.0.1:0` ephemeral. TUI/desktop/CLI talk over loopback only.

⇒ Host-IP churn from DHCP or network switching cannot affect any local
surface, ever. Remote access is an explicit user decision (tailscale /
SSH tunnel / reverse proxy) layered *outside* this design — we never bind
wildcard interfaces implicitly. The per-process bearer token stays the
auth boundary; IP allowlists are explicitly NOT part of the model.

### 2. Channel plane — crash-only reconnectors

Long-lived outbound connections (Telegram long-poll today; Discord
gateway per doc 22 extensions) obey:

- **Never exit on transient failure.** No consecutive-failure give-up.
  Backoff is exponential with jitter, capped (30 s), reset on first
  success. Process lifetime ≠ connection lifetime.
- **Resume, don't replay.** Cursor state (Telegram offset) advances only
  on success; ownership probe (`timeout=0, offset=-1`) re-syncs after any
  gap, so restarts and outages neither lose nor duplicate updates.
- **Single-writer arbitration.** Multi-host conflicts degrade to hot
  standby with auto-takeover, never 409 flapping.
- **Liveness is implicit in the protocol**: long-poll timeouts and WS
  heartbeat acks double as keepalives — a dead network surfaces as an
  ordinary transient within one cycle, not as a silent zombie.

Wake-from-sleep collapses into this: the first post-wake tick fails
transiently (or succeeds if the network recovered before userspace), the
backoff resets on success, nothing else changes.

### 3. Inference plane — frozen ladder + endurance + breaker

Provider dispatch already follows invariant 7: typed failure domains,
retry within the committed ladder honoring Retry-After under watchdog
deadlines, run-level endurance when nothing was committed, shared breaker
whose open state fails fast but paces its half-close probe to the
remaining cooldown.

Network events map onto it without new machinery:
- unreachable/refused/DNS-failure ⇒ blind transience (retried; feeds
  breaker like any blind failure)
- mid-stream truncation ⇒ same; receipts record partial attempts
- hibernation ⇒ in-flight deadline fires on wake (tokio timers collapse
  across sleep), attempt is receipted, endurance re-runs the turn
- breaker opens during a long outage ⇒ fail-fast turns get honest typed
  errors instead of hanging; the paced probe lets the very next window
  through once connectivity returns — no restart, no manual reset

Recovery is proven, not assumed: `scripts/fault_proxy.py` sits between
provider and client; the recovery suite drops traffic for a window and
asserts the next turn completes without process restart.

### 4. Delivery plane — store-and-forward, inbox is the floor

Every outbound notification passes the `gateway::deliver_and_record`
chokepoint: transports are best-effort push with bounded retry, while the
inbox entry is durable capture. A channel being down during a fire means
the entry waits unread in the inbox — never lost, never duplicated — and
surfaces pick it up whenever they next connect. Missed cron slots follow
the same philosophy: per-tick evaluation fires each missed slot once
after wake; boot-time catch-up covers full downtime.

## Non-goals

- Platform network-state listeners (SCNetworkReachability et al): redundant
  when every consumer is already self-healing; they add platform surface
  for no behavioral gain.
- Automatic interface rebinding / wildcard binds: the local plane's
  immunity comes precisely from not caring what the host IP is.
- User-facing "reconnect" buttons or restart instructions.

## Regression seams

| Behavior | Test |
|---|---|
| Bridge outlives multi-minute outage, resumes cursor | gateway suite: mock API refuses N polls then serves; assert continued consumption, no exit |
| Inference recovers after outage window | fault_proxy scenario: refuse window ⇒ breaker opens ⇒ proxy restores ⇒ next turn succeeds |
| Wake fires missed slot exactly once | scheduler suite: backdate last_run past a slot boundary, tick once |
| Delivery survives channel-down | P6: zero-transports watchdog lands in inbox (existing) |
