# 40 — Harness engineering
Status: implemented in 2.0.0

VAK has four complementary regression lanes:

* `scripts/harness_500.py` discovers library, binary, and integration tests,
  tags them by capability, runs a balanced matrix with isolated subprocesses,
  per-case timeouts, parallel workers, and JSON/Markdown evidence.
* `scripts/compound_regression.py` exercises multi-step offline workflows and
  bounded live Ollama workflows, including skills, child agents, memory,
  plugins, MCP, schedules, hooks, and feed ingestion.
* `scripts/prompt_scenarios.py` gives a real model natural-language tasks in
  fresh workspaces and verifies postconditions, scope, ledger evidence, and
  final output. Model mistakes are retained as failures rather than hidden by
  retries.
* `vak eval` is the no-model continuous lane. Its scripted provider exercises
  the production loop and brokered tools against fixed and generated generic
  work scenarios. Seeded `(seed, index)` cases can be run in bounded batches,
  while live-model quality remains a separate, opt-in measure.

The deterministic runner discovers individual Rust tests at execution time. A
balanced 500-case run is the default acceptance gate; it is a selected unique
subset, not 500 claims of end-to-end model quality. Repetition is reported as
execution volume and never added to the unique-scenario count. Prompt-driven
cases are the quality lane, and live Ollama runs are bounded because model
inference is substantially slower and less deterministic than Rust tests.

Prompt scenario identity is a hash of the task, fixture, postcondition, output
contract, scope contract, and allowed changes. Renaming an identical fixture
does not create a new semantic scenario. Evidence comes from parsed JSONL
`tool_use` records, not substring matches against user or system prompt text.
The runner snapshots the complete workspace around each case, rejects
unexpected changes, verifies that child process groups are gone, writes atomic
partial reports plus a heartbeat, supports resume, and holds an exclusive
stale-PID-aware report lock so two processes cannot corrupt one run.

Live result status is multidimensional. Provider admission failures are
`blocked` and do not run fixture postconditions; functional completion and
skill/tool/orchestration contract compliance are recorded independently. A
successful implementation with a hallucinated capability call is therefore
visible as functional success plus contract recovery/failure, rather than being
collapsed into either a false pass or a generic agent failure. Reports retain
redacted tool traces, selected skill paths, verification counts, process-group
cleanup, Git revision, and binary digest so a failure remains diagnosable after
the temporary workspace is removed.

Live results are reported on three separate axes: correctness (postconditions
and scope), control-plane integrity (ledger, timeout, cancellation, and child
cleanup), and performance (latency, queueing, concurrency, CPU/RSS, and
thermal observations). A local model may pass correctness while exceeding a
remote-model-oriented timeout under concurrency; that is recorded as a
capacity result and must not be silently converted into a correctness pass.
Concurrency acceptance therefore includes a serial correctness baseline,
  provider-specific latency budgets, and an explicit saturation curve instead
  of one universal timeout. All built-in streaming providers use one shared
  provider endpoint+credential-fingerprint lease across processes: local
  routes default to one slot, remote routes to eight, and
  `VAK_PROVIDER_CONCURRENCY` overrides the slot count for either route. The
  key is model-independent: models using one endpoint and credential share
  that provider/key capacity, while distinct credentials get independent
  pools. Raw credentials never enter the lease filename or report.

### Provider routing policy

Routing is admission-time constraint satisfaction followed by a frozen,
deterministic ladder. A candidate is eligible only when its endpoint is
configured, credentials are present, the model was discovered or explicitly
permitted, the request fits its context/tool/structured-output capabilities,
and its current health/quota state is not a hard deny. Among eligible
candidates, VAK scores demand fit, recent reliability, latency, observed
queue/load, remaining request and token quota, cost, and the declared quality
objective. Capacity and quota are hard constraints when known; unknown values
are neutral with an annotation, never fabricated as unlimited. A 429,
retry-after, or provider overload updates the provider/key cooldown and causes
admission to prefer another eligible pool; transient failures use bounded
exponential backoff with jitter, while authentication, permission,
invalid-model, and invalid-request failures are not retried blindly.

The route is frozen into the session contract so a changing workspace default
cannot rewrite an active session. Dispatch walks only that ladder, records
provider/model attribution for every attempt, and may settle to a later leg
only on a typed failure permitted by the work receipt ceiling. A provider/key
pool is the capacity owner; model is a capability and quality dimension
inside that pool, not a separate concurrency pool.

### Signal normalization and scope

The router must retain signal scope instead of flattening every value into a
per-key counter:

| Signal family | Normalized value | Possible scope | Unknown policy |
| --- | --- | --- | --- |
| Request quota | limit, remaining, reset | provider, organization, project, workspace, model class | do not reserve or advertise headroom |
| Token quota | input/output/total limit, remaining, reset | provider, organization, project, workspace, model class | estimate locally, mark stale/estimated |
| Spend quota | remaining cap, reset | account, billing project, workspace | hard-deny only when explicitly exhausted |
| Queue capacity | running, waiting, concurrency | endpoint, server, model | use local gate when no server metric exists |
| Health | success, overload, timeout, auth, invalid request | endpoint+credential, optionally model | circuit-break only retryable domains |
| Performance | TTFT, completion latency, tokens/sec | endpoint+model | use TTL-weighted observations, never invent zeros |
| Capability | context, vision, tools, structured output, modality | model+endpoint | hard-filter when known unsupported |

Every observation carries `observed_at`, `source`, `scope`, and freshness. A
provider adapter may report several overlapping scopes; the admission check
uses the most restrictive applicable known limit. Quota identity is not
assumed from credential identity: Anthropic documents organization/workspace
limits, Gemini documents project limits, and providers may group multiple model
ids into one model-class bucket. Conversely, a local endpoint's physical queue
is shared by its loaded server capacity. This is why the lease is endpoint plus
credential while quota observations retain their provider-declared scope.

Selection is lexicographic before weighted scoring: permission/capability and
hard quota eligibility, then cooldown/health, then predicted completion
deadline and queue risk, then objective-specific quality/cost/reliability. A
weighted score must never allow a cheap but ineligible or known-overloaded leg
to outrank an eligible leg. The chosen ladder records the signal snapshot that
caused admission, allowing later reports to distinguish a routing decision from
the provider state that changed afterward.

## What a mature harness still adds

### Context portability is a routing constraint

Every route leg has its own effective context window and maximum output. The
router must treat `estimated_input + reserved_output <= model_limit` as hard
eligibility, not as a quality preference. Unknown limits are explicitly
unknown and never treated as unlimited.

If a fallback is smaller than the current model, VAK must compact the
append-only transcript into a durable compaction entry before dispatching that
leg. It keeps the original ledger and recent actionable turns, then
re-estimates against the fallback limit; it never silently truncates or
deletes conversation state. If a provider reveals a smaller limit only at
dispatch time, typed context overflow must trigger the same compaction-and-
retry path rather than turning a recoverable route change into a failed task.

The strongest public agent-evaluation systems combine several ideas that VAK
should continue adopting:

* LangSmith separates offline datasets from online production evaluation,
  supports trajectory/tool-call evaluation, repetitions, concurrency, and
  experiment comparison. VAK has the local matrix and append-only evidence, but
  should add durable evaluator scores and an online feedback importer.
* OpenAI Agents SDK tracing records model generations, tool calls, handoffs,
  guardrails, and custom events, while its testing guidance uses deterministic
  provider-neutral doubles. VAK has session JSONL, receipts, and event
  projections; the next parity step is a normalized trace/span export with
  redaction and flush guarantees.
* SWE-bench evaluates real repository issues in resettable sandboxed
  environments and scores whether the resulting patch solves the issue. VAK
  has worktrees, sandboxing, tests, and code-task prompts, but needs a
  curated real-issue corpus and patch-level scoring before claiming SWE-bench
  parity.
* BrowserGym treats browser tasks as an extensible benchmark family with
  reproducible task state and trajectory evaluation. VAK has browser tools and
  SSRF/security coverage, but should add a small opt-in browser task corpus
  when browser dependencies are available.

## 24/7 operating model

Run the deterministic lane continuously with bounded workers and retain each
JSON report. Run prompt/live lanes on a slower schedule with model, provider,
workspace seed, and git revision recorded. Alert on new failures, timeout
percentiles, tool-call drift, scope violations, missing ledger evidence, and
quality-score regressions. A soak run must be treated as evidence of behavior
under a duration and load budget, not as proof of indefinite availability.

Example commands:

```text
vak eval --generated 500 --seed 20260930 --offset 0 --report target/eval-0.json
vak eval --generated 500 --seed 20260930 --offset 500 --report target/eval-500.json
python3 scripts/harness_500.py --limit 500 --repeat 1 --workers 8
python3 scripts/compound_regression.py --repeat 2 --live --provider ollama --model gemma4:e2b-mlx
python3 scripts/prompt_scenarios.py --provider openrouter --model <free-model> --workers 2
```

The prompt and compound live lanes accept an explicit provider; they must not
silently force Ollama. Their temporary `HOME` isolates feed databases, while
the harness preserves the interpreter's installed dependency paths so a
machine-local DuckDB/feed installation remains usable without a manual
`--feed-site` override.
