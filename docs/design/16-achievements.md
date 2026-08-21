# Achievements — vakcoder v0.1.x

What has been built, verified, and survived contact with a real model.
Companion to `00-roadmap.md` (plan) and `15-reliability.md` (failure matrix).

## Harness milestones

| Milestone | Evidence |
|---|---|
| Walking skeleton: loop + tools + JSONL sessions + `exec` | offline mock e2e, phase 1 |
| Inline TUI with live steering + approvals | PTY smoke 8/8 (`scripts/tui_smoke.py`) |
| Permission engine (rules × modes) wired end-to-end | allowed vs denied calls audited in session JSONL |
| Seatbelt OS sandbox | blocks `$HOME` escapes at kernel level; allows cwd writes (execution tests) |
| Multi-provider: Anthropic, OpenAI-responses, OpenAI-completions (OpenAI/OpenRouter/Ollama/OpenCode Zen), Gemini | per-family mock-stream suites + binary e2e against mock servers |
| Skills · subagents · hooks · MCP client | Claude-Code-skill compatible; child-session lineage tests; hook unit+agent tests; fake-MCP roundtrip |
| Resource-claim wave scheduler | timing-proven: disjoint writers overlap (~0.5s), conflicting writer serialized (~1.0s vs 1.5s serial) |
| Static flows + dynamic planner | validation/layers/failure-policy/resume tests; plan→execute→replan→fail-closed matrix |
| Eval harness | deterministic suite CI-gated (~100ms); live-model mode |
| HTTP+SSE server | lifecycle/approvals/transcript e2e over real HTTP |
| Checkpoints/rewind + worktree isolation | capture/restore incl. deletions; worktree lifecycle tests |
| Reliability pass | retry+backoff, watchdog, session resume, server cancel, graceful shutdown, cross-run circuit breaker |

## Live dogfood campaign (Ox Alpha Free via OpenCode Zen)

First real-model exercise of the full stack. **10/10 battery PASS.**

| # | Scenario | Result |
|---|---|---|
| T1 | write fizzbuzz.py + run + verify | PASS |
| T2 | multi-file package (4 files) + iterate to ALL PASS | PASS (13.2K tok in) |
| T3 | fix 3 planted bugs until tests pass | PASS — error-recovery cycle worked |
| T4 | cross-file class rename, zero stale refs | PASS |
| T5 | resume same ledger, coherent follow-up feature | PASS (11.2K tok in = full history projected) |
| T6 | checkpoint auto-capture ×2 + restore | revert + stray-file removal exact |
| T7 | 3-layer flow (bash → agent → merge), live model in the loop | PASS |
| T8 | subagent delegation via `task` tool | PASS — child session lineage recorded |
| T9 | 3000-line log analysis, exact counts, self-verified | PASS |
| T10 | live eval suite re-run | stable 3/3 |

**Harness bugs found & fixed by the campaign:** `flow run` lacked
`--provider`/`--model` selection (failed closed correctly; flags added and
applied before provider resolution). Everything else held: no panics, no
ledger corruption, token accounting accurate across all runs.

## Current numbers

- 15 crates, ~14.5K LOC
- 129 tests green; fmt + clippy `-D warnings` clean
- 6 provider families; 4 API wire formats
- Failure matrix fully implemented (retry/watchdog/breaker/resume/cancel)
