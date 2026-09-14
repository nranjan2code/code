# 00 — Roadmap
Status: historical

**Baseline: 2.0.0.** Versions before it are unsupported and not described
here (`docs/design/46-stabilization-install-and-onboarding.md`, Part VII.1).
This file states what is true now and what is planned next. It is not a
history — git and `CHANGELOG.md` hold that.

## What the runtime does today

Every item below is implemented, tested, and live. Each links to the doc
that owns its contract.

| Capability | Contract |
|---|---|
| Agent loop, frozen route contract, append-only JSONL sessions | `docs/design/03-agent-loop.md`, `docs/design/02-sessions.md` |
| Multi-provider dispatch with discovered model catalogues | `docs/design/01-llm.md` |
| Permission engine × modes, OS sandbox backends, brokered tool boundary | `docs/design/08-permissions.md`, `docs/design/24-agent-security.md` |
| Docker command-scoped containment | `docs/design/25-docker-sandbox.md` |
| Skills, subagents, hooks, MCP, plugins, custom commands | `docs/design/09-extensibility.md`, `docs/design/39-plugin-ecosystem.md` |
| Static flows, dynamic planner, run-graph projection | `docs/design/10-flows.md`, `docs/design/11-planner.md` |
| Eval harness, checkpoints/rewind, worktree isolation | `docs/design/12-evals.md`, `docs/design/14-checkpoints.md` |
| HTTP+SSE server, and one workspace client over it in three hosts — Tauri shell, browser on loopback, browser against a headless box | `docs/design/13-server.md`, `docs/design/20-tauri-desktop.md`, `docs/design/48-web-client.md` |
| Gateway, chat surfaces, channel governance, multi-bot identity | `docs/design/22-gateway.md`, `docs/design/34-channel-onboarding.md` |
| Web admin console | `docs/design/33-admin-console.md` |
| Memory, cross-session search, learning loop | `docs/design/23-memory.md`, `docs/design/26-learning.md` |
| Reliability: retry, watchdog, breaker, endurance, resume | `docs/design/15-reliability.md`, `docs/design/31-network-resilience.md` |
| Work receipts, FinOps admission, dispatch forensics | `docs/design/42-managed-work-contracts.md` |
| Layered configuration and scoped capabilities | `docs/design/05-config.md` |
| Layered, editable prompt | `docs/design/45-prompt-layers.md` |
| Intent kernel: behavioural reading, narrowing engagement, durable commitments | `docs/design/47-commitment-kernel.md` |
| Outcome-directed runtime: bounded OutcomeSpec, evidence-aware evaluation, collaborative goal updates, and result-scoped cross-surface presentation | `docs/design/52-outcome-directed-runtime.md`, `docs/design/30-output-engineering.md` |
| Service control plane, Operations Center | `docs/design/28-operations.md` |
| Installed lifecycle: install, verify, status, update, uninstall | `docs/design/32-release-engineering.md` |
| Agent-owned conversations, audience-scoped memory, channel endpoint routing, lifecycle-gated admission, and durable delivery provenance | `docs/design/64-agent-owned-platform.md` |

## What 2.0.0 delivers

The runtime was finished before anyone outside the project could install it.
2.0.0 closes exactly that: one bundle, one install, one first run, and a
forward contract that keeps every later release non-destructive.

Phases S0–S11 with exit criteria live in
`docs/design/46-stabilization-install-and-onboarding.md`, Part X. In short:

| Phase | Delivers | State |
|---|---|---|
| S0 | Baseline declared; record cleaned | done |
| S1 | Install split from setup; derived readiness projection | done |
| S2 | The setup server and the web onboarding wizard | done |
| S3 | `vak setup --terminal` parity | done |
| S4 | Desktop adopts the wizard; selection stops implying trust | done |
| S5 | Durable state registry | done |
| S6 | The upgrade gate | done |
| S7 | DMG, tarball, bootstrap script | done |
| S8 | CI release pipeline and supply-chain evidence | done |
| S9 | Uninstall parity | done |
| S10 | Signing and notarization | **blocked**: needs an Apple Developer ID. The artifact verifier is built and runs; the certificate is not available |
| S11 | Windows | **not started**: a platform project (paths, a service backend, a sandbox backend), not a packaging step |

## After 2.0.0

Nothing here may break an existing install; the contract in doc 46 Part VII
applies to every item.

| Work | State |
|---|---|
| Managed work contracts — hardening and outcome-linked progress | shipped in 3.0.10; direct mode unchanged |
| Discord and Slack real-time transports (gateway websocket, Socket Mode) | deferred; both bridges poll configured channel ids today |
| Interactive approval components on Discord and Slack | deferred; typed yes/no fallback ships |
| Skill intent-discovery | **subsumed** by the intent kernel (doc 47): skills are selected through the admitted capability slice rather than a separate discovery path |
| Commitment kernel | shipped end to end (doc 47) |
| Web client — the workspace surface in a browser, a headless box you can *use*, and a complete public product surface | shipped, Phases 0–4 plus outcome projection and public `/outcomes` page (docs 48, 52) |
| Sandboxed workspace execution runtime (2026 unified `bash` execution engine with streaming events, ANSI line folding, real-time process telemetry, live stdout/stderr Workbench visibility, quarantined scratch containment under `.vak/scratch/`, and package tracking) | shipped in 3.0.22 |
| Distributed Event & Message Fabric (`crates/vak-bus`, NATS Core + JetStream, AES-256-GCM envelope security, W3C/Merkle causal lineage, Dead-Letter Queues) | shipped in 3.0.22 (doc 53) |
| Universal & adaptable platform: semantic entity memory distillation, interactive living outcome canvas, polyglot document ingestion (`doc_read`), specialist domain archetypes, collaborative delegation evals, and agent scheduling | shipped in 3.0.84 (doc 65) |
| Multi-user cloud (per-principal data homes, pooled Cores, quotas) | **explicitly out of scope** (doc 48 §8 E). A deployment is one operator's vak, and the token is a password to the whole box; the route shapes leave the seams open, nothing more |
| Windows platform support | S11 |

## Decisions locked during research

- Rust, SDK-first, server optional.
- Full permission system with prompts.
- Multi-provider from day one via raw provider APIs, no meta-SDK.
- Static flows plus a dynamic planner: planner → repair → validate,
  fail-closed `planning_failed`, bounded replan of one attempt.
- Parallel subagent fan-out gated by resource claims.
- Model-visible-means-logged; durable-vs-live event split; capability seams
  as traits; a minimal profile as the eval baseline; boot-tree
  introspection via `config dump`.
