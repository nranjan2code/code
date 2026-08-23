# Changelog

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

## Unreleased

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
  strips the entry from `~/.vakcoder/.env`, clears the runtime override and
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
