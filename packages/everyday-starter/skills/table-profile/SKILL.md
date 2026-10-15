---
name: table-profile
description: Profile a CSV or TSV table before using it: rows, each column's type, missing and distinct values, ranges, totals, duplicate and ragged rows.
serves: documents, code-exec
---

# Table profile

Call the `table-profile` tool on a table before you analyse, chart or
change it, so what you say about the data comes from the data.

- `path` is a CSV or TSV file in the workspace (relative to it); `text`
  is a table someone pasted, header row first. Give one.
- Each column comes back typed `integer`, `number`, `date`, `text` or
  `empty`, with `missing` and `distinct` counts, and `min`, `max`,
  `mean` and `total` for numbers. A text column with `mixed_types`
  holds values of more than one kind (a number column with a stray
  word): name it before you compute with it.
- `duplicate_rows` and `rows_with_a_different_number_of_cells` are
  problems to report, never to fix silently.
- For an Excel workbook or a PDF table, read it with `doc_read` first.
