# vakcoder

A Rust coding-agent harness. Thesis: **Codex-grade safety, pi-grade
transparency, Claude Code-grade extensibility, opencode-grade simplicity.**

v0.1.0 — the full roadmap (docs/design/00-roadmap.md) is implemented.

## Quick start

```sh
export ANTHROPIC_API_KEY=sk-ant-…        # or OPENAI_API_KEY / OPENROUTER_API_KEY / Ollama
cargo run                                 # interactive TUI
cargo run -- exec "fix the failing test"  # headless
cargo run -- config dump                  # effective boot config
cargo run -- eval                         # deterministic regression suite
```

Offline smoke tests: `scripts/mock_anthropic.py`, `scripts/mock_openai.py`,
`scripts/tui_smoke.py`, `scripts/server_smoke.py`.

## What's inside

| Capability | Notes |
|---|---|
| Multi-provider | Anthropic + any OpenAI-compatible endpoint (OpenAI, OpenRouter, Ollama, …); SSE streaming; delta+snapshot events; abort preserves partials |
| Sessions | Append-only JSONL trees; frozen execution contract header; context derived only from the log (`model-visible means logged`); branch/fork/compact-as-entry |
| Tools | read/write/edit/bash/glob/grep behind one trait; atomic edits; process-group kill; bounded outputs |
| Safety | Rule engine (`allow/ask/deny` × read-only/workspace-write/full-access) gating every call; macOS Seatbelt sandbox derived from mode; denials feed back as error results the model adapts to |
| TUI | Inline stream-based rendering on native scrollback; live steering while running; y/n approval prompts; slash commands |
| Extensibility | Skills (progressive disclosure), blocking subagents with lineage-linked child sessions, lifecycle hooks with matcher syntax, MCP client via a lazy meta-tool |
| Agentic depth | Parallel fan-out gated by resource-claim waves; static flow DAGs (validate-before-run, typed failure policy, resume); dynamic planner with bounded replan (fail-closed) |
| Server | HTTP+SSE over the same core: sessions, runs, steering, approvals, transcripts |
| Checkpoints | State-based workspace snapshots at every run start; restore reverts edits and removes post-checkpoint files; git-worktree isolation for exec/plan |
| Evals | Deterministic in-process suite (~100ms) gated in CI; JSON reports with token/cost accounting |

## Architecture

```
vakcoder (bin: tui · exec · plan · flow · serve · eval · checkpoints · config)
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
