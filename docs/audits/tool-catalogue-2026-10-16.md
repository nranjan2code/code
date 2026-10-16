# Tool catalogue audit (2026-10-16)

Status: reference

A read of every tool a turn can admit and every mechanism a run can add,
from the code (`Core::scoped_tools`, `Core::tool_declarations`, the
turn-bound tools, the card tools, the loop's own), with the design doc each
answers to. No code was changed. Request sizes are from a first request
captured behind the Ollama proxy on the dev server (`vak-inv-live`, a
throwaway home with no account, skill or MCP server; route Ollama
`qwen3.5:0.8b-mlx`).

How a tool reaches the model (`capability/surface.rs::build_tool_surface`):
a tool that declares `always_loaded`, a card tool, or a tool that declares
no `serves` domain is sent with its schema; a grounded card
(`Tool::grounded_in`) once the conversation holds its fact; everything else
is deferred and listed under "More tools" (name and first sentence),
loaded from the step it is called by name. Reach is the
`vak_permission::Reach` a tool declares; a tool that declares none is
`Other` (read-only mode denies it, workspace-write asks).

## 1. Built-in tools

| Tool | Lives in | For | Reach | Serves | Surface | Notes and dependants |
|---|---|---|---|---|---|---|
| `read` | vak-tools read.rs, worker | read a workspace file as text | ReadPath | filesystem | loaded | doc 04; invariant 10 |
| `write` | vak-tools write.rs, worker | write a file; an artifact claim | WritePath | documents | loaded | doc 04, 82 |
| `edit` | vak-tools edit.rs, worker | exact replace in a file | WritePath | documents | loaded | doc 04 |
| `bash` | vak-tools bash.rs, worker | run a shell command in the workspace | Command | code-exec | loaded | doc 04, invariant 35; grounds the terminal/test cards |
| `glob` | vak-tools glob.rs, worker | list paths by pattern | ReadPath | filesystem | loaded | doc 04 |
| `grep` | vak-tools grep.rs, worker | search file contents | ReadPath | filesystem | loaded | doc 04 |
| `doc_read` | vak-tools doc_read.rs, worker | anchored, labelled read of Office, PDF and structured text; the `base_digest` and anchors `office_apply` edits against | ReadPath | filesystem, documents | loaded | doc 72, 77, invariants 38, 39. Not a second `read` |
| `office_apply` | vak-tools office_apply.rs, worker | typed ops on Office/PDF, a draft for Review | WritePath | documents | deferred | doc 72, 77; `delivered_file` receipt for the plan check |
| `webfetch` | vak-tools webfetch.rs, worker parses (`HtmlRead`) | GET a known URL; readable text, links or source | Network | web, live-data | loaded (`[tools]` switch) | doc 04, 15; grounds `emit_sources_card` |
| `browse` | vak-tools webbrowse.rs, worker | render a URL in headless Chromium; same three outputs | Network | web, live-data | deferred (`[tools]` switch) | same guard and output contract as `webfetch` |
| `recall` | vak-tools recall.rs, answered by the loop | everything before this turn: this and other conversations, notes, profile, entities, sources, deliverables (`query`), `#n`/`pres:`/`ev:`/`turn:` (`open`) | ReadStore | memory | loaded | doc 68 §3, 85; the only history tool since 2026-10-16 |
| `plan` | vak-tools plan.rs, loop's own | declare deliverables for the plan check | none (loop) | — | loaded | doc 86 |
| `remember` | vak-core learning.rs | keep a memory note | Learning | memory | loaded (`[memory]` write) | doc 23, 26 |
| `forget_memory` | vak-core learning.rs | remove one note by its recall id | Learning | memory | deferred | doc 23 |
| `propose_skill` | vak-core learning.rs | propose a skill, or a program with tests (an ability) | Learning | memory | deferred (`[memory]` proposals) | doc 26, 93; the self-evolution entry point |
| `entity_record` | vak-core entities.rs | write a typed entity with attributes and relations | **Other** (none declared) | memory | deferred (`[memory]` write) | doc 23; recall reads entities |
| `data_query` | vak-core data_engine.rs, **in the Core process** | select/filter/group/sort over CSV, JSON or a Markdown table passed inline | **Other** (none declared) | documents | deferred | doc 72 F3, F10 (to move to the worker, open) |
| `automations` | vak-core tools_automations.rs | list and manage the owner's automations (triggers) | Schedule | orchestration | deferred | doc 94, 38; `triggers::*` |
| `commitments` | vak-core tools_commitments.rs | read this Agent's commitments | ReadStore | memory, orchestration | loaded (when commitments on) | doc 47 |
| `mail_calendar` | vak-core mail_calendar.rs, worker | read connected mail and calendar | ReadStore | mail, calendar | **loaded always**, with or without an account | doc 80; largest schema in every request (below) |
| `agent_network` | vak-core agent_network.rs | send to or receive from an authorized Agent workspace | **Other** (none declared) | messaging | deferred (only when the broker is enabled for the workspace) | doc 25, 08 "Provenance" (BOTH) |
| `skill` | vak-core skills.rs, turn-bound | load a skill's instructions (and a program's declaration) | ReadStore | per skill | loaded (when any skill) | doc 41, 93 |
| `mcp` | vak-mcp tool.rs, turn-bound | list or call an MCP server's tools | Integration (`server/tool`) | per server | loaded (when any server) | doc 09, invariant 25 |
| `task` | vak-agent task.rs, turn-bound | delegate to a worker | Other | orchestration | deferred (when workers on) | doc 84 |
| `workers` | vak-agent workers_tool.rs, turn-bound | look at, message, wait for, stop own workers | Coordination | orchestration | deferred | doc 84 |
| `ask_parent` | vak-agent questions.rs | a worker's question to its parent | Coordination | — | in workers only | doc 84 |
| card tools (20) | vak-core presentation_tools.rs, `show_card` in vak-agent | one per form (doc 92), plus `emit_ability_table_card` and `emit_ability_chart_card` filled from an ability's answer | Display | — | 13 loaded; 7 grounded wait in "More tools" | doc 87, 92; packs add kinds (doc 91) |

Wrappers, not tools: `broker::BrokeredTool` (worker boundary, forwards
every declaration), `write_lease` (background writers). Ability tools
(`capability::CapabilityTool`) are listed in section 2.

Measured request (first step, no account, skill or server): 26 tools
loaded, 17 in "More tools". Tool schemas 42,584 chars beside a system
prompt of 11,335. Card schemas are 22,694 (53 %); `mail_calendar` alone is
7,486 (18 %), larger than any card; `doc_read` 2,326; `recall` 1,729.

## 2. What runs can add

| Mechanism | Added by | Admitted | Listed to the model | Versioned | Withdrawn / removed |
|---|---|---|---|---|---|
| Ability (skill with a program, doc 93) | `propose_skill` with `source`; the owner | tests run by the runtime in their own run; self-admitted for its own Agent when tests pass, no host, no install, no foreign words and `[owner] abilities_go_live` (`skill_programs::review_reason`); else owner review | **twice**: as a tool under "More tools" (or loaded when it declares no `serves`) and in the Skills list | Library artifact versions, each naming its parent | `vak skills turn-off`, decline; `promote` to Shared is the owner's |
| Skill (instructions) | `propose_skill`, plugins, Shared/project folders | review queue for proposals; layers narrowest-wins | Skills list ("load with `skill`") | proposals as Documents; installed skills by folder | delete/disable by layer; channel allow/deny |
| MCP server | owner config (`/config/mcp`, desktop manager), plugins | project layer demoted until trusted; spawned lazily on demand (invariant 25) | "MCP servers" section: server names and the tool names last discovered | none | config removal; channel overlay deny; idle eviction |
| Hook | owner config, plugins | project hooks demoted until trusted | not model-visible | none (`enabled = false` kept, invariant 21) | disable or remove in config |
| Plugin | `vak plugins install` (catalogue, signature check) | enable per scope; manual review, pinned commit, local-only options | through what it contributes (skills, packs, MCP, hooks, commands) | `versions`, `update`, `rollback` | disable, remove, key revocation |
| Card pack (doc 91) | plugin, person's save or import | declarative only; channel allow/deny | adds kinds to its block's `semantic_type` enum; no new tool | digest per spec in the presentation library Document | suppress per layer; disable the plugin |

Background writers that are not tools but add durable state the model
later reads through `recall`: **reflection** (`reflection.rs`, opt-in
`[memory] reflection`) and **consolidation** (`consolidation.rs`, run at
every compaction, `lib.rs` after the compaction write).

## 3. Overlaps and gaps

Each with its evidence. "Measured" means a live or captured run; "code"
means read from source only.

**O1. Consolidation decides by word lists and copies notes (code).**
`consolidation.rs` promotes a note to an `invariant` note when its text
contains "always", "never", "must" or "invariant", or recurs in two
sessions, and calls two notes contradictory from a pair list ("tabs" /
"spaces", "enable" / "disable", "always" / "never", …). It also distils
entity records from `fact` notes. It runs at every compaction with its
result discarded (`let _ =`). Nothing reads an `invariant` note but recall
(invariant 28 keeps it out of the prompt; test
`memory_notes_never_become_prompt_guardrails`), so a promotion is a second
copy of the same note in recall's results: duplicates that grow with use.
This breaks "No word lists, no tuned limits".

**O2. Two memory writers, two permissions (code + measured).**
`remember` (Learning, loaded) and `entity_record` (no declaration, so
`Other`, deferred) both write durable memory for this workspace that
`recall` searches. Because `entity_record` declares nothing,
workspace-write asks the person and read-only denies it, where `remember`
does neither. The history audit (docs/audits/history-lookup-2026-10-16.md)
saw the read side of the same split: `far` called `entity_query` by name
twice and stopped on its approval, which is why it was folded into
`recall`. The write side is the same split one level up.

**O3. `data_query` (code).** Takes the table inline, so the model copies
every value into the arguments (the failure the ability cards were built
to avoid); parses in the Core process, which doc 72 F3 marks as owing a
move to the worker under invariant 14; declares no reach (`Other`, asks).
Its job is covered by the shipped `table-profile` ability (path or text,
in the worker sandbox, tested) for profiling and by `bash` for anything
else. Never called in any captured run this week.

**O4. An ability is listed as a skill and a tool, under a header that
says it is not one (code).** `skills.rs` builds a `CapabilityTool` named
after the skill; `prompt_section_from_capabilities` lists every skill,
programs included, under "a skill name is never a tool name", and the
system prompt repeats it. A model that reads the header has been told not
to call the ability the catalogue offers it. Not yet measured: the dev
home has no skills; needs a home with `everyday-starter`.

**O5. `mail_calendar` is always loaded (measured).** 7,486 chars, 18 % of
every request's tool bytes, on a home with no account. `always_loaded`
has been `true` since the tool was added (7021a942e) with no recorded
reason; its `serves` (mail, calendar) would defer it like every other
domain tool.

**O6. Ability cards offered with no ability (measured).**
`emit_ability_table_card` and `emit_ability_chart_card` are in "More
tools" on a home with no ability, where every call can only be refused.
One was called on a `followup` turn in the onehist runs (refused). The
other five grounded cards wait for a fact the conversation can come to
hold; these two wait for a tool that is not admitted.

**O7. `webfetch` and `browse`: one job, chosen by a property of the page
(code).** Same reach, `serves`, guard, worker parser and three output
formats; they differ in whether JavaScript runs. The model must guess
from a URL which one a page needs; `webfetch` is loaded and `browse`
deferred. `webfetch`'s description still says "discover an available
search tool first", from before `find_tools` was removed: there is no
discovery step now. (Their fixed totals are listed as protection in doc 88
§5a, not a finding here.)

**O8. Three indexes, three call conventions (code).** "More tools" (call
by name), the Skills list (`skill({name})`), the MCP section
(`mcp({action, server, tool})`). Each is byte-stable and per-turn, so the
cache holds, but an ability sits in two of them (O4) and the MCP section
names every discovered tool of every server, so it grows with the
servers an owner installs. For a commercial product this is the one that
does not stay small; it is a design question, not a defect today.

**O9. Undeclared reach (code).** `entity_record`, `data_query` and
`agent_network` declare no permission and so read as `Other`. For
`agent_network` that is safe (it leaves the workspace and holds BOTH
provenance), but it is an accident of the default, not a declaration.

**O10. Reflection's tuned caps (code).** `MAX_NOTES_PER_RUN = 2`,
`TAIL_MESSAGES = 24`, `MAX_TOKENS = 700`: numbers picked in code, which
the AGENTS.md rule removes when found. Opt-in, so lower risk.

What is not an overlap, though the names suggest one: `read` and
`doc_read` (plain text vs anchored, labelled read Review edits against);
`recall` and `commitments` (history vs the commitment portfolio, read
only); `task`/`workers` and `automations` (now vs later); `task` and
`agent_network` (own workers vs another Agent's workspace). Manageability
holds for abilities: one Library artifact per name with parent-linked
versions, duplicate screening on proposal, turn-off and promote by name.

## 4. Live check of the `open` + `query` fix

Three five-turn conversations (`nturn.sh`, labels `both`, `both-n2`,
`both-n3`); turn 5 asks "what is my cat called?" (answer: Miso), told in
turn 1. The run left running at commit 3f11319ed wrote no outcomes, so it
was run again on this session's build of 3f11319ed.

| Run | Turn 5 calls | Steps | Answer |
|---|---|---|---|
| `both` | `recall` ×4, `glob` | 7 | "your cat is called **Miso**" |
| `both-n2` | `recall` ×2 | 4 | "Miso" |
| `both-n3` | none | 1 | answered as the model itself, no cat named (wrong) |

Two of three right, both through `recall`; the third never looked. Turn 4
(`followup`, "and for a rainy day instead?") ended aborted in all three
(one at the script's 300 s stop), reaching for `read`, `glob`, `grep`,
`bash`, `edit` or `webfetch` with 1 to 5 refused calls: a separate open
line, not part of this audit. Outcomes are in `/tmp/vak-live/measure/both*/
outcomes.json`.

## 5. Proposal, ranked

Each is a redesign, no shim, with one measurement. "Must not break" lists
what the change touches that a turn, a safety invariant or self-evolution
relies on.

1. **Remove consolidation's word-list promotion and pair-list
   contradiction check (O1).** Keep compaction; drop the `invariant`
   copies and the entity distillation from notes (entities become only
   what a person or the model recorded). If consolidation has no job
   left, delete the module and its call. *Blast radius:* vak-core
   consolidation.rs, the compaction path in lib.rs, the `consolidation`
   report type and any admin/CLI that shows it, reflection's `invariant`
   kind (keep as a kind a person or model may write). *Must not break:*
   compaction outcome and receipt, recall over notes, erasure lineage
   (derived rows), invariant 28's test. *Measure:* none needed beyond
   targeted tests; behaviour removed, not tuned.
2. **One memory writer (O2, O9).** Fold `entity_record` into `remember`:
   a note may carry a type, attributes and relations; one tool, one
   reach (Learning), one store recall reads. *Blast radius:* entities.rs,
   learning.rs, recall's entity source, `vak entities` CLI, server
   entity routes, the catalog source, erasure. *Must not break:* existing
   recall of entities (8.0 baseline: no migration, a fresh store),
   invariant 37 isolation, the `[memory]` write switch. *Measure:* the
   history scenarios (`remember`, `name`, `far`) before and after.
3. **Remove `data_query` (O3).** Profiling is `table-profile`'s, querying
   is `bash`'s or a future ability's (doc 93: tested, sandboxed,
   versioned). Closes doc 72 F3/F10 by removal. *Blast radius:*
   data_engine.rs, scoped_tools, docs 72 and any eval that names it.
   *Must not break:* invariant 14 (improves), table cards. *Measure:* the
   `table` scenario.
4. **List an ability once (O4).** A skill with a program is a tool: it
   appears in "More tools" (or loaded) and not in the Skills list; `skill`
   still loads its procedure and declaration for improving it; the
   header's "never a tool name" clause goes. *Blast radius:* skills.rs
   prompt section, prompt seed text (doc 07 diff note), surface tests.
   *Must not break:* self-evolution (load-to-improve, new version naming
   its parent), the byte-stable prefix. *Measure:* a home with
   `everyday-starter`, a date question, before and after.
5. **Defer `mail_calendar` (O5).** Drop `always_loaded`; it is found by
   name like every domain tool. *Blast radius:* one declaration, surface
   tests, doc 80. *Must not break:* routine reads (an automation's turn
   calls it by name), the account-selection rules in its description.
   *Measure:* request size and one mail-shaped question on a home with a
   connected account.
6. **Admit ability cards only with an ability (O6).** The two
   `emit_ability_*` tools are admitted when the turn admits at least one
   program tool. Admission follows what is installed, not the reading
   (doc 88 §12.1). *Blast radius:* presentation_tools, `scoped_tools` /
   turn admission, the card-forms equality test. *Must not break:* the
   card-forms equality check, invariant 31 (an ability added mid-session
   admits them at the next turn). *Measure:* the `followup` scenario.
7. **One web reader (O7).** `webfetch` with a `render` argument the
   model sets; `browse` removed; the stale discovery sentence goes.
   *Blast radius:* webfetch.rs, webbrowse.rs, `[tools]` switches
   (`effective_browse`), channel policies naming `browse`, the sources
   card's Retrieved grounds, doc 04. *Must not break:* the private-range
   guard, the worker parser, provenance (untrusted) and the network ask.
   *Measure:* a JS-built page and a plain page, before and after.
8. **Declare reach for `agent_network` (O9).** Network, explicitly.
   *Blast radius:* one declaration. *Must not break:* its BOTH
   provenance ask.
9. **Design question, not a build: one index (O8).** Whether MCP tools
   and skills join "More tools" as named entries (an MCP tool as
   `server/tool`, loaded by name like any deferred tool) so the model has
   one catalogue and one way to call. Needs a doc before any code
   (invariant 25: discovery only on demand).
10. **Reflection's caps (O10).** Remove or move to `[control.limits]`
    with a shown default; decide with 1.
