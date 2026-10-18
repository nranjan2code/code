# docs/architecture — rules for agents and people

This folder holds the system blueprint and older dated diagrams. The
repository's [AGENTS.md](../../AGENTS.md) still applies in full; this file
adds what is specific here.

## The blueprint

`blueprint.html` is the system blueprint: the layer stack, process
topology, one turn end to end, the data map, a bill of materials for every
crate, where the size and the work are, a critic's findings and a target
shape.

It is **generated**. Never edit `blueprint.html` or `measures.json` by hand.

To review the architecture, check a finding or add one, follow
[review-method.md](review-method.md): every finding is a claim tested by a
measurement and given a verdict.

| Part | Where it comes from |
|---|---|
| Every number (lines, files, tests, commits, dependencies, routes, tools, worker tasks, card forms, durable paths, longest files and functions) | measured by `scripts/blueprint.py` from the tree, `cargo metadata` and `git log` |
| The evidence behind each finding (fix density, unforwarded declarations) | the checks in `scripts/blueprint/evidence.py`, run on every generation |
| Cognitive complexity | `python3 scripts/blueprint.py --deep` (clippy), cached in `measures.json` |
| The layer stack, the dependency matrix, the upward-dependency list, the bill of materials, the size and work tables | drawn by `scripts/blueprint.py` from those measurements |
| Each crate's layer and one-line role | the `CRATES` table in `scripts/blueprint.py` |
| The process topology, the turn sequence, the data tree, the findings, what to keep and the target shape | written by hand in `scripts/blueprint/template.html`, with live numbers through `{{…}}` placeholders |

### When it is regenerated

- `scripts/build.sh` regenerates it on every build.
- `scripts/release.sh` regenerates it and refuses to release while the
  committed page differs from the tree. Commit the regenerated page before
  releasing.
- By hand: `python3 scripts/blueprint.py`, or with `--deep` to refresh
  complexity (about a minute). Refresh `--deep` before a release; `scripts/release.sh` warns
  when `measures.json` predates the last release tag.

Commit the regenerated page in a commit of its own, right after the
change that moved its numbers, so the page's history in git is the
architecture's history. Commits that touch only `blueprint.html` and
`measures.json` are left out of every count, and the page names the last commit that touched
anything else. So after a commit of only those two files, generating again
changes nothing. A page committed together with other files is always one commit
behind, and `scripts/release.sh` would refuse it.

### When it prints "needs review"

The generator checks the names the hand-drawn sheets depend on (the
`ANCHORS` list) and every `{{…}}` placeholder in the template. A name it no
longer finds prints `blueprint: needs review: …` and puts a red banner at
the top of the page. It never fails the build. Resolve each one in the same
change that caused it:

| Message | What to do |
|---|---|
| `fn … (template)`, `file … (template)` or `crate … (template)` | The template quotes something that was renamed or removed. Update the placeholder and the sentence around it. |
| `… (crates/…)` from `ANCHORS` | A function or type that the process or turn drawing names has moved or gone. Retrace that step in the code, fix the drawing and the step list, then fix `ANCHORS`. |
| `crate … has no layer` | A new crate. Add it to `CRATES` with its layer and a one-line role. |
| `crate … is gone` | Remove it from `CRATES`, and from any prose that names it. |
| `evidence … (template)`, `measure … (template)` or `completion model …` | A check no longer produces that value, or a type it reads moved. Fix the check in `evidence.py` or the placeholder. |
| `complexity of … (template; run --deep)` | Run `python3 scripts/blueprint.py --deep`, or the function was renamed. |

### Keeping the prose true

The numbers update themselves; the judgements don't. When a change
resolves a finding, makes one wrong, or adds one, edit the finding in
`scripts/blueprint/template.html` and update the "Prose reviewed" line in
its title block to the version and date. Do that in the same commit as
the change. Rules for the prose:

- State only what the tree or a dated audit shows, and cite where.
- A finding that a recorded decision settles stays settled. Owning the
  storage layer is one such decision (no third-party database beyond the
  embedded SQLite; [decisions.md](../decisions.md), 2026-10-18). Describe
  its cost, not its replacement.
- Write the numbers as placeholders (`{{fn:name}}`, `{{file:path}}`,
  `{{src:crate}}`, `{{tests:crate}}`, `{{deps:crate}}`, `{{m:key}}`,
  `{{e:key}}` for an evidence check, `{{cc:fn}}` and `{{ccrank:fn}}` for
  complexity, `{{hist:fn}}` for a function's commits), never as literals, so
  they cannot go stale. Dated history (a commit id, what one run did) may be
  quoted as written.
- List only findings with a cost today, each with a verdict chip and the
  parts review-method.md lists. A finding that is fixed, or that the
  evidence does not support, leaves the page; git keeps it.
- Remove a finding once it no longer holds. Git keeps the old text.

### Measuring

- Source lines count `src/**/*.rs`, including inline unit tests. Rust test
  lines count `tests/**`.
- Function lengths run from the header to the closing brace at the same
  indentation, which assumes rustfmt layout. Inline
  `#[cfg(test)] mod … { }` blocks are left out.
- Dependencies are normal (non-dev) dependencies from
  `cargo metadata --no-deps --offline`.
- The generator writes `blueprint.html` (and, with `--deep`,
  `measures.json`), plus a cache of per-commit file lists in the git
  directory. It reads no data home. Nothing reaches the network. The
  page measures the working tree, so uncommitted work in the checkout is
  counted until it is committed or discarded.

## The other pages

The other HTML pages here are dated snapshots, kept as records
([README.md](README.md)). Never update their numbers. When the blueprint
covers what one of them shows, mark that page as superseded in the README
rather than deleting it.
