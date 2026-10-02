# 57 — Adaptive presentation runtime and reusable experience packs

Status: **implementation in progress; P1/P4 library, durable-store, session-audit, mode-toggle, bounded-revision, plugin-pack, pack-lifecycle, plugin-lifecycle-revocation, server-control, client-control, activation-control, revision-control, lifecycle-test, visible-management, delivery-integration, generic feedback-audit, visible-feedback, primitive-introspection, scoped-precedence, pack-portability, pack-management, result-scoped-proposal, original-fallback, declarative-seed-pack, everyday-theme-pack, semantic-selection, reset, cross-surface audit, generic constrained-surface lowering, live projection library-awareness, native adaptive output-contract, selected-tree emission, domain-neutral shape-inference, canonical renderer-envelope, cross-surface packet-validation, bounded proposal-chain, light-theme chrome polish, and fail-closed fallback regression slices landed**.

Current implementation: `crates/vak-presentation` provides the bounded
declarative spec, JSON-path binding validator, deterministic compiler,
content digest, coverage accounting, semantic-type matching, and explicit
fallback path. The crate is also re-exported by `vak-delivery`; selected
session results now emit the native adaptive output alongside the preserved
document fallback.

The surface audit found no second card-selection contract in the messaging
adapters. Desktop/browser/JSON use the semantic delivery packet; Telegram,
Slack, Discord, plain text, and terminal lower that same packet through their
existing markup/chunk converters. The legacy structured-fence projector and
provider-shape adapters remain compatibility boundaries: they validate typed
data, never mutate the session ledger, and always retain the exact Markdown
fallback. New experience packs therefore do not add channel-specific branches;
future surfaces plug in by declaring capabilities and a lowering adapter.

The delivery crate now also exposes one generic primitive-to-Markdown lowering
function for constrained surfaces. It handles the closed primitive vocabulary
without semantic/domain branches and returns the compiler's exact fallback for
degraded presentations; the two adaptive projection tests cover both paths.

`vak-store::presentation::PresentationStore` now persists the reusable library
outside append-only session ledgers with atomic replacement and safe empty-store
initialization. The delivery adapter can select and compile an active scoped
definition while retaining the original fallback text.

`vak-session` now records typed append-only presentation selection and feedback
activities. These records are projection-neutral and excluded from model
context; callers append a normal message as well whenever feedback becomes
model-visible.

The client now has one persisted Everyday/Advanced presentation-mode toggle,
defaulting to Everyday. It is a presentation preference only: switching it does
not rotate sessions, alter permissions, or change voice behavior.

The visual theme pack now includes two everyday palettes, Quiet Sage and Soft
Paper, alongside the existing technical palettes. They are host-owned token
sets: renderer behavior, permissions, voice, and presentation data remain
unchanged when a theme changes.

The host exposes a declarative built-in starter pack of 88 disabled definitions:
45 everyday layouts, 10 coding-flow layouts, and 20 universal interaction
shapes (`EVERYDAY`/`CODING`/`UNIVERSAL` in `crates/vak-presentation/src/seeds.rs`;
the count is pinned by `seed_pack_is_rich_disabled_and_validated_by_host_types`).
They use only the bounded primitive vocabulary and the same fallback/compiler
path as installed packs; they are seed content, not renderer-specific branches.

`vak-presentation::propose_revision` now validates user-guided candidates,
preserves identity, assigns the next immutable revision, computes its digest,
and caps retries at two attempts. Invalid candidates never replace the active
definition.

`PresentationLibrary::register_revision` adds the validated candidate as a
disabled preview; activation is a separate explicit operation, so previewing a
revision cannot silently change future selection.

`vak-plugin::PackageInspection::presentation_specs` now inspects declared JSON
presentation files through the bounded runtime validator. It returns inert,
validated specs only; plugin-authored code is never executed during inspection.
`presentation_records` converts those specs into disabled, workspace-scoped
library records carrying plugin and generation provenance; activation remains an
explicit host operation.

`vak-store::PresentationStore::register_pack` atomically registers inspected
records, while `revoke_plugin` removes that plugin's current definitions and
activations. Historical session selection receipts remain untouched.

The server now exposes `GET/POST /presentations` and
`DELETE /presentations/plugins/{plugin_id}` over the same workspace state, so
pack registration and revocation can be driven by the existing control plane.

The client API now exposes matching authenticated helpers for listing,
registering, and revoking presentation packs; they use the existing bearer or
cookie transport rather than a second credential path.

Settings also reports the current workspace's validated presentation-definition
and activation counts, making pack state visible without exposing raw plugin
implementation details.

The control plane now supports explicit scope/owner-bound activation and
deactivation endpoints. Registration remains preview-only until activation is
requested.

`POST /presentations/revisions` now validates and persists a disabled immutable
revision preview through `register_revision`; activation remains a separate
explicit request.

The library test matrix now covers plugin revocation removing both definitions
and activation pointers, while preserving the append-only history boundary.

The workspace verification pass also fixed the stale health-report shape test
for the already-shipped local TTS check; the focused health test and workspace
check are green.

The complete `cargo test --workspace --lib` suite now passes, including the
voice, server, delivery, plugin, store, and presentation crates.

The durable-store integration test also caught and fixed a serialization bug:
the in-memory tuple-keyed revision map is now written as a stable
`definitions`/`activations` JSON envelope, so non-empty pack registration
round-trips correctly.

That envelope is schema-versioned and refuses unsupported versions rather than
partially reading unknown durable state.

Atomic store writes now use a process-and-time-unique temporary path, avoiding
same-process concurrent-save collisions before the final rename.

Settings now lists each validated definition with its revision and provenance,
and offers explicit workspace Activate/Deactivate controls backed by the
scope-bound server endpoints.

Structured result cards also expose generic `Use this` and `Suggest a change`
controls. They submit arbitrary feedback through the authenticated session
endpoint; no result domain or scenario is encoded in the client. Feedback is
bounded, recorded as typed activity, and does not alter result truth.
The exact fallback projection remains in result data and inspection surfaces.
The everyday conversation shows the useful card without a `Show original`
debug control.

The delivery adapter now has an integration test proving unactivated previews
are not selected and activated specs compile into a bounded rich render tree.

Selection now resolves explicit user intent before workspace intent through a
deterministic fallback method. Scope layers remain separate; the selector never
copies effective values into a narrower layer.

Presentation packs can now be exported as a schema-versioned JSON projection and
imported atomically into the local library. Imported definitions remain disabled
previews; activation intent is never silently granted by import.
The store also defensively clears enabled bits on every pack registration, so
the preview-only invariant holds even if an external pack was tampered with.

The implemented control-plane surface is now:

- `GET/POST /presentations` for current definitions and preview registration;
- `GET /presentations/primitives` and `GET /presentations/specs/:id/:revision` for the host vocabulary and reviewable immutable definition;
- `GET /presentations/export` and `POST /presentations/import` for deterministic definition-pack portability; imports remain disabled previews until explicit activation;
- `POST /presentations/revisions` for bounded immutable revision previews;
- `POST /sessions/:id/presentation/proposals` for result-scoped, audited immutable previews;
- `POST /sessions/:id/presentation/feedback` for arbitrary choice/feedback activity, without scenario- or domain-specific assumptions;
- `POST /sessions/:id/presentation/select` for audited `use_once` selection or explicit user/workspace remembering;
- `POST /presentations/:id/:revision/activate` and
  `POST /presentations/:id/deactivate` for explicit scoped selection;
- `POST /presentations/:id/reset` to restore revision 1 (or clear activation) without deleting history; and
- `DELETE /presentations/plugins/:plugin_id` for plugin revocation.

All of these operate on the replaceable presentation projection. They do not
rewrite session ledgers, change permission or capability state, or alter the
voice pipeline; selection and feedback remain append-only session activities.
The selection endpoint also accepts a semantic type for legacy structured
outputs and resolves the active immutable definition server-side, so a card can
select the exact current revision without requiring a schema change in older
result envelopes.

P1/P4 verification is green (`cargo test -p vak-presentation`, `cargo test
-p vak-store --lib`, workspace
check, and strict library Clippy). The complete workspace library suite is also
green, including the existing voice, server, delivery, plugin, store, and
session tests. The first P3-style generic scenario is also now
landed as the validated `plan.timeline` semantic type and desktop/browser
timeline card; it retains the existing exact Markdown/channel fallback.

All 88 built-in definitions are registered and selected by default in every
build. They bind the complete validated shape of their emit tool, including
collection children where the renderer needs them. Explicit user and workspace
activations take precedence. Deactivation records a durable scoped suppression
so the default cannot silently reactivate a pack; activating it clears that
suppression. A failed compilation leaves the existing structured card in place.
The same effective library is loaded when a historical session is reopened.
Conformance tests exercise selection and compilation for all 88 definitions
against emitted payload shapes. Real browser review has covered timeline,
table, research, and recipe; the remaining visual combinations still need
human inspection. Packs can be searched, activated, deactivated, imported,
and exported in Settings. Import remains a disabled preview until activation.

This document is the implementation contract for replacing Vak's finite set of
hard-coded rich-result cards with a safe, adaptive presentation runtime. The
runtime may compose, preview, revise, store, reuse, export, import, and deliver
presentations for open-ended coding and non-coding work without executing
model-authored UI code or weakening any existing permission, evidence, session,
plugin, or delivery guarantee.

The implementation is greenfield at the presentation-composition layer. It is
not a rewrite of the agent, ledger, outcome, plugin, or delivery systems. Those
systems are the boundaries the new runtime must preserve.

---

## 1. Goal

One runtime should support an unbounded set of useful result experiences:

- a trip itinerary, meal plan, lesson, comparison, checklist, schedule, budget,
  meeting follow-up, shopping decision, or household project;
- a diff, test report, incident timeline, deployment receipt, terminal session,
  benchmark, dataset, dashboard, or architecture review;
- domain-specific results introduced by an installed extension pack;
- a one-off layout generated for the current result and corrected by the user;
- a layout remembered for one user or workspace and reused later;
- a portable, reviewable pack that can be shared without leaking result data.

The product promise is:

> Show the result in the clearest safe form available. Let the user improve that
> form in plain language. Remember only when asked. Preserve the exact result and
> a faithful fallback everywhere.

Success does not mean that a model can generate arbitrary React. Success means
that a model can describe a presentation using a bounded vocabulary whose
interpreter, actions, accessibility, provenance, and degradation are owned by
Vak.

---

## 2. Existing functionality that must survive

The implementation must retain, not emulate and later remove, all of the
following currently shipped behavior:

1. `vak-delivery` schema-v2 `OutputTimeline`, `OutputItem`,
   `PresentationDocument`, exact Markdown fallback, result-scoped outcome data,
   evidence references, diagnostics, and reconnectable delta plus snapshot.
2. The closed Markdown AST, inert raw HTML, safe URL policy, typed approvals,
   artifacts, recovery states, and outcome review actions.
3. Native renderers for research synthesis, coding diffs, test reports,
   terminal views, charts, data grids, culinary recipes, UI previews, links,
   metrics, and media.
4. Outcome/Balanced/Audit transcript density, external-media controls, reduced
   motion, themes, font scaling, and accessibility fallbacks.
5. The isolated delivery worker, trusted templates, deterministic channel
   projection, ordered chunks, semantic webhook envelope, durable outbox, and
   Telegram/Slack/Discord adapters.
6. Plugin inspection, content-addressed generations, install-disabled default,
   explicit activation, rollback, revocation, provenance, scope inheritance,
   and no-restart capability reconciliation.
7. Append-only sessions and the rule that every model-visible input is
   reconstructable through `derive_messages()`.
8. Permission-before-dispatch, brokered tools, sandboxing, capability overlays,
   frozen turn authority, result evidence, and runtime-owned satisfaction.

No phase may temporarily route existing typed outputs through a less capable
path. Compatibility is additive within the current major version.

---

## 3. Product model: one application, two disclosure modes

Vak has one conversation and one result history. `Everyday` and `Advanced` are
projections over that same state, not separate products, sessions, ledgers, or
rendering contracts.

### Everyday — default

- plain-language navigation and status;
- outcome-first conversation;
- presentations optimized for comprehension and action;
- files, notes, next steps, reminders, and saved conversations;
- security stated plainly, for example “Private on this device” and “Vak needs
  your approval before changing this file”;
- technical metadata hidden behind `View details`, never deleted.

### Advanced

- workspace and task hierarchy;
- Activity, Changes, Terminal, artifacts, receipts, route/model, sandbox,
  permissions, evidence, and diagnostics;
- the same result card, with audit and provenance affordances exposed;
- no change to capability, authority, or underlying session.

The mode is a user-interface preference. It must never select tools, change the
model route, weaken an approval, alter evidence requirements, or modify result
data. Switching is immediate and does not rotate a session. The existing
Outcome/Balanced/Audit density remains an orthogonal detail control inside
Advanced; Everyday uses an outcome-first projection with mandatory gates and
failures still visible.

---

## 4. Core decision: declarative presentations, never generated application code

There are two explicitly different products:

### 4.1 Adaptive card

A card is a `PresentationSpec` interpreted by Vak using a closed set of trusted
primitives. A model may propose or revise the spec. The runtime validates,
compiles, records, and renders it. This is the default for ordinary results.

### 4.2 Interactive artifact

A generated website, React application, notebook, or other executable output is
an artifact. It uses the existing sandboxed `ui.preview`/artifact path, carries
stronger isolation and promotion rules, and is never silently converted into a
native card.

An adaptive card may contain approved actions, but an action is a typed intent
that re-enters the host's normal permission and approval boundary. A card never
executes code, calls a tool, reads a file, fetches a URL, or accesses a secret by
itself.

---

## 5. Canon contract

The canonical result remains `OutputItem` plus its exact fallback. Presentation
is a versioned projection attached by reference.

```rust
pub struct PresentationRef {
    pub spec_id: String,
    pub revision: u64,
    pub digest: String,
    pub selection: SelectionReceipt,
}

pub struct SelectionReceipt {
    pub semantic_type: String,
    pub recipe_id: Option<String>,
    pub recipe_version: Option<String>,
    pub source: PresentationSource,
    pub capability_epoch: u64,
    pub surface: String,
    pub mode: ExperienceMode,
    pub reasons: Vec<String>,
    pub fallback_reason: Option<String>,
}

pub enum PresentationSource {
    BuiltIn,
    Plugin { plugin_id: String, generation: String },
    UserLibrary,
    WorkspaceLibrary,
    OneOffProposal,
}

pub enum ExperienceMode {
    Everyday,
    Advanced,
}
```

The exact field names may change during implementation, but the following
properties may not:

- result data and presentation definition have separate identities;
- the selected definition is immutable and content-addressed;
- the session records the exact revision and selection receipt;
- re-rendering never changes the recorded result;
- selection never upgrades completion or evidence;
- absent, revoked, invalid, or unsupported definitions fall back visibly;
- a presentation reference is additive to schema v2, not a reinterpretation of
  an existing field.

### Model-visible logging

If a model sees a card definition, renderer diagnostic, preview screenshot,
user correction, or stored preference, the session must contain a typed entry
from which that input can be reconstructed. At minimum add additive entry types
for:

- `PresentationProposed`;
- `PresentationFeedback`;
- `PresentationRevisionSelected`;
- `PresentationPreferenceRequested`.

Library activation and sharing receipts belong in their own append-only
capability/audit stores. No session entry is rewritten when a definition is
revised or revoked.

---

## 6. Declarative specification

`PresentationSpec` is data with a strict schema, size limits, bounded nesting,
stable identifiers, and no executable or ambient behavior.

```json
{
  "schema_version": 1,
  "id": "travel.weekend-itinerary",
  "revision": 1,
  "accepts": ["travel.itinerary"],
  "root": {
    "primitive": "timeline",
    "title": { "bind": "$.destination" },
    "items": { "bind": "$.days" },
    "item_title": { "bind": "$.label" },
    "item_body": { "bind": "$.activities" }
  },
  "fallback": { "kind": "document" },
  "accessibility": {
    "summary": { "bind": "$.accessible_summary" }
  }
}
```

Bindings use a deliberately small path language: object keys, bounded array
iteration, literals, formatting functions from a host registry, and no general
expressions. Missing values use explicit empty/fallback behavior. Binding
evaluation is pure, deterministic, recursion-limited, allocation-limited, and
network-free.

### Primitive vocabulary

The vocabulary must cover both general and technical work. It is the
`Primitive` enum in `crates/vak-presentation/src/lib.rs` — this table tracks
that enum and nothing else; `vak-delivery/src/adaptive.rs` matches it
exhaustively, so a variant added here without a lowering arm fails the build.

| Group | Primitives |
|---|---|
| Structure | `stack`, `row`, `group`, `section`, `divider` |
| Content | `title`, `text`, `rich_text`, `label`, `badge`, `callout`, `quote` |
| Collections | `list`, `checklist`, `timeline`, `steps`, `key_value`, `table` |
| Data | `metric`, `progress`, `chart`, `data_grid`, `comparison` |
| Media | `image`, `audio`, `video`, `file`, `link_preview`, `gallery` |
| Work | `diff`, `test_matrix`, `terminal`, `artifact`, `citation_list` |
| Interaction | `disclosure`, `filter`, `sort`, `search`, `stepper`, `timer` |
| State | `loading`, `empty`, `partial`, `error`, `unavailable`, `stale` |
| Universal | `map`, `calendar`, `board`, `graph`, `entity`, `evidence`, `form`, `transaction`, `alert`, `conversation`, `simulation` |
| Domain | `recipe`, `research`, `ui_preview` |

`tabs` was in the first draft of this table and is not in the enum; a tabbed
layout composes from `section` plus `disclosure`. Host actions
(`open_file`, `open_url`, `copy`, `download`, `review`, `request_action`) are
typed action descriptors carried in node props, not primitives.

The bar for a new primitive is that no composition of existing primitives can
express it, and the enum records that reasoning in a doc-comment on each
variant — see `Recipe`/`Research`/`UiPreview` in `lib.rs` and the block comment
immediately after the enum explaining why `MetricGrid`, `Media` and
`UniversalCard` were deliberately NOT added (they are `Row`/`Section` of
`Metric`; `Image`/`Audio`/`Video`/`File`/`Gallery`; and `Entity` or `Section`
plus `KeyValue` respectively). A client-side rendering shortcut is not a
reason for a host primitive.

Specialized components are not part of this design: the client has one generic
renderer over this vocabulary. Compound definitions may compose primitives but
may not introduce a second execution mechanism. The contributor walkthrough for
both paths — new semantic type over an existing primitive, and new primitive —
is `docs/design/67-presentation-renderer-guide.md`.

### Styling

Specs use semantic tokens and bounded variants, not CSS:

- `tone`: neutral, accent, info, success, warning, danger;
- `emphasis`: quiet, normal, strong;
- `density`: compact, regular, relaxed;
- `width`: content, fill;
- documented typography, spacing, border, and alignment tokens.

Arbitrary colors, font families, CSS strings, absolute positioning, scripts,
iframes, inline event handlers, and raw HTML are rejected. The host owns theme,
responsive behavior, focus order, contrast, reduced motion, localization, and
screen-reader structure.

---

## 7. Semantic results and generic composition

Rich presentation must not depend on the model happening to emit a special
fence correctly. The runtime accepts structured results from four sources:

1. a tool with a declared typed output;
2. an admitted outcome/result schema;
3. a validated structured block in an assistant response;
4. a deterministic extractor that derives structure without changing meaning.

An extractor may identify headings, tables, citations, dates, steps, or existing
typed tool data. It may not invent missing facts. Model-assisted extraction is a
separate recorded dispatch and its output remains `Asserted` unless supported by
the original result/evidence. Failure returns the original document unchanged.

Every semantic type requires:

- a versioned payload schema;
- an accessible textual projection;
- representative valid, partial, invalid, oversized, and adversarial fixtures;
- declared evidence expectations where claims require them;
- at least one eligible renderer or an explicit generic fallback.

Unknown semantic types use a generic inspector only in Advanced mode and their
exact fallback in Everyday mode. They never display raw JSON as the primary
user experience.

---

## 8. Selection and compilation

Selection is deterministic, explainable, and ordered from narrowest user intent
to broadest safe fallback:

```text
turn-scoped user choice
→ exact user preference
→ exact workspace preference
→ enabled plugin recipe
→ built-in specialized recipe
→ generic primitive composition
→ PresentationDocument
→ exact Markdown fallback
```

Within a layer, candidates are ordered by specificity, compatible schema
version, declared priority, and stable ID. Equal candidates are an error with a
diagnostic, never nondeterministic selection.

Compilation is a pure pipeline:

```text
result + outcome + evidence
  → validate semantic payload
  → resolve immutable capability epoch
  → select recipe and spec
  → validate spec
  → bind data
  → lower to RenderTree
  → surface capability projection
  → native renderer or deterministic fallback
  → selection and coverage receipt
```

`RenderTree` is the only input accepted by native card components. It contains
resolved text/data, primitive nodes, typed host-action descriptors, provenance,
and accessibility labels. It contains no code and no unresolved filesystem or
network reference.

Every source field must be covered by one of: rendered, summarized, intentionally
hidden with a reason, or retained in fallback. Silent loss is a compile error.

---

## 9. Runtime generation and user-guided revision

When no satisfactory definition exists, the agent may propose one inside the
authority and budget already admitted for the task. Presentation generation is
not permission and cannot widen the capability slice.

The flow is:

1. Render the safe generic/document fallback immediately.
2. If adaptive presentation is enabled, propose a bounded spec from the
   semantic schema and sample-shaped data with sensitive values redacted where
   possible.
3. Validate and compile it in an isolated, credential-free worker.
4. Display it as a preview associated with the same immutable result.
5. Let the user choose `Use this`, `Keep the original`, or provide feedback.
6. Append the choice/feedback, create a new immutable revision, and preview
   again. Never edit the prior revision.
7. Offer persistence only after explicit user intent.

Useful feedback commands include:

- make this simpler / show more detail;
- use a timeline, comparison, checklist, table, or steps;
- hide or reorder a named section;
- keep a section visible while interacting;
- change plain-language labels;
- always use this for a named scenario;
- restore the built-in default.

One bad card must be recoverable in one action. `Show original` is always
available. Feedback about layout is not treated as correction of the factual
result unless the user explicitly corrects the result too.

Generated proposals have bounded attempts and a doom-loop rule: after two
invalid revisions or two rejected previews for the same feedback chain, stop
automatic regeneration, retain the fallback, and ask for a concrete preference.

---

## 10. Persistence, matching, and privacy

There are four lifetimes:

| Lifetime | Scope | Activation |
|---|---|---|
| Preview | one result | automatic after validation |
| Use once | one result revision | explicit selection |
| Remember | user or workspace | explicit user request |
| Pack | portable installed generation | inspect, install disabled, enable |

Definitions live outside session ledgers in a content-addressed presentation
store under the canonical Vak data home, with user and workspace activation
indexes. A definition stores schema, bindings, layout, fixtures, provenance,
digest, revision ancestry, and activation receipts. It must not store the
user's result payload unless the user separately saves an example fixture.
Fixtures are redacted by default and visibly reviewed before export.

Matching rules are declarative and narrow:

- semantic type and compatible version are mandatory;
- optional domain, modality, surface, mode, and schema-feature predicates;
- no matching on secret values or arbitrary prompt substrings;
- preferences may narrow applicability but never change authorization;
- a broad generated preference requires a preview and confirmation.

Deleting a preference removes its activation pointer, not historical session
receipts or immutable package generations still referenced by history.

---

## 11. Experience packs and plugins

An experience pack is a plugin presentation contribution that may contain:

- semantic schemas;
- declarative `PresentationSpec` files;
- selection recipes;
- deterministic extractors from the approved extractor vocabulary;
- localization catalogs;
- redacted fixtures and golden fallbacks;
- channel projection hints;
- static, non-executable assets within existing limits.

It may not contain native React components, JavaScript renderers, remote code,
CSS, embedded credentials, or direct tool actions. A future isolated standard
UI host is a separate proposal and must not be smuggled into this contract.

Packs use the existing plugin lifecycle and capability epoch. Installation does
not activate them. Enabling publishes a new capability epoch; additions apply
at the next turn boundary, revocation fails closed immediately, and no restart
or session rotation occurs.

Plugin-defined semantic types must render using the shared primitive vocabulary.
This closes the current gap where the server can recognize a plugin type but the
desktop falls through to an unsupported JSON view.

Export includes no result data by default. Sharing produces a deterministic
archive and manifest with publisher metadata, source scope, component inventory,
schema/API requirements, digest, license, and optional signature. Import stages
and inspects it exactly like any other plugin generation.

### Pack authoring and upgrade contract

Pack authors should validate a definition before publishing it with the host
validator, then test at least one redacted fixture through both the rich tree
and the exact fallback compiler. A pack is not publishable when validation
reports an unsupported primitive, binding, semantic mismatch, unsafe action, or
limit violation. The authoring checklist is deliberately host-vocabulary
based; it does not contain domain-specific card names or renderer code.

Upgrades are additive within the supported major schema. Existing immutable
revisions and session receipts remain readable, while unknown future fields are
ignored by older readers. An upgraded pack is staged disabled, inspected, and
only then explicitly enabled. If its rich renderer, primitive, or pack
generation is unavailable, replay retains the recorded fallback and reports
the refusal reason; it never silently reconstructs a different card. Pack
removal revokes activation immediately but does not rewrite historical
receipts.

---

## 12. Delivery and sharing

The canonical `OutputTimeline` and exact fallback remain common to desktop,
browser, webhook, and messaging channels. Each target receives the richest
projection it declares:

- desktop/browser: native `RenderTree` primitives and approved interactions;
- JSON webhook: semantic result, presentation reference/spec when requested,
  fallback, coverage, actions, provenance, and diagnostics;
- Slack/Discord/Telegram: channel-native markup, tables/media/actions only when
  supported, otherwise ordered text chunks and explicit fallback;
- terminal/TUI: accessible text/table projection; TUI widgets remain optional;
- exports: exact Markdown plus optional portable presentation metadata.

Interactive state such as a local timer, filter, or expanded section is view
state, not session truth. A consequential action becomes a logged host request
and passes through the permission engine. Channels never receive buttons that
their adapter cannot render. Degradation must preserve all material content and
state why richer behavior was unavailable.

Sharing a result is different from sharing a definition. The UI must offer
distinct commands:

- `Share result` — follows existing transcript/artifact privacy and delivery;
- `Save presentation` — stores the definition in user/workspace scope;
- `Export experience pack` — exports definition assets and reviewed fixtures.

---

## 13. APIs and events

Additive endpoints, names indicative:

```text
GET  /presentations/primitives
GET  /presentations/library?scope=user|workspace
GET  /presentations/specs/:id/:revision
POST /sessions/:id/results/:result_id/presentation/proposals
POST /sessions/:id/results/:result_id/presentation/select
POST /sessions/:id/results/:result_id/presentation/feedback
POST /presentations/library
DELETE /presentations/library/:id/activation
POST /presentations/export
POST /presentations/import
```

All mutations use existing authentication, origin/token rules, workspace
boundary checks, atomic stores, size limits, and audit receipts. A session and
result ID are mandatory for result-scoped mutation. Bulk mutation rechecks the
same boundary per item.

SSE adds presentation proposal/selection/revocation deltas with complete
snapshots. Every streaming event follows the repository rule: delta and
snapshot describe the same point. Reconnect reconstructs selected cards without
rerunning a model or depending on an expired cache.

---

## 14. UI contract

### Card chrome

Every adaptive card has consistent, quiet controls:

- title and optional status/evidence summary;
- `View original`;
- `Adjust presentation…`;
- `View details` for recipe, renderer, provenance, evidence, and diagnostics;
- accessible loading, partial, invalid, revoked, and unavailable states.

Everyday uses plain labels and outcome-first content. Advanced adds technical
metadata without changing the card's factual body. Dense technical components
remain available in Advanced and may be used in Everyday when they are the
clearest ordinary representation, such as a simple comparison table.

### Presentation editor

The first editor is guided, not a general canvas builder:

- preview beside original;
- natural-language feedback;
- primitive/layout choice;
- section visibility and order;
- scope chooser: once, this workspace, everywhere;
- reset, revision history, and export;
- explicit warning when a broader match will affect future results.

Do not expose JSON as the primary editing experience. Advanced may provide a
read-only spec inspector and validation diagnostics.

### Mode preference

Add `experience_mode = everyday | advanced` to the user UI preference layer,
defaulting to Everyday for new installs. Existing users retain the current
Advanced experience on upgrade to avoid silently removing controls. The mode
toggle appears once in the application header/profile area, not again in the
composer. It synchronizes across the user's surfaces where shared preferences
already synchronize, with a local fallback when disconnected.

---

## 15. Security and resource limits

The compiler and renderer must enforce:

- maximum spec bytes, nodes, depth, bindings, collection length, text length,
  media count, and action count;
- execution deadline and memory budget in the isolated compiler worker;
- scheme-checked links and media using the existing safe URL contract;
- no filesystem resolution except already-authorized artifact references;
- no network, environment, secrets, tools, subprocesses, dynamic imports,
  script, HTML, CSS, SVG script, or model-authored accessibility bypass;
- stable focus order and no keyboard traps;
- color never as the only state carrier;
- fallback and diagnostics on every refusal;
- capability/revocation check at compile and action time;
- digest verification before reuse, import, preview, or delivery.

A spec is presentation, not authority. “Approve”, “send”, “delete”, “purchase”,
or similar labels cannot manufacture an action. The spec references an action
descriptor already issued by the runtime; the host decides whether and how to
render it.

---

## 16. Implementation architecture

Create a dedicated crate rather than expanding `vak-delivery` into a UI engine:

```text
crates/vak-presentation/
  src/spec.rs          schema and compatibility
  src/primitives.rs    closed vocabulary
  src/binding.rs       pure bounded binding engine
  src/validate.rs      structural/security validation
  src/compile.rs       spec + result → RenderTree
  src/select.rs        deterministic recipe selection
  src/store.rs         immutable definitions and activation indexes
  src/pack.rs          import/export manifest
  src/fallback.rs      document/channel lowering
  src/receipt.rs       selection/coverage/provenance
```

Integrations:

- `vak-delivery`: owns `OutputTimeline`, surface capabilities, channel lowering,
  exact fallback, and delivery worker integration;
- `vak-server`: APIs, SSE, workspace checks, merged capability epoch, proposal
  orchestration, persistence, and audit;
- `vak-client-ui`: the generic primitive renderer
  (`src/components/presentation/GenericSpecRenderer.tsx` — one `renderX()` per
  primitive, plus `buildXSpec()` adapters that lower today's raw
  `semantic_type` payloads into the same node shape `compile()` emits), guided
  editor, and mode projections. P2 is done: there are no specialized per-type
  card components left to retain;
- `vak-plugin`: inventories specs/schemas/recipes/fixtures as presentation
  components and exposes their immutable generation;
- `vak-session`: additive presentation events and selection references;
- `vak-core`: optional proposal dispatch under admitted budget/route and current
  capability epoch; no rendering decisions;
- `vak-admin-ui`: inspect, enable, revoke, compare, and audit definitions/packs;
- `vak-config`: layered preferences and security/resource ceilings.

There must be one compiler and selection implementation used by live desktop,
historical replay, preview, webhook, channels, and export. Clients render the
compiled tree; they do not independently select recipes.

---

## 17. Delivery phases

Each phase is independently shippable, preserves fallback, and ends with its
tests green. An implementing agent must complete phases in order.

### P0 — Baseline characterization

- Freeze golden fixtures for every current built-in renderer and its Markdown
  fallback.
- Add cross-surface snapshots for desktop JSON, Markdown, Plain, Telegram,
  Slack, Discord, and webhook.
- Record current accessibility and failure behavior.

Exit: later phases can prove that no current renderer or fallback regressed.

### P1 — Spec, primitives, compiler

- Add `vak-presentation` with schema, validation, binding, `RenderTree`, coverage,
  and deterministic fallback.
- Implement the primitive vocabulary without model generation or persistence.
- Property/fuzz test hostile specs and payloads.

Exit: fixtures compile deterministically; invalid input cannot panic, execute,
escape limits, or lose uncovered data.

### P2 — Migrate existing renderers — **landed**

The nine hand-written per-type components (`DataGrid`, `RecipeCard`,
`ResearchCards`, `TerminalConsole`, `TestMatrix`, `TimelineCard`,
`UIPreviewCard`, `UniversalCard`, `UniversalChart`) are deleted. All
`STRUCTURED_RENDERERS` keys route through `GenericSpecRenderer`.

- Wrap or lower all current structured renderers through trusted primitive or
  compound definitions.
- Preserve specialized behaviors: diff/editor integration, test filtering,
  chart crosshair/CSV, grid sort/search/CSV, terminal metadata, recipe scaling
  and timers, UI preview docking, citations, media, and links.

Exit: P0 goldens and interaction tests pass with no legacy parallel selection
path.

### P3 — Generic everyday scenarios

- Add built-in semantic schemas/specs for timeline/itinerary, plan, comparison,
  checklist, schedule, lesson, budget, meeting follow-up, decision, and generic
  collection/detail.
- Add conservative deterministic structure extraction.

Exit: representative coding and non-coding fixtures render usefully without
custom frontend components and degrade faithfully.

### P4 — Library, revisions, and selection

- Add immutable store, activation indexes, selection precedence, receipts,
  session entries, APIs, replay, and revocation.
- Ensure user/project layering stores intent, not copied effective state.

Exit: use-once and remembered definitions survive restart; historical sessions
reconstruct the selected revision; revocation is immediate.

### P5 — Guided feedback and proposal loop

- Add proposal generation through the admitted route/budget.
- Add preview/original comparison, natural-language feedback, immutable
  revision, bounded retry, reset, and explicit persistence.

Exit: a user can correct a poor card, use it once, remember it, and restore the
original without altering result truth.

### P6 — Plugin experience packs

- Extend plugin inspection and capability reconciliation for schemas, specs,
  recipes, localization, fixtures, and static assets.
- Replace unsupported plugin JSON display with primitive compilation/fallback.
- Add deterministic export/import and review surfaces.

Exit: an enabled local pack adds a new semantic result presentation on the next
turn without restart; disable/revoke removes it immediately; rollback restores
the prior immutable generation.

### P7 — Everyday/Advanced experience

- Add the experience-mode preference and two projections over one task.
- Redesign navigation/status/detail labels without hiding gates, failures,
  privacy, or outcome truth.
- Preserve current users in Advanced; default new installs to Everyday.

Exit: switching is immediate, session-stable, and capability-neutral across
desktop and browser at supported widths.

### P8 — Delivery parity and sharing

- Project compiled presentations across every adapter and JSON webhook.
- Add distinct result sharing, definition saving, and pack export.
- Preserve ordered chunks, actions, coverage, outbox, and failure diagnostics.

Exit: every fixture has a loss-accounted projection on every surface; unsupported
interaction becomes explicit text rather than disappearing.

### P9 — Operations, quality, and release gate

- Add health/operations evidence for compiler failures, proposal cost/latency,
  selected/fallback counts, invalid packs, revocations, and delivery degradation.
- Add documentation, examples, authoring validation, and upgrade notes.
- Run repository-wide format, lint, unit, integration, browser, accessibility,
  replay, security, and packaging checks.

Exit: the final done-contract below passes from a clean checkout and from an
upgrade fixture with existing sessions and plugins.

Operations evidence must be derived from append-only presentation activities,
delivery receipts, plugin inspection results, and compiler diagnostics. Until
the dedicated counters are wired into the Operations Center, the absence of a
counter is reported as unavailable rather than zero; the console must not
invent adaptive success, latency, or fallback rates from unrelated process
metrics.

---

## 18. Agent execution loop

An autonomous implementation goal should use this exact loop:

1. Read this document and every directly affected invariant/design document.
2. Inspect current source and tests for the phase; never assume a status line is
   proof of behavior.
3. Write or update the phase's executable acceptance tests first.
4. Implement one vertical slice through types, persistence, server, UI,
   delivery, and replay where the phase requires them.
5. Run the narrowest relevant tests, then crate/package tests.
6. Inspect the rendered desktop/browser result once at all supported widths;
   fix findings in one batch and confirm once.
7. Run fallback/channel fixtures and append-only/replay tests.
8. Run `cargo fmt --all --check` and affected clippy/tests.
9. Record the phase status and evidence in this document only after commands
   actually pass.
10. Continue to the next phase while a safe, in-scope implementation step
    remains. Stop only for missing authority, external credentials/signing, or
    a design contradiction that changes this contract materially.

The agent must not declare completion from fixture construction, screenshots,
type existence, or documentation. Completion requires the end-to-end behaviors
and failure cases below.

---

## 19. Verification matrix

Minimum fixture families:

- travel itinerary, recipe, study lesson, shopping comparison, household plan;
- research synthesis with valid/invalid citations;
- spreadsheet/table, sparse data, large data, temporal chart;
- code diff, tests, terminal, incident timeline, UI artifact;
- partial, failed, cancelled, stale-evidence, approval-blocked results;
- unknown semantic type, missing fields, extra fields, future schema version;
- malicious binding paths, oversized/nested specs, unsafe URLs/media, action
  forgery, revoked plugin, missing renderer, worker crash, reconnect;
- user correction, two rejected revisions, use once, remember, reset, export,
  import, update, rollback, revoke;
- Everyday/Advanced switching during idle, streaming, approval, failure, and
  historical replay;
- desktop, browser, narrow browser, Markdown, Plain, Telegram, Slack, Discord,
  JSON webhook, transcript export.

Required assertions:

- identical result identity and outcome across every projection;
- exact fallback contains every material source fact;
- no typed renderer upgrades evidence or completion;
- no model-authored code executes;
- no card action bypasses permission/approval;
- no plugin addition requires restart/session rotation;
- revocation blocks current capability immediately;
- every model-visible proposal/feedback input is derivable from the ledger;
- every SSE event carries matching delta and snapshot;
- partial output survives cancellation/failure;
- presentation data never leaks across workspace, chat, bot, user, or pack;
- existing rich components retain their behaviors.

---

## 20. Final done-contract

This program is complete only when all statements are true:

1. A user asks for an unanticipated ordinary result, receives a useful safe
   generic presentation, improves it in natural language, uses the revision,
   remembers it, and sees it selected deterministically in a later session.
2. A developer installs and enables an experience pack defining a new semantic
   type and declarative layout; every live session can use it at the next turn
   without restart, and revocation removes it immediately.
3. The same result renders natively in desktop/browser and degrades without
   silent loss through every supported delivery adapter and export.
4. Everyday and Advanced show the same session, result, approval, outcome, and
   artifacts with different disclosure, and switching changes no authority.
5. Historical replay reconstructs the exact selected immutable definition and
   falls back faithfully if its executable host support is unavailable.
6. Generated specs cannot execute code, access ambient state, forge actions,
   hide mandatory safety/evidence state, or escape resource limits.
7. Existing research, diff, tests, chart, grid, terminal, recipe, media, link,
   approval, artifact, and UI-preview behavior remains repository-verified.
8. Upgrade tests prove additive compatibility for existing schema-v2 sessions,
   output configuration, plugins, delivery jobs, and user preferences.
9. Format, clippy, Rust tests, client tests/build, browser/accessibility checks,
   security/fuzz tests, documentation path checks, and committed bundle/source
   verification all pass.

---

## 21. Non-goals

- Executing arbitrary model- or plugin-authored React, JavaScript, HTML, CSS, or
  native code in the chat renderer.
- Replacing the artifact sandbox or treating generated applications as cards.
- Making UI mode an authorization or routing preference.
- Letting presentation decide whether work is complete.
- Guaranteeing native interactivity in channels that cannot express it.
- Publishing a marketplace or assigning trust from popularity.
- Storing private result payloads inside shared definitions by default.
- Maintaining both the old and new presentation selection pipelines after
  migration; one canonical compiler/selector must replace the old path.

The scalable unit is not a bespoke card component. It is a versioned semantic
result plus a safe, composable, revisable presentation definition with an exact
fallback and an auditable selection receipt.
