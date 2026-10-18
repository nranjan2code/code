import assert from "node:assert/strict";
import { formatChartNumber, niceTicks, scalePosition } from "../src/components/cards/chartScale.ts";

// The live chart that showed 0, 44, 88 for three percentages: its axis is
// round numbers that reach past the top value.
assert.deepEqual(niceTicks(0, 88), [0, 25, 50, 75, 100]);
assert.deepEqual(niceTicks(0, 3), [0, 1, 2, 3]);
assert.deepEqual(niceTicks(0, 0.3), [0, 0.1, 0.2, 0.3]);
assert.deepEqual(niceTicks(0, 2e-12), [0, 5e-13, 1e-12, 1.5e-12, 2e-12]);
assert.deepEqual(niceTicks(0, 2e-12).map(formatChartNumber), ["0", "5e-13", "1e-12", "1.5e-12", "2e-12"]);
const mixed = niceTicks(-12, 30);
assert.ok(mixed[0] <= -12 && mixed.at(-1) >= 30 && mixed.includes(0), String(mixed));
const big = niceTicks(0, 22776.1);
assert.equal(big[0], 0);
assert.ok(big.at(-1) >= 22776.1 && big.length <= 6, String(big));

// Every finite ordered or reversed domain gets a small, increasing scale that
// contains the complete domain. Numerically unrepresentable grids keep their
// exact endpoints instead of rounding away the interval.
for (const [min, max] of [
  [1e-12, 8e-12], [-8e-12, -1e-12], [-3, 8], [1, 1 + Number.EPSILON],
  [1e300, 1.1e300], [-Number.MAX_VALUE, Number.MAX_VALUE],
  [Number.MIN_VALUE, 2 * Number.MIN_VALUE], [3, -2],
]) {
  const ticks = niceTicks(min, max);
  const low = Math.min(min, max);
  const high = Math.max(min, max);
  assert.ok(ticks.length >= 2 && ticks.length <= 8, `${min}..${max}: ${ticks}`);
  assert.ok(ticks.every(Number.isFinite), `${min}..${max}: ${ticks}`);
  assert.ok(ticks[0] <= low && ticks.at(-1) >= high, `${min}..${max}: ${ticks}`);
  assert.ok(ticks.every((tick, index) => index === 0 || tick > ticks[index - 1]), `${min}..${max}: ${ticks}`);
}

assert.deepEqual(niceTicks(Infinity, 1), []);
assert.deepEqual(niceTicks(NaN, 1), []);
assert.deepEqual(niceTicks(4, 4).slice(0, 1), [4]);
assert.ok(niceTicks(4, 4).at(-1) > 4);
assert.ok(niceTicks(-4, -4)[0] <= -4 && niceTicks(-4, -4).at(-1) >= -4);
const equalTinyTicks = niceTicks(Number.MIN_VALUE, Number.MIN_VALUE);
assert.ok(equalTinyTicks[0] <= Number.MIN_VALUE && equalTinyTicks.at(-1) > Number.MIN_VALUE);
const maxTicks = niceTicks(Number.MAX_VALUE, Number.MAX_VALUE);
assert.ok(maxTicks[0] < Number.MAX_VALUE && maxTicks.at(-1) >= Number.MAX_VALUE);
assert.equal(scalePosition(-Number.MAX_VALUE, -Number.MAX_VALUE, Number.MAX_VALUE), 0);
assert.equal(scalePosition(0, -Number.MAX_VALUE, Number.MAX_VALUE), 0.5);
assert.equal(scalePosition(Number.MAX_VALUE, -Number.MAX_VALUE, Number.MAX_VALUE), 1);
const tinyLabels = niceTicks(1e-12, 8e-12).map(formatChartNumber);
assert.equal(new Set(tinyLabels).size, tinyLabels.length, tinyLabels.join(", "));
const adjacentLabels = [1, 1 + Number.EPSILON].map(formatChartNumber);
assert.notEqual(adjacentLabels[0], adjacentLabels[1]);
for (const value of [Number.MIN_VALUE, -Number.MIN_VALUE, Number.MAX_VALUE, -Number.MAX_VALUE]) {
  const label = formatChartNumber(value);
  assert.equal(Number(label), value);
  assert.ok(label.length <= 24, `${value}: ${label.length} characters`);
}
assert.equal(formatChartNumber(-0), "0");
console.log("chart scale ok");
