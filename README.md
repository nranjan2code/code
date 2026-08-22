# vakcoder

A Rust coding-agent harness. Thesis: **Codex-grade safety, pi-grade
transparency, Claude Code-grade extensibility, opencode-grade simplicity.**

v0.2.0 — the full roadmap (docs/design/00-roadmap.md) is implemented, plus
five hardening/UX phases since (CHANGELOG.md).

## Quick start

```sh
# keys: put them in .env (gitignored) or export directly
export ANTHROPIC_API_KEY=sk-ant-…        # or OPENAI_API_KEY / OPENCODE_API_KEY / GEMINI_API_KEY / Ollama
cargo run                                 # interactive TUI
cargo run -- exec "fix the failing test"  # headless
cargo run -- config dump                  # effective boot config
cargo run -- eval                         # deterministic regression suite (coding + non-coding scenarios)
cargo run -- eval --live --provider opencode-zen --model x-preview-f-free  # live (free model)

# desktop app (first run builds the webview UI)
cd crates/vak-desktop/ui && npm install && npm run build && cd -
cargo run -p vak-desktop
```

Offline smoke tests: `scripts/mock_anthropic.py`, `scripts/mock_openai.py`,
`scripts/tui_smoke.py`, `scripts/server_smoke.py`.

## What's inside

| Capability | Notes |
|---|---|
| Multi-provider | Anthropic, OpenAI (responses + completions), OpenRouter, OpenCode Zen (incl. free models), Gemini, Ollama; SSE streaming; delta+snapshot events; abort preserves partials |
| Sessions | Append-only JSONL trees; frozen execution contract header; context derived only from the log (`model-visible means logged`); branch/fork/compact-as-entry |
| Tools | read/write/edit/bash/glob/grep behind one trait; atomic edits; process-group kill; bounded outputs |
| Safety | Rule engine (`allow/ask/deny` × read-only/workspace-write/full-access) gating every call; learned allow rules persisted per-workspace (`[p]` on an approval → scoped rule); OS sandbox per platform — macOS Seatbelt, Linux Landlock (5.13+, fail-closed probe), network denied while sandboxed; built-in stop gate blocks premature/truncated completions (config-gated); denials feed back as error results the model adapts to. On other platforms containment is the permission engine alone (`serve` also requires a per-process bearer token printed at startup) |
| TUI | Inline stream-based rendering on native scrollback; markdown + syntax-highlighted code; tool cards with live edit diffs; status row (spinner · elapsed · tokens · ctx% · cost); approval queue with diff previews and always-allow; multiline/paste input, Tab completion, Ctrl-R search; /resume + /rewind checkpoints; themes; live subagent streams; slash commands |
| Extensibility | Skills (progressive disclosure), blocking subagents with lineage-linked child sessions, lifecycle hooks with matcher syntax, MCP client via a lazy meta-tool |
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
