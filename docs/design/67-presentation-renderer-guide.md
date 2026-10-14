# 67 — Adding or editing a card form

Status: **reference**: the contributor guide for shipped code, rewritten for
the card forms on 2026-10-14 (docs/design/92-card-forms.md). The guide it
replaces described the per-kind registry the forms removed; it is in the
history of this file.

Read this before touching anything that decides how a result looks.

## 1. The model

A card is a **form**: a shape of information, never a topic. One list names
the forms, `FORMS` in `crates/vak-delivery/src/skills.rs`, and four places
must agree with it, which `the_card_kinds_are_exactly_the_types_the_client_draws`
(vak-server) checks:

| Place | What it holds for each form |
|---|---|
| `crates/vak-core/src/presentation_tools.rs` `SHAPES` | the card tool `emit_<form>_card`: flat arguments and the builder that stores them |
| `crates/vak-delivery/src/skills.rs` | `validate_form` (the stored shape) and `structured_markdown` (its text form for channels, exports and the model) |
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
renderer, and the form's sample in each test (`fixture_for` in
`presentation_tools.rs`, `sample` in `skills.rs`, `samples` in
`tests/presentation.tsx`) in one change. Keep the argument rules of
docs/design/89-constrained-tool-arguments.md: flat, every field described,
one name one shape (`one_argument_name_is_one_shape_on_every_block`).

## 4. Adding a form

Add it to all four places in §1 and to the table in doc 92 §2, only when
real turns show a shape no form carries. A topic (a recipe, a trip, a
budget) is never a form: it is a composition of forms in one answer
(docs/design/87-composed-answers.md).

## 5. A different layout for a form

That is a pack, not code: a `PresentationSpec` whose `block` names the form
and whose tree places the form's primitive or layout around its fields
(docs/design/91-presentation-capabilities.md). `packages/answer-layouts` is
the worked example, checked by `every_package_pack_compiles_from_its_block_payload`.
