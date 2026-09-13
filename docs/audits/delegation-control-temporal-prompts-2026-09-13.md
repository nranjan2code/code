# Delegation, control, communication, and temporal prompt audit

Status: audit findings open; no production changes. Examined revision `fb38b76a`, version 3.0.82.

This extends `docs/audits/system-prompts-2026-09-13.md` in response to the user's requirements to cover subagents, all model-call purposes, intent, commitments, stopping, planning, communication, and time. The standard remains a universal platform whose behavior adapts to the request and whose custom Agent instructions add to vak's foundation.

## How prompts actually reach each call

| Call/path | System instructions | Task/context supplied separately | Recording/inheritance |
| --- | --- | --- | --- |
| Main Agent execution | Core resolves layers and selected capabilities into `cfg.system_prompt` | `session.derive_messages()`, plus retrieved context | Per-turn system/schema binding; parent Agent identity is frozen in the session |
| Default `task` child | Parent Core cloned with `Surface::Subagent`; prompt re-resolved using session capability descriptors | Self-contained `task.prompt`; current parent request appended as alignment context | Separate child ledger stores full system text; parent transcript is not automatically copied |
| Named-role child | Pre-rendered entry from `role_prompts`, selected by `task.role` | Same child task message | Unknown roles rejected; composition defects from the primary audit still apply |
| Saved-Agent child | Default/role prompt, then saved Agent identity/personality/working style appended | Same child task message | Selected profile stored in header, but this differs from top-level Agent composition |
| Flow Agent node | `ExecutorDeps.system_prompt` copied verbatim | Rendered node prompt including dependency outputs | Separate ledger; managed flow carries Agent/conversation identity, but no per-node prompt-layer descriptors |
| Workflow planner/replanner | `PLANNER_SYSTEM`, a fixed TOML-authoring prompt | Task, tool catalogue, and replan context | No tools on the planner request; returned workflow validated before execution |
| Managed-contract author | Fixed JSON-authoring system prompt | Request supplied to contract authoring | Contract validated and appended before activation; pending assumptions can hold execution |
| Goal completion auditor | `AUDIT_SYSTEM` | Objective, criteria, transcript digest, optional workspace delta | Model verdict is distinct from deterministic commitment evidence |
| Compactor | `COMPACTION_SYSTEM` | Rendered transcript segment | Summary appended to session; original ledger retained |
| Handoff author | `HANDOFF_SYSTEM` | Objective/digest | Structured handoff used for context reset |
| Reflector | Independent reflection system prompt | Recent conversation | Proposes notes/skill; application has separate gates |
| Heartbeat | Normal background Core prompt | Watchdog task requesting findings or `nothing` | Not a special replacement system prompt |
| Bash, file, and MCP calls | No separate LLM system prompt in ordinary tool execution | Tool name and structured arguments | Broker/permission execution, followed by tool result in model context |

The regular Agent sends the system string again on each inference step, including after tool results. Provider adapters translate `ChatRequest.system`: Anthropic uses a system text block, OpenAI Responses uses `instructions`, Chat Completions uses a system message, and Google uses `systemInstruction`. Adapters do not resolve Agent settings themselves. Sources: `crates/vak-llm/src/anthropic.rs:58`, `openai_responses.rs:59`, `openai.rs:185`, and `google.rs:145`.

Task children cannot recursively delegate through the normal Core path: the task tool is constructed from the tool list before adding itself. Children exclude flow, and read-only children receive a reduced tool list. Task execution supports steering/cancellation and returns child output to the parent. This audit used a scripted provider, not actual delegated LLM work.

## Additional findings

### D01 — P1: child instructions and recorded Agent ownership can disagree

At `crates/vak-core/src/lib.rs:5035`, the child Core retains the parent's Agent identity when composing its system prompt. But `crates/vak-agent/src/task.rs:548` creates the header with the selected saved profile or a literal default Vak identity; `TaskDeps` does not carry the parent Agent identity. Without `task.agent`, a custom Agent's child therefore records Vak while potentially being instructed to act as the custom Agent. Its conversation is also constructed as a new local `subagent` context rather than inheriting the parent's authorized audience context.

**Demonstrated:** a scripted parent with `research-agent` identity delegated without selecting another Agent. The child request retained Research Agent instructions; the child header recorded `agent.id = vak`. Parent linkage existed and layer descriptors were empty. This is an ownership/provenance defect; no cross-audience disclosure was tested.

**Repair:** admit the child with the parent's resolved Agent/ConversationKey/audience unless an explicit authorized selection chooses another Agent. Identity must travel as typed context, never be reconstructed from prose.

### D02 — P1: saved-Agent delegation uses a separate prompt and trust path

`task.rs:105` reads Shared and project `.vak/agents.json` directly. Unlike the server's `agents::effective(core)`, this loader has no project-trust input or check before applying the project definition. It also supports selecting an Agent by parsing the natural-language prefix `Use my Agent “…”` in the model-authored child prompt (`task.rs:489`). Selection then appends a second identity block to the parent/role system prompt instead of replacing the identity through one admitted composition path.

The result can contain “You are Agent A” followed by “Selected Agent identity … Agent B,” and future custom-system-instruction settings would need duplicate wiring. Lifecycle is checked at load time, which is useful, but that does not establish consistent trust/ownership admission.

**Repair:** use the canonical Agent resolver and a typed admitted selection. Preserve shared foundation and applicable role guidance, resolve exactly one identity, freeze its revision and authority, and remove prose-based identity selection. This is source-verified; no hostile project execution was attempted.

### D03 — P2: child prompts are rendered before the final child tool subset is known

Core composes child prompts from the session capability set before final turn filtering (`lib.rs:5036`), and `task.rs:443` subsequently selects read-only tools and excludes flow without recomposing the prompt. Skills and MCP descriptors are retained in the child contract regardless of that reduced set. Thus a child can be instructed to use capabilities missing from its schemas. The full prompt is saved, but `prompt_layers` is empty (`task.rs:605`), losing contribution provenance.

**Repair:** resolve each child prompt from its final admitted capability snapshot and role/Agent settings, record schemas and layer contributions, and pass the actual delegated outcome rather than only the parent's latest message. Keep the self-contained task explicit; do not silently copy unrelated private parent history.

### D04 — P2: flow Agent nodes copy parent-facing prompts without adapting them

The managed flow dispatcher carries `cfg.system_prompt` into `ExecutorDeps` (`lib.rs:5117`, `lib.rs:204`). Flow nodes use it unchanged (`crates/vak-flow/src/exec.rs:548,563`), even when a node is read-only and its output is consumed by another node. Surface text can therefore describe a human desktop/chat reader, and tool instructions can describe a wider parent set.

**Repair:** use the same child prompt assembly for task children and flow nodes, with node audience, selected tools, evidence obligations, and result format. Helpers should inherit relevant task constraints without blindly inheriting conversational personality into strict JSON/TOML outputs.

### D05 — P2: the stop gate forces execution for valid non-code work

`OutcomeSpec::requires_execution()` (`crates/vak-intent/src/outcome.rs:530`) treats deliverables containing `author` and `verify` as execution requirements. The stop gate defines execution as Bash or file modification. Independently, `demands_verification()` (`crates/vak-agent/src/stop_policy.rs:235`) treats the word `verify` as requiring Bash, regardless of the actual evidence needed.

**Demonstrated:** an Author reading for “Write a short birthday greeting here” rejects a complete prose greeting for missing execution. Source verification with a successful inspection receipt also produces `VerificationMissing`, whose nudge explicitly demands Bash. The first probe supplies the Author reading directly; it does not claim every model/request classification follows that route.

**Repair:** completion must match the requested deliverable and evidence method. Prose can be an authored result; verification can be a source check, computation, external receipt, visual inspection, or a runtime test. Remove the parallel keyword-based Bash requirement that overrides that interpretation.

### D06 — P1: lengthy false-success prose bypasses the unresolved-error check

`reports_blocker()` (`stop_policy.rs:102`) returns true for **any text longer than 60 bytes**, or any mention of the tool, even if the answer claims success. A probe with an unresolved `write: disk full` error and a long “saved successfully” answer produced no stop reason.

**Repair:** retain unresolved failures as structured result state, require relevant recovery evidence or an actual blocker disclosure, and prevent successful completion claims from being inferred from response length. Bound continuation attempts, but when they are exhausted preserve the failure/unknown outcome. Prompt exhortations alone cannot close this gap.

### D07 — P2: an old intent note survives a newer intent that needs no note

`crates/vak-session/src/log.rs:908` finds the latest Intent entry **whose `model_visible` is Some**. A newer general intent with no note therefore does not supersede an older note. A previous instruction to ask before acting or provide a particular evidence level can remain in later unrelated context.

**Repair:** select the latest Intent first, then project its optional note. Test high-stakes → ordinary question and cited-research → creative-writing transitions, including after compaction. This is source-verified, not a new runtime probe.

### D08 — P2: intent communication posture is not connected to the inspected delivery path

Intent derives output shape, cadence, and urgency (`crates/vak-intent/src/engage.rs:562`), including quiet digest delivery for unattended work. Server delivery constructs profiles with default Live/Notify and applies formatting preferences (`crates/vak-server/src/delivery.rs:350,410`). `deliver()` consults that profile's posture at line 508, but the inspected path does not map the resolved intent posture into it. Repository references to `posture.delivery` outside intent are CLI display, not this delivery handoff.

The delivery library correctly holds packets and exempts approvals/interrupts **when given the appropriate posture**. That capability does not establish that the Agent's intent reaches it.

**Repair:** carry admitted communication posture through the turn and delivery envelope, respecting explicit user notification settings. Keep mandatory approvals immediate. Test real profile construction through enqueue/replay; do not rely only on a unit test of the disposition function. This is a source-traced integration gap, not a live notification test.

### D09 — P2: commitment progress can be inferred from prose length

Commitment closure itself is stronger: `may_close()` checks outstanding criteria and minimum evidence strength, and the ledger calls it before appending success. Semantic criteria remain Asserted; machine-checked criteria and external receipts have distinct strengths.

However, `crates/vak-core/src/commitments.rs:181` classifies any completed response longer than 40 bytes as `Learned` when no criterion moved. This can clear a stall streak without evidence of learning. Repeated eloquent non-progress could therefore keep work eligible longer than intended. This is a source-level risk, not a campaign demonstrating runaway execution.

**Repair:** count evidence-backed state changes or attributable new information as progress. Keep honest output, task completion, commitment satisfaction, and scheduler eligibility as distinct states.

## Intent, planning, commitments, and stopping: the full control chain

Intent resolution uses declared axes, deterministic signals, confidence, and narrowing limits. Core currently consumes `resolution.intent()`; the main inspected path does not issue an extra intent-classification model call on escalation. Its optional posture note is stored in `IntentRecord.model_visible` and projected into context as an `<intent>` user-message block, rather than editing the seed. Uncertain interpretation and adaptation must preserve general-purpose behavior.

Managed planning uses a separate JSON-authoring call; workflow planning uses TOML. Their instructions must preserve the user's complete task, scope, and relevant Agent guidance, while runtime validation—not the planner's prose—owns admissibility. The authoring prompt lists shapes and enums but should also explicitly distinguish proposals, unresolved assumptions, and evidence requirements. A planning response does not itself complete work or authorize effects.

On an assistant response without tool calls, the inspected order is Stop hooks → built-in stop policy → goal audit → managed-work gate → `TurnOutcome::Completed` (`crates/vak-agent/src/lib.rs:1260`). Hooks/guards inject logged continuation messages. Goal audit executes brokered verification and can call a judge. Durable commitment evaluation is a separate Core/commit-ledger stage; a completed conversational turn is not automatically a fulfilled commitment.

Both `[stop-hook]` and `[stop-guard]` continuation append errors are ignored before continuing. Unlike a successfully appended note, a missing append cannot provide the intended correction to the next request. Repair this alongside the primary audit's F04 persistence contract. Also preserve failure semantics when bounded stop/audit budgets end; neither retry count nor fluent final prose constitutes evidence.

## Temporal handling

### T01 — P2: no first-class current-time context in the inspected model prompt path

The main prompt assembly includes identity, rules, capabilities, surface, skills, and diagnostics, but no current instant or resolved user timezone. Ledger entries have timestamps; normal message projection copies the message alone (`crates/vak-session/src/log.rs:919`). Those timestamps therefore do not automatically tell the model when an old “tomorrow” was written. Compaction and handoff instructions likewise do not explicitly retain temporal anchors.

**Repair:** provide a recorded per-turn temporal context: UTC instant, resolved user/conversation IANA timezone with provenance, and local date. Preserve the original timestamp and interpreted target of relative-time requests. Refresh clock context across long-lived turns and scheduled runs without changing old ledger entries. Unknown timezone must remain unknown; the server's zone is not automatically the user's.

### T02 — P2: scheduled work is host-local and lacks a one-shot due-time contract

`cron_next_after()` accepts `DateTime<Local>` (`crates/vak-core/src/tasks.rs:131`), and the server scheduler uses `Local::now()` (`crates/vak-server/src/lib.rs:13623`). `TaskDef` stores cron or interval but no named timezone or one-shot `due_at` (`tasks.rs:200`). The model `tasks` schema likewise offers cron/every_secs, not timezone or a one-time reminder timestamp. The HTTP task listing exposes the current numeric UTC offset as `timezone`, not a stored IANA zone.

For a universal multi-machine/channel platform, “tomorrow at 9” and “weekdays at 9 in New York” cannot be reliably represented by silently choosing host-local cron. A cron expression is recurring, not intrinsically one-shot. Daylight-saving changes also cannot be represented by permanently converting a named zone to its current offset.

**Repair:** define explicit named-zone and one-shot semantics in the scheduling contract, expose them consistently through tools/API/UI, and have the model confirm the interpreted date/time when ambiguity materially affects the action. Do not simulate scheduling by promising a future response.

Existing strengths: cron parsing rejects invalid expressions; spring-forward gaps are skipped and fall-back folds choose the earliest instant; startup catch-up is bounded to a catch-up run. These are scheduler mechanics, not a substitute for user timezone resolution.

### T03 — P2: freshness currently measures the latest successful tool receipt, not necessarily the cited fact

Core derives turn evidence freshness from the latest successful tool receipt timestamp (`lib.rs:5444`). Fetching an old article now produces a new receipt, but does not make its claims current. This age signal is useful for execution freshness; it cannot establish source publication time, event time, claim relevance, or per-result applicability. Existing outcome evaluation explicitly retains some support/freshness states as Unknown, which should be preserved.

Further, `evidence_state_from_age()` (`crates/vak-intent/src/outcome.rs:301`) marks **any future timestamp Fresh**, without a bounded clock-skew tolerance. Evaluation overwrites the admitted effective evidence-age limit with `self.inner.config.intent.evidence_max_age_secs` (`lib.rs:5399`), potentially differing from the effective limit used at admission.

**Repair:** keep observation time, source publication time, event/effective time, retrieval time, and expiry separate; bind evidence to the actual requirement. Preserve the admitted freshness policy through evaluation. Future timestamps beyond a documented skew tolerance should be unknown/invalid, not automatically fresh.

### T04 — P2: delayed runs receive the stored task wording without explicit scheduling context

Task execution passes `snapshot.prompt` to `spawn_isolated_run()` (`crates/vak-server/src/lib.rs:13308`). The inspected call does not add scheduled-for time, actual trigger time, timezone, or catch-up status. A stored “today” can intentionally mean each run's current day, while “remind me tomorrow about yesterday's meeting” refers to creation-time anchors; those meanings need an explicit persisted distinction.

**Repair:** record original request time and normalized schedule, then supply scheduled-for/started-at/catch-up context on each run. Relative dates in one-shot obligations should freeze to intended instants; relative dates in recurring report instructions should be explicitly run-relative. Late delivery should state the relevant event/observation time rather than presenting delayed information as current.

Runtime deadlines/retry pacing already use bounded timeouts and monotonic elapsed time in the agent loop. Keep duration measurement separate from wall-clock calendar reasoning. This audit inspected those boundaries but did not rerun the complete network-resilience campaign.

## Acceptance requirements and validation

### Reconciliation after implementation

The original findings above describe the pre-repair baseline. Current status is: D01, D05, D06, D07, T01, T02, and T04 have targeted implementation coverage; D03 and flow provenance now retain inherited prompt-layer descriptors; and delivery posture has a conversion path into the outbox profile. D02, D04, D08, D09, and T03 still require broader end-to-end coverage or additional runtime wiring. Historical diagnostic fixtures remain labeled as baseline probes and are not product acceptance tests.

### Implementation update (2026-09-13)

The initial corrective pass now carries additive Agent custom instructions through save, identity, top-level prompt layers, and typed child-agent selection; adds explicit UTC/local temporal context to runtime prompt sections; propagates the parent Agent identity into task dependencies; and makes capability/intent ledger failures abort the turn. Stop-policy execution demands were narrowed so conversational authoring and generic verification wording do not force Bash receipts. Task storage and APIs now also carry optional named-timezone and one-shot `due_at` fields with validation; cron evaluation uses `chrono-tz` for real IANA zones, and one-shot tasks disable after firing. Flow and child sessions retain prompt-layer provenance, evidence receipts distinguish source/event time from retrieval time, and delivery metadata can map intent cadence/urgency into the actual outbox profile. Remaining D03/D04 and T03–T04 items require broader runtime changes (emitting posture metadata from every resolved turn, flow prompt composition beyond inherited layers, and requirement-bound evidence evaluation).

Add end-to-end cases covering: custom Agent → default child; saved Agent → named-role child; read-only child with missing Bash/MCP; flow child on a phone-origin request; changing intent with an empty new note; prose-only authoring; source-based verification; unresolved failure with confident long prose; bounded stopping with honest unresolved state; useful learning versus repetitive commentary; unattended digest through actual delivery; a channel user in another timezone; midnight-crossing conversations; DST transitions; one-shot reminders; delayed catch-up; stale articles fetched now; future-dated evidence; and freshness-policy edits between admission and evaluation.

Executed on 3.0.82: **77 intent unit tests, 15 commitment unit tests, 11 commitment lifecycle tests, 27 task-store/scheduling/tool tests, and four diagnostic probes passed.** The scheduling tests include DST gaps/folds, leap-day rollover, and strict next-fire behavior. The probes confirm D01, both D05 cases, and D06; they do not validate desired behavior. Documentation path and fixture-format checks passed. Probe source: `docs/audits/fixtures/subagent-prompt-audit-2026-09-13.rs`, adapted from the existing subagent roundtrip test. Temporarily copy it to `crates/vak-agent/tests/subagent_prompt_audit_probe.rs`, run `cargo test -p vak-agent --test subagent_prompt_audit_probe`, then remove that temporary copy.

No live models, external sends, scheduler triggers, private Agent configuration, or production prompt changes were used. Other findings are source-traced and must be closed with focused tests plus representative live acceptance. This is a prompt/control integration audit, not a claim that every scheduling, transport, and security path has been exhaustively proven.
