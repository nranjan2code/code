---
name: planning-and-organizing
description: Turn a goal into a practical plan with dated steps, what to prepare, open decisions, and anything to schedule or remember.
serves: orchestration
---

# Planning and organizing

1. Get the outcome, the deadline or dates, and the constraints from what
   the person said; ask only for what changes the plan.
2. Work out every date, duration and weekday with the `date-calculator`
   tool, never in your head.
3. Size the plan to the work: a few steps for an afternoon, phases for a
   project. Every step is a short action someone can start; keep owners,
   deadlines and open decisions explicit.
4. Lay it out from cards, each naming the `plan` deliverable it delivers:
   - `emit_summary_card`: the plan in a headline and a sentence, with
     the start, the end and the one thing not to miss as points;
   - `emit_steps_card`: the steps in order, each with its `time` or
     `duration_minutes` when known;
   - `emit_list_card` with `checkable: true`: what to bring, buy or
     prepare, when there is any;
   - `emit_calendar_card` when the plan is events on dates;
   - `emit_callout_card` (`tone: "warning"` or `"tip"`) for the one risk
     or decision a step cannot carry.
5. When the person wants to be reminded or wants something to happen
   later, make it with `automations` (in their time zone) rather than
   telling them to set a reminder. Keep a decision worth remembering
   after this conversation with `remember`, only when they ask or it is
   plainly durable.
