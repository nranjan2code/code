# 67 — Adding or editing a presentation renderer

Status: **contributor guide for shipped code.** Every file, function, and test
named here exists today. The architecture it describes is
`docs/design/57-adaptive-presentation-runtime.md`; this document is the
how-to that sits beside it.

Read this before touching anything that decides how a result looks. The single
most common mistake in this area is reaching for a new component or a new
`Primitive` variant when a one-line registry entry was the correct change.

## 1. The two-layer model

There are exactly two extension layers, and they have very different costs.

### PRIMITIVES — closed, compiled-in vocabulary

The `Primitive` enum in `crates/vak-presentation/src/lib.rs` is the host's
closed vocabulary. It is the *only* thing a spec may name. Adding a variant is
a real code change in three crates plus the client, because:

- `crates/vak-delivery/src/adaptive.rs` matches `Primitive` **exhaustively** in
  `render_node()` — a new variant without a lowering arm fails the build. That
  is deliberate: it is what guarantees a constrained surface (Telegram, Slack,
  Discord, plain text) can always lower any primitive to text.
- `crates/vak-client-ui/src/components/presentation/GenericSpecRenderer.tsx`
  dispatches on `node.primitive` in `renderNode()`. An unknown primitive
  degrades to `renderFallback()` — safe, but visibly generic.

The bar for a new primitive is high and is written into the code. Each of the
three most recent variants carries a doc-comment saying *why no composition of
existing primitives expresses it* (`lib.rs`):

- `Recipe` — "Table/KeyValue cannot express the ingredient/step/timer triple
  without the surface guessing which column means what."
- `Research` — "The citation-to-takeaway relationship is lost if this is
  flattened into a Section plus a CitationList."
- `UiPreview` — "The isolation contract (no ambient privileges for the
  previewed document) is part of the primitive."

And the block comment immediately after the enum records the variants
deliberately **not** added, with their compositions:

```
metric_grid    = `Row`/`Section` whose children (or `each`/`item`) are `Metric`
media          = `Image` / `Audio` / `Video` / `File` / `Gallery`
universal_card = `Entity` or `Section` containing `KeyValue` rows
```

> "A client-side rendering shortcut is not a reason for a host primitive; only
> a concept that no composition can express is."

Follow that convention. If you cannot write that sentence for your variant, you
do not need a variant.

### PACKS — runtime-pluggable, zero code change

A pack is data: `PresentationSpec` records wrapped in `StoredPresentation`,
carrying a `PresentationOrigin` (`LibraryScope::User`/`Workspace`, owner,
optional `plugin_id`, generation). Packs are registered, previewed, and
activated at runtime through `PresentationLibrary` / `vak-core`'s
`PresentationStore`, and revoked per plugin generation via `revoke_plugin`.

A pack **composes existing primitives** and needs no recompilation. This is
proven, not asserted: `new_primitives_compile_rich_with_their_own_payloads` and
its sibling tests in `crates/vak-presentation/src/seeds.rs` register packs at
runtime and compile them through the same generic pipeline.

`crates/vak-presentation/src/seeds.rs` is the worked example of pack authoring:
75 disabled starter definitions (45 `EVERYDAY`, 10 `CODING`, 20 `UNIVERSAL`)
built from bindings (`$.title`, `$.items`, `$.summary`) over a root primitive
chosen by a `match` on the accepted semantic type. Note the file's own opening
line: *"Adding a seed never adds a renderer branch."*

### Deciding which layer you need

| You want | Layer | Cost |
|---|---|---|
| A new `semantic_type` shown like an existing one | Registry entry | 1 line |
| A reusable layout over existing primitives | Pack / seed | data only, no rebuild |
| A shape no composition can express | Primitive | enum + fallback arm + client pair |

## 2. "I want a new semantic_type that reuses an existing primitive"

This is the common case and the cheap one.

**Step 0 — confirm the type is actually server-registered.** A client
`STRUCTURED_RENDERERS` key that the server never validates is dead code: any
answer using it is rejected by `SkillRegistry::validate()`
(`vak-delivery/src/skills.rs`) before it reaches the client, "unknown type."
Check `vak_delivery::skills::built_in_semantic_types()` directly for the
exact accepted set — do not assume a name is live just because a renderer
or a `system-prompt.md` example mentions it.
`crates/vak-core/src/presentation_tools.rs`'s
`every_registered_semantic_type_across_all_shapes_renders` test exercises
every one of those types end-to-end and will fail loudly if a shape's tool
claims a type the registry doesn't accept, but it doesn't enumerate them for
you — call `built_in_semantic_types()` for that. As of 3.4.5 the registry
holds 97 types. This check exists because `STRUCTURED_RENDERERS` grew ten
keys across two commits (85408ebc, 1d34cd6b) without a matching registry
update — `decision_matrix`, `criteria_matrix`, `tradeoff_analysis`,
`metric_chart`, `comparison_chart`, `telemetry.chart`, `telemetry.metric`,
`weather`, `lifestyle.recipe`, `lifestyle.culinary_recipe` — silently
unreachable for a full release cycle until caught while building the
`emit_*_card` tool-calling path (2026-09-18) and registered.

**Step 1 — add the registry entry.** `STRUCTURED_RENDERERS` in
`crates/vak-client-ui/src/components/PresentationRenderer.tsx` is the single
registry; its header comment states the invariant: *"Every named semantic type
resolves through this single registry."* Point your key at an existing
`buildXSpec`:

```tsx
"my.new_type": ({ data }) => <GenericSpecRenderer node={buildRecipeSpec(data)} />,
```

Several keys sharing one builder is the intended pattern, not duplication.
`recipe.card`, `recipe`, `recipe_summary`, `lifestyle.recipe`, and
`lifestyle.culinary_recipe` are five registry lines that all resolve to the
same `buildRecipeSpec` / `renderRecipe` pair. Likewise `trend`, `timeseries`,
`metric_chart`, and `comparison_chart` all resolve to `buildChartSpec`, with
`bar_chart` differing only by the forced `"bar"` override argument, and
`decision_matrix` / `criteria_matrix` / `tradeoff_analysis` all resolving to
`buildTableSpec` with different default titles.

Conditional routing is allowed where the payload genuinely decides the shape —
`meal_plan` picks `buildRecipeSpec` when `steps` or `ingredients` are present
and `buildTimelineSpec` otherwise. Do not extend this to guessing: an
*unregistered* type must stay unregistered. `StructuredRenderer` deliberately
refuses to infer a renderer from payload shape, because "that lets a new
contract silently take over an older card and makes regressions invisible."

**Step 2 (optional) — add a seed** in `crates/vak-presentation/src/seeds.rs`:
one tuple in `EVERYDAY`/`CODING`/`UNIVERSAL` and, if it needs a specific root,
one arm in `seed()`'s `match accepts`. Bump the seed-count assertion in
`seed_pack_is_rich_disabled_and_validated_by_host_types`. Seeds register
**disabled**; activation stays an explicit operation.

**Step 3 (optional) — teach the model.** Only if the type is worth actively
prompting for. The `capability_contract` block in
`crates/vak-core/src/system-prompt.md` is the *only* model-facing location in
the repo that teaches `semantic_type` emission — onboarding text, capability
snapshots, and tool descriptions carry none. Untaught-but-accepted types are
still listed by name: `presentation_catalogue_section()` in
`crates/vak-core/src/prompts.rs` generates the "Also accepted as
`semantic_type`" list from `vak_delivery::skills::built_in_semantic_types()`,
so a registered type is discoverable even with no worked example. Add a worked
example only when the payload shape needs demonstrating — and see §5 before you
write one.

## 3. "I want a genuinely new primitive"

The checklist below is what commit `65e5951d` actually did for `Recipe`,
`Research` and `UiPreview`. Do all of it.

1. **Justify it in a doc-comment on the variant** in
   `crates/vak-presentation/src/lib.rs`, in the established form: name the
   compositions that *almost* work and say what they lose. If the answer is
   "nothing, it would just be more convenient on the client", stop — add a
   registry entry or a pack instead, and consider recording the rejection in
   the "Deliberately absent" comment after the enum.
2. **Add the fallback arm** in `crates/vak-delivery/src/adaptive.rs`
   `render_node()`. The build will force this. Do not widen an existing
   catch-all `|` arm to swallow the new variant unless plain bullet lowering is
   genuinely the right text projection — the three domain primitives each got
   their own arm because they each have structure worth preserving in text.
3. **Add the client pair** in
   `crates/vak-client-ui/src/components/presentation/GenericSpecRenderer.tsx`:
   a `case "my_primitive":` in `renderNode()`'s switch, a
   `renderMyPrimitive(node, surface)` that honours both `"full"` and
   `"compact"` surfaces, and an exported
   `buildMyPrimitiveSpec(data): AdaptiveRenderNode` adapter. Keep the adapter
   tolerant — the file's own convention is "try several known field names,
   degrade to an empty/placeholder value, never throw on missing or malformed
   input."
4. **Wire the registry** — one or more `STRUCTURED_RENDERERS` keys pointing at
   the new builder, per §2.
5. **Add a seed** in `seeds.rs` so the primitive is reachable end to end, and
   update the counts.
6. **Prove it compiles through, don't just assert it.** The pattern is
   `new_primitives_compile_rich_with_their_own_payloads` in `seeds.rs`: feed a
   realistic payload through the real `compile()` and assert the result is
   `CompiledPresentation::Rich` **whose root primitive is the new one**. Its
   comment states the point: "The enum additions are reachable end to end, not
   decorative." A test that only asserts the enum variant exists proves
   nothing. `seeds_reach_the_newly_added_primitives` is the companion check.
7. **Teach the model** in `system-prompt.md` if the primitive is something the
   model should reach for, per §2 step 3.

## 4. Verification checklist

Run all of these before calling the change done.

Fast inner loop, narrowest first:

```sh
cargo build --workspace --exclude vak-desktop
cargo test -p vak-presentation   # spec/compiler/seed-count/primitive reachability
cargo test -p vak-delivery       # exhaustive primitive lowering, skills validator
cargo test -p vak-core           # prompt-phrase pin + catalogue tests
```

Then the repo-wide gate from `AGENTS.md`, which is what CI runs and what a
commit is judged against:

```sh
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
scripts/check-version.sh && python3 scripts/check_doc_paths.py
```

`check_doc_paths.py` matters here specifically: this area's docs cite crate
paths, and it fails on any that no longer exist.

Client:

```sh
cd crates/vak-client-ui
npx tsc --noEmit
npm run build                   # BOTH bundles; dist-web/ is committed and must match source
```

`npm run build` writes `dist/.src-manifest` and the server build re-verifies
it, so a source change without a rebuilt bundle fails the build by name. Never
weaken that check — regenerate the bundle.

**If you changed rendering behaviour visibly, run the browser check.** The
permanent harness is `crates/vak-client-ui/tests/presentation.html` +
`presentation.tsx`; see `crates/vak-client-ui/tests/README.md`:

```sh
npm run dev:web -- --port 1421
agent-browser --session vak-render-check open http://localhost:1421/app/tests/presentation.html
agent-browser --session vak-render-check wait --load networkidle
agent-browser --session vak-render-check eval 'window.runChecks()'
```

It iterates `structuredRendererTypes` — derived from `STRUCTURED_RENDERERS`
itself — so a newly registered type cannot bypass the completed-turn contract,
and it asserts one rendered slot per registered type.

For a *migration* rather than an addition, the technique used throughout the
`GenericSpecRenderer` migration is stronger and worth repeating: stand up a
throwaway harness that renders the old component and the new primitive path
side by side over the same payload — happy path **and** edge cases (missing
field, `null`, wrong type, empty array) — and diff the resulting DOM for
byte-identical output. Then boot the real app and check the console is clean.
"It looked right in a screenshot" is not this.

## 5. Common mistakes (all of these actually happened here)

**Teaching the model a field name the renderer never reads.** The
`system-prompt.md` `decision` example taught
`{options:[{name,score,pros,cons}]}`. The timeline builder's field-guessing
never recognises `options`, so the example silently degraded to a raw
JSON-dump fallback — a broken example that no test caught, because nothing tied
the prompt's payload shapes to the builders that consume them. It now teaches
`choices[{name,reason,status}]`, the shape actually read. **Cross-check every
prompt example against the real `buildXSpec` field list and, for validated
types, against `crates/vak-delivery/src/skills.rs`.** Do not write the example
from the payload you wish existed.

**Guarding a renderer on a field and rendering nothing when it is absent.** The
old registry guarded every chart key with
`Array.isArray(data?.series) ? <UniversalChart .../> : <></>`, so a payload
whose `series` was missing, `null`, or a non-array rendered *literally
nothing* — no card, no error, no fallback. `buildChartSpec` now normalises any
shape to an empty series list and `renderChart` shows its
"No data points supplied." placeholder. The same class of bug existed in
`test_matrix` (the old component called `.filter` on a possibly-absent `tests`)
and in `diff` (a raw patch under `diff`/`patch`/`raw_diff`/`content` rendered
an empty card). **An empty or malformed payload must reach a visible empty
state, never a blank node and never a throw.** Test the empty array, not just
the populated one.

**Stringifying a structured value instead of carrying it.** `JSON.stringify` on
a nested value is a last-resort leaf projection for display, and it is lossy —
the node schema is `primitive` / `props` / `children` and structure belongs in
children. If you find yourself stringifying to get something on screen, you are
usually missing a nesting level; `SpecNode` lets any primitive nest any other,
which is exactly why `metric_grid`, `media` and `universal_card` did not need
enum variants.

**Defaulting a status field.** `buildTestMatrixSpec` deliberately does **not**
default a test's status to `"passed"`. A defaulted status turns missing data
into a positive claim, which is the presentation-layer version of upgrading an
unsupported claim into observed evidence. Leave it empty and let the renderer
show it as unknown.

**Adding a primitive for a rendering convenience.** See §1. Three variants were
added in `65e5951d`; three more were explicitly rejected in the same commit and
the rejection reasoning was committed as a code comment so the next contributor
does not re-litigate it.

## Related

- `docs/design/57-adaptive-presentation-runtime.md` — the architecture: spec,
  compiler, primitive vocabulary, pack lifecycle, scoping and revocation.
- `docs/design/30-render-architecture.md` — ledger → projection → delivery,
  everything upstream of the client renderer.
- `docs/design/30-output-engineering.md` — channel lowering and surface
  capabilities.
- `AGENTS.md`, "Code rules" — the binding one-paragraph version of this
  contract.

## Layout stress check

Short fixtures hide layout bugs: text that escapes its container only shows up
with real-world lengths (long titles, snippets, URLs). Open the card harness
with `?stress=1` (`/app/harness.html?stress=1` under `npm run dev:web`) and it
pads every string in every fixture with long prose plus an unbroken URL. Then
measure, per element, whether it spills its parent or the card
(`scrollWidth > clientWidth` with `overflow: visible`, or a bounding box past
its parent's edge), ignoring intentional scrollers (`.diff-code-area`,
`.semantic-table-wrap`) and hover-only popovers. Run it at a narrow viewport as
well. New cards should come out with zero findings. Flex children that hold
`nowrap` text need `min-width: 0`; headers with actions need `flex-wrap: wrap`.
