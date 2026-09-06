# Provider, routing, and runtime integration audit — 6 September 2026

## Follow-up implementation status

The active stabilization goal has since landed a correction slice: goal/control classification uses command boundaries, half-open breakers reserve one probe, live evidence freshness is refreshed in Core, project credential pools are workspace-scoped, output caps only narrow, canonical configured provider names survive fallback receipts/events, and same-model alternate credentials are retained as independent route legs. Typed projection now reads the serialized evaluation fields at their actual offsets and admission is projected onto the following user turn; plan revision checks fall back to the live intent record and human plan edits no longer mutate resolver identity. Watchdog cancellation now revokes the provider attempt token, model discovery rejects empty catalogues and stale cached catalogues cannot enter a route ladder, child and flow executions inherit retry/watchdog/breaker/spend policy, final evaluation uses the admitted outcome revision, current-turn evidence is isolated, and refusal responses cannot satisfy deliverables. Focused regression tests and the full workspace suite cover the implemented changes. The remaining findings below are still open until their production-path contracts are implemented and verified.

**Recommendation: hold the larger rollout until the authority, outcome-revision, and evidence defects below are fixed.** The architecture has useful foundations, but several advertised guarantees currently exist in types, comments, or isolated tests without holding across their consumers. Adding more classifiers, renderers, or routing heuristics before repairing those boundaries will multiply inconsistent behavior.

## Scope and evidence

Reviewed the current working tree, including untracked `vak-intent/src/goal.rs`, `outcome.rs`, and design 52, against base commit `edaa5eb6fb9fa1c3bfc4857df8e9220a1b65f328`. The pass covered provider construction/discovery, ladder admission and dispatch, retries/stream ownership, Core configuration, agent/task/flow consumers, outcome evaluation, ledger projection, server/gateway controls, delivery/plugins, UI controls, and evaluation changes. Ancillary manifests, schema changes, site changes, and documentation were inspected for integration implications.

The working tree continued changing during review, including site/documentation edits. Findings refer to the cited source locations at review time; this is not certification of subsequent edits or exhaustive proof of every unchanged subsystem. No application source was changed. This report is the only repository artifact created by the audit.

Validation completed:

- `cargo test --workspace`: **1,452 passed, 0 failed, 3 ignored**, across 120 result groups.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.
- Client UI `npm run typecheck`: passed.
- Version and documentation-path checks: passed.
- Separate Rust probes, outside the repository, exercised the built libraries with fixture credentials and an isolated home. Confirmed false completion, false goal classifications, unrestricted half-open probes, adapter/provider attribution collisions, missing project pool keys, ineffective live evidence configuration, and production projection mismatches.

No paid provider calls, live external integrations, deployment, or browser interaction were performed. Compilation verifies the embedded-bundle checks at build time; a full interactive UI acceptance run remains necessary. Tests and probes are evidence of the cases they execute, not proof of the design's broader completion claims.

## Confirmed findings and source-supported defects

P1 means fix before expanding or releasing this runtime path. P2 means a material correctness/reliability defect that should be included in the stabilization sequence. “New” identifies current uncommitted behavior; “existing” identifies a pre-existing boundary exposed by the expanded review.

### F01 — P1 — Nonempty prose satisfies arbitrary deliverables (new; reproduced)

[Requirement evaluator](/Users/nisheethranjan/Projects/vakcoder/crates/vak-intent/src/outcome.rs:399) marks **every** Deliverable requirement `Met` when any response text exists. It does not examine requirement description, target, output artifacts, or actual task success. A probe requested a CSV with 100 verified rows, added that as an explicit mandatory deliverable, supplied `I cannot do that.`, and obtained `Complete`.

This is especially dangerous because `human_review_state(Complete)` returns `not_required`. Runtime execution of the evaluator does not make its evidence strong. A refusal, an unexecuted plan, or an unrelated answer must not establish satisfaction. Bind each requirement to a checkable result; when no applicable evaluator exists, return Unknown. Keep Produced as a separate fact.

### F02 — P1 — Accepted outcome revisions are discarded at final evaluation (new; source trace)

The agent consumes updates into `self.config.outcome` at [the safe boundary](/Users/nisheethranjan/Projects/vakcoder/crates/vak-agent/src/lib.rs:766). But [Core's final evaluator](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:5107) reconstructs an OutcomeSpec from the **original prompt and original reading**, resetting revision to zero and losing added/removed requirements. New turns likewise construct a fresh spec rather than composing the durable accepted one. Task dependencies capture an earlier clone.

An accepted “also include a CSV” can therefore be recorded while final evaluation ignores the CSV requirement. Evaluate the exact committed revision that governed the result, and carry its identity through child results and delivery. Test revisions during execution, while idle, across follow-up turns, and after restart.

### F03 — P1 — Child and flow inference bypass the parent's spending machinery (existing, affected by new outcome propagation; source trace)

[Task AgentConfig construction](/Users/nisheethranjan/Projects/vakcoder/crates/vak-agent/src/task.rs:435) and [flow node construction](/Users/nisheethranjan/Projects/vakcoder/crates/vak-flow/src/exec.rs:536) preserve permission/sandbox fields but construct fresh defaults without the parent's spend gate, breaker, frozen ladder, context policy, or retry settings. [Core attaches its spend gate separately](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:4532) to the main AgentConfig. Adding `outcome` to TaskDeps/ExecutorDeps does not close this gap.

A parent spending cap does not authorize each child provider dispatch. Introduce one admitted execution context consumed by main runs, children, flows, planning, and evaluations, with atomic shared spending reservations and explicitly narrowed per-child limits. This finding does not claim that filesystem permission checks are bypassed.

### F04 — P1 — Main intent limits are not wired to the newly added counter (new; source trace)

[Core creates admitted_outcome](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:4374) using `from_reading`, which initializes `max_turns: None`; Core never copies the engagement's turn cap into that execution object. The server preview does copy it, so the preview and execution disagree. The new agent counter checks only `outcome.max_turns`. The final/recorded specs also omit the cap.

Additionally, [TaskTool's subagent budget](/Users/nisheethranjan/Projects/vakcoder/crates/vak-agent/src/task.rs:291) rejects only `Some(0)`; positive budgets are not consumed. Ladder limits are applied at prompt-aware session creation, not re-applied from each later turn's engagement before fallback construction. Bind all admitted limits to their actual dispatch consumers. Use Core-level integration tests rather than manually constructing already-correct AgentConfig fixtures.

### F05 — P1 — A watchdog timeout can leave the paid provider stream alive (existing; source trace)

[The watchdog](/Users/nisheethranjan/Projects/vakcoder/crates/vak-agent/src/lib.rs:2598) drops the consuming future on timeout. Adapters such as [Responses](/Users/nisheethranjan/Projects/vakcoder/crates/vak-llm/src/openai_responses.rs:386) spawn independent stream-reading tasks with a clone of the run cancellation token. [EventStream](/Users/nisheethranjan/Projects/vakcoder/crates/vak-llm/src/stream.rs:94) has no cancellation-on-drop ownership for that task. Dropping the stream releases the provider permit while the producer can keep reading the network.

A retry can overlap the old request, invalidating concurrency and cost assumptions. Use an attempt-owned cancellation token and producer handle; timeout/drop must cancel and settle that attempt while preserving the latest partial snapshot. Verify with a local streaming server that observes connection closure before the retry starts. This overlap was traced in source, not exercised against a paid provider.

### F06 — P2 — Routing evidence records adapter identity instead of provider identity (existing; reproduced)

[Dispatch receipts](/Users/nisheethranjan/Projects/vakcoder/crates/vak-agent/src/lib.rs:2492) stamp `provider_arc.name()`. Probes show configured `openrouter`, `ollama`, and `opencode-zen` all report `openai-completions`; `openrouter-responses` reports `openai-responses`. Admission candidates and [evidence lookup](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/routing.rs:102) use configured provider names.

Evidence therefore misses its intended candidate or merges unrelated endpoints. OpenAI's automatic Responses selection has the same mismatch. Preserve the canonical RouteLeg identity, endpoint/credential scope, and wire dialect separately through dispatch and receipts. An adapter name is not a provider/account identity.

### F07 — P2 — Project credential pools lose their secondary keys (existing; reproduced)

[Pool assembly](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:3283) resolves the primary via workspace-aware `provider_secret` but reads the plural pool through process-global `vak_config::get_var`. An isolated project with one singular key and two plural keys produced **one** credential identity instead of three.

Use one scoped resolver for singular and plural secrets, discovery, status, and dispatch. Test two pooled workspaces with different project pools and a Shared pool, including replacement and removal. Do not infer that successful singular authentication proves the pool works.

### F08 — P2 — Same-model alternate credentials are removed from the ladder (existing; source trace)

Admission can create a leg for a second credential, but [assemble_ladder](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/routing.rs:287) skips any leg with the primary's provider/model pair, regardless of credential identity. Thus fixing pool discovery alone still will not enable same-model alternate-key fallback. Provider-seat caps can further exclude healthy alternatives from a single-provider installation.

Deduplicate complete route identities. Model provider-wide, endpoint-wide, and account-specific failure domains explicitly; diversity should not accidentally erase independently usable account capacity.

### F09 — P2 — Frozen fallback availability depends on picker history and ignores cache age (existing; source trace)

[plan_route_ladder](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:3778) uses only the in-memory discovery cache and discards the timestamps while reading it. The production discovery callers found are setup and the server model-list endpoint; there is no provider-catalog warm/reconcile path at admission. A freshly started headless process can freeze a single-leg route despite configured fallbacks, while a long-lived process can admit indefinitely stale model IDs.

Create a bounded, level-triggered discovery service outside turn admission, retain freshness/error state, and make cold/stale/unavailable outcomes explicit. Preserve the no-network-on-critical-admission principle without making reliability depend on opening Settings. Test cold boot, expiration, model removal, and key replacement.

### F10 — P2 — Model listing does not prove the capability required by a route (existing; architectural correctness gap)

[Discovery](/Users/nisheethranjan/Projects/vakcoder/crates/vak-llm/src/models.rs:240) reduces models to strings; missing response structure becomes a successful empty list. Google generation-method metadata is ignored. [Dialect selection](/Users/nisheethranjan/Projects/vakcoder/crates/vak-llm/src/route.rs:43) selects Responses from provider name plus tool/reasoning demand without per-model capability evidence.

The implementation therefore cannot substantiate its “executable candidate” claim merely from catalogue membership. Preserve discovered capabilities and unknown states, validate response/pagination completeness, and use explicit endpoint support or bounded probes. Do not replace these gaps with a hardcoded model catalogue. Actual upstream support remains a live-validation question.

### F11 — P2 — Context-limit calculation can widen an explicit output cap (existing; source trace)

[route_context_limits](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:3737) returns `max_output.max(1_000.min(context_window))`. A configured or provider-reported 128-token output maximum becomes 1,000 when the context window permits it. This is the opposite of narrowing.

The same path performs sequential metadata HTTP calls before agent execution, silently converts errors to None, caches that failure as data for five minutes, and does not take a cancellation token. Separate metadata availability from values; move refresh off the critical path and enforce cap composition with `min` throughout.

### F12 — P2 — Circuit half-open state admits every caller (existing; reproduced)

[check_key](/Users/nisheethranjan/Projects/vakcoder/crates/vak-agent/src/circuit.rs:75) clears `opened_at` when cooldown expires but never claims a probe. Two checks without an intervening settlement both returned Ok in the probe. Concurrent sessions can all retry a recovering provider. The inner per-leg retry loop also checks the breaker only before entering the leg.

Represent Closed/Open/HalfOpen explicitly and reserve a single probe atomically. Define cancellation/timeout cleanup for the probe and use typed cooldown information instead of relying on an error string.

### F13 — P2 — Typed timeline reads the wrong completion and receipt fields (new; reproduced)

[Projection builds](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/projection.rs:200) `status|evaluation_json|receipt_ids|completion`. But [result_outcome](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/projection.rs:29) reads index 2 as completion and index 1 as receipt IDs. [Output status](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/projection.rs:285) likewise reads index 2. Document metadata later parses a shortened string correctly, so the typed result can disagree with its own document.

A separate fixture compiled the actual projection source and produced `item status Partial`, `typed completion Some("receipt-123")`, `typed receipt ids Some(["[]"])`, while document completion remained `complete`. Use a typed evaluation object end-to-end. Tests must compare typed status, document metadata, channel projection, and replay from the same real record, including zero and multiple receipts.

### F14 — P2 — Outcome admission is attached to the previous turn in projection (new; reproduced)

Core appends Intent before Agent.run appends the new user message. [Projection](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/projection.rs:160) attaches Intent to `scan_turn`, which advances only when that user message arrives. The first request's outcome lands at turn zero; the next request's intent can decorate the previous answer. The production-projection fixture confirmed `document objective None` despite a preceding admitted objective.

Use explicit turn/admission IDs. Test the actual production append order, steering messages, image-only requests, compaction, and forks. A synthetic fixture that appends messages in a different order cannot validate this boundary.

### F15 — P2 — Plan revision checks fail during the runs they are intended to control (new; source trace)

[plan_change](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/lib.rs:3881) reads the revision only from `handle.session`. While a runner owns the log this is None, so revision becomes zero. `/control-state` correctly falls back to `handle.intent`, making its advertised revision unusable for the next change after revision one. UI requests can then receive false stale conflicts. Without target_revision, separate clone/write locks allow concurrent changes to overwrite each other's requirements.

Use one atomic revision compare-and-append operation. Do not increment `resolver_version` for a human scope edit: that field identifies the resolver implementation, not the contract revision. Route `/steering` scope changes through the same operation; currently it accepts them as text without creating the revision that `/plan-change` creates.

### F16 — P2 — Reviewing an older result records a verdict on the latest result (new; source trace)

[Review actions](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/projection.rs:857) carry only session_id and verdict. [The endpoint](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/lib.rs:4141) selects the latest evaluation. Clicking Accept on an older turn therefore accepts the newest evaluated turn. During an active run, the missing log is reported as 404 despite the task existing.

Require result/evaluation ID and revision in the action and validate that exact target. Append the verdict to that target, with explicit stale/conflict behavior. Avoid a full-page reload as the client synchronization mechanism.

### F17 — P2 — Goal projection invents control state from incidental words (new; reproduced)

[The classifier](/Users/nisheethranjan/Projects/vakcoder/crates/vak-intent/src/goal.rs:81) uses arbitrary substrings. Probes returned Status for `Implement a progress bar`, Cancels for `Fix the cancellation button`, and Pauses for `Set the threshold to 10` because “threshold” contains “hold”. A first `Continue building the app` produces Resumes without establishing an objective.

These labels affect the displayed GoalState; they are not proof the run was actually paused/cancelled. Real pause/resume handlers record Activity rather than the GoalUpdate consumed by this projection. [Goal revision lookup](/Users/nisheethranjan/Projects/vakcoder/crates/vak-session/src/log.rs:194) also scans all entries rather than the active branch, and Core allocates revisions from active work revisions rather than the latest control revision. Use explicit control events and conservative intent interpretation; preserve branch-local monotonic revision identity.

### F18 — P2 — Pause/resume has a lost-wakeup window (new; source trace)

[wait_if_paused](/Users/nisheethranjan/Projects/vakcoder/crates/vak-agent/src/steering.rs:60) checks the atomic flag before registering the Notify waiter. `resume()` can clear the flag and call `notify_waiters()` between those operations. The waiter can then sleep indefinitely until a second resume or cancellation.

Use a watch channel or register the waiter before rechecking state. Distinguish pause requested from pause reached; the UI currently says “Paused at safe boundary” immediately after setting a request flag, while a provider/tool operation can still be executing.

### F19 — P2 — Evidence policy saves successfully but does not become live (new; reproduced)

[The endpoint](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/lib.rs:7930) persists the value and calls refresh. [refresh_persisted_preferences](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:2248) never updates intent settings. The probe observed disk=120 seconds and live Core=86,400 after a successful refresh.

The UI always writes this setting when saving Agent defaults, reads its effective value, and rounds it to whole hours. That can materialize an inherited value into a narrower layer or overwrite sub-hour precision. Give it the same scoped persisted/effective/provenance contract as other preferences. Reuse one serialized configuration writer rather than independent read-modify-write locks.

### F20 — P2 — Result/evidence/extension contracts are still disconnected (new; source trace)

[Core evaluation](/Users/nisheethranjan/Projects/vakcoder/crates/vak-core/src/lib.rs:5117) collects successful tool calls across the entire session and uses the latest successful receipt's timestamp as the evidence state for the current outcome. A successful unrelated command can label a later request's evidence Fresh. It does not establish producer, claim relevance, artifact revision, or requirement/result binding. The evaluator conservatively leaves Evidence requirements Unknown, which avoids claiming those are verified, but the exposed evidence metadata is still misleading.

[Projection merges plugin requirements](/Users/nisheethranjan/Projects/vakcoder/crates/vak-server/src/projection.rs:278) only into a presentation clone, after runtime evaluation. Those requirements are neither admitted in the durable outcome nor evaluated with it. Production projection assigns turn-level evaluations to answer text blocks rather than independently evaluated result records. Finally, [built-in recipes](/Users/nisheethranjan/Projects/vakcoder/crates/vak-delivery/src/skills.rs:475) remove the typed-output requirement from research recipes while retaining it only for test reports, contrary to the new evidence-gated recipe contract.

Implement result-scoped evidence/evaluation as one vertical slice before advertising general result-level completion. Register plugin requirements at admission, bind evidence to result and requirement IDs, and let presentation consume those decisions without creating new ones.

## Architecture and validation assessment

The strongest existing pieces are the append-only ledger, explicit permission/broker boundaries, credential fingerprints, pure ladder ordering, typed provider errors, and shared UI/server types. Preserve these.

The main weakness is repeated reconstruction: provider identity is reconstructed from adapters; outcome state from prompts; result completion from delimiter strings; controls from keywords; current policy from immutable configuration snapshots. Each creates another source of truth. A single authoritative record with typed consumers would remove more defects than additional layers of inference.

The routing code primarily orders **fallbacks** while keeping the operator-selected primary pinned. That is a reasonable product contract, but it is not adaptive primary-model selection. Its score uses transport settlement and latency plus configured quality hints, not demonstrated task quality. Beliefs are stored on Core despite being described as session-scoped. Demand input tokens are always passed as zero, and prompt-less admission is used by important surfaces. Distinguish these facts from claims of outcome-aware intelligent routing.

The new [comparison evaluator](/Users/nisheethranjan/Projects/vakcoder/crates/vak-eval/src/runner.rs:193) replays the same scripted provider actions with/without OutcomeSpec. It can validate reporting and restrictions; it cannot measure improved model decisions, discovery, routing, semantic task completion, or practical token savings. The external verifier influences eval verdicts through a path not used by ordinary Core completion. Add held-out live/local-server fault cases only after deterministic production-path acceptance tests exist. Do not present the current three scripted held-out cases as evidence of general workload improvement.

Design 52's “shipped; cross-surface acceptance verified” and “workspace consumer matrix verified” descriptions are stronger than this implementation supports. Change those claims to track specific acceptance cases and unresolved defects before release.

## Ordered implementation plan

### 1. Freeze the contracts and capture failing production-path tests

Record an immutable patch/commit for the coming change. Define the identities that must survive every stage: canonical route, credential/endpoint scope, turn/admission, outcome revision, result, requirement, evidence receipt, and review target. Turn F01–F20 into regression cases with fixture credentials, isolated homes, and local provider servers. Keep tests against Core and HTTP consumers, not only hand-built structs.

Exit: every must-fix scenario fails for the intended reason; unrelated baseline tests stay green. No new product behavior is required in this phase.

### 2. Repair dispatch ownership and enforce admitted limits

Address F03–F05 and F12 first. Introduce a shared admitted execution context that carries authority, route, context limits, cancellation ownership, and spending reservations through every inference path. Narrow child budgets atomically; account for all actual paid attempts. Cancel/join timed-out producers before releasing capacity. Apply per-turn intent limits to the frozen ladder prefix without widening or reordering it.

Exit: zero-budget children cannot dispatch; positive delegation budgets are consumed; main intent caps work through Core; all permitted fallbacks stay inside the frozen contract; cancellation leaves no orphan stream and preserves partial output.

### 3. Repair provider identity, discovery, and model metadata

Address F06–F11. Separate provider/account identity from protocol adapter; unify scoped credentials; preserve same-model alternate accounts; reconcile fresh model metadata outside turn admission; reject malformed/incomplete discovery; enforce context/output caps without floors that widen them. Preserve useful explicit primary choices while making fallback availability and incompatibility reasons inspectable.

Exit: cold-start and warm-start behavior has an explicit contract; stale/malformed discovery cannot masquerade as executable capability; receipts map back to exact route legs; project pools remain isolated; endpoint changes and key revocation behave consistently.

### 4. Make revision and evidence the source of completion

Address F01–F02, F13–F16, and F20. Commit one outcome revision, evaluate exact result/requirement bindings, and produce a typed evaluation record. Reuse it for Core, HTTP, gateway, desktop, channel text, and historical replay. A renderer must not introduce unevaluated requirements. Result review must target the result the user clicked.

Exit: an absent/wrong artifact cannot be Complete; unrelated or stale receipts cannot verify it; adding/removing a requirement changes the next evaluation; restart/fork/replay preserves result identity and verdict; every surface agrees.

### 5. Unify controls and configuration, then verify the UI

Address F17–F19 plus scope-change routing in F15. Use explicit state transitions, atomic revision checks, and durable control events. Show requested vs reached pause states. Make evidence policy a real scoped preference with immediate effective-state reporting. Test desktop/web and channel control flows, including active-run plan edits and reviewing older results.

Exit: no incidental word pauses/cancels the goal projection; resume cannot lose a wakeup; repeated plan edits succeed with current revisions and reject stale ones; save/reload/live execution agree across user/project scopes.

### 6. Release only with an evidence-backed consumer matrix

Run all existing checks plus the new production-path cases. Rebuild and verify all touched embedded bundles. Exercise mixed-result tasks, multi-turn revisions, short/long context, cold discovery, credentials with different model access, concurrent jobs, truncated streams, timeouts, quotas, and channel delivery/restart. Then perform representative model-backed evaluation with explicit cost approval/bounds and documented coverage.

Exit: publication claims match tested behavior; remaining operational gaps are named. Ship this in small reviewable changes organized around these contracts rather than one larger framework patch.

## Open validation work

The audit has not demonstrated actual provider endpoint capability coverage, a browser UI run, process-crash durability of accepted pending revisions, paid-attempt settlement under all disconnect timings, or all security/installation paths outside the reviewed dependencies. Those remain explicit acceptance work, not implied passes. The current green suite is a useful baseline; the reproduced counterexamples explain why it is not sufficient release evidence.

## Local validation artifacts

- [Workspace test log](/tmp/vak-audit-tests-20260906.log)
- [Clippy log](/tmp/vak-audit-clippy-20260906.log)
- [General probe source](/tmp/vak-audit-probes-20260906.rs)
- [Scoped Core probe source](/tmp/vak-audit-core-probe-20260906.rs)
- [Production projection fixture](/tmp/vak-audit-projection-project-20260906/main.rs) and [output](/tmp/vak-audit-projection-20260906.log)
- [Working-tree file hashes captured during the audit](/tmp/vak-audit-reviewed-tree-20260906.json); this records observed files, not an atomic repository snapshot.
