const MAX_TICKS = 8;

/**
 * Return ascending, finite ticks covering a finite numeric domain.
 * Reversed endpoints are accepted; a non-finite endpoint yields no scale.
 * Equal endpoints are expanded by one unit when possible, or to the adjacent
 * representable value at the finite-number boundary. Difficult domains use
 * their endpoints directly, preserving distinctions that a rounded grid
 * cannot represent safely.
 */
export function niceTicks(min: number, max: number): number[] {
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [];
  let low = Math.min(min, max);
  let high = Math.max(min, max);
  if (low === high) {
    const increment = Math.abs(low) * Number.EPSILON || Number.MIN_VALUE;
    if (high < Number.MAX_VALUE) high = low + (low === 0 ? 1 : increment);
    else low = high - increment;
  }

  const span = high - low;
  const raw = span / 4;
  if (!Number.isFinite(span) || !(raw > 0) || !Number.isFinite(raw)) return [low, high];

  const power = 10 ** Math.floor(Math.log10(raw));
  if (!(power > 0) || !Number.isFinite(power)) return [low, high];
  const exponent = Math.floor(Math.log10(raw));
  const multiple = [1, 2, 2.5, 5, 10].find((candidate) => candidate * power >= raw);
  if (multiple === undefined) return [low, high];
  const coefficient = multiple === 2.5 ? 25 : multiple === 10 ? 1 : multiple;
  const tickExponent = exponent + (multiple === 2.5 ? -1 : multiple === 10 ? 1 : 0);
  const step = coefficient * 10 ** tickExponent;
  if (!(step > 0) || !Number.isFinite(step)) return [low, high];

  const firstIndex = Math.floor(low / step);
  if (!Number.isFinite(firstIndex)) return [low, high];
  if (!Number.isSafeInteger(firstIndex)) return [low, high];
  let index = firstIndex;
  const gridValue = (gridIndex: number): number | undefined => {
    const product = gridIndex * coefficient;
    if (!Number.isSafeInteger(product)) return undefined;
    const value = Number(`${product}e${tickExponent}`);
    return Number.isFinite(value) ? value : undefined;
  };
  let value = gridValue(index);
  if (value === undefined) return [low, high];
  if (value > low) {
    index -= 1;
    if (!Number.isSafeInteger(index)) return [low, high];
    value = gridValue(index);
    if (value === undefined || value > low) return [low, high];
  }

  const ticks: number[] = [];
  for (let count = 0; count < MAX_TICKS; count++) {
    if (!Number.isFinite(value) || (ticks.length > 0 && value <= ticks[ticks.length - 1])) return [low, high];
    ticks.push(value);
    if (value >= high) {
      return ticks.length >= 2 && ticks[0] <= low ? ticks : [low, high];
    }
    index += 1;
    if (!Number.isSafeInteger(index)) return [low, high];
    value = gridValue(index);
    if (value === undefined) return [low, high];
  }
  return [low, high];
}

/** Map a finite value into a [0, 1] interval without overflowing its span. */
export function scalePosition(value: number, min: number, max: number): number {
  if (min === max) return 0.5;
  if (min >= 0 || max <= 0) return (value - min) / (max - min);
  const scale = Math.max(Math.abs(min), Math.abs(max), Math.abs(value));
  return (value / scale - min / scale) / (max / scale - min / scale);
}

/** Use shortest round-trip text: distinct finite values stay distinct and extremes stay compact. */
export function formatChartNumber(value: number): string {
  return value.toString();
}

export type ChartKey = string | number;
export type ChartAxis = "category" | "number" | "time";

/** Numeric/date interpretation is opt-in; category identifiers remain lexical. */
export function chartAxis(keys: ChartKey[], axis: ChartAxis): { keys: ChartKey[]; positions: number[]; valid: boolean } {
  if (axis === "category") return { keys, positions: keys.map((_, index) => (index + 0.5) / Math.max(1, keys.length)), valid: true };
  const numeric = (key: ChartKey): number => {
    if (axis === "time") {
      if (typeof key !== "string" || !/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)$/i.test(key)) return NaN;
      const parts = /^(\d{4})-(\d\d)-(\d\d)T(\d\d):(\d\d):(\d\d)/i.exec(key);
      if (!parts) return NaN;
      const [year, month, day, hour, minute, second] = parts.slice(1).map(Number);
      const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
      const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
      if (month < 1 || month > 12 || day < 1 || day > days[month - 1] || hour > 23 || minute > 59 || second > 59) return NaN;
      return Date.parse(key);
    }
    if (typeof key === "number") return key;
    if (!/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(key)) return NaN;
    return Number(key);
  };
  const points = keys.map((key) => ({ key, value: numeric(key) }));
  if (points.some(({ value }) => !Number.isFinite(value))) return { keys, positions: [], valid: false };
  points.sort((a, b) => a.value < b.value ? -1 : a.value > b.value ? 1 : 0);
  // Two distinct lexical values at the same position cannot identify two observations.
  if (points.some((point, index) => index > 0 && point.value === points[index - 1].value)) return { keys, positions: [], valid: false };
  const min = points[0]?.value ?? 0;
  const max = points.at(-1)?.value ?? 0;
  return { keys: points.map(({ key }) => key), positions: points.map(({ value }) => scalePosition(value, min, max)), valid: true };
}

/** Missing observations split a line/area; never imply a measured connection. */
export function chartSegments(values: (number | null)[]): { i: number; y: number }[][] {
  const segments: { i: number; y: number }[][] = [];
  let segment: { i: number; y: number }[] = [];
  values.forEach((y, i) => {
    if (y === null || !Number.isFinite(y)) {
      if (segment.length) segments.push(segment);
      segment = [];
    } else segment.push({ i, y });
  });
  if (segment.length) segments.push(segment);
  return segments;
}
