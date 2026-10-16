# History and discovery lookups on the smallest route (2026-10-16)

Status: reference

Route: Ollama `qwen3.5:0.8b-mlx`, the installed route. Method as in
docs/audits/grounded-cards-2026-10-15.md: first requests captured behind
the Ollama proxy, replayed unchanged 16 times; ten scenarios, two of them a
five-turn conversation (`followup` refers to the turn before; `far` asks
for a fact from turn 1, outside the carried card, so only `recall` can
reach it).

## Cause

About half of all first steps were `recall`, `session_search` or
`find_tools` on requests that needed none. Removing the prompt's history,
discovery, grounding or `find_tools` wording one at a time did not move it.
Rewording the tools did not either: told that `session_search` never
searches this conversation, the model still chose it for `far` (1 of 16
`recall`). Two look-alike history tools were the fault: for `far`,
`session_search` (which excludes the current conversation) was chosen 13
times in 16. `find_tools` was called for card tools already loaded
(compare 8 of 16, table 5 of 16).

## Change

One history tool: `recall` with `query` (this conversation's turns plus,
through `vak_core::elsewhere`, other conversations, notes, the profile,
sources and deliverables, filtered by Agent and audience) and `open`
(`#3`, `pres:`, `ev:`, a hit's `turn:` or `conversation:…/turn:…`).
`session_search` removed. No discovery tool: a "More tools" entry is
called by its name and promoted for the rest of the turn; `find_tools`
removed, and no wire is sent a deferred tool (Anthropic's `defer_loading`
and tool search dropped). Simulated first (both removed together gave the
best numbers; each alone helped little), then built.

## Results

Replayed first step, before (`e82f4f08b` plus grounded cards) and after,
on-task / lookup / bad / other of about 16:

| scenario | before | after |
|---|---|---|
| remember | 6/6/0/4 | 8/3/2/3 |
| name | 0/16/0/0 | 0/13/1/2 |
| compare | 0/15/1/0 | 1/2/6/4 |
| list | 1/13/2/0 | 3/5/6/2 |
| define | 7/3/3/2 | 7/2/4/3 |
| steps | 3/8/1/4 | 5/4/1/5 |
| table | 0/9/2/5 | 0/5/4/6 |
| code | 15/0/0/1 | 11/0/0/5 |
| followup | 0/13/0/3 | 0/8/3/5 |
| far | 3/13/0/0 | 9/0/2/5 |
| all | 35/96/9/19 of 159 | 44/42/29/40 of 155 |

Lookups halved and `far` tripled with no dead-end tool left. Bad calls
rose: mostly invalid card arguments (regenerated under the schema in the
loop) and catalogue tools called by name (valid in the loop).

Live, in the loop, three five-turn conversations: `far` answered with
`recall` once; twice it called `entity_query` by name, which asked for
approval on this dev server and was stopped. "remember that" called
`remember` at once; "whats my name?" used `recall`.

## Open

- `entity_query` reads the entity graph that `recall` already searches: a
  second look-back tool, the same confusion one level down.
- The first step still reaches for `recall` on fresh requests often
  (`name` 13 of 16 though the name is in the carried card).
- Card argument validity on this route (bad calls 29) is measured only as
  written; whether the loop's regeneration repairs them was not measured.
