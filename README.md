# vakcoder

A Rust coding-agent harness. Thesis: **Codex-grade safety, pi-grade
transparency, Claude Code-grade extensibility, opencode-grade simplicity.**

## Status

Phase 1 walking skeleton (see `docs/design/00-roadmap.md`):

- unified provider API with SSE streaming, delta+snapshot events, typed errors,
  abort-with-partial (`vak-llm`)
- append-only JSONL session trees with frozen execution contract and log-only
  context projection (`vak-session`)
- read/write/edit/bash/glob/grep behind one `Tool` trait; atomic edits;
  process-group kill on timeout/cancel (`vak-tools`)
- agent loop with steering queues, parallel tool execution, errors-as-values
  (`vak-agent`)
- layered TOML config, `config dump` boot introspection (`vak-config`)
- SDK facade + `exec` headless mode (`vak-core`, `vakcoder`)

## Quick start

```sh
export ANTHROPIC_API_KEY=sk-ant-…
cargo run -- exec "explain what this repo does"
cargo run -- config dump
cargo run -- sessions
```

Offline smoke test against the mock server:

```sh
python3 scripts/mock_anthropic.py 8791 &
ANTHROPIC_API_KEY=test VAKCODER_ANTHROPIC_BASE_URL=http://127.0.0.1:8791 \
  cargo run -- exec "run the smoke test"
```

## Architecture

```
vakcoder (bin: exec / config / sessions)
  └── vak-core      SDK facade, system prompt, session bootstrap
        ├── vak-agent     loop · steering · parallel tools · TurnOutcome
        │     ├── vak-tools     Tool trait · read/write/edit/bash/glob/grep
        │     └── vak-session   JSONL trees · frozen contract · projection
        │           └── vak-llm     providers · SSE · EventStream(delta+snapshot)
        └── vak-config    defaults < global < project < env
```

Design invariants live in `AGENTS.md`; rationale in `docs/design/`.
