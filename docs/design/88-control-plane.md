# 88 — The control plane

Status: proposal, requested by the maintainer on 2026-10-11, being built.
C0 done (2026-10-11: blast radius re-scanned, §12 decided). C1 done
(2026-10-11): `crates/vak-control` holds the decision types and the loop
records, as `EntryPayload::Decision`, the decisions it already makes:
admission, each step's frame as asked, every call's verdict (a refusal
names what refused it), each correction, the accepted answer and the
ending. C2 done (2026-10-11): `ChatRequest.settings` (reasoning and
sampling) replaces `temperature`, `think` and `effort`; each adapter
declares its reach, a model's own report (Ollama `/api/show`
capabilities, Anthropic's model capabilities) narrows it, and a request
the provider refuses as invalid is sent once more without its optional
settings, the refusal learned only when that is taken (the wording-based
`rejects_effort` is removed). Every answered dispatch records what was
applied and whether the response reasoned (`DispatchAttempt.applied`,
`reasoned`), and the receipt what was asked (`WorkReceipt.asked`).
`GET /runs/{id}/decisions` and the admin Run detail's "How it went" show a
turn's decisions; the doctor reports the latest dispatch's settings.
Later stages are not built. Builds on docs/design/86-model-plans-runtime-checks.md (the model
plans, the runtime checks) and extends invariant 33 (the runtime evaluates
satisfaction; the model never does) from satisfaction to every decision.
The start prompt and working order for the session that builds it are in
docs/plans/handover-2026-10-11-control-plane.md.

## 1. Why

Seven real turns on 2026-10-11 (run ids in
docs/plans/handover-2026-10-11.md) each failed for a different reason, and
every reason was the same shape: something was left to the model or to a
default, and the runtime tried to repair the result afterwards.

- The model reasoned "no tool calls needed, I'll create a stats card, a
  callout card and a steps card", then wrote Markdown. Nothing framed the
  final step, so nothing held its form.
- A correction was read as the person speaking; the model planned three
  more times and invented a tool.
- Three separate limits ended turns with words the model did not write.
- Reasoning depth is set in one case only, from a word-list reading.
  Temperature is never set; top-p and top-k are not in the request type.
  The runtime does not know whether a model reasons, whether that can be
  turned off, or whether its output can be constrained, and an adapter that
  lacks a switch ignores it silently.

Measured on main at `9c16054ba`: decisions are made in at least seven
places (the intent reader, the agent loop, the stop policy, the plan check,
the repair code, the outcome evaluator in `vak-core`, the projection).
Control messages are written from 4 files (12 sites in the 9,787-line agent
loop, 9 in the projection) over 17 control kinds, each as free text.
`vak-intent` holds 34 word-list constants. There is no one place that
answers "why did this turn go the way it did".

## 2. The rule

**The model proposes; the runtime decides; every decision is a typed
record.**

- The model proposes three things: a plan, a call, an answer. It chooses
  their content. It never chooses the frame it works in, and it is never
  the decider at a decision point.
- The runtime decides at a closed set of decision points, from recorded
  facts only: receipts, typed results, measured capabilities, configuration,
  and what a person said through a control channel. Never from the words of
  a request, the wording of an error, or a number tuned to pass a case.
- A person can overrule any decision. Authority still only narrows
  (invariant 32); the control plane grants nothing.
- No free text carries a decision. A correction, a refusal and an ending
  are typed values; their wording for the model is rendered in one place.

## 3. Decision points

A closed enum. Adding a point is a code change, on purpose.

| Point | Decides | From | Output type |
|---|---|---|---|
| `AdmitTurn` | whether the turn starts, under which Agent, route and limits | admission facts (exists today) | `Admission` |
| `FrameStep` | the step's contract: reasoning, sampling, tools offered, output form | the step's purpose, the model's capabilities, configuration | `StepContract` |
| `AcceptCall` | read, validate, authorise, or refuse a proposed call | the tool's schema, permission, the turn's state | `CallVerdict` |
| `AcceptAnswer` | whether a proposed answer ends the turn | the plan, receipts, the answer's form | `AnswerVerdict` |
| `Correct` | what the model is told, and what changes in the frame | the verdict that failed, what was already tried | `Correction` |
| `EndTurn` | how the turn ends and what is reported | verdicts, the operator's turn limit, control state | `Ending` |
| `Promote` | whether a proposed change to Vak itself is offered for review | measurements | `Proposal` (added at C7) |

A step's **purpose** is a fact the runtime already has, never a reading of
the request: `Plan`, `Act`, `Answer`, `Repair`, and the side purposes that
`WorkPurpose` already names (classify, compact, judge, summarise).

## 4. The step contract

`FrameStep` returns, for every dispatch:

- **Reasoning**: off, or a depth. A repair step and a side call need none.
- **Sampling**: temperature, top-p, top-k, each set or left to the model.
- **Tools offered**: the subset this purpose needs. An `Answer` step is not
  offered lookup tools; a `Repair` step is offered the one tool under
  repair.
- **Output form**: free, one required tool, or a schema. The `Answer`
  step's form is the answer document of doc 87: the model writes one
  structured answer whose parts are blocks, so there is no choice between
  "tool call" and "prose" to get wrong.

Values come from configuration with shipped defaults for each purpose, not
from constants in code, and they are applied through what the model can
actually do:

**Model capabilities are discovered and recorded**, beside the capacity
profile: whether the model reasons, whether that can be turned off or
graded, which sampling settings it accepts, which constrained-output forms
it supports. An adapter declares what it can pass through; what a provider
reports is used; what it does not report is probed once at bind time.
Nothing names a model, provider or tool in the loop.

**The receipt records what was asked and what was applied.** "Asked for no
reasoning, the provider reasoned anyway" is then a visible fact and a
measurement, not a mystery.

Where a model cannot honour part of a contract, the contract degrades to
what it can honour and the receipt says so. The runtime never pretends.

## 5. Corrections

One ladder, used at every point, replacing the six ad-hoc ones:

1. **Tell once, with the evidence**: a typed `Correction` naming the
   verdict, the facts, and what would satisfy it. It is signed as the
   runtime's.
2. **Change the frame**: withdraw the tool, narrow the tools offered,
   constrain the output form, lower or raise reasoning, or move to the next
   route leg. The next step's contract is different, so the same failure is
   not asked for again.
3. **End with what exists**: the turn ends with the model's own work and a
   typed `Ending` that says what is missing. No ending replaces the model's
   answer with other words, and nothing produced is discarded.

No count decides a rung. The operator's turn limit is the only number that
bounds a run.

## 6. Records

Each decision is an audit-only ledger entry (a new entry type, additive):
the point, the decider (runtime, person, provider), the evidence ids, the
choice, and for `FrameStep` the contract asked and applied. It is not
model-visible unless a `Correction` is rendered from it, and that rendering
is the one model-visible record (invariant 1).

A turn's decisions read in order are its explanation. The admin Run detail
and `vak runs` show them.

## 6a. Scale

Agents, workers and sub-agents run by the hundreds or thousands, so every
decision point is a pure function over values the caller already holds: no
lock, no I/O, no allocation beyond its output, safe to call from any task
at once. What costs is recording, and that is one ledger append under the
session's own lock, never a lock shared across sessions. The capability
store is the one shared structure; it is read once per dispatch and
written only when a provider's answer teaches something.

## 7. Crates

All typed, all pure where they decide; the agent loop becomes the executor
that gathers facts, calls them, and carries out what they return.

- `vak-control` (new): the decision points, `StepContract`,
  `ModelCapabilities`, `Correction`, `Ending`, the ladder. No I/O, no
  provider, no session handle; unit-testable without a model.
- `vak-judge` (new): receipts to verdicts. The plan check, the outcome and
  completion evaluation now in `vak-core`, the commitment satisfaction
  lattice's evaluation, and call verdicts move here. One vocabulary of
  evidence strength.
- `vak-eval` (extended): measures real ledgers as well as fixtures: how
  often a contract was honoured, corrections per turn, refusals per tool,
  steps and seconds per answer, by model and route.
- Evolution stays under governed self-evolution
  (docs/design/43-governed-self-evolution.md): `Promote` turns measurements
  into proposals a person approves. Nothing changes itself.
- `vak-llm`: capability declaration per adapter, sampling fields on the
  request, constrained output per provider, applied settings on the
  receipt.
- `vak-agent`: loses its inline gates, nudges and thresholds to the two new
  crates.
- `vak-intent`: keeps what must not depend on a model (risk words that
  raise the approval floor, explicit commands, surface facts) and loses
  every reading that drives behaviour.

## 8. What is removed

In the same change that replaces each (invariant 30):

- Behaviour driven by the fixed reader: reasoning depth from a "direct
  reply" reading, domains that load tools, the guessed outcome
  requirements.
- `ToolErrorKind::classify`'s reading of error text. A tool error becomes a
  typed value at its source (`ToolOutput::error` takes a kind).
- `DOOM_LOOP_THRESHOLD`, `READ_REPEAT_LIMIT`, `WORKERS_GATE_BLOCKS`, and the
  stop policy's wording checks.
- The 17 free-text control messages, replaced by `Correction` rendered in
  one place.
- The block tools as separate calls, once the answer form exists (doc 87
  stage 2 then finishes on top of it).

Bounds that protect against hostile input (parser limits in `vak-ooxml`,
`vak-pdf`, `vak-intake`, sandbox and transport caps) are not behaviour
thresholds and stay.

## 9. Blast radius

To be re-scanned at the start of each stage. Counts at `9c16054ba`,
re-scanned at C0 on `139754d7c`: control kinds 15 (`ControlKind`), written
or matched at 23 sites in `vak-agent/src/lib.rs`, 21 in
`vak-server/src/projection.rs`, 4 in `vak-server/src/lib.rs` and 4 in
`vak-session/src/turns.rs`; `ToolErrorKind::classify` read at
`vak-agent/src/lib.rs` (2 sites) and `vak-core/src/lib.rs` (1);
word-list constants 28 in `vak-intent` (`signals.rs` 23); `ChatRequest`
has `temperature` and `think` but no top-p or top-k; the per-turn effort is
set at `vak-core/src/lib.rs` (`cfg.effort`) from the engagement.

- `crates/vak-agent/src/lib.rs` (9,787 lines): the loop's gates and its 12
  control-message sites.
- `crates/vak-agent/src/stop_policy.rs`, `plan_check.rs`.
- `crates/vak-core/src/lib.rs`: the outcome evaluation and the per-turn
  effort setting; `crates/vak-core/src/intent.rs`.
- `crates/vak-intent`: the reader, `control.rs` (the control vocabulary,
  mirrored in the client and checked by
  `vak-server/tests/control_vocabulary_sync.rs`).
- `crates/vak-llm`: every adapter, `ChatRequest`, `WorkReceipt`.
- `crates/vak-tools`: `ToolErrorKind`, `ToolOutput`.
- `crates/vak-server/src/projection.rs` (9 control sites) and the client's
  `structured.ts` lists.
- `crates/vak-session`: one new entry type; `docs/reference/records.md`.
- `crates/vak-context`: the tail and the frozen tools array now follow the
  step contract, so the byte-stable prefix rule (invariant 36) needs a
  stated answer for a step that offers fewer tools.
- AGENTS.md invariants 7, 32, 33 and 36, and docs 01, 03, 15, 47, 68, 86,
  87.

## 10. What this buys

- **Solid**: one place decides, from facts; a model that reasons its way
  elsewhere still answers in the form the step asked for.
- **Fast and efficient**: no reasoning where none is needed, fewer tools in
  a request, fewer correction steps, a stable frame for cache hits.
- **Self-healing**: a failure changes the next step's frame instead of
  repeating the request.
- **Evolving**: every refusal and correction is counted, and the counts
  become reviewed proposals.

## 11. Stages

Each ships whole with its tests, its docs and the removal of what it
replaces, and is accepted by real turns on the configured route read back
from the ledger.

- **C0**: scan and confirm the blast radius; decide the open questions in
  §12 with the maintainer.
- **C1**: `vak-control` types and the decision record entry; the loop
  records decisions it already makes, with no behaviour change.
- **C2**: model capabilities discovered and recorded; sampling and
  reasoning on the request; applied settings on the receipt.
- **C3**: `FrameStep` and the step contract for every purpose, from
  configuration.
- **C4**: `vak-judge`; verdicts for calls and answers; typed tool errors.
- **C5**: `Correction` and the ladder; the control messages, thresholds and
  stop-policy checks removed.
- **C6**: the answer form as the `Answer` step's contract; doc 87 stage 2
  finished on it; the block tools retired.
- **C7**: `vak-eval` over real ledgers; `Promote` and the review path.

## 12. Decisions (maintainer, 2026-10-11, at C0)

1. **The fixed reader keeps safety only**: risk words that raise stakes or
   the approval floor, explicit control commands, and surface and
   attachment facts. Every other reading that drives behaviour is removed
   (reasoning depth from a "direct reply" reading, domains that load tools,
   guessed outcome requirements). No model-made reading replaces it.
2. **A model that cannot be constrained gets the same ladder**: one
   `Correction` with the evidence; the frame cannot change, so the next
   answer stands, shown as it is, with a typed `Ending` that says what the
   contract asked and what the model could honour. No special case.
3. **The turn's tools array stays byte-stable** (invariant 36). An `Answer`
   step is framed by forced tool choice or an output schema where the model
   supports either; tools are dropped from the request only for a model
   that supports neither, and the receipt records that it was done.
4. **Defaults are per purpose, overridable per route**: shipped defaults
   for each purpose live in configuration; the Shared and project layers
   and a route-keyed table may override them. Measured capabilities still
   degrade what a model cannot honour.
