---
name: adding-a-check
description: Add a measurement or check to Vak's blueprint harness, or retire one. Use when a finding needs evidence the harness does not yet produce, when a recurring kind of drift needs a gate check, or when a finding leaves and its check has nothing left to feed.
---

# Adding a check to the blueprint

Status: reference

The harness measures only what something on the page uses. A check is added
for a finding or a recurring drift, and removed with it. Code that feeds
nothing is dead code here as anywhere.

## Pick the kind

| You need | Put it in | Runs |
|---|---|---|
| A number about the tree, its graph or its history | `harness/measure.py` (into the measures dict `m`) | every build |
| Evidence a finding rests on | `harness/evidence.py`, one function returning plain data | every build |
| Something slow, or that needs the network | `evidence.deep`, cached in `measures.json` | `run.py deep` |
| A summary of a record file | `harness/record.py` | every build |
| A rule documents must follow, failing the gate | `scripts/check_records_drift.py` | the gate, and the blueprint's health |

## Steps

1. Write the function so it depends only on the tree and its history, never
   on the wall clock or a data home. Keep it deterministic: generating twice
   must give the same bytes.
2. State any heuristic in its docstring and on the page (for example, fix
   commits found by words in commit messages compare files, they do not
   count bugs).
3. Expose its values through `model.py`: a key for `{{e:…}}`, a list for
   `{{list:…}}`, or a section of the model the renderers draw. Draw it in
   all three views in `render.py`, so README.md, the HTML and the JSON never
   disagree.
4. Cite it from `content.toml` as a placeholder, never as a literal number.
5. Measure the whole population, not just the case you care about, and
   compare against the tree's own baseline (median, percentile, rank), never
   a threshold picked by taste.
6. Run `python3 blueprint/harness/run.py`, check the three views, and
   commit the check, then the regenerated views on their own.

## Retiring one

When the last finding or rule that used a check leaves, remove the
function, its model keys and its rendering in the same change, and name
what was removed in the commit message.
