type Decimal = {
  sign: -1 | 0 | 1;
  order: bigint;
  digits: string;
};

const DECIMAL = /^([+-]?)(?:(\d+)(?:\.(\d*))?|\.(\d+))(?:[eE]([+-]?\d+))?$/;

function decimal(value: unknown): Decimal | null {
  if (typeof value === "number") {
    if (!Number.isFinite(value)) return null;
    value = String(value);
  }
  if (typeof value !== "string") return null;

  const match = DECIMAL.exec(value.trim());
  if (!match) return null;
  const negative = match[1] === "-";
  const integer = match[2] ?? "";
  const fraction = match[3] ?? match[4] ?? "";
  const allDigits = `${integer}${fraction}`;
  const significant = allDigits.replace(/^0+/, "");
  if (significant.length === 0) return { sign: 0, order: 0n, digits: "0" };

  const exponent = BigInt(match[5] ?? "0");
  return {
    sign: negative ? -1 : 1,
    order: exponent - BigInt(fraction.length) + BigInt(significant.length - 1),
    digits: significant,
  };
}

function compareMagnitude(left: Decimal, right: Decimal): number {
  if (left.order !== right.order) return left.order < right.order ? -1 : 1;
  const length = Math.max(left.digits.length, right.digits.length);
  for (let index = 0; index < length; index += 1) {
    const a = left.digits.charCodeAt(index) || 48;
    const b = right.digits.charCodeAt(index) || 48;
    if (a !== b) return a < b ? -1 : 1;
  }
  return 0;
}

function compareDecimals(left: Decimal, right: Decimal): number {
  if (left.sign !== right.sign) return left.sign < right.sign ? -1 : 1;
  if (left.sign === 0) return 0;
  const magnitude = compareMagnitude(left, right);
  return left.sign < 0 ? -magnitude : magnitude;
}

/** Compare decimal strings exactly; non-decimals sort after decimals. */
export function compareDecimalValues(leftValue: unknown, rightValue: unknown): number {
  const left = decimal(leftValue);
  const right = decimal(rightValue);
  if (left === null) return right === null ? 0 : 1;
  if (right === null) return -1;
  return compareDecimals(left, right);
}

/** Sort rows stably, keeping missing or invalid numeric values last either way. */
export function sortTableRows(
  rows: readonly unknown[][],
  column: number,
  numeric: boolean,
  ascending: boolean,
): unknown[][] {
  return rows
    .map((row, index) => ({ row, index, parsed: numeric ? decimal(row[column]) : null }))
    .sort((left, right) => {
      let compared: number;
      if (numeric) {
        const a = left.parsed;
        const b = right.parsed;
        if (a === null || b === null) {
          if (a === null && b === null) return left.index - right.index;
          return a === null ? 1 : -1;
        }
        compared = compareDecimals(a, b);
      } else {
        const a = String(left.row[column] ?? "");
        const b = String(right.row[column] ?? "");
        compared = a < b ? -1 : a > b ? 1 : 0;
      }
      return compared === 0 ? left.index - right.index : ascending ? compared : -compared;
    })
    .map(({ row }) => row);
}
