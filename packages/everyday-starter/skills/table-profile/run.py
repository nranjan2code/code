import csv
import datetime
import io
import json
import os
import sys


def refuse(why):
    print(why, file=sys.stderr)
    sys.exit(2)


def kind(value):
    for parse, name in ((int, "integer"), (float, "number")):
        try:
            parse(value.replace(",", "") if name == "number" else value)
            return name
        except ValueError:
            pass
    try:
        datetime.date.fromisoformat(value[:10])
        return "date"
    except ValueError:
        return "text"


def number(value):
    return float(value.replace(",", ""))


args = json.load(sys.stdin)
if not isinstance(args, dict):
    refuse("the arguments must be an object")
path, text = args.get("path"), args.get("text")
if (path is None) == (text is None):
    refuse("give either path (a CSV or TSV file in the workspace) or text (the table itself)")
if path is not None:
    workspace = os.path.realpath(os.getcwd())
    full = os.path.realpath(os.path.join(workspace, path))
    if os.path.commonpath([workspace, full]) != workspace:
        refuse(f"{path} is outside the workspace")
    if not os.path.isfile(full):
        refuse(f"{path} is not a file in the workspace")
    with open(full, newline="", encoding="utf-8-sig") as handle:
        text = handle.read()
delimiter = args.get("delimiter")
if delimiter is None:
    first = text.splitlines()[0] if text.strip() else ""
    delimiter = max(",\t;|", key=first.count)
if not isinstance(delimiter, str) or len(delimiter) != 1:
    refuse("delimiter is one character, such as , or a tab")
rows = list(csv.reader(io.StringIO(text), delimiter=delimiter))
rows = [row for row in rows if any(cell.strip() for cell in row)]
if not rows:
    refuse("the table is empty")
header, body = rows[0], rows[1:]
columns = []
for index, name in enumerate(header):
    values = [row[index].strip() if index < len(row) else "" for row in body]
    present = [value for value in values if value]
    kinds = {kind(value) for value in present}
    if not kinds:
        column_type = "empty"
    elif kinds <= {"integer"}:
        column_type = "integer"
    elif kinds <= {"integer", "number"}:
        column_type = "number"
    elif kinds == {"date"}:
        column_type = "date"
    else:
        column_type = "text"
    column = {
        "name": name,
        "type": column_type,
        "missing": len(values) - len(present),
        "distinct": len(set(present)),
    }
    if column_type in ("integer", "number"):
        numbers = [int(value) if column_type == "integer" else number(value) for value in present]
        column.update(min=min(numbers), max=max(numbers), mean=round(sum(numbers) / len(numbers), 6), total=sum(numbers))
    elif column_type == "date":
        column.update(min=min(present), max=max(present))
    elif column_type == "text":
        if len(kinds) > 1:
            column["mixed_types"] = sorted(kinds)
    columns.append(column)
ragged = sum(1 for row in body if len(row) != len(header))
seen, duplicates = set(), 0
for row in body:
    key = tuple(row)
    duplicates += key in seen
    seen.add(key)
print(json.dumps({
    "rows": len(body),
    "delimiter": "tab" if delimiter == "\t" else delimiter,
    "columns": columns,
    "duplicate_rows": duplicates,
    "rows_with_a_different_number_of_cells": ragged,
}))
