# 00 — Roadmap

Phases with exit criteria. Each phase ships a usable product.

| Phase | Delivers | Exit criterion |
|---|---|---|
| 0 | Scaffolding, CI, AGENTS.md, design docs | workspace builds green in CI |
| 1 ✅ | Kernel: loop + Anthropic + 6 tools + JSONL sessions + frozen contract + `exec` mode | offline mock e2e: prompt → tool call → result → final answer, fully auditable JSONL |
| 2 ✅ | Inline TUI (stream-based, native scrollback), steering input, slash commands | PTY-driven smoke: live streaming, tool status, steering queue, /commands; editor+keys+command unit tests |
| 3 ✅ | Permission engine (rules × modes), OS sandbox backends (Seatbelt; Landlock later) | allowed vs denied calls audited in JSONL; seatbelt blocks $HOME escapes, allows cwd writes |
| 4 (openai-completions ✅) | Multi-provider: OpenAI/OpenRouter/Ollama live; next: openai-responses + Google | same session history converts across providers; mock e2e per family |
| 5 ✅ | Extensibility complete: skills, blocking subagents, hooks, MCP client (lazy meta-tool) | fake-server roundtrip tests + e2e mcp list through the binary |
| 6 ✅ | Agentic depth complete: fan-out scheduler, static flows, dynamic planner + bounded replan | plan/execute/replan/fail-closed all tested; e2e through binary |
| 7 ✅ | Phase 7 complete: eval harness, HTTP+SSE server, checkpoints/rewind, worktree isolation | all slices tested + e2e through the binary |
| post-v0.1.0 ✅ | Reliability pass: retry+backoff, watchdog, session resume, server cancel, graceful shutdown, circuit breaker | failure matrix in docs/design/15-reliability.md |
| post-v0.1.0 ✅ (2) | OpenCode Zen provider + .env secrets; live-model evals; auto-compaction for long-horizon sessions (docs/design/17-context.md) | 10/10 Ox Alpha dogfood battery; compaction overflow tests; 134 tests green |
| post-v0.1.0 ✅ (3) | TUI/UX pass: markdown+syntax rendering, tool cards w/ diffs, live status row, approval queue+always-allow, multiline+paste input, completions, /resume+/rewind, themes (docs/design/18-tui.md); steering wired through Core | PTY smoke 10/10 incl. approval flow; workspace fmt/clippy/tests green |
| post-v0.1.0 ✅ (4) | TUI slices 2–3: approval diff previews, readline editing (Ctrl-U/W, Alt-b/f), thinking indicator, subagent lifecycle+tool streams, /theme runtime switch, cost estimates, Ctrl-R history search, session browser with first-prompt snippets, /doctor + /transcript | 191+ tests green; PTY smoke 10/10; clippy -D warnings clean |
| post-v0.1.0 ✅ (5) | Linux Landlock sandbox backend (safe `landlock` crate; self-exec `__sandbox` runner, fail-closed kernel probe); subagent token rollup into /cost; CI ubuntu job incl. landlock smoke | darwin+linux clippy/test green; scripts/landlock_smoke.sh 5/5 on ubuntu |
| post-v0.1.0 ✅ (6) | Dogfood + capability gauntlet campaigns: stop-gate built from findings, sandbox temp-dir fix, learned allow rules, exec/TUI output parity; architecture ladder + extensibility battery + live brownout chaos | docs/design/16-achievements.md gauntlet sections; all live probes verified independently |

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

## Post-v0.1.0 achievements

See `16-achievements.md` for the full table. Highlights: OpenCode Zen
provider (free Ox Alpha model), live-model eval mode, reliability pass
(retry/watchdog/circuit-breaker/resume/cancel), and a 10/10 live dogfood
battery — the first real-model exercise of the entire stack.
