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
C3a done (2026-10-11): `FrameStep` (`vak_control::frame`) frames every
step from its purpose and `StepDefaults` (shipped, then `[control.steps]`,
then `[control.routes."provider/model"]`), for the turn's own steps and
every side dispatch (plan, judge, classify, compact, summarise); the
"direct reply" reading and `DemandHint::direct_reply` are removed;
`/config/control` (user or project layer), Settings › "How the model
thinks" and the admin's Models panel edit it live. C3b done
(2026-10-11): tools are loaded by their own declaration
(`Tool::always_loaded`, cards, no declared domain, or loaded earlier in
the session), never the reading; `Limits.required_domains`, `DomainSet`,
the act-to-domain table, the misread ledger and the tier-2/3 model reading
(`[intent] escalate`, `classify_model`, `max_classify_usd`) are removed.
C4 done (2026-10-11): `crates/vak-judge` turns receipts into verdicts
(the verdict on a proposed call from gathered facts, a turn's unrecovered
failures, the plan check, and the outcome, requirement and completion
evaluation moved out of `vak-intent` and the agent crate). A tool error is
typed at its source: `ToolOutput::error` takes a `vak_control::Fault`
(shape, target, tool, refused, cancelled), the worker protocol carries it,
the loop records each failed call's fault as `CallEffect::Failed`, and
`ToolErrorKind::classify` and `never_ran` are removed. A name that is no
tool this turn offers is `Refusal::Unknown` at `AcceptCall`. The workspace
is checkpointed when the first call that declares it may change the
workspace is accepted (`Tool::changes_workspace`, `AgentConfig::before_effect`),
not from a reading of the request. C5 done (2026-10-11):
`vak_control::correct` holds the typed `Finding`s, the one `Ladder` (told
once, then the frame changes or the answer stands; a check that passes can
fail anew) and `render`, the one place a correction's wording is written;
every correction in the loop goes through `Agent::correct`. Removed: the
freshness, grounding and topic-mismatch checks with their word lists and
the three endings that replaced the model's answer; the stop gate's
`max_blocks` budget, its "does the answer quote the error" reading and the
worker gate's count; `DOOM_LOOP_THRESHOLD` and `READ_REPEAT_LIMIT`, replaced
by an observed state (a call that returned the same result again is
refused, `Refusal::Repeated`); `AgentConfig::retrieval_check` and
`observation_check`. A model step's `request_timeout` bounds silence between
stream events, never the whole step (§5a). C6 is in progress
(2026-10-11). Its first slice is built: a step's required tool
(`OutputForm::Tool`) reaches the wire. `ChatRequest::must_call` is named in
each provider's own tool choice (Anthropic `tool_choice`, OpenAI chat and
Responses `tool_choice`, Google `toolConfig`), a wire with no such switch
is offered only that tool (native Ollama; §12.3), and the receipt says
which (`Applied::forced`); `ModelCapabilities::tool_choice` is each wire's
reach. The answer document is built (docs/design/87-composed-answers.md
§2): the model answers with one loop-owned `answer` tool of blocks, made
at run time from the admitted card tools (`Tool::card_kinds`); a card tool
is no longer offered or callable by its own name (`Refusal::Unknown`), and
the per-card loop paths it made unreachable are removed (routing a card
call to its owner, the repeated-card acknowledgement, the duplicate-card
fence check). A valid `answer` call is recorded as `AcceptCall`, its cards
as `PresentationSource::Answer`, and ends the turn with no further model
step. The plan check runs on that path and its reframe rung requires
`answer` of one step (`Finding::AnswerRequired`), once, after which what
is left stands; a `text` block answering a deliverable the plan lists as a
card is refused. Seen live 2026-10-11 through the web app on the local
route (`qwen3.5:4b-mlx`): a three-part request shown as four cards from one
`answer` call in three steps (`run_01a12ae9-3cb3-7cf2-8db2-8644b6c96212`),
a plain question answered as prose (`run_01a12aea-9061-7b01-b24a-b56ec1a0ef32`),
and a document request whose draft was delivered for Review
(`run_01a12aeb-8eb3-7fb1-821f-3352e65e8945`); and once on the hosted
Responses route (`gpt-6-luna`): a plan of three cards, each placed in its
section (`run_01a12aed-56a0-7f00-bfc5-ddea72daa7c5`), and a one-sentence
answer as prose (`run_01a12aed-c2ce-76a2-abfc-ceac50b4bed8`). The faults
those runs found in Vak's contract are fixed: the `answer` block schema
had untyped union fields, a `blocks` sent as text skipped the one argument
reader and its refusal did not say why, a field of another kind was not
named, the repair directive and the unknown-tool message listed the card
tools as callable, a plan form naming a card's kind was refused, and the
plan check's correction after an answer added a second copy of its cards
(a later `answer` now replaces the turn's earlier one). On 2026-10-12 the
nested `answer` call was retired (maintainer's choice, after measurement,
docs/audits/cards-in-answers-2026-10-11.md): on the same captured requests
it came back as broken JSON in 7 of 16 steps, separate card calls in at
most one. Each card is again its own call of the card tool that owns its
kind, the loop's own display channel (`show_card`), recorded as
`PresentationSource::ToolCall` with the deliverable its `deliverable` names;
the outline and layout stay. The reframe rung requires the card tool of
the kind the plan named (`Finding::CardRequired`, `Deliverable::kind`), and
a card naming no planned deliverable is refused while the plan lists
cards. The items carried below are not built.

### Open in C6 (found while building its first slice)

- The hosted wires' named tool choice is written from each provider's
  published API and covered by a unit test of the request body; it has not
  been sent to a hosted provider from this tree. Anthropic refuses a named
  tool choice while extended thinking is on, and this adapter leaves
  thinking to the model's default, so that pairing needs a live check.
- A wire that refuses a named tool choice is not learned yet: the
  capability is each adapter's fixed reach (`Unknown` for OpenAI-compatible
  endpoints, which sends it).
- Tool arguments that fail as broken JSON, on every tool and not only
  cards, are designed for in docs/design/89-constrained-tool-arguments.md
  (proposal): a wire that can enforce a tool's schema is asked to, and an
  unreadable call is regenerated under its schema before the model is
  told.
- One answer document versus per-block tools: decided for per-block tools
  on 2026-10-12 (docs/audits/cards-in-answers-2026-10-11.md). The first
  measurement used toy schemas with the same fields both ways and did not
  see the fault; replaying Vak's real captured requests on the dev route
  (`qwen3.5:4b-mlx`, eight runs each of two answer steps) gave good
  `answer` calls in 5 and 2 of 8, one `answer` block per call (the union
  schema) in 0 and 1, and separate card calls in 8 and 6. Other routes are
  to be measured the same way, with the replay of a captured request.

- Seen in the C6 live runs. Fixed since: every card was squeezed into half
  of a phone screen when an answer had no closing prose (that path skipped
  the one answer composer); a card header's title group did not take the
  row's width; an audio card overflowed a narrow card. The client's
  presentation harness now checks that all registered types fit a 260, 293
  and 358 px card. Fixed too: a card block's refusal named the card tool to
  call again, and `office_apply`'s missing-anchor error pointed only at
  `doc_read`, not at the anchors an earlier result for the draft named.
  A card citing a URL nothing retrieved is refused.
  Still open, with no fault found in Vak's contract: asked for a new Word
  file, a model answered at its first step that it could not make one
  (`run_01a12aeb-04df-7200-b6f3-d57e0389aed9`). `office_apply` was offered
  but deferred, its catalogue line read "Create or edit Word, Excel,
  PowerPoint and PDF files with typed ops", and the model's own thinking
  said the tool needs a source or a digest, which its schema says to leave
  out for a new file. The next try, asked in other words, made the draft.

- The same question from the live side (2026-10-11,
  docs/design/15-reliability.md "Staged repair"): in full turns on the dev
  route the `blocks` JSON was malformed in 9 of 11 `answer` calls, each a
  single slip that voided the whole document. Settled by the item above.

### Carried into later stages (found while building C0–C5)

- The commitment satisfaction lattice (`vak-commit/src/satisfy.rs`) is
  already pure and stays in `vak-commit` until the judge takes a
  `vak-session` dependency for criteria (C7, with `vak-eval`).
- C6 (done, maintainer's choice 2026-10-11): a commitment opens and work
  runs managed only when asked explicitly (`/until-done`, `/goal replace`);
  `Posture::managed`, `Posture::open_commitment` and
  `Reading::may_open_commitment` are removed. The horizon reading still
  sets the context profile (`derive_context`) and an unattended gate's
  fallback (`Authority::gate_fallback`); both are readings to replace.
- C6 (done): the time constants in §5a (probe, channel wait, command,
  script, hook and person waits) are `[control.limits]`, edited in Settings
  and the admin console beside their shipped defaults; §5a lists the three
  things still open there.
- C6 (done): the outcome requirements guessed from the request's words are
  removed: the write-verb, file-extension and Office-term lists behind
  `saved_file_target`, `expects_artifact` and `permits_authored_text_file`,
  the gate that refused an unrequested text file, and the artifact delivery
  shape they set. A file the model's plan lists is observed when a call
  whose tool declares a delivered file, an artifact or a workspace change
  succeeded.
- C6 (done): the loop's receipts are read from declarations, not tool
  names. A call ran a command when its tool declares one
  (`Tool::command`); it changed something when its tool declares a file
  it writes, a delivered file or a workspace change; a code change is a
  declared write to a code path; and a call looked something up unless
  its tool says it does not (`Tool::looks_up`: a card tool, `find_tools`
  and `plan` do not). Core reads a plan from `Tool::planned`. The
  `write | edit | patch | remember | propose_skill` and `edit | write |
  patch` lists, the `bash` name checks in the receipts and in the
  draft-copy screen, `looks_something_up` and `is_substantive_command`
  (a word list that discounted commands whose text said "verified" or
  "dummy") are removed: a command that ran is a command that ran. A
  delegated task and a remembered fact now count as a change when the
  call is made, by their workspace claim, as a write always did.
  Done on 2026-10-12: card membership reads `presents_cards`; delivered
  files and a continued save read `delivered_file` and `file_access`
  (`vak_tools::tool_for_call`); the permission engine, its rules, hook
  matchers, remembered approvals and `auto_approve` read each tool's
  declared reach and rule subject (`Tool::permission`, docs/design/
  08-permissions.md "Declared reach"), and the engine's name tables,
  vak-core's `NETWORK_TOOLS` and `presenting_tool_names` are gone.
  Also done the same day: an argument alias is its tool's own
  `fill_defaults` (`vak_tools::arguments::alias`), which wrappers now
  forward; a batch runs in order when two calls touch one path by
  `Tool::file_access`; the audit command and a scheduled script use the
  tool that declares the `Command` reach, called in its declared argument
  names (`vak_tools::command_call`); the sandbox and document prompt
  contracts appear while an admitted tool declares them
  (`Tool::prompt_contract`); a task copy and a channel block restrict by
  reach (`PermissionEngine::restrict_reach`); the flow tool's read-only
  tools are those that read paths. No tool name decides behaviour outside
  the loop's own tools (`plan`, `recall`, `task`, `work`, the card calls).

Builds on docs/design/86-model-plans-runtime-checks.md (the model
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

A frame that is changed is changed in what the next request carries: a
withdrawn tool leaves the offered tools as well as the admitted set. Found
live on 2026-10-11: it did not, so the same refusal was given eight times
(docs/design/15-reliability.md).

## 5a. Time

Requested by the maintainer on 2026-10-11: a model, a tool or a person may
be fast or slow, so nothing is ended because a number of seconds picked in
code went by. A wait ends on an observed state:

- **the work finished**: the stream closed, the process exited, the person
  answered;
- **someone stopped it**: cancellation, a revoked capability, a fenced
  process;
- **it stopped making progress**: no event arrived on a stream, or a
  holder's liveness lapsed. Silence is measured between events, never over
  the whole of a piece of work, so a slow answer that is still arriving is
  never cut;
- **the operator's limit**: a limit a person configured, shown in Settings
  with its shipped default, recorded in the `Ending` when it is what ended
  the work.

The operator's limits are `[control.limits]`, in seconds, each with its
shipped default (`vak_control::TimeLimits`): `command_secs` 120 (a command
whose call names no limit), `command_max_secs` 600 (the longest limit a
command's call may name; a call that names more runs with this), `script_secs` 120 (a scheduled script),
`hook_secs` 10 (a hook that sets none of its own), `approval_secs` 900 and
`question_secs` 900 (waiting for a person; silence still means no,
invariant 15), `reply_secs` 240 (how long a channel's `wait` request is
held before it is told the turn is still running; the turn goes on) and
`probe_secs` 30 (one request of the background capacity probe). They are
layered like the step settings, read live (`Core::time_limits`), edited
through `/config/control` (which returns the limits in force beside the
shipped ones), the client's Settings › Time limits and the admin console's
Models panel. A limit of zero is read as unset. An untrusted project sets
none of them (`control.limits` is a privileged key).

A tool says which of its arguments bounds one call
(`Tool::time_limit_argument`; `bash` names `timeout_ms`), and the loop fills
that argument from `command_secs` when the call leaves it out, before the
call is ledgered, so the record shows the limit it ran with. The loop
names no tool to do this. Seen live on the local route
(`run_01a12b7f-2142-7062-85f7-b5c2879cf4f2`): with `command_secs` set to
45 in Settings, the turn's command call is recorded as
`{"command":"uname -sm","timeout_ms":45000}` and the turn ends answered.

Audit of time constants in code, first taken at C4 (`b213cc1f6`), by what
they bound:

| Kind | Where | State |
|---|---|---|
| A whole model dispatch | `AgentConfig::request_timeout` wrapped the entire streamed step | done at C5: it bounds silence between stream events (`within` in the agent loop) |
| A whole model call, fixed | `REFLECTION_CALL_TIMEOUT` 120 s (was defined three times) and `HEARTBEAT_TURN_TIMEOUT` 120 s | removed at C6: reflection is bounded by the operator's `request_timeout_secs` inside the call, and a check-in runs until the next is due (`[heartbeat] interval_secs`) |
| A probe request, a channel's wait | `PROBE_REQUEST_TIMEOUT` 30 s, gateway `WAIT_TIMEOUT` 240 s | done at C6: `probe_secs`, `reply_secs` |
| A command or script | bash `DEFAULT_TIMEOUT_MS` 120 s, `SCRIPT_TIMEOUT_MS` 120 s, hooks `DEFAULT_TIMEOUT_MS` 10 s | done at C6: `command_secs`, `script_secs`, `hook_secs` |
| Waiting for a person | `QUESTION_TIMEOUT` 900 s, `HTTP_APPROVAL_TIMEOUT` 900 s | done at C6: `question_secs`, `approval_secs` |
| Protection against a hostile or dead peer | discovery 15 s, MCP list 10 s, webfetch and browse totals, `VERIFY_DEADLINE`, delivery worker 5 s, credential probe | stay: they bound input Vak does not control, like the parser limits in §8 |
| Freshness of a cache or a liveness beat | profile and discovery TTLs, reconcile intervals, `LIVENESS_RENEW_SECS` | stay: they schedule work and end nothing |

The `vak flow` and `vak plan` commands and the flow tool build their
executor with the operator's `request_timeout_secs` (`Core::request_timeout`,
the one reading the turn path uses; 2026-10-12), no longer a fixed 600 s.
The loop also clamps a limit a call did name to `command_max_secs`, so the
bound on one command is the operator's setting, not a constant in `bash`.
An operator's limit ends a call or a wait, never a turn by itself (the
turn's own limit is `End::StepLimit`): a command stopped at its limit fails
with `Fault::Limit`, which the call's `failed` decision records, so the
trail says it was the limit and not the tool (2026-10-12).

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
- `ToolErrorKind::classify`'s reading of error text (removed at C4). A tool
  error is a typed value at its source (`ToolOutput::error` takes a `Fault`).
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
- `crates/vak-agent/src/stop_policy.rs`; the plan check is `crates/vak-judge/src/plan.rs`.
- `crates/vak-core/src/lib.rs`: the outcome evaluation and the per-turn
  effort setting; `crates/vak-core/src/intent.rs`.
- `crates/vak-intent`: the reader, `control.rs` (the control vocabulary,
  mirrored in the client and checked by
  `vak-server/tests/control_vocabulary_sync.rs`).
- `crates/vak-llm`: every adapter, `ChatRequest`, `WorkReceipt`.
- `crates/vak-tools`: `ToolOutput` and its `Fault`.
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
