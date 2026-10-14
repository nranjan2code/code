---
name: decide-between
description: Lay out a choice between two or more options (products, places, approaches, plans) as one answer built from cards.
---

# A choice between options

1. `emit_summary_card`: the recommendation as the headline and the reason
   in one sentence. When the choice depends on something the person has
   not said, say what, and lead with the option that fits the common case.
2. `emit_comparison_card`: every option with a short verdict, its
   strengths and weaknesses, the aspects they differ on (the same aspects,
   in the same order, for every option), and the recommendation.
3. `emit_table_card` only when there are more than four aspects or more
   than four options: one row per option, one column per aspect.
4. `emit_callout_card` with tone `warning` for a risk that would change
   the choice, only when there is one.

Each card names its plan deliverable. Keep every verdict, strength and
weakness to a few words. Use only facts the person gave or a source you
retrieved; when a figure is not known, leave it out rather than guess.
