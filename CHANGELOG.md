# Changelog

## Unreleased — normal-user bring-up

- **Tray launcher**: new "Open VakCoder" and "Open Admin Console…" menu
  items. The admin link reads `runtime/gateway.json`, verifies pid
  liveness, and deep-links with a `#token=` fragment — the SPA trades it
  for the HttpOnly session cookie and scrubs the fragment immediately,
  so one menu click signs in a non-technical user (fragment never
  reaches the network; dead/garbage runtime falls back to the login
  page). Unit-tested URL builder.
- Admin SPA supports `#token=` auto-login; Tavily key now provisioned in
  the canonical `.env` alongside Telegram/provider keys.

## Unreleased — M4.2/M4.3 thin-client completion

- **Desktop live-streaming fixed**: the SPA still attached SSE with
  `?token=` query strings via `EventSource`, which the server stopped
  accepting in the admin-hardening pass — every desktop stream 401'd.
  Streams now ride `fetch` with the bearer header, line-buffered SSE
  parsing, and capped-backoff reconnect; role casing matches the
  lowercase wire (`"user"`), transcript reads tolerate the mid-run
  `{error}` envelope, and the event union gained `HandoffReset` plus
  optional `ContextCompacted` counters.

- **SSE terminal-frame ordering fix**: `RunFinished` was sent directly to
  the session broadcast channel while deltas drained through the async
  bridge, so the terminal frame could overtake un-delivered events.
  All main-session and best-of-N terminals now flow through the ordered
  per-run bridge; cancel prefers it too. New `sse_fanout` integration
  test proves five concurrent subscribers each receive the full turn.
- **TLS guard** (`vak-client::Client::connect`): refuses plaintext
  `http://` to non-loopback hosts — bearer tokens never leave the
  machine unencrypted. Loopback and https unaffected; surfaces migrated.
- **Version handshake**: `GET /version` reports name/version/protocol;
  the TUI records build drift between surface and base on connect and
  shows it in the statusline.
- **Desktop memory writes via the router**: `append_profile_note` no
  longer touches vak-core directly; it POSTs `/memory` (scope=profile)
  through the embedded base like every other surface.
- **Memory wire-contract fixes**: `GET /memory` envelope unwrapping,
  `MemoryNote` fields aligned to the server payload (`ts`/`scope` not
  `created_at`/`tier`), and TUI profile-scope appends actually reach the
  profile tier (`scope` key, not the ignored `tier`).

## Unreleased — M4.2 TUI thin-client rewrite

- **vak-tui rebuilt as a thin client** over the `vak-client` HTTP+SSE
  contract: the 5.8k-line god-file split into 16 focused modules, all
  slash commands ported, adapter deleted, zero agent state in the
  surface. Presentation prefs (themes/keymap/composer/a11y) stay local.
- Server endpoints added: `/config/commands`, `/tools`, `/breaker`,
  `/sessions/{id}/compact`, `/config/sandbox`.
- Fixed pre-existing suite breakages: `Core::cache_home` re-entrant
  mutex deadlock that hung gateway integration tests; heartbeat,
  scheduler and webhook fixtures now boot in gateway mode per the
  singleton invariant; `POST /sessions` accepts an empty body again;
  doctor check-list test updated.

## 0.8.0 — canonical layout release

Platform-standard filesystem locations. One-time automatic migration from
`~/.vakcoder` to `~/Library/Application Support/vakcoder` (macOS) or
`~/.local/share/vakcoder` (Linux). Logs to `~/Library/Logs/vakcoder`,
cache (store.db) to `~/Library/Caches/vakcoder`. Desktop app now ships
in the install bundle with frontend assets.

- **`vak-config::paths` module**: single source of truth for data home,
  cache home, and log directory. `VAKCODER_HOME` override nests everything
  under one directory for tests and portable installs.
- **`user_env_path()` follows data home**: secrets live in
  `data_home()/.env`, not a hardcoded `~/.vakcoder/.env`.
- **Desktop in bundle**: `self install` copies `vak-desktop` binary and
  frontend assets into the app bundle. `Resources/` now contains the
  SolidJS SPA.
- **Stale reference purge**: 36 files changed — all user-facing
  `~/.vakcoder` strings replaced in source code, 19 doc references fixed
  across 16 design docs, AGENTS.md, SECURITY.md, README.md, hosting.md.
- **Legacy migration**: one-time rename of `~/.vakcoder` → canonical data
  home. store.db* → cache. logs → Library/Logs. Skipped when
  `VAKCODER_HOME` is set. Called at all binary entry points.

## 0.7.0 — admin console release

The web admin console: one binary, one URL, full control of a running
vakcoder from any browser (docs/design/33-admin-console.md).

- **`vak-store` crate**: SQLite FTS5 rebuildable index over session JSONL.
  BM25 full-text search with snippets across ALL content blocks (tool
  commands, results, thinking — not just prose), structured metadata
  queries (role/kind/provider/model/date/project), idempotent import,
  WAL mode. JSONL stays the source of truth; the index is disposable and
  rebuilt automatically (startup, post-run, on-demand `refresh=true`).
- **Global event hub + SSE**: typed SystemEvent broadcast (agent runs,
  session lifecycle, config changes, gateway inbound,
  approval requested/granted/denied, security alerts) streamed at
  `/admin/api/events`; browsers reconnect with backoff and lag explicitly.
- **Cookie auth alongside bearer**: `POST /admin/login` validates with
  constant-time comparison and sets an HttpOnly SameSite=Strict cookie —
  EventSource cannot send headers, so this unlocks browser SSE. Login is
  rate-limited; failures land in the security-events log and alert live.
- **Embedded console SPA** (`crates/vak-admin-ui`, SolidJS+Vite, dist
  committed so cargo needs no node): overview with live activity feed and
  stat cards, session catalog → transcripts with role rails/tool badges/
  error highlighting and pagination, global FTS5 search with highlighted
  snippets, color-coded security audit log, inbox with unread badge.
- **Operational console**: answer approval gates from anywhere (args
  preview, live refetch via SSE), edit provider/model, switch permission
  mode via consequence-labeled cards, rebuild the index, cancel runs,
  live-tail transcripts during active runs (session-scoped SSE → debounced
  refresh), sign out.
- **Canonical-client interaction**: composer sends prompts (Enter),
  mid-run sends become steering automatically, ×1–×4 selector fans prompts
  out as best-of-N candidates in isolated worktrees; "+ New session"
  creates sessions from the browser.
- **Security floor** (Phase 0 hardening): per-IP sliding-window rate
  limiting (`[gateway.rate_limit]`), path confinement for tool resolution,
  hook/PTY env allowlisting, SHA-256 trust markers, temp-file atomicity,
  Telegram chat allowlist + sender attribution, append-only security-
  events JSONL surfaced live, config audit trail across all mutation
  endpoints, SECURITY.md disclosure policy.

## Unreleased

- **Base + addons architecture (M0–M3)**: the server is now THE core;
  all surfaces connect over HTTP+SSE. See `docs/design/34-base-addons.md`.
  - **M0 — Concurrency floor**: singleton duties gated behind
    `force_gateway` (cron/replay/rebuild never run in a plain `serve`
    process), SQLite `busy_timeout=5000`, transactional FTS import
    (`BEGIN IMMEDIATE`/`COMMIT`), gateway flock (`libc::flock` on
    `data_home/locks/gateway.lock`), unique `.env.tmp.<pid>` for
    concurrent config mutations. Dead legacy migration code removed (-57
    lines net).
  - **M1 — Identity & discovery**: auto-generated `vk_*` token persisted
    to `data_home/.env` on first gateway boot, runtime file
    (`data_home/runtime/gateway.json`) with pid/token/addr written on
    boot and cleaned on SIGINT/SIGTERM, doctor topology check via PID
    liveness (`kill -0`).
  - **M2 — Connect command**: `vakcoder connect` with 5-step resolution
    (explicit flags → profile → `[connect]` config → runtime file →
    onboarding prompt), `[connect]` config section (url, token, profile,
    profiles map), `--save` persists back to config, token masked in
    output.
  - **M3 — Multi-project base**: `create_session` endpoint accepts
    optional `{"cwd": "path"}` for per-session workspace, `Core::start_session_in(cwd)`
    stamps session header, `list_sessions` scans all project hash dirs
    (not just Core.cwd's).

- **Channel-aware message formatting**: the agent writes GFM markdown once;
  delivery converts per surface. Telegram replies now render as native HTML
  (bold headings/links/inline-code, fences and tables as monospace,
  blockquotes, • bullets) with tag-safe chunking at 4096 chars and an
  automatic plain-text fallback. Future channels add flavors (Slack mrkdwn,
  Discord cards) without touching agent code.
- **Tavily web search via MCP**: stdio servers get `${VAR}` interpolation in
  env values (resolved through the standard secret path — keys stay out of
  config files), a per-server egress flag (`[mcp.servers.X] network =
  true`, privileged) for remote-API tools, and operational env (PATH with
  the server's own dir, HOME/TMPDIR) so npx-based servers run correctly
  under launchd/systemd.

- **TUI `/services`**: status lines plus start/stop/restart for the gateway
  and Telegram bridge via vak-ops.
- **Desktop ▸ Settings ▸ Services**: live status dots with
  Stop/Start/Restart per service and Install/Uninstall for the pair; polls
  every 5 s while open.
- **Desktop ▸ Settings ▸ Learning** (L2): skill-proposal queue with
  Promote/Reject and a recent-notes viewer with provenance.
- **L1 reflection loop**: opt-in `[memory] reflection = true` runs an
  auxiliary call after clean completions proposing ≤2 notes (+ optional
  skill draft), deduped by Jaccard against existing notes; drafts always
  land in the human review queue.
- Gateway-path test fixtures are hermetic against the developer's global
  config so personal defaults (reflection etc.) never leak into CI.

## 0.3.0 — 2026-08-23 · platform release

The always-on platform phase (docs/design/22-gateway.md through 27):
one headless core, many surfaces, durable services.

- **Gateway**: `POST /gateway/inbound` (+wait long-poll) routes chat
  surfaces to persistent sessions; bindings survive restarts; busy turns
  queue as logged steering; unattended approvals fail closed by default or
  forward to an approver surface (`approvals = "forward"`, timeout-deny).
- **Transports**: Telegram bridge (`vakcoder telegram`) and outbound
  webhook targets with fail-closed bearer auth; cron routines push real
  final answers to any surface (`TaskDef.deliver_to`).
- **Media passthrough**: images from chat reach vision models as native
  content blocks (Anthropic/OpenAI/Google/Responses wire shapes); Telegram
  photos auto-download.
- **Memory & recall**: `session_search` tool + `/search` endpoint rank
  curated MEMORY.md notes above transcripts; `remember` persists decisions
  with provenance.
- **Learning loop**: `propose_skill` queues drafts for human promotion via
  HTTP/CLI into user-level skill discovery — never automatic.
- **Docker exec backend**: `[sandbox] backend = "docker"` runs bash in a
  no-network container with the workspace bind-mounted at its real path.
- **Security pass**: broker-worker tool execution, bounded subprocess
  environments, workspace-rooted restricted reads, permission-change
  revocation, threat model in docs/design/24-agent-security.md.
- **Operations**: `scripts/install_gateway_service.sh` installs
  launchd/systemd services; `vakcoder-tray` menu-bar controller with live
  indicators and a watchdog that auto-restarts crashed services;
  `VAKCODER_GATEWAY_TOKEN` pins auth across restarts; hosting guide in
  docs/hosting.md.
- **Reliability fix found in production**: OpenAI-compatible endpoints that
  close tool-call turns without canonical finish reasons no longer strand
  dangling tool calls, and subscriber-less runs are never self-cancelled.

## 0.4.0–0.6.0 (released)

- **Model catalogues are discovered, not hardcoded.** `Core::models_for` — a
  static per-provider table — is gone. `vak_llm::models::list_models` asks the
  provider what the user's key actually reaches (`GET /models`), following
  pagination for Anthropic (`has_more`/`last_id`, 20/page default) and Google
  (`nextPageToken`, 50/page default) so long catalogues are not silently
  truncated; `Core::discover_models` memoises for 5 minutes. Exposed as
  `GET /providers/:name/models`, consumed by the desktop gate, desktop
  Settings and the TUI `/model` picker. Discovery failure reports the reason
  (invalid key, provider down) instead of substituting a stale list — the old
  table advertised 2 opencode-zen models where the key reaches 64, and offered
  4 Anthropic models for a key that no longer authenticates.
- **Provider keys are revocable.** `DELETE /config/key`, a Remove key control
  in desktop Settings, and `/key <provider> --remove` in the TUI. Revoking
  strips the entry from the user `.env`, clears the runtime override and
  the loaded-dotenv copy, drops the cached provider client and discovered
  models, and reports `shadowed_by_env` when the variable is also exported in
  the real environment (which the app cannot unset).
- **Desktop onboarding fixes.** The project gate could strand on the folder
  picker: a `Composer` ref was dereferenced before assignment
  (`ta.selectionStart`), and the throw propagated out of `setBackend` inside
  `refreshBackend`, leaving providers unset. Also: `backend-ready` was emitted
  before the webview subscribed, the gate's recovery poll tore down its own
  timer, `showConnect` captured a non-reactive boolean, and CORS omitted
  PATCH/DELETE while the router and client both used them.
- **Desktop UI**: project switcher and run controls (permission mode,
  transcript density, tokens, context ring) moved from the sidebar footer and
  the status strip into the composer toolbar; sidebar gutters normalised.
- **Single instance**: a second launch refocuses the live window instead of
  starting a rival shell with its own backend.

- **Doc 21 closed out — world-class TUI pass complete.** Interactive
  keymap rebind UI (`/keymap`: ↑↓ select, `r` capture, conflicts surfaced);
  subagent picker + attach/steer (`Alt-S` / `/subagents`): every live child
  registers its steering queues and cancel token in a shared
  `SubagentRegistry`, so Enter steers the child, Tab queues its follow-up,
  Ctrl-C stops only it, Esc detaches.
- **Personalization**: three truecolor theme packs (midnight, synthwave,
  forest) plus custom themes via `[ui.themes.<name>]` (hex or named colors
  over the dark base), all previewed live in the theme picker; Emacs/Vim
  composer modes (`[ui] composer`, `/composer [emacs|vim]`, `/vim`,
  `/emacs`) with normal-mode motions, operator+motion edits, yank/paste
  register, and undo/redo.
- **Custom slash commands**: markdown prompt templates from
  `.vakcoder/commands/*.md` (project), `.vakcoder/plugins/*/commands/*.md`
  (plugin-contributed palette actions), and user `commands/*.md`;
  `$ARGUMENTS` substitution; project > plugin > user precedence; wired into
  completion, the Ctrl-P palette, and `/help`.
- **Opt-in OSC52 clipboard copy** (`[ui] osc52 = true`, then `Alt-Y` or
  `/copy` copies the last response) — never automatic.
- **Accessibility modes** (`[ui.accessibility]`, runtime `/a11y
  plain|motion|reader on|off`): plain/screen-reader strip imposed colors and
  fold box-drawing/decorative glyphs to ASCII in committed history and
  transient panels; reduced motion renders a static spinner glyph.

- **Stop gate** (built-in premature-completion policy, on by default):
  blocks completions that look truncated (trailing plan marker, non-heading
  colon line, unclosed code fence) or that skip verification the prompt
  explicitly demanded with zero commands run. Reuses stop-hook
  continuation (`[stop-guard]` prefix), capped at `max_blocks` per run so
  it can nudge but never trap. `[stop_policy]` config section to tune or
  disable. Born from the dogfood campaign's 5/7 premature-stop rate.
- Workspace-write sandboxes now allow OS temp areas (/tmp plus macOS's
  /var/folders TMPDIR and $TMPDIR): test suites using tempfile/std::env::temp_dir
  no longer die under `cargo test` driven through the agent. Found by
  dogfooding — the sandbox was doing its job a little too well.
- **Learned allow rules**: `[p]` on an approval persists a scoped rule
  (`bash(cargo *)`, `edit(src/x.rs)`, `mcp(server/*)`, …) to
  `.vakcoder/permissions.local.toml`; loaded into every future run in that
  workspace (trusted only), round-trip validated, and unable to shadow
  explicit denies. `[a]` stays session-only for unscopeable calls.
- Sandbox network parity: all sandboxed modes now deny TCP bind/connect —
  Landlock (Linux) handles it explicitly via ABI v4 with fail-closed
  enforcement checks in both read-only and workspace-write; Seatbelt
  (macOS) already denied network implicitly via deny-default profiles.
  FullAccess remains unsandboxed. Smoke scenarios extended to 7.

## 0.2.0 (2026-08-22)

Five post-0.1.0 phases. Headline changes:

### Safety

- **Linux Landlock sandbox** behind the existing `Sandbox` trait: reads and
  execute everywhere, writes only under the canonicalized cwd in
  workspace-write mode, nothing writable in read-only mode. Commands run
  through a hidden `__sandbox` self-exec runner that applies the ruleset
  before spawning the command (children inherit containment). Fail-closed:
  unsupported kernels report sandbox `off` instead of pretending.
  Enforcement smoke (`scripts/landlock_smoke.sh`, 5 scenarios) gates CI's
  new ubuntu job; macOS keeps Seatbelt.
- New dependency: `landlock = "0.4"` — safe wrapper over the Landlock LSM
  syscalls, keeping raw `unsafe` out of the workspace.

### TUI/UX (details in docs/design/18-tui.md)

- Markdown rendering of assistant output with syntax-highlighted fenced
  code; unified diffs for `edit` calls and approval previews; error tails
  on tool failures; thinking indicator.
- Live status row: spinner · elapsed · ↑/↓ tokens · context-window %.
- Approval queue with structured cards (pretty-printed args, rule reason),
  `[y]` allow-once / `[a]` always-this-tool-this-session / `[n·Esc]` deny;
  stray keystrokes can no longer silently answer or deny a pending approval.
- Input: multiline (Alt/Ctrl-J), bracketed paste, wrap- and CJK-aware caret,
  readline keys (Ctrl-U, Ctrl-W, Alt-b/f), Tab completion for slash commands
  and `@file` paths, Ctrl-R reverse history search, persisted history.
- Sessions: `/resume [n|id-prefix]`, `/rewind [seq]` checkpoint browser,
  `/transcript [n]` dump, session browser with first-prompt snippets.
- `/doctor` health panel; `/theme dark|light|plain` runtime switch; bell on
  turn finish; window title.
- Subagents: lifecycle and per-tool-call streams visible live in the parent
  UI; child token usage rolls up into `/cost` and the completed-turn footer.

### Core / reliability

- Steering queues wired through `Core::run_turn_with` — mid-run user input
  is now actually consumed by the loop (previously a UI-side dead end).
- `AgentEvent` carries tool args/result previews; `Approver::approve` sees
  full tool input, enabling informed approval cards.
- `[ui]` config section (`theme`, `bell`); cost estimates from a per-family
  pricing table (unknown models omit dollar figures rather than guessing).

## 0.1.0

Initial release — the full original roadmap:

- Kernel: agent loop + six core tools + append-only JSONL sessions with a
  frozen execution contract (`model-visible means logged`)
- Multi-provider over raw APIs: Anthropic, OpenAI responses + completions,
  OpenRouter, OpenCode Zen (incl. free models), Gemini, Ollama
- Permission engine (rules × modes) + macOS Seatbelt sandbox
- Extensibility: skills, blocking subagents with lineage-linked child
  sessions, lifecycle hooks, MCP client via a lazy meta-tool
- Agentic depth: parallel fan-out gated by resource-claim waves, static flow
  DAGs, dynamic planner with bounded replan (fail-closed)
- Eval harness (deterministic + live-model), HTTP+SSE server, checkpoints/
  rewind, git-worktree isolation
- Reliability pass: retry/backoff honoring Retry-After, per-step watchdog,
  cross-run circuit breaker with run-level endurance, session resume, server
  cancel, graceful shutdown; auto-compaction for long-horizon sessions
