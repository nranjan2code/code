# vakcoder

A Rust coding-agent harness. Thesis: **Codex-grade safety, pi-grade
transparency, Claude Code-grade extensibility, opencode-grade simplicity.**

v0.2.0 — the full roadmap (docs/design/00-roadmap.md) is implemented, plus
five hardening/UX phases since (CHANGELOG.md).

## Quick start

```sh
# keys: put them in .env (gitignored) or export directly
export ANTHROPIC_API_KEY=sk-ant-…        # or OPENAI_API_KEY / OPENCODE_API_KEY / GEMINI_API_KEY / Ollama
cargo run --bin vakcoder -- tui                                 # interactive TUI
cargo run --bin vakcoder -- exec "fix the failing test"         # headless
cargo run --bin vakcoder -- config dump                         # effective boot config
cargo run --bin vakcoder -- eval                                # deterministic regression suite
cargo run --bin vakcoder -- eval --live --provider opencode-zen --model x-preview-f-free

# desktop app (first run builds the webview UI)
cd crates/vak-desktop/ui && npm install && npm run build && cd -
cargo run -p vak-desktop
```

For a clean macOS release build, bundle, and user-level install:

```bash
./build-install.sh
```

Use `./build-install.sh --no-clean` for a faster incremental build or
`./build-install.sh --no-install` to create bundles without installing the app.

Offline smoke tests: `scripts/mock_anthropic.py`, `scripts/mock_openai.py`,
`scripts/tui_smoke.py`, `scripts/server_smoke.py`.

## What's inside

| Capability | Notes |
|---|---|
| Multi-provider | Anthropic, OpenAI (responses + completions), OpenRouter, OpenCode Zen (incl. free models), Gemini, Ollama; SSE streaming; delta+snapshot events; abort preserves partials |
| Sessions | Append-only JSONL trees; frozen execution contract header; context derived only from the log (`model-visible means logged`); branch/fork/compact-as-entry |
| Tools | read/write/edit/bash/glob/grep behind one trait and a versioned broker-worker protocol; atomic edits; process-group kill; bounded inputs and outputs |
| Safety | Rule engine (`allow/ask/deny` × read-only/workspace-write/full-access) gates every call. Built-in tools run in disposable workers; MCP servers run as separately sandboxed workers. Restricted file tools are canonical-workspace confined, child environments are secret-safe, permission changes revoke active runs and approvals, and missing containment fails closed. Seatbelt/Landlock restrict local workers; opt-in Docker contains Bash with no network and resource/privilege limits. FullAccess is an explicit unsandboxed trust decision. See [`docs/design/24-agent-security.md`](docs/design/24-agent-security.md). |
| TUI | Full-screen retained terminal workspace with responsive header/transcript/composer regions and settings/help/features modals; markdown + syntax-highlighted code; tool cards with live edit diffs; status row (spinner · elapsed · tokens · ctx% · cost); approval queue with diff previews and always-allow; multiline/paste input, completion, Ctrl-R search; provider/model pickers; /resume + /rewind checkpoints; live-preview themes — ANSI packs (dark/light/neo/rich/teenage/plain) + truecolor packs (midnight/synthwave/forest) + custom `[ui.themes]` definitions; interactive keymap viewer and rebind UI (`/keymap`); Emacs/Vim composer modes (`/composer vim`); subagent attach/steer/stop (`Alt-S`, `/subagents`); opt-in OSC52 copy (`[ui] osc52` + `Alt-Y`/`/copy`); accessibility modes (`/a11y plain/motion/reader`); live subagent streams |
| Extensibility | Skills (progressive disclosure), blocking subagents with lineage-linked child sessions + live attach/steer registry, lifecycle hooks with matcher syntax, MCP client via a lazy meta-tool, custom slash commands as markdown templates (`.vakcoder/commands/`, `plugins/*/commands/`, user `commands/` — `$ARGUMENTS` substitution, palette/completion integration) |
| Agentic depth | Parallel fan-out gated by resource-claim waves; static flow DAGs (validate-before-run, typed failure policy, resume); dynamic planner with bounded replan (fail-closed) |
| Server | HTTP+SSE over the same core: sessions, runs, steering, approvals, transcripts, diffs, fs (workspace-confined), side chats, best-of-N, PR status/merge, scheduled tasks, dev-server launch |
| Desktop | Native Tauri 2 shell over the serve contract (`cargo run -p vak-desktop`): parallel sessions with worktree isolation, streaming chat + inline approvals, diff review w/ line-comment steering, PTY terminal, file editor, @file mentions, best-of-N compare (keep=merge/discard), PR monitor with auto-fix/auto-merge, local scheduled routines, preview pane for dev servers; see docs/design/20-tauri-desktop.md |
| Checkpoints | State-based workspace snapshots at every run start; restore reverts edits and removes post-checkpoint files; git-worktree isolation for exec/plan |
| Reliability | Retry w/ exponential backoff + Retry-After, per-step watchdog, cross-run circuit breaker, session resume (`--session`), server cancel + graceful shutdown |
| Evals | Deterministic in-process suite (~100ms) gated in CI; JSON reports with token/cost accounting |

## Architecture

```
vakcoder (bin: tui · exec · plan · flow · serve · eval · checkpoints · config)
  └── vak-desktop   Tauri 2 native shell (sidecar-free embed of the server contract)
  └── vak-core      SDK facade, system prompt, session bootstrap, checkpoints, worktrees
        ├── vak-agent       loop · steering · parallel tools · subagents · TurnOutcome
        │     ├── vak-tools     Tool trait · built-ins · sandbox backends
        │     ├── vak-hooks     lifecycle hooks (pre/post-tool-use, stop, …)
        │     └── vak-mcp       MCP stdio client (lazy meta-tool)
        ├── vak-flow        static flows · dynamic planner · receipts
        ├── vak-permission  modes × rules → Allow/Ask/Deny
        ├── vak-eval        deterministic eval harness
        ├── vak-server      axum HTTP+SSE wrapper
        └── vak-session     JSONL trees · frozen contract · projection
              └── vak-llm   providers · SSE · EventStream(delta+snapshot)
```

Design invariants live in [AGENTS.md](AGENTS.md); per-subsystem rationale in
[docs/design/](docs/design/).

## Verification

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```
