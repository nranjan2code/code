# vak — Agent Engineering Contract

Rules for every AI agent (and human) working in this repository.

## Identity

vak is a Rust general-purpose agent harness. Thesis: Codex-grade safety, pi-grade
transparency, Claude Code-grade extensibility, opencode-grade simplicity.
When a feature request conflicts with simplicity, resolve it as an extension,
not core.

**Status: v1.0.0 — all roadmap phases implemented and live-tested.**
Two earlier version lines are retired: the original `0.1.0`–`0.11.51`, and the
short `0.2.0`–`0.2.4` line created by the reset in `fc9c78f`. `1.0.0` sorts
above both, so version ordering is meaningful again and every version in `1.x`
is free; CHANGELOG.md records the history.
See `docs/design/00-roadmap.md` for the phase history and
`docs/design/15-reliability.md` for the failure-handling matrix. Security work
must also follow the threat model and priority order in
`docs/design/24-agent-security.md`. The always-on platform layer follows
`docs/design/22-gateway.md` (gateway/approvals), `docs/design/23-memory.md`
(recall), `docs/design/25-docker-sandbox.md` (execution backends),
`docs/design/26-learning.md` (learning loop),
`docs/design/28-operations.md` with docs/hosting.md for running the stack
as durable services, `docs/design/31-network-resilience.md` for the
four-plane network contract (loopback-only local, crash-only channels,
ladder+endurance inference, store-and-forward delivery), and `docs/design/27-vakyartha-adoption.md` for the
long-horizon program (work receipts + dispatch ceiling ✅, context packet
accounting + deterministic gate ✅, FinOps budget admission ✅,
loop-engineering kernel ✅, frozen-ladder routing ✅, router-grade
ordering over that ladder (demand objectives, cross-model fallbacks,
beliefs) ✅ via the vakrouter study Phase R, runs→flows
adopt/diff ✅, run-graph projection ✅, checkpoint-delta auditing ✅;
remaining (parked): skill intent-discovery, release supply-chain
hardening (SBOM/signing), capacity-exhaustion Ask type — all three
original parked items (scenario-harness, planner done-contracts,
typed tool outputs) have landed). The personal-use completion pass —
tiered memory (USER.md profile + forget/amend), indexed + cross-project
search, cron/watchdog/pinned automation with budget alerts,
doctor/wizard/update-check/backup/digest, SSRF-guarded webfetch +
headless browse, shared markdown transcript export, duplicate-screened
skill proposals, inbox attention layer (P6), heartbeat proactive
check-ins (P7) — is ✅ per `docs/design/29-personal-os.md`
(enterprise deferred). Channel delivery projection now has a typed contract,
isolated renderer, templates, semantic adapter envelope, ordered chunks, and
durable retry outbox per `docs/design/30-output-engineering.md`; native
desktop/TUI block widgets remain presentation-layer extensions.
The web admin console follows `docs/design/33-admin-console.md`:
vak-store FTS5 index (rebuildable, JSONL stays source of truth), global
event hub + SSE, cookie login on the secured router, and an embedded
SolidJS console at `/admin` covering observation (overview/search/
security/inbox), operation (approvals/config/cancel), and interaction
(prompts/steering/best-of-N fan-out) — shipped through Phase 3.
Design docs above 33 cover the work added since: channel onboarding and
multi-bot identity (34), Tavily (35), first-run onboarding (36),
distribution (37), voice and personality (38), the plugin ecosystem (39),
harness engineering lanes (40), the capability registry (41), managed work
contracts (42), governed self-evolution (43), and shared/global
configuration (44). Each carries its own `Status:` line — read it before
assuming a document describes shipped behaviour rather than a proposal.

## Non-negotiable invariants

1. **Model-visible means logged.** Anything that reaches a model request must
   be reconstructable from the session JSONL via `derive_messages()`. New
   model-visible input ⇒ new session entry type. Tests enforce this.
2. **Append-only sessions.** Never rewrite or delete session entries.
   Branching = new entry with `parent_id`. Compaction = an entry, never deletion.
3. **Errors are values.** Tools return `is_error` outputs; providers push typed
   errors into streams; the loop returns `TurnOutcome`. Library code never
   panics on bad input; `unwrap`/`expect`/`panic!` are forbidden outside tests.
4. **Every streaming event carries delta AND snapshot.** Consumers choose their
   abstraction level; never force re-derivation.
5. **Abort preserves partial output.** Cancellation tokens thread through every
   async call; partial results survive.
6. **Unsafe is denied** workspace-wide except process-group kill in
   `vak-tools/src/bash.rs` (annotated).
7. **Transient provider failures retry within the frozen route ladder
   committed at admission.** The ladder is part of the contract, so
   walking it never changes the contract. Never dispatch outside the
   frozen ladder; never switch providers outside it; retries honor
   `Retry-After` under a per-step watchdog deadline; run-level endurance
   re-attempts the same turn after cancel-aware backoff when nothing was
   committed. Informed transience (429 with Retry-After, explicit
   overload) feeds endurance but does not trip the shared circuit breaker;
   blind failures (network loss, deadlines, truncated/malformed streams)
   do. An open breaker fails fast; endurance paces its waits to the
   remaining cooldown so the half-close probe gets through. Never retry
   user aborts.
8. **Secrets never enter git.** API keys live in `.env` (project) or
   the user `.env` at `data_home()/.env` (gitignored), loaded via
   `vak_config::load_env_file/get_var`. Real environment variables take
   precedence over `.env`. Never hardcode, echo, or commit keys. Keys are
   user-supplied and user-revocable: `Core::set_provider_key` /
   `remove_provider_key` own the whole lifecycle, and both invalidate the
   cached provider client and the discovered-model cache.
9. **Model catalogues are discovered, never hardcoded.** The set of models a
   provider offers is a property of the user's key, not of our source tree —
   a baked-in list hides models shipped yesterday and offers ones the key
   cannot reach. `vak_llm::models::list_models` asks the provider
   (`GET /models`, paginated where the provider pages) and
   `Core::discover_models` memoises it for 5 minutes. When discovery fails,
   surface the reason; never substitute a static list. Endpoint *hosts* are
   configuration and may have defaults; model *ids* may not.
10. **Restricted filesystem access is workspace-rooted.** In read-only and
    workspace-write modes, automatic read/glob/grep access must resolve inside
    the canonical session workspace; traversal and symlink escapes fail
    closed. Any exception requires an explicit scoped rule or FullAccess.
    Never weaken this with string-prefix checks or check only one file tool.
11. **Permission changes revoke old capability.** A runtime mode change must
    cancel in-flight main and side runs and reject pending approvals before the
    new mode is reported. Never let an agent continue with a stale FullAccess
    or sandbox snapshot.
12. **Secrets are not ambient tool state.** Bash and MCP subprocesses receive a
    small operational environment allowlist, not the parent environment.
    Provider, gateway, and connector credentials require explicit,
    recipient-scoped injection; never restore blanket environment passthrough.
13. **FullAccess is an explicit human trust decision.** It is unsandboxed by
    design and must never be selected automatically after a denial, tool
    failure, retry, prompt request, or model recommendation. Restricted-mode
    network denial and approval prompts are independent layers; an approval
    never silently disables the OS sandbox.
14. **Model tools cross a broker boundary.** Built-in filesystem and Bash tools
    execute through the versioned `__tool_worker` protocol in a disposable
    process group; MCP servers execute as separately sandboxed workers. Raw
    built-in tool construction is reserved for the worker implementation and
    deterministic unit fixtures. Task orchestration and session search are
    narrow broker-owned capabilities: never give a worker the policy engine,
    approvals, provider credentials, session store, or control-plane handles.
    A missing worker or unavailable restricted sandbox fails closed. A
    command-scoped backend such as Docker must be applied by the broker to the
    validated Bash command; never try to execute a host worker binary inside
    an image that does not contain the pinned worker artifact.
15. **Unattended surfaces fail closed.** The gateway ships disabled, cannot be
    enabled by untrusted project config, and chat-driven turns auto-deny
    escalations unless an explicitly configured approver surface answers a
    forwarded gate inside its window — silence, timeout, or missing delivery
    credential always means no, and a late reply resolves nothing. Never turn
    a denial into permission, forward gates through ambient state, or let
    verdict-shaped chatter from non-approver chats resolve anything. An empty
    `gateway.chat_allowlist` fails closed (rejects every chat) unless the
    operator explicitly sets `chat_allowlist_open = true`; never restore
    "empty means allow all" for compatibility. The live source of truth is
    `<sessions_home>/gateway/allowlist.json` (docs/design/34), seeded once
    from `gateway.chat_allowlist` — an unknown chat lands as a reviewable
    `pending` entry rather than a flat, dead-end rejection; a `denied` entry
    stays sticky and never re-prompts. Inbound channel bridges (Telegram,
    Discord, Slack) build their payload through `InboundChannel` +
    `InboundRequest::new` (`vak-server/src/gateway.rs`), which rejects an
    empty or placeholder `chat`/`sender` — a bridge that reuses one fixed
    identity for every remote user would otherwise defeat the allowlist
    silently. A pooled per-workspace `Core` (multi-tenant CorePool,
    docs/design/34 Phase 2) goes through the exact same trust/permission
    resolution as a local `vak` run in that workspace — pooling never
    grants a channel more access than approval already gated it into.
16. **Every execution path authorizes before dispatch.** Agent turns, task
    children, static flows, dynamic plans, evals, server runs, and desktop runs
    must use the same permission decision and brokered registry. A direct flow
    node or convenience SDK path may not call an effectful tool before
    evaluating `PermissionEngine` and resolving `Ask` through its approver.
17. **Configuration has one contract across every surface.** Persistent
    workspace preferences changed through authenticated `/config` are written
    atomically to `.vak/config.toml` before the live Core override is
    applied. `/health`, `/config`, `/providers`, admin snapshots, and session
    transcripts must report effective values and their source where relevant.
    CLI flags, task pins, heartbeat pins, and subagent pins are scoped
    overrides and must remain visibly non-global. Session provider/model
    contracts freeze at session creation; changing workspace defaults never
    rewrites an existing session or silently changes its dispatch. Provider
    and model are one atomic route at every read/write boundary. New-session
    admission refreshes persisted defaults across processes unless an explicit
    scoped pin is present. Gateway bindings expose route provenance and stale
    reasons; a changed effective route rotates to a new frozen session while
    preserving the old append-only ledger. Persisted max-turns, theme, MCP,
    hooks, and permission mode use the same cross-process refresh; permission
    changes revoke active capabilities before apply — true today for
    `state.sessions` (`apply_permission_mode`, vak-server/src/lib.rs; a
    gateway-registered handle is in that same map, so the *current* turn on
    a channel is reached too) and for the gateway's own default workspace
    (its `CorePool` entry is `state.core` by shared `Arc` identity, not a
    copy). It does **not** yet hold for another workspace the same gateway
    also pools (docs/design/34 Phase 2): a warm, un-refreshed pool entry
    keeps its mode until idle eviction. Pinned down by
    `core_pool::tests::warm_pool_entry_does_not_see_a_permission_mode_change_written_after_it_started`
    — read that test's doc comment (and docs/design/34's "Known
    limitation") before attempting a fix; a straightforward one already
    reproducibly broke `busy_message_is_steered_not_dropped` for reasons
    not yet root-caused.
18. **Durable services use the canonical default workspace.** Generated
    launchd/systemd units for the gateway and channel bridges must execute from
    `vak_config::paths::default_workspace()` (`~/vak-home` for each account),
    never the directory where `self services-sync` happened to run. Per-chat
    workspace overrides remain explicit gateway state. Units must also carry
    the invoking user's non-secret `HOME`; a sanitized manager environment may
    otherwise resolve the workspace as the platform data home and migrate its
    `.vak` directory away. Canonical path resolution must never use the
    current working directory as a missing-home fallback; GUI launches use the
    OS account home and otherwise fail closed to an absolute path.
19. **`doctor` diagnoses; `doctor --repair` only acts on checks with a known
    mechanical fix**, then re-collects and re-prints the report. A check
    with no mechanical fix (provider auth, config warnings) is left for the
    operator — `--repair` must never guess at those or paper over a failure
    it can't actually resolve. `scripts/vak.sh <verb>` is a thin dispatcher
    over `scripts/build.sh` / `scripts/release.sh` / `vak self <verb>` /
    `vak doctor` — it routes, it never reimplements a verb's logic.
20. **Channel capability overlays are restrictive and execution-scoped.** An
    allowlist entry may inherit or narrow built-in tools, MCP server/tool
    patterns, visible skills, and hooks; `None` inherits, `Some([])` blocks
    that category, and deny patterns win. The overlay is persisted with the
    binding, included in the `CorePool` identity, filtered from advertised
    surfaces, and enforced again at MCP call time. Skills are visibility
    controls, not a replacement for permission rules. Secrets remain owned by
    the workspace integration and are never assigned to a channel overlay.
21. **A GET that seeds a same-shape PUT must report only the layer that PUT
    writes, never a merged/effective view.** `GET /config/hooks` and `GET
    /config/mcp` report the *project* `.vak/config.toml` alone, not
    `state.core`'s global+project merge — because their `PUT` always
    replaces the whole project-layer list/map with whatever the matching
    `GET` last returned. Reporting the merged view would silently write an
    inherited global entry into the project file on the very next save. For
    a name-keyed map (MCP servers) that only forks a copy; for a bare `Vec`
    merged by `extend` with no dedup (hooks, before `HookConfig::enabled`
    and this rule existed) it duplicated without bound and re-fired a
    side-effecting command once per copy. Any new config surface with this
    read-then-full-replace shape must follow the same rule. A disabled hook
    is data, not an absence — it stays in the file (`enabled = false`),
    because deleting it on disable is indistinguishable from deleting it on
    purpose, and undoing "disable" must not require retyping the hook.
22. **A workspace/session-scoped mutation and its bulk form must share one
    boundary check.** `sessions_home`-level sidecars (`archive.json`,
    `deleted.json`, the gateway allowlist) are keyed globally by id, not
    per-workspace, but a session's ledger file — and therefore what a
    *single* mutation on it may reach — lives only under this process's own
    `sessions_dir(sessions_home, cwd)`. A bulk endpoint over the same
    sidecar (e.g. delete-all-archived) must re-derive and apply that same
    per-item existence check, not iterate the sidecar's full keyspace on
    the assumption that every key belongs to this workspace.
23. **A bot is its own identity, not a synonym for its surface.**
    Multi-bot-per-channel (docs/design/34 Phase 5) means `telegram`/
    `discord`/`slack` name a *transport*, not a credential slot — more than
    one `Bot` (`vak-server/src/gateway.rs`) can share a surface, each with
    its own token, policy, permission_mode, and route. Capability/permission
    resolution is a three-tier chain, bot → chat → workspace, composed with
    the same override-or-inherit convention every other tier here already
    uses: `ChannelPolicy::merge` for allow/deny (chat's `_allow` wins when
    set else falls back to the bot's; `_deny` lists concatenate, never
    override, since they only remove access), `PermissionMode::capped_by`
    chained twice (chat capped by bot capped by workspace — never
    escalates), and route falling through chat → bot → legacy binding →
    workspace default. `AllowlistEntry.inherit_bot_policy` is the literal
    "break inheritance" switch: `false` skips the bot tier entirely and the
    chat resolves purely against the workspace, same as before bots
    existed. A `Bot`'s token is never returned by the admin API once set,
    matching the legacy per-surface token's own never-shown-again property.
24. **One physical chat served by several bots gets one identity per bot,
    never one shared identity.** A key is `surface:chat` for a legacy/
    single-bot chat, or `surface:chat:bot_id` once a bridge's inbound
    payload names its own bot id (`gateway_inbound`, `vak-server/src/
    gateway.rs`) — the bot id is a third segment, not folded into `chat`,
    so `legacy_key_for` can strip it back off. Two bots in the same
    physical chat therefore get two independent `AllowlistEntry` rows, two
    independent sessions/bindings, and two independently-resolved policy
    chains (rule 23) — never one shared conversation governed by whichever
    bot's row happens to exist. `allowlist_resolve_inbound` is the one
    place a bot-scoped key is minted: seen for the first time, it inherits
    an already-`Allowed` legacy `surface:chat` row's
    workspace/policy/route (a chat approved before bots were scoped into
    the key, or a `chat_allowlist` config row, must not need a needless
    re-approval the moment a bot id starts arriving) rather than starting
    a fresh pending review — this is a real map mutation, not merely a
    read, so it must persist the allowlist file like any other write.
    Outbound delivery mirrors the same rule: `AdapterRegistry`
    (`vak-server/src/delivery.rs`) resolves a bot-scoped target through a
    `(surface, bot_id)`-keyed adapter map, never the single per-surface
    fallback, so a reply is never sent under a different bot's token than
    the one the chat is actually bound to — a target naming an
    unconfigured bot must fail loudly, not silently fall back to some
    other bot's identity. Any new code that builds or parses a delivery
    target or allowlist key must preserve this three-segment shape rather
    than assuming exactly `surface:address`.
25. **Optional integrations never gate turn admission, and service managers
    own recovery.** A configured MCP server is advertised by name during
    prompt construction but is spawned and discovered only when the lazy
    MCP meta-tool is invoked; a missing or wedged integration must not delay
    recording the user message or dispatching the provider, and abandoned
    MCP clients must terminate their child process. For durable services,
    launchd/systemd PID state is process truth and `KeepAlive` is the sole
    automatic restart owner. Health probes describe readiness; a transient
    timeout may notify but must never authorize a competing restart.
26. **The Operations Center is an evidence projection, never a synthetic
    dashboard.** `/ops/center` derives resource state from live session
    handles, gateway/CorePool snapshots, service-manager probes, durable task,
    outbox, security, and health data. Missing or unavailable state remains
    explicit; operational metrics must never be mock, sample, or placeholder
    values. Operations URLs preserve `workspace` and `time` query context, and
    every internal drill-down link carries that context forward. Resource
    detail views stop at raw session JSONL, provider work receipts, durable
    outbox records, incident evidence, or manager state. Incident fingerprints
    reconcile open, resolved, and reopened records in
    `<sessions_home>/operations/incidents.jsonl`; service and delivery actions
    append before/after verification receipts to
    `<sessions_home>/operations/actions.jsonl`. Blocking service probes such
    as `reqwest::blocking` must run on a blocking worker, never inside an async
    request handler. Desktop and tray Operations Center links use the same
    bound listener port and bearer/cookie authentication as the admin console.
27. **User configuration is the base layer; narrower scopes store only
    intent.** The Shared layer (`~/vak-home/.vak/config.toml` plus
    `~/vak-home/.env` and its capability stores) is inherited by every
    workspace. A project layer
    may inherit, replace a same-named item, add an item, or explicitly disable
    inheritance; it must never receive a copied snapshot of effective user
    values. Session, task, bot, chat, and subagent pins resolve after the
    workspace and remain scoped. Admin, Desktop, CLI, and server APIs use the
    same explicit `user`/`project` vocabulary and expose provenance. A GET used
    to seed a write returns that exact layer, never the merged projection.
    Secrets follow the same lookup chain but stay outside TOML: project secret
    → Shared secret → process environment. A project secret must never enter a
    process-global override map where another pooled workspace could observe
    it. Curated integrations are executable definitions backed by real
    packages; the product must not advertise mock, placeholder, or TODO
    capabilities.

28. **The prompt is layered; its safety floor only grows.** The shipped
    prompt is a seed, not a constant (docs/design/45). `identity`,
    `operating_rules`, `guardrails`, and `surface_note` are user-editable and
    resolve through the same seed → Shared → project → surface → bot → chat →
    agent-role chain everything else uses. The capability contract, the
    `Surface:` line, and the skill/MCP inventories are **code-owned**:
    `PromptBlock` cannot name them, so no API can accept an edit to one — they
    describe the callable interface as it actually is, and a user who could
    edit them could only make the model wrong about its own tools.
    `identity`/`operating_rules` fall through narrowest-wins like `route`;
    `guardrails`/`surface_note` concatenate and de-duplicate like
    `ChannelPolicy` deny lists, so nothing narrower can remove what a wider
    layer said, and a surface note appends to the generated line rather than
    replacing it. File presence is the inheritance switch: there is
    deliberately no way to spell `guardrails.inherit = false`. A project layer
    is untrusted config until the workspace is trusted — its identity, rules,
    and surface notes are demoted exactly like `hooks`/`allow`/`mcp.servers`,
    while its **guardrails still apply**, because a guardrail can only narrow.
    Guardrail text instructs and never enforces; `PermissionEngine`, the
    broker, and the sandbox are the boundary, and no surface may word it
    otherwise. Every winning contribution is recorded in
    `FrozenContract.prompt_layers` with a digest, ordered by layer breadth,
    never alphabetically. On resume an implicit binding (a gateway chat)
    rotates and records the drift; a session the user named by id fails closed
    until `--accept-drift`. An empty frozen list means *unknown baseline*, not
    *everything changed*. An inbound message may never write a prompt layer.

## Code rules

- Edition 2024, stable toolchain. `cargo fmt` + `cargo clippy -D warnings` must pass.
- No comments unless semantics are non-obvious; doc-comment public API items
  whose contracts aren't clear from names (invariants especially).
- No new dependencies without exact versions pinned in the workspace manifest
  and a one-line justification in the PR.
- Output presentation is schema-v2 and semantic: `vak-delivery` owns the typed
  `OutputTimeline`/`PresentationDocument` contract, exact Markdown fallback,
  capability projection, and deterministic degradation. Desktop/native clients
  render the closed AST registry; raw HTML and untrusted model-authored UI are
  never mounted. Presentation snapshot/SSE endpoints are reconnectable
  projections over the ledger and live events; legacy transcript, `AgentEvent`,
  webhook, and channel text paths remain compatibility surfaces.
- The shipped prompt seed stays under 1500 tokens and carries its
  `<!-- block: -->` markers; changes require a diff note in
  `docs/design/07-prompt.md`. Layer composition, trust, and the editing
  surfaces are `docs/design/45-prompt-layers.md`.
- A committed or shipped frontend bundle must match its source. `npm run
  build` writes `dist/.src-manifest`; `vak-server`'s and `vak-desktop`'s build
  scripts re-verify it and fail the build naming the stale file. Never
  weaken that check to get a build through — regenerate the bundle.
- Config keys unknown to this version are ignored with a warning, never fatal.

## Layout

```
crates/vak-llm       unified provider API (anthropic / openai-responses /
                     openai-completions / google), SSE, delta+snapshot events,
                     live model discovery (models.rs), work receipts +
                     dispatch ceiling (work.rs), frozen-ladder ordering:
                     demand-scored objectives, belief demotion,
                     cross-model fallbacks (route.rs) -- docs/design/27
                     Phases A+B+R + Gemini Live voice synthesis
                     (google_live.rs): BidiGenerateContent WebSocket
                     session, wall-clock timeout + input-length cap since
                     no upstream deadline exists otherwise, WAV wrapping,
                     WorkPurpose::VoiceSynthesis receipts (docs/design/
                     38-voice-personality.md)
crates/vak-session   append-only JSONL trees, frozen contract, projection,
                      receipt entries (audit-only, projection-neutral),
                      compaction packet partitions (docs/design/27 Phase C),
                      dependency-free cross-session search w/ mtime-indexed
                      cache + cross-project search_all (docs/design/
                      23-memory.md)
crates/vak-store     SQLite FTS5 rebuildable index over session JSONL:
                     BM25 full-text search (all content blocks incl. tool
                     calls/results/thinking), structured metadata queries,
                     idempotent import, WAL mode — docs/design/23 +
                     33 (JSONL stays source of truth)
                     Transport vs. formatting: `vak-server/src/surfaces/`
                     holds the chat-surface TRANSPORT adapters (long-poll /
                     webhook bridges into POST /gateway/inbound), while
                     `vak-delivery`'s same-named modules own the MARKUP
                     projection for each surface. Both used to sit at
                     `src/telegram.rs` in their own crate and read as
                     duplication until you opened both.
crates/vak-delivery  schema-v2 channel-neutral semantic output contract,
                     closed AST/compiler, capability projection, safe templates,
                     exact Markdown fallback, ordered chunks, Telegram HTML,
                     isolated renderer worker, and append-only retry outbox
                     (docs/design/30-output-engineering.md)
crates/vak-tools     read/write/edit/bash/glob/grep/webfetch/browse
                     behind Tool trait, versioned broker-worker protocol,
                     bounded subprocess environment, resource claims,
                     sandbox backends (Seatbelt/Landlock)
crates/vak-permission rule engine: modes × rules -> Allow/Ask/Deny. Deny/Ask
                     match existentially (one bad effect gates the call);
                     Allow is UNIVERSAL — every segment of a compound shell
                     command must be covered, or the call falls through to
                     the mode default. Quote-aware splitting; redirection to
                     a path and command substitution make coverage
                     unprovable. tests/escapes.rs is the standing
                     adversarial corpus and every fix lands a case there.
crates/vak-plugin    plugin packages: manifest parsing, capability
                     declaration (skills/commands/hooks/MCP manifests),
                     workspace- and user-scoped stores, enable/disable/
                     rollback, invocation records, Ed25519 catalog-signature
                     verification with key revocation
                     (docs/design/39-plugin-ecosystem.md)
crates/vak-hooks     lifecycle hooks: pre/post-tool-use, stop, session-start
crates/vak-mcp       MCP stdio client behind a lazy meta-tool
crates/vak-agent     loop, steering queues (full user messages: text +
                     image blocks), parallel tool execution w/
                     resource-claim waves, retries + watchdog + circuit
                     breaker + stop gate (premature-completion guard),
                     spend-gate seam (docs/design/27 Phase D), frozen-ladder
                     leg walk (docs/design/27 Phase B), goal mode + audited
                     completion + regression obligations + handoff reset
                     (docs/design/27 Phase H), subagents (task tool) +
                     parent-scoped SubagentRegistry
crates/vak-flow      static flow DAGs + dynamic planner (bounded replan)
crates/vak-eval      deterministic eval suite + live-model mode +
                     context-quality scorecard (docs/design/27 Phase C)
crates/vak-config    layered TOML config + atomic persisted workspace
                     preferences + .env secret loading + canonical filesystem
                     paths (paths.rs: data_home, cache_home,
                     logs_dir, migrate_legacy_home) + [finops]
                     caps/pricing + [goal] policy + [route] ladder
                     preferences (docs/design/27 Phases D+H+R) +
                     [automation]/[update]/[tools] (docs/design/29)
crates/vak-core      SDK facade, system prompt, checkpoints, worktrees,
                     session_search injection w/ profile tier, memory/
                     skill-proposal tools + duplicate screening
                     (docs/design/26-learning.md, 29 P5), sandbox
                     selection incl. Docker exec backend + runtime backend
                     override (docs/design/25-docker-sandbox.md), manual
                     compaction (compact_session_now), runtime MCP table
                     hot-apply, cost ledger + budget admission gate +
                     alert rows (docs/design/27 Phase D), task store +
                     cron engine, health report, backup export/import,
                     digest, shared transcript_md renderer
                     (docs/design/29-personal-os.md)
crates/vak-server    HTTP+SSE wrapper (sessions/runs/approvals/transcripts/
                     subagent steer-stop/MCP management/memory
                     CRUD/search-all/transcript.md/doctor/backup/
                     digest/inbox-ack endpoints) + always-on gateway:
                     chat-surface routing, persisted bindings,
                     cron+script-watchdog scheduler w/ catch-up and model
                     pinning, heartbeat proactive check-ins, budget-alert
                     delivery-to-surface, approval forwarding with
                     addressed-gate resolution (Telegram forwards render as
                     inline-keyboard buttons via TelegramAdapter, resolved
                     through the same verdict-text path a typed yes/no
                     uses), semantic adapter registry, Telegram document
                     attachments inlined as text (image attachments stay
                     vision content), Telegram/webhook transports, and
                     outbox replay
                     semantic presentation snapshots/SSE alongside legacy
                     AgentEvent and transcript endpoints
                     (docs/design/22-gateway.md, 28-operations.md,
                     29-personal-os.md, 30-output-engineering.md) +
                     multi-bot-per-channel: `Bot` identities independent of
                     surface, bot->chat->workspace policy/permission/route
                     resolution chain, bots.json store, /gateway/bots CRUD
                     (docs/design/34 Phase 5) + per-bot/per-chat voice
                     override riding the same inheritance chain and
                     `inherit_bot_policy` switch as route/permission_mode,
                     POST /voice/speak (docs/design/38-voice-personality.md) +
                     admin console: global event hub + SSE, cookie login
                     (HttpOnly SameSite=Strict) alongside bearer auth,
                     /admin/api/* data plane, embedded SolidJS SPA at
                     /admin (docs/design/33-admin-console.md), including
                     memory CRUD/cleanup, live parent-scoped subagent
                     controls, and the evidence-backed Operations Center
                     (`/ops/center`, durable incidents, action receipts, and
                     bookmarkable resource drill-downs)
crates/vak-admin-ui  SolidJS + Vite admin console source; built dist is
                     committed so cargo builds need no node — observation,
                     operation, and interaction views per docs/design/
                     33-admin-console.md, including the six-area navigation,
                     persistent Operations context bar, and raw-evidence
                     drill-downs, plus a VoiceConfigEditor
                     (inherit-toggle + live Preview button) on the per-bot
                     and per-chat panels (docs/design/38-voice-personality.md)
crates/vak-desktop   Tauri 2 desktop app over an embedded secured_router —
                     SolidJS SPA with schema-v2 outcome-first semantic
                     timeline/AST registry: sessions, split view, approvals, diff
                     review, subagents tab, MCP manager, image attachments,
                     best-of-N, tasks (cron/script/pin), side chats,
                     memory tier editor, global search, diagnostics,
                     backup/digest cards, budget banner
                     (docs/design/20-tauri-desktop.md, 29-personal-os.md) +
                     voice narration of turn completions and approval
                     prompts via a shared `<audio>` element and
                     `/voice/speak` (no native audio crate — the webview
                     plays the returned WAV Blob), Settings toggle + voice
                     picker (docs/design/38-voice-personality.md)
crates/vak-ops       service-control layer over launchd/systemd — status,
                     start/stop/restart, install/uninstall shared by tray,
                     CLI and desktop (docs/design/28-operations.md)
crates/vak-tray      menu-bar controller: colour-coded service dot,
                     start/stop/restart/install/uninstall, logs, watchdog
                     with auto-restart + notifications
crates/vak           binary: exec / plan / flow / serve [--gateway] /
                     telegram|discord|slack [--bot-id <id>] / eval /
                     checkpoints / config dump / sessions / skills /
                     skills-review / plugins / doctor / backup / digest /
                     tasks / memory / inbox / user / workspace / self
                     (install, update, services-sync) (+ first-run wizard,
                     opt-in update check). There is no `tui` subcommand: the
                     inline TUI shipped in v0.1.x and was withdrawn in favour
                     of the desktop and server surfaces.
docs/design/         architecture decisions — update with behavior changes;
                     security boundaries and roadmap in 24-agent-security.md
scripts/             dev utilities (mock servers, PTY/HTTP smoke drivers)
```

## Verification before every commit

```
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
scripts/check-version.sh && python3 scripts/check_doc_paths.py
```

`check-version.sh` covers the stamps a user reads (README badge, CHANGELOG,
git tags), not just the ones the build system reads, and fails on a version
that moves BACKWARDS unless the abandoned line is declared in
`scripts/.version-reset` — a decreasing version makes every install on the
higher line permanently un-updatable. `check_doc_paths.py` fails on a design
doc citing a path that no longer exists; docs whose `Status:` line says
"proposal" are skipped, because their paths are targets rather than
citations.

Live checks (needs API key in `.env`):

```
target/debug/vak eval                    # deterministic suite, ~100ms
target/debug/vak eval --live             # real model benchmark
```

## Parallel agents

Only touch files you changed in this session. Sessions are per-cwd-hashed;
never edit another session's files under the data home (`~/Library/Application Support/vak` on macOS, `~/.local/share/vak` on Linux).
