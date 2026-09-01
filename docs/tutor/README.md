# VAK Architecture Tutor

This is the visual guide to VAK: start with the system map, follow one request through the runtime, then use the deep dives to understand trust, execution, state, delivery, and operations.

## Recommended path

1. [System overview](#1-system-overview)
2. [One request end to end](#2-one-request-end-to-end)
3. [Normal vs managed flow](#3-normal-vs-managed-flow)
4. [Inside the runtime](#4-inside-the-runtime)
5. [Trust, configuration, and execution](#5-trust-configuration-and-execution)
6. [State, delivery, and operations](#6-state-delivery-and-operations)
7. [Personal OS and user surfaces](#7-personal-os-and-user-surfaces)

## 1. System overview

### [Layered architecture](vak-architecture-overview.png)

The complete crate map, organized from user surfaces and gateway operations down to orchestration, policy, intelligence, delivery, persistence, and infrastructure.

## 2. One request end to end

### [Request lifecycle](vak-request-end-to-end.png)

Shows the common path for desktop, CLI, web, Telegram, Discord, and Slack: normalize identity, resolve trust and routing, freeze the session, authorize, execute, and deliver.

### [Gateway, channels, and trust](vak-gateway-channels-trust.png)

Explains channel bridges, bot-scoped identities, allowlist states, the bot → chat → workspace policy chain, route resolution, and safe outbound adapter selection.

## 3. Normal vs managed flow

### [Normal vs Managed Flow](vak-normal-vs-managed-flow.png)

Contrasts the fast direct agent loop with Managed Flow’s durable WorkContract, work items, ownership, evidence, checkpoints, and verified completion.

### [Flows, planner, and evaluation](vak-flows-planner-evaluation.png)

Explains static DAGs, dynamic candidate plans, validation, deterministic repair, bounded replanning, governed node execution, and evaluation boundaries.

## 4. Inside the runtime

### [The `vak-core` runtime facade](vak-core-runtime-facade.png)

Shows how a workspace-scoped `Core` composes configuration, sessions, providers, agent execution, policy, tools, persistence, delivery, and operational features for every entry point.

### [Inside one agent turn](vak-inside-one-agent-turn.png)

Follows admission, context projection, provider streaming, assistant messages, the stop gate, tool waves, subagents, managed work, goal mode, receipts, checkpoints, and final outcomes.

## 5. Trust, configuration, and execution

### [Permission and tool security](vak-permission-tool-security.png)

Shows the enforcement boundary: PermissionEngine → approval → brokered registry → validated operation → worker → Seatbelt/Landlock/Docker → bounded result.

### [Configuration, routing, and runtime contract](vak-config-routing-contract.png)

Explains global/project/scoped configuration precedence, atomic persistence, route provenance, provider/model freezing, secrets, permission revocation, sandbox selection, and FinOps limits.

## 6. State, delivery, and operations

### [The append-only session ledger](vak-session-ledger.png)

Explains JSONL entries, parent-pointer trees, model-visible projections, audit-only receipts, compaction, branches, checkpoints, recovery, and the rebuildable search index.

### [Semantic delivery pipeline](vak-semantic-delivery.png)

Shows how internal outcomes become `OutputTimeline` and `PresentationDocument` values, pass through capability projection and renderers, and reach UI/channel adapters or the retry outbox.

### [Operations Center](vak-operations-center.png)

Explains how live handles, service-manager probes, gateway state, Doctor, security events, tasks, and delivery state become an evidence-backed operational projection.

### [`vak doctor`](vak-doctor.png)

Shows context collection, health checks, facts, report ordering, known mechanical repairs, re-collection, and the fail-closed “never guess” boundary.

## 7. Personal OS and user surfaces

### [Memory, learning, and automation](vak-memory-learning-automation.png)

Connects session search, memory tiers, reflection, skill proposals, scheduled tasks, heartbeats, budget admission, inbox, outbox, and governed memory writes.

### [Desktop, admin, and extensions](vak-desktop-admin-extensions.png)

Maps the Tauri desktop, SolidJS admin console, tray, CLI/TUI, secured router, SSE projections, operations views, plugins, skills, MCP, hooks, and channel adapters.

## Cross-cutting rules

- Every execution path authorizes before dispatch.
- The session ledger is append-only and remains the source of truth.
- Provider and model are one frozen route per session.
- Channel capability overlays can restrict access but never grant secrets.
- Managed work adds durable planning and verification; it does not replace direct chat.
- The service manager owns process liveness; health probes do not compete with `KeepAlive` or `Restart`.
- The Operations Center reports evidence, not synthetic metrics.

## Related source documentation

- [Architecture diagrams](../architecture/README.md)
- [Agent loop](../design/03-agent-loop.md)
- [Permissions](../design/08-permissions.md)
- [Flows](../design/10-flows.md)
- [Planner](../design/11-planner.md)
- [Gateway](../design/22-gateway.md)
- [Memory](../design/23-memory.md)
- [Agent security](../design/24-agent-security.md)
- [Operations](../design/28-operations.md)
- [Output engineering](../design/30-output-engineering.md)
- [Channel onboarding](../design/34-channel-onboarding.md)
- [Managed work contracts](../design/42-managed-work-contracts.md)

[Back to docs](../)
