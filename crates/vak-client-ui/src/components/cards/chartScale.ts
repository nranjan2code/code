/** Round axis ticks from the data's floor (or zero) to just past its top. */
export function niceTicks(min: number, max: number): number[] {
  if (min === max) max = min + 1;
  const raw = (max - min) / 4;
  const power = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * power).find((s) => s >= raw) ?? raw;
  const start = Math.floor(min / step) * step;
  const ticks: number[] = [];
  for (let value = start; value < max + step * 0.999; value += step) ticks.push(Number(value.toFixed(10)));
  return ticks;
}
