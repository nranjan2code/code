# 96 — Answer projection gaps

Status: in progress (2026-10-16): Q1, Q2 and Q3 are built; P1 to P6 are
open. Written from a review of what an answer can do on screen.

## 1. What is wrong

The card path is strict and recorded: one precise schema per form, the
wire enforcing it where it can, a ledger record per card, a text form for
every channel, and isolated previews. What it does not do, read from the
code on 2026-10-16:

1. **No answer can be operated.** A person can sort a table, download a
   CSV from a table or a chart, tick a list item, and pick a comparison
   option. Nothing computes: there is no input whose change updates a
   result, so a loan payment, a bill split or a unit conversion is prose
   or a static table.
2. **A choice is English text, and state is lost.** "Use this" on a
   comparison option (`optionInteractionFor`,
   `vak-client-ui/src/components/PresentationRenderer.tsx`) writes a fixed
   English sentence into the composer. A checkable list's ticks
   (`components/cards/forms.tsx`) and a table's sort live in the browser
   and are gone on reload. Neither is a record, so neither is model-visible
   in the sense of invariant 1.
3. **Nothing shows while a card is written.** A card appears only when its
   call is complete and valid. The provider stream's `ToolUseStart` (id and
   name, early) is dropped by `client_events::project`
   (`vak-server/src/client_events.rs`); the client's first sign of a call is
   `ToolCallStart`, which carries the finished `args_json`. A long table is
   a silent wait.
4. **Generated markup does not match the app.** `sandboxedSrcdoc`
   (`vak-client-ui/src/safeUrl.ts`) writes the CSP and the markup and
   nothing else, so a markup preview has the browser's defaults in every
   theme. With `connect-src 'none'` it also cannot load a chart or diagram
   library unless the model inlines one.
5. **No page stays current.** A card is a snapshot of its turn. A
   person who wants a weekly view of something re-asks; the Library,
   automations and grants exist but nothing composes them into one living
   page.
6. **MCP servers' declared UI is unused.** A server may declare HTML UI
   resources (`ui://`) linked to its tools; Vakyartha shows the tool's text
   result only. AGENTS.md forbids adopting a third-party UI bridge, so this
   is a decision, not a defect.
7. **No place on a map.** Doc 92 deferred a map form until real turns
   need it; places go in a list or details card with a link.
8. **Format choice is not measured.** The plan check (doc 86) measures
   whether planned parts were delivered, never whether a card helped.

## 2. What does not change

- A card is a form, decided by the model's plan, never by reading the
  request's words or by a count (doc 92 §2). No classifier picks a format.
- Generated markup stays the escape hatch, never the default. Previews keep
  their opaque origin and closed network (doc 66 §3.1).
- Builders check structure, never sizes: no row, column or item caps.
- Model-visible means logged (invariant 1); a person's action on a card is
  steering, never permission (invariant 32).

## 3. Stages

Ordered by value for effort. Q stages are small and independent; P stages
are projects.

### Q1 Theme in markup previews

- `sandboxedSrcdoc` writes, after the CSP and before the markup, one
  `<style>` defining the active theme's tokens as CSS variables on `:root`
  (`--vak-bg`, `--vak-fg`, `--vak-muted`, `--vak-accent`, `--vak-rule`,
  `--vak-font`, `--vak-font-mono`, `--vak-radius`), read from the app's
  computed root style when the document is built, plus
  `color-scheme` and `body { font-family: var(--vak-font) }`.
- A theme change rebuilds the open preview's document; nothing else
  reloads.
- `emit_markup_preview_card`'s description names the variables, so the
  model can use them. No field is added to the schema.
- The variables are the only thing written; no script, no network, no
  change to the sandbox value.

Done when: a markup preview that uses the variables follows every theme
switch in the running app at 1440 × 900 and 390 × 844, light and dark;
`tests/preview-isolation.mjs` and
`ui_preview_never_carries_isolation_settings_from_the_model` still pass;
doc 66 §3.1 says what the frame receives.

### Q2 A placeholder while a card is written

- The agent loop, on the provider's `ToolUseStart` for a tool that
  `presents_cards`, sends `AgentEvent::CardDrafting { id, block }`, the
  block taken from the tool's own declaration, never from its name.
- `client_events::project` projects it as `ClientEvent::CardDrafting`
  (deltas only, invariant 4). It is not a ledger entry: it shows only that
  a call started, and nothing is model-visible.
- The client draws an empty card shell of that form in the answer at the
  place the plan gives the deliverable, or after the sections when it
  names none, with a quiet "Drawing a <form>" line. It is replaced by the
  recorded card on `ToolCallEnd` success, and removed on a refusal, an
  error or a run that ends.
- Never a partial card: nothing from the streaming arguments is drawn.

Done when: on the dev route a long table call shows its shell within a
second of the call starting, the recorded card takes its place, a refused
call leaves no shell, and a test in `client_events` covers the projection.

Built (2026-10-16). The block is `Tool::card_block`, which a card tool
declares as its form. A card call is the loop's own (`show_card`) and sent
no `ToolCallEnd`; it now sends one when the call is checked, so a refusal
removes its shell at once. A recorded card's shell stays until the card
with that call's `tool_call_id` is in the timeline, because the card is
projected after the call ends. Only the conversation's live turn draws
shells: the chat mounts one timeline per turn, and each would otherwise
draw them. Measured live: one shell per call, replaced by its card. On the
dev route the native Ollama chat API delivers a tool call whole, so
`ToolUseStart` arrives with the arguments complete and the shell stands only
while the call is checked; a route that streams arguments shows it while
they are written.

### Q3 Card actions as records

- A new session entry, `CardAction { presentation_id, action, values }`,
  traced like every row (`every_ledger_row_type_is_traced`), declared in
  the records reference (`VAK_UPDATE_RECORDS=1`). `action` is a closed
  set: `choose` (an option of a comparison, a value of a control) and
  `mark` (a list item ticked or unticked). `values` is typed by the form.
- `choose` is a directive from the person: it starts a turn exactly as a
  typed message does, and `derive_messages()` renders it as the person's
  choice naming the card, so the model reads what was chosen and from
  which card. It replaces `optionInteractionFor`'s composer text; the
  composer is left alone.
- `mark` starts no turn. It is card state: the projection serves each
  card's current marks, and the next turn's tail carries the marks made
  since the last turn, so the model knows the list's state without a
  replay (a rollup over the session's `CardAction` rows, per the data
  rules).
- `ControlSource` is the transport's: a client action is human steering.
  It authorizes nothing; every gated call still reaches the permission
  engine.
- One route, `POST /sessions/{id}/card-actions`, checked through
  `require_bearer` and, for a guest, `enforce_participant_audience`.

Done when: a chosen option starts a turn whose transcript shows the
choice; ticks survive a reload and a second surface; `derive_messages()`
reconstructs both; the composer receives no text; a guest without access
is refused through the real layers.

Built (2026-10-16), with two decisions of the maintainer. A session
ledger is not a record chain, so no rollup Document can fold it: the
next turn's `<card_marks>` block is read from the `CardAction` entries
between the previous directive and this turn's, as the other tail
sections are read (`TailSections::card_marks`). A choice records its
`CardAction` and starts the turn through the ordinary run path with a
user message whose `MessageMeta::card_action` names that entry; its words
are `card_actions::choice_text`, and the route returns them so a client
shows the person's message. Card actions are the owner's alone: a guest
is refused behind `require_bearer` and `enforce_participant_audience`. A
choice names a comparison's option or a steps card's choice (the steps
card is where "Use this" is drawn); a tick names a checkable list's item
by place and label, and anything else is refused with what does not
fit. While a turn runs the ledger is the agent's, so an action then is
refused and the client puts a tick back. The projection serves each
card's state as `OutputItem::card_state`. Verified live on the dev route:
a tick survives a reload and the next turn's answer named the ticked
item; a choice is verified through the HTTP test only, since no live
turn drew a steps card with choices.

### P1 A controls form

- A nineteenth form, `controls`, one card tool with one flat schema
  (doc 89): `inputs` (each `label`, `name`, `kind` of number, range,
  choice, toggle or date, `value`, optional `unit`, `min`, `max`, `step`,
  `options`), `outputs` (each `label`, `unit`, `expression`), and
  `example` (the inputs it was checked with and the outputs it gives).
- `expression` is written in a closed language evaluated in the client:
  numbers and input names, `+ - * / ^ %`, parentheses, comparison,
  `if(c, a, b)`, `min`, `max`, `round(x, n)`, `floor`, `ceil`, `abs`,
  `sqrt`, `log`, `exp`, and `sum`/`avg` over a list input. No string
  operations, no calls out, no loops. The same evaluator runs in the
  builder (Rust) and the client (TypeScript), held equal by a shared test
  table.
- The builder parses every expression, refuses an unknown name or
  operator with the position, and evaluates `example`: a result that does
  not equal the stated output is refused with both values, so a card
  cannot show a formula that does not compute what the model said.
- The client draws inputs with the forms' controls and recomputes outputs
  on every change. A changed value is local until the person chooses
  "Ask about this", which sends a Q3 `choose` with the current values.
- Text form for channels: the example as a two-column table.
- Added to `vak_delivery::FORMS`, `vak_presentation::PRIMITIVES` and the
  client's `FORMS`, checked by
  `the_card_kinds_are_exactly_the_types_the_client_draws`.

Done when: a bill split, a loan payment and a unit conversion asked on the
dev route draw as working controls (checked in the browser at both sizes
and themes); a wrong example is refused and repaired by the loop; a
channel receives the example table; the evaluator equality test passes.

### P2 Pages that stay current

- A living page is a Library artifact whose content is composed cards
  (doc 87's outline) and whose versions are produced by an automation: a
  `Trigger` whose run ends by recording a new version of the page
  (`Tool::artifact` claim), naming the previous version as parent.
- It opens in the Canvas as a new subject kind (`page`), with its version
  history and comments, and is shared through `vak_core::grants`. A grant
  reaches the page, never the conversation that made it.
- No hosted runtime and no code: a page is cards. When the run fails, the
  page keeps its last version and the inbox says why (`RoutineFailed`).

Done when: a weekly page made by a routine on the dev route gains a
version per run, keeps the old ones, opens in the Canvas, and a guest with
a grant reads it and nothing else.

### P3 Libraries for markup previews

- A fixed set of vendored libraries at pinned versions (a chart library
  and a diagram library), served from the preview origin under a fixed
  path, listed in `emit_markup_preview_card`'s description. `connect-src`
  stays closed; only `script-src` names the preview origin's path.
- Each library is a workspace dependency under the dependency rule:
  pinned, justified, and rebuilt into the bundle manifest.

Done when: a markup preview drawing a chart with the vendored library
works offline in the opaque frame and the isolation tests pass.

### P4 MCP UI resources

The maintainer chooses, recorded in docs/decisions.md:

1. **Text only** (today): the tool's text result is shown; a declared UI
   resource is ignored.
2. **Vakyartha's own renderer**: a `ui://` HTML resource is fetched through
   the MCP broker, drawn in the opaque sandbox exactly as a markup preview,
   and its messages to the host are a closed set Vakyartha implements; a
   tool call from the UI is an `Ask` through the permission engine with
   the server's declared reach, never auto-approved.

### P5 A map form

Only when logged turns show places forced into lists. Places with
coordinates drawn without network tiles (a schematic of points and
order), or tiles behind an owner-granted network allowance declared like
an MCP server's hosts (invariant 13).

### P6 Measuring format choice

A live eval (`vak eval --live`) of plain requests, each scored on whether
its cards helped (a rubric over the plan, the receipts and the answer),
reported beside latency and tokens, as the outcome contract asks of every
evaluator.

## 4. Order

Q1, Q2 and Q3 are independent and each is a commit of its own. P1 depends
on Q3 for "Ask about this". P2 depends on nothing here. P3 waits on Q1. P4
waits on the maintainer. P5 and P6 wait on evidence.
