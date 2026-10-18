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
