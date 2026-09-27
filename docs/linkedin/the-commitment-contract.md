# The Commitment Contract: How to Stop AI Agents from Wandering Off

*Why long-horizon autonomy breaks without an external state machine, and how the satisfaction lattice replaces blind trust.*

![The Commitment Contract Architecture](commitment_banner.jpg)

If you manage engineers, analysts, or researchers, you already know the golden rule of delegation: **never assign a high-stakes outcome without agreeing on acceptance criteria upfront.**

When you delegate a critical migration or a quarterly market synthesis to a staff member, you do not just say a few sentences and walk away. You define what done looks like. You agree on what evidence will be produced, which systems can be touched, what budget is allocated, and what checkpoints require approval.

In human organizations, this is called a contract of delegation. It protects the company from runaway initiatives, and it protects the worker from ambiguous expectations.

Yet in the autonomous agent world, almost every framework still operates on a chaotic gamble:
1. You hand the agent an ambitious prompt.
2. The agent begins executing tool calls in a conversational loop.
3. As the turn count climbs into the teens and twenties, the model gets distracted by intermediate error logs and begins working on tangential side-quests.
4. After burning forty dollars in API credits, it announces victory with zero verifiable deliverables to show for it.

Over the past year of building and dogfooding vak, a general-purpose agent harness I run daily across research, data analysis, and software development, this was the most frustrating failure mode I witnessed. 

A model would begin with great intentions. But by turn fifteen, it was refactoring an unrelated utility script or searching for third-party documentation it did not need, completely oblivious to the original goal.

When builders hit this wall, the standard reaction is to blame the language model. But switching to a more capable frontier model does not fix this on its own. Smarter models do not stop wandering off. They simply invent more articulate, sophisticated rationalizations for why their side-quest was necessary.

Long-horizon reliability does not come from prompt engineering. It comes from an external architectural anchor: **The Commitment Contract.**

---

## Why Long-Horizon Prompts Suffer from Context Amnesia

To understand why autonomous agents lose focus, you have to look at the physics of large language model context windows.

In a simple single-turn prompt, the original goal occupies 80% of the active attention focus. The model understands the target clearly.

In a long-horizon task spanning twenty or thirty tool calls, the operational environment changes completely:
- Terminal stderr traces and compiler warnings dump thousands of tokens into the prompt.
- File inspection outputs and JSON payloads push the original objective further and further back into history.
- Intermediate reasoning steps introduce tentative hypotheses that subsequent turns mistake for primary requirements.

Researchers refer to this degradation as **attention dilution and context drift**. Under heavy context pressure, the original prompt stops acting as a steering wheel. It becomes background noise.

The moment the agent encounters an unexpected error (a missing dependency, an undocumented API response, or a flaky network call), it branches. Instead of handling the error within the boundary of the original task, it adopts the error as its new identity. 

I have watched unchecked models spend twenty consecutive turns writing custom test harnesses for third-party libraries they were only supposed to inspect. The agent did not quit. It simply forgot what it was hired to do.

If you rely on the model's conversational memory to remember the goal, you will lose every time. **The goal state must live outside the model.**

---

## The Fatal Assumption: Chat Sessions are the Wrong Unit of Work

Almost every agent platform today makes the same foundational mistake: they treat the **chat session** as the primary unit of work.

A chat session is an ephemeral sequence of messages. It has a start, a scrolling transcript, and a termination point when generation stops. 

Treating a session as the unit of work introduces three catastrophic flaws:

### 1. Obligations Outlive Transcripts
Real work does not live inside a conversation window. A database backfill might take six hours. A competitive market tracker might run once every night. A compliance audit might require three days of human review before the next step executes. 

If your execution state is tied to an active chat session, a network disconnection, a process restart, or context compaction destroys the obligation.

### 2. The Model Can Gaslight Its Own History
When the session is the only record of truth, the model has ambient read and write access to its own narrative. If an agent struggles to achieve a difficult requirement, it can subtly reframe the requirement in subsequent conversational turns, gradually convincing itself (and an unobservant user) that the modified outcome was what was originally requested.

### 3. "Stopped Talking" is an Unfalsifiable Signal
In a session-based architecture, the harness assumes the task is complete because the model stopped emitting tokens. As explored in my first article, silence is not proof. A model that runs out of tokens or hits an internal confidence flatline has not finished the work; it has merely stopped typing.

To build reliable autonomous systems, you must decouple the **Unit of Conversation** from the **Unit of Obligation**.

In my runtime harness, chat sessions are disposable views. The durable unit of work is a **Commitment**.

---

## The Anatomy of a Commitment Contract

A Commitment is an immutable, append-only record stored in an external state machine completely separate from the LLM's prompt window.

Before the agent executes a single effectful tool, the runtime analyzes the user's objective and registers an explicit contract:

![The 6 Pillars of an Autonomous Commitment Contract](commitment_contract_spec.jpg)

This contract establishes six non-negotiable boundaries upfront:

### 1. Typed Deliverable Specification
The contract defines the expected outcome shape. Is this an operational change that alters workspace state? Is it an analytical synthesis requiring cited evidence? Or is it an informational answer? By typing the deliverable, the runtime establishes what category of proof will be required.

### 2. Pre-Flight Resource Ceilings
Human managers do not give contractors an open company credit card without spending limits. A commitment contract enforces hard operational ceilings:
- Maximum dollar spend on model tokens.
- Maximum allowable tool execution turns.
- Maximum wall-clock execution time.
If an agent hits any of these boundaries without fulfilling its goal, it cannot loop indefinitely. The runtime halts execution and files a structured breach report.

### 3. The Capability Envelope
Why expose fifty dangerous system tools to an agent that only needs to read logs? Based on the task's Act axis, the commitment contract locks the capability envelope. If the contract defines an inspection act, write and execution tools are withheld from the context window entirely.

### 4. The Standard of Proof
The contract records the required tier of evidence upfront. A conversational claim like *"I verified the data"* is explicitly rejected if the contract demanded machine-checked receipts.

### 5. The Immutable Audit Trail
The model cannot edit, truncate, or rewrite the commitment. The commitment exists in an independent ledger on disk. Even if the conversation transcript is compacted, reset, or transferred to another process, the commitment remains frozen.

### 6. Explicit Breach & Escalation Policies
If an external API goes down or an approval times out, what should happen? The contract specifies the failure mode upfront: fail closed immediately, suspend into an inbox, or request human escalation.

---

## The Commitment Lifecycle State Machine

Once registered, a commitment moves through a formal state machine governed by deterministic runtime code:

![The Commitment Lifecycle State Machine](commitment_lifecycle.jpg)

The lifecycle operates across five distinct operational phases:

### Phase 1: Proposed (Pre-Flight Admission)
The contract is drafted. The runtime evaluates whether the requested operation is permitted under current security policies, whether sufficient budget exists, and whether the required tool integrations are online. If pre-flight checks fail, the task is rejected before a single dollar of frontier compute is burned.

### Phase 2: Active (Governed Execution)
The agent executes within its capability envelope. Crucially, the model does not update the commitment ledger directly. System telemetry records tool receipts, exit codes, and inspected offsets into an external Evidence Ledger on every step.

### Phase 3: Suspended (Zero-Token Parking)
One of the biggest money pits in modern agent frameworks is polling. If an agent needs human approval or is waiting for an overnight batch job to complete, naive implementations keep the LLM running in a loop, repeatedly checking status and burning context tokens.
In my harness, waiting work enters `Suspended`. The runtime freezes the turn, unloads the model from memory, and burns zero tokens. The commitment sits quietly in the ledger until an external webhook, a timer, or a user reply wakes it back up.

### Phase 4: Blocked (Anti-Loop Circuit Breaker)
What happens when an agent hits an authentic wall, such as a missing secret key or a broken external endpoint?
Instead of allowing the model to hallucinate workarounds or enter an infinite retry doom-loop, the state transitions to `Blocked`. The runtime enforces a hard intervention ceiling: maximum two recovery attempts. If progress cannot be made, execution halts cleanly.

### Phase 5: Satisfying (The Audit Gate)
When the model finally signals that it has completed the work, it does not close the session. It merely moves the commitment into `Satisfying`. 
At this boundary, the runtime audits the physical Evidence Ledger against the Commitment Contract using an objective hierarchy of proof.

---

## The Satisfaction Lattice: The 4 Tiers of Proof

How does a runtime know whether a task is actually finished?

In most agent setups, the harness relies entirely on model confidence. If the LLM generates text that sounds confident, the system assumes success.

This is a category error. **Confidence is an internal state of a language model. Proof is a verifiable fact about the external world.**

To evaluate completion objectively, my harness organizes evidence into a four-tier **Satisfaction Lattice**:

![The Satisfaction Lattice Proof Hierarchy](satisfaction_lattice.jpg)

The lattice establishes a strict inequality:

```
Asserted  <  Cited  <  Observed  <  Attested
```

Strength is determined by **how an outcome was established**, not by how eloquently the model describes it:

### Tier 1: Asserted (Weakest)
The model merely claims that the work was done in conversational prose:
> *"I have reviewed all customer records, resolved the duplicate rows, and updated the schema."*
- **Verifier:** Next-token generation.
- **Guarantee:** 0%. Highly vulnerable to hallucinations and first-signal bias.

### Tier 2: Cited (Traceable Reading)
The model backs up its statements by referencing specific, traceable source artifacts:
> *"Source: Q2_financial_report.pdf, page 14, byte offset 18,400."*
- **Verifier:** Document and retrieval telemetry.
- **Guarantee:** Proves that reading physically took place, but does not prove the model's interpretation is correct.

### Tier 3: Observed (Machine-Checked Reality)
The runtime independently recorded physical system receipts from subprocesses or tools:
> *Command `pytest tests/test_auth.py` exited with code 0. 14 passed. Workspace git diff confirms exactly 2 files modified.*
- **Verifier:** OS kernel, shell exit codes, and filesystem snapshots outside the model's control.
- **Guarantee:** High. Objective machine proof that external state was modified and validated.

### Tier 4: Attested (Maximum Authority)
An independent third party or human authority explicitly signs off on the outcome:
> *Cryptographically signed webhook from production CI/CD pipeline received, or human operator approved final diff.*
- **Verifier:** External cryptographic or organizational authority.
- **Guarantee:** Complete. Suitable for regulated enterprise workflows.

### The Lattice Rule
The governing law of the runtime is absolute: **a commitment may never close as Fulfilled below the proof strength demanded by its contract.**

If a task commitment specifies `Observed` proof (e.g. software testing or database migrations), ten thousand tokens of `Asserted` prose will be rejected by the gate at append time. The model cannot talk the runtime out of caution it established upfront.

---

## Why Honest Failure is Better Than Fake Success

When evaluating autonomous agents, many teams track a single, flawed metric: **Completion Rate.**

If you optimize your agent solely for completion rate, you incentivize it to hide its failures. When a model gets stuck, it will fabricate a plausible result, emit a stop token, and declare victory to keep its score high.

In production environments, a fake success is a disaster. It means silent bugs deployed to production, fabricated metrics presented to executives, and corrupted records written to customer databases.

A robust commitment engine embraces a counter-intuitive principle: **Honest failure is an asset.**

When a commitment closes, the ledger records a typed verdict:
- `Fulfilled`: The evidence ledger fully satisfied the contract criteria.
- `Breached`: The agent hit a budget cap, turn limit, or watchdog deadline without delivering proof.
- `Abandoned`: The human operator cancelled the task or superseded it with a new directive.
- `Unknown`: The process was interrupted or terminated abruptly without audit verification.

Recording `Unknown` or `Breached` is not a flaw; it is engineering honesty. It tells the human operator exactly where autonomy ended, what was achieved before interruption, and what remains to be done. A system that can record a lie is not an audit trail.

---

## The Takeaways for System Builders

If you are building or deploying autonomous agents for high-consequence work:

1. **Abandon prompt-only task tracking.** Context windows suffer from unavoidable attention dilution over long horizons. Store the goal state in an external state machine that the LLM cannot edit.
2. **Enforce pre-flight admission.** Agree on resource ceilings, allowed capability slices, and the Definition of Done before spending API compute.
3. **Audit against the satisfaction lattice.** Separate what the model *asserts* from what the runtime *observes*. If a deliverable requires machine-checked receipts, reject text claims that lack tool evidence.
4. **Value typed, honest failure.** An agent that suspends or reports a clean breach when blocked is an agent you can safely trust with production delegation.

Autonomy without enforceable contracts is just expensive guesswork. Autonomous delegation becomes production-grade when commitments are as disciplined as the software systems they run on.

---

## A Question for Builders

When your agents run for 20+ turns, how do you keep them from drifting into side-quests? Do you track goal completion inside the prompt history, or have you moved acceptance criteria into an external system?

I would love to hear how other teams are tackling context drift and proof of work. Let's discuss in the comments.

---

### Suggested Hashtags for Publication
`#AIAgents` `#SoftwareArchitecture` `#SystemDesign` `#AgenticAI` `#TechLeadership`
