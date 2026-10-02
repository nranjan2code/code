# Reliable work execution — implementation plan

Status: in progress; E0 audit is incomplete and E2 has a first recovery slice.
Date: 2026-09-30.
Reviewed source: `0bfcaa39e341cef46360513e9d6f037c0f5f2493`.
Design review: [reliable-work-execution-review.md](reliable-work-execution-review.md).

## Objective and acceptance

Complete general-purpose work at the existing evidence and quality standard,
using fewer unnecessary provider attempts and smaller justified requests, while
preserving conversation recall, artifacts and resumable progress.

The acceptance gate is joint: quality, correctness, memory, latency and resource
use. Reducing tokens by omitting a required fact, lowering evidence requirements,
abandoning work early, or mislabelling a partial result is a regression.

This is an extension of designs 15, 42, 47, 50, 52 and 68. It uses existing work
ownership and storage, and does not depend on starting M1 of the data refactor.

## Decisions to settle in stage E0

1. Root ownership: use the existing admitted request/episode and its source ledger
   identity; distinguish it from the session and from a durable commitment. Do
   not introduce the future data-architecture `Run` record in this change, but
   key any durable root account by that plan's typed `RunId`, which lands
   first in its M1 (data plan revision 3, review 2 R46), so M4 adopts the
   account instead of replacing it.
2. Policy: finite outer resource limits come from operator/user policy and current
   admission. Calibrate defaults against measured workloads. Learned estimates
   may allocate within those limits, never silently raise them.
3. Identity: define which provider observations prove shared account capacity,
   and how region, model group and endpoint-specific limits narrow it. Unknown
   sharing stays explicit. Reuse existing credential fingerprint/identity seams.
4. Recovery: distinguish temporary throttling, request too large for a refill
   window, exhausted quota, overload and uncertain settlement. Agree how each
   enters the existing error, ladder, endurance and outcome contracts.
5. Scope: account coordination is shared across all Core instances in a service
   process. Independent processes must not claim globally enforced limits without
   a shared authority. Audit current gateway/bus ownership before choosing any
   additional cross-process mechanism.
6. Resumption: rederive spent resources, completed evidence and outstanding work
   from existing durable ledgers. Unknown in-flight settlements stay uncertain;
   effects require reconciliation before retry.

### Provider capacity inventory (2026-10-02)

Model IDs remain live-discovered per credential; this is an adapter inventory,
not a baked-in model/limit catalogue. A route's effective quota may be narrower
than its credential identity, so every observation needs an explicit scope and
freshness before it can drive admission. The selectable route identifiers in
this tree are `openai`, `openai-responses`, `anthropic`, `google`, `openrouter`,
`openrouter-responses`, `opencode-zen`, `bedrock`, and `ollama`. They map to
seven capacity families below: the OpenAI and OpenRouter pairs intentionally
share account/model gates; the OpenAI-compatible providers use the OpenAI
protocol adapter without inheriting OpenAI's account limits. There is no
separate runtime provider adapter for arbitrary OpenAI-compatible endpoints;
they are configured as one of those route families.

| Adapter family | Capacity scope to account for | Useful live evidence | Not inferable from model ID alone |
|---|---|---|---|
| OpenAI Chat, Responses, audio transcription/speech, Realtime | Organization/project, model or shared model group, credential and endpoint | Chat/Responses plus audio transcription/speech capture request/token headers and resets; project token headers are recorded on the shared account gate, so different model routes coordinate; Realtime normalizes its websocket URL to the HTTP API base and joins the same account/model gate, using handshake headers where present | Project headers are only observed when returned; published shared-model groups are not discovered; daily spend/quota control plane remains unqueried |
| Anthropic Messages | Organization and optional Workspace plus model/rate-limit group; request and token buckets | Messages adapter captures request and generic or separate input/output token headers, each reset, and `Retry-After`; authenticated `anthropic-organization-id` responses are one-way fingerprinted so keys in the same organization share process-local model gates and throttle cooldowns after their first response; typed API error class | Workspace identity/overrides and shared model-group aliases are not yet applied; the configured Rate Limits API requires `org:admin` credentials and does not return live remaining capacity, so it is not queried; daily billing/spend ceilings remain unavailable from the inference response |
| Google Gemini GenerateContent, Live and audio | Project (not API key) and model; RPM, input TPM, RPD/TPD, sometimes spend | Set `[providers.google].project_id` to share capacity observations across keys for one project; GenerateContent, Live transcription, and Interactions inspect typed `QuotaFailure` details when returned; quota metric/id/value and the Pacific daily reset feed short-window or daily request/token admission. Explicit input/output token metrics use their respective buckets; ambiguous daily token metrics reserve against combined demand. Compatible response headers are also captured. Live calls join account/model admission and send API keys in headers | Project identity remains credential-scoped when `project_id` is unset; limits and usage not included in an error response; Gemini's configured project limit baseline is not queried from an authorized quota surface; Live websocket has no successful-response usage counters |
| OpenRouter Chat/Responses | OpenRouter account/key and model/provider route; free-model request/day ceilings or key spend/reset budgets; BYOK may add upstream scope | OpenAI-compatible adapters capture standard headers; authenticated `/api/v1/key` status reports account budgets and observed free-model daily requests; a per-account single-flight refresh runs at most once per minute before admission and feeds only `:free` route admission; `/api/v1/credits` reports credits | Refresh errors leave the last sample in place until its five-minute expiry; upstream model/provider quota, especially if OpenRouter routes/falls back internally |
| Configured OpenAI-compatible gateway and OpenCode Zen | Gateway-defined account, model, tenant, endpoint or upstream dimensions | Standard rate-limit and project-token headers plus typed errors when emitted; routes reuse OpenAI protocol adapters | any limit/scope the gateway does not publish; generic gateways have no universal introspection API |
| Amazon Bedrock Mantle | AWS account, endpoint Region and model; separate input TPM and output TPM on models with published quotas; no RPM quota on Mantle | OpenAI-compatible headers and typed 429 errors if emitted; bounded concurrency and provider-specific reservations when separate quota observations exist | Admission reserves estimated input plus `max_tokens` against input TPM and `max_tokens` against output TPM; settlement accounts actual non-cached input plus actual output in the input bucket and actual output in the output bucket. Cached input is exempt and AWS replenishes unused output reservation. Vak does not query Service Quotas or receive a documented Mantle quota-remaining header; numeric bucket enforcement therefore waits for trustworthy separate observations, and cached input cannot yet reduce the conservative estimate |
| Ollama | Local host/model scheduling and operator resources | local overload errors and per-endpoint concurrency gate | hosted TPM/day quotas (not applicable unless a remote compatible gateway is configured); token/request windows are not reported by the adapter |

Provider source references: [OpenAI rate limits](https://developers.openai.com/api/docs/guides/rate-limits), [Anthropic rate limits](https://platform.claude.com/docs/en/api/rate-limits), [Anthropic Rate Limits API](https://platform.claude.com/docs/en/manage-claude/rate-limits-api), [Anthropic API response headers](https://platform.claude.com/docs/en/api/overview), [Gemini rate limits](https://ai.google.dev/gemini-api/docs/rate-limits), [OpenRouter current key](https://openrouter.ai/docs/api/api-reference/api-keys/get-current-key), and [Bedrock Mantle quotas](https://docs.aws.amazon.com/bedrock/latest/userguide/quotas-mantle.html). These describe provider scopes and signal formats, not defaults the router may assume for a specific customer key. Ollama concurrency and queue depth are operator/runtime settings, not hosted token quotas. No adapter may manufacture a numeric capacity when the active account does not provide one. The user-facing Traffic rollup is only service-process activity;
heterogeneous model/account headroom must not be summed into a misleading total.
OpenRouter's daily free-model count is enforced only for `:free` routes, based
on the live account endpoint response.

## E0 — audit, baseline and contract

**Work**

- Match the deployed version/source to the current tree before drawing a causal
  conclusion about the exact implementation.
- Inventory provider calls for ordinary execution, intent resolution, planning,
  verification, compaction, reflection, workers, flows, evals, gateway and desktop.
  Include background discovery/probing: their costs retain their own ownership
  and still participate in shared provider capacity admission.
- Audit rate-limit normalization, headers, error codes, cache-tier accounting,
  request estimates, token usage, tool durations, approval waits and final outcome.
- Trace why this request began with a large working set and how it grew. Record
  metadata and causal references; keep content out of logs.
- Create a deterministic, content-free fixture that reproduces the incident's
  TPM constraints, context growth and retry pattern. It must not reproduce keys,
  private account identifiers or unrelated conversation text.
- Specify ownership, reservation, settlement, uncertainty, deadline and completion
  contracts. Map them onto existing structs and consumers before adding types.
- Establish a diverse baseline: conversation, factual lookup, writing, document
  creation, data analysis, code changes, verification and operation. Include
  ambiguous requests, long-history references and mixed deliverables.

**Exit gate**: the consumer inventory has an owner and accounting path for every
dispatch; all E0 decisions are explicit; the fixture distinguishes rate waiting,
tool waiting and model work. Baseline measures joint quality and resource use.

### Provider-dispatch consumer inventory (2026-10-02)

This inventory is based on the current source call graph, not on the deployed
instance. “Capacity admitted” means the consumer obtains the shared
`vak-llm::RequestAdmission`/`RateLimitGate` reservation before making the HTTP
call; it does not imply cross-process enforcement or durable root-work
accounting.

| Consumer | Owner and dispatch path | Capacity admission and accounting | Remaining boundary |
|---|---|---|---|
| Main agent turns, intent resolution, tools/worker turns, retry ladder | `vak-agent` through the Core turn dispatcher and `Provider::stream`; per-attempt `WorkReceipt` is attached to the owning session | `RateLimitGate` model demand, account concurrency/cooldown, adapter observations and usage settlement; route aliases use provider capacity identity | Process-local only; durable root budget and cross-process reservation are open |
| Flow planning | `vak-flow/src/planner.rs` | `RequestAdmission` around planner stream; returned usage settles the reservation | No shared E1 root account; E2 process boundary remains |
| Flow node execution | `vak-flow/src/exec.rs` creates an Agent for the node; execution is owned by the flow run | Inherits Agent dispatch admission and receipts | Root flow aggregate accounting/restart reconstruction are not E1-complete |
| Core measured-capacity probes and cache measurements | `vak-core/src/lib.rs` background probe and cache paths | `RequestAdmission` with bounded per-call timeout; usage and admission pass through the same provider gate | Probe lifecycle/attempt throttle is process-local; probe cost lacks a durable root RunId account |
| Context classification, compaction, reflection | `vak-core/src/lib.rs` auxiliary calls | `RequestAdmission`; bounded timeout; known pre-dispatch refusals release reservations and returned usage settles | Root turn accounting does not yet aggregate all auxiliary work; cross-process gate remains |
| Provider model discovery and model-context/capability metadata | `vak-llm/src/models.rs` invoked by Core discovery/status paths | Each page/request uses account concurrency/cooldown admission and settles one request; Anthropic capability lookup follows same gate | Metadata calls are not all attached to the initiating session's root resource account |
| OpenRouter key/credit status refresh | `vak-llm/src/provider_status.rs` | Account request gate; single-flight refresh; observed account budget updates the shared process gate | Refresh ownership/cache is process-local; key endpoints do not supply all upstream/model constraints |
| OpenAI audio transcription/speech and Realtime | `vak-llm/src/openai.rs`, `openai_realtime.rs` | Model/account quota reservations and account dispatch slot; response/handshake observations update gates | Auxiliary usage is not yet aggregated into a durable root account; Realtime usage may be absent |
| Gemini audio, Live and Interactions | `vak-llm/src/google_live.rs` | Account/model admission and typed quota failure observations; request settlements where response usage exists | Live successful usage and project limits may be unpublished; configured project identity is required to share across keys |
| Eval runs | `vak-eval/src/runner.rs` creates an Agent for each case; the public live-model path accepts any `Provider` | Agent dispatch admission applies to both the deterministic scripted provider and injected live providers | Eval suite/root accounting and aggregate budgets are not durable E1 accounts |
| Ollama local inference | `vak-llm/src/ollama.rs` | Shared account/model concurrency admission; overload is surfaced | No hosted TPM/day quota is published; host resource utilization is outside provider token accounting |

This closes the source-level inventory for current direct `Provider::stream`
consumers found by repository search. It does not prove that deployed binaries
match this tree, account for calls hidden behind a future adapter, or satisfy
E1 durable accounting. New provider dispatch call sites must be added here and
must use the shared adapter admission path or document a separately bounded
owner.

## E1 — shared accounting and exact settlement

**Work**

- Extend the existing spend/admission seam with a root work resource account.
  Children receive a share of the parent allowance; parent and child execution
  consume one aggregate account, with atomic reservations under concurrency.
- Track provider dispatches, measured tokens by tier, estimated/settled money
  when priced, elapsed work time and recoveries. Retain step receipts and attach
  them to their root ownership. Do not count receipt usage and assistant-message
  usage twice.
- Give each admission reservation a stable identity. Settle or release that exact
  reservation once on success, definitive rejection, cancellation or uncertainty.
  Unknown charge is recorded separately from known usage and estimated exposure.
- Preserve operator day and session/commitment caps as independent enclosing
  limits. Dollar pricing may be unknown; token/time/dispatch admission must still
  work, and unknown dollars must stay unknown.
- Restore accounting on restart from durable evidence. No new message, worker,
  fallback, compaction or approval can silently reset accumulated consumption.
- Remove superseded reservation/accounting paths in the same change.

**Likely files**: `vak-core/src/finops.rs`, Core spend-gate ownership,
`vak-agent` spend trait and dispatch paths, `vak-llm/src/work.rs`, session receipt
types/projections, worker/flow/planner/eval consumers identified in E0.

**Exit gate**: concurrent children cannot overspend a shared allowance; every
reservation has exactly one terminal disposition; failures with missing usage
remain explicit; restart/retry cannot reset consumption; all inventoried calls
participate or have a documented, separately bounded owner.

## E2 — coordinate provider capacity and recovery

Depends on E1.

**Work**

- Normalize published provider rate-limit observations and retry timing into
  provider-independent typed hints in VAK-owned HTTP adapters. Preserve error
  codes; distinguish exhausted quota from retryable throttling. Handle supported
  duration/date forms without an English-message keyword policy in the agent loop.
- Add shared account capacity admission before dispatch. Combine request demand,
  observed remaining capacity/reset time and uncertain estimates. Queue work
  cancelably when capacity will refill; do not issue paid model calls to poll it.
- Carry cooldown/admission state across confirmed aliases of one account.
  Walking the admitted ladder must not evade a shared capacity limit. An eligible
  independent leg still follows existing route and model-identity policy.
- If request demand exceeds the known refill-window capacity, waiting alone is
  not recovery. Return a typed decision for a logged context replan or an eligible
  alternate; never replay the same oversized request indefinitely.
- Preserve the distinction between informed transience and blind failures in
  invariant 7. A provider-capacity scheduler is not a replacement circuit breaker.
- Derive attempt watchdogs from admitted remaining time and observed latency,
  within policy bounds; nested retries cannot reset the root deadline. Separate
  service work, provider queueing and human approval waits in accounting.

**Exit gate**: a modeled TPM window results in coordinated waiting and useful
completion rather than repeated rejection; parallel sessions and protocol aliases
do not race one shared capacity bucket; unknown-limit behavior remains bounded;
cancel/revocation interrupts waits; quota exhaustion is not transient endurance.

## E3 — context justified by work, with intact recall

Can follow E1 independently of E2; both must land before E4's combined gate.

**Work**

- Extend `vak-context` planning to use the request's required references,
  constraints and evidence as well as the capacity profile. Model capacity is an
  upper bound, not a target to fill.
- Prefer the smallest context that preserves the required dependencies, with
  uncertainty-driven recall when relevance is unclear. Measure coverage of named
  references and constraints; no domain-specific source-count or word-count rule.
- Audit tool-result ingestion into the current evidence/digest path. Preserve full
  material, provenance and expansion handles, using registered structured or
  content-aware representations rather than raw truncation.
- Preserve whole-turn fidelity, within-turn frozen request planning and stable
  cache prefixes. Any explicit replan/handoff is logged and reproducible; do not
  mutate an exact retry invisibly. A redesign of open-turn representation requires
  an explicit doc 68 decision and its own no-loss gate.
- Keep durable memory separate from the model's temporary working set. References
  excluded from a request remain recallable through existing trash-aware paths.
  Budget decisions never delete history, evidence or remembered preferences.

**Likely files**: `vak-context/src/planner.rs`, capacity/assembly, session turn
index and evidence/digest/recall paths, Core context posture, agent tool-result
recording and explicit replan transitions.

**Exit gate**: exact reference/constraint recall is retained in long histories;
unrelated historical material does not fill a large model window; evidence
expansion restores the original material; no-cut and request-reconstruction
invariants pass; request reduction does not lower outcome quality.

## E4 — progress, completion reserve and truthful recovery

Depends on E1–E3.

**Work**

- Consume existing `OutcomeSpec`, requirements and scoped evidence in continuation
  decisions. Progress can be a supported claim, artifact revision, check result,
  resolved dependency or sufficient conversational response. Tool success alone
  does not prove semantic completion, and prose length is not progress.
- Make expensive continuations state the unmet requirement and intended evidence
  change. Repeated output/failed admission does not refresh progress accounting.
  Runtime evidence validates the proposal; the model cannot grant itself budget.
- Reserve an adaptive allocation for required synthesis, verification and delivery
  within the root budget before allocating more exploration. Small turns need no
  mandatory planner or verifier model call.
- Use measured estimates to allocate additional work within admitted policy.
  Begin with observable evidence transitions and conservative uncertainty; evaluate
  more speculative predictions of value before they can control termination.
- Preserve quality requirements across routes and recovery. On exhausted resources,
  deliver supported partial findings if useful, mark unmet requirements explicitly,
  preserve the obligation and progress, and reserve completion for satisfied work.
- Resume only through existing user/commitment/task authority, rechecking policy,
  permissions, freshness and uncertain effects. Reuse completed work rather than
  repeat it. Introduce no automatic new recurring job.

**Exit gate**: stalled work changes strategy or holds without spending indefinitely;
valid creative/conversational answers need no tool proof; complete verdicts retain
all must-requirements; partial delivery is useful and truthful; resumption preserves
evidence and does not duplicate effects or create unauthorized background work.

## E5 — shared surfaces, evaluation and rollout

Depends on E4.

**Work**

- Project one consistent outcome/wait state to CLI, terminal, gateway, desktop and
  browser. Plain language explains a rate-limit wait or held work and retained
  progress. Technical disclosure shows recorded tokens, estimates, missing usage,
  root/child attribution, attempt counts and reasons without exposing secrets.
- Add root usage to existing snapshots and projections. Live reconnect/resync is
  observational and never redispatches work.
- Run deterministic fault and concurrency coverage plus blinded quality evaluation
  across the E0 workload set. Include unrelated-history growth, cache misses,
  small/large contexts, missing pricing, provider outages, TPM throttling,
  malformed results, parallel children, revocation, cancellation and restart.
- Set release gates against the measured baseline before rollout: retained quality
  and recall, lower avoidable attempts/context, correct completion semantics and
  accountable latency. Do not accept a token reduction alone as improvement.
- Keep production observations passive during evaluation. Live paid runs require
  an explicit spend allowance. Deploy only through the normal release and AWS
  managed-install process after the implementation is reviewed.

**Exit gate**: general workloads meet the joint gate and all execution/surface
consumers are accounted for. Update affected design docs, AGENTS.md invariants,
release notes and plan status; remove superseded code in the same changes.

## Change boundaries and verification

Each stage should be a reviewable change with its named acceptance tests and doc
updates. Keep existing policy authority, sandbox/broker boundaries, route contracts
and append-only ledgers. New runtime durable files, if unavoidable after E0,
must use canonical paths and be declared in `vak_core::state::REGISTRY`.

Before implementation, refresh file locations and the dispatch consumer inventory.
Before any commit, run the repository-required formatting, lint, workspace tests,
version and documentation-path checks. Frontend changes also require matching
bundles and the applicable visual acceptance process. This planning change makes
no AWS configuration, scheduling, version or deployment changes.

## Progress log

- 2026-09-30: Added a process-shared, cancelable cooldown keyed by opaque
  provider-capacity identity; OpenAI Chat Completions and Responses adapters
  share the key when configured with the same endpoint and credential. The
  Responses adapter now carries structured retry timing from streamed failures
  and the response header. The agent waits before spend admission and dispatch,
  shares typed rate-limit outcomes across aliased routes, and clears learned
  cooldown only after a successful probe. This is a bounded E2 foundation, not
  the E2 exit gate: it does not yet coordinate independent processes, model the
  provider's full refill capacity, or cover every non-agent provider consumer.
- Existing code inspection confirmed that long tool results already retain full
  evidence and are windowed in the open turn; closed turns project schema-aware
  digests with recall handles. No context truncation change is justified by the
  incident evidence so far. E0 still needs deployed-source matching, the full
  provider-consumer inventory, the content-free fixture and diverse baseline.
- 2026-10-02: Began the provider-capacity follow-on: account-scoped in-process
  admission now bounds concurrent dispatches and the OpenAI, Anthropic and
  Gemini adapters capture supported short-window observations. The Traffic
  endpoint and user/Admin hints report activity without account identifiers.
  This remains an E2 foundation, not a quota scheduler: model-specific demand
  reservation, provider-project identity across multiple keys, daily/spend
  usage APIs, persistent/shared-process authority, and non-agent consumers are
  still open. Traffic never adds incompatible provider/model headroom values.
- 2026-10-02: Extended agent admission with per-account/model estimated token
  and request reservations against fresh short-window observations; settlement
  uses returned usage, while uncertain dispatched reservations remain
  conservative through their observed reset. Corrected local usage deltas to
  avoid double-counting the provider's reported remaining headroom. Traffic
  now reports observed model windows rather than implying account-level
  coverage. Daily request observations now constrain admission through their
  provider reset; daily token values remain observation-only until usage is
  trustworthy for that window. OpenAI audio transcription/speech join shared
  admission and capture normal rate headers. OpenAI Realtime and Google Live
  voice helpers now join account/model admission; Gemini HTTP and websocket
  credentials are sent in headers rather than query strings. OpenRouter's
  `/key` sample refreshes through the shared provider hook before admission,
  throttled to one call per account per minute, and feeds daily-request
  admission for `:free` routes.
  Admission now returns a typed, non-retryable quota-exhausted outcome if an
  observed window is spent and the provider supplied no reset, instead of
  polling forever or amplifying retries. Observations expire after five
  minutes without a new provider sample, and uncertain reservations expire
  after fifteen minutes so abandoned dispatches cannot hold capacity forever.
  This is still partial E2 work: live Gemini project-limit baseline discovery,
  cross-process refresh coordination,
  and source-derived evidence fixtures remain open.
- 2026-10-02: Gemini `QuotaFailure` responses now contribute observed per-model
  short-window request/token exhaustion and daily request/token exhaustion,
  including the documented Pacific-midnight daily reset. The scheduler uses
  the returned quota metric/id/value only; active RPM/TPM/RPD/TPD limits are
  still not queried from an account control plane. Operators can now set
  `[providers.google].project_id` to share learned state across keys in that
  project; with the setting unset, capacity remains key-scoped.
- 2026-10-02: OpenAI's project-token headers now feed an account-scoped token
  reservation, so model routes on the same configured credential/endpoint
  share that published project limit. Provider cache identity now includes
  non-secret options such as explicit Google project scope. The Anthropic and Bedrock inventory now
  distinguishes their finer quota dimensions from the generic values Vak can
  currently observe and enforce. Account reservations activate only when a
  fresh account-scoped quota observation exists; OpenRouter's daily free-model
  count remains restricted to `:free` routes.
- 2026-10-02: Quota reservations now carry separate input and output demand.
  Anthropic's input/output token headers use independent counters and reset
  times; Gemini's documented input TPM/TPD exhaustion feeds the input bucket.
  OpenAI request and token reset headers now remain independent. Short-window
  resets preserve active reservations that still consume daily capacity.
  Bedrock Mantle's separate counters remain unenforced until the endpoint
  returns suitable evidence; Service Quotas is not queried by the runtime.
- 2026-10-02: Daily quota observations now outlive the five-minute short-window
  sample expiry and remain enforceable through their provider reset, with a
  24-hour stale cap when no reset is available. Daily reset no longer erases
  in-flight reservations. E2 remains open for cross-process coordination,
  context-replan handling for requests larger than known windows, root-deadline
  aware admission, and the source-derived parallel-load evidence fixture.
- 2026-10-02: Bedrock Mantle admission now follows its documented input
  reservation rule (`estimated input + max output`) and separately reserves
  max output. Settlement uses actual non-cached input plus actual output for
  the input bucket and actual output for the output bucket. This prevents the
  generic combined-token estimate from misrepresenting Mantle if a future or
  configured endpoint supplies separate quota evidence; Mantle's numeric
  limits remain unknown because Vak does not query Service Quotas and the
  endpoint does not document quota-remaining headers.
- 2026-10-02: When observed per-model or account capacity cannot admit a
  request (including a request larger than the known window), admission now
  skips that route and continues the live turn ladder. Cancellation and other
  admission errors still return directly; provider execution retry and
  endurance semantics are unchanged. A quota denial remains non-retryable by
  itself, but an eligible alternate route may satisfy the same turn.
- 2026-10-02: Replaced the account concurrency gate's 25 ms polling loop with
  a shared, cancelable FIFO semaphore (eight active dispatches per account
  identity). Traffic continues to report active and queued counts from that
  same gate. This is process-local coordination; it does not claim a global
  limit across independent service processes or hosts.
- 2026-10-02: Provider Retry-After parsing now accepts both RFC 9110
  delay-seconds and HTTP-date values across OpenAI-compatible, Anthropic,
  Gemini HTTP and Gemini Live responses. Retryable 503/529 overloads preserve
  that delay as typed data, feed the shared account cooldown, and remain
  distinct from blind failures for breaker policy. Responses with no usable
  guidance keep the existing bounded exponential fallback.
- 2026-10-02: Model quota waiters now wake when a reservation settles, is
  released, or expires instead of sleeping until the provider's next window
  reset. This lets parallel turns reuse headroom freed by an actual response
  or a pre-dispatch refusal, while retaining cancel-aware waits and reset-based
  refill behavior. FIFO ticket cleanup is cancellation-safe: dropping a
  cancelled waiter removes its ticket and wakes the next caller. Known local
  Context/quota refusals in reflection, classification, compaction, flow, and
  Core probes release reservations; post-dispatch timeouts remain conservative.
- 2026-10-02: Added `RequestAdmission` for provider calls whose caller does not
  already own Agent dispatch admission. Flow planning, Core capacity probes and
  cache measurements, classification, compaction and reflection now refresh
  published capacity, reserve input/output demand against shared account/model
  gates, join account concurrency, and settle on returned usage. Flow carries
  the configured route separately from the wire-adapter name, preserving
  OpenRouter free-route and Bedrock Mantle semantics. Uncertain dispatched
  failures retain conservative reservations. All direct `Provider::stream`
  call sites found in flow/Core are now covered. `acquire_with_timeout` starts
  the caller's per-request budget before status refresh and capacity queueing,
  then keeps the same cancellation deadline through provider dispatch. The
  background probes, classification, flow planning, compaction and reflection
  use their existing per-call limits for this whole path. A shared root-run
  deadline across retries/children and broader E0 fixtures remain open.
- 2026-10-02: Provider/model capacity observations now carry a dispatch-order
  sequence through OpenAI Chat/Responses, Anthropic, Gemini, OpenAI audio and
  Realtime, Gemini audio/Live, and OpenRouter key refresh. A delayed response
  from an older request can no longer overwrite a newer remaining-capacity
  sample or clear the newer sample's local usage delta. This ordering is
  process-local; independent processes remain uncoordinated by design.
- 2026-10-02: Gemini `QuotaFailure` mapping now distinguishes explicit input,
  output, and ambiguous token metrics. Google documents standard TPM as input
  tokens, while TPD is model-dependent; ambiguous daily token quotas therefore
  constrain combined demand instead of being treated as input-only. Quota
  identity remains project-scoped only when an operator configures the project
  id, because the provider adapter has no authorized project discovery surface.
- 2026-10-02: Auxiliary OpenAI and Gemini audio calls now reserve model quota
  before taking an active account dispatch slot, so quota queueing does not
  inflate active Traffic. A failed/cancelled slot admission releases the
  pre-dispatch model reservation. OpenAI Realtime normalizes its websocket
  endpoint to the HTTP API base and shares the OpenAI account/model identity;
  OpenRouter refresh and the OpenAI protocol adapters use the same opaque key
  helper. Account dispatch queue counts also use a cancellation-safe ticket,
  so dropping a queued task cannot leave a stale waiting count or pin an idle
  account gate in the process registry.
- 2026-10-02: Model/account quota reservations now take FIFO tickets. When
  headroom becomes available, only the oldest live waiter can reserve it;
  cancellation or an admission error removes its ticket and wakes the next
  waiter. This avoids starvation when many same-key requests queue together.
- 2026-10-02: Daily token, input-token, output-token, and request observations
  now retain independent reset deadlines. A request-count reset can no longer
  refill token capacity early (or vice versa); OpenAI day-token and day-request
  headers, Gemini metric-specific Pacific resets, and OpenRouter daily free
  request limits feed their matching buckets.
- 2026-10-02: Anthropic Messages now learns its authenticated organization ID
  from response headers, stores only a one-way fingerprint, and reuses the
  resulting account identity for model admission and 429/529 cooldown sharing
  across API keys after each key's first authenticated response. Credential to
  organization associations stay in a bounded 1,024-entry in-process LRU, so an
  idle but returning key does not lose the learned alias. The Admin
  Rate Limits API remains unqueried: it requires `org:admin` credentials and
  publishes configured ceilings, not current remaining headroom; workspace
  overrides and model-group aliases remain explicit gaps.
- 2026-10-02: Demand larger than any observed short-window or daily token
  bucket now returns a context-replan error that names each exceeded bucket
  and its estimated demand and observed limit. It does not wait for a reset
  that cannot make the request fit, and remains outside transient retry.
- 2026-10-02: Traffic reports `limited` when a fresh provider observation
  shows a short-window or daily quota has no remaining headroom after settled
  local estimates and active reservations, even when that response supplied
  no Retry-After cooldown. The rollup still omits account and model identities.
- 2026-10-02: Traffic and admission now honor each observed bucket's own reset
  deadline. At refill, local usage resets and remaining capacity returns to a
  published limit, or becomes unknown when no limit was published; an expired
  zero sample can no longer keep a route permanently limited.
- 2026-10-02: Live model listing, model-context lookup, and Anthropic capability
  discovery now use the same account request and concurrency/cooldown gate as
  inference. Every received metadata response settles one request; pagination
  admits each page separately. OpenRouter `/key` and `/credits` refresh/inspect
  requests use the account concurrency/cooldown gate too. OpenRouter's
  free-model daily request counter stays exclusive to `:free` inference routes.
  Gemini discovery credentials now use `x-goog-api-key` headers rather than
  query strings, and discovery responses preserve `Retry-After` in typed
  throttling/overload errors.
- 2026-10-02: Added `vak-llm/tests/fixtures/provider_capacity_growth.json` and
  a deterministic virtual-time regression that separates tool wait and model
  work, consumes an observed short TPM window, queues through refill, observes
  `Retry-After`, and returns a context-replan error when a grown request cannot
  fit. The fixture is synthetic and content-free; E0 still needs sanitized
  counters matched to the captured production incident before it can serve as
  the incident-specific reproducer.
- 2026-10-02: Added adapter-level capacity evidence checks for OpenAI's model,
  project and daily headers; Anthropic's separate input, output and request
  windows; Gemini's typed daily input-token quota failure and unknown-success
  behavior; and OpenRouter's free-model daily request bucket. These tests lock
  only values actually published by the adapters and explicitly leave absent
  limit dimensions unknown. The existing `vak-llm` unit and integration suite
  passes (140 unit tests plus adapter integration tests; two live Gemini smoke
  tests remain ignored). This does not close the production-source E0 fixture,
  cross-process authority, durable root accounting, or diverse workload gates.
- 2026-10-02: Explicit provider quota codes now map to `QuotaExhausted` instead
  of generic retryable 429s for OpenAI-compatible endpoints and metadata calls;
  Gemini structured daily `QuotaFailure` records do the same for GenerateContent,
  discovery and Live calls. Ordinary rate-limit codes remain retryable. OpenAI
  Realtime WebSocket handshake failures now retain HTTP status, capacity headers,
  Retry-After and the shared typed error mapping instead of becoming blind
  network errors. The normalized quota error deliberately omits provider quota
  identifiers from user-visible text; the capacity observation carries only
  numeric bucket evidence. Adapter tests cover the terminal-vs-throttle split.
