# 67 — Adding or editing a card form

Status: **reference**: the contributor guide for shipped code, rewritten for
the card forms on 2026-10-14 (docs/design/92-card-forms.md). The guide it
replaces described the per-kind registry the forms removed; it is in the
history of this file.

Read this before touching anything that decides how a result looks.

## Table scalar fidelity

Authored table cells remain strings. The optional `numeric_columns` tool
argument names headers whose cells have decimal meaning; repeated, missing
or ambiguous declarations and malformed decimals are refused. The grammar
accepts signs, decimal fractions and exponents, excludes locale commas,
NaN and Infinity, and never converts through f64. Ability tables derive
numeric columns from the source JSON value types; explicit declarations
can also name decimal strings. Parsed source floats already have their
source parser's precision; this builder cannot recover their original tokens.

The stored shape remains columns with `label`/`numeric` and scalar rows;
existing numeric JSON cells remain valid. New cells preserve their decimal
spelling as strings. Numeric sorting compares sign, decimal order and
significant digits exactly, with invalid/missing values stably last in
both directions. Text sorting is lexical. Zero and negative zero compare
equally. The comparator never expands exponent zeroes. Rendering and text
fallback preserve strings; CSV retains its existing formula escaping.
Dynamic expression cells retain their binding paths and computed numbers.

`tests/table-fidelity.html` exercises production form rendering, sorting,
CSV and controls-to-table bindings. Its `window.runTableChecks()` is a
scoped fixture, not installed-app or whole-gallery acceptance.

## Numeric chart scales

Chart category labels remain text; numeric-looking identifiers are never
reparsed as numbers. Chart y values must be numbers after the common
argument reader; a formatted string such as `1,5` is refused rather than
guessing its locale. Ability-backed chart categories follow the same rule.

The shared chart uses bounded decimal-grid ticks. Ticks remain finite and
strictly increasing and span the input domain; where a nice grid cannot
preserve representable endpoints, those endpoints are the scale. Equal
finite endpoints expand to a representable interval. Reversed endpoints
are normalized and nonfinite endpoints produce no scale. Tick generation
has a fixed iteration bound and never rounds all tiny values to zero.

Labels use shortest round-trip number text, including compact exponential
notation for extremes. Positioning normalizes opposite-sign extremes to
avoid overflowing their span. Source tests cover tiny, adjacent, subnormal,
equal and extreme values. `x_scale` explicitly selects `category` (the
default), `number`, or `time`. Categories keep exact lexical labels and
source order. Numeric coordinates require finite numbers or JSON numeric
text; time coordinates require RFC3339 timestamps with offsets. Continuous
axes sort by coordinate and preserve unequal intervals. Invalid/ambiguous
coordinates show a visible explanation and keep the numbers available;
there is no fallback to misleading equal spacing. Each series has at most
one observation at a coordinate. Missing y observations split line and area
segments; an empty chart says it has no measured values. Browser acceptance
of these geometry/gap cases remains open under hardening P10.

## 1. The model

A card is a **form**: a shape of information, never a topic. One list names
the forms, `FORMS` in `crates/vak-delivery/src/skills.rs`, and the following
places must agree with it. `the_card_kinds_are_exactly_the_types_the_client_draws`
(vak-server) checks the host/client vocabulary;
`every_current_form_has_a_native_projection_on_every_channel` (vak-delivery)
checks channel coverage:

| Place | What it holds for each form |
|---|---|
| `crates/vak-core/src/presentation_tools.rs` `SHAPES` | the card tool `emit_<form>_card`: flat arguments and the builder that stores them |
| `crates/vak-delivery/src/channel/` | shared form projection and native channel compilers; doc 102 |
| `crates/vak-delivery/src/skills.rs` | `validate_form` (the stored shape) and `structured_markdown` (its text form for exports, explicitly textual surfaces and the model) |
| `crates/vak-presentation/src/lib.rs` `Primitive` | the form as a primitive a pack can place, beside layout (`stack`, `row`, `group`, `section`, `text`, `metric`) |
| `crates/vak-client-ui/src/components/cards/index.tsx` `FORMS` | the renderer that draws it |

Every card carries its form as `StructuredOutput.block`. The client draws by
block, never by kind; a kind selects a pack on the host and nothing else.

## 2. Changing how a form looks

Edit its renderer under `crates/vak-client-ui/src/components/cards/`
(`forms.tsx`, `Chart.tsx`, or `code.tsx` for diff, terminal, test report,
previews and media) and the matching rules in `styles.css` under "Card
forms". Every form sits in the `Card` shell from `cards/shared.tsx`; do not
add a second frame. Card text is `--fs-reading`, prose keeps `--measure`,
colours are theme tokens. Rebuild both bundles and check the form in the
browser at 1440 × 900 and 390 × 844, light and dark; the gallery is
`crates/vak-client-ui/tests/presentation.html` with `window.runChecks()`.

## 3. Changing what a form holds

Change the tool's arguments and builder, the validator and text form, the
renderer, and the native channel projection and form's sample in each test (`fixture_for` in
`presentation_tools.rs`, `sample` in `skills.rs`, `samples` in
`tests/presentation.tsx`) in one change. Keep the argument rules of
docs/design/89-constrained-tool-arguments.md: flat, every field described,
one name one shape (`one_argument_name_is_one_shape_on_every_block`).

## 4. Adding a form

Add it to all places in §1 and to the table in doc 92 §2, only when
real turns show a shape no form carries. A topic (a recipe, a trip, a
budget) is never a form: it is a composition of forms in one answer
(docs/design/87-composed-answers.md).

## 5. A different layout for a form

That is a pack, not code: a `PresentationSpec` whose `block` names the form
and whose tree places the form's primitive or layout around its fields
(docs/design/91-presentation-capabilities.md). `packages/answer-layouts` is
the worked example, checked by `every_package_pack_compiles_from_its_block_payload`.
