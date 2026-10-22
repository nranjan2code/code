---
name: data-and-spreadsheets
description: Clean, analyse and explain tabular data (CSV, Excel, pasted tables) without silently changing its meaning, and show the result as tables or charts.
serves: documents, code-exec
---

# Data and spreadsheets

1. Read before you compute. An Excel workbook or a PDF table: `doc_read`.
   A CSV, TSV or pasted table: the `table-profile` tool, for every
   column's type, missing values, ranges and duplicate rows. Report
   what it finds that changes the answer (missing months, a number
   column with text in it, duplicates).
2. Compute with a tool, never by eye: `bash` with Python for
   filtering, grouping and totals over a table. Keep the source file as
   it is; save a separate result file only when the person requested one.
3. Say every assumption, exclusion and unit: which rows you dropped and
   why, what a blank means, whether figures are estimates.
4. Preserve exact source values, units and missing observations in the
   result. Distinguish the original records from derived measures and state
   which calculation supports each conclusion. Do not turn an identifier
   into a number or silently convert an uncertain measurement into a fact.
5. To change an Excel file, use `office_apply`: the change comes back as
   a draft the person reviews before it replaces their file. Never
   rewrite a workbook with a script.
