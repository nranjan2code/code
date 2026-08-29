    An empty
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
18. **Durable services retain workspace identity.** Generated launchd/systemd
    units must execute from the workspace captured by `self services-sync` so
    gateway and Telegram runs load that workspace's config and project `.env`,
    rather than the service manager's default directory. Units must also carry
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
crates/vak      binary: tui / exec / plan / flow / serve [--gateway] /
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
target/debug/vak eval                    # deterministic suite, ~100ms
target/debug/vak eval --live             # real model benchmark
```

## Parallel agents

Only touch files you changed in this session. Sessions are per-cwd-hashed;
never edit another session's files under the data home (`~/Library/Application Support/vak` on macOS, `~/.local/share/vak` on Linux).
