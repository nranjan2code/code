# 68 — Context engine: measured capacity, turn working set, recallable evidence
Status: implemented. Crate: `vak-context` (`capacity`, `planner`,
`assemble`) over the ledger in `vak-session`. Native provider compaction
(§12) is not implemented; the hosted probe ladder is opt-in via
`[probe] hosted`.

## Why this design

What a model can *use* is a property of that model, not of the window it
declares. Measured on `gemma4:e2b-mlx` (same system prompt, same 30 tools,
real session messages): it called a tool 6/6 with ≤13k tokens of context
and 0/6 at ≥18k, against a declared 131k window. A frontier model on the
same ledger keeps dozens of turns whole. The only way to serve both from
one session is to measure each model, keep the session itself complete,
and compute what each request carries from those two things alone:

- the **ledger** is the one rich original — every directive, step, tool
  exchange, thinking block, receipt, presentation and card, append-only,
  never rewritten or trimmed;
- the **bound model's `CapacityProfile`** says how much of it this model
  can actually use;
- every request is a **projection** of the ledger chosen against that
  profile, and whatever comes back is written to the ledger in full.

Nothing a weaker model could not fit is ever lost to a stronger one bound
later, and nothing model-visible is cut by a character count.

## Principles

1. **Measure, never assume.** Every number that shapes a request — window,
   usable horizon, tokens per char, prefill cost, cache behaviour — is
   discovered from the provider and the model, recorded in the ledger, and
   revised from every turn's usage. No constant in this design is a cap; the
   only constants are search ladders and confidence thresholds.
2. **No blind cuts.** Nothing model-visible is truncated by a character count.
   Content leaves the working set only by (a) being replaced with a
   content-aware digest that the model can expand with a tool call, or (b)
   being summarised into a compaction packet that names what it covers. Both
   paths are reversible from the ledger.
3. **The turn is the unit.** A turn is one user directive plus every
   assistant step and tool exchange until the final answer. The working set
   is whole turns; a turn is never split.
4. **Stable prefix, moving tail.** Bytes that do not change across turns
   (system prompt, tool schemas) come first and stay byte-identical. Bytes
   that change every turn (time, intent, stance, thread, nudges) come last.
5. **One source per fact.** A tool schema, a directive, an intent note appears
   in the request exactly once.
6. **Fail narrow, recover wide.** When a decision cannot be made confidently,
   send less and give the model a way to ask for more; never send everything.

## Components

The engine is its own crate, `vak-context` (`capacity`, `planner`,
`assemble`): pure functions over the ledger, no I/O. `vak-session` owns the
ledger it reads (`TurnIndex`, `TurnCard`, range-keyed packets,
`derive_with_plan`, `recall`); `vak-agent` and `vak-core` own the I/O around
it (the probe, the summariser call, receipts and feedback written back).

```
                  ┌──────────────────┐
  provider ──────▶│ CapacityProbe    │──▶ CapacityProfile (ledger, per model)
  usage/latency ─▶│ (bind + feedback)│
                  └──────────────────┘
                            │ horizon, tok/char, output reserve
                            ▼
  ledger chain ──▶ TurnIndex ──▶ WorkingSetPlanner ──▶ RequestAssembler ──▶ ChatRequest
                      │               │  ▲                  │
                      │               │  │ relevance         │ stable prefix / tail
                      ▼               ▼  │                   ▼
                 EvidenceStore   CompactionPacket      ToolSurface (core + index + find_tools)
                 (full results,   (summary + ids)
                  recall tool)
```

### 1. CapacityProbe → CapacityProfile

A `CapacityProfile` is a ledger-recorded, per-`(provider, model, quantisation)`
record. It is the *only* input to budgeting.

```rust
pub struct CapacityProfile {
    pub declared_window: u64,          // provider metadata (existing model_context)
    pub verified_window: Option<u64>,  // largest prompt the provider accepted
    pub instruction_horizon: Horizon,  // largest prompt at which the model still followed a tool instruction
    pub tokens_per_char: Ewma,         // provider usage.input_tokens / chars sent
    pub prefill_tps: Ewma,             // input tokens / (first-token latency), cache-miss turns only
    pub cache: CacheBehaviour,         // None | PrefixStable | ProviderReported
    pub output_reserve: u64,           // provider max output, or observed max completion
    pub provenance: ProbeProvenance,   // when, which ladder rungs, which signals
}
pub struct Horizon { pub tokens: u64, pub confidence: f64, pub last_confirmed: Instant }
```

**Bind-time read, background probe.** `capacity_profile_for` never runs the
ladder on a turn's own critical path: it synchronously returns the newer (by
`provenance.probed_at`) of the ledger-recorded profile and the in-process
cache, or a metadata-only profile (`declared_window`/`output_reserve` alone,
`confidence` low) when neither exists yet, and the request is assembled
against whatever that is. `maybe_start_capacity_probe` runs the ladder as a
background task, started only once a turn has finished (docs/design/68 §1:
"only while that model is idle") and only when the leg is eligible (local,
or hosted with `[probe] hosted = "full"`), the cached profile is missing,
stale, or `needs_reprobe`, and no probe for that key is already in flight
(single-flight; a real turn about to use the same key cancels any
in-progress background probe for it rather than compete with it for
compute). The task updates the process cache on completion; the session
ledger catches up at the next bind, since the ledger belongs to the run that
called it, not to the detached probe task. The ladder itself:

1. Metadata rung: `model_context()` (Ollama `/api/show` incl.
   `num_ctx`, `/v1/models`, Anthropic/OpenAI catalogues). Sets
   `declared_window`, `output_reserve`.
2. Horizon ladder: send synthetic histories of geometrically increasing
   size (4k, 8k, 16k, 32k, 64k, … up to `declared_window × 0.9`), shaped
   like the turns the model will really see — user question, assistant
   tool call, digest-shaped result with an `[evidence:…]` tag, short answer
   — ending with *one* instruction that requires a tool call (`probe_ack`,
   no side effects). A rung's verdict is the majority of three identical
   samples (a sampling model is a coin flip per completion; the second and
   third samples hit the prefix cache, so they cost decode time only).
   Binary-search the boundary between "followed" and "did not". The largest
   passing rung is `instruction_horizon`. A 400/413 on any rung sets
   `verified_window`. Each sample is one request with `max_tokens` ≈ 32.
   Measured: single samples over inert prose gave 7k, 14k and 48k for one
   model within an hour; majority-of-three over turn-shaped filler gave 64k
   six times out of six.
3. Cache rung: two identical requests back to back; if the provider reports
   cached tokens (`prompt_tokens_details.cached_tokens`,
   `cache_read_input_tokens`) or the second is ≥5× faster, `cache =
   PrefixStable`.

The probe is a ledger `Activity` (never model-visible) so the decision it
produced can be reconstructed. It is skipped when a fresh profile exists
(TTL 24h for local models, 7d for hosted; a changed digest of the model
metadata invalidates it).

Cost control for hosted models: the horizon ladder runs on the cheapest
available model of the same family only when the operator has opted in
(`probe.hosted = full`); otherwise hosted profiles start from
`instruction_horizon = declared_window` with `confidence = 0.3` and are
tightened by feedback. Local models are always eligible for the background
probe: the probe is free apart from time, and time is exactly what it saves.

**Feedback.** Every turn's receipt updates the profile:

- `tokens_per_char` ← `usage.prompt_tokens() / chars_sent` — every prompt
  token the provider billed, cache tiers included, never `input_tokens`
  alone: `chars_sent` counts the whole request whether or not it hit the
  cache, so the numerator has to match, or a growing cache-hit rate would
  collapse the ratio toward zero (a full cache hit reports `input_tokens ==
  0`) for a request that was not remotely empty (EWMA α=0.3).
- `prefill_tps` ← on cache-miss turns only.
- Horizon tightening: a turn in which the assembled request exceeded
  `instruction_horizon.tokens × 0.8` **and** the model failed an explicit
  instruction it was given (the runtime already classifies this: required
  card not emitted, required tool not called, stop-policy block) lowers the
  horizon to that request size with confidence 0.6 and schedules a re-probe.
- Horizon loosening: only a re-probe can raise it. Feedback never widens.

### 2. TurnIndex

Built from `chain_to_root()` once per request. A `Turn` is:

```rust
pub struct Turn {
    pub id: EntryId,                 // the user directive entry
    pub directive: Message,          // user text (+images)
    pub steps: Vec<Step>,            // assistant messages and their tool exchanges, in order
    pub final_answer: Option<Message>,
    pub tokens: u64,                 // measured via tokens_per_char, or exact if a receipt covers it
    pub evidence: Vec<EvidenceId>,   // tool results captured for this turn
    pub summary: Option<String>,     // filled when the turn leaves the working set
}
```

Control messages (nudges, intent notes, thread, work contract) are not turns;
they are tail material owned by the assembler. Compaction packets become
`Turn::Summary` entries covering a range of turn ids.

### 3. EvidenceStore and the `recall` tool

Every tool result is stored in full under an `EvidenceId` (the existing
receipt/`EvidenceRef::ToolResult` identity). In the request the model sees:

- **Current turn:** every result verbatim up to 30,000 characters. A longer
  result is carried as a *window* (`vak_tools::window`): whole lines from its
  start and end, and one line naming the omitted range, its size in
  characters, and the `recall({ id, range })` call that returns it. Inside a
  window a line over 2,000 characters shows its start and says how many
  characters it continues for; a result that fits is never windowed, however
  long its lines. The `ToolResult` block logs exactly what the request
  carried, and an `EvidenceBody` entry beside it holds the whole result, so
  `recall`, the closed-turn digests and `SessionLog::evidence` read what the
  tool returned. A `recall` result over the limit is windowed the same way
  under its own id.
- **Working-set turns before the current one:** a *digest*, produced
  deterministically from the result's shape, never by character count:
  - JSON: top-level keys, array lengths, and the first element of each array
    rendered in full.
  - Search/crawl results: every title and URL, no bodies.
  - Text/markdown: headings, first paragraph, line and byte count.
  - Command output: exit code, first and last 10 lines.
  - Errors: verbatim (they are short and the model must see them).
  Each digest ends with `[evidence:<id> — <n> chars; call recall to expand]`.
- **Turns outside the working set:** nothing; the compaction packet lists
  their evidence ids with one-line descriptions.

`recall({ id, range? })` is a core tool that returns the full result (or a
line range). The summariser requests (compaction, handoff) show results the
same way a closed turn does — as their digest with the evidence id — never
cut to a character count. Recalled content is a current-turn result, so it is verbatim for
the rest of that turn and digested afterwards: every digest is reversible by
the model.

### 4. WorkingSetPlanner

Inputs: `CapacityProfile`, `TurnIndex`, the incoming directive, the assembled
prefix size (measured, see §6), the tail size.

```
budget      = horizon.tokens
            − prefix_tokens              (system + tool surface, measured)
            − tail_tokens                (time, intent, thread, nudges)
            − output_reserve
            − current_turn_reserve       (= max observed current-turn size for this model, EWMA)
# never split a turn: every cost check is turn-whole
value(turn) = max(recency, relevance, anaphora)
              recency   = 1 / (1 + age), only for a relevant or referenced turn
              relevance = subject-token search of the directive / best score
              anaphora  = 1.0 for the preceding turn when the directive refers back
cost each rendered Full record and Card line with the current profile;
never trust a stored estimate that omits tool arguments
full_history_budget = min(budget, 12,000 tokens)
fill eligible turns at Full in descending value while their combined cost ≤
full_history_budget, skipping one that does not fit for a cheaper one further
down; the rest as cards
newest → oldest while they fit; the overflow collapses into one packet.
summary     = compaction packet over the packet range, or the stored one
```

Properties:

- On a small model with a 13k horizon and a 12k prefix, the working set is
  the current turn only, and the model is told so (the summary says what it
  is missing and how to `recall`). That is the correct outcome for that model
  and it is reached by measurement, not by a constant.
- On a frontier model the same session keeps dozens of turns whole.
- Compaction is incremental, never an overflow emergency: the packet is
  refreshed whenever a turn leaves the working set, so the summariser call
  is amortised and the request never overshoots. Turns evicted past the
  working set move into the packet in fixed-size batches
  (`PACKET_BATCH_TURNS`, 8), never one at a time, so the packet's boundary
  holds steady across several turns instead of growing every request.
- **A packet is a cache, never a boundary.** A `Compaction` entry is keyed
  by the inclusive turn range it summarises (`first_turn_id..=last_turn_id`)
  and records the model whose plan asked for it. The projection renders a
  stored packet only when the current plan's `packet_range` is exactly that
  range; a plan that wants those turns at `Full` or `Card` gets them from
  the ledger, whatever packets exist. So the ledger stays the one rich
  original, and every request is a function of `(ledger, bound model's
  profile)` alone: a session that ran on a 5B model and packeted its first
  25 turns re-projects those 25 turns whole the moment a frontier model is
  bound, and re-projects the stored packet — with no second summariser
  call — when the small model is bound again. Growing a packet (the same
  first turn, a later last turn) seeds the summariser with the longest
  stored packet over that first turn and adds only the newly covered cards.
  The one true boundary is the reset-with-handoff entry (`reset_all`,
  docs/design/42), the rescue for a profile with no usable horizon: turns
  behind it are marked `behind_reset` in the `TurnIndex`, never planned and
  never listed as covered, though `recall({ turn })` still reaches them.
- If a request is still rejected as over-length (provider 400 / context
  error), that is a `CapacityProfile` contradiction: `verified_window` is set
  to the rejected size, the working set is re-planned, and the turn retries
  once. It is never a permanent error.

### 5. Tool surface

Every admitted tool is callable; the surface decides only which are *loaded*
this turn (`vak_core::capability::surface`, docs/design/41-capability-registry.md
"Tool surface"). Each tool declares itself on the `Tool` trait.

- **Loaded**: tools that declare `always_loaded` (`read`, `glob`, `grep`,
  `find_tools`, `recall`, `skill`, `mcp`, `session_search`, `commitments`);
  tools whose `serves` meets the reading's required domains; undeclared
  tools; and the card tools the request itself reads as. An uncertain reading
  arrives as the explicit *orientation floor* (`Engagement::orienting`,
  docs/design/47-commitment-kernel.md), so it loads the filesystem and memory
  tools. `DomainSet::All` loads everything and is what a *disabled* kernel or
  `slice_capabilities = false` produces — never a fallback for uncertainty.
- **Catalogue** ("More tools"), one line per admitted tool that is not
  always loaded (`name — first sentence`), sorted, in the stable prefix. It
  depends on the admitted set only, never on the reading, so it does not move
  when the loaded set does. No schemas.
- **`find_tools({ query })`** returns full schemas for matching deferred
  tools and promotes them to loaded definitions for the rest of the turn on
  every provider (`vak_agent::load_discovered`). Its result reports how many
  tool names are new to the current turn; repeating discovery with a different
  wording that returns only already surfaced schemas says so explicitly. The
  prompt tells the model to stop rephrasing that search and use an available
  tool or state the missing capability. The presentation-check nudge loads
  the card tool it names the same way. A deferred tool the reading did not
  predict and the model then used is the measured misread. On Anthropic the
  deferred schemas additionally ride `defer_loading: true` with the server-side
  tool search tool (§11).
- **MCP**: reached only through `mcp`, and started only by that demand
  (docs/design/41-capability-registry.md). The prompt names each admitted
  server with the tool *names* the pool last observed (none before its first
  use); `mcp` `list` with a server returns its schemas and descriptions when
  they are needed.
- **Skills**: listed once in the prompt (name and description); a body is
  loaded through `skill` on demand, never inlined.

### 6. RequestAssembler: stable prefix, moving tail

The code is authoritative for section order: `compose_tail` emits
`<turn_context>`, `<intent>`, `<stance>`, `<work_contract>`,
`<workspace_delta>`, `<conversation_thread>`, and `prefix_digest` is
SHA-256. `docs/design/83-prompt-system.md` §10 shows the current tail.

```
[system]   static layers only: identity, contract, guardrails, surface,
           skill list, MCP server names, tool catalogue.  ← byte-identical across turns
[tools]    core schemas in a fixed order, then any find_tools additions
[messages] retrieved older turns (whole)
           compaction packet (if any)
           working-set turns (whole, digested evidence)
           current turn's directive ─┬─ tail block inserted here, before the
                                      │  directive's own text:
                                      │  <turn_context> UTC instant, local
                                      │    time, timezone
                                      │  <intent> latest note only
                                      │  <stance> epistemic stance — phrased
                                      │    so it never forbids a tool the
                                      │    prompt asks for ("answer directly;
                                      │    still use emit_*_card when a card
                                      │    type fits")
                                      │  <thread> directives NOT in the
                                      │    working set verbatim
                                      │  <work_contract> if active
           current turn's later steps (tool_use/tool_result pairs, nudges) ──
           appended after the directive, unmodified, one per step
```

The prefix size is *measured*: the assembler sends it once per profile as a
usage-only request (`max_tokens: 1`) and records `prefix_tokens`; thereafter
the provider's reported input tokens minus the known message sizes keep it
calibrated. Chars/4 is retired.

A `prefix_digest` (blake3 of system + tool list) is written with every
receipt; a turn whose digest differs from the previous turn's is a cache
break and is surfaced as an `Activity` so regressions are visible.


**Placement.** `attach_tail(messages, tail, directive_index)` inserts the
tail into the turn's DIRECTIVE message specifically — resolved once by
`SessionLog::derive_with_plan_and_directive` from the turn structure a flat
`Vec<Message>` no longer carries — not "the last user message": within one
turn, every step after the first appends more messages (tool results,
control nudges) after the directive, and re-deriving "last message" each
step used to move the tail onto whichever one came last, silently changing
the shape of an already-sent, earlier message between requests — exactly
what an append-only request must never do (required for provider-specific
preserved-thinking checks, and it maximises cache hits generally). The tail
sits before any text in the directive's own content, after any leading
`tool_result` blocks, so the last thing the model reads there is the user's
own words — the directive on the first step, a runtime nudge (appended
verbatim as its own later message) on a redo. The tail never restates the
directive: measured live, an echo after a tool result read as the user
asking again and the same card was re-emitted up to nineteen times in one
turn. Also measured: with the tail appended after the directive,
`gemma4:e2b-mlx` answered the `<stance>` block ("As an analytical agent, I
can handle tasks…") instead of the question.

**Repeated cards.** An `emit_*_card` call identical to one already shown
is acknowledged as a no-op; after three consecutive all-repeat batches the
turn closes on that card as its answer instead of spending the turn budget
on acknowledgements.

**Empty steps.** A response with neither text nor a tool call (a
thinking-only completion — the model planned an action in its reasoning
channel and stopped) is not an answer; it gets one `[empty-step]` redo,
unless a card was already emitted in the run, in which case the card is the
answer. Measured live: four of six replays ended a step on "Final Plan: 1.
Use tavily_search…" with nothing executed.

### 7. Drift management

Drift is handled by runtime checks, not by a sentence asking the model to
tolerate topic changes:

- Every turn already produces an intent `Reading` and an `outcome`. The
  planner keeps the active goal (`goal_state`) and the last N directive
  readings.
- **Directive drift** (the user changed subject): the working set is
  re-ranked so the retrieved slice favours the new subject; the thread lists
  the superseded directives as "earlier, now paused". No cut, a re-weighting.
- **Stale data** (the directive asks for a value as it stands now — temporal
  deixis such as "current", "right now", "today", "latest" sets the
  `live-data` domain on the reading, but only in a request that seeks a fact
  (answer, locate, analyse) and not beside something local, a time, or the
  agent's own state: "the current price of copper" and "the live score" are
  live data, "the current directory", "refactor the current parser" and
  "what are you holding right now" are not; `live` counts only in its
  *current* sense, never as the verb "reside" ("we live in the city"),
  resolver version 6 — and no call that observes current state succeeded in
  the run): the answer or card can only repeat an earlier turn's figure. An
  observation is any call whose capability declares that it reads the world
  as it is now — a retrieval, and equally reading a file, listing a
  directory or running a command (`AgentConfig::observation_check`, built
  from `serves`): demanding a web search for "what is on the current branch"
  replaced correct answers with "I could not retrieve a current value".
  A card call is intercepted before execution and answered with a
  `[freshness-check]` error value the model repairs by retrieving first; a
  prose answer gets the same as one redo nudge. An explicit "no live data"
  is accepted. After that one repair the turn **fails closed**: a second
  card with still nothing retrieved, or a final answer that still carries
  no retrieval, ends the turn with a system-authored statement that no
  current value was retrieved, naming the last figure the conversation
  recorded and when — never a carried-over figure presented as current.
  A request to recall what the assistant reported earlier is historical
  retrieval, even when it names a temporal topic such as “the latest weather
  answer”; it must not be routed through this freshness gate.
  Measured live: the model's "repair" was a different stale card from an
  older turn. The signal sets the domain
  only — raising the evidence standard through the stance text made the
  small model deliberate in its thinking channel and emit nothing. Measured live: five of six replays of a "current weather"
  question emitted a card carrying a temperature from a previous turn.
- **Model drift** (a step ends with no tool call and its final text is a
  verbatim repeat of a prior turn's `TurnCard` narration — domain-mismatch
  detection was tried and dropped: it also fired on a legitimately related
  follow-up): the `[steering-drift]` nudge fires with the *specific*
  directive it should serve, and the event lowers horizon confidence if the
  request was near the horizon. Three consecutive model-drift events end the
  turn with the system-authored degraded outcome (same shape as tool-repair
  exhaustion, docs/design/15-reliability.md).
- **Topic mismatch** (a card's own content is unrelated to both the
  directive and this turn's own retrieval, found in real post-release use:
  asked "what is the current top news in AI", the model correctly called
  `tavily_search`, got real AI-news results back, and then wrote
  `emit_metric_card` for "Noida Weather, 28°C" — a payload copied from an
  unrelated, much older turn still sitting in context. Freshness had nothing
  to say: a retrieval genuinely had succeeded this run). The check is a word-
  overlap test between the card's serialized payload and the directive plus
  this turn's own retrieved evidence text (never the directive alone —
  evidence is far richer, so a card correctly titled from what was actually
  found, e.g. "OpenAI announces GPT-6" for a directive that only said "AI
  news", still passes on zero overlap against the directive). Deliberately
  **scoped to only fire when a retrieval has already succeeded this run**:
  a card built from the model's own reasoning or from data already in the
  directive routinely shares no vocabulary with anything (an empty chart
  skeleton, a bare numeric metric) whether it is right or wrong, and
  checking those broke a real, previously-passing test the first time this
  was tried unscoped. A gated card is intercepted before execution
  (`[topic-mismatch]`, one repair); the second strike fails closed with the
  raw evidence text quoted rather than a wrong card or nothing.

A card-only turn is a complete answer: the outcome evaluator sees the turn's
presentation entries as response content, so a model that emits a card and
ends without prose (Gemma finishes inside its thinking channel) is not
graded "no response content".

### 8. Ollama specifics (measured on 0.34.2, MLX runner)

- Over-length is HTTP 400, never truncation → §4 recovery path.
- `OLLAMA_KEEP_ALIVE` default 5m unloads the model; the desktop should pass
  `keep_alive` on each request (Ollama honours it on `/api/chat`; the
  OpenAI-compatible path ignores options, so the Ollama provider should move
  to `/api/chat` for this and for `num_ctx`).
- Prefill on this machine is ~740 tok/s; with a stable prefix a 30k-token
  session costs ~1s of prefill per turn instead of ~40s.

### 9. No first-class integrations

Vendor and topic names that entered the runtime during development as test
fixtures must not steer production behaviour. Audit of `crates/*/src`
(excluding `#[cfg(test)]` modules, whose fixture names are fine):

| Hard-coding | Where | Replacement |
|---|---|---|
| `s == "tavily"` tool-name sniffing to derive presentation signals | `vak-delivery/src/skills.rs` (`signals_from_context`) | Signals come from the capability's declared `serves`/domains (Web, LiveData → citations, sources); a tool name is never matched. |
| `WeatherApiCurrentAdapter` — a specific vendor JSON shape (`current.temp_c`) baked into the delivery pipeline | `vak-delivery/src/adapters.rs` | Delete. Result adaptation is the model's job through `emit_*_card`, or an adapter declared by the integration's own manifest; the core ships none. |
| Skill-name → domain table (`"research-and-sources" => [Web, LiveData]`, …) | `vak-core/src/capability/provider.rs` | Each skill declares `serves` in its own front-matter; the seeded skills in `seed.rs` carry it there. Undeclared stays `Undeclared`. |
| Seeded skill `research-and-sources` and its guidelines injected into every turn's system prompt | `vak-core/src/seed.rs`, prompt builder | Seeds are ordinary skills loaded through `skill`, disclosed by the tool index, never inlined by name. |
| Worked card examples in the system prompt (`"San Francisco Weather"`, "weather, telemetry") | `vak-core/src/prompts.rs` | Card catalogue stays, but examples are drawn from the card schema (`metric`: label/value/unit) rather than a topic; the catalogue moves to a skill loaded when the surface renders cards. |
| Doc comments narrating "the weather bug" / "the tavily server" | `health.rs`, `engage.rs`, `reach.rs`, `report.rs`, `provider.rs` | Rewrite in general terms (a live-data question; a configured search server). Comments are not behaviour, but they are how the next hard-coding gets justified. |

Invariant, enforced by a test in `vak-eval`: no production source under
`crates/*/src` outside `#[cfg(test)]` contains a vendor name from the
configured MCP catalogue or a topic word used as a routing key. The list of
banned tokens is the set of MCP server names in the operator's config plus
`weather`, `noida`; the test fails on any match that is not in a comment.

### 10. What each call carries: within a turn vs. follow-up turns

The two regimes want opposite treatment. Within a turn the request is a
**growing log** the model is still working from; across turns it is a
**record of what happened** the model may need to refer to.

#### Within a turn (step k → step k+1)

The request for step k+1 is the request for step k plus the assistant step
and its tool results. Nothing before the tail changes. This is what makes the
provider prefix cache hit ~100% (observed: `matched=28964/29063`).

| Content | Rule | Why |
|---|---|---|
| Current-turn `tool_use`/`tool_result` pairs | verbatim, always | the model is still using them; APIs require the pair |
| Current-turn assistant text between calls | verbatim | it is the model's own working state |
| Current-turn thinking | replayed **only if the provider requires it** (Anthropic signed thinking with tool use; OpenAI Responses reasoning items or `previous_response_id`); dropped for OpenAI-chat/Ollama/Google | required for API validity there, pure cost elsewhere |
| Nudges (stop-policy re-dispatch, repair directive, steering) | appended at the tail, never inserted earlier | keeps the prefix stable |
| `find_tools` additions | appended to the tool list, never reordered | same |
| Retries / re-dispatch after transient errors | identical bytes | cache hit on the retry |
| Oversized result mid-turn (next step would exceed the horizon) | the *oldest* current-turn result is digested, the working set is re-planned, cache loss accepted | the only case where the middle changes, and it is a replan, not a cut |

Parallel tool calls in one step execute concurrently and their results are
appended in call order.

**Side calls inside a turn** (intent tier 2/3, summariser, presentation
nudge classifier, any judge) are separate model calls and follow their own
rule: cheapest capable model, a purpose-built packet, never the turn's
history. The intent resolver gets the directive plus the thread; the
summariser gets one turn record plus the previous packet; a judge gets the
artefact it is judging. The turn's request is never reused as a side-call
prompt.

#### Follow-up turns (turn N seen from turn N+1)

Every finished turn is closed with a **TurnCard**, written to the ledger once
and never rewritten:

A turn has three layers, and the TurnCard keeps them apart:

```
evidence      raw tool results (search hits, file contents, command output)   → EvidenceStore
presentation  emit_*_card payloads: the answer as the user saw it             → Presentation ledger entries (below)
narration     the assistant's prose around the cards                          → text
```

**Presentations are ledger entries.** An emitted card is never rebuilt
from the arguments of its `emit_*_card` `tool_use` block, and
`presentations.json` holds card *definitions*, not instances: the runtime
writes a hash-linked entry at the moment a card call validates, and the
server, feedback and selection all key on it:

```rust
EntryPayload::Presentation(PresentationRecord)

pub struct PresentationRecord {
    pub turn_id: EntryId,             // the directive this card answers
    pub source: PresentationSource,   // ToolCall { tool_use_id } | Fence { message_entry_id }
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub schema_version: u32,
    pub payload: Value,               // canonical (validated, key-sorted) form
    pub payload_digest: Blake3,       // of the canonical payload
    pub derived_from: Vec<EvidenceId>,
    pub title: String,
    pub identity_digest: String,      // the schema-driven digest used in TurnCards
}
```

Consequences:

- `PresentationRef.id` is the entry id. The display channel and the
  model-visible history read the same record; nothing is rebuilt from tool
  arguments, and the tool framework's result-length limits stop mattering.
- `derive` treats `Presentation` like `Receipt`: never model-visible raw.
  The current turn sees the card through the `tool_use` input it wrote; past
  turns see it through the TurnCard or the full-record rendering, both of
  which read the entry.
- The inline-fence fallback (models without tool calling) converges on the
  same entry: a fence that parses and validates is written as
  `PresentationSource::Fence`. Duplicate detection is one rule — a second
  card in the same turn with the same `payload_digest` is not written, and
  its fence is dropped from the projection.
- Feedback and selection (`PresentationFeedbackBody`,
  `PresentationSelectionBody`) key on the entry id, so "the user dismissed
  this card" is an event about a ledger fact.
- Replay fidelity: the presentation the user saw on 12 September can be
  re-rendered from the chain months later, and the TurnCard index is
  rebuildable from entries alone.

`presentations.json` stays what it is — the definition library — and is not
part of the chain.

The answer to a card turn is the **presentation**, not the narration: a
metric card *is* the answer (108 chars in the ledger); a research card is a
derived synthesis (~4k chars) built from ~75k chars of evidence. Indexing
the assistant text would index the least informative layer.

```rust
pub struct TurnCard {
    pub turn_id: EntryId,
    pub asked: String,                 // the directive, verbatim if ≤ ~60 tokens, else first sentence + gist
    pub did: Vec<TraceLine>,           // one per non-presentation tool call: name, args digest, evidence id, result shape
    pub answered: Answer,
    pub outcome: Outcome,              // completed / degraded / cancelled / failed
    pub reading: ReadingKey,           // act, domains, entities, modalities from the intent reading
    pub tokens_full: u64,              // cost of the two-message record, measured
    pub tokens_card: u64,              // cost of this card, measured
}
pub struct Answer {
    pub presentations: Vec<PresentationRef>,  // in emit order
    pub narration: String,                    // gist of the prose; verbatim when short
}
pub struct PresentationRef {
    pub id: EntryId,                   // the Presentation ledger entry
    pub semantic_type: String,         // metric, research.synthesis, chart, …
    pub title: String,
    pub digest: String,                // schema-driven, see below
    pub derived_from: Vec<EvidenceId>, // the tool results present in the turn before this emit
}
```

**Presentation digest is schema-driven, not length-driven.** Each card
schema declares which fields are its identity: `metric` → all fields (it is
already minimal); `research.synthesis` → takeaways + source titles/URLs, not
snippets; `table` → title, columns, row count, first row; `chart` → title,
series names, point count, `accessible_summary`; `entity` → title, type,
fields; `checklist`/`timeline` → title + item labels. The digest of a metric
card is the whole card; the digest of the 3,943-char research card above is
~600 chars. The full payload is one `recall({ presentation })` away.

**Derivation is recorded.** `derived_from` links each card to the evidence
it was built from — the tool results that appeared in the turn before the
emit call. That is what lets a later turn go card → evidence without
re-searching, and it is what makes "where did that number come from?"
answerable from the ledger.

`asked`, `did`, `outcome`, `reading`, the presentation refs and both token
counts are deterministic, and so is `narration`: turn close must never
dispatch a model call of its own and block the run finishing on it (measured
live: a 3s mock provider delay showed up as a 3.02s gap between the last
streamed text and `RunFinished`). `narration` is verbatim when short (≤60
words); otherwise it is the leading sentence via a semantic-boundary
(`. ! ?`) fallback, never a character count and never a side model call. The
**index text** for a
turn is `asked` + every presentation's title, labels, values and takeaways +
`narration` + the trace tool names and query arguments. Structured card
fields index better than prose: "Noida" in an entity field or "Temperature:
29.1°C" in a metric is an exact attribute match, not a fuzzy one. BM25 at
write time, embeddings when an embedding model is configured, so discovery
is a lookup, not a scan.

**In-turn consequences.** The `emit_*_card` tool result becomes a short ack
(`{ "presentation": "<id>", "ok": true }`), never an echo of the payload:
the payload is already in the `tool_use` input and stays there verbatim for
the rest of the turn. An inline ```` ```vak ```` fence for a card that was
emitted by tool is a duplicate and is dropped from the ledger's
model-visible projection (the "no repeats" rule already governs display;
this applies it to what the model sees of its own history too).

A follow-up turn then sees past turns at one of three fidelities, chosen by
the planner, never by a constant:

| Fidelity | Shape | When |
|---|---|---|
| **Full record** | two messages — `user: directive` / `assistant: trace lines + final answer` | the last K turns, where K is however many fit after the horizon budget is met (§4); and any older turn the retrieval step selects for the current directive |
| **Card** | one line in a `<turns>` block: `#17 asked: … → did: search×2 → research.synthesis "Sensex 15 Sep" (4 takeaways) [pres:a1; ev:9f2,9f3]` | every other turn in the session |
| **Packet** | a compaction summary over a *range* of cards | when even the cards would not fit; the packet names the range so `recall` can reopen a turn |

Relevance is intent-driven: the current directive's reading (act, domains,
entities) and its text are matched against the card index; the top matches
are promoted to full record inside the 12,000-token history slice. A large
model horizon does not authorize filling the prompt with every lexical match.
A turn that shares no domain, entity, or lexical overlap with the directive
stays a card. Lexical matching finds candidates; it is not semantic proof that
the user meant a candidate. Anaphora ("do that again", "the second one")
promotes the immediately preceding turn when it fits the same slice. The model
can reopen specific records with `recall` rather than receiving an unbounded
collection of them up front.

Any turn at card or packet fidelity can be reopened by the model with
`recall({ turn: 17 })`, which returns its full record; the model sees the
evidence ids on every card so it can go one level deeper with
`recall({ id })`. Nothing is unreachable; the default is just small.

Typical shape for a 31-turn session on a small model: ~2 turns full
(~500 tokens), ~29 cards (~80 tokens each ≈ 2.3k), no packet — under 3k
tokens of history for the whole session.

The full record, when used, is the turn as recorded with thinking dropped,
every evidence result replaced by its digest and every card result by the
short ack — the real `tool_use`/`tool_result` pairs stay, because what a
later turn imitates is the assistant role's *behaviour* (search, then card),
and a prose transcript of a call teaches a small model to write calls as
text (observed live: `▸ emit_metric_card {…}` emitted as plain text). Pairs
are always complete, so the record is API-valid everywhere and byte-stable
once the turn closes:

```
user:      <directive verbatim>  (+ images)
assistant: tool_use tavily_search {"query":"…"}
user:      tool_result → digest: titles + URLs … [evidence:9f2 — 14.2k chars; call recall to expand]
assistant: tool_use emit_research_card {payload}
user:      tool_result → {"presentation":"a1","ok":true}
assistant: <narration>           the prose around the card, verbatim, minus
                                 any `vak` fence that duplicates an emitted card
```

| Content | Follow-up rule | Why |
|---|---|---|
| Directive | verbatim | it is the intent record and what references ("that", "the second one") resolve against |
| Final answer | presentation payloads verbatim (they are the answer) + narration verbatim | what the user saw; the payload is the `tool_use` input, replayed as assistant text, not as a tool call |
| Intermediate assistant narration ("Let me search…") | dropped | process, not information; on small models it is the prose pattern the model then imitates |
| Tool calls | the real `tool_use` block | the call pattern is what the next turn imitates; a prose rendering of it is imitated as prose |
| Tool results | the schema-driven digest carrying the evidence id, as the paired `tool_result`; full result via `recall` | §3 |
| Thinking | never | no provider needs it across turns; it is the largest and least useful block |
| Nudges, repair directives, intent notes, stance, thread | never | they were runtime guidance for that turn |
| Compaction packet | rendered from turn records (directive + trace + answer), not from raw exchanges | the summary is of decisions, not of tool dumps |

Compaction works in whole turns (`/compact` runs the same planner through
`plan_for_session` and writes the same range-keyed packet as the loop's
incremental compaction), and a closed turn's record never changes once
written, so the prefix cache survives across turns as well as within them.

#### Cache mechanics per provider

Append-only is necessary but not sufficient: some providers cache nothing
unless told where. `ChatRequest.cache` carries the session key and the
breakpoints `assemble::cache_breakpoints` computes (after the prefix, after
the previous turn, on the current step); each adapter renders them its own
way — Anthropic `cache_control` on at most four blocks, OpenAI
`prompt_cache_key`, OpenRouter `session_id` alongside it, nothing for
runners that cache by prefix on their own. Because the per-turn material
rides in the tail and never in the system prompt, the prefix is written
once and read on every step and every turn.

Every receipt records `cache_read`/`cached_tokens` and the assembler's
`prefix_digest`. The harness asserts, for every within-turn step,
`cached_tokens(step k+1) ≥ input_tokens(step k) − tail_tokens`, and for
every new turn `cached_tokens ≥ prefix_tokens`. A step that misses is an
`Activity` with the digest that changed, so a cache regression is visible in
the ledger rather than in the bill.

### 11. Provider matrix

Verified against provider documentation on 2026-09-19. These are *hints*
for the assembler, not assumptions the planner relies on: `CapacityProfile`
still probes cache behaviour and horizon per model, because a provider
behind OpenRouter or a proxy may not pass any of this through, and a local
runner may have none of it.

Registry: `anthropic`, `openai` (chat), `openai-responses`, `google`,
`openrouter`, `openrouter-responses`, compatible gateways, `ollama` (native
`/api/chat`), `bedrock` (Mantle, OpenAI-compat). Realtime/live voice
providers are out of scope here.

| Provider (Vak name) | Prefix caching | Breakpoints / keys | Thinking across steps | Native deferred tools | Native compaction / context editing | Assembler rules |
|---|---|---|---|---|---|---|
| **Anthropic** | explicit; order tools → system → messages; min 512 (Fable/Mythos/Opus 5), 1,024 (Sonnet 5/4.6/4.5, Opus 4.8), 2,048 (Opus 4.7), 4,096 (Haiku 4.5, Opus 4.6/4.5); writes 1.25× (5m) / 2× (1h), reads 0.1× (0.025× Fable 5.1 / Mythos 5.1); 20-block lookback per breakpoint, runs of tool_use/tool_result count as one position | up to 4 `cache_control` blocks, or top-level `cache_control` for an auto-advancing breakpoint; `max_tokens: 0` pre-warms | signed `thinking` blocks must be replayed within a turn when tools are used; Opus 4.5+/Sonnet 4.6+/Fable/Mythos keep prior-turn thinking without breaking cache, earlier models and Haiku strip it | `defer_loading: true` per tool + `tool_search_tool_regex_20251119` / `_bm25_20251119` (all 4.5+ models); deferred schemas stay out of the prefix; a deferred tool cannot carry `cache_control`; custom search may return `tool_reference` blocks | `context-management-2025-06-27`: `clear_tool_uses_20250919` (trigger/keep/clear_at_least/exclude_tools), `clear_thinking_20251015`; `compact-2026-01-12`: `compact_20260112` (min trigger 50k, default 150k; readable summary block; `pause_after_compaction`; billing in `usage.iterations`) | breakpoint after tools+system, after the last past-turn message, and on the current step's last block (moved each step); top-level `cache_control` is the simpler equivalent — use it and keep one explicit breakpoint on the prefix. Use `defer_loading` for the non-core tool set and `clear_thinking` with `keep: all` on models that preserve it. Native compaction is opt-in (§12). |
| **OpenAI chat** (`openai`) | GPT-5.6+: explicit or implicit, min 1,024, writes 1.25×, reads 0.1×; earlier models implicit only, `cached_tokens` rounded down to 128 | `prompt_cache_options: {mode: explicit\|implicit, ttl: "30m"}` + `prompt_cache_breakpoint: {mode: explicit}` on a content block, up to 4 writes; earlier models: `prompt_cache_key` (routing) and `prompt_cache_retention: "24h" \| "in_memory"` | chat completions carry no reasoning items; nothing to replay | none on chat | none on chat | send `prompt_cache_key = session id`; on GPT-5.6+ explicit breakpoints at the same three positions as Anthropic; never change tools, schemas, reasoning effort or verbosity mid-session (each resets the prefix) |
| **OpenAI Responses** (`openai-responses`) | as above | as above; `instructions` cannot carry a breakpoint, so the stable prefix must be a developer message | reasoning items must be carried: either replay them (stateless) or chain with `previous_response_id` (server-held); `configuration_update` items change effort without breaking the prefix | none (client-side `find_tools`) | `context_management: {compact_threshold}` server-side, or `POST /responses/compact`; compaction items are **opaque and encrypted** | within a turn prefer `previous_response_id` (no reasoning replay); across turns Vak's own projection with stateless input; native compaction off by default because its output cannot be audited in the ledger |
| **Google** (`google`) | implicit on Gemini 2.5+, min 2,048 (2.5) / 4,096 (3.x); reported as `usage.total_cached_tokens`; explicit `cachedContents` for long stable prefixes | none per block; explicit cache object with TTL | thinking models require **thought signatures** replayed exactly (stateless) or `store: true` + `previous_interaction_id` (Interactions API, server-managed) | none | none client-visible | keep the prefix identical; replay thought blocks within a turn; explicit cache for a prefix above the minimum when the session is long-lived; `thinking_level` chosen by intent stakes, not fixed |
| **OpenRouter** (`openrouter`, `-responses`) | pass-through: OpenAI automatic + explicit, Anthropic `cache_control`, Gemini implicit/explicit, DeepSeek/Grok/Groq/Moonshot/Z.AI automatic, Qwen explicit | Anthropic-style `cache_control` on content blocks is translated per upstream; top-level `cache_control` for auto; `session_id` pins the upstream for cache reuse; `usage.prompt_tokens_details.{cached_tokens, cache_write_tokens, cache_discount}` | depends on upstream; treat as stateless replay | none | none | always send `session_id`; place `cache_control` as for Anthropic and let OpenRouter translate; the probe decides whether hits actually occur for the routed upstream |
| **Bedrock Mantle** (`bedrock`) | OpenAI-compatible Responses models with explicit caching; Anthropic models via the Messages API with `cache_control` | `cache_control` / `prompt_cache_key` as documented by AWS | as OpenAI Responses | as upstream | as upstream | same rules as the upstream family; the probe verifies pass-through |
| **Ollama** (`ollama`, 0.34.2) | runner-side prefix cache (`matched=` in the log), independent of context; lost on unload (`keep_alive`, default 5m) | none in usage; the native `/api/chat` adapter passes `keep_alive` and `options.num_ctx` explicitly and reads prompt-eval timing into `Usage` | thinking returned but not required back | 0.34 adds "OpenAI-compatible client tool search"; semantics unverified — probe before relying on it | 0.34 adds "response compaction"; same caveat | over-length is a hard 400, mapped to `LlmError::Context` and replanned once (§4); the probe measures prefill tok/s and cache from timing since usage does not report it |
| Compatible gateway | OpenAI-compatible gateway; caching depends on the routed upstream | unknown | unknown | unknown | unknown | treat as a routed gateway without pass-through guarantees; everything comes from the probe |

**Future providers** (Groq, Mistral, DeepSeek, xAI, Moonshot, Z.AI, Qwen,
vLLM, llama.cpp, LM Studio, MLX-native) all fit one of three shapes the
assembler already handles: *explicit breakpoints* (Anthropic-like),
*implicit prefix cache* (OpenAI/Gemini/vLLM-like, where only byte-stability
matters), or *none reported* (local runners, where the probe infers cache
from timing). A new provider therefore needs a `CacheBehaviour` hint and a
`model_context` discovery rung, nothing else; the horizon is probed the same
way for all of them.

### 12. Native features: use, but stay authoritative

Provider-native compaction, tool search and context editing are faster and
cheaper than doing the same client-side. The rule for using them:

- **Use** a native feature when its effect is visible in the ledger and the
  request stays reproducible: Anthropic `defer_loading` + tool search (the
  discovered `tool_reference` blocks are in the response), `clear_thinking`,
  OpenAI `previous_response_id` within a turn, Gemini `previous_interaction_id`
  within a turn.
- **Not implemented** for a native feature whose output the ledger cannot
  audit or that would replace history server-side: OpenAI compaction items
  are encrypted; Anthropic `compact_20260112` is readable but drops
  everything before it on the server. Vak's own card-level compaction
  already covers the need portably, so there is no switch for these and no
  dead code behind one. If one is ever wired, the returned block must be
  stored as a `Compaction` ledger entry so `derive` remains the single
  source of what the model saw.
- **Never** let a native feature change what Vak sends to a different
  provider in the same ladder: the projection is computed once per turn from
  the ledger and rendered per provider; a fallback leg gets the same turns at
  the same fidelity.

## Verification

- **Probe harness** (`vak-eval`): for each configured model, print the
  measured profile and the working-set plan for a fixture session; assert the
  plan never exceeds the horizon and never splits a turn.
- **Live replay**: a ledger that once produced a prose-instead-of-card
  turn, re-assembled for `gemma4:e2b-mlx`, must produce a request ≤ the
  probed horizon; run it 6× against the live model and require a card in
  ≥5.
- **Cache test**: two consecutive turns must share a prefix digest; Ollama
  log `matched` must be ≥ prefix_tokens on the second.
- **No-cut invariant**: a property test that every tool result present in the
  ledger is either verbatim, a digest carrying its evidence id, or named in a
  compaction packet — never absent and never cut.
- **Two-model replay** (`context_engine_gate`): the fixture session planned
  under a small profile writes a packet; re-planned under a large profile,
  the packeted turns must come back at `Full` in the plan *and* in the
  projection, with no `<context_summary>`; re-planned under the small
  profile again, the stored packet must be reused with no compaction needed
  and a byte-identical projection.
- **Uniqueness invariant**: no tool name appears twice across schemas, index,
  and system prompt; no directive text appears both in a working-set turn and
  in the thread.

### Context selection repair, 2026-09-30

The installed desktop weather comparison exposed two planner defects: unrelated
closed turns were admitted by recency alone, and recorded Full costs omitted
tool arguments while Card costs measured the search index rather than the wire
line. The planner now admits Full records by subject relevance or a reference,
uses recency only within admitted context, and remeasures the actual projections
with the current profile. A minimal reading carries compact references and
only promotes an immediate follow-up reference. The append-only ledger stays
complete. These repairs do not establish million-turn scalability; the remaining
indexing and semantic-selection work is tracked in
[the repair plan](../plans/context-selection-repair.md).

Recall is an on-demand expansion path, not a mandatory step on every turn. The
system prompt directs the model to answer fresh or unrelated requests without
recalling history, and to recall only when a specific prior detail is needed.
An empty recall query is invalid and returns a corrective tool error; it must
not be dispatched as a broad history search. Focused tool tests cover this
contract. Live model behavior and million-turn scaling remain unproven; see
[the repair plan](../plans/context-selection-repair.md).
