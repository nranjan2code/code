# Changelog

## Unreleased

Rebuilt the install, update, and release lifecycle around one owner, one
version, and verifiable artifacts. Found by exercising every command
against a real prefix rather than reading the code.

### Concurrency

- Fixed a self-deadlock in `Core::cache_home` that hung `cargo test
  --workspace` indefinitely. It locked `sessions_home_override` and then,
  still holding the guard, called `sessions_home()` — which locks the same
  mutex. `std::sync::Mutex` is not reentrant, so the thread wedged. The
  branch is only reached when the override is set, which production never
  does and every test fixture does; all seven tests in
  `crates/vak-server/tests/gateway.rs` blocked on it and now run in 0.35s.
- Removed the idiom that made this possible. `if let Ok(g) =
  slot.lock() && …` keeps the guard alive for the whole body, so the
  hazard is invisible at the call site. All fifteen override accessors now
  go through `Core::read_override` / `write_override`, which clone out
  under a minimal scope, so no guard is ever held across another call.
- Added regression coverage that runs each accessor on a worker thread
  with a deadline. A reintroduced deadlock fails the suite in 10s with a
  message naming the cause, rather than hanging it — a test that hangs
  reports nothing and blocks every gate behind it.
- Fixed `doctor_reports_checks_facts_and_optional_ladder`, which had
  expected four health checks since before the "install layout" check
  joined the ladder in f6131a5. Cargo runs test binaries sequentially, so
  the hang in `gateway.rs` meant this binary never ran and the stale
  assertion stayed invisible for the whole 0.8.0 cycle.

### Install and uninstall

- Removed the second installer. `build-install.sh` copied a Tauri bundle
  to `~/Applications/VakCoder.app` while `self install` managed
  `vakcoder.app` — the same directory on a case-insensitive volume, which
  every macOS default is. The script's `rm -rf` destroyed the manifest of
  a managed install, after which `status` reported nothing installed and
  `uninstall` could not clean up. Placement is now solely `self install`;
  `scripts/build.sh` builds and hands off.
- `--prefix` is accepted by every `self` subcommand, not just `install`.
  Installing to a custom prefix previously left an install that could not
  be inspected, updated, or removed.
- Install, reinstall, and update are transactions. Every file is staged
  and verified before any is placed, and a failure restores the prior
  state — no half-installed prefix, and no window where the CLI is new
  while the tray is still old.
- Added `self verify` and `self reinstall`. The manifest (schema 2) now
  records a SHA-256 per component, so tampering and truncation are
  detected instead of assumed absent. Schema 1 manifests migrate on read.
- Install verifies its own result before reporting success, and reports
  how to put the CLI on PATH when it is not.
- Uninstall names components living outside the prefix that removing the
  prefix will not reach, and no longer treats an absent install as an
  error.

### Update

- Fixed `self update` panicking before it did anything. It builds a
  `reqwest::blocking` client inside the CLI's tokio runtime, which aborts
  with "Cannot drop a runtime in a context where blocking is not
  allowed". The transfer now runs on its own thread, the confinement the
  passive update check already used.
- Fixed version comparison being lexical. `manifest.version <= CARGO_PKG_VERSION`
  compares strings, and `"0.10.0" <= "0.8.0"` is true — the first release
  past `0.9` would have reported "up to date" permanently. Ordering now
  goes through `semver::Version`, with build metadata stripped, because
  the crate's own `Ord` ranks `0.8.0+build.7` above `0.8.0` while semver
  §10 requires build metadata be ignored for precedence.
- Downloaded artifacts are verified against a SHA-256 from the feed
  before anything is written. A feed entry without a digest is refused
  rather than trusted.
- Update replaces every component in the release, not only `vakcoder`.
  Previously the manifest version was rewritten while the tray, desktop,
  and worker stayed on the old build, so `status` reported a clean
  install that was actually mixed-version.
- Update compares against the installed version rather than the running
  build, and gained `--dry-run`.

### Versioning, build, and release

- Restored version singularity per docs/design/32. `tauri.conf.json` and
  both frontend `package.json` files carried their own `0.7.0` stamp
  while the workspace was at `0.8.0`, so the shipped app reported the
  wrong version. Tauri now derives the version from its crate, the
  private frontends are pinned to `0.0.0`, and `scripts/check-version.sh`
  fails if a second stamp reappears.
- Added `scripts/release.sh`, which produces the `release.json` feed that
  `self update` consumes. That feed had a consumer and no producer, so
  the update path could never work end to end. It gates on version
  singularity, fmt, clippy, tests, a clean tree, and an unused tag before
  building, then emits binaries, `SHA256SUMS`, and the feed.
- Added `scripts/bump-version.sh` (one edit plus a lockfile refresh) and
  `scripts/build.sh` (build, install, verify), replacing the root
  `build-install.sh`.

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
