# vakcoder — Agent Engineering Contract

Rules for every AI agent (and human) working in this repository.

## Identity

vakcoder is a Rust coding-agent harness. Thesis: Codex-grade safety, pi-grade
transparency, Claude Code-grade extensibility, opencode-grade simplicity.
When a feature request conflicts with simplicity, resolve it as an extension,
not core.

**Status: v0.3.0+ — all roadmap phases implemented and live-tested.**
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
    verdict-shaped chatter from non-approver chats resolve anything.
16. **Every execution path authorizes before dispatch.** Agent turns, task
    children, static flows, dynamic plans, evals, server runs, and desktop runs
    must use the same permission decision and brokered registry. A direct flow
    node or convenience SDK path may not call an effectful tool before
    evaluating `PermissionEngine` and resolving `Ask` through its approver.

## Code rules

- Edition 2024, stable toolchain. `cargo fmt` + `cargo clippy -D warnings` must pass.
- No comments unless semantics are non-obvious; doc-comment public API items
  whose contracts aren't clear from names (invariants especially).
- No new dependencies without exact versions pinned in the workspace manifest
  and a one-line justification in the PR.
- System prompt stays under 1500 tokens; changes require updating
  `docs/design/07-prompt.md` diff notes.
- Config keys unknown to this version are ignored with a warning, never fatal.

## Layout

```
crates/vak-llm       unified provider API (anthropic / openai-responses /
                     openai-completions / google), SSE, delta+snapshot events,
                     live model discovery (models.rs), work receipts +
                     dispatch ceiling (work.rs), frozen-ladder ordering:
                     demand-scored objectives, belief demotion,
                     cross-model fallbacks (route.rs) -- docs/design/27
                     Phases A+B+R
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
crates/vak-delivery  channel-neutral output contract, safe templates,
                     exact Markdown fallback, ordered chunks, Telegram HTML,
                     isolated renderer worker, and append-only retry outbox
                     (docs/design/30-output-engineering.md)
crates/vak-tools     read/write/edit/bash/glob/grep/webfetch/browse
                     behind Tool trait, versioned broker-worker protocol,
                     bounded subprocess environment, resource claims,
                     sandbox backends (Seatbelt/Landlock)
crates/vak-permission rule engine: modes × rules -> Allow/Ask/Deny
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
 crates/vak-config    layered TOML config + .env secret loading + canonical
                     filesystem paths (paths.rs: data_home, cache_home,
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
                     addressed-gate resolution, semantic adapter registry,
                     Telegram/webhook transports, and outbox replay
                     (docs/design/22-gateway.md, 28-operations.md,
                     29-personal-os.md, 30-output-engineering.md) +
                     admin console: global event hub + SSE, cookie login
                     (HttpOnly SameSite=Strict) alongside bearer auth,
                     /admin/api/* data plane, embedded SolidJS SPA at
                     /admin (docs/design/33-admin-console.md)
crates/vak-admin-ui  SolidJS + Vite admin console source; built dist is
                     committed so cargo builds need no node — observation,
                     operation, and interaction views per docs/design/
                     33-admin-console.md
crates/vak-desktop   Tauri 2 desktop app over an embedded secured_router —
                     SolidJS SPA: sessions, split view, approvals, diff
                     review, subagents tab, MCP manager, image attachments,
                     best-of-N, tasks (cron/script/pin), side chats,
                     memory tier editor, global search, diagnostics,
                     backup/digest cards, budget banner
                     (docs/design/20-tauri-desktop.md, 29-personal-os.md)
crates/vak-ops       service-control layer over launchd/systemd — status,
                     start/stop/restart, install/uninstall shared by tray,
                     TUI and desktop (docs/design/28-operations.md)
crates/vak-tray      menu-bar controller: colour-coded service dot,
                     start/stop/restart/install/uninstall, logs, watchdog
                     with auto-restart + notifications
crates/vakcoder      binary: tui / exec / plan / flow / serve [--gateway] /
                     telegram / eval / checkpoints / config dump / sessions
                     / doctor / backup / digest / tasks / memory / inbox
                     (+ first-run wizard, opt-in update check)
docs/design/         architecture decisions — update with behavior changes;
                     security boundaries and roadmap in 24-agent-security.md
scripts/             dev utilities (mock servers, PTY/HTTP smoke drivers)
```

## Verification before every commit

```
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Live checks (needs API key in `.env`):

```
target/debug/vakcoder eval                    # deterministic suite, ~100ms
target/debug/vakcoder eval --live             # real model benchmark
```

## Parallel agents

Only touch files you changed in this session. Sessions are per-cwd-hashed;
never edit another session's files under the data home (`~/Library/Application Support/vakcoder` on macOS, `~/.local/share/vakcoder` on Linux).
