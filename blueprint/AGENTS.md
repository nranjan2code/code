# blueprint/: rules for agents and people

This folder is the one place to see what the system is, what is happening,
and what needs attention. The repository's [AGENTS.md](../AGENTS.md) still
applies in full; this file adds what is specific here.

## What is here

| Path | What it is | Written by |
|---|---|---|
| [README.md](README.md) | The blueprint as Markdown: GitHub renders it, any reader or agent can read it | generated |
| [blueprint.html](blueprint.html) | The same model with diagrams and filterable tables; open it in a browser | generated |
| [blueprint.json](blueprint.json) | The same model as data, for agents and scripts | generated |
| [measures.json](measures.json) | The slow measures (complexity, advisories), cached by `run.py deep` | generated |
| [content.toml](content.toml) | The hand-written judgement: the summary, the findings, what to keep, the target shape | by hand |
| [methods/](methods/) | How to work here, one method per file, with name and description front matter | by hand |
| `harness/` | The code: `run.py` (the entry point), `measure.py`, `subsystems.py`, `evidence.py`, `record.py`, `model.py`, `render.py`, `template.html` | by hand |

Never edit a generated file. Change its source and run the harness.

## The one entry point

```bash
python3 blueprint/harness/run.py
```

| Verb | Does | Time |
|---|---|---|
| `build` (the default) | measures the tree, reads the record and `content.toml`, runs the record checks, writes the three views | seconds |
| `deep` | also runs clippy's cognitive complexity over every library and `cargo audit`, into `measures.json`, then builds | about a minute; `cargo audit` needs the network |
| `check` | prints the record's health and exits 1 if a check fails | seconds |
| `names` | lists the code names living documents cite that no longer exist, for triage | seconds |

`scripts/build.sh` runs `build` on every build and never fails because of
it. `scripts/release.sh` runs it and refuses to release while a generated
view differs from the tree, and it warns when `measures.json` predates the
last release tag; run `deep` before a release.

## Where each part comes from

| On the page | Source |
|---|---|
| Every number: lines, files, tests, commits, dependencies, routes, tools, forms, worker tasks, durable paths, longest files and functions | `measure.py`, from the tree, `cargo metadata` and `git log` |
| The evidence findings rest on (fix density, unforwarded Tool declarations) | `evidence.py`, every build |
| Complexity and advisories | `evidence.deep`, cached in `measures.json` |
| Health: the record, doc-path and version checks, and the stale-name count | `evidence.health`, running the gate's own scripts |
| Open work, decisions, specs, unreleased changes, recent commits | `record.py`, reading `docs/plans/open-items.md`, `docs/decisions.md`, every design doc's `Status:` line, `CHANGELOG.md` and git |
| Each crate's layer and role | `CRATES` in `measure.py` |
| The subsystems: what each is for, its modules, routes, `vak` subcommands and design docs | `SUBSYSTEMS` and `GROUPS` in `harness/subsystems.py` |
| The summary, findings, what to keep, the target shape | `content.toml` |
| The process topology, the turn sequence and the data tree | drawn by hand in `harness/template.html` |

The record is read, never copied: a decision is written in
`docs/decisions.md`, open work in the tracker, a design's state on its
`Status:` line. The blueprint summarises each and links back. If you want
to change what the blueprint says about one of them, change the source.

## Committing

Commit the regenerated views in a commit of their own, right after the
change that moved their numbers. Commits that touch only the generated
files are left out of every count, and the page names the last commit that
touched anything else, so after such a commit, generating again changes
nothing. Views committed together with other files are always one commit
behind, and `scripts/release.sh` would refuse them.

The views measure the working tree. In a checkout where other work is not
yet committed, generate them in a clean checkout of `HEAD` (a temporary
`git worktree`) and copy them back, so the committed views describe the
committed code.

## Needs review

The harness checks every name the hand-drawn sheets depend on (`ANCHORS` in
`measure.py`) and every placeholder in `content.toml` and the template. A
name it no longer finds is printed as `blueprint: needs review: …` and
bannered at the top of every view. It never fails the build. Resolve it in
the same change that caused it:

| Message | What to do |
|---|---|
| `fn …`, `file …`, `crate …` | Something the content quotes was renamed or removed. Fix the placeholder and its sentence. |
| `… (crates/…)` from `ANCHORS` | A function or type a drawing names moved. Retrace that step in the code, fix the drawing and the step list, then `ANCHORS`. |
| `crate … has no layer` | A new crate: add it to `CRATES` with its layer and a one-line role. |
| `module … has no subsystem` | A new source file, script, package or part: add it to the subsystem it serves in `harness/subsystems.py`, or a new subsystem (and its group) when it serves none. |
| `module …`, `route …` `is claimed equally by …` | Two subsystems claim it at the same specificity: name the file exactly in the one that owns it. |
| `route … has no subsystem` | A new route: add its prefix to the subsystem that serves it. |
| `vak … has no subsystem` | A new subcommand: add it to the subsystem's verbs. |
| `subsystem …: pattern, route prefix, vak or doc … matches nothing` | Something a subsystem names was renamed or removed: fix the entry. |
| `crate … is gone` | Remove it from `CRATES` and any content that names it. |
| `evidence …`, `measure …`, `list …` | A check no longer produces that value. Fix the check or the placeholder. |
| `complexity of … (run deep)` | Run `run.py deep`, or the function was renamed. |

## Subsystems

A crate says where code lives; a subsystem says what it is for. Doctor, for
example, spans `crates/vak/src/doctor.rs`, `vak-core`'s health report, the
`/health` and `/doctor` routes, `vak doctor` and the admin Diagnostics
screen, and no crate is named after it. `harness/subsystems.py` maps every
unit to exactly one subsystem: each source file under `crates/*/src`
(Rust and TypeScript, tests left out), the public site, each plugin
package, each entry of `scripts/`, and `docker/`; every route the server
registers, by its longest claiming prefix; and every `vak` subcommand. The
most specific module pattern wins (an exact path over a glob, then the
longer literal prefix), so a feature claims its files out of a crate
another subsystem holds. Anything left out, claimed twice or naming
nothing is a `needs review` line, so a new part of the system cannot go
unmapped.

## Findings

Findings are the judgement part, and they follow
[methods/architecture-review.md](methods/architecture-review.md):

- List only findings with a cost today. A finding that is fixed, or that
  the evidence does not support, leaves `content.toml`; git keeps it.
- Each has an id, a tag, a verdict, a title that states what is true, the
  evidence, why it matters, the fix and its sources.
- A finding a recorded decision settles stays settled: describe its cost,
  never its reversal (owning the storage layer, docs/decisions.md,
  2026-10-18).
- When a change makes a finding wrong, fixes it, or adds one, edit
  `content.toml` and set `prose_reviewed` to the version and date, in the
  same commit.

## Placeholders

Numbers in `content.toml` and the template are placeholders, never
literals, so they cannot go stale. Dated history (a commit id, what one run
did) may be quoted as written.

| Placeholder | Value |
|---|---|
| `{{m:key}}`, `{{k:key}}` | a measure (`version`, `revision`, `routes`, `subsystems`, `card_tools`, `cc_median`, …); `k` abbreviates thousands |
| `{{e:key}}` | an evidence value (`hot.median`, `tool.wrappers`, …) |
| `{{list:key}}` | an evidence list, as Markdown items (`wrappers`) |
| `{{fn:name}}`, `{{file:path}}` | a function's or file's length in lines |
| `{{src:crate}}`, `{{tests:crate}}`, `{{deps:crate}}`, `{{usedby:crate}}`, `{{commits:crate}}` | a crate's figures |
| `{{cc:fn}}`, `{{ccrank:fn}}`, `{{ccx:fn}}` | a function's complexity, its rank, and its multiple of the median |
| `{{hist:fn}}`, `{{histpct:fn}}` | commits that changed a function's current lines, and their share |
| `{{pct:a/b}}` | one measure as a percentage of another |

## Measuring

- Source lines count `src/**/*.rs`, inline unit tests included; Rust test
  lines count `tests/**`.
- Function lengths run from the header to the closing brace at the same
  indentation (rustfmt layout); inline `#[cfg(test)] mod … { }` blocks are
  left out.
- Routes are those registered outside inline test modules. Built-in tools
  are the `Tool` impls with a literal name in `vak-tools`, `vak-core`,
  `vak-agent` and `vak-mcp`; card tools are counted from their one table
  in `presentation_tools.rs`.
- Dependencies are normal (non-dev) ones from `cargo metadata --no-deps
  --offline`.
- Recent activity is the fortnight before the last measured commit, never
  the wall clock.
- The harness writes only the generated files here and a cache of
  per-commit file lists in the git directory. It reads no data home. Only
  `deep` reaches the network.
