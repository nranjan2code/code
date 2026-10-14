# 91 — Presentations are capabilities

Status: implemented: stages 1 to 5 built 2026-10-14; direction from the maintainer on 2026-10-14 ("like the
others we have: tools, MCP, skills; with the admin, inheritance, security,
config and enable/disable"). Builds on docs/design/41-capability-registry.md,
docs/design/39-plugin-ecosystem.md, docs/design/57-adaptive-presentation-runtime.md
and docs/design/87-composed-answers.md.

## 1. What is wrong

Read from the code on 2026-10-14:

1. **The kinds a card can be are compiled in.** Each card tool's
   `semantic_type` enum is the static `SHAPES` table
   (`vak-core/src/presentation_tools.rs`). The social add-ons' twelve types
   are typed into it, into the `core` skill of `vak-delivery`'s registry,
   and into the client's renderer registry. A pack an add-on installs for a
   new kind is unreachable: the schema refuses the call before the pack is
   ever selected. This is the table of instance names invariant 31 forbids.
2. **Packs are not in the capability registry.** The registry owns tools,
   skills, MCP servers, hooks and commands (doc 41); a pack has its own
   library Document, its own activation model and its own screens, so it is
   missing from the one inventory the model, `doctor` and the admin console
   render, and its changes are not published at a turn boundary.
3. **Scope order runs backwards.** `PresentationLibrary::select_preferred`
   picks the user scope before the workspace scope; invariant 27 makes the
   Shared (user) layer the base and the project layer narrower, so the
   project's choice wins.
4. **A skill's mid-turn change was misreported.** Skills are bound per turn
   (`TurnCapabilities::frozen_skills`), so an edit applies from the next
   turn, as invariant 31 asks; but loading one edited during the turn said
   "start a new session". The message now says the new text is offered from
   the next turn.
5. **The client draws no composition.** `Row`, `Stack`, `Group` and
   `Section` are host primitives a pack composes with (`vak-presentation`
   says a metric grid is a `Row` of `Metric`s), and the client draws each
   through its generic fallback, one column.

## 2. The model

A **presentation** is a capability of kind `presentation`, named by its spec
id. Like a skill it affects what is shown, never what is authorized.

- **Where it comes from.** The one store stays the presentation library
  (`Core::scope().presentations()`, a Document): built-in seeds, packs an
  enabled plugin contributes (doc 39), and definitions a person saved or
  imported. No second store or file layout is added (invariant 30).
- **Which block it draws.** A pack names the card block whose payload it
  reads in its spec's `block` (`steps`, `table`, `research`…, the card
  tool's name without `emit_` and `_card`). The kinds it `accepts` become
  kinds of that block for as long as the pack is active: the card tool's
  `semantic_type` enum is its own kinds plus every active pack's kinds for
  it, read when the turn's tools are assembled, so installing, enabling or
  disabling a pack takes effect from the next turn of every live session
  and never needs a restart (invariant 31). A pack kind is built by its
  block's builder exactly as the block's own kind is.
- **Enable and disable.** A plugin's packs are read from the enabled
  plugins whenever the library is read (`Core::effective_presentations`,
  composed by `presentation_store::effective` with the seeds), as its
  skills are: enabling the plugin activates them in the workspace unless
  the owner suppressed one or an explicit activation already draws one of
  its kinds, and disabling the plugin removes them on the next read. No
  copy is written to the library and no refresh activates anything
  (doc 39). The social add-ons' own pack route is removed (invariant 30).
- **Inheritance.** Built-in → Shared (user) → project, narrowest wins
  (invariant 27). A project may suppress an inherited pack, as it may
  already suppress a seed.
- **Security.** A pack is declarative: it compiles into the closed
  primitive set and runs no code, fetches nothing and reads no file
  (doc 57 §4). It is admitted like a skill: channel policy can allow or
  deny it by name, and a deny wins (invariant 20).
- **Inventory.** Each active pack is a `Declaration` of kind
  `presentation` with its origin (built-in, plugin, user), digest, block and
  kinds, so it appears wherever every other capability does.
- **History.** A card records the block it was built by. When the pack
  that drew its kind is later disabled, the card is drawn by its block's
  own renderer, never as raw data.

## 3. Presentation packages

A presentation package is an ordinary plugin (doc 39) with two components:
a skill saying when and how to compose an answer from blocks, and packs
saying how those blocks look. Nothing in the harness names one. The model
decides from a skill's description whether to load it; nothing loads a
skill by matching the request's words.

### 3.1 Shipped

`packages/answer-layouts` ships with Vakyartha, installed and enabled like
the starters (docs/design/05-config.md, "Capability seeds"): three skills
(`plan-in-time`, `decide-between`, `how-to-guide`) and one pack
(`at_a_glance`, a kind of the stats block drawn as a row of figures).

### 3.2 Scope and policy

- Pack selection takes the workspace's activation before the user's
  (`PresentationLibrary::select_preferred`), as invariant 27 orders layers.
- A channel's `presentations_allow` and `presentations_deny` (pack ids,
  matched like skills) admit or remove a pack: a denied pack is not in the
  turn's capabilities and adds no kind to the card tools. The admin channel
  editor offers the active packs.
- The capability report lists packs beside every other kind; the model's
  list of unusable capabilities leaves them out, since a pack is not called.

## 4. Stages

1. **Kinds from packs.** `block` on the spec; card kinds from active packs;
   the record keeps its block; the social add-ons' packs declare
   `block: research` and their types leave the three compiled lists.
   `CapabilityKind::Presentation` declarations.
2. **Composition drawn.** The client draws `Row`, `Stack`, `Group` and
   `Section` as layout.
3. **Live skills.** The mid-turn message is corrected (done 2026-10-14).
4. **Packages.** `answer-layouts` (§3.1). Its first live turn on the
   home's local route is recorded in docs/plans/open-items.md.
5. **Scope and policy.** Workspace before user in pack selection; channel
   allow and deny for presentations; the admin and desktop capability
   screens list presentations beside skills.
