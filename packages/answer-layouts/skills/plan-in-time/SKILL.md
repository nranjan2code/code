---
name: plan-in-time
description: Lay out a plan that happens over time (a day, a trip, a routine, an event, a project) as one answer built from cards.
---

# A plan in time

Compose the answer in this order, each part its own card, each naming the
plan deliverable it delivers:

1. `emit_summary_card`: the plan in one headline and one or two sentences,
   with up to five key points (start, end, the one thing not to miss).
2. When the plan has figures worth seeing at a glance (total time, cost,
   distance, number of stops), `emit_stats_card` with
   `semantic_type: "at_a_glance"`, one reading per figure.
3. `emit_steps_card` with the kind that fits (`itinerary` for a trip,
   `schedule` or `agenda` for a day, `habit_plan` for a routine,
   `project_plan` for work): every step a short label, its time as the
   person gave it, and a detail only when it helps.
4. `emit_checklist_card` for what to bring, buy or prepare, when there is
   any.
5. `emit_callout_card` with tone `tip` or `warning` for the one thing a
   card cannot carry, only when there is one.

Write a sentence of text only where it joins two parts; never restate a
card. Use only facts the person gave or a source you retrieved.
