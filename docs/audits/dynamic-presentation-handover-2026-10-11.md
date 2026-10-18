# Vakyartha dynamic presentation — continuation prompt

Status: historical. A dated record, kept as written on 2026-10-11.

Continue the active goal in `/Users/nisheethranjan/Projects/Vak/code`. Read this handover, `AGENTS.md`, and `docs/audits/presentation-registry-and-live-coverage-2026-10-10.md` before working. Preserve the dirty worktree: it contains many changes from earlier work and the maintainer. Do not reset files or assume every diff belongs to this latest change.

## Problem and intended result

Make the dynamic presentation pipeline reliable from a natural user request through intent, relevant context/recall, tool discovery, provider request, schema admission, execution, ledger, projection, rendering and final delivery. The catalogue has 88 starter presentation packs, 109 built-in semantic types and 12 generic emitter shapes. These are different layers, not a limit of 12 cards per answer. A simple answer may need one presentation; a complex task may need several distinct fitting presentations.

The user wants normal people to ask naturally, without naming cards or card counts. They want appropriate presentation selection, readable sizes, usable interactions, proper animation without flicker, faithful content, complete answers and intelligent recovery. They do not accept harness coverage as proof of real model selection or a rendered acknowledgement as proof of UI quality.

Use clean, provider-neutral mechanisms. Ollama is the chosen test route, not a privileged provider in the design. Do not add Qwen branches, prompt-specific mappings, arbitrary headings such as `Note:`, generic trailing-text suppression, or a new supplemental-output tool to hide failures. The user explicitly rejected those shortcuts. Diagnose upstream causes before changing prompts. Preserve the existing Agent-owned N-turn system, context management, ledger, search and recall. Do not resend full history or fill context merely because space exists. Long-distance follow-ups must remain recallable.

## Live environment and ownership

- Existing Codex browser tab: `http://127.0.0.1:8901/app#/s/01a12500-dbf0-7ec3-a5d8-078bd439258a`. Reuse it. Do not open another app tab or create another conversation/Agent.
- Conversation: `ses_01a12500-dbf0-7ec3-a5d8-078bd439258a`; Agent: `agt_00000000-0000-7fab-ac45-d05723223536`.
- Shared core is tested through the headless web app. Do not launch the desktop app.
- Server command: `target/debug/vak -C /Users/nisheethranjan/vak-home serve --port 8901`.
- Last observed server PID: `92262`, process identity `prc_01a12871-372e-7e53-a1a4-a2f54d7aac92`. Recheck rather than relying on these transient identifiers.
- Health reported 7.0.1, healthy, provider `ollama`, model `qwen3.5:4b-mlx`, project config source, zero provider failures.
- Data home: `/Users/nisheethranjan/Library/Application Support/vak`; Agent session home is its `agents/vak` subtree. The working directory is `/Users/nisheethranjan/vak-home`.
- Logs: `/Users/nisheethranjan/Library/Logs/vak/vak-server.jsonl`.
- Keep durable running work alive across browser close/reopen. Do not restart the server while a run is active merely to deploy UI changes. Browser sessions can expire after restart by design; use the normal sign-in flow in the existing tab when needed.

## Verified work carried forward

Earlier investigation and tests are documented in the coverage audit. Important repairs include:

1. Recipe packs expected objects while the emitter supplies string arrays. Generic binding repair now preserves actual ingredient/step text. A rebuilt real Ollama recipe run rendered correctly.
2. Unrelated requests inherited a stale garden handoff summary. Summarization now uses the working-set plan, and reset summary projection is limited to its open originating turn. A later unrelated reading-circle run used the correct current facts and emitted schedule, budget and checklist presentations.
3. Card-complete empty-text guidance conflicted with preserving explicitly requested wording. The conflicting prompt/tool acknowledgements were aligned. Live adherence is still imperfect; do not claim this solved completion narration.
4. Discovery was structurally gated by request vocabulary: initial schema prediction and presentation recovery used the same recipes. A generic structured-draft recovery now exposes all admitted shape schemas when substantial parsed output has no matched request route. It excludes short prose, code-only structures and explicit text-only requests. The reconsideration is optional when no turn capacity remains and no longer counts a heuristic miss as a model-capacity failure.
5. Recovery guidance now explains that sections, headings, steps and bullets describe organization; they do not automatically request text-only/Markdown output. The regression and live recovery reach a card, though shape quality remains poor.
6. Following Astra review, the shared tool surface now loads **every admitted presentation schema from the first model call**, on every provider. Removed the `predicted_cards` parameter from `build_tool_surface`. Request prediction remains in `complete_output_contract` for output demand/composition; it does not gate schema access. Non-presentation domain deferral, permission admission and historical content selection remain independent. Removed redundant card entries and obsolete field-list plumbing from the `More tools` catalogue.

The 12 names/descriptions/input schemas total 17,861 serialized bytes. This is a payload-size measurement, not measured tokens or proof of cheaper inference. Measure live call count, input/output usage and latency rather than assuming fewer discovery calls guarantee better answers.

## Current edits and validation

Latest shared-schema changes are in:

- `crates/vak-core/src/capability/surface.rs`
- `crates/vak-core/src/lib.rs`
- `crates/vak-core/src/presentation_tools.rs`
- `crates/vak-core/src/intent.rs`
- `crates/vak-core/tests/intent_projection.rs`
- `docs/design/68-context-engine.md`, `docs/design/41-capability-registry.md`, `AGENTS.md`

Related earlier changes in `vak-agent`, delivery, presentation seeds, context/session and UI are documented in the coverage audit; preserve them while reviewing their ownership and correctness.

Passed focused checks: six tool-surface tests; all 109 semantic types map to exactly one loaded emitter under an empty domain prediction; nine intent-projection tests; small talk remains non-structured despite available card schemas; eight agent presentation-check tests; five tool-refusal tests. Earlier delivery/all-pack/full-projection checks are recorded in the audit. `cargo build -p vak --bin vak` passes, with the existing linker unwind-size warning. Targeted `git diff --check` passes. No full-suite success is claimed.

A separate startup UI fix has been written and built but **is not yet deployed in the running process**:

- `App.tsx`: distinguish unknown initial backend state from a confirmed empty/unready install; render an opening status until the latest initial probe resolves.
- `components/WorkspaceGate.tsx`: unknown authentication status renders a checking status rather than the onboarding welcome screen.
- `npm run build:web` passed TypeScript checking and the web production build; the subsequent Rust binary build passed.

This addresses an observed welcome-screen flash during rehydration without disabling animations. It still needs live reload/reconnect acceptance after the active run settles. The latest binary on disk includes this UI build; the current process was started before it.

## Recent real runs and unresolved failures

All are in the original Agent conversation, using local Ollama/Qwen.

- Seed-swap guide, no recipe match: Qwen interpreted “a clear format I can share” as a saved file and used `write`, creating `neighborhood_seed_swap_guide.md`. File delivery bypasses later card repair. This is an output-channel ambiguity, not successful card coverage. Keep the artifact.
- Balcony herb guide asking for sections/bullets in chat: initial live run returned Markdown after two dispatches, about 160 seconds. Qwen reasoned that sections and bullets meant prose. After the guidance correction, it attempted a table, received unequal-column error, corrected it and emitted a native table. That recovery run took 234 seconds across five dispatches (`run_01a12868-a1a1-73f2-a609-eab4cc9d7b1d`). The result contains long paragraphs in cells and exposes completeness-check prose. Native rendering is verified; appropriate shape, concision and content accuracy are not.
- Screenshot of that visible defect: `docs/audits/evidence/presentation-recovery-prose-2026-10-11.jpg`.

The **latest pantry run** is the first natural multi-part test after deploying initial access to all schemas. It settled with run status `completed` at 2026-10-11 00:57:49 UTC, but the application shows **Partial result**, label-only tables and recall-error prose. It consumed 37 model dispatches over 373.892 seconds. Run status means the run ended; it does not establish a successful answer. Screenshot: `docs/audits/evidence/pantry-partial-result-2026-10-11.jpg`.

`run_01a12871-98b8-7112-859c-561174faa4c9`, turn `trn_01a12871-98b7-7c32-9941-dacc0200f41a`, opened 2026-10-11 00:51:35 UTC.

Exact request:

> I want to make our shared pantry easier to use. We keep rice and pasta on the upper shelf, tins on the middle shelf, and snacks on the lower shelf. We restock on Saturdays and check expiry dates on the first Sunday of each month. Give us a quick routine everyone can follow, and compare keeping these shelves with arranging foods by meal. Keep the answer short and show it here.

Ledger evidence so far:

1. First reasoning correctly identified routine plus comparison, but `emit_table_card` sent `rows` as a Markdown string despite an array schema. Schema admission rejected it.
2. Later accepted calls used arrays containing only `Shelves`, `Category-Based Organization`, `Meal-Based Organization`, `Best Choice` (and a similar variant). These are technically valid one-column tables but do not answer the comparison. A valid schema is not semantic completeness.
3. It repeatedly called `recall` with `{}` after exact invalid-argument feedback. It also searched a freshly minted commitment and fetched it, although all required pantry facts were in the current request.
4. Its reasoning mistook the runtime note `Part 3 uses the result of part 2. Part 4 uses the result of part 3.` for a user reference to an earlier workflow. This is an upstream control-note/intent interaction, not a renderer bug.

Do not claim initial schema access solved this run. Examine its final ledger and recorded context/control traffic. It has now settled, so the prepared startup UI build can be deployed when continuing; first recheck that no newer run is active.

## Most useful next investigations

1. **Malformed recall bypasses recovery.** In `vak-agent/src/lib.rs`, around the `failed_correctable` classification, every `recall` error is unconditionally excluded from the repair controller. This treats malformed `{}` calls like unavailable optional history and permits repetition. `execute_batch` appends `[optional-history]` to every recall failure. Distinguish invalid request shape from a well-formed lookup that finds no history, using typed/pure argument validation rather than new error-text sniffing. Preserve optional-history behavior for legitimate misses. No fix to this path has been made in this latest turn.
2. **Relationship notes lose their scope.** `vak-intent/src/resolve.rs::strand_note` emits numbered dependency sentences. Relation construction treats same-message deixis as dependency on the previous strand. The internal relation can be correct while its model-facing note falsely implies earlier work or generated results. Inspect the pantry intent/context plan before changing segmentation, lineage or recall. Preserve genuine compound workflows and distant follow-ups; avoid another keyword fix for pantry/garden examples.
3. **Completion-check prose leaks.** The ledger confirms this is model final content, not prose invented by projection. Inspect the exact assembled instruction and native provider response fields. Keep legitimate user-requested prose and exact wording. Do not implement blanket hiding of final text, prefix filters, or a new supplemental-output tool.
4. **Semantic completeness/shape quality.** Investigate why label-only tables pass while requested routine/comparison outcomes remain unanswered. Do not reject valid one-column tables globally or hardcode known bad headers. Validate coverage with broad contrasting tasks, and review the existing outcome/stop-policy design before changes.
5. **Deploy and accept the startup UI fix** after verifying no durable run is active. Reload the same tab, verify original history, run state and cards restore, and capture opening state without the onboarding flash.
6. **Continue broad coverage.** Use natural tasks spanning each of the 12 shapes, then each registered semantic type/pack and real renderer interactions. Evidence-bound shapes need real executed commands/tests/changes or retrieved sources. Include simple, multiple distinct outputs, complex dashboards, same-topic/distant follow-ups, unrelated-topic switches, repairs, reconnect, narrow/wide readability and animation. Synthetic fixtures prove registration/payload/render contracts; real turns prove selection and user experience. Neither substitutes for the other.

Read design status lines before treating proposals as shipped. Start from docs 64, 68, both full `30-*` filenames, 57, 67 and 15 for ownership, context, output, packs and failure handling. Keep documentation synchronized with actual behavior and mark pending live checks honestly.

## Continuation operations and resource use

The user explicitly deleted the old Codex goal and requested a fresh one. A new ACTIVE goal now covers this complete pipeline, all packs/types, real local runs, UI quality, recovery/context/durable-run safeguards and durable documentation. No token budget was set. Do not mark it complete until the objective is actually achieved.

The user has other Codex logins and wants effective token use, not reduced quality. Last observed Plus usage was 58% of the five-hour window and 32% weekly. Recheck when relevant. Use focused Astra/Sol reviews for consequential design changes, with compact context; existing Astra review found no concrete regression in the initial-schema change. All application inference remains on the configured local Ollama route. Keep provider support generic.

Useful read-only commands:

```sh
target/debug/vak -C /Users/nisheethranjan/vak-home runs show run_01a12871-98b8-7112-859c-561174faa4c9
target/debug/vak -C /Users/nisheethranjan/vak-home export 01a12500-dbf0-7ec3-a5d8-078bd439258a
curl -fsS http://127.0.0.1:8901/health
```

Use the session ledgers, context plans, control records and model-call receipts to reconstruct what was sent. Never expose credentials or dump unrelated history merely to inspect one failing turn. Continue from evidence; do not replay completed tests or start another conversation to make results cleaner.
