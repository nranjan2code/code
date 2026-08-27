# 06 — Research notes: what we took from each harness

Distilled 2026-08 from deep study of Claude Code, Codex CLI, OpenCode, pi,
vakyartha (sibling project), and DeepSeek Harness.

## Claude Code — lifecycle control plane
- ~30 hook events with matcher syntax; 5 handler types. Runtime keeps the
  supported lifecycle surface intentionally small and explicit.
- Skills = markdown content loaded on demand (progressive disclosure).
- CLAUDE.md memory hierarchy → AGENTS.md walk-up merge.
- Lesson: explicit lifecycle controls and permission rules such as `Bash(git *)` are valuable.

## Codex CLI — safety & config engineering
- OS sandbox decoupled from approval policy (read-only / workspace-write /
  full-access).
- Declarative TOML + named profiles per environment.
- Lesson: sandbox ≠ prompts; config is a designed surface.

## OpenCode — architecture & UX simplicity
- Client/server split; TUI is just a client. → SDK-first now, server endpoint
  behind the same seams.
- LSP diagnostics fed back to agent; event-bus plugins.
- Lesson: one headless core, many surfaces; zero-config first run.

## pi — philosophy & internals (deepest lessons)
- ~750-token system prompt; 4 tools cover everything else as extensions.
- JSONL session trees with parent pointers → branch/fork/compact for free.
- Errors-as-values everywhere; delta+snapshot events; AbortSignal plumbed;
  steering+follow-up queues; stream-based TUI with synchronized output.
- Negative space as product decision (refusals are features).

## vakyartha — agentic governance (sibling project)
Borrowed:
- **Frozen execution contract** snapshot at session start (audit/replay from
  frozen state, never current settings).
- **Typed failure policy** (`planning_failed` ≠ empty output ≠ blocked tool);
  retry only the same frozen contract.
- **Planner output = candidates**: repair + validate before execution,
  fail-closed, never substitute fallbacks; bounded replan (1 attempt) seeded
  with settled nodes; hard-failed verify nodes excluded.
- **Resource-claim scheduling**: parallel only when file/artifact/shell claims
  are disjoint.
- **Grants consumed at point of use** for dangerous ops.
Rejected: 8-lane execution sprawl (they consolidated it themselves), dynamic
DAG planning as default for coding work, workbench product surfaces.

## DeepSeek Harness (dsh) — composition discipline
Adopted:
- **"Model-visible means logged" as an enforced invariant** with log-only
  projection (`derive_messages`) — upgraded our sessions from audit trail to
  single source of truth.
- Durable session events vs live waterfall events split.
- Capability seams (definition/provider/consumer) → Rust trait seams; server =
  different provider set over the same seams.
- Runtime modes as compositions; minimal profile doubles as eval baseline.
- Boot-tree introspection (`config dump`).
- Inbox semantics: injected context waits for a real message to admit it.
Product boundary: runtime plugin mounting is intentionally excluded; provider
and tool Rust trait seams provide the supported extension points.

## Meta-lesson
Harness variance beats model variance (Binding-Constraint Thesis evidence:
harness-only changes moved Terminal-Bench 2.0 by 10–14pp). The eval harness
  is a first-class deliverable, not an afterthought.
