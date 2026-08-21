# 00 — Roadmap

Phases with exit criteria. Each phase ships a usable product.

| Phase | Delivers | Exit criterion |
|---|---|---|
| 0 | Scaffolding, CI, AGENTS.md, design docs | workspace builds green in CI |
| 1 ✅ | Kernel: loop + Anthropic + 6 tools + JSONL sessions + frozen contract + `exec` mode | offline mock e2e: prompt → tool call → result → final answer, fully auditable JSONL |
| 2 ✅ | Inline TUI (stream-based, native scrollback), steering input, slash commands | PTY-driven smoke: live streaming, tool status, steering queue, /commands; editor+keys+command unit tests |
| 3 | Permission engine (rules × modes), OS sandbox backends (Seatbelt/Landlock) | safe unattended runs with audit trail |
| 4 | Multi-provider (OpenAI-responses, Google, OpenRouter/Ollama), lazy loading, mid-session switching | same session switches providers without breaking history |
| 5 | Extensibility: hooks+matchers, skills (progressive disclosure), MCP client (lazy tools), blocking subagents | one real Claude Code skill + one MCP server work unchanged |
| 6 | Agentic depth: parallel fan-out w/ resource-claim scheduler, static flows, dynamic planner + bounded replan | 3-way parallel refactor across disjoint modules beats serial; planned task survives mid-run failure |
| 7 | Server mode (HTTP+SSE), checkpoints/rewind, worktree isolation, eval harness | Terminal-Bench-style suite runs nightly; regressions block releases |

## Decisions locked during research (2026-08)

- Rust, SDK-first, server optional later
- Full permission system with prompts (Claude Code-style rules)
- Multi-provider from day 1 via raw provider APIs (no meta-SDK)
- Static flows + dynamic planner (vakyartha-style planner→repair→validate,
  fail-closed `planning_failed`, bounded replan = 1 attempt)
- Full parallel subagent fan-out gated by resource claims
- DeepSeek-Harness lessons adopted: model-visible-means-logged invariant,
  durable-vs-live event split, capability seams as traits, minimal profile as
  eval baseline, boot-tree introspection (`config dump`)
