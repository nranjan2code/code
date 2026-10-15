---
name: date-calculator
description: Count the days, weeks or working days between two dates, or move a date by days, working days or months, with each date's weekday.
serves: orchestration
---

# Date calculator

Call the `date-calculator` tool instead of working out dates in your head:
counts of days, deadlines, "three working days after Friday", "a month
from 31 January", and the weekday of any date.

- Dates are `YYYY-MM-DD`. Turn the person's words ("next Friday", "15
  Oct") into that form from today's date in the time line first; when a
  date could mean two things, ask.
- `end` gives `days_between`, `weeks_between` and
  `business_days_between` (Monday to Friday; public holidays are not
  known to it, so say so when they matter).
- `add_months` then `add_days` move from `start`; `business_days: true`
  counts the days as working days.
- Quote its dates and weekdays exactly. A refusal names what to fix in
  the arguments.
