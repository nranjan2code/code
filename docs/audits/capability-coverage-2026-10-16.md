# Capability coverage harness (2026-10-16)

Status: reference

A domain-neutral harness that judges a contract change across everything a
turn can reach, in place of a few hand-picked prompts. This records the
method, the harness and a first partial run. The full run was stopped at
the maintainer's request after 8 of 135 cases, so the results below are a
first reading, not a measurement of every capability.

The harness is run by hand only. Neither `scripts/gate.sh` nor
`scripts/release.sh` calls it, and the repository has no CI configuration
that could.

## Harness

`scripts/capability_coverage.py`, one script with five steps (`all` runs
them in order):

- `registry`: starts a dev server on a fresh home (`VAK_HOME` under
  `/tmp/vak-coverage`), seeds the shipped packages (`POST
  /onboarding/seed`: everyday-starter, developer-starter, answer-layouts),
  binds one turn and reads `GET /sessions/{id}/interface`. That route is
  new and read-only: it returns the last turn's bound interface (every
  admitted tool's name, description and schema, the deferred names, the
  system prompt) beside the published capability set, each capability
  with its declared `serves`. Each capability becomes a *unit* with a
  digest of its own declaration. The owner call of a unit comes from its
  kind alone: a tool is called by its name; an instruction skill is
  `skill` with its `name`; a card pack is the card tool whose
  `semantic_type` enum holds the pack's kind; an MCP tool is `mcp` with
  its server and tool. No unit is named in the harness.
- `corpus`: keeps `scripts/capability_coverage/cases.json` in step with the
  registry by declaration digest. A unit that is new or whose declaration
  changed is listed for generation; a unit that left the registry loses
  its cases. The generation prompt shows the unit's whole declaration and
  the one-line catalogue of every other unit, and asks for two one-turn
  cases, one two-turn follow-up (turn 2 needs turn 1's result) and the
  path of units each turn should use. Pairs are formed deterministically
  (each unit with the one half the sorted list away) and get one case
  that needs both. `--generate PORT` sends the prompts to a Vak server on
  a hosted route; `--emit-prompts` and `--answers` let an answer set come
  from elsewhere. A capability added at runtime gets cases at the next
  `registry` plus `corpus`.
- `run`: per route, each case live through the server API on a fresh
  home, files written into the workspace first. On an Ollama route the
  server's provider points at an in-process capture proxy (from
  `scripts/measure_tool_form.py`), so each case's first request is kept.
  The turn wait (`--turn-timeout`) is the harness's own, never Vak's.
- `replay`: replays each single-turn case's captured first request
  `--trials` times on Ollama and records the first call, for A/B between
  two builds' captures (`--label`). Follow-ups are measured live only.
- `report`: the per-unit table below, from the stored records.

What is measured, all from typed records (`/sessions/{id}/decisions` and
the transcript), never a reading of the request's or answer's words and
never a verdict on the model:

- **first**: whether the first call that does work (`plan` declares
  deliverables and is skipped) is the first unit of the case's path;
- **reached**: whether every unit on the path was called and its call
  accepted, with no `Fault` and no error result;
- **refused**: calls whose `accept_call` verdict was refused, by
  `Refusal`, and the `Fault` of every failed call (`shape` is the call's
  shape, `target` what it names, `tool` the tool or what it reaches,
  `refused` a rule, mode or person);
- **detours**: calls to tools on no path unit (`plan` excepted);
- **used**: the share of an owner call's new tokens (not in the request
  or the system prompt) that the answer and later card arguments carry;
- **plan**: `correct` decisions with cause `plan` (plan-check misses);
- **end**: the turn's `end_turn`.

## Corpus

54 units on a fresh seeded home: 41 tools (20 card tools and the two
abilities `date-calculator` and `table-profile` among them), 11
instruction skills and 1 card pack (`answer-layouts.at-a-glance`). `flow`
is in the capability set but is not bound to an everyday turn, so it has
no cases. A fresh home has no MCP server, so no `mcp:` unit was produced;
that path is in the code and has not been exercised.

`cases.json` holds 2 single, 1 follow-up and 27 pair cases per the
generator prompt, 189 in all. No hosted provider key was on this machine,
so the first set was written by the authoring session (Claude Opus 5.5)
answering exactly the prompts `corpus --emit-prompts` wrote, and every
case is marked `"reviewed": false` with that generator named. They are
fixtures to review, then to regenerate through `--generate` once a hosted
route exists.

## Routes

- Ollama `qwen3.5:0.8b-mlx`, the installed route: run, partially.
- Hosted route: **not run.** The installed home has no hosted provider
  key (`/providers` lists only Ollama), and the route is Ollama on
  purpose. The harness passes a provider key only from its own
  environment. Until a second route runs, no result here can be read as
  "fails on every route" (a contract finding) versus "differs between
  routes" (a declared wire capability such as `strict_tools`): every
  finding below is single-route.

## First results (Ollama, 8 of 135 cases, `--singles 1`)

| unit | offered at start | first | reached | refused | faults | detours |
|---|---|---|---|---|---|---|
| `pack:answer-layouts.at-a-glance` | loaded | 0/2 | 0/2 | 11 | refused, shape, target, tool | 18 |
| `skill:code-review` | through `skill` | 0/2 | 0/2 | 16 | refused, shape, target | 49 |
| `skill:data-and-spreadsheets` | through `skill` | 0/2 | 0/2 | 8 | refused, shape, target | 27 |
| `skill:debugging` | through `skill` | 0/2 | 0/2 | 11 | refused, shape, target, tool | 111 |

Ends: answered 4, aborted 3, step limit 1. No plan-check miss was
recorded. Raw records were left in `/tmp/vak-coverage/runs/base-*`.

What the refused calls were, read from the records:

1. **File calls name paths outside the workspace.** Most `policy`
   refusals are `read`, `doc_read` or `glob` with an invented absolute
   path (`/workspace/...`, `/private/tmp/vak-coverage/...`, `/sales.csv`)
   while the file sat in the workspace. The system prompt states the real
   workspace and that a relative path names a file in it, but `read`'s
   `path` is described as "relative to cwd or absolute", inviting the
   absolute form the check then refuses. To investigate: one
   description of `path` across the file tools, stating the relative
   form.
2. **No instruction skill was loaded.** In 6 of 6 skill turns `skill` was
   never called; the turns went straight to file tools. To investigate
   with the full run: whether the Skills list's wording or its place in
   the prompt is what is missed.
3. **Card arguments in another card's shape.** `emit_table_card` was sent
   `columns` (the table card takes `rows` strings), one-cell rows, or a
   header with no row; `emit_details_card` was sent `points`; a
   card named `emit_card` and a tool named `discover` were called. Each
   was refused with what to fix. The card pack's own kind
   (`at_a_glance` on `emit_stats_card`) was never asked for.
4. **A web page was refused by provenance.** After a `bash` call (private)
   and a `webfetch` result (someone else's words), a further `webfetch`
   was put to the person and refused, as invariant 43 says it must be on
   an unattended surface. The system working, not a finding.

## Open

- Finish the Ollama run (`run --label base --singles 1`, then `replay`
  and `report`), about 4.5 hours on this machine.
- Run a hosted route once the maintainer sets a key, then read each
  finding as contract (both routes) or wire capability (one route).
- Review `cases.json` and regenerate it through a hosted route.
- Exercise the `mcp:` unit path on a home with an MCP server.
