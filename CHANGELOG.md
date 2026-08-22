# Changelog

## Unreleased

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
