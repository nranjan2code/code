# vakcoder — Agent Engineering Contract

Rules for every AI agent (and human) working in this repository.

## Identity

vakcoder is a Rust coding-agent harness. Thesis: Codex-grade safety, pi-grade
transparency, Claude Code-grade extensibility, opencode-grade simplicity.
When a feature request conflicts with simplicity, resolve it as an extension,
not core.

**Status: v0.2.0+ — all roadmap phases implemented and live-tested.**
See `docs/design/00-roadmap.md` for the phase history and
`docs/design/15-reliability.md` for the failure-handling matrix.

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
7. **Transient provider failures retry, then trip the breaker.** 429/529/network
   errors back off exponentially (`max_retries`, honoring `Retry-After`) under
   a per-step watchdog deadline; if the window outlasts that budget,
   **run-level endurance** (`run_retry_attempts`) re-attempts the same turn
   after cancel-aware backoff — nothing was committed, so the re-attempt is
   exact. Informed transience (429 with Retry-After, explicit overload) feeds
   endurance but does not trip the shared circuit breaker; blind failures
   (network loss, deadlines, truncated/malformed streams) do. An open breaker
   fails fast; endurance paces its waits to the remaining cooldown so the
   half-close probe gets through. Never retry user aborts; never switch
   providers mid-contract.
8. **Secrets never enter git.** API keys live in `.env` (project) or
   `~/.vakcoder/.env` (user), both gitignored, loaded via
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
                     live model discovery (models.rs)
crates/vak-session   append-only JSONL trees, frozen contract, projection
crates/vak-tools     read/write/edit/bash/glob/grep behind Tool trait,
                     resource claims, sandbox backends (Seatbelt)
crates/vak-permission rule engine: modes × rules -> Allow/Ask/Deny
crates/vak-hooks     lifecycle hooks: pre/post-tool-use, stop, session-start
crates/vak-mcp       MCP stdio client behind a lazy meta-tool
crates/vak-agent     loop, steering queues, parallel tool execution w/
                     resource-claim waves, retries + watchdog + circuit
                     breaker + stop gate (premature-completion guard),
                     subagents (task tool) + live SubagentRegistry
crates/vak-flow      static flow DAGs + dynamic planner (bounded replan)
crates/vak-eval      deterministic eval suite + live-model mode
crates/vak-config    layered TOML config + .env secret loading
crates/vak-core      SDK facade, system prompt, checkpoints, worktrees
crates/vak-tui       retained-render terminal UI: contextual keymap +
                     interactive rebind, themes + custom theme packs,
                     vim/emacs composer, subagent attach/steer,
                     custom commands, OSC52 copy, accessibility modes
crates/vak-server    HTTP+SSE wrapper (sessions/runs/approvals/transcripts) +
                     always-on gateway: chat-surface routing, persisted
                     bindings, cron delivery-to-surface (docs/design/22-gateway.md)
crates/vak-desktop   Tauri 2 desktop orchestrator (sidecar over vak-server;
                     docs/design/20-tauri-desktop.md)
crates/vakcoder      binary: tui / exec / plan / flow / serve / eval /
                     checkpoints / config dump / sessions
docs/design/         architecture decisions — update with behavior changes
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
never edit another session's files under `~/.vakcoder`.
