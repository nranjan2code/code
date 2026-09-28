---
target: Deep review of Vak recipes, AST, renderers, delivery surfaces and preview
total_score: 18
max_score: 40
na_heuristics: 
p0_count: 0
p1_count: 5
timestamp: 2026-09-05T00-38-36Z
slug: rendering-system-preview-html
---
Method: dual-agent (A: renderer_ui_audit · B: surface_delivery_audit), with a separate code/AST audit and primary-agent source verification. The detector used a limited regex fallback; browser findings are reported separately.

# Vak rendering review — design direction is promising; the contract is not approval-ready

Yes, we can do substantially better. The important improvement is not thirteen prettier cards. It is one reliable semantic pipeline, a small composable renderer vocabulary, and a preview that actually executes the contract it asks us to approve.

The earlier preview was overstated as complete. It is a useful visual storyboard, not a full recipe/AST/renderer/delivery preview. Production code and the preview were not modified during this review.

## What is worth keeping

The warm, flat visual direction fits Vak's “Auditor’s Desk” identity. Weather's outcome-first anatomy, failure-first test presentation, and the approval scope summary are useful starting points. The proposed separation between catalogue, AST, and delivery inspection is also sound.

However, the design still feels like a component showroom rather than a continuous conversation. Recipe IDs, renderer badges, generic validation chips, module headings, and implementation commentary repeatedly compete with the answer. Product character should come from restrained typography, real evidence, useful interactions, and precise state—not extra framing.

## Five priority findings

### [P1] 1. The base AST can change the meaning and structure of ordinary Markdown

The tight-list reproduction is concrete:

`- Stocks include **Axis Bank**, **Adani Ports**, and **HDFC Bank**.`

The compiler produces one list item containing seven Paragraph blocks: the introduction, each bold name, and each punctuation/text fragment. A genuinely loose two-paragraph list item correctly produces two paragraphs, so flattening every list paragraph with CSS would erase valid structure.

The cause is recursive conversion of an Item's direct inline children, followed by a catch-all that creates a Paragraph for each child independently. The renderer then emits a separate `<p>` for each fragment. This explains the screenshot's isolated bold names and punctuation. The attempted CSS fix targets a `.semantic-document` wrapper that does not exist inside these list items. [Compiler](/Users/example/Projects/vakcoder/crates/vak-delivery/src/presentation.rs:655), [catch-all](/Users/example/Projects/vakcoder/crates/vak-delivery/src/presentation.rs:704), [renderer](/Users/example/Projects/vakcoder/crates/vak-client-ui/src/components/PresentationRenderer.tsx:147), [CSS](/Users/example/Projects/vakcoder/crates/vak-client-ui/src/styles.css:438).

A second semantic loss appears at three table rows: the renderer automatically promotes the table to DataGrid and flattens cells through `inlineToText`. Links use `label`, which this helper does not traverse, so link labels disappear rather than merely losing styling. Two-row and three-row versions of the same content therefore behave differently. [Table conversion](/Users/example/Projects/vakcoder/crates/vak-client-ui/src/components/PresentationRenderer.tsx:129).

Recommendation: group contiguous inline siblings into one paragraph, preserving explicit paragraphs and nested blocks. Keep ordinary tables quiet and semantic; a dataset's operations, not row count, should justify a grid. Regression-test both structures before introducing richer presentation. Design follow-up: Impeccable harden and typeset, after the compiler correction.

### [P1] 2. Recipe selection is a label, not a validated composition contract

`PresentationPlanner::plan` selects a recipe before candidate validation. Selection counts matching signals, with registration order resolving ties. All built-in surface mappings resolve to `builtin:generic`, and a mapping's presence is reported as Native even when it is generic. The recipe's `primary` vocabulary also includes names that do not match the structured registry. [Planner](/Users/example/Projects/vakcoder/crates/vak-delivery/src/skills.rs:64), [selection](/Users/example/Projects/vakcoder/crates/vak-delivery/src/skills.rs:123), [mappings](/Users/example/Projects/vakcoder/crates/vak-delivery/src/skills.rs:242).

The server projection supplies link previews as its structured candidates, chooses for hard-coded desktop capabilities, and records recipe metadata on compiled prose. Selecting `weather.forecast` does not create a typed forecast; selecting `data.multi_chart` does not establish usable chart data. [Projection](/Users/example/Projects/vakcoder/crates/vak-server/src/projection.rs:89).

The registry advertises twelve structured types, but validation recognizes only six. It checks required-key presence, not field types, nested shape, sizes, or supported output/skill versions. Thus six specialist types are rejected while malformed recognized payloads can pass. [Validation](/Users/example/Projects/vakcoder/crates/vak-delivery/src/skills.rs:651).

Recommendation: validate candidates first; choose a composition only when its required data and renderer capabilities exist; use explicit priorities and stable tie-breaks; record rejection reasons. A recipe should arrange references to reusable blocks, not classify an entire answer into one exclusive domain. Missing data must preserve the original answer. Design follow-up: Impeccable shape and clarify.

### [P1] 3. Rich presentation can invent evidence or hide content

The terminal recipe can turn a shell code fence into a console with `exit_code: 0`, even though a code example is not proof of execution. Disabling rich previews displays a settings notice instead of the structured item's fallback. Media preferences are checked in some structured paths but not all inline/block paths. [Terminal conversion](/Users/example/Projects/vakcoder/crates/vak-client-ui/src/components/PresentationRenderer.tsx:184), [rich preference](/Users/example/Projects/vakcoder/crates/vak-client-ui/src/components/PresentationRenderer.tsx:287), [block media](/Users/example/Projects/vakcoder/crates/vak-client-ui/src/components/PresentationRenderer.tsx:194).

The chart renderer assumes every series has `points`, fixes the minimum Y value at zero, distributes X positions by array index rather than actual X values, and substitutes zero for missing values in its crosshair. These are data-integrity problems, not styling preferences. Connecting more payloads before fixing the contract would activate latent failures. [Chart](/Users/example/Projects/vakcoder/crates/vak-client-ui/src/components/presentation/UniversalChart.tsx:41).

The presentation stream's TextDelta carries only a delta and ItemCompleted only identity/status, rather than the repository's required delta-and-snapshot contract. Stable IDs, partial output, and live-to-reloaded consistency need explicit tests before richer local interaction state is added. [Stream contract](/Users/example/Projects/vakcoder/crates/vak-delivery/src/presentation.rs:145).

Recommendation: separate payload validity, tool execution status, claim evidence, and delivery status. None should imply another. Keep faithful fallback accessible in disabled/unsupported/error states. Preserve missing values as missing and render data according to its real coordinates. Design follow-up: Impeccable harden and audit.

### [P1] 4. Delivery surfaces bypass different parts of the formatting contract

Telegram reads the packet's ordered chunks and sends them as HTML. Slack and Discord instead extract the gateway response's `text`, discard the delivery packet, and split that text again. The proactive Slack and Discord adapters use generic Markdown profiles rather than their distinct dialect profiles. This explains inconsistent delivery formatting even when a converter exists elsewhere. [Telegram](/Users/example/Projects/vakcoder/crates/vak-server/src/surfaces/telegram.rs:547), [Slack](/Users/example/Projects/vakcoder/crates/vak-server/src/surfaces/slack.rs:218), [Discord](/Users/example/Projects/vakcoder/crates/vak-server/src/surfaces/discord.rs:210), [proactive profiles](/Users/example/Projects/vakcoder/crates/vak-server/src/delivery.rs:712).

Recommendation: interactive replies and proactive sends must consume the same surface-projected packet. The transport sends; it does not reinterpret, append unbudgeted formatting, or independently re-chunk. Test escaping, balanced fences/tags, Unicode boundaries, action text, attachment degradation, ordered retries, and the treatment of already-sent chunks. Distinguish original Markdown from surface-projected text: byte-exact original source and recipient-readable degradation are two different outputs.

Design follow-up: Impeccable adapt for recipient presentation, with protocol-level tests outside the design layer.

### [P1] 5. The preview cannot yet serve as the implementation approval gate

Per-recipe Terminal, Telegram, Slack, and Discord tabs show the same abbreviated Markdown; only their labels change. Native HTML and fallback text are independently handwritten, and contain different information. “Validated,” version `2.0.0`, exact fallback, and recorded validation receipts are hard-coded claims rather than executed results. [Preview rendering](/Users/example/.codex/visualizations/2026/09/04/01a06d35-f15f-7183-9c00-9521ab0f03da/rendering-system-preview.html:150).

Browser checks confirmed that servings controls, the timer, and approval buttons do nothing. Recipe/surface selection drops focus to BODY; ArrowRight does not implement tab navigation. At 1280×720, hiding the rail leaves the fixed two-column grid intact, squeezing AST and delivery views into the 226px first column. [Layout](/Users/example/.codex/visualizations/2026/09/04/01a06d35-f15f-7183-9c00-9521ab0f03da/rendering-system-preview.html:45), [handlers](/Users/example/.codex/visualizations/2026/09/04/01a06d35-f15f-7183-9c00-9521ab0f03da/rendering-system-preview.html:175).

The preview's fourteen “AST” entries include TaskList, which is not a current DocumentBlock variant, yet omit the Mermaid/diagram behavior central to two supplied screenshots. The real inventory is thirteen block variants, ten inline variants, twelve structured types, nine timeline content variants, and seven statuses. An inventory is not evidence that every combination was executed.

Recommendation: retain the best visual anatomy but replace the mock engine with a fixture-driven inspection harness. Every sample must be clearly marked as fixture data. Proposed behavior and current behavior must be separately labeled. No fake validation, inert controls presented as implemented, or manually abbreviated “exact” fallback. Design follow-up: Impeccable shape, harden, and layout; polish only after behavior is trustworthy.

## A smaller, more coherent design for all thirteen recipes

| Current recipe | Recommended presentation |
|---|---|
| answer.basic | Unframed, excellent prose; lists and inline emphasis remain continuous. |
| answer.research | The same reading flow with precise claim-linked citations and a quiet source list. |
| research.synthesis | Shared evidence blocks organized around takeaways, disagreements, and limitations—not unsupported confidence chips. |
| weather.forecast | Location, observation time, condition and temperature first; short forecast only when supplied; secondary metrics quieter. |
| coding.change_summary | Outcome, actual file scope, and verification receipts; expandable detail rather than a dashboard. |
| coding.diff_inspector | File/hunk navigation, line identity, selection/copy, and appropriate unified/split behavior. |
| coding.test_report | Failure-first report; totals reconcile; skipped, failed, cancelled, and passed remain distinct. |
| terminal.session | A reusable execution block driven by a real tool receipt; preserve command, output, duration and actual exit state. |
| data.multi_chart | Proper coordinate scales, missing-data gaps, units, keyboard/touch inspection, and an equivalent data table. |
| data.spreadsheet_grid | A real dataset workspace only when needed; ordinary explanatory tables retain their natural reading form. |
| lifestyle.culinary_recipe | Ingredients and steps dominate; servings scale consistently; timers have real local lifecycle state. |
| workflow.approval | A runtime interrupt, not a recipe inferred from prose; exact scope plus pending/resolved/expired states. |
| artifact.collection | Compact, genuinely addressable files with type/size/availability; previews only when useful. |

Diagrams are an explicit missing capability, not a solved recipe. The next preview must show a valid diagram, malformed source, unsupported/safe fallback, and copyable original source. Do not infer diagram correctness from a Mermaid language label.

## Recommended architecture

One canonical path should branch only at presentation:

```text
Append-only ledger + live events + original source
                    ↓
Canonical semantic document + provenance
                    ↓
Validated composition plan referencing stable blocks
                    ↓
        ┌───────────┴───────────┐
        Native renderer         Surface projector
        Continuous chat         Ordered packet → transport
```

Recipes remain data-driven compositions over a small closed vocabulary. They do not mount arbitrary HTML or create thirteen independent apps. One answer may contain prose, a chart, sources, and files without one whole-turn recipe displacing everything else. Placement and source coverage must prevent duplication or silent omission.

Original Markdown is immutable and byte-exact. Derived plain text, Slack text, Telegram HTML, and other projections are explicitly named derived forms with truthful degradation receipts. Validation happens before rendering; failure is a visible value with a readable fallback.

Use schema-v2-compatible additive fields/envelopes, not a casual enum replacement. Verify unknown-field preservation, version refusal, and ledger replay. Streaming must carry delta and snapshot, preserve partial output on abort, and retain stable identities across completion and reconnect. Renderer state such as selection, expanded failures, and timers must not be accidentally reset by re-keying the whole document.

## What the next full preview must prove before implementation

Use the same serialized fixture to show: source/tool payload → AST and selection diagnostics → actual native rendering → actual surface packet. Present both the recipient-readable result and raw payload/chunk boundaries. Do not claim pixel-perfect third-party app rendering without an actual recipient check.

Cover every registered recipe, every block and inline form, every structured renderer, and representative timeline/status transitions. Use a coverage matrix instead of an unnecessary Cartesian explosion. Include:

- The five screenshot regressions: weather routing, split bold list content, both Mermaid cases, ordinary table versus operational grid.
- Rich previews off, external media off, empty and malformed payloads, unknown versions, and renderer failure.
- Negative and missing chart values, irregular timestamps, unequal series, long labels, and CSV escaping.
- Streaming, abort, reconnect, stale results, pending/denied/expired/resolved approval, and unavailable artifacts.
- Long code, Unicode, nested formatting and multi-chunk delivery, with deterministic degradation.
- Real chat placement with a composer, long history and scroll anchoring; keyboard and screen-reader semantics; dark/light themes, narrow screens and zoom.

Interaction in the approval preview must be isolated simulation, never a production action. Preserve inspection as the normal review path; keep developer diagnostics behind a deliberate disclosure in the end-user chat.

## Design health of the current preview

These are qualitative review scores for this artifact, not a measured product-wide quality score.

| Heuristic | Score /4 | Main gap |
|---|---:|---|
| System status | 2 | Fixed validation labels; missing lifecycle states. |
| Real-world language | 2 | Internal renderer vocabulary and unsupported evidence claims. |
| User control | 1 | Main demonstration actions are inert. |
| Consistency | 2 | Coherent appearance; inconsistent tabs, focus and tokens. |
| Error prevention | 1 | Invalid, stale and disabled paths are not exercised. |
| Recognition | 3 | Visible catalogue, but thirteen ungrouped choices. |
| Efficiency | 1 | No useful comparison, keyboard flow or filtering. |
| Aesthetic restraint | 3 | Calm direction, with excessive surrounding metadata. |
| Error recovery | 1 | No demonstrated recovery while preserving content. |
| Help and inspection | 2 | Good receipt concept, but mostly asserted rather than inspectable. |
| Total | 18/40 | Visually promising; not approval-ready. |

The cognitive-load problem is not that technical readers cannot understand recipe names. It is repeated competition between answer, catalogue, surface selection, and implementation claims, with comparison requiring memory across tabs. Group the catalogue and co-locate current/proposed projections. The emotional journey starts calmly, then breaks trust when controls do nothing or “Validated” is not supported.

For a power user, dead export/timer controls and missing comparison waste time. For a keyboard or screen-reader user, replaced focus and incomplete tab/panel relationships break the main inspection flow. For a cautious operator, a green validation chip beside an approval or failed test confuses schema validity with action safety and observed evidence.

## Deterministic and browser evidence

The detector returned two side-border warnings in the fragment, repeated in its standalone wrapper. They are low-priority contextual findings: the quote/approval accents have semantic purpose, so these are not evidence for indiscriminately removing borders. The detector lacked HTML/CSS parser dependencies and used regex fallback; it did not compute contrast or evaluate selectors.

Independent browser checks found dark-theme contrast ratios of 3.02:1 for a selected small recipe ID, 3.74:1 for the count, and 3.62:1 for the Preview only chip. Controls measured roughly 33–34px high. The default browser focus outline remains visible, but DOM replacement still loses focus. The chart fixture has an accessible label/title/description and the spreadsheet fixture uses scoped column headers; these are useful details to retain.

The proposed 1024/736/360 responsive sequence did not complete in this pass. No claim of comprehensive mobile, screen-reader, or live third-party delivery verification is made.

## Decision

Keep the restrained visual direction. Do not approve the existing storyboard as the full implementation specification. First make the preview truthful, compositional, stateful and driven by actual contracts; then use it to agree the design. Production fixes should follow that approval gate, with semantic integrity and delivery correctness ahead of decorative refinement.
