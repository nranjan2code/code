# Achievements — vakcoder v0.1.x

What has been built, verified, and survived contact with a real model.
Companion to `00-roadmap.md` (plan) and `15-reliability.md` (failure matrix).

## v0.2.0 verification battery (Ox Alpha Free via OpenCode Zen)

Re-run of the live surface after the steering-through-Core change and new
event plumbing: `eval --live` **3/3 PASS**; subagent delegation with
lineage-linked child ledger (read-only tool subset confirmed in header);
`exec --session` resume with full-history projection (12.5K tok in) and a
verified follow-up feature. No panics, no ledger corruption.

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
| TUI/UX pass (docs/design/18-tui.md) | markdown+syntax rendering, tool cards w/ edit diffs, status row, approval queue w/ diff previews + always-allow, multiline/paste input, Tab completion, Ctrl-R search, /resume+/rewind, themes, cost estimates, live subagent streams | PTY smoke 10/10 (`scripts/tui_smoke.py`); steering wired through Core (was a UI-side dead end) |

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

## Long-horizon campaign (Ox Alpha Free, same day)

First real-model exercise of long-context behavior: build a complete
6-feature static site generator (`sitesmith`) in one append-only ledger,
driven by chained `exec --session` resumes.

| Metric | Value |
|---|---|
| Turns | 1 initial + 4 resumes, single ledger |
| Tool calls | 39 |
| Tokens | 168.5K in / 17.4K out |
| Auto-compactions | 3 (first live firing) |

- **Auto-compaction works under real load**: summaries stayed dense, the
  model remained coherent across context resets, no repeated work; ledger
  grew to 80 entries with compaction as entries (never deletion).
- **Model finding (not a harness bug)**: Ox Alpha ends its turn after each
  milestone instead of sustaining a multi-step task solo. Chained resume on
  one session is the working pattern; each resume projected full history
  (9K → 29K → 123K tokens).
- Deliverable verified independently: 30/30 tests, CLI init/build/
  incremental/--full all correct.
- Harness bugs found & fixed en route (see commit bbf4306): planner TOML
  string sanitizer (newlines + nested quotes), truncated-SSE completion for
  OpenAI-compatible proxies, planner-call retries, TUI done-signal on Err.

## Extensibility battery (Ox Alpha Free, same day)

First real-model exercise of hooks, MCP, steering, and the circuit
breaker. All four PASS; no harness bugs found.

| # | Scenario | Result |
|---|---|---|
| X1 | MCP: stdio spawn + `action=list` discovery + `action=call` round-trip | `echo: mcp-live-ok` via fake server |
| X2 | Hooks: pre-tool-use deny, post-tool-use log, stop | deny reason surfaced to model; blocked tool never ran; side-effect log exact |
| X3 | Mid-run steering (TUI PTY) | queued while a tool slept → honored in final reply |
| X4 | Circuit breaker vs always-429 endpoint | run 1: 2 retries / 3 requests / typed failure; run 2 in cooldown: 0 requests, fail-fast with remaining cooldown |

Bonus: X1's model issued `bash` + `mcp` in one turn — parallel
resource-claim waves held under a real mixed workload.

With this battery every major subsystem has survived contact with a real
model: loop, tools, permissions, sandbox config, sessions/resume,
checkpoints, flows, planner, compaction, subagents, evals, server, TUI,
MCP, hooks, steering, breaker.

## Chaos endurance campaign (Ox Alpha Free, same night)

Full-binary fault-injection: a reverse proxy (`scripts/fault_proxy.py`)
sits between vakcoder and Zen injecting failures mid-work while a 10-phase
driver (`scripts/chaos_driver.py`) builds a real package on one append-only
ledger. **Final result: 11/11 PASS** (after harness fixes below).

| Phase | Fault | Result |
|---|---|---|
| P2 | 15×429 rate-limit window mid-feature | bridged by endurance; feature landed, suite green |
| P3 | 12×503 storm during subagent delegation | child + parent both survived |
| P4 | truncated SSE streams (no [DONE]) | adapter completed/retried cleanly |
| P5 | 3s/chunk slow drip vs 75s watchdog | watchdog fired, endurance carried through window |
| P6 | hung upstream vs watchdog | 1 hang absorbed |
| P7 | max_tokens=120 token starvation | graceful MaxTokens stops, no corruption |
| P8 | malformed SSE frames | parse failures retried at run level |
| P9 | perspective probes after everything | 8/8 WikiStore methods recalled in order |

**Harness bugs the campaign found & fixed (all offline-regression-tested):**
1. No run-level endurance — sustained fault windows outlived per-step retry
   budgets and killed whole runs (`run_retry_attempts` added).
2. Informed transience (429+Retry-After / overload) tripped the shared
   breaker mid-window, failing runs the window would have released seconds
   later — breaker now counts only blind failures.
3. Endurance burned its budget on instant circuit-open no-ops — now paces
   waits to the remaining cooldown.
4. Config layer merge dropped `run_retry_*` keys silently (regression test).

CI: deterministic failure-handling regressions gate every PR via
`cargo test` (`tests/run_endurance.rs`, adapter stream tests); the live
campaign runs nightly/manual via `.github/workflows/chaos.yml`.

## Current numbers

- 14 crates, ~21.5K LOC
- 191 tests green; fmt + clippy `-D warnings` clean
- 6 provider families; 4 API wire formats
- Failure matrix fully implemented (retry/watchdog/breaker/resume/cancel)
