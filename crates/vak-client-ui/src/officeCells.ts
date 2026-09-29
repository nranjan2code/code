// A cell's shown value as the Office reader writes it (docs/design/72, P4):
// a plain value, or a formula with the value cached in the file, which may
// be stale because Vakyartha never calculates (`fullCalcOnLoad` recalculates when
// the file is opened in Excel).

export type Cell = {
  shown: string;
  formula: string | null;
  stale: boolean;
  notCalculated: boolean;
};

const FORMULA = /^(=[\s\S]*?|\(shared formula\)) \[(?:cached: ([\s\S]*?)(, stale until recalculated)?|(not calculated yet))\]$/;

export function parseCell(value: string): Cell {
  const match = FORMULA.exec(value);
  if (!match) return { shown: value, formula: null, stale: false, notCalculated: false };
  return {
    shown: match[2] ?? "",
    formula: match[1],
    stale: Boolean(match[3]),
    notCalculated: Boolean(match[4]),
  };
}

/** Parse familiar values typed into the shared workbook editor. A leading
 * apostrophe keeps an entry literal; formulas remain strings for the worker's
 * formula convention. */
export function parseCellInput(value: string): string | number | boolean {
  const trimmed = value.trim();
  if (value.startsWith("'") || value.startsWith("=")) return value;
  if (/^(true|false)$/i.test(trimmed)) return trimmed.toLowerCase() === "true";
  if (trimmed !== "") {
    const numeric = Number(trimmed);
    if (Number.isFinite(numeric)) return numeric;
  }
  return value;
}

/** "B12" → { column: 2, row: 12 }; null for anything else. */
export function cellAddress(address: string): { column: number; row: number } | null {
  const match = /^([A-Za-z]{1,3})(\d+)$/.exec(address);
  if (!match) return null;
  let column = 0;
  for (const letter of match[1].toUpperCase()) column = column * 26 + (letter.charCodeAt(0) - 64);
  return { column, row: Number(match[2]) };
}

export function columnName(column: number): string {
  let name = "";
  for (let rest = column; rest > 0; rest = Math.floor((rest - 1) / 26)) {
    name = String.fromCharCode(65 + ((rest - 1) % 26)) + name;
  }
  return name;
}

/** "B2" or "B2:D9" → its corners, first above-left of last; null otherwise. */
export function cellRange(cells: string): { first: { column: number; row: number }; last: { column: number; row: number } } | null {
  const [from, to = from] = cells.trim().split(":");
  const a = cellAddress(from);
  const b = cellAddress(to);
  if (!a || !b) return null;
  return {
    first: { column: Math.min(a.column, b.column), row: Math.min(a.row, b.row) },
    last: { column: Math.max(a.column, b.column), row: Math.max(a.row, b.row) },
  };
}

/** Move one cell while treating a native merged range as one selectable cell. */
export function moveCell(
  address: string,
  key: "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown",
  mergedRanges: ReturnType<typeof cellRange>[],
): string {
  const current = cellAddress(address);
  if (!current) return address;
  const columnDelta = key === "ArrowLeft" ? -1 : key === "ArrowRight" ? 1 : 0;
  const rowDelta = key === "ArrowUp" ? -1 : key === "ArrowDown" ? 1 : 0;
  let column = Math.max(1, current.column + columnDelta);
  let row = Math.max(1, current.row + rowDelta);
  let merge = mergedRanges.find((range) => range
    && column >= range.first.column && column <= range.last.column
    && row >= range.first.row && row <= range.last.row);
  let target = merge
    ? `${columnName(merge.first.column)}${merge.first.row}`
    : `${columnName(column)}${row}`;

  // A move from the active merge's top-left back into its own covered cells
  // must advance past the whole merge or keyboard focus cannot escape it.
  if (target === `${columnName(current.column)}${current.row}` && merge) {
    column = columnDelta < 0 ? Math.max(1, merge.first.column - 1)
      : columnDelta > 0 ? merge.last.column + 1
      : column;
    row = rowDelta < 0 ? Math.max(1, merge.first.row - 1)
      : rowDelta > 0 ? merge.last.row + 1
      : row;
    merge = mergedRanges.find((range) => range
      && column >= range.first.column && column <= range.last.column
      && row >= range.first.row && row <= range.last.row);
    target = merge
      ? `${columnName(merge.first.column)}${merge.first.row}`
      : `${columnName(column)}${row}`;
  }
  return target;
}
