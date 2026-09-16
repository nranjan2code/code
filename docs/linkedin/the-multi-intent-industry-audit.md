# The Multi-Intent Problem in AI Agents: An Architectural Taxonomy & Honest Audit

*A critical comparison of how modern agent architectures—from LangGraph and Claude Code to Vak—handle compound requests, context degradation, and completion verification.*

---

## Executive Summary: The Realities of Compound Intent

Over the past two years, the AI agent ecosystem transitioned from simple single-turn chatbot loops to systems executing multi-step workflows. Yet every practitioner building autonomous agents encounters the same underlying wall: **compound requests**.

When a user submits a realistic, multi-clause instruction:
> *"Search the repo for the authentication token parsing bug, refactor the parser to support the new claims schema, and verify that the integration test suite passes."*

Production systems routinely break down in one of three ways:
1. **Tool Starvation:** A router classifies the prompt as a single action (e.g. `Modify`), stripping observability tools (`grep`, `read`) to reduce token bloat, blinding the agent.
2. **The "Done" Illusion:** An agent edits code, hallucinates that tests passed, and exits without executing a single terminal verification command.
3. **Context Amnesia:** After 15 turns of debugging compiler errors, the agent completely forgets the user's secondary and tertiary requirements.

This report evaluates four major architectural paradigms for addressing this problem:
* **The Static Graph Paradigm** (*LangGraph, LlamaIndex Workflows*)
* **The Conversational Swarm Paradigm** (*CrewAI, AutoGen*)
* **The Native CLI Harness Paradigm** (*Claude Code, Cursor/Devin*)
* **The Algebraic Intent Kernel Paradigm** (*Vak*)

---

## 1. Architectural Taxonomy: Four Ways to Handle Compound Requests

```
┌───────────────────────────────────────────────────────────────────────────────────┐
│                           COMPOUND USER REQUEST                                   │
│     "Find bug in auth.rs, refactor parser, verify tests, and document changes"    │
└─────────────────────────────────────┬─────────────────────────────────────────────┘
                                      │
         ┌────────────────────────────┼────────────────────────────┐
         ▼                            ▼                            ▼
┌──────────────────┐         ┌──────────────────┐         ┌──────────────────┐
│   STATIC DAGS    │         │ MULTI-AGENT SWARM│         │ ALGEBRAIC KERNEL │
│   (LangGraph)    │         │ (CrewAI/AutoGen) │         │      (Vak)       │
├──────────────────┤         ├──────────────────┤         ├──────────────────┤
│ Pre-compiled     │         │ Manager agent    │         │ 7-axis scoring;  │
│ workflow nodes   │         │ delegates to N   │         │ contender band   │
│ & conditional    │         │ chatty workers   │         │ unions tools in  │
│ edges.           │         │ in natural prose.│         │ single loop.     │
├──────────────────┤         ├──────────────────┤         ├──────────────────┤
│ Rigid if prompt  │         │ High token churn;│         │ Fast zero-token; │
│ violates graph   │         │ latency; context │         │ but sequential   │
│ topology.        │         │ exhaustion.      │         │ execution only.  │
└──────────────────┘         └──────────────────┘         └──────────────────┘
```

### Paradigm 1: Pre-Compiled Workflow Graphs (LangGraph, LlamaIndex Workflows)
* **Mechanism:** Developers manually construct directed acyclic graphs (DAGs) in Python/TypeScript. A triage node classifies the request and routes along conditional edges to specialized nodes (`coder_node`, `test_node`).
* **Strengths:** Predictable, easy to debug, production-proven checkpointer persistence (Postgres/Redis), excellent for structured business processes.
* **Failure Modes:** Extremely fragile when exposed to open-ended human prompts. If a user's prompt combines three steps that don't match a pre-wired edge, the system either forces the entire request into a single generic "fallback" node or deadlocks.

### Paradigm 2: Conversational Multi-Agent Swarms (CrewAI, AutoGen)
* **Mechanism:** A supervisor or "manager" agent uses an LLM call to decompose the prompt into discrete tasks, passing them to specialized sub-agents (`Researcher`, `Coder`, `QA`).
* **Strengths:** Intuitive role-based abstraction; highly flexible for brainstorming and content workflows.
* **Failure Modes:** Severe token bloat and high latency. Sub-agents communicate via natural language chat, burning context on handoff negotiations. Security boundaries suffer from "compound contamination"—if one agent in the chain is tricked, ambient tools across the swarm can be compromised.

### Paradigm 3: Implicit Attention in Unified Harnesses (Claude Code, Devin)
* **Mechanism:** A single agent loop with full tool access (read, write, bash). Relies on frontier model self-attention, instruction-following prompts (e.g. `CLAUDE.md`), and system-level hooks to track multi-part goals.
* **Strengths:** Zero classification overhead; natural fluid execution; tightly coupled to terminal and file diffs.
* **Failure Modes:** Relies entirely on the LLM's willingness to keep track of its own plan. When context fills with large tool outputs (compiler logs, stack traces), attention drifts toward the immediate error, causing the agent to quietly drop remaining goals.

### Paradigm 4: Algebraic Intent Kernels (Vak)
* **Mechanism:** A typed systems harness that resolves intent across 7 orthogonal axes using microsecond lexical heuristics, admits sibling contenders within a 50% score band, unions capabilities, and enforces stop-guards.
* **Strengths:** Zero token latency for extraction; formal mathematical proofs against privilege widening (bounded semilattice); hard refusal of model self-completion.
* **Failure Modes:** Sequential turn bottleneck (cannot execute independent sub-tasks in parallel); English-centric lexical heuristics; arbitrary heuristic thresholds (e.g. 0.5 band).

---

## 2. Comparative Matrix: Trade-offs Across Engineering Dimensions

| Engineering Dimension | **LangGraph** | **Claude Code** | **CrewAI / AutoGen** | **Vak (Current State)** |
| :--- | :--- | :--- | :--- | :--- |
| **Multi-Intent Handling** | Pre-wired conditional edges. Rigid if input breaks graph topology. | Implicit model attention + markdown plan files. Prone to drift under bloat. | Manager agent natural language delegation. High latency, heavy token churn. | **Contender Band (50%).** Unions tools in single turn loop. Fast, but sequential only. |
| **Capability Security** | Node-level function isolation. No formal non-widening lattice. | Process groups + interactive approval prompt. Binary auto-mode. | Ambient toolsets per agent. Vulnerable to chained injection. | **Bounded Meet-Semilattice.** Mathematically proves $\text{meet}(a, b) \sqsubseteq a$. Never widens perms. |
| **Context Compaction** | Checkpointer state saving + custom rolling summary prompts. | Automatic `/compact` at ~95% token threshold. Somewhat lossy. | Conversational history duplication. Rapidly exhausts context. | **Boundary Snapping.** Never splits `tool_use`/`result`. Mandatory task preservation contract. |
| **Completion Audit** | Developer-written conditional assertion nodes. | User-extensible bash stop hooks. Model can self-certify. | Subjective manager agent prose review. Prone to loops. | **StopGuard Triad.** Audits `VerificationStale`, `TruncatedPlan`. Rejects prose completion. |
| **Production Maturity** | **High.** Battle-tested across millions of enterprise runs. | **High.** Polished terminal UX backed by frontier foundation models. | **Moderate.** Broad community adoption, primarily prototyping. | **Experimental.** 0 external production users; validated on synthetic test suites only. |

---

## 3. A Critical, Honest Audit of Vak's Architecture

To build a genuinely reliable system, we must be brutally transparent about Vak's current flaws, blind spots, and architectural trade-offs:

### Flaw 1: The Contender Band (0.5) is an Arbitrary Heuristic
In `vak-intent`, an act joins the active capability set if:
$$\text{weight} \ge 0.5 \times \text{winner\_weight}$$
* **The Reality:** The constant `0.5` is an uncalibrated engineering guess. It was not derived from empirical optimization across 100,000 real-world developer conversations.
* **The Risk:** 
  - On dense multi-task prompts, subtle secondary intents with weights at 0.48 will be discarded (false negative), causing tool starvation.
  - On ambiguous prompts, weak noise words can cross the 0.5 threshold, loading unnecessary tools and increasing prompt token overhead (false positive).

### Flaw 2: The Sequential Turn Bottleneck
* **The Reality:** Vak executes compound intents sequentially within a single turn loop (e.g. search $\rightarrow$ edit $\rightarrow$ test).
* **The Failure Mode:** When a user asks for two completely independent operations:
  > *"Check the staging deployment on Kubernetes, and fetch the open bugs from GitHub issues."*
  LangGraph or a multi-agent swarm can fire both requests concurrently in 800ms. Vak executes them in sequence across multiple turns, incurring a 2x–3x latency penalty.

### Flaw 3: English-Centric Lexical Brittleness (Tier 1)
* **The Reality:** Vak's zero-token Tier-1 extractor relies on regexes, English imperative verb lists, and English inflectional stemming (`-ing`, `-ed`, `-s`).
* **The Failure Mode:**
  - Non-English prompts (e.g. German compound verbs, Japanese non-concatenative morphology) fail Tier-1 extraction completely and fall back to ambient floors or require cloud model classification.
  - Colloquialisms, typos, or passive phrasing (*"It would be great if the tests were looked at"*) defeat the regex heuristics.

### Flaw 4: Stop-Guard False-Positive Traps
* **The Reality:** The `VerificationMissing` stop-guard checks if files were changed and requires a verification tool call if verification intent was detected.
* **The Failure Mode:** For documentation updates or trivial text changes (*"Fix typo in README.md and verify formatting"*), the agent has no automated test suite to run. A rigid stop-guard can trap the agent in an exit-blocked loop where it wastes tokens apologizing or trying to run irrelevant bash commands.

### Flaw 5: Synthetic Test Isolation vs. Wild Human Inputs
* **The Reality:** Vak's test suite boasts 154 integration tests and over 100,000 combinatorial assertions.
* **The Limitation:** All of these tests were written by the same team that wrote the harness. They test the system against our own assumptions. They do not prove that Vak will handle the ambiguity, chaotic typos, and contradictory instructions of real human operators in the wild.

---

## 4. What Vak Genuinely Gets Right

Despite these limitations, Vak introduces three architectural ideas that address real gaps in today's agent ecosystem:

1. **The Bounded Meet-Semilattice Guarantee (`vak-intent`):**
   Modeling capabilities as $\bot = \text{Empty} \le \text{Only}(names) \le \top = \text{All}$ with monotonic meet operations guarantees that resolving complex compound intents can *never* accidentally escalate privileges or widen sandbox boundaries.
2. **Rejection of Model Self-Certification (`work.rs`):**
   Enforcing `#[error("work item '{0}' cannot be marked succeeded by a model event")] ModelCompletion` at the session layer ensures that an agent cannot talk its way into completing a contract. Completion requires concrete runtime execution receipts.
3. **Verification Staleness Auditing (`stop_policy.rs`):**
   Detecting when an agent modified source files *after* running tests prevents one of the most common failure modes in autonomous coding: the unverified post-test tweak.

---

## 5. Next Steps: The Roadmap to Fix Vak's System

To move from an interesting architectural experiment to a production-grade system, our immediate engineering focus must be:

1. **Calibrate the Contender Threshold:** Replace the hardcoded `0.5` constant with dynamic calibration or multi-label ranking evaluated against public agent benchmarks.
2. **Parallel Sub-Task Dispatch:** Implement parallel branching within `CorePool` / `TaskTool` for independent sibling intents that do not share disk or state dependencies.
3. **Context-Aware Stop Guards:** Refine `VerificationMissing` to distinguish between executable code changes (requiring tests) and non-executable documentation edits (requiring visual/formatting inspection).
4. **Empirical Benchmark Qualification:** Run Vak against standard public benchmarks (SWE-bench Lite / Verified, ToolBench) to evaluate real-world compound task completion against industry peers.

---

*Vak is an open-source research agent harness developed in Rust. Source code, formal proofs, and architecture documents are available at [github.com/nranjan2code/code](https://github.com/nranjan2code/code).*
