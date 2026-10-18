---
name: architecture-review
description: Review Vak's architecture with evidence rather than opinion. Turn each concern into a claim that can be tested, measure it from the tree and its history, give a verdict (confirmed, partly confirmed, not supported), and record it in the generated system blueprint. Use this when asked whether a design or structural concern is real, when auditing architecture, or when a finding in docs/architecture/blueprint.html needs checking or updating.
---

# Architecture review by evidence

Status: reference

This is the method behind the findings in the
[system blueprint](blueprint.html). Developers and agents of any kind can
follow it. It uses only the repository, git, cargo and Python.
[AGENTS.md](AGENTS.md) in this folder holds the rules for editing the
blueprint itself.

## The rule

A finding is a claim with evidence and a verdict, never an impression.
"This function is long" is an observation. "This function has the highest
cognitive complexity of 4,000 functions, a tenth of all commits change it,
and two recorded bugs came from checks interfering inside it" is a finding.
When the evidence contradicts the claim, the claim changes. Say so plainly,
and keep the correction visible.

## Steps

1. **State the claim so it can fail.** Write the concern as a question with
   an answer you could measure: "Do the largest files need more fixing per
   line?", not "the files are too big". Name the harm the claim implies:
   bugs, change cost, security exposure, or a broken rule.
2. **Pick a measure that answers it.** Prefer, in order:
   - a check over the code that finds concrete instances (a wrapper that
     does not forward a method, an advisory in the lockfile, a blocking call
     in an async function);
   - the history, followed exactly (`git log -L` for a function's lines,
     `git log -S` for when a symbol appeared, commit messages that record a
     fix and its cause);
   - a statistic over the whole tree, compared with the tree's own baseline
     (median, 99th percentile, a rank correlation), never with a number
     chosen by taste;
   - a recorded incident in `docs/audits/`, `docs/plans/open-items.md` or
     the CHANGELOG.
3. **Measure the whole population, then the case.** Score every function or
   file, then place the one in question within that distribution. A value
   means nothing without the baseline.
4. **Look for the harm, not just the shape.** A complex function that never
   breaks, or a big file with ordinary fix density, is a weaker finding than
   it looks. Read the fix commits that touched the code and sort them:
   caused by the structure, or a mistake any structure would have had?
5. **Check reachability.** An advisory in a dependency matters only if Vak
   reaches the vulnerable code. A missing forward matters only if the
   default answers differently. Check, and say which you checked by hand.
6. **Give a verdict.** One of:
   - **Confirmed**: the evidence shows the claim and a cost.
   - **Partly confirmed**: something real, narrower than claimed, and say
     what.
   - **Not supported**: the evidence does not show it. Say what is true
     instead.

   Add "no incident yet" or "caused a defect" when the record shows which.
7. **Say what would fix it**, sized to the evidence. A narrow finding gets a
   narrow fix (a test, a lint, a version bump), not a rewrite.
8. **Record it** in the blueprint (below) and commit it, if it has a cost
   today. A finding the evidence does not support, or one that is fixed,
   is not listed: say so in the commit, and git keeps it.

## Tools in the repository

| Tool | What it answers |
|---|---|
| `python3 scripts/blueprint.py` | Regenerates the blueprint and every live number in it (about 4 s once the commit cache in the git directory is warm). |
| `python3 scripts/blueprint.py --deep` | Also runs the slow check: clippy's `cognitive_complexity` at threshold 1 over every workspace library. The result goes to `measures.json`. Takes about a minute. |
| `scripts/blueprint/evidence.py` | One function per check: `hotspots` (file size against fix density, Spearman), `fanout` (how many crates rebuild when one changes), `tool_wrappers` (unforwarded `Tool` declarations), `test_density` (tests per thousand lines, fuzz targets), `completion_models` (separate "is it done" vocabularies and translations between them), `ledger_types` (what the ledger takes from the provider and intent crates), `deep` (cognitive complexity). Checks are added when a finding needs them and removed with the finding. |
| `git log -L <start>,<end>:<file>` | Every commit that changed a function's current lines, followed back through edits. |
| `git log -S'<symbol>'` | When a symbol was added or removed: for example, whether a new trait method reached its wrappers in the same commit. |
| `cargo metadata --no-deps --offline` | The crate graph. |
| `scripts/check-advisories.sh` | Known advisories in `Cargo.lock` (`cargo audit`); part of the gate. |
| `python3 scripts/check_records.py --names` | Code names the docs cite that no longer exist. |

To add a check, write a function in `evidence.py` that returns plain data,
render it in `evidence_blocks` in `scripts/blueprint.py`, and cite it from
the template as `{{e:key}}` or a `%%BLOCK%%` table.

## Recording a finding

Findings live in `scripts/blueprint/template.html`, in the findings sheet.
Each one has the same parts:

- a tag (`Structure`, `Layering` or `Drift`) and a verdict chip
  (`verdict yes`, `verdict part`, `verdict no`);
- a heading that states what is true, not what was suspected;
- **Claim tested**: the question from step 1;
- **Measured**: the evidence, with every number a placeholder (`{{e:…}}`,
  `{{fn:…}}`, `{{cc:…}}`, `{{hist:…}}`, `{{m:…}}`), so it stays current;
- **Verdict**, with what would fix it;
- an evidence line naming the check, file or commit.

Dated history, such as a commit id or what one run did, may be quoted
literally. Measurements may not.

Then:

1. Run `python3 scripts/blueprint.py` (with `--deep` if you used complexity
   or audit numbers). Resolve every `needs review` line.
2. Update the "Prose reviewed" version and date in the template's title
   block.
3. Commit the template and scripts, then commit the regenerated
   `blueprint.html` and `measures.json` on their own (AGENTS.md explains
   why).

## Pitfalls already met

- **Count inline tests.** Rust keeps many tests in `#[cfg(test)]` modules
  inside `src/`. Counting only `tests/` once made the sandbox look untested
  when it sits close to the median.
- **Count all source.** Line totals that include `tests/` overstate the
  code. Report source and tests separately.
- **Match multi-line code.** A route or call split across lines is missed
  by a per-line grep. One count read 271 routes where there are over 400.
- **Leave test fixtures out.** Tools defined inside test modules once
  inflated the tool count.
- **Distinguish async from sync.** `tokio::process` is async;
  `std::process` is not. A first count of blocking process calls mixed them
  up.
- **Heuristics compare, they don't count.** Fix commits found by words in
  their messages are fine for comparing files with each other, not for
  counting bugs. Say so wherever one is used.
- **A decision settles a finding.** When `docs/decisions.md` records a
  choice (owning the storage layer, for example), describe its cost and
  what it requires. Do not argue for reversing it.
- **Never close a finding by naming the model.** The repository rule
  ("Investigating a failure" in the root AGENTS.md) applies to reviews too.
