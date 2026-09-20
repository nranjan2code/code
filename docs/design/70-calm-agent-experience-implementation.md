# 70 — Calm Agent experience: visual reference and implementation ledger

Status: **approved direction; implementation in progress, not shipped as a unified experience**. Updated 2026-09-20. This is the durable work list for the four screens created with the user in this task. The images are design references, not evidence that the UI or example data exists. Do not substitute the older `docs/assets/presentation-2026/` mockups for these screens.

## User decision and product aim

Vak is for everyday work across many domains. Most users are not developers. A person should arrive, say or type what they want, work with an Agent, see a clear result, and continue their day. The interface should be calm, attentive, smooth, and useful every day. It should conceal machinery until that machinery helps the user make a decision; it must not remove capabilities. Voice is a primary/default entry alongside typing. Agents have recognizable characters, glyphs, names and personalities, with restrained motion instead of perpetual attention seeking.

The user explicitly chose **invited humans in the same Agent conversation and draft**. Human and Agent coworking, inline revision, review, and acceptance are central to this direction. Sandbox work is primarily an Agent/LLM working environment; the user sees the outcome, evidence, preview and decisions. Coding, canvas and technical inspection remain powerful when relevant, without defining Vak's whole identity. Preserve the existing broad capability and presentation system, including the many semantic card/renderer types, while changing their everyday expression.

Authoritative architecture still comes from `64-agent-owned-platform.md`. Use `30-output-engineering.md`, `30-render-architecture.md`, `57-adaptive-presentation-runtime.md`, `67-presentation-renderer-guide.md`, `54-task-environments-and-promotion.md`, `66-immersive-artifact-canvas.md`, `38-voice-personality.md`, `49-live-voice.md`, and `69-shared-conversation-coworking.md` for their respective contracts. Read each document's `Status:` before treating it as shipped.

## The four screens to preserve

| Screen | Reference | Essential interaction |
|---|---|---|
| 1. Everyday result | [01-everyday-result.png](../assets/vak-experience-2026/01-everyday-result.png) | An Agent gives a concise outcome first; a result card exposes preview, checks, draft status, review and revision without filling the conversation with internal activity. |
| 2. Everyday plan | [02-everyday-plan.png](../assets/vak-experience-2026/02-everyday-plan.png) | A nontechnical plan is a useful, editable result with options and actions; voice and typing are both ready in the composer. This is a universal product, not a coding shell with a friendly theme. |
| 3. Review and accept | [03-review-and-accept.png](../assets/vak-experience-2026/03-review-and-accept.png) | Show the actual candidate, exact changes, observed checks, destination and remaining uncertainty. Accept selected changes into the workspace only after review; request changes or keep the draft. |
| 4. Conversation and canvas | [04-conversation-and-canvas.png](../assets/vak-experience-2026/04-conversation-and-canvas.png) | The Agent conversation and live artifact are visible together. The person can try, comment, ask for a revision, switch viewport, focus the canvas and move to review without losing context. |

These are generated visual proposals. Example names, dates, results, checks, venues, prices and code are illustrative, not app fixtures or claims about runtime evidence. Preserve the **relationships and interaction patterns**, not accidental text or fake proof. A later visual may replace an image only if it covers the same user decisions at least as clearly; record the replacement and why here. Do not silently revert to the old dense workbench or use the old `presentation-2026` mockups as the target.

## Visual and interaction rules

- Use a warm, quiet canvas, readable typography, gentle curves, sparse borders and deliberate contrast. Color and motion communicate state; they do not demand attention while idle.
- Give each result a clear headline, useful artifact, evidence and next action. Put operational detail one deliberate step away. Never invent a success metric or show a check as passed before observed evidence exists.
- Keep the Agent present through a distinct glyph/character, name and writing voice. Animation is contextual and respects reduced-motion settings. Personality is expressed in helpful behaviour, not decorative chatter.
- Keep the composer persistently available for speech and typing. Voice capture, transcription, listening, interruption and spoken responses must have legible states and parity with text work.
- Let the same result move through conversation, canvas, review and acceptance using stable identities. Inline feedback identifies what the user is referring to. Revision creates a new version; prior feedback remains attached to the version it addressed.
- Let a person open source, files, terminal, logs, checks and execution provenance when the task calls for them. Technical chrome is contextual rather than a permanent first impression.
- Support narrow and wide windows, keyboard access, focus management, contrast and screen readers. Avoid fixed card widths and modal-only paths for routine work.

## Implementation ledger

`[x]` means code exists in the current working tree; it does **not** mean the four screens match the visual references or that a release has shipped. Keep this section honest as work progresses.

### Work completed in this task so far

- [x] Four visual references preserved in this repository under `docs/assets/vak-experience-2026/`.
- [x] `AgentMark.tsx` adds five restrained SVG character marks, used in conversation and several Agent-facing surfaces. Idle marks are still; working marks animate subtly and respect reduced motion. This is an initial character vocabulary, not the full personality system.
- [x] The conversation exposes a `Review draft` entry point for sandbox artifacts.
- [x] Workbench has a candidate review dialog with actual draft/workspace file comparison, selection, inspected-file tracking, feedback to the Agent, keyboard focus handling and apply action.
- [x] Artifact Canvas has an in-context revision composer that steers the current Agent conversation about the open draft.
- [x] Candidate export now generates and stores a server-owned candidate ID/manifest. Promotion loads the saved candidate, applies only selected manifest paths, rejects changed candidate content and workspace conflicts, and records a receipt. The arbitrary browser `POST /sandbox/records` route was removed.
- [x] `69-shared-conversation-coworking.md` records the chosen same-conversation/same-draft multi-human contract and its security boundaries. It is a proposal, not a shipped collaboration feature.
- [x] Verification performed for the above slice: client typecheck and production build; nine focused server promotion tests; `git diff --check`. The presentation card harness loaded visually. These checks do not establish full end-to-end usability of the new screens.

### Next implementation work: the four screens

- [ ] Build Screen 1 as the actual everyday Agent conversation: Agent identity and presence, compact user turns, outcome-first response, one primary result, secondary evidence, review/preview/revise controls, and quiet progress. Drive it from `OutputTimeline` and stable result IDs, not example card data.
- [ ] Build Screen 2 with a real noncoding result journey. Prove the existing semantic renderer can present a plan, options and meaningful actions without a developer workbench. Preserve the card vocabulary and improve selection/composition rules rather than hardcoding a small set of domains.
- [ ] Make voice visibly primary in both screens, including listening, processing, interruption and fallback to text. Use the existing voice transport and personality contracts; verify actual speech and transcription flows.
- [ ] Build Screen 3 as a first-class review route/surface, with candidate versions, selected files, readable change summary, observed checks and provenance, explicit destination, exact acceptance scope, revision request and keep-draft action. Keep advanced file/code diff available inside it.
- [ ] Build Screen 4 as a synchronized split conversation and canvas: real artifact preview, contextual comments or selections, draft version, responsive viewport controls, focus/split modes and direct path to review. Preserve existing polyglot canvas formats.
- [ ] Connect all four through one result/conversation identity and coherent navigation, rather than separate modal islands.

### Platform wiring required for the experience

- [ ] Complete the task-environment contract in `54-task-environments-and-promotion.md`: true isolated task copies, immutable candidate freeze, candidate/result/preview/check binding, recoverable multi-file apply, destination conflict handling, scoped undo where possible, and post-apply integration verification. Current promotion is compare-before-write per file; it is not a crash-safe multi-file transaction.
- [ ] Give human participants distinct authenticated principals, conversation/audience-scoped invitations, revocation and read/write route guards. The current browser cookie is one operator identity; do not expose invitations through it.
- [ ] Add append-only, attributed human messages and comments anchored to result/candidate versions and optional artifact locations; add explicit conversion of feedback into Agent intervention. A participant must not inherit the owner's FullAccess, secrets or approval authority.
- [ ] Scope shared transcript, SSE, preview, candidate and artifact reads to the same audience grant. Keep review, acceptance and external publishing as separate decisions with actor and exact candidate receipts.
- [ ] Complete Agent character/personality settings and presentation across sidebar, conversation, voice, results, channels and compact surfaces. Character must remain identifiable without relying on animation or color alone.
- [ ] Audit all existing card/renderer types and presentation surfaces against the new design. Make common cards calm and legible; preserve specialist and technical capability behind contextual access.
- [ ] Verify real workflows: everyday question, noncoding plan, document/data result, coding change, live canvas revision, draft review/acceptance, two humans in one conversation, participant revocation, and narrow-screen/keyboard/voice operation.

## Completion bar

This work is complete only when the running desktop and web UI can demonstrate the four journeys with real data and runtime evidence, the same Agent conversation and draft can safely include an invited human, and technical capability remains reachable without dominating everyday use. Update this ledger with tested evidence and screenshots of the **running implementation** as each screen lands. A build passing or a generated mockup alone does not close a checkbox for a screen.
