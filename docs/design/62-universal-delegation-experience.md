# 62 — Universal delegation experience

Status: **superseded — the Agent-owned platform contract in docs/design/64-agent-owned-platform.md is authoritative; this document is retained only as historical design context.**

Date: 2026-09-13

## 1. Product decision

Vak is a universal assistant. A person states what they want, Vak organises and
performs the authorised work, and the person receives a useful result. The
person can continue talking, change direction, leave, and return without
coordinating agents or reconstructing progress from execution logs.

The interface must work equally well for household organisation, learning,
travel, research, writing, creative work, analysis, and software. These are
acceptance examples, not hardcoded modes, agent roles, or capability categories.

User-facing promise: **Ask. Continue with your day. Return to useful results
and clear decisions.** Background operation is promised only when the runtime
can actually sustain it; closing a client and stopping its host are different.

This document extends design 61 with an implementation contract for delegation,
concurrent conversation, durable results, and completion delivery. Design 61
remains the shell and interaction authority. Designs 42, 47, and 52 remain the
work, intent, commitment, and outcome authorities. Design 57 remains the
adaptive presentation authority. Do not revive the modes superseded in design 60.

## 2. Starting evidence and limits

The September 13 review inspected source and design documents. It did not run
live acceptance journeys or establish competitor reliability.

| Area | Existing evidence | Required follow-through |
| --- | --- | --- |
| Delegation | `crates/vak-agent/src/task.rs` implements child agents, parent linkage, narrowed tools, and result return; delegation is blocking at the tool boundary | Reuse for bounded subtasks; do not mistake a child tool call for a durable independent job |
| Parallel execution | `crates/vak-agent/src/lib.rs` schedules parallel tool work; task declarations include resource scopes | Verify independence and shared budget enforcement; unknown conflicting effects must serialize |
| Admission | `crates/vak-core/src/lib.rs` registers the task capability with permissions, budgets, and contract context | Preserve this boundary for every new dispatch path |
| Conversation | `crates/vak-client-ui/src/App.tsx` sends input during a running task through steering | Distinguish steering from new independent work without losing either request |
| Background work | `crates/vak-core/src/tools_automations.rs`, trigger storage, and `crates/vak-server/src/heartbeat.rs` provide scheduling and attention mechanisms | Verify durable ownership, recovery, and origin-linked delivery across surfaces |
| Results | `crates/vak-client-ui/src/components/PresentationRenderer.tsx` contains rich renderers and alias dispatch | Complete the existing typed presentation contract and stable result identity |
| Experience | Design 61 marks the simplified shell implemented and later journeys phased | Deliver and verify those journeys rather than declaring feature parity from component counts |

The implementing agent must recheck these observations against current HEAD.
Record any drift before changing code; reuse newly landed implementations.

## 3. Scope and non-goals

Deliver one coherent journey from request through work, decision, result, and
continuation. Keep simple questions lightweight. Promote work to existing
durable contracts only when its lifetime, dependencies, or requested monitoring
requires durability.

Do not create a second scheduler, task database, intent classifier pipeline,
permission system, message ledger, or renderer registry. Extend existing owners
additively. Do not add an agent marketplace, agent-count selector, mandatory
planning wizard, domain selector, or permanent technical dashboard to the main UI.

Multiple agents are an execution choice. Use them only when independent work
benefits enough to justify added latency, coordination, and cost. A single-agent
completion is equally valid. Do not promise autonomous access to apps or
transactions that the installed, authorised capabilities cannot perform.

## 4. Canonical user journey

Example request: “Plan a weekend away, draft an invitation for my friends, and
watch for availability at the place we choose.”

1. Record the request before optional integration discovery or planning.
2. Reuse known, permitted context. Ask only for information necessary to avoid
   materially wrong work, such as dates when live availability depends on them.
3. Admit the outcome and its bounds through the existing runtime. Make safe
   progress on independent pieces while necessary questions remain pending.
4. Split comparison and invitation drafting if useful. Keep shared constraints
   linked to the same outcome revision. Availability monitoring depends on an
   actual selection; never silently choose and activate that dependency.
5. Present a comparison and invitation draft with appropriate actions. Preparing
   a draft does not authorise sending it or committing money.
6. Create the requested monitor when its necessary scope is resolved. Confirm
   timing, expiry, notification rule, and where updates will arrive.
7. When something changes, attach the update to the originating work. A booking
   requires the existing approval boundary and an exact, fresh review.
8. A correction such as “Make it suitable for children” revises affected work.
   Retain prior results and make the current revision clear.

## 5. Conversation and work routing

The conversation is the visible place; execution sessions retain their own
frozen contracts and append-only histories. A unified conversation projection
must not merge security scopes or silently rewrite a session's route.

Interpret each input using existing outcome/intent machinery and explicit UI
target references where available. Avoid another mandatory model call for every
message. Routing interpretation never grants authority.

| Input relationship | Behaviour |
| --- | --- |
| Follow-up or correction | Target the relevant existing outcome/result revision; steer active work or begin a follow-up episode |
| Independent request | Admit a separate work item through existing orchestration; leave unrelated work running |
| Status question | Answer from current evidence without cancelling, restarting, or revising work |
| Explicit cancellation | Cancel the addressed work and its dependent children; retain completed and partial results |
| Schedule or monitor request | Use the canonical durable scheduling mechanism; record scope, timing, destination, and stop condition |
| Ambiguous reference | Preserve input and running work; resolve from conversation context, or ask a short clarification when a consequential target remains ambiguous |

Do not change the meaning of the existing steering endpoint. Introduce any
required additive routing envelope at the shared admission boundary, with old
explicit steering still targeting steering. The default client send path uses
one routing contract rather than racing a new-job request against steering.

Required routing facts: originating message ID, conversation/session references,
target outcome or work ID where known, relation, outcome revision, and decision
provenance. Reuse existing fields and entries first. New model-visible facts
require logged entry types and `derive_messages()` coverage.

Use a client request ID for idempotent admission. A timeout/retry must recover
the original admission, not create duplicate work. If routing is unavailable,
show the input as pending or failed with retry; never silently dispatch it to an
unrelated active run.

## 6. Runtime ownership and delegation

The existing outcome runtime owns decomposition and synthesis; existing work
contracts own dependencies and evidence; the commitment/task scheduler owns
durable continuation. The UI projects those owners and never becomes a scheduler.

Each independently running item needs an owner, origin, admitted objective,
dependency references, cancellation scope, budget accounting, and recovery
state. Bind these to existing types rather than introduce a parallel vocabulary.
Durable jobs may outlive the client; blocking children remain bounded to their
parent lifecycle unless existing durable machinery explicitly owns the work.

Delegation rules:

- Children receive only relevant logged context, their objective and completion
  criteria, and capabilities no broader than their parent.
- A saved helper may be selected by stable profile ID or name through the task
  tool's `agent` field. The runtime loads the profile from the workspace,
  records the selected revision in the child contract, and applies only its
  personality and working-style text. An unknown helper fails closed; profile
  selection never widens role, tools, permissions, credentials, budget, or
  approval behaviour.
- Durable scheduled tasks carry the same `agent_id` and
  `agent_revision` provenance fields. A background result can therefore
  identify the helper personality that created it even after the helper is
  edited or removed. The scheduler treats this as provenance, never as a
  permission or model pin.
- Context snapshots identify the outcome revision. Parent corrections invalidate
  affected pending work; completed older results remain labelled with provenance.
- Apply configured concurrency and shared spend limits before dispatch. Sum child
  spend into the parent budget; splitting work cannot multiply permitted spend.
- Reuse resource-claim scheduling. Independent reads can overlap; potentially
  conflicting writes or external mutations serialize unless independence is proven.
- One child failing does not erase successful siblings. Retry only eligible
  failures within the frozen route ladder, remaining budget, and cancellation rules.
- Synthesis references child evidence and discloses unresolved contradictions.
  Child prose alone cannot establish observed or attested completion.
- Capability or permission revocation cancels affected leases immediately, including
  waiting jobs and children. A later wake revalidates current authority.
- No new recursive agent topology is required. Preserve current depth limits unless
  a separately justified runtime change is necessary.

## 7. Work and result projection contract

The following is a conceptual projection checklist, not a replacement persisted
schema. Map it to existing `OutcomeSpec`, work/commitment types, and
`OutputTimeline`/`PresentationDocument` before proposing additive fields.

| Projection | Required information |
| --- | --- |
| Work | Stable ID, origin, objective revision, owner, dependencies, lifecycle, current action, timestamps, result references, evidence references, optional helper profile ID/revision |
| Result | Stable ID, originating work and turn, semantic kind/version, immutable revision, content or artifact reference, lifecycle, freshness, actions, evidence, optional helper profile ID/revision |
| Action | Stable action ID, target ID/revision, readable label, typed arguments, current authorisation requirements, availability/reason, idempotency identity |
| Attention | Origin and target, reason, deduplication identity, read/resolved state, optional approval ID, delivery receipt |

Expose plain-language work states: queued, working, waiting for you, scheduled,
complete, partial, failed, and stopped. These are projections of canonical
runtime states, not a second lifecycle. Keep execution failure, pending delivery,
and unsupported preview separate. A queued notification does not make work fail;
a renderer succeeding does not make work complete.

Runtime evidence determines completion. Missing evidence, stale data, and
unsupported presentation remain explicit. No fabricated percent-complete, time
estimate, connector state, citations, artifacts, or success receipts.

Every streaming update carries delta and snapshot with stable identity and
ordering. Reconnect, repeated events, hydration, and historical replay must
produce the same result collection without duplicate cards. Editing creates a
new revision; do not rewrite ledger entries. Stale action revisions are rejected
or refreshed for review, particularly approvals and external effects.

Use the existing typed renderer registration path. Remove superseded alias and
heuristic dispatch for converted producers in the same change, while preserving
historical ledger readability and exact Markdown fallback. Unknown types degrade
to clean supported content, never raw transport JSON. Generated interactive
artifacts remain sandboxed; model-authored HTML never mounts into the host UI.

## 8. Interface specification

Preserve the Vak mark, semantic theme tokens, and one universal shell. Use
typographic hierarchy and spacing to distinguish conversation, activity, and
results. Give Vak a recognisable character using the direction below. Avoid
animated agent networks, glowing status panels, or nested cards that consume
more space than their content. Character supports the experience without
becoming another control surface.

### Visual direction: warm, expressive, composed

The desired feeling is a welcoming personal studio: useful enough for serious
work, pleasant enough to return to for everyday life. The product must feel
designed beyond a sidebar and a generic chat box. Personality is part of this
scope, not a later decorative pass.

Use warm paper backgrounds and ink-like text for the primary light treatment,
warm charcoal for dark, and restrained terracotta with sage supporting accents.
Keep the existing themes and contrast guarantees. Use colour purposefully in
illustrations and result content; reserve semantic error/warning colours for
their meanings. Avoid washing every element in the same accent.

Use generous outer whitespace, comfortable reading text, stronger result titles,
and small secondary metadata. Navigation is quieter than the conversation.
Prefer flat surfaces with selective elevation for the composer, open menus, and
inspected results. Avoid borders around every paragraph and identical rounded
cards for every kind of answer. Derive sizing from a small consistent token scale.

Results should have an appropriate visual shape: an itinerary reads as a clear
day-by-day sequence, a comparison as aligned alternatives, a letter as a readable
page, a learning plan as approachable steps. Use actual relevant images when
available and authorised; never fabricate a destination photo or evidence image.
Illustration is allowed as clearly decorative content. Generic answers remain
well-set prose; rich layout must earn its space.

### Vak's character

Create a small original companion motif derived from the existing Vak mark,
without replacing the brand icon. Explore two or three visual treatments in
the design prototype, then ship one coherent treatment. Suggested starting
point: a soft abstract figure with a distinctive silhouette and restrained
expression, legible at 24px and pleasant at 64–96px. It should feel friendly to
adults without assuming everyone wants a cartoon companion.

Provide a quiet appearance option using the Vak mark alone. Personality and
animation preferences must not change capability, permissions, or evidence.
Do not make avatar customisation part of mandatory onboarding.

| Moment | Character behaviour | Constraint |
| --- | --- | --- |
| Empty conversation | Small welcoming illustration and an inviting prompt | No repeated greeting wall or invented familiarity |
| Working | Subtle expression or short motion when real activity starts | No endless bouncing, pretend emotions, or animation presented as progress |
| Needs your decision | Calm attentive pose beside a clear decision card | Never pressure, guilt, or make approval feel like pleasing the character |
| Result ready | Brief, restrained acknowledgment | No confetti for routine work; result remains the focal point |
| Failure/offline | Neutral, helpful state with recovery text | No sad mascot substituting for the cause or hiding lost connectivity |

Motion uses brief transitions around 120–220ms for controls and 180–300ms for
panel/character state changes. Avoid continuous decorative loops. Reduced-motion
mode removes nonessential motion while preserving all status information.

### Personality in language and voice

Vak's default manner is warm, observant, practical, and candid. It can show light
humour when appropriate, but must adapt to serious or sensitive requests. Be
specific about useful progress, and be comfortable saying what remains unknown.
Avoid corporate jargon, theatrical agent chatter, excessive praise, and constant
exclamation marks. Never claim feelings, personal experiences, or remembered
facts the system does not have.

Examples of default copy:

| Situation | Preferred copy |
| --- | --- |
| New conversation | “What would you like a hand with?” |
| Independent task accepted | “I'll work on the invitation too.” |
| Working | “Comparing places that fit your dates.” |
| Result | “Here are three options within your budget.” |
| Partial completion | “The draft is ready. I couldn't check availability yet.” |
| Approval | “Send this invitation to these four people?” with the actual draft and recipients |
| Missing connection | “Connect your calendar to add these dates.” |

These are writing examples, not strings to display regardless of evidence.
Do not imply an effect occurred before its receipt exists. Reuse design 38 voice
and design 45 prompt-layer mechanisms for preferences; do not add a second
persona prompt store. Presentation character state is UI data; it cannot edit
code-owned capability or safety blocks. Voice narration is optional and never
autoplays simply because a background result arrived.

### Optional characters for delegated work

The user may enjoy seeing that several helpers are contributing. Support this
as optional, compact expression of actual delegation, not a required team setup.
Vak remains the consistent primary assistant and owns the combined answer.

When several real children are running, an expandable line can say “Working on
the options and invitation” with at most three small character marks plus an
overflow count. Expanding reveals a plain label such as “Comparing options” or
“Preparing your draft,” current state, and optional details. Single-agent work
does not invent helpers to make the screen look busy.

Character appearance should be stable for a delegated item's lifetime and
consistent across reconnect. Store or deterministically derive it from existing
stable identity in the projection. Use a small original visual family with
distinct silhouettes and accessible labels; colour alone cannot distinguish them.
Friendly display names may be optional, but the work label is always primary.
Names and illustrations do not imply separate credentials, expertise, persistent
memory, or authority. Never label a character a licensed professional.

Do not hardcode a “researcher/writer/coder” team into the harness. Labels follow
actual admitted work; visual traits are presentation metadata. No unsolicited
messages from each helper, mandatory agent switching, or new top-level character
navigation. Users can inspect delegation when curious and ignore it entirely
while still receiving the same complete result.

### Personal agents: names, characters, personality, and behaviour

Users can personalise Vak and create reusable named helpers. This is a
first-class optional experience, beyond anonymous decorations on running tasks.
A person might name a helper “Pip,” choose a lively illustrated character, and
ask it to communicate warmly and favour practical next steps. Another might
create “Atlas,” with a quiet geometric character and a concise, methodical style.
These are illustrative user choices, not built-in specialist roles.

Vak remains useful immediately with its default identity. People who want their
own cast of helpers can build one without writing prompts or configuration files.
Personalisation should feel like creating a character, with a live preview,
rather than configuring an execution engine.

| Editable property | Product behaviour | Runtime boundary |
| --- | --- | --- |
| Name | Display name, optional pronunciation, and optional short description | Stable internal ID survives renaming; names need not be globally unique |
| Character | Choose an original preset, upload an image, or generate one when an authorised image capability exists | Assets are validated and safely rendered; unavailable generation is explicit |
| Appearance | Accent, supported expression set, and optional visual accessories | Contrast and state labels stay accessible; no arbitrary executable character code |
| Personality | Plain-language description plus editable examples such as warm, playful, direct, thoughtful, or formal | Compiles to existing editable identity/rule layers; never overrides guardrails |
| Communication | Preferred brevity, explanation depth, language, humour, and formatting | Preferences adapt to the request; required uncertainty and evidence remain visible |
| Working style | For example, compare alternatives, suggest a plan for complex work, or verify before presenting | Preferences guide work inside the admitted outcome; they cannot force unnecessary planning or bypass evidence gates |
| Animation | Off, subtle, or expressive; preview available state animations | Global reduced-motion preference wins; execution state remains truthful |
| Voice | Available voice and supported delivery controls with an explicit play preview | Use existing voice configuration and discovered availability; do not create a second voice engine |
| Preferred responsibilities | User-written description of work this helper is useful for | Guidance for admitted role selection, not a credential or capability grant |

Keep semantic permission and notification controls separate from personality.
“Bold,” “independent,” or “never ask me” cannot increase access, spend, background
scope, or authority. Describe this in the editor with one short explanation near
working preferences, rather than interrupting ordinary character customisation.

#### Creation and editing journey

Provide “Personalise Vak” and a secondary “Your agents” entry in Settings or the
profile menu. The latter opens a small collection of saved helpers. It is not a
mandatory primary destination or a prerequisite for delegating work.

The character editor combines a large preview with approachable sections:
“Name and look,” “Personality,” “How they work,” and “Voice and movement.” Start
with name and appearance; expose additional controls progressively. Use clear
choices and prose examples instead of pseudo-scientific personality scores.

Include Create, Edit, Duplicate, Reset changes, and Remove actions. Editing uses
a draft until Save; Cancel has no persistent effect. Reset changes restores the
saved revision, while restoring defaults is separately labelled. Preview text,
voice, and animations without modifying a live agent. Static previews cost no
model call; an optional generated reply/voice preview is explicit, bounded, and
cannot call effectful tools or use private task context automatically.

Show a few example replies in different situations so a personality feels real:
a greeting, a useful result, a clarification, and a failure. Playfulness must
soften appropriately for serious requests and failures. Do not create fake
emotional attachment, guilt for inactivity, or pressure to keep interacting.

#### Assigning a helper to work

Allow conversational assignment (“Ask Pip to prepare the invitation”) and an
optional picker attached to a task. Resolve the display name to an admitted
stable role/profile ID; ambiguous names require a choice. Selecting a helper
must not silently create a new conversation or change workspace/provider.

When no helper is specified, Vak may choose a suitable admitted role within the
existing delegation policy, or work directly. The user never has to fill every
role. Do not spawn every saved helper for each request. Multiple instances of
one helper may run concurrently; show distinct work labels and retain separate
execution identities under the shared character profile.

The task row pairs the selected name and character with its actual work:
“Pip · Preparing the invitation.” Opening it shows that helper's contribution,
state, and evidence. The main conversation still receives one coherent result
from Vak. An explicit user request to inspect a helper can reveal its scoped
conversation through existing controls; no separate unsolicited chatter stream.

#### Storage, inheritance, and application

Map reusable helper behaviour to the existing named-role definitions and
identity/operating-rule layers in design 45. The saved character profile holds
presentation metadata and references the canonical role identity; it must not
store a second competing system prompt. Implement one editor facade over those
owners, with a validated revision and atomic publication so failed saves do not
leave mismatched appearance and behaviour.

Use existing Shared/project inheritance with explicit provenance. Shared helpers
are reusable across workspaces; project overrides store only local intent.
Project prompts remain subject to trust checks. Task-level selection is scoped,
and cannot mutate the saved default personality. Cross-workspace availability
does not grant cross-workspace memory or file access.

Names, appearances, and optional voice preferences are versioned. At execution
admission, record the selected profile/role revision and all model-visible
contributions through the existing prompt and session contracts. Historical
messages retain their recorded identity; renaming must not rewrite their ledger.
Purely visual edits can refresh current UI without interrupting work. Behaviour
edits apply at the next boundary permitted by the existing frozen prompt/drift
contract; explain when an already-running or explicitly resumed session retains
its previous version. Do not bypass drift acceptance or silently change a frozen
contract for the sake of instant personalisation.

Adding a helper follows capability reconciliation without restarting the daemon.
Removing one revokes future selection immediately and follows existing immediate
revocation rules for active capability leases. Preserve historical profile
references and task evidence. Preview the affected active tasks before a removal
that would stop them; removal never deletes conversation ledgers.

Character artwork should be a bounded set of safe assets and declarative state
mappings. Uploads enforce format/size limits and safe handling; arbitrary SVG,
scripts, remote trackers, and model-authored executable animation never enter
the host DOM. Asset failure falls back to initials or the Vak mark with the
agent's name still readable. Generated artwork uses the normal authorised
capability, explicit output scope, and artifact lifecycle.

#### Character acceptance criteria

- Create a named helper entirely through the UI, customise its character,
  personality, working preferences, animation, and available voice, and reuse it
  in another authorised task after restart.
- Assign it by name and by picker; resolve duplicate names without guessing;
  verify that the admitted role and logged prompt match the saved revision.
- Demonstrate two concurrent instances with one character and distinct work
  labels, cancellation scopes, and evidence.
- Edit, cancel an edit, duplicate, reset, and remove; verify scope/provenance,
  atomic save failure, no ledger rewrites, and correct active-work handling.
- Verify behaviour preview is isolated, animation respects reduced motion,
  missing assets degrade cleanly, and voice does not autoplay.
- Attempt a personality instruction to bypass approvals, reveal another task's
  context, or raise spending; existing runtime boundaries must still reject it.

### Required design prototype before broad UI implementation

Produce a reviewable prototype covering: first visit, an ordinary conversation,
two concurrent tasks, a useful rich result, an approval, and a partial failure.
Include the character treatment, the quiet alternative, the helper collection,
and the character editor with live preview and task assignment. Use clearly labelled
fictional fixture content only in this prototype, never in production projections.

Show both light and dark at desktop width, plus the narrow-screen conversation
and result inspection. Verify long titles, no-image results, keyboard focus, and
reduced motion. Select one visual treatment and define its tokens, character
assets, state mapping, and component anatomy before spreading styling changes
through the app. This visual prototype can precede P1; production UI still lands
with the vertical slices below. Keep assets original or appropriately licensed.

The review question is: “Can someone unfamiliar with agent software understand
what to ask, what is happening, and what they received—and enjoy using it?”
Do not use visual similarity to another product as the acceptance criterion.

### Navigation and Home

Retain design 61 destinations. Put an “Ongoing” collection within Home, populated
from actual work/commitment projections, alongside items needing a decision and
recent useful results. Do not add another top-level task dashboard. A new user
gets a welcoming composer and a few broad examples; no sample activity is shown
as real. Scheduled and Inbox continue to use their existing canonical stores.

Use “conversation” for the place people talk and “task” for work Vak is doing.
“Result” names a deliverable. Technical terms such as worker, dispatch receipt,
and execution environment belong in Details or settings.

On first launch, open the canonical default workspace (`~/vak-home`) automatically
and take the person straight to the conversation. Workspace selection is a later
switch available from the workspace controls; it is not an onboarding gate. A
separate trust review still appears when a selected workspace requests privileged
behaviour, because choosing where to start and authorising project settings are
different decisions.

### Conversation

Keep reading width roughly 680–800 CSS pixels on wide displays. An active turn
has one compact activity summary grounded in events, for example “Comparing
three options.” Multiple independent tasks each retain a short labelled row;
do not mix their messages into an unattributed stream. Completed internal
activity collapses into optional evidence.

Show concise result content and the relevant action together. A comparison may
offer “Choose”; a draft “Edit” and “Download”; a reminder “Change” and “Pause”.
Only offer actions backed by a callable, currently authorised implementation.

### Composer and concurrency

Keep text, attachments, and voice easy to reach. Model, workspace, and permission
configuration stay in the existing deliberate disclosure; healthy infrastructure
does not occupy permanent screen space. Do not require choosing an agent.

When explicitly replying to a result, show a removable target chip. Otherwise
accept natural conversation and apply the routing contract. The composer remains
usable while work runs. A status question should not look like a queued mutation.
Show brief attribution after independent work is accepted, so a routing mistake
can be corrected. Corrections append new intent and safely cancel/revise affected
work; they cannot undo an already committed external effect.

### Result inspection and decisions

Open a resizable result panel only on user action or explicit request. Preserve
conversation scroll and result selection. On narrow screens use a full-width
detail view with a clear return action and restored focus. Keep previews, versions,
and evidence associated with the selected result.

An approval card shows the concrete action, recipient/target, relevant cost or
scope, and expiry where applicable. Approval uses deterministic controls bound
to the exact request. “Keep going” or unrelated chat must not approve a different
pending action. Missing credentials lead to the existing secure setup flow,
never a request to paste secrets into conversation.

### Accessibility and finish

Use readable body text around 16px, consistent spacing and control sizing,
visible keyboard focus, and WCAG AA contrast across existing themes. Meet 24px
minimum target sizing and aim for 44px touch targets. Status must not rely on
colour. Announce meaningful state changes without narrating every streamed token.
Respect reduced motion. Test keyboard navigation, screen-reader labels, long
content, and responsive layouts at 390px, 768px, and 1440px widths.

## 9. Background continuity and delivery

Before claiming “I'll keep checking,” persist a schedule/commitment and verify
that the configured service can execute it. Show next run, timezone, notification
rule, expiry or stop condition, and pause/remove controls. If the host must stay
running, state that fact. Do not introduce a client timer as background execution.

Wake processing is level-triggered and idempotent. Service managers own process
recovery. Durable work recovery reconciles recorded dispatch and effect receipts
before re-execution. An uncertain external effect becomes a reconciliation or
human decision state; never blindly retry a possible purchase or sent message.

Persist origin references through tasks, commitments, inbox, and outbox. A
completion links back to the originating conversation and current result revision.
Deduplicate by source event/result revision plus destination, not generated prose.
Use the existing delivery system and preserve workspace and bot identity scopes.
Unreachable origins produce an explicit delivery state, never rerouting to a
different workspace or chat.

Notify for a requested completion, a meaningful monitored change, a terminal
failure, or necessary user action. Unchanged checks remain quiet. Group related
updates; repeated retries do not generate repeated unread items. Reading an
update and resolving the underlying decision are separate actions.

## 10. Implementation sequence and acceptance gates

The production visual reference for the shell, character treatment, six-state
walkthrough, and responsive review is [62-universal-delegation-prototype.md](62-universal-delegation-prototype.md).

Implement vertical slices; each phase must work through admission, execution,
durability, presentation, and relevant delivery before proceeding. Update phase
status with evidence, not checkmarks inferred from code presence.

| Phase | Deliverable | Exit gate |
| --- | --- | --- |
| P0 — Contract map and visual direction | Map existing types, endpoints, recovery owners, and tests; produce the six-state visual prototype and select a character treatment | No competing runtime owner; gaps recorded; light/dark/mobile, expressive/quiet treatments, and accessible character states are reviewable |
| P1 — Persistent results | One stable result projection and typed actions; first slices are a document and a comparison | Both stream, survive reload/reconnect, reopen, revise, and preserve exact fallback and evidence |
| P2 — Concurrent conversation | Shared routing admission, explicit target references, idempotency, independent-work orchestration | A second unrelated request proceeds without steering/cancelling the first; corrections and status questions target correctly |
| P3 — Background continuity | Origin-linked durable work, recovery, monitor controls, deduplicated completion delivery | Close/reopen client and restart host; results recover, effects do not duplicate, updates return to the correct origin |
| P4 — Universal experience | Home projection, contextual results, approvals, warm language, reusable named helpers, character editor, assignment, personality/voice/animation preferences, responsive polish | Universal scenarios and character acceptance pass on desktop/web; expressive and quiet presentations work; configuring helpers remains optional |
| P5 — Release verification | Regression suite, real journey evidence, accessibility checks, rebuilt affected bundles, reconciled docs | No unresolved critical correctness/security failures; unsupported journeys are accurately documented |

P0 should be a concise section appended here or a linked audit, not a new
architecture essay. Recheck design 61 phases and avoid parallel implementations.

## 11. Verification matrix

Use deterministic fixtures for routing, durable lifecycle, ordering, and policy;
use controlled connector doubles for external effects. Live tests establish actual
capability separately. Never present fixture data as production behaviour.

| Scenario | Required observable result |
| --- | --- |
| Simple question | Direct answer; no unnecessary durable goal, plan UI, or agents |
| Weekend plan plus invitation | Shared constraints, independently useful outputs, no invitation sent without approval |
| Unrelated request during work | Two attributable outcomes; original work continues; request retry admits only once |
| “Make that shorter” on selected draft | Correct result revised; earlier version and unrelated tasks preserved |
| “How is it going?” | Evidence-backed status; no new job or silent cancellation |
| Household availability monitor | Persisted schedule, timezone, next run, expiry/stop control; unchanged checks silent; changed result delivered once |
| Learning plan plus worksheet | Usable editable/downloadable artifacts; follow-up retains context and origin |
| Research or data comparison | Sources/data provenance, uncertainty, and freshness; missing inputs never fabricated |
| Software change | Contextual diff and verification remain available without changing the universal shell |
| Missing integration | Actual availability reason and useful next step; no fake capability or endless setup loop |
| Child failure or budget exhaustion | Successful partial results preserved; affected work and remaining limits clear |
| Permission revoked mid-run | Main/child/durable dispatch fails closed; stale approval cannot resume it |
| Crash after external dispatch | Receipt reconciliation prevents duplicate effect; uncertainty stays visible |
| Reload/reconnect/out-of-order events | Stable IDs, no duplicate result cards or notifications, retained draft and reading position |
| Two workspaces/bots | No context, credentials, results, approvals, or delivery leakage |
| Unknown result schema or failed preview | Supported readable fallback or explicit unavailable state; no false completion |
| Character and delegation display | Only actual children appear; stable identities after reconnect; quiet appearance preserves all information; characters do not imply credentials or authority |
| Visual and language review | All six prototype states implemented coherently; light/dark/mobile, keyboard, reduced motion, and contrast verified; routine journeys require no technical vocabulary |

For visual/interactive acceptance, use a disposable workspace outside this source
checkout, explicit admitted output scope, exact artifact read-back, real preview
inspection, and quarantine/promotion rules from AGENTS.md. Closing the app is
not adequate evidence of host crash recovery; test both separately.

Record scenario, environment, model/provider where applicable, inputs, runtime
receipts, actual output, screenshots for UI cases, and pass/fail with limitations.
Measure completion, user corrections, unnecessary questions, misrouting,
duplicate effects/deliveries, latency, and spend. Require zero duplicate effects,
cross-scope leaks, or false completion in deterministic acceptance; report live
success honestly without inventing parity scores. Parallelisation must demonstrate
a benefit on independent workloads and stay within the same admitted budget.

Run focused tests for changed contracts, then repository-required checks before
committing. Rebuild affected embedded frontend bundles. A live model response,
type check, or screenshot alone cannot establish this whole contract.

## 12. Handoff instructions for the implementing AI agent

1. Read AGENTS.md and the current status of designs 41, 42, 47, 52, 54, 57, and 61.
2. Complete P0 using source evidence. Preserve unrelated work and all existing
   security, schema, configuration, and append-only contracts.
3. Establish the P0 visual direction, then implement P1 as a complete document/comparison journey. Follow the
   dependency order above; do not begin with a broad cosmetic rewrite.
4. Keep business-specific behaviours in capabilities and typed presentation data.
   Core owns generic routing, identity, policy, lifecycle, evidence, and delivery.
5. Replace superseded active paths in the same change, without removing historical
   schema support guaranteed within the supported major version.
6. For each phase, report what works, test evidence, remaining failures, and exact
   implementation locations. Update design 61 and this document consistently.
7. Do not declare this proposal implemented until the complete universal scenario
   matrix is verified or explicitly bounded by documented unavailable capabilities.

## 13. External inspiration

These sources inform interaction goals; they are product descriptions, not
independent evidence of real-world reliability or requirements to copy a brand.

- [How We Designed Muse](https://introducing.muse.ai/) — ongoing conversation,
  concurrent background work, goals, inspectable activity, meaningful notifications,
  and rich artifacts.
- [Meta introduces Muse](https://about.fb.com/news/2026/09/introducing-muse-personal-ai-agent/amp/)
  — personal delegation, persistent execution, and review of sensitive actions.
- [Meta describes Muse Spark](https://about.fb.com/news/2026/04/introducing-muse-spark-meta-superintelligence-labs/)
  — parallel workers contributing to one user request.

Vak's own contracts govern implementation. Its identity remains universal,
inspectable, extensible, and simple to use.
