# From Prompt to Final Output: End-to-End Execution Flow, Routing Logic, and Multi-Agent Orchestration

> **Status:** Production Architecture Specification (v3.0.29)  
> **Repository:** [vakcoder](file:///Users/nisheethranjan/Projects/vakcoder)  
> **Focus:** Exhaustive tracing of an inbound user prompt through ingress, routing, intent classification, demand scoring, frozen ladders, prompt layering, agent & subagent orchestration, brokered sandboxing, verifiable done-contracts, and multi-surface output projection.

---

## 1. Architectural Overview: The 8-Phase Execution Lifecycle

When a user submits a prompt to vak, the request does **not** simply get forwarded to an LLM. Instead, it travels through an 8-phase deterministic pipeline where security ceilings, routing decisions, intent-driven capability pruning, and real-world verifications govern every step:

```
[User Prompt] (CLI, Tauri Desktop, SolidJS Web, Slack, Discord, Telegram)
      │
      ▼
PHASE 1: INGRESS, IDENTITY & TENANT DISPATCH ROUTER
  ├─ 1.1 Inbound Normalization & 3-Segment Key: `surface:chat:bot_id`
  ├─ 1.2 Fail-Closed Allowlist Gate (`allowlist.json`)
  ├─ 1.3 Tenant Workspace Resolution & `CorePool` LRU Acquisition
  └─ 1.4 Command & Verb Router (System Verbs vs Flows vs Agent Turns)
      │
      ▼
PHASE 2: 7-AXIS INTENT KERNEL & COGNITIVE ADMISSION (vak-intent)
  ├─ 2.1 7-Axis Behavioral Classification (act, horizon, stakes, etc.)
  ├─ 2.2 The Narrowing Invariant: Reading ∩ Authority = Engagement Profile
  ├─ 2.3 Progressive Capability Slicing (Dynamic Tool/MCP Pruning)
  └─ 2.4 Durable Commitment Registration (commitments.jsonl)
      │
      ▼
PHASE 3: DEMAND ROUTER & FROZEN LADDER ADMISSION (vak-llm)
  ├─ 3.1 Demand Scoring Algorithm (`score_demand`: tokens, reasoning, tools)
  ├─ 3.2 Quality Objective Resolution (Utility vs Balanced vs QualityCritical)
  ├─ 3.3 Dynamic Model Discovery (/models) & Capability Filtering
  ├─ 3.4 Frozen Route Ladder Commitment ([Primary, Fallback 1, Fallback 2])
  └─ 3.5 FinOps Budget Admission Gate (Token & Cost Ceilings)
      │
      ▼
PHASE 4: 6-BLOCK LAYERED PROMPT ENGINE (docs/design/45-prompt-layers)
  ├─ 4.1 Block Composition (Identity, Operating Rules, Guardrails Floor)
  ├─ 4.2 Code-Owned Contracts (Capability Contract, Surface, Skills/MCP)
  └─ 4.3 7-Tier Inheritance: Seed -> Shared -> Project -> Surface -> Bot -> Chat -> Role
      │
      ▼
PHASE 5: AGENT LOOP & MULTI-AGENT ORCHESTRATION (vak-agent & vak-core)
  ├─ 5.1 Append-Only Ledger Derivation (`derive_messages()`)
  ├─ 5.2 Steering Queue Drainage (Injecting pending user steering)
  ├─ 5.3 Resilient LLM Inference (Watchdog Deadline + Informed Circuit Breaker)
  └─ 5.4 Execution Branching:
         ├─ Branch A: Direct Text Response
         ├─ Branch B: Brokered Tool Batch Execution
         ├─ Branch C: Subagent Spawning (`Surface::Subagent`, Child Core, Role Prompts)
         └─ Branch D: Dynamic Multi-Step Planner DAG (`vak-flow`)
      │
      ▼
PHASE 6: PERMISSION GATE & BROKERED SANDBOX EXECUTION (vak-permission & vak-tools)
  ├─ 6.1 Policy Evaluation (ReadOnly, Restricted, FullAccess)
  ├─ 6.2 Scoped Human-in-the-Loop Escalation (`Ask` Gate; Unattended Fails Closed)
  ├─ 6.3 Broker IPC Boundary (`__tool_worker` in Disposable Process Group)
  └─ 6.4 Containment (Landlock LSM, Docker Container, Sanitized ENV, .vak/scratch/)
      │
      ▼
PHASE 7: REAL-WORLD DONE-CONTRACT VERIFICATION (vak-commit)
  ├─ 7.1 Stop Policy Heuristic Gates (Marker Gate & Verify Gate)
  ├─ 7.2 The Satisfaction Lattice: None -> Claimed -> Cited -> Verified -> Audited
  └─ 7.3 Real-World Ground Truth Checks (Compiler Exit 0, Clean Diffs, Test Passes)
      │
      ▼
PHASE 8: IMMUTABLE AUDIT & MULTI-SURFACE PROJECTION DELIVERY
  ├─ 8.1 Append-Only Ledger Commit (`session.jsonl` + SQLite FTS5 Index)
  ├─ 8.2 Work Receipts Logging (`actions.jsonl` & `incidents.jsonl`)
  └─ 8.3 Output Projection:
         ├─ Interactive Desktop/Web Canvas (Diff Inspector, Test Matrix, Telemetry)
         └─ Remote Channel Outbox (CommonMark AST -> Durable Retry Queue for Slack/Discord)
```

---

## 2. Phase 1: Ingress, Identity & Tenant Dispatch Router

Before any prompt touches an LLM or activates an agent loop, it must be ingested, authenticated, mapped to an isolated tenant, and inspected for command routing.

### 1.1 Inbound Normalization & 3-Segment Identity
Inbound messages arrive from diverse protocols:
- Terminal standard input (`vak exec`)
- Native IPC from the Tauri Desktop app (`vak-desktop`)
- WebSocket and HTTP SSE requests from the SolidJS Web Client (`vak-server`)
- Webhook payloads from remote messaging bridges (Slack, Discord, Telegram)

To eliminate shared-bot credential poisoning and cross-channel bleed, the gateway converts every inbound payload into an `InboundRequest` stamped with a **3-segment identity key**:
$$\text{Key} = \text{surface} : \text{chat} : \text{bot\_id}$$

*Why this matters:* If two independent bots (e.g. an engineering bot and a release bot) operate in the exact same physical Slack channel, they receive **completely separate identity keys, separate session ledgers, and independent capability overlays**.

### 1.2 The Fail-Closed Allowlist Gate
The gateway evaluates the key against `<sessions_home>/gateway/allowlist.json`:
- **Allowed:** Request proceeds.
- **Denied:** Request is permanently dropped with zero execution.
- **Unknown:** If the sender has never been seen, vak **fails closed**—the chat is recorded as `pending` in the allowlist for operator review. An empty allowlist never permits ambient access unless explicitly configured.

### 1.3 Tenant Workspace Resolution & `CorePool`
vak is single-tenant by process design. When the gateway receives a request, it resolves the target canonical directory and queries `CorePool` (`vak-server`):
- `CorePool` maintains an **LRU cache of 4+1 warm `Arc<Core>` instances**.
- If the workspace is cached, `CorePool` immediately **re-checks the persisted security ceiling** in `.vak/config.toml`. If the workspace was demoted to `ReadOnly` on disk, the cached instance instantly revokes all active capabilities.
- If the workspace is not in the pool, an idle `Core` is flushed to disk and evicted, and the new workspace `Core` is initialized.
- Each `Core` owns its own session lock (`SessionManager`) preventing race conditions across concurrent requests.

### 1.4 Command & Verb Router
Before constructing agent loops, the prompt is checked by the Verb Router:
- **System Verbs / Slash Commands:** If the prompt starts with a direct directive (`vak doctor`, `vak config dump`, `vak memory`, `/cancel`, `/help`, `vak intent explain`), it completely bypasses model inference and executes deterministic system routines.
- **Static Declarative Flows:** If matched to a predefined static workflow (`vak-flow`), the system executes a declarative multi-step DAG.
- **Agent Turns:** All standard prompts continue to Phase 2.

---

## 3. Phase 2: 7-Axis Intent Kernel & Cognitive Admission (`vak-intent`)

Most agent systems fail because they treat every prompt identically—allocating the same tools and permissions to "what is the capital of France?" as they do to "delete all test databases."

In vak, [`crates/vak-intent`](file:///Users/nisheethranjan/Projects/vakcoder/crates/vak-intent) executes cognitive classification before prompt assembly:

```
                  ┌────────────────────────────────────────┐
                  │              USER PROMPT               │
                  └───────────────────┬────────────────────┘
                                      │
                                      ▼
                  ┌────────────────────────────────────────┐
                  │         7-AXIS INTENT READING          │
                  │  act  •  horizon  •  stakes  •  evid   │
                  │  clarity  •  modality  •  attendance   │
                  └───────────────────┬────────────────────┘
                                      │
            ┌─────────────────────────┴─────────────────────────┐
            │                                                   │
            ▼                                                   ▼
┌───────────────────────────────┐               ┌───────────────────────────────┐
│     THE NARROWING INVARIANT   │               │     DURABLE COMMITMENT        │
│  Reading ∩ Authority = Engmt  │               │   If horizon >= durable       │
│  (Can ONLY narrow, not widen) │               │   Registered in commitments   │
└───────────────┬───────────────┘               └───────────────────────────────┘
                │
                ▼
┌───────────────────────────────┐
│  PROGRESSIVE CAPABILITY SLICE │
│  Expose ONLY necessary tools  │
└───────────────────────────────┘
```

### 2.1 The 7 Orthogonal Behavioral Axes
1. **`act`** (`converse`, `answer`, `locate`, `analyze`, `author`, `modify`, `operate`, `verify`, `orchestrate`, `govern`): Controls output shape, stop profiles, and capability slicing.
2. **`horizon`** (`immediate`, `turn`, `session`, `durable`): Governs whether the request terminates in this turn or becomes a durable commitment.
3. **`stakes`** (`inert`, `reversible`, `costly`, `irreversible`): Sets human approval requirements and triggers automated Git checkpoints before execution.
4. **`evidence`** (`none`, `cited`, `verified`, `audited`): Defines the minimum proof required before the task can be completed.
5. **`clarity`** (`clear`, `underspecified`, `ambiguous`): If `ambiguous + irreversible` $\rightarrow$ interrupt and ask user; if `ambiguous + inert` $\rightarrow$ proceed on stated assumption.
6. **`modality`** (`text`, `image`, `audio`, `video`, `screen`, `data`, `stream`): Filters LLM routes for multimodal capabilities.
7. **`attendance`** (`interactive`, `supervised`, `unattended`): Governs approval timeout behavior (unattended always fails closed).

### 2.2 The Narrowing Invariant
$$\text{Engagement Profile} = \text{Reading} \cap \text{Authority}$$
An intent classification **can only narrow authority, never escalate it**. If a workspace is configured with `FullAccess`, but the user prompt merely asks an informational question (`act = answer`), write tools and Bash execution are stripped from the turn.

### 2.3 Progressive Capability Slicing
Instead of injecting all 50+ tool schemas, skills, and MCP servers into the model's context window, `vak-intent` emits a **Capability Slice**. An informational query receives zero tool definitions; a code search receives read-only grep/glob tools; only a destructive command receives bash workers. This prevents hallucinated tool calls and reduces token overhead.

### 2.4 Durable Commitment Registration
If `horizon >= durable`, the request outlives the interactive turn. A `DurableCommitment` is logged to `<workspace>/.vak/commitments.jsonl` with real-world satisfaction criteria.

---

## 4. Phase 3: Demand Router & Frozen Ladder Admission (`vak-llm`)

Once the engagement profile is derived, vak evaluates the **Demand Router** to select the optimal model route and freeze it.

### 3.1 Demand Scoring Algorithm (`score_demand`)
In [`crates/vak-llm/src/route.rs`](file:///Users/nisheethranjan/Projects/vakcoder/crates/vak-llm/src/route.rs), request difficulty is scored mathematically across weighted saturating anchors:

$$\text{Demand Score} = 0.30 \cdot C_{\text{ctx}} + 0.25 \cdot O_{\text{out}} + 0.15 \cdot R_{\text{reason}} + 0.12 \cdot T_{\text{tools}} + 0.10 \cdot S_{\text{struct}} + 0.08 \cdot E_{\text{evid}}$$

- $C_{\text{ctx}}$: Input token context saturation (saturates at 100k tokens; unknown reads as 0.5, never 0.0).
- $O_{\text{out}}$: Requested output token budget.
- $R_{\text{reason}}$: 1.0 if deep chain-of-thought reasoning is required, else 0.0.
- $T_{\text{tools}}$: Breadth of tools in the capability slice.
- $S_{\text{struct}}$: 1.0 if structured JSON output is required.
- $E_{\text{evid}}$: 1.0 if cited or audited evidence is required.

The score maps into three discrete **Demand Bands**:
- **`Low`** ($\text{Score} < 0.25$): Simple conversational answers or basic lookups.
- **`Moderate`** ($0.25 \le \text{Score} < 0.60$): Multi-file inspection, standard refactoring, basic tool calls.
- **`High`** ($\text{Score} \ge 0.60$): Complex architectural planning, deep reasoning, multi-file code modifications.

### 3.2 Quality Objective Resolution
The system resolves the quality objective based on the demand band and config:
- `Low` $\rightarrow$ **`Utility`** (Optimizes for latency and cost; e.g., Claude 3.5 Haiku, GPT-4o-mini).
- `Moderate` $\rightarrow$ **`Balanced`** (Balances reasoning depth with throughput; e.g., Claude 3.5 Sonnet, GPT-4o).
- `High` $\rightarrow$ **`QualityCritical`** (Selects top-tier reasoning engines; e.g., Claude 3.7 Sonnet Thinking, o1/o3-mini).

### 3.3 Dynamic Model Discovery & Filtering
vak queries its live 5-minute `/models` cache (populated directly from provider API keys). Models that cannot satisfy the turn's required modalities (e.g. vision or function calling) are filtered out.

### 3.4 The Frozen Route Ladder
The router orders the remaining candidate models into an immutable **Route Ladder**:
$$\text{Route Ladder} = [\text{Primary Model}, \text{Fallback Model 1}, \text{Fallback Model 2}]$$
**Contractual Guarantee:** The route ladder is committed at turn admission. The runtime may walk the ladder during transient failures, but it **never dynamically switches providers outside this frozen contract**.

### 3.5 FinOps Budget Admission Gate
Before dispatching any network calls, the workspace's remaining spend is verified against `[finops]` limits. If the estimated cost exceeds the session token cap or daily dollar ceiling, the turn is rejected immediately at admission.

---

## 5. Phase 4: The 6-Block Layered Prompt Engine

vak constructs the system prompt through a disciplined 6-block architecture ([`docs/design/45-prompt-layers.md`](file:///Users/nisheethranjan/Projects/vakcoder/docs/design/45-prompt-layers.md)) resolved across 7 inheritance tiers:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                       THE 6-BLOCK SYSTEM PROMPT                             │
├──────────────────────┬────────────────────────┬─────────────────────────────┤
│ Block Name           │ Owner                  │ Composition Rule            │
├──────────────────────┼────────────────────────┼─────────────────────────────┤
│ 1. identity          │ User / Role            │ Replace-or-Inherit          │
│ 2. operating_rules   │ User / Workspace       │ Replace-or-Inherit          │
│ 3. guardrails        │ Security Policy        │ Concatenate, NEVER Subtract │
│ 4. capability_contract│ Code-Owned (vak)      │ Immutable Factual Interface │
│ 5. surface & note    │ Code-Owned + User      │ Observation + Concatenate   │
│ 6. skills & mcp      │ Code-Owned (Inventory) │ Filtered Capability Slice   │
└──────────────────────┴────────────────────────┴─────────────────────────────┘
```

### The 7-Tier Resolution Hierarchy
Prompt blocks cascade through seven tiers, where narrower layers override identity but **can only tighten safety guardrails**:
$$\text{Seed} \longrightarrow \text{Shared User} \longrightarrow \text{Project Workspace} \longrightarrow \text{Surface} \longrightarrow \text{Bot} \longrightarrow \text{Chat} \longrightarrow \text{Agent Role}$$

- **Safety Floor Invariant:** `guardrails.md` concatenates across every tier. A project or remote chat prompt **cannot** disable guardrails defined in the user's base configuration.
- **Untrusted Clone Protection:** When cloning an untrusted Git repository, `Core::new_with_trust(dir, false)` strips project-level prompt files and demotes them, preventing malicious repositories from overriding safety rules or interface contracts.

---

## 6. Phase 5: Agent Loop & Multi-Agent Orchestration

In [`crates/vak-agent`](file:///Users/nisheethranjan/Projects/vakcoder/crates/vak-agent), the turn loop executes with streaming token deltas and parallel tool execution.

```
                    ┌──────────────────────────────────┐
                    │     DETERMINISTIC MESSAGES       │
                    │      derive_messages()           │
                    └─────────────────┬────────────────┘
                                      │
                                      ▼
                    ┌──────────────────────────────────┐
                    │       STREAMING INFERENCE        │
                    │   (Watchdog + Circuit Breaker)   │
                    └─────────────────┬────────────────┘
                                      │
         ┌────────────────────────────┼────────────────────────────┐
         │                            │                            │
         ▼                            ▼                            ▼
┌──────────────────┐        ┌──────────────────┐        ┌──────────────────┐
│    BRANCH A:     │        │    BRANCH B:     │        │    BRANCH C:     │
│   Direct Text    │        │  Tool Execution  │        │ Subagent Spawning│
│   (Stop Policy)  │        │  (Broker Sandbox)│        │ (Surface::Subag) │
└──────────────────┘        └──────────────────┘        └──────────────────┘
```

### 5.1 Append-Only History Derivation (`derive_messages()`)
The agent loop never stores ephemeral prompt strings. Prior to calling the provider, it invokes `session.derive_messages()`, deterministically reconstructing the exact message sequence from the append-only JSONL ledger.

### 5.2 Steering Queue Drain
If the user typed a steering correction while a background turn was in progress, the steering queue is drained and prepended as a prioritized user message before the model call.

### 5.3 Resilient Streaming Inference & Circuit Breaker
- **Watchdog Deadline:** Every token chunk is expected within a strict timeout deadline.
- **Informed Circuit Breaker:**
  - If the provider returns HTTP 429 with `Retry-After`, this is categorized as *informed transience*. It feeds endurance backoff waits without tripping the breaker.
  - If the provider encounters a connection reset, TLS failure, or truncated stream, it trips the circuit breaker, immediately falling back to the next model on the **Frozen Route Ladder**.

### 5.4 The Execution Branches:

#### Branch A: Direct Text Response
If the model generates no tool invocations, execution terminates conversational generation and transitions to the Stop-Gate verification phase.

#### Branch B: Tool Execution Batch
If the model emits one or more tool calls (`ToolUse` blocks), the calls are parsed and dispatched in parallel across the permission broker.

#### Branch C: Subagent Spawning (`subagent` tool)
When a task benefits from isolated sub-delegation (e.g., performing a deep dependency audit or searching logs without polluting parent context):
1. **Budget Check:** Verifies that subagents are enabled (`effective_subagents()`) and that the turn has remaining `subagent_budget`.
2. **Child Core Cloning:** Clones the parent `Core` with a new surface: `Surface::Subagent`.
   - *Why this is critical:* A subagent's consumer is the parent agent, not a human. Setting `Surface::Subagent` modifies prompt layer notes and prevents the subagent from outputting user-facing conversational fluff.
3. **Role-Specific Prompts:** Loads specialized instructions from `.vak/prompts/agents/<role>/`.
4. **Registry Registration:** Registers the running child in `Arc<SubagentRegistry>`, allowing human operators to inspect, attach to, or cancel subagents from the UI.
5. **Sandboxed Child Loop:** The subagent runs its own isolated turn loop, producing an append-only child ledger. Its final output is returned to the parent agent as a typed `ToolResult`.

#### Branch D: Dynamic Multi-Step Planner DAG (`vak-flow`)
If the prompt was admitted as a multi-step plan, `vak-flow` constructs a Directed Acyclic Graph (DAG) where nodes execute sequentially or concurrently based on dependency resolution.

---

## 7. Phase 6: Permission Gate & Brokered Sandbox Execution

Every effect proposed by an agent or subagent must pass through the **Broker Boundary** before touching host resources:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            PERMISSION ENGINE                                │
│        Evaluates Mode: ReadOnly  •  Restricted  •  FullAccess               │
│        Verifies Workspace Containment (Rejects Path Traversal)              │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
            ┌──────────────────────────┴──────────────────────────┐
            │ (Allowed)                                           │ (Requires Escalation)
            ▼                                                     ▼
┌──────────────────────────────────────┐        ┌─────────────────────────────┐
│       THE BROKER BOUNDARY (IPC)      │        │       HUMAN "ASK" GATE      │
│  Spawns disposable __tool_worker     │        │ Prompts human for approval  │
├──────────────────────────────────────┤        │ (Unattended FAILS CLOSED)   │
│ • Linux Landlock LSM Sandboxing      │        └─────────────────────────────┘
│ • Docker Container Isolation Backend │
│ • Sanitized ENV (No Ambient Secrets) │
│ • Quarantined Writes to .vak/scratch/│
└──────────────────────────────────────┘
```

1. **Permission Engine:** Checks tool action against current workspace rules. If outside allowable scopes, issues a typed `Ask` prompt to the user.
2. **Unattended Fails Closed:** If running unattended (via Slack/Discord or scheduled cron), an approval prompt that times out **fails closed**—never escalating silently.
3. **The `__tool_worker` Protocol:** Tools do not run in the main vak server daemon. The engine spawns a lightweight disposable worker process group (`__tool_worker`).
4. **Secrets Quarantine:** Subprocesses receive a strict, minimal environment allowlist. Provider API keys, gateway tokens, and system credentials are completely withheld.
5. **Filesystem Containment:** Linux Landlock LSM syscalls restrict filesystem write access strictly to the workspace root or the quarantined `.vak/scratch/` directory.

---

## 8. Phase 7: Real-World Done-Contract Verification (`vak-commit`)

In vak, **the model is never allowed to unilaterally declare a task complete**.

### 7.1 Stop Policy Heuristics
Before returning `Completed`, `vak-agent` evaluates its internal stop gates:
- **Marker Gate:** Checks for unclosed fenced code blocks or trailing colons that indicate the model was truncated mid-sentence.
- **Verify Gate:** If the user prompt demanded verification ("ensure tests pass", "prove that it builds"), but zero bash commands were executed during the run, the gate blocks completion and injects: `[stop-guard]: Verification was demanded but no verification commands were run. Please verify.`

### 7.2 The Satisfaction Lattice & Verifiable Contracts
For managed work contracts, completion requires ground-truth evidence:
$$\text{None} \longrightarrow \text{Claimed} \longrightarrow \text{Cited} \longrightarrow \text{Verified} \longrightarrow \text{Audited}$$

The runtime verifies real-world predicates:
- Compiler/Linter exit code equals `0`.
- Automated test suites execute cleanly.
- Target Git diffs match expected modified files.
- Non-empty verification receipts recorded in `actions.jsonl`.

Only when ground-truth evidence satisfies the commitment's required `evidence` axis does the contract transition to `Verified`.

---

## 9. Phase 8: Immutable Audit & Multi-Surface Projection Delivery

Once the turn outcome is verified, vak performs atomic persistence and delivers outputs:

1. **Append-Only Ledger Commit (`vak-session`):**
   - The user prompt, intent classification, model deltas, tool calls, and verified outcomes are appended to `<workspace>/.vak/sessions/<session_id>.jsonl`.
   - History is never rewritten or deleted; branching appends a new entry with a `parent_id`.
   - SQLite FTS5 search index (`vak-store`) is updated in the background.

2. **Operations Center Receipts (`vak-ops`):**
   - Incident fingerprints are reconciled in `operations/incidents.jsonl`.
   - Work receipts with before/after state verification are appended to `operations/actions.jsonl`.

3. **Output Projection Engine (`vak-delivery`):**
   - Internal event streams are parsed into a standards-compliant CommonMark AST (`pulldown-cmark`).
   - **Desktop / Web Client Canvas:** SolidJS client renders rich, outcome-first widgets (interactive diff inspector, test matrix, SVG crosshair telemetry charts, and live PTY terminal).
   - **Remote Chat Bridges:** The delivery engine maps the AST into channel-specific semantic adapter envelopes (Slack Block Kit, Discord embeds, Telegram HTML) and enqueues them into a **Durable Retry Outbox** to guarantee delivery across unstable networks.

---

## 10. Summary Matrix: The Complete Execution Path

| Pipeline Phase | Responsible Crates | Key Inputs & Decision Gates | Output & Guarantees |
|---|---|---|---|
| **1. Ingress & Tenant Router** | `vak-server` | Inbound payload, `surface:chat:bot_id`, `allowlist.json` | Fail-closed security; warm `Arc<Core>` acquired from `CorePool`. |
| **2. Intent Kernel** | `vak-intent`, `vak-commit` | Prompt text, workspace permission ceiling | 7-axis classification; **Narrowing Invariant** enforced; capability slice emitted. |
| **3. Demand Router** | `vak-llm` | Token budget, reasoning requirement, modality | DemandBand resolved; **Frozen Route Ladder** committed; FinOps budget admitted. |
| **4. Prompt Layering** | `vak-core`, `vak-agent` | 6 prompt blocks across 7 inheritance tiers | Safety floor concatenated; code-owned interface contracts preserved. |
| **5. Agent Orchestration** | `vak-agent`, `vak-core` | `derive_messages()`, steering queue | Streaming inference; parallel tool execution; isolated subagents spawned. |
| **6. Broker & Sandbox** | `vak-permission`, `vak-sandbox`, `vak-tools` | Security mode (`ReadOnly`/`Restricted`), Landlock LSM | `__tool_worker` isolation in disposable process group; zero ambient secrets. |
| **7. Done Verification** | `vak-commit`, `vak-agent` | Real-world compiler exit codes, test suites, git diffs | Satisfaction Lattice advances to `Verified`; model self-declaration rejected. |
| **8. Ledger & Delivery** | `vak-session`, `vak-delivery`, `vak-store` | Verified turn outcome, CommonMark AST | Append-only JSONL commit; SQLite FTS5 sync; durable outbox channel delivery. |
