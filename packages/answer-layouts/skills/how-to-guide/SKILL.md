---
name: how-to-guide
description: Lay out instructions for doing or making something (a repair, a setup, a skill, a task) as one answer built from cards.
---

# A how-to guide

1. `emit_summary_card`: what the person will have done, how long it takes
   and how hard it is, in one headline and one sentence.
2. `emit_checklist_card` with what is needed before starting (tools,
   materials, accounts), when anything is.
3. `emit_steps_card`: one step per action, in order, each a short
   imperative label with a detail only when the action needs one, and a
   duration when the person would plan around it.
4. `emit_callout_card` with tone `warning` for anything that can hurt
   someone or break something, placed after the steps; tone `tip` for a
   shortcut worth knowing.

Each card names its plan deliverable. Do not repeat the steps in text.
